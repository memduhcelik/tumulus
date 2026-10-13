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
    let mut parser = Parser::new(text);
    let mut parsed = Vec::new();
    loop {
        parser.skip_ws();
        if parser.eof() {
            break;
        }
        let value = match parser.parse_value() {
            Ok(value) => value,
            Err(err) => {
                return Ok(ParsedStream {
                    values: locate(bytes, bom, parsed),
                    error: Some(err.text()),
                });
            }
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
                return Ok(ParsedStream {
                    values: locate(bytes, bom, parsed),
                    error: Some(parser.fail(phrase, false).text()),
                });
            }
        }
        parsed.push((value, end));
    }
    Ok(ParsedStream {
        values: locate(bytes, bom, parsed),
        error: None,
    })
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
