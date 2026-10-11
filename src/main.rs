use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::process;

use tumulus::{write_input, Error, Indent, Layout, Style};

fn main() {
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

fn run(out: &mut dyn Write) -> Result<i32, Error> {
    let options = options()?;
    if options.files.is_empty() {
        write_input(&read_stdin()?, options.style, out)?;
        return Ok(0);
    }
    let mut missing = false;
    for path in &options.files {
        match read_input(path) {
            Ok(bytes) => write_input(&bytes, options.style, out)?,
            Err(err) if path != "-" && err.kind() == io::ErrorKind::NotFound => {
                // jq names the missing file and keeps going.
                eprintln!("jq: error: Could not open file {path}: {}", os_reason(&err));
                missing = true;
            }
            Err(err) => return Err(Error::new(format!("cannot read {path}: {err}"))),
        }
    }
    Ok(if missing { 2 } else { 0 })
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
    files: Vec<String>,
}

fn options() -> Result<Options, Error> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut style = Style::default();
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
            other if other.starts_with('-') => {
                apply_short_cluster(&mut style, other)?;
            }
            _ => operands.push(arg.clone()),
        }
        index += 1;
    }
    if operands.is_empty() {
        // With no filter argument, jq applies `.`.
        return Ok(Options {
            style,
            files: Vec::new(),
        });
    }
    if operands[0] != "." {
        return Err(Error::new(format!("unsupported filter: {}", operands[0])));
    }
    operands.remove(0);
    Ok(Options {
        style,
        files: operands,
    })
}

fn apply_short_cluster(style: &mut Style, arg: &str) -> Result<(), Error> {
    for flag in arg.chars().skip(1) {
        match flag {
            'c' => style.layout = Layout::Compact,
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
