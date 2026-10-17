use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process;

use tumulus::{
    bind_source, compile_with, environ_now, eval_stream, location, next_main, open_failed,
    parse_stream, program_stream, program_stream_with, reads_input, render, set_dump_flags,
    set_environ, set_import_origin, set_input_line, set_library_paths, set_prog_origin,
    set_unknown, Error, Failure, Filter, Indent, Layout, MainNext, ProgramError, Source, Style,
    Value,
};

fn main() {
    // The runtime ignores SIGPIPE and turns a closed pipe into an error.
    // jq dies on that signal instead, which is what `head` expects.
    reset_sigpipe();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let code = match run(&mut out) {
        Ok(code) => code,
        Err(err) => {
            let _ = out.flush();
            eprintln!("tumulus: {err}");
            1
        }
    };
    if let Err(err) = out.flush() {
        eprintln!("tumulus: {err}");
        process::exit(1);
    }
    if code != 0 {
        process::exit(code);
    }
}

fn reset_sigpipe() {
    #[cfg(unix)]
    unsafe {
        extern "C" {
            fn signal(sig: i32, handler: usize) -> usize;
        }
        // 13 is SIGPIPE. 0 is SIG_DFL.
        signal(13, 0);
    }
}

/// Directory of a `-f` file, as given. A relative path is joined to the working
/// directory and is not canonicalized, so `..` in `search` follows a symlink.
/// A command-line program has no file directory.
fn import_base(program: &Program) -> Option<PathBuf> {
    let Program::File(path) = program else {
        return None;
    };
    let path = Path::new(path);
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            if parent.is_absolute() {
                Some(parent.to_path_buf())
            } else {
                Some(
                    env::current_dir()
                        .map(|cwd| cwd.join(parent))
                        .unwrap_or_else(|_| parent.to_path_buf()),
                )
            }
        }
        _ => env::current_dir().ok(),
    }
}

/// Directory of a `-f` program, or the current directory when the program is an argument.
fn program_origin(program: &Program) -> String {
    let fallback = || {
        env::current_dir()
            .ok()
            .and_then(|path| fs::canonicalize(&path).ok())
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_string())
    };
    match program {
        Program::Text(_) => fallback(),
        Program::File(path) => fs::canonicalize(path)
            .ok()
            .and_then(|full| full.parent().map(|dir| dir.to_string_lossy().into_owned()))
            .filter(|dir| !dir.is_empty())
            .unwrap_or_else(fallback),
    }
}

fn run(out: &mut dyn Write) -> Result<i32, Error> {
    let options = options()?;
    let source = match &options.program {
        Program::Text(text) => text.clone(),
        Program::File(path) => match read_filter(path) {
            Ok(text) => text,
            Err(line) => {
                eprintln!("{line}");
                return Ok(2);
            }
        },
    };
    let scope = defined_names(&options.named);
    set_library_paths(options.libraries.clone());
    set_import_origin(import_base(&options.program));
    let program = match compile_with(&source, &scope) {
        Ok(program) => program,
        Err(ProgramError::Syntax) => {
            eprintln!("tumulus: syntax error");
            return Ok(3);
        }
        Err(ProgramError::Report(text)) => {
            eprint!("{text}");
            return Ok(3);
        }
    };
    let environ = environ_now();
    set_environ(environ.clone());
    set_dump_flags(options.style.ascii, options.style.sort_keys);
    set_prog_origin(program_origin(&options.program));
    set_input_line(0);
    let env = initial_env(&options.named, &options.positional, &environ);
    let mut run = Run::new();
    // `input` reads the same stream the main loop walks.
    if reads_input(&program) {
        apply_shared(&options, &program, &env, &mut run, out)?;
    } else if options.null_input {
        apply_null(options.style, &program, &env, &mut run, out)?;
    } else if options.raw_input && options.slurp {
        apply_raw_slurp(&options.files, options.style, &program, &env, &mut run, out)?;
    } else if options.raw_input {
        apply_raw(&options.files, options.style, &program, &env, &mut run, out)?;
    } else if options.slurp {
        apply_slurp(
            &options.files,
            options.style,
            options.stream,
            options.stream_errors,
            &program,
            &env,
            &mut run,
            out,
        )?;
    } else if options.stream {
        // The same reader as `inputs`, so a value can run on into the next file.
        apply_shared(&options, &program, &env, &mut run, out)?;
    } else {
        apply_values(&options.files, options.style, &program, &env, &mut run, out)?;
    }
    apply_exit_status(&mut run, options.exit_status);
    Ok(run.status)
}

