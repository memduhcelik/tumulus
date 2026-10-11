use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::process;

use tumulus::{compile, eval, parse_stream, render, Error, Indent, Layout, ProgramError, Style};

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

fn run(out: &mut dyn Write) -> Result<i32, Error> {
    let options = options()?;
    let program = match compile(&options.program) {
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
    // `-n` runs the filter once on null and does not read stdin or files.
    if options.null_input {
        return apply_null(options.style, &program, out);
    }
    if options.files.is_empty() {
        let outcome = apply(&read_stdin()?, options.style, &program, "<stdin>", out)?;
        return Ok(outcome.status);
    }
    let mut status = 0;
    let mut missing = false;
    for path in &options.files {
        match read_input(path) {
            Ok(bytes) => {
                let source = if path == "-" {
                    "<stdin>"
                } else {
                    path.as_str()
                };
                let outcome = apply(&bytes, options.style, &program, source, out)?;
                if outcome.status != 0 {
                    status = outcome.status;
                } else if outcome.clear_failure && status == 5 && !missing {
                    // A later value that runs cleanly clears an earlier filter failure.
                    status = 0;
                }
                if outcome.halt {
                    break;
                }
            }
            Err(err) if path != "-" && err.kind() == io::ErrorKind::NotFound => {
                // jq names the missing file and keeps going.
                eprintln!("jq: error: Could not open file {path}: {}", os_reason(&err));
                if status == 0 {
                    status = 2;
                }
                missing = true;
            }
            Err(err) => return Err(Error::new(format!("cannot read {path}: {err}"))),
        }
    }
    Ok(status)
}

struct Outcome {
    status: i32,
    halt: bool,
    /// The last value in this input ran without a filter error.
    clear_failure: bool,
}

fn apply(
    bytes: &[u8],
    style: Style,
    program: &tumulus::Filter,
    source: &str,
    out: &mut dyn Write,
) -> Result<Outcome, Error> {
    let parsed = parse_stream(bytes)?;
    let mut failed = false;
    for item in &parsed.values {
        match eval(program, &item.value) {
            Ok(values) => {
                write_values(&values, style, out)?;
                failed = false;
            }
            Err(failure) => {
                write_values(&failure.partial, style, out)?;
                eprintln!("jq: error (at {source}:{}): {}", item.line, failure.message);
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
        });
    }
    Ok(Outcome {
        status: if failed { 5 } else { 0 },
        halt: false,
        clear_failure: !failed && !parsed.values.is_empty(),
    })
}

fn write_values(values: &[tumulus::Value], style: Style, out: &mut dyn Write) -> Result<(), Error> {
    for value in values {
        let rendered = render(value, style);
        out.write_all(rendered.as_bytes())?;
        if !style.join {
            out.write_all(b"\n")?;
        }
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

struct Options {
    style: Style,
    null_input: bool,
    program: String,
    files: Vec<String>,
}

fn options() -> Result<Options, Error> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut style = Style::default();
    let mut null_input = false;
    let mut operands = Vec::new();
    let mut end = false;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if end {
            operands.push(arg.clone());
            index += 1;
            continue;
        }
        if arg == "--" {
            end = true;
            index += 1;
            continue;
        }
        if arg == "--indent" {
            let Some(value) = args.get(index + 1) else {
                jq_usage("jq: --indent takes one parameter");
            };
            index += 2;
            style.layout = indent_layout(value);
            continue;
        }
        match arg.as_str() {
            "-c" | "--compact-output" => style.layout = Layout::Compact,
            "-n" | "--null-input" => null_input = true,
            "--tab" => style.layout = Layout::Pretty(Indent::Tabs),
            "-r" | "--raw-output" => style.raw = true,
            "-j" | "--join-output" => {
                style.join = true;
                style.raw = true;
            }
            "-a" | "--ascii-output" => style.ascii = true,
            "-S" | "--sort-keys" => style.sort_keys = true,
            "-" => operands.push(arg.clone()),
            other if other.starts_with("--") => {
                return Err(Error::new(format!("unknown option: {other}")));
            }
            // `-nc` is options. `-1` and `-.a` are programs, because the
            // character after the dash is not a letter.
            other if is_option_cluster(other) => {
                apply_short_cluster(&mut style, &mut null_input, other)?;
            }
            _ => operands.push(arg.clone()),
        }
        index += 1;
    }
    if operands.is_empty() {
        // With no filter argument, jq applies `.`.
        return Ok(Options {
            style,
            null_input,
            program: ".".to_string(),
            files: Vec::new(),
        });
    }
    let program = operands.remove(0);
    Ok(Options {
        style,
        null_input,
        program,
        files: operands,
    })
}

fn apply_null(style: Style, program: &tumulus::Filter, out: &mut dyn Write) -> Result<i32, Error> {
    match eval(program, &tumulus::Value::Null) {
        Ok(values) => {
            write_values(&values, style, out)?;
            Ok(0)
        }
        Err(failure) => {
            write_values(&failure.partial, style, out)?;
            eprintln!("jq: error (at <unknown>): {}", failure.message);
            Ok(5)
        }
    }
}

fn is_option_cluster(arg: &str) -> bool {
    let mut chars = arg.chars();
    chars.next() == Some('-') && chars.next().is_some_and(|ch| ch.is_ascii_alphabetic())
}

fn apply_short_cluster(style: &mut Style, null_input: &mut bool, arg: &str) -> Result<(), Error> {
    for flag in arg.chars().skip(1) {
        match flag {
            'c' => style.layout = Layout::Compact,
            'n' => *null_input = true,
            'r' => style.raw = true,
            'j' => {
                style.join = true;
                style.raw = true;
            }
            'a' => style.ascii = true,
            'S' => style.sort_keys = true,
            _ => return Err(Error::new(format!("unknown option: {arg}"))),
        }
    }
    Ok(())
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
