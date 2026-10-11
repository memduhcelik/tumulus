use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::process;

use tumulus::{write_input, Error};

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
        write_input(&read_stdin()?, options.compact, out)?;
        return Ok(0);
    }
    let mut missing = false;
    for path in &options.files {
        match read_input(path) {
            Ok(bytes) => write_input(&bytes, options.compact, out)?,
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
    compact: bool,
    files: Vec<String>,
}

fn options() -> Result<Options, Error> {
    let mut compact = false;
    let mut operands = Vec::new();
    let mut end = false;
    for arg in env::args().skip(1) {
        if end {
            operands.push(arg);
            continue;
        }
        match arg.as_str() {
            "--" => end = true,
            "-c" | "--compact-output" => compact = true,
            "-" => operands.push(arg),
            other if other.starts_with('-') => {
                return Err(Error::new(format!("unknown option: {other}")));
            }
            _ => operands.push(arg),
        }
    }
    if operands.is_empty() {
        // With no filter argument, jq applies `.`.
        return Ok(Options {
            compact,
            files: Vec::new(),
        });
    }
    if operands[0] != "." {
        return Err(Error::new(format!("unsupported filter: {}", operands[0])));
    }
    operands.remove(0);
    Ok(Options {
        compact,
        files: operands,
    })
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
