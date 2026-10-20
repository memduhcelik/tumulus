use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::eprint;
use crate::eprintln;
use crate::{
    bind_source, compile_with, environ_now, eval_stream, location, next_main, open_failed,
    output_place, parse_colors, parse_seq, parse_stream, program_stream, program_stream_with,
    reads_input, render, set_dump_flags, set_environ, set_import_origin, set_input_line,
    set_library_paths, set_output_place, set_palette, set_prog_origin, set_unknown, Error, Failure,
    Filter, Indent, Layout, MainNext, ProgramError, SeqItem, Source, Style, Value,
};

/// What one run wrote, and the status it would pass to `exit`.
pub struct RunOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub code: i32,
}

/// Run the binary's program against the real standard streams.
pub fn run_cli(out: &mut dyn Write) -> Result<i32, Error> {
    run(out)
}

/// Run once in this process. `args[0]` is the program name, as `argv[0]` is
/// for the binary. The operating system keeps the low eight bits of `code`.
pub fn execute(args: &[String], stdin: &[u8]) -> RunOutput {
    // jq is a new process for every program, so its zone name starts over.
    let _zone = crate::date::zone_guard();
    crate::host::begin(args, stdin);
    let mut stdout = Vec::new();
    let caught =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match run(&mut stdout) {
            Ok(code) => code,
            Err(err) => {
                eprintln!("tumulus: {err}");
                1
            }
        }));
    let (stderr, extra) = crate::host::take();
    let mut out = extra;
    out.extend(stdout);
    let code = match caught {
        Ok(code) => code,
        Err(payload) => match payload.downcast::<crate::host::Stop>() {
            Ok(stop) => stop.0,
            Err(payload) => std::panic::resume_unwind(payload),
        },
    };
    RunOutput {
        stdout: out,
        stderr,
        code: code as u8 as i32,
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
    // A test runs many programs on one thread. Start from an empty reader.
    crate::feed::reset_for_run();
    crate::print::reset_palette();
    crate::filter::reset_for_run();
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
    if let Ok(text) = env::var("JQ_COLORS") {
        match parse_colors(&text) {
            Some(colors) => set_palette(colors),
            None => eprintln!("Failed to set $JQ_COLORS"),
        }
    }
    set_dump_flags(options.style.ascii, options.style.sort_keys);
    crate::set_dump_color(options.style.color);
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
            options.seq,
            &program,
            &env,
            &mut run,
            out,
        )?;
    } else if options.stream || (!options.seq && !options.files.is_empty()) {
        // Every file is one stream. An array or an object can open in one
        // file and close in a later one. Plain `--seq` is joined in
        // `apply_values`. Standard input on its own stays one read.
        apply_shared(&options, &program, &env, &mut run, out)?;
    } else {
        apply_values(
            &options.files,
            options.style,
            options.seq,
            &program,
            &env,
            &mut run,
            out,
        )?;
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
    seq: bool,
    program: &Filter,
    env: &[(String, Value)],
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<(), Error> {
    // `--unbuffered` prints a finished value before stdin closes.
    if files.is_empty() && style.flush {
        return apply_incremental(style, seq, program, env, run, out);
    }
    if files.is_empty() {
        let outcome = apply(
            &read_stdin()?,
            style,
            seq,
            program,
            "<stdin>",
            &[],
            env,
            run,
            out,
        )?;
        if outcome.jq_halt {
            run.stopped = true;
        }
        run.status = outcome.status;
        return Ok(());
    }
    // `--seq` is one byte stream. A number can stop in one file and
    // finish in the next, and a truncated number is warned once, at the end.
    if seq {
        return apply_seq_files(files, style, program, env, run, out);
    }
    let mut dirs = DirNote::default();
    let mut saw_dir = false;
    let mut saw_file = false;
    for path in files {
        if path_is_dir(path) {
            // The message waits until another file is opened. The last
            // directory of the run stays quiet, and the status is still 2.
            dirs.directory();
            saw_dir = true;
            run.missing = true;
            if run.status == 0 {
                run.status = 2;
            }
            continue;
        }
        dirs.other();
        match read_input(path) {
            Ok(bytes) => {
                saw_file = true;
                let source = if path == "-" { "<stdin>" } else { path };
                let outcome = apply(&bytes, style, seq, program, source, &[], env, run, out)?;
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
    // `--seq` still parses an empty input when every file was a directory.
    if seq && saw_dir && !saw_file && !run.stopped {
        let outcome = apply(b"", style, true, program, "<stdin>", &[], env, run, out)?;
        if outcome.jq_halt {
            run.stopped = true;
            run.status = outcome.status;
        }
    }
    if saw_dir && !run.stopped {
        run.status = 2;
    }
    Ok(())
}

fn apply(
    bytes: &[u8],
    style: Style,
    seq: bool,
    program: &Filter,
    source: &str,
    spans: &[SeqSpan],
    env: &[(String, Value)],
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<Outcome, Error> {
    if seq {
        return apply_seq(bytes, style, program, source, spans, env, run, out);
    }
    let parsed = parse_stream(bytes)?;
    let mut failed = false;
    for item in &parsed.values {
        set_input_line(item.line);
        set_output_place(format!("{source}:{}", item.line));
        match run_program(program, &item.value, env, style, run, out)? {
            Ok(()) if run.stopped => {
                return Ok(Outcome {
                    status: run.status,
                    halt: true,
                    clear_failure: false,
                    jq_halt: false,
                });
            }
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

fn apply_incremental(
    style: Style,
    seq: bool,
    program: &Filter,
    env: &[(String, Value)],
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let _guard = bind_source(Source {
        files: Vec::new(),
        raw: false,
        slurp: false,
        stream: false,
        stream_errors: false,
        seq,
        piecewise: true,
    });
    let mut failed = false;
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
            MainNext::Soft(message) => {
                eprintln!("jq: ignoring parse error: {message}");
            }
            MainNext::Value(value) => {
                failed = emit(program, &value, env, style, "<stdin>", 0, run, out)?;
            }
        }
    }
    run.status = if run.stopped {
        run.status
    } else if failed {
        5
    } else {
        0
    };
    Ok(())
}

/// One file inside a joined `--seq` input. `start` is its first byte.
struct SeqSpan {
    name: String,
    start: usize,
}

fn apply_seq_files(
    files: &[String],
    style: Style,
    program: &Filter,
    env: &[(String, Value)],
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let mut buf = Vec::new();
    let mut spans = Vec::new();
    let mut dirs = DirNote::default();
    let mut saw_dir = false;
    let mut saw_file = false;
    // Values already handed to the program. A later file can finish a
    // number, so the buffer is parsed again, and those values are not run twice.
    let mut ran = 0usize;
    for (index, path) in files.iter().enumerate() {
        if run.stopped {
            break;
        }
        if path_is_dir(path) {
            // The message waits until another file is opened. The last
            // directory of the run stays quiet, and the status is still 2.
            dirs.directory();
            saw_dir = true;
            run.missing = true;
            if run.status == 0 {
                run.status = 2;
            }
        } else {
            dirs.other();
            match read_input(path) {
                Ok(bytes) => {
                    saw_file = true;
                    let source = if path == "-" {
                        "<stdin>".to_string()
                    } else {
                        path.to_string()
                    };
                    spans.push(SeqSpan {
                        name: source,
                        start: buf.len(),
                    });
                    buf.extend_from_slice(&bytes);
                }
                Err(err) if path != "-" && err.kind() == io::ErrorKind::NotFound => {
                    eprintln!("jq: error: Could not open file {path}: {}", os_reason(&err));
                    if run.status == 0 {
                        run.status = 2;
                    }
                    run.missing = true;
                }
                Err(err) => return Err(Error::new(format!("cannot read {path}: {err}"))),
            }
        }
        if !saw_file || run.stopped {
            continue;
        }
        // An error at EOF may still be repaired by a later file. `halt`
        // stops before that file is opened, once the current value has run.
        let items = parse_seq(&buf);
        let more = index + 1 < files.len();
        let end = seq_run_end(&items, more);
        if end <= ran {
            continue;
        }
        let outcome = drive_seq(
            &items[ran..end],
            &buf,
            style,
            program,
            "<stdin>",
            &spans,
            env,
            run,
            out,
        )?;
        ran = end;
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
    // `--seq` still parses an empty input when every file was a directory.
    if saw_dir && !saw_file && !run.stopped {
        let outcome = apply(b"", style, true, program, "<stdin>", &[], env, run, out)?;
        if outcome.jq_halt {
            run.stopped = true;
            run.status = outcome.status;
        }
    }
    if saw_dir && !run.stopped {
        run.status = 2;
    }
    Ok(())
}

/// How many seq items are safe to run. The last error is kept back while
/// another file might still finish the value.
fn seq_run_end(items: &[SeqItem], more: bool) -> usize {
    if more {
        if let Some(SeqItem::Error(message)) = items.last() {
            if message.contains(" at EOF ") {
                return items.len() - 1;
            }
        }
    }
    items.len()
}

fn apply_seq(
    bytes: &[u8],
    style: Style,
    program: &Filter,
    source: &str,
    spans: &[SeqSpan],
    env: &[(String, Value)],
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<Outcome, Error> {
    drive_seq(
        &parse_seq(bytes),
        bytes,
        style,
        program,
        source,
        spans,
        env,
        run,
        out,
    )
}

fn drive_seq(
    items: &[SeqItem],
    bytes: &[u8],
    style: Style,
    program: &Filter,
    source: &str,
    spans: &[SeqSpan],
    env: &[(String, Value)],
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<Outcome, Error> {
    let mut failed = false;
    let mut any = false;
    for item in items {
        match item {
            SeqItem::Error(message) => {
                eprintln!("jq: ignoring parse error: {message}");
            }
            SeqItem::Value(item) => {
                any = true;
                let (name, line) = seq_place(source, spans, bytes, item.end, item.line);
                set_input_line(line);
                set_output_place(format!("{name}:{line}"));
                match run_program(program, &item.value, env, style, run, out)? {
                    Ok(()) if run.stopped => {
                        return Ok(Outcome {
                            status: run.status,
                            halt: true,
                            clear_failure: false,
                            jq_halt: false,
                        });
                    }
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
                        report_error(&format!("{name}:{line}"), &failure);
                        failed = true;
                    }
                }
            }
        }
    }
    Ok(Outcome {
        status: if failed { 5 } else { 0 },
        halt: false,
        clear_failure: !failed && any,
        jq_halt: false,
    })
}

/// File and line of a `--seq` value. One file keeps the line the parser
/// counted. Several files use the file that holds the value's last byte,
/// and the line count starts again in that file.
fn seq_place(
    source: &str,
    spans: &[SeqSpan],
    bytes: &[u8],
    end: usize,
    parsed_line: usize,
) -> (String, usize) {
    if spans.len() < 2 {
        let name = if spans.len() == 1 {
            spans[0].name.clone()
        } else {
            source.to_string()
        };
        return (name, parsed_line);
    }
    let at = end.saturating_sub(1);
    let mut index = 0usize;
    for (i, span) in spans.iter().enumerate() {
        if span.start <= at {
            index = i;
        } else {
            break;
        }
    }
    let start = spans[index].start;
    let stop = spans
        .get(index + 1)
        .map(|span| span.start)
        .unwrap_or(bytes.len());
    let local = end.saturating_sub(start).min(stop.saturating_sub(start));
    let line = 1 + bytes[start..start + local]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count();
    (spans[index].name.clone(), line)
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
        seq: options.seq && !options.raw_input,
        piecewise: options.style.flush,
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
                MainNext::Soft(message) => {
                    eprintln!("jq: ignoring parse error: {message}");
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
        Ok(()) if run.stopped => {}
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

/// A directory is not announced until a later file is opened.
#[derive(Default)]
struct DirNote {
    pending: bool,
}

impl DirNote {
    fn directory(&mut self) {
        self.announce();
        self.pending = true;
    }

    fn other(&mut self) {
        self.announce();
    }

    fn announce(&mut self) {
        if self.pending {
            eprintln!("jq: error: Is a directory");
            self.pending = false;
        }
    }
}

fn path_is_dir(path: &str) -> bool {
    path != "-" && fs::metadata(path).is_ok_and(|meta| meta.is_dir())
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
    seq: bool,
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
    let mut dirs = DirNote::default();
    for path in input_paths(files) {
        at.source = source_name(path).to_string();
        at.line = 0;
        if path_is_dir(path) {
            dirs.directory();
            missing = true;
            continue;
        }
        dirs.other();
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
                if stream_errors || seq {
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
    if seq {
        let mut values = Vec::new();
        for item in parse_seq(&buf) {
            match item {
                SeqItem::Error(message) => {
                    eprintln!("jq: ignoring parse error: {message}");
                }
                SeqItem::Value(item) => values.push(item.value),
            }
        }
        let input = if stream {
            let mut streamed = Vec::new();
            for value in values {
                let bytes = render(
                    &value,
                    Style {
                        layout: Layout::Compact,
                        ..Style::default()
                    },
                )
                .into_bytes();
                let fed = program_stream(&bytes, false);
                for item in fed.values {
                    streamed.push(item.value);
                }
            }
            Value::array(streamed)
        } else {
            Value::array(values)
        };
        set_input_line(at.line);
        let failed = emit(program, &input, env, style, &at.source, at.line, run, out)?;
        run.status = stopped_status(run, missing, failed);
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
    let mut dirs = DirNote::default();
    for path in input_paths(files) {
        at.source = source_name(path).to_string();
        at.line = 0;
        if path_is_dir(path) {
            dirs.directory();
            missing = true;
            continue;
        }
        dirs.other();
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
    let mut dirs = DirNote::default();
    for path in input_paths(files) {
        if stop {
            break;
        }
        at.source = source_name(path).to_string();
        at.line = 0;
        if path_is_dir(path) {
            dirs.directory();
            missing = true;
            continue;
        }
        dirs.other();
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
    set_output_place(at.to_string());
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
    let result = eval_stream(program, input, env, &mut |value| match write_one(
        value, style, run, out,
    ) {
        Ok(true) => true,
        Ok(false) => false,
        Err(err) => {
            io_err = Some(err);
            true
        }
    });
    if let Some(err) = io_err {
        return Err(err);
    }
    Ok(result)
}

/// `halt` and `halt_error` write their own stderr and choose the status.
fn take_stop(failure: &Failure) -> Option<i32> {
    let stop = failure.stop.as_ref()?;
    crate::host::write_stderr_bytes(&stop.stderr);
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
                ..Style::default()
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
        if write_one(value, style, run, out)? {
            break;
        }
    }
    Ok(())
}

fn write_one(
    value: &Value,
    style: Style,
    run: &mut Run,
    out: &mut dyn Write,
) -> Result<bool, Error> {
    if style.nul {
        if let Value::String(text) = value {
            if style.raw && !style.ascii && text.chars().any(|ch| ch == '\0') {
                let at = {
                    let place = output_place();
                    if place.is_empty() {
                        location().unwrap_or_else(|| "<unknown>".to_string())
                    } else {
                        place
                    }
                };
                eprintln!(
                    "jq: error (at {at}): Cannot dump a string containing NUL with --raw-output0 option"
                );
                run.status = 5;
                run.stopped = true;
                return Ok(true);
            }
        }
    }
    // A raw string is not a JSON text, so `--seq` does not prefix it.
    let raw_string = matches!(value, Value::String(_)) && style.raw && !style.ascii;
    if style.seq && !raw_string {
        out.write_all(b"\x1e")?;
    }
    let rendered = render(value, style);
    out.write_all(rendered.as_bytes())?;
    if style.nul {
        out.write_all(b"\0")?;
    } else if !style.join {
        out.write_all(b"\n")?;
    }
    run.produced = true;
    run.last_false = matches!(value, Value::Null | Value::Bool(false));
    if style.flush || crate::streaming() {
        out.flush()?;
    }
    Ok(false)
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
    seq: bool,
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
    /// `-C` was seen. `-M` still wins.
    color: bool,
    /// `-M` was seen.
    mono: bool,
    seq: bool,
    unbuffered: bool,
    libraries: Option<Vec<String>>,
}

fn options() -> Result<Options, Error> {
    let args: Vec<String> = crate::host::args().into_iter().skip(1).collect();
    let mut flags = Flags {
        style: Style::default(),
        null_input: false,
        slurp: false,
        raw_input: false,
        exit_status: false,
        from_file: false,
        stream: false,
        stream_errors: false,
        color: false,
        mono: false,
        seq: false,
        unbuffered: false,
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
            "--seq" => flags.seq = true,
            "--raw-output0" => {
                flags.style.raw = true;
                flags.style.nul = true;
            }
            "--unbuffered" => flags.unbuffered = true,
            "--color-output" => flags.color = true,
            "--monochrome-output" => flags.mono = true,
            "--version" => {
                print_version();
                crate::host::stop(0);
            }
            "--help" => {
                print_help();
                crate::host::stop(0);
            }
            "--build-configuration" => {
                print_build();
                crate::host::stop(0);
            }
            // The file name is the first operand, not the next argument.
            "-f" | "--from-file" => flags.from_file = true,
            "-" => push_operand(&mut operands, &mut positional, &mut took, tail, arg),
            other if other.starts_with("--") => {
                jq_usage(&format!("jq: Unknown option {other}"));
            }
            // `-nc` is options. `-1` and `-.a` are programs, because the
            // character after the dash is not a letter.
            other if is_option_cluster(other) => {
                let body = &other[1..];
                // `h` anywhere in the cluster is help, including `-hx` and
                // `-Vh`, and the other letters are not checked. `V` is the
                // version only when `h` is absent.
                if body.contains('h') {
                    print_help();
                    crate::host::stop(0);
                }
                if body.contains('V') {
                    print_version();
                    crate::host::stop(0);
                }
                apply_short_cluster(&mut flags, other)?;
            }
            _ => push_operand(&mut operands, &mut positional, &mut took, tail, arg),
        }
        index += 1;
    }
    flags.style.color = if flags.mono {
        false
    } else if flags.color {
        true
    } else {
        auto_color()
    };
    flags.style.seq = flags.seq;
    flags.style.flush = flags.unbuffered;
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
        seq: flags.seq,
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
            'C' => flags.color = true,
            'M' => flags.mono = true,
            // The whole argument is named, including letters that are known.
            _ => jq_usage(&format!("jq: Unknown option {arg}")),
        }
    }
    Ok(())
}

/// Color when stdout is a terminal, unless `NO_COLOR` is set to something.
fn auto_color() -> bool {
    match env::var_os("NO_COLOR") {
        Some(value) if !value.is_empty() => false,
        _ => crate::host::stdout_is_terminal(),
    }
}

/// `tumulus-0.11.0`. jq prints its own build the same way.
fn print_version() {
    crate::host::write_stdout_line(&format!("tumulus-{}", env!("CARGO_PKG_VERSION")));
}

/// One line. The text is the version the bundled library reports.
fn print_build() {
    crate::host::write_stdout_line(&format!("oniguruma={}", oniguruma_version()));
}

fn oniguruma_version() -> String {
    let text = unsafe { std::ffi::CStr::from_ptr(onig_sys::onig_version()) };
    text.to_string_lossy().into_owned()
}

fn print_help() {
    let invoked = crate::host::args()
        .into_iter()
        .next()
        .unwrap_or_else(|| "tumulus".to_string());
    let text = help_text(&invoked);
    for line in text.split('\n') {
        crate::host::write_stdout_line(line);
    }
}

fn help_text(invoked: &str) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let onig = oniguruma_version();
    format!(
        "\
tumulus {version}

Usage:  {invoked} [options] <filter> [file...]
        {invoked} [options] --args <filter> [string...]
        {invoked} [options] --jsonargs <filter> [JSON text...]

tumulus reads JSON values, runs the filter on each one, and writes
each result as JSON. With no filter, the filter is `.`, which passes
the value through and changes only the layout.

Options:
  -n, --null-input         Read nothing. Run the filter once on null.
  -R, --raw-input          Take each line as one string.
  -s, --slurp              Gather every value into one array, then run once.
  -c, --compact-output     Print compact JSON.
  -r, --raw-output         Print a string as text, not as a JSON string.
  --raw-output0            Like -r, and end each output with a NUL.
  -j, --join-output        Like -r, and do not add a newline.
  -a, --ascii-output       Write non-ASCII as \\uXXXX.
  -S, --sort-keys          Sort the keys of each object.
  -C, --color-output       Color the JSON.
  -M, --monochrome-output  Leave the JSON uncolored. This wins over -C.
  --tab                    Indent with tabs.
  --indent N               Indent with N spaces, from 1 to 7. 0 is compact
                           and -1 is tabs.
  --unbuffered             Flush after each output.
  --stream                 Read the input as path items.
  --stream-errors          Like --stream, and hand a parse error to the filter
                           as one value.
  --seq                    Read and write application/json-seq.
  -f, --from-file FILE     Read the filter from FILE.
  -L DIR                   Search for modules in DIR.
  --arg NAME VALUE         Set $NAME to the string VALUE.
  --argjson NAME VALUE     Set $NAME to the JSON value VALUE.
  --slurpfile NAME FILE    Set $NAME to an array of the JSON values in FILE.
  --rawfile NAME FILE      Set $NAME to the text of FILE.
  --args                   Take the remaining arguments as strings.
  --jsonargs               Take the remaining arguments as JSON values.
  -e, --exit-status        Exit 1 when the last result is false or null, and
                           4 when the filter yields nothing.
  -V, --version            Print the version and exit.
  --build-configuration    Print oniguruma={onig} and exit.
  -h, --help               Print this text and exit.
  --                       Stop reading options.

The first of -h, --help, -V, --version, and --build-configuration is
enough. Later arguments are not read. In one run of short options, h
is help, and the other letters are not read.

--arg and --argjson are also fields of $ARGS.named. --args and
--jsonargs fill $ARGS.positional.\
"
    )
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
    crate::host::stop(2);
}

fn jq_usage(line: &str) -> ! {
    eprintln!("{line}");
    eprintln!("Use jq --help for help with command-line options,");
    eprintln!("or see the jq manpage, or online docs  at https://jqlang.github.io/jq");
    crate::host::stop(2);
}

fn read_stdin() -> Result<Vec<u8>, std::io::Error> {
    let mut buf = Vec::new();
    crate::host::read_stdin_to_end(&mut buf)?;
    Ok(buf)
}

fn read_input(path: &str) -> Result<Vec<u8>, std::io::Error> {
    if path == "-" {
        return read_stdin();
    }
    fs::read(path)
}
