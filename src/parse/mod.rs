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
/// A leading UTF-8 BOM is ignored. Numbers are rewritten the way jq rewrites them.
/// When the tail is truncated, values before that tail are still written.
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
    let bytes = strip_bom(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| Error::new("input is not utf-8"))?;
    let mut parser = Parser::new(text);
    let mut parsed = Vec::new();
    loop {
        parser.skip_ws();
        if parser.eof() {
            break;
        }
        let start_line = parser.line;
        let value = match parser.parse_value() {
            Ok(value) => value,
            Err(err) => {
                return Ok(ParsedStream {
                    values: locate(text, parsed),
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
                    values: locate(text, parsed),
                    error: Some(parser.fail(phrase, false).text()),
                });
            }
        }
        parsed.push((value, start_line, end));
    }
    Ok(ParsedStream {
        values: locate(text, parsed),
        error: None,
    })
}

pub(crate) fn parse_json_string_literal(text: &str) -> Result<(String, usize), Error> {
    let mut parser = Parser::new(text);
    let value = parser
        .parse_string()
        .map_err(|_| Error::new("bad string"))?;
    Ok((value, parser.index))
}

fn locate(text: &str, parsed: Vec<(Value, usize, usize)>) -> Vec<LocatedValue> {
    let last = parsed.len();
    let mut out = Vec::with_capacity(last);
    for (index, (value, start_line, end)) in parsed.into_iter().enumerate() {
        let newline_after = text[end..].contains('\n');
        let line = if index + 1 == last && !newline_after {
            start_line.saturating_sub(1)
        } else {
            start_line
        };
        out.push(LocatedValue { value, line });
    }
    out
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    }
}

fn is_atom(value: &Value) -> bool {
    matches!(value, Value::Null | Value::Bool(_) | Value::Number(_))
}