struct Run {
    status: i32,
    missing: bool,
    produced: bool,
    /// The last value written was `null` or `false`.
    last_false: bool,
    /// `halt` or `halt_error` already chose the status.
    stopped: bool,
}

impl Run {
    fn new() -> Self {
        Self {
            status: 0,
            missing: false,
            produced: false,
            last_false: false,
            stopped: false,
        }
    }
}

/// `-e` replaces a successful exit. No output is 4. A last `null` or
/// `false` is 1. Any other value, including `0` and `""`, stays 0.
/// Status 2, 3, and 5 are left as they are. `halt` and `halt_error` are too,
/// including a status of 0.
fn apply_exit_status(run: &mut Run, enabled: bool) {
    if !enabled || run.status != 0 || run.stopped {
        return;
    }
    run.status = if !run.produced {
        4
    } else if run.last_false {
        1
    } else {
        0
    };
}

struct Outcome {
    status: i32,
    halt: bool,
    /// The last value in this input ran without a filter error.
    clear_failure: bool,
    /// `halt` or `halt_error` chose `status`, including 0.
    jq_halt: bool,
}

fn apply_values(
    files: &[String],
    style: Style,
    program: &Filter,
    env: &[(String, Value)],
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<(), Error> {
    if files.is_empty() {
        let outcome = apply(&read_stdin()?, style, program, "<stdin>", env, run, out)?;
        if outcome.jq_halt {
            run.stopped = true;
        }
        run.status = outcome.status;
        return Ok(());
    }
    for path in files {
        match read_input(path) {
            Ok(bytes) => {
                let source = if path == "-" { "<stdin>" } else { path };
                let outcome = apply(&bytes, style, program, source, env, run, out)?;
                if outcome.jq_halt {
                    run.stopped = true;
                    run.status = outcome.status;
                    break;
                }
                if outcome.status != 0 {
                    run.status = outcome.status;
                } else if outcome.clear_failure && run.status == 5 && !run.missing {
                    // A later value that runs cleanly clears an earlier filter failure.
                    run.status = 0;
                }
                if outcome.halt {
                    break;
                }
            }
            Err(err) if path != "-" && err.kind() == io::ErrorKind::NotFound => {
                // jq names the missing file and keeps going.
                eprintln!("jq: error: Could not open file {path}: {}", os_reason(&err));
                if run.status == 0 {
                    run.status = 2;
                }
                run.missing = true;
            }
            Err(err) => return Err(Error::new(format!("cannot read {path}: {err}"))),
        }
    }
    Ok(())
}

fn apply(
    bytes: &[u8],
    style: Style,
    program: &Filter,
    source: &str,
    env: &[(String, Value)],
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<Outcome, Error> {
    let parsed = parse_stream(bytes)?;
    let mut failed = false;
    for item in &parsed.values {
        set_input_line(item.line);
        match run_program(program, &item.value, env, style, run, out)? {
            Ok(()) => failed = false,
            Err(failure) => {
                write_values(&failure.partial, style, run, out)?;
                if let Some(code) = take_stop(&failure) {
                    return Ok(Outcome {
                        status: code,
                        halt: true,
                        clear_failure: false,
                        jq_halt: true,
                    });
                }
                report_error(&format!("{source}:{}", item.line), &failure);
                failed = true;
            }
        }
    }
    if let Some(message) = parsed.error {
        eprintln!("jq: parse error: {message}");
        return Ok(Outcome {
            status: 5,
            halt: true,
            clear_failure: false,
            jq_halt: false,
        });
    }
    Ok(Outcome {
        status: if failed { 5 } else { 0 },
        halt: false,
        clear_failure: !failed && !parsed.values.is_empty(),
        jq_halt: false,
    })
}

/// One stream for `.` and for `input`. `-n` does not take a value first.
fn apply_shared(
    options: &Options,
    program: &Filter,
    env: &[(String, Value)],
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let _guard = bind_source(Source {
        files: options.files.clone(),
        raw: options.raw_input,
        slurp: options.slurp,
        stream: (options.stream || options.stream_errors) && !options.raw_input,
        stream_errors: options.stream_errors && !options.raw_input,
    });
    let mut failed = false;
    if options.null_input {
        set_unknown();
        failed = emit(
            program,
            &Value::Null,
            env,
            options.style,
            "<unknown>",
            0,
            run,
            out,
        )?;
    } else {
        loop {
            if run.stopped {
                break;
            }
            match next_main()? {
                MainNext::Eof => break,
                MainNext::Parse(message) => {
                    eprintln!("jq: parse error: {message}");
                    failed = true;
                    break;
                }
                MainNext::Value(value) => {
                    failed = emit(program, &value, env, options.style, "", 0, run, out)?;
                }
            }
        }
    }
    // A file that could not be opened wins over a filter or parse failure.
    run.status = if open_failed() {
        2
    } else if run.stopped {
        run.status
    } else if failed {
        5
    } else {
        0
    };
    Ok(())
}

fn apply_null(
    style: Style,
    program: &Filter,
    env: &[(String, Value)],
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<(), Error> {
    set_input_line(0);
    match run_program(program, &Value::Null, env, style, run, out)? {
        Ok(()) => run.status = 0,
        Err(failure) => {
            write_values(&failure.partial, style, run, out)?;
            if !note_stop(&failure, run) {
                report_error("<unknown>", &failure);
                run.status = 5;
            }
        }
    }
    Ok(())
}

/// Where a slurped value or a raw line is reported.
struct At {
    source: String,
    line: usize,
}

enum Opened {
    Bytes(Vec<u8>),
    Missing,
}

fn open_input(path: &str) -> Result<Opened, Error> {
    match read_input(path) {
        Ok(bytes) => Ok(Opened::Bytes(bytes)),
        Err(err) if path != "-" && err.kind() == io::ErrorKind::NotFound => {
            eprintln!("jq: error: Could not open file {path}: {}", os_reason(&err));
            Ok(Opened::Missing)
        }
        Err(err) => Err(Error::new(format!("cannot read {path}: {err}"))),
    }
}

fn input_paths(files: &[String]) -> Vec<&str> {
    if files.is_empty() {
        vec!["-"]
    } else {
        files.iter().map(String::as_str).collect()
    }
}

fn source_name(path: &str) -> &str {
    if path == "-" {
        "<stdin>"
    } else {
        path
    }
}

fn newline_count(bytes: &[u8]) -> usize {
    bytes.iter().filter(|byte| **byte == b'\n').count()
}

/// A parse error jq would raise before asking for another file.
/// An error at EOF can still be repaired by the next file's bytes.
fn committed_parse_error(bytes: &[u8]) -> Option<String> {
    let message = parse_stream(bytes).ok()?.error?;
    if message.contains(" at EOF ") {
        None
    } else {
        Some(message)
    }
}

fn finish_input(missing: bool, failed: bool) -> i32 {
    // A missing file wins over a later filter or parse failure.
    if missing {
        2
    } else if failed {
        5
    } else {
        0
    }
}

/// JSON values from every opened file, one array, one run.
/// Bytes are concatenated, so a token can cross a file boundary.
fn apply_slurp(
    files: &[String],
    style: Style,
    stream: bool,
    stream_errors: bool,
    program: &Filter,
    env: &[(String, Value)],
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let mut buf = Vec::new();
    let mut bounds = Vec::new();
    let mut at = At {
        source: "<stdin>".to_string(),
        line: 0,
    };
    let mut missing = false;
    let mut early = None;
    for path in input_paths(files) {
        at.source = source_name(path).to_string();
        at.line = 0;
        match open_input(path)? {
            Opened::Missing => missing = true,
            Opened::Bytes(bytes) => {
                at.line = newline_count(&bytes);
                if !buf.is_empty() {
                    bounds.push(buf.len());
                }
                buf.extend_from_slice(&bytes);
                // A parse error is a value with `--stream-errors`, so a later
                // file is still part of the input.
                if stream_errors {
                    continue;
                }
                if let Some(message) = if stream {
                    committed_stream_error(&buf)
                } else {
                    committed_parse_error(&buf)
                } {
                    early = Some(message);
                    break;
                }
            }
        }
    }
    if let Some(message) = early {
        eprintln!("jq: parse error: {message}");
        run.status = finish_input(missing, true);
        return Ok(());
    }
    if stream {
        let fed = program_stream_with(&buf, true, stream_errors, &bounds);
        let mut failed = false;
        for item in &fed.values {
            if run.stopped {
                break;
            }
            set_input_line(item.line);
            let place = if item.unknown {
                "<unknown>".to_string()
            } else {
                format!("{}:{}", at.source, item.line)
            };
            failed = emit_at(program, &item.value, env, style, &place, run, out)?;
        }
        if let Some(message) = fed.error {
            eprintln!("jq: parse error: {message}");
            run.status = finish_input(missing, true);
            return Ok(());
        }
        run.status = stopped_status(run, missing, failed);
        return Ok(());
    }
    let parsed = parse_stream(&buf)?;
    if let Some(message) = parsed.error {
        eprintln!("jq: parse error: {message}");
        run.status = finish_input(missing, true);
        return Ok(());
    }
    let input = Value::array(parsed.values.into_iter().map(|item| item.value).collect());
    set_input_line(at.line);
    let failed = emit(program, &input, env, style, &at.source, at.line, run, out)?;
    run.status = stopped_status(run, missing, failed);
    Ok(())
}

/// A parse error jq would raise before asking for another file.
/// An error at EOF can still be repaired by the next file's bytes.
fn committed_stream_error(bytes: &[u8]) -> Option<String> {
    let message = program_stream(bytes, true).error?;
    if message.contains(" at EOF ") {
        None
    } else {
        Some(message)
    }
}

/// One string of every opened file, including a trailing newline.
fn apply_raw_slurp(
    files: &[String],
    style: Style,
    program: &Filter,
    env: &[(String, Value)],
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let mut buf = Vec::new();
    let mut at = At {
        source: "<stdin>".to_string(),
        line: 0,
    };
    let mut missing = false;
    for path in input_paths(files) {
        at.source = source_name(path).to_string();
        at.line = 0;
        match open_input(path)? {
            Opened::Missing => missing = true,
            Opened::Bytes(bytes) => {
                at.line = newline_count(&bytes);
                buf.extend_from_slice(&bytes);
            }
        }
    }
    let input = Value::String(decode_raw(&buf));
    set_input_line(at.line);
    let failed = emit(program, &input, env, style, &at.source, at.line, run, out)?;
    run.status = stopped_status(run, missing, failed);
    Ok(())
}

/// One string per line. A line with no newline runs on into the next file.
/// After a missing file, jq reads one more line and then stops.
fn apply_raw(
    files: &[String],
    style: Style,
    program: &Filter,
    env: &[(String, Value)],
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let mut pending = Vec::new();
    let mut at = At {
        source: "<stdin>".to_string(),
        line: 0,
    };
    let mut missing = false;
    let mut one_left = false;
    let mut stop = false;
    let mut last_failed = false;
    for path in input_paths(files) {
        if stop {
            break;
        }
        at.source = source_name(path).to_string();
        at.line = 0;
        match open_input(path)? {
            Opened::Missing => {
                missing = true;
                one_left = true;
            }
            Opened::Bytes(bytes) => {
                let mut line = 0;
                for byte in bytes {
                    if byte == b'\n' {
                        line += 1;
                        let text = decode_raw(&pending);
                        pending.clear();
                        set_input_line(line);
                        last_failed = emit(
                            program,
                            &Value::String(text),
                            env,
                            style,
                            &at.source,
                            line,
                            run,
                            out,
                        )?;
                        if run.stopped {
                            stop = true;
                            break;
                        }
                        if one_left {
                            stop = true;
                            break;
                        }
                    } else {
                        pending.push(byte);
                    }
                }
                at.line = line;
            }
        }
    }
    if !stop && !pending.is_empty() {
        set_input_line(at.line);
        last_failed = emit(
            program,
            &Value::String(decode_raw(&pending)),
            env,
            style,
            &at.source,
            at.line,
            run,
            out,
        )?;
    }
    run.status = stopped_status(run, missing, last_failed);
    Ok(())
}

/// A missing file still wins. `halt` keeps the status it chose.
fn stopped_status(run: &Run, missing: bool, failed: bool) -> i32 {
    if missing {
        2
    } else if run.stopped {
        run.status
    } else {
        finish_input(missing, failed)
    }
}

fn emit(
    program: &Filter,
    input: &Value,
    env: &[(String, Value)],
    style: Style,
    source: &str,
    line: usize,
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<bool, Error> {
    emit_at(
        program,
        input,
        env,
        style,
        &format!("{source}:{line}"),
        run,
        out,
    )
}

fn emit_at(
    program: &Filter,
    input: &Value,
    env: &[(String, Value)],
    style: Style,
    at: &str,
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<bool, Error> {
    match run_program(program, input, env, style, run, out)? {
        Ok(()) => Ok(false),
        Err(failure) => {
            write_values(&failure.partial, style, run, out)?;
            if !note_stop(&failure, run) {
                report_error(at, &failure);
            }
            Ok(true)
        }
    }
}

/// Print each value as it is produced, then drop it. An error's partial
/// results are the values that were not printed yet.
fn run_program(
    program: &Filter,
    input: &Value,
    env: &[(String, Value)],
    style: Style,
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<Result<(), Failure>, Error> {
    let mut io_err = None;
    let result = eval_stream(program, input, env, &mut |value| {
        if let Err(err) = write_one(value, style, run, out) {
            io_err = Some(err);
            return true;
        }
        false
    });
    if let Some(err) = io_err {
        return Err(err);
    }
    Ok(result)
}

/// `halt` and `halt_error` write their own stderr and choose the status.
fn take_stop(failure: &Failure) -> Option<i32> {
    let stop = failure.stop.as_ref()?;
    let mut err = io::stderr();
    let _ = err.write_all(&stop.stderr);
    let _ = err.flush();
    Some(stop.code)
}

fn note_stop(failure: &Failure, run: &mut Run) -> bool {
    let Some(code) = take_stop(failure) else {
        return false;
    };
    run.stopped = true;
    run.status = code;
    true
}

fn report_error(at: &str, failure: &Failure) {
    let at = location().unwrap_or_else(|| at.to_string());
    match &failure.caught {
        Some(Value::String(text)) => {
            let shown = text.split('\0').next().unwrap_or("");
            eprintln!("jq: error (at {at}): {shown}");
        }
        Some(value) => {
            let style = Style {
                layout: Layout::Compact,
                raw: false,
                join: false,
                ascii: false,
                sort_keys: false,
            };
            eprintln!(
                "jq: error (at {at}) (not a string): {}",
                render(value, style)
            );
        }
        None => eprintln!("jq: error (at {at}): {}", failure.message),
    }
}

fn decode_raw(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn write_values(
    values: &[Value],
    style: Style,
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<(), Error> {
    for value in values {
        write_one(value, style, run, out)?;
    }
    Ok(())
}

fn write_one(value: &Value, style: Style, run: &mut Run, out: &mut dyn Write) -> Result<(), Error> {
    let rendered = render(value, style);
    out.write_all(rendered.as_bytes())?;
    if !style.join {
        out.write_all(b"\n")?;
    }
    run.produced = true;
    run.last_false = matches!(value, Value::Null | Value::Bool(false));
    if tumulus::streaming() {
        out.flush()?;
    }
    Ok(())
}

fn os_reason(err: &std::io::Error) -> String {
    let text = err.to_string();
    match text.rfind(" (os error") {
        Some(end) => text[..end].to_string(),
        None => text,
    }
}

enum Program {
    Text(String),
    File(String),
}

struct Options {
    style: Style,
    null_input: bool,
    slurp: bool,
    raw_input: bool,
    exit_status: bool,
    stream: bool,
    stream_errors: bool,
    /// `None` keeps jq's default module path. `Some` is every `-L`.
    libraries: Option<Vec<String>>,
    program: Program,
    files: Vec<String>,
    named: Vec<(String, Value)>,
    positional: Vec<Value>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tail {
    Files,
    Text,
    Json,
}

struct Flags {
    style: Style,
    null_input: bool,
    slurp: bool,
    raw_input: bool,
    exit_status: bool,
    from_file: bool,
    stream: bool,
    stream_errors: bool,
    libraries: Option<Vec<String>>,
}

fn options() -> Result<Options, Error> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut flags = Flags {
        style: Style::default(),
        null_input: false,
        slurp: false,
        raw_input: false,
        exit_status: false,
        from_file: false,
        stream: false,
        stream_errors: false,
        libraries: None,
    };
    let mut named = Vec::new();
    let mut positional = Vec::new();
    let mut operands = Vec::new();
    let mut tail = Tail::Files;
    let mut took = false;
    let mut end = false;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].clone();
        if end {
            push_operand(&mut operands, &mut positional, &mut took, tail, arg);
            index += 1;
            continue;
        }
        if arg == "--" {
            end = true;
            index += 1;
            continue;
        }
        if arg == "-L" {
            let Some(path) = args.get(index + 1) else {
                jq_usage("-L takes a parameter: (e.g. -L /search/path or -L/search/path)");
            };
            index += 2;
            flags
                .libraries
                .get_or_insert_with(Vec::new)
                .push(library_directory(path));
            continue;
        }
        if let Some(path) = arg.strip_prefix("-L") {
            if !path.is_empty() {
                index += 1;
                flags
                    .libraries
                    .get_or_insert_with(Vec::new)
                    .push(library_directory(path));
                continue;
            }
        }
        if arg == "--indent" {
            let Some(value) = args.get(index + 1) else {
                jq_usage("jq: --indent takes one parameter");
            };
            index += 2;
            flags.style.layout = indent_layout(value);
            continue;
        }
        if arg == "--arg" || arg == "--argjson" {
            let json = arg == "--argjson";
            let usage = if json {
                "jq: --argjson takes two parameters (e.g. --argjson varname text)"
            } else {
                "jq: --arg takes two parameters (e.g. --arg varname value)"
            };
            let Some(name) = args.get(index + 1).cloned() else {
                jq_usage(usage);
            };
            let Some(text) = args.get(index + 2).cloned() else {
                jq_usage(usage);
            };
            index += 3;
            // The first definition wins. A repeat is not parsed again, so a
            // later `--argjson` with bad text is not an error.
            if named.iter().any(|(have, _)| have == &name) {
                continue;
            }
            let value = if json {
                match one_json(&text) {
                    Some(value) => value,
                    None => jq_usage("jq: invalid JSON text passed to --argjson"),
                }
            } else {
                Value::String(text)
            };
            named.push((name, value));
            continue;
        }
        if arg == "--slurpfile" || arg == "--rawfile" {
            let raw = arg == "--rawfile";
            let usage = if raw {
                "jq: --rawfile takes two parameters (e.g. --rawfile varname filename)"
            } else {
                "jq: --slurpfile takes two parameters (e.g. --slurpfile varname filename)"
            };
            let Some(name) = args.get(index + 1).cloned() else {
                jq_usage(usage);
            };
            let Some(path) = args.get(index + 2).cloned() else {
                jq_usage(usage);
            };
            index += 3;
            if named.iter().any(|(have, _)| have == &name) {
                continue;
            }
            named.push((name.clone(), file_binding(&arg, &name, &path)));
            continue;
        }
        if arg == "--args" {
            tail = Tail::Text;
            index += 1;
            continue;
        }
        if arg == "--jsonargs" {
            tail = Tail::Json;
            index += 1;
            continue;
        }
        match arg.as_str() {
            "-c" | "--compact-output" => flags.style.layout = Layout::Compact,
            "-n" | "--null-input" => flags.null_input = true,
            "--tab" => flags.style.layout = Layout::Pretty(Indent::Tabs),
            "-r" | "--raw-output" => flags.style.raw = true,
            "-j" | "--join-output" => {
                flags.style.join = true;
                flags.style.raw = true;
            }
            "-a" | "--ascii-output" => flags.style.ascii = true,
            "-S" | "--sort-keys" => flags.style.sort_keys = true,
            "-s" | "--slurp" => flags.slurp = true,
            "-e" | "--exit-status" => flags.exit_status = true,
            "-R" | "--raw-input" => flags.raw_input = true,
            "--stream" => flags.stream = true,
            "--stream-errors" => {
                flags.stream = true;
                flags.stream_errors = true;
            }
            // The file name is the first operand, not the next argument.
            "-f" | "--from-file" => flags.from_file = true,
            "-" => push_operand(&mut operands, &mut positional, &mut took, tail, arg),
            other if other.starts_with("--") => {
                return Err(Error::new(format!("unknown option: {other}")));
            }
            // `-nc` is options. `-1` and `-.a` are programs, because the
            // character after the dash is not a letter.
            other if is_option_cluster(other) => {
                apply_short_cluster(&mut flags, other)?;
            }
            _ => push_operand(&mut operands, &mut positional, &mut took, tail, arg),
        }
        index += 1;
    }
    let (program, files) = if flags.from_file {
        if operands.is_empty() {
            (Program::File(".".to_string()), Vec::new())
        } else {
            let path = operands.remove(0);
            (Program::File(path), operands)
        }
    } else if operands.is_empty() {
        (Program::Text(".".to_string()), Vec::new())
    } else {
        let text = operands.remove(0);
        (Program::Text(text), operands)
    };
    Ok(Options {
        style: flags.style,
        null_input: flags.null_input,
        slurp: flags.slurp,
        raw_input: flags.raw_input,
        exit_status: flags.exit_status,
        stream: flags.stream,
        stream_errors: flags.stream_errors,
        program,
        files,
        named,
        positional,
        libraries: flags.libraries,
    })
}

/// An existing directory, including one reached through a symlink, is its
/// canonical path. A path that cannot be resolved stays as written.
fn library_directory(text: &str) -> String {
    fs::canonicalize(text)
        .map(|full| full.to_string_lossy().into_owned())
        .unwrap_or_else(|_| text.to_string())
}

fn push_operand(
    operands: &mut Vec<String>,
    positional: &mut Vec<Value>,
    took: &mut bool,
    tail: Tail,
    arg: String,
) {
    // The program, or the file named by `-f`, is never a positional argument.
    if !*took {
        *took = true;
        operands.push(arg);
        return;
    }
    match tail {
        Tail::Files => operands.push(arg),
        Tail::Text => positional.push(Value::String(arg)),
        Tail::Json => match one_json(&arg) {
            Some(value) => positional.push(value),
            None => jq_usage("jq: invalid JSON text passed to --jsonargs"),
        },
    }
}

fn file_binding(flag: &str, name: &str, path: &str) -> Value {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            let reason = filter_reason(path, &err);
            jq_die(&format!(
                "jq: Bad JSON in {flag} {name} {path}: Could not open {path}: {reason}"
            ));
        }
    };
    if flag == "--rawfile" {
        return Value::String(String::from_utf8_lossy(&bytes).into_owned());
    }
    let parsed = match parse_stream(&bytes) {
        Ok(parsed) => parsed,
        Err(err) => jq_die(&format!("jq: Bad JSON in {flag} {name} {path}: {err}")),
    };
    if let Some(message) = parsed.error {
        jq_die(&format!("jq: Bad JSON in {flag} {name} {path}: {message}"));
    }
    Value::array(parsed.values.into_iter().map(|item| item.value).collect())
}

fn is_option_cluster(arg: &str) -> bool {
    let mut chars = arg.chars();
    chars.next() == Some('-') && chars.next().is_some_and(|ch| ch.is_ascii_alphabetic())
}

fn apply_short_cluster(flags: &mut Flags, arg: &str) -> Result<(), Error> {
    for flag in arg.chars().skip(1) {
        match flag {
            'c' => flags.style.layout = Layout::Compact,
            'n' => flags.null_input = true,
            'r' => flags.style.raw = true,
            'j' => {
                flags.style.join = true;
                flags.style.raw = true;
            }
            'a' => flags.style.ascii = true,
            'S' => flags.style.sort_keys = true,
            's' => flags.slurp = true,
            'e' => flags.exit_status = true,
            'R' => flags.raw_input = true,
            'f' => flags.from_file = true,
            _ => return Err(Error::new(format!("unknown option: {arg}"))),
        }
    }
    Ok(())
}

fn one_json(text: &str) -> Option<Value> {
    let parsed = parse_stream(text.as_bytes()).ok()?;
    if parsed.error.is_some() || parsed.values.len() != 1 {
        return None;
    }
    parsed.values.into_iter().next().map(|item| item.value)
}

/// Names a program can read. `$ARGS` is always one of them.
/// A name that is not a `$name` spelling still lands in `$ARGS.named`.
fn defined_names(named: &[(String, Value)]) -> Vec<String> {
    let mut names = vec!["ARGS".to_string(), "ENV".to_string()];
    for (name, _) in named {
        // `$ENV` is the process environment. `--arg ENV` only fills `$ARGS.named`.
        if name != "ARGS" && name != "ENV" && is_binding_name(name) {
            names.push(name.clone());
        }
    }
    names
}

fn initial_env(
    named: &[(String, Value)],
    positional: &[Value],
    environ: &Value,
) -> Vec<(String, Value)> {
    let mut fields = Vec::new();
    let mut env = Vec::new();
    for (name, value) in named {
        fields.push((name.clone(), value.clone()));
        if name != "ARGS" && name != "ENV" && is_binding_name(name) {
            env.push((name.clone(), value.clone()));
        }
    }
    let args = Value::object(vec![
        ("positional".to_string(), Value::array(positional.to_vec())),
        ("named".to_string(), Value::object(fields)),
    ]);
    // The first binding wins. `$ARGS` and `$ENV` sit in front of `--arg`.
    env.insert(0, ("ENV".to_string(), environ.clone()));
    env.insert(0, ("ARGS".to_string(), args));
    env
}

fn is_binding_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn read_filter(path: &str) -> Result<String, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(err) => Err(format!(
            "jq: Could not open {path}: {}",
            filter_reason(path, &err)
        )),
    }
}

fn filter_reason(path: &str, err: &std::io::Error) -> String {
    if err.kind() == io::ErrorKind::IsADirectory
        || err.raw_os_error() == Some(21)
        || fs::metadata(path).is_ok_and(|meta| meta.is_dir())
    {
        "It's a directory".to_string()
    } else {
        os_reason(err)
    }
}

fn indent_layout(text: &str) -> Layout {
    // jq parses the argument with strtol, keeps the low 32 bits, and
    // accepts only -1 through 7. -1 is tabs and 0 is compact.
    let wide = c_strtol(text);
    let narrowed = wide as i32;
    match narrowed {
        0 => Layout::Compact,
        -1 => Layout::Pretty(Indent::Tabs),
        1..=7 => Layout::Pretty(Indent::Spaces(narrowed as u8)),
        _ => jq_usage("jq: --indent takes a number between -1 and 7"),
    }
}

fn c_strtol(text: &str) -> i64 {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() && matches!(bytes[index], b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
    {
        index += 1;
    }
    let mut negative = false;
    if index < bytes.len() && (bytes[index] == b'+' || bytes[index] == b'-') {
        negative = bytes[index] == b'-';
        index += 1;
    }
    if index >= bytes.len() || !bytes[index].is_ascii_digit() {
        return 0;
    }
    let mut magnitude: i128 = 0;
    let mut overflow = false;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        let digit = i128::from(bytes[index] - b'0');
        magnitude = magnitude.saturating_mul(10).saturating_add(digit);
        if (!negative && magnitude > i128::from(i64::MAX))
            || (negative && magnitude > i128::from(i64::MAX) + 1)
        {
            overflow = true;
        }
        index += 1;
    }
    if overflow {
        return if negative { i64::MIN } else { i64::MAX };
    }
    if negative {
        if magnitude == i128::from(i64::MAX) + 1 {
            i64::MIN
        } else {
            -(magnitude as i64)
        }
    } else {
        magnitude as i64
    }
}

fn jq_die(line: &str) -> ! {
    eprintln!("{line}");
    process::exit(2);
}

fn jq_usage(line: &str) -> ! {
    eprintln!("{line}");
    eprintln!("Use jq --help for help with command-line options,");
    eprintln!("or see the jq manpage, or online docs  at https://jqlang.github.io/jq");
    process::exit(2);
}

fn read_stdin() -> Result<Vec<u8>, std::io::Error> {
    let mut buf = Vec::new();
    io::stdin().read_to_end(&mut buf)?;
    Ok(buf)
}

fn read_input(path: &str) -> Result<Vec<u8>, std::io::Error> {
    if path == "-" {
        return read_stdin();
    }
    fs::read(path)
}
