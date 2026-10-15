//! Read a JSON text into values, and report a jq parse error for a bad tail.

mod atom;
mod parser;
mod string;
mod value;

use std::io::Write;

use crate::error::Error;
use crate::print::{self, Style};
use crate::value::Value;

use parser::Parser;

/// One JSON value and the line jq would name in a runtime error.
pub struct LocatedValue {
    pub value: Value,
    /// Line number jq prints in a runtime error for this value.
    pub line: usize,
}

/// Values read from one input, plus a parse error if the tail is not valid JSON.
///
/// Complete values that ended before the bad token are kept. jq prints those
/// before it reports the parse error.
pub struct ParsedStream {
    pub values: Vec<LocatedValue>,
    pub error: Option<String>,
}

/// Parse every JSON value in `bytes` and write jq's layout for each one.
///
/// A leading UTF-8 BOM is ignored. Bytes that are not valid UTF-8 inside a
/// string become U+FFFD, and parsing continues. Numbers are rewritten the way
/// jq rewrites them. When the tail is truncated, values before that tail are
/// still written.
pub fn write_input(bytes: &[u8], style: Style, out: &mut dyn Write) -> Result<(), Error> {
    let parsed = parse_stream(bytes)?;
    for item in &parsed.values {
        let rendered = print::render(&item.value, style);
        out.write_all(rendered.as_bytes())?;
        if !style.join {
            out.write_all(b"\n")?;
        }
    }
    if let Some(message) = parsed.error {
        return Err(Error::new(message));
    }
    Ok(())
}

pub fn parse_stream(bytes: &[u8]) -> Result<ParsedStream, Error> {
    let (bom, text) = split_bom(bytes);
    let scanned = scan_text(text, 0, 1);
    Ok(ParsedStream {
        values: locate(bytes, bom, scanned.values),
        error: scanned.error.map(|err| err.text()),
    })
}

/// One pass over `bytes` starting at `start`, which is an index into the
/// original buffer. `line` is the parser line already reached.
///
/// `resume` is where a later pass should start after a hard error. An
/// `incomplete` error still needs more bytes; `restart` is the start of
/// the value that ran out.
pub struct InputScan {
    pub values: Vec<(Value, usize)>,
    pub error: Option<String>,
    pub incomplete: bool,
    /// Absolute index of the byte the error was noticed on.
    pub error_at: usize,
    /// Absolute index to continue from after a hard error.
    pub resume: usize,
    /// Parser line at `resume`, or at `restart` when the value is unfinished.
    pub resume_line: usize,
    /// Absolute index of an unfinished value. More bytes are parsed again
    /// from here, so the values before it are not produced twice.
    pub restart: usize,
    pub restart_line: usize,
}

pub fn scan_input(bytes: &[u8], start: usize, line: usize) -> InputScan {
    let bom = if start == 0 && bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        0
    };
    let origin = if start == 0 { bom } else { start };
    let scanned = scan_text(&bytes[bom..], origin - bom, line);
    let shift = |index: usize| index + bom;
    InputScan {
        values: scanned
            .values
            .into_iter()
            .map(|(value, end)| (value, shift(end)))
            .collect(),
        error: scanned.error.as_ref().map(parser::ParseErr::text),
        incomplete: scanned.incomplete,
        error_at: shift(scanned.error_at),
        resume: shift(scanned.resume),
        resume_line: scanned.resume_line,
        restart: shift(scanned.restart),
        restart_line: scanned.restart_line,
    }
}

struct Scanned {
    values: Vec<(Value, usize)>,
    error: Option<parser::ParseErr>,
    incomplete: bool,
    error_at: usize,
    resume: usize,
    resume_line: usize,
    restart: usize,
    restart_line: usize,
}

fn scan_text(text: &[u8], start: usize, line: usize) -> Scanned {
    let mut parser = Parser::new(text);
    parser.seek(start, line);
    let mut values = Vec::new();
    loop {
        parser.skip_ws();
        if parser.eof() {
            return Scanned {
                values,
                error: None,
                incomplete: false,
                error_at: parser.index,
                resume: parser.index,
                resume_line: parser.line,
                restart: parser.index,
                restart_line: parser.line,
            };
        }
        let restart = parser.index;
        let restart_line = parser.line;
        let value = match parser.parse_value() {
            Ok(value) => value,
            Err(err) => return stop(values, err, &parser, restart, restart_line),
        };
        let end = parser.index;
        // A number or literal glued to ] } , : is not a finished value.
        // jq ends the token on that character and then rejects it.
        if is_atom(&value) {
            let phrase = match parser.peek() {
                Some(']') => Some("Unmatched ']'"),
                Some('}') => Some("Unmatched '}'"),
                Some(',') => Some("Expected value before ','"),
                Some(':') => Some("Expected string key before ':'"),
                _ => None,
            };
            if let Some(phrase) = phrase {
                parser.bump();
                let err = parser.fail(phrase, false);
                return stop(values, err, &parser, restart, restart_line);
            }
        }
        values.push((value, end));
    }
}

