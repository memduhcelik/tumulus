use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::process;

use tumulus::{write_input, Error};

fn main() {
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    if let Err(err) = run(&mut out) {
        let _ = out.flush();
        eprintln!("tumulus: {err}");
        process::exit(1);
    }
    if let Err(err) = out.flush() {
        eprintln!("tumulus: {err}");
        process::exit(1);
    }
}

fn run(out: &mut dyn Write) -> Result<(), Error> {
    let options = options()?;
    if options.files.is_empty() {
        write_input(&read_stdin()?, options.compact, out)?;
        return Ok(());
    }
    for path in &options.files {
        let bytes = read_input(path)?;
        write_input(&bytes, options.compact, out)?;
    }
    Ok(())
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

fn read_stdin() -> Result<Vec<u8>, Error> {
    let mut buf = Vec::new();
    io::stdin().read_to_end(&mut buf)?;
    Ok(buf)
}

fn read_input(path: &str) -> Result<Vec<u8>, Error> {
    if path == "-" {
        return read_stdin();
    }
    fs::read(path).map_err(|err| Error::new(format!("cannot read {path}: {err}")))
}