fn stop(
    values: Vec<(Value, usize)>,
    err: parser::ParseErr,
    parser: &Parser,
    restart: usize,
    restart_line: usize,
) -> Scanned {
    let incomplete = err.ran_out();
    let (resume, resume_line) = if incomplete || parser.column() == 0 {
        // A column of 0 means the bad token already consumed its newline.
        (parser.index, parser.line)
    } else {
        skip_rest_of_line(parser.src, parser.index, parser.line)
    };
    Scanned {
        values,
        error: Some(err),
        incomplete,
        error_at: parser.index,
        resume,
        resume_line,
        restart,
        restart_line,
    }
}

/// The next value after a hard error starts on the following line.
fn skip_rest_of_line(text: &[u8], mut index: usize, mut line: usize) -> (usize, usize) {
    while index < text.len() && text[index] != b'\n' {
        index += 1;
    }
    if index < text.len() && text[index] == b'\n' {
        index += 1;
        line += 1;
    }
    (index, line)
}

/// Newlines jq's reader has consumed to reach `end`.
///
/// `end` is the next byte after a value, in `text`. An atom is not finished
/// until that next byte has been read. One read is at most 4095 bytes and
/// stops at a newline, so a newline later in that chunk still counts.
pub fn stream_line(text: &[u8], end: usize, atom: bool) -> usize {
    let need = if atom && end < text.len() {
        end
    } else {
        end.saturating_sub(1)
    };
    let mut line = 0usize;
    let mut filled = 0usize;
    while filled <= need && filled < text.len() {
        let next = read_chunk(text, filled);
        line += newline_count(&text[filled..next]);
        if next == filled {
            break;
        }
        filled = next;
    }
    line
}

/// Decode one JSON string token, or jq's message for a bad escape.
pub(crate) fn decode_string_token(text: &str) -> Result<String, String> {
    let mut parser = Parser::new(text.as_bytes());
    match parser.parse_string() {
        Ok(value) => Ok(value),
        Err(err) => Err(format!("{} (while parsing '{text}')", err.text())),
    }
}

// jq reads with `fgets` into a 4096-byte buffer, so one read is at most
// 4095 bytes and stops at a newline. A runtime error names how many
// newlines that reader has pulled in when the value is finished. A
// number or literal is not finished until the next byte is in. The BOM
// is not parsed, and its bytes still count toward that read.
const FGETS_MAX: usize = 4095;

fn locate(text: &[u8], bom: usize, parsed: Vec<(Value, usize)>) -> Vec<LocatedValue> {
    let mut line = 0usize;
    let mut filled = 0usize;
    let mut out = Vec::with_capacity(parsed.len());
    for (value, end) in parsed {
        let end = end + bom;
        // `end` is the next byte. Atoms have not consumed it; other
        // values finish on the byte just before it.
        let need = if is_atom(&value) && end < text.len() {
            end
        } else {
            end.saturating_sub(1)
        };
        while filled <= need && filled < text.len() {
            let next = read_chunk(text, filled);
            line += newline_count(&text[filled..next]);
            if next == filled {
                break;
            }
            filled = next;
        }
        out.push(LocatedValue { value, line });
    }
    out
}

fn read_chunk(text: &[u8], start: usize) -> usize {
    let limit = start.saturating_add(FGETS_MAX).min(text.len());
    let mut end = start;
    while end < limit {
        end += 1;
        if text[end - 1] == b'\n' {
            break;
        }
    }
    end
}

fn newline_count(text: &[u8]) -> usize {
    text.iter().filter(|byte| **byte == b'\n').count()
}

fn split_bom(bytes: &[u8]) -> (usize, &[u8]) {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (3, &bytes[3..])
    } else {
        (0, bytes)
    }
}

fn is_atom(value: &Value) -> bool {
    matches!(value, Value::Null | Value::Bool(_) | Value::Number(_))
}
