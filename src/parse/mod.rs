//! Read a JSON text into values, and report a jq parse error for a bad tail.

mod atom;
mod parser;
mod stream;
mod string;
mod value;

use std::io::Write;

use crate::error::Error;
use crate::print::{self, Style};
use crate::value::Value;

use parser::Parser;

pub(crate) use stream::{closer_is_detached, pull_stream, StreamState};
pub use stream::{program_stream, program_stream_with, StreamFeed, StreamValue};

/// Bytes that are not UTF-8 become U+FFFD, the way a JSON string does.
pub(crate) fn repair_utf8(bytes: &[u8]) -> String {
    parser::sanitize_content(bytes)
}

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

/// macOS jq ends the input at a NUL that has no newline after it.
/// Linux jq keeps the later bytes; the NUL only ends that token.
pub(crate) fn nul_cut(bytes: &[u8]) -> usize {
    #[cfg(target_os = "macos")]
    {
        match bytes.iter().rposition(|byte| *byte == 0) {
            Some(index) if !bytes[index + 1..].contains(&b'\n') => index,
            _ => bytes.len(),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        bytes.len()
    }
}

/// One JSON text from a `--seq` input, or an error jq ignores and keeps reading.
pub enum SeqItem {
    Value(LocatedValue),
    Error(String),
}

/// `application/json-seq`. Bytes before the first ASCII RS are skipped.
/// A value cut off by RS is dropped. An input with no RS is abandoned text.
pub fn parse_seq(bytes: &[u8]) -> Vec<SeqItem> {
    let mut items = Vec::new();
    let mut index = 0;
    let mut line = 1usize;
    let mut column = 0usize;
    while index < bytes.len() && bytes[index] != b'\x1e' {
        step(&mut line, &mut column, bytes[index]);
        index += 1;
    }
    if index == bytes.len() {
        items.push(SeqItem::Error(format!(
            "Unfinished abandoned text at EOF at line {line}, column {column}"
        )));
        return items;
    }
    let mut saw_newline = false;
    while index < bytes.len() {
        // The RS itself.
        step(&mut line, &mut column, bytes[index]);
        index += 1;
        let start = index;
        let start_line = line;
        let start_column = column;
        let mut chunk_newline = false;
        while index < bytes.len() && bytes[index] != b'\x1e' {
            if bytes[index] == b'\n' {
                chunk_newline = true;
            }
            step(&mut line, &mut column, bytes[index]);
            index += 1;
        }
        let ended_by_rs = index < bytes.len() && bytes[index] == b'\x1e';
        let chunk = &bytes[start..index];
        // A record with no newline, after an earlier newline, is discarded.
        if saw_newline && !chunk_newline {
            continue;
        }
        saw_newline |= chunk_newline;
        push_chunk(
            &mut items,
            chunk,
            start_line,
            start_column,
            ended_by_rs,
            line,
            column,
        );
    }
    items
}

fn step(line: &mut usize, column: &mut usize, byte: u8) {
    if byte == b'\n' {
        *line += 1;
        *column = 0;
    } else {
        *column += 1;
    }
}

fn push_chunk(
    items: &mut Vec<SeqItem>,
    chunk: &[u8],
    start_line: usize,
    start_column: usize,
    ended_by_rs: bool,
    end_line: usize,
    end_column: usize,
) {
    let mut offset = 0usize;
    let mut line = start_line;
    let mut column = start_column;
    while offset < chunk.len() {
        let scanned = scan_text_at(&chunk[offset..], line, column);
        let base = offset;
        let mut values = scanned.values;
        let at_end = base
            + scanned
                .error
                .as_ref()
                .map(|_| scanned.error_at)
                .unwrap_or(chunk.len() - base)
            >= chunk.len()
            || scanned.error.is_none();
        let slice_end = chunk.len() - base;
        let trailing_ws = matches!(chunk[base..].last(), Some(b' ' | b'\t' | b'\n' | b'\r'));
        if scanned.error.is_none() && !trailing_ws {
            if let Some((value, end)) = values.last() {
                let end = *end;
                if end == slice_end
                    && is_atom(value)
                    && (ended_by_rs || matches!(value, Value::Number(_)))
                {
                    let value = value.clone();
                    values.pop();
                    push_kept(items, &values, &chunk[base..], line);
                    if ended_by_rs {
                        let column = end_column + 1;
                        let phrase = if matches!(value, Value::Number(_)) {
                            format!(
                                "Potentially truncated top-level numeric value at line {end_line}, column {column}"
                            )
                        } else {
                            format!("Truncated value at line {end_line}, column {column}")
                        };
                        items.push(SeqItem::Error(phrase));
                    } else if matches!(value, Value::Number(_)) {
                        items.push(SeqItem::Error(format!(
                            "Potentially truncated top-level numeric value at EOF at line {end_line}, column {end_column}"
                        )));
                    } else {
                        push_kept(items, &[(value, end)], &chunk[base..], line);
                    }
                    return;
                }
            }
        }
        push_kept(items, &values, &chunk[base..], line);
        let Some(err) = scanned.error else {
            return;
        };
        let error_line = err.line();
        let message = err.text();
        if ended_by_rs && at_end && message.contains(" at EOF ") {
            if chunk.contains(&b'\n') {
                return;
            }
            let column = end_column + 1;
            items.push(SeqItem::Error(format!(
                "Truncated value at line {end_line}, column {column}"
            )));
            return;
        }
        if message.contains(" at EOF ") {
            items.push(SeqItem::Error(message));
            return;
        }
        items.push(SeqItem::Error(format!("{message} (need RS to resync)")));
        // jq names the following line as well when the bad token did not
        // already end there, then keeps parsing.
        if scanned.resume_line > error_line && base + scanned.resume < chunk.len() {
            items.push(SeqItem::Error(format!(
                "Invalid numeric literal at line {}, column 0 (need RS to resync)",
                scanned.resume_line
            )));
        }
        let resume = scanned.resume;
        if resume == 0 {
            return;
        }
        offset = base + resume;
        line = scanned.resume_line;
        column = 0;
        if offset >= chunk.len() {
            return;
        }
    }
}

fn push_kept(items: &mut Vec<SeqItem>, values: &[(Value, usize)], chunk: &[u8], start_line: usize) {
    for (value, end) in values {
        let line = start_line + chunk[..*end].iter().filter(|byte| **byte == b'\n').count();
        items.push(SeqItem::Value(LocatedValue {
            value: value.clone(),
            line,
        }));
    }
}

fn scan_text_at(text: &[u8], line: usize, column: usize) -> Scanned {
    let mut parser = Parser::new(text);
    parser.seek_at(0, line, column);
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

pub fn parse_stream(bytes: &[u8]) -> Result<ParsedStream, Error> {
    let bytes = &bytes[..nul_cut(bytes)];
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
    let bytes = &bytes[..nul_cut(bytes)];
    let start = start.min(bytes.len());
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
pub(in crate::parse) fn skip_rest_of_line(
    text: &[u8],
    mut index: usize,
    mut line: usize,
) -> (usize, usize) {
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
    LineCursor::default().line_at(text, end, atom)
}

/// Running total of newlines jq's reader has consumed.
///
/// A later request continues from the last read when the byte it needs is
/// not before that read. A request that finishes earlier starts over, so a
/// newline that belongs only to a later read is not kept.
#[derive(Clone, Copy, Default)]
pub(crate) struct LineCursor {
    filled: usize,
    line: usize,
    /// Start of the last read included in `line`.
    last_start: usize,
}

impl LineCursor {
    /// The line jq would report is fixed. More bytes would not change it,
    /// or the input has already ended.
    pub(crate) fn ready(&self, text: &[u8], end: usize, atom: bool, eof: bool) -> bool {
        if atom && end >= text.len() && !eof {
            return false;
        }
        let need = if atom && end < text.len() {
            end
        } else {
            end.saturating_sub(1)
        };
        let mut filled = self.filled;
        let mut last_start = self.last_start;
        if filled > 0 && need < last_start {
            filled = 0;
            last_start = 0;
        }
        let _ = last_start;
        while filled <= need {
            if filled >= text.len() {
                return eof;
            }
            let start = filled;
            let limit = start.saturating_add(FGETS_MAX).min(text.len());
            let mut next = start;
            let mut at_newline = false;
            while next < limit {
                next += 1;
                if text[next - 1] == b'\n' {
                    at_newline = true;
                    break;
                }
            }
            if next == start {
                return eof;
            }
            if next == text.len() && !at_newline && !eof && next - start < FGETS_MAX {
                return false;
            }
            filled = next;
        }
        true
    }

    pub(crate) fn line_at(&mut self, text: &[u8], end: usize, atom: bool) -> usize {
        let need = if atom && end < text.len() {
            end
        } else {
            end.saturating_sub(1)
        };
        if self.filled > 0 && need < self.last_start {
            *self = Self::default();
        }
        while self.filled <= need && self.filled < text.len() {
            let start = self.filled;
            let next = read_chunk(text, start);
            self.line += newline_count(&text[start..next]);
            if next == start {
                break;
            }
            self.last_start = start;
            self.filled = next;
        }
        self.line
    }
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
    let mut cursor = LineCursor::default();
    let mut out = Vec::with_capacity(parsed.len());
    for (value, end) in parsed {
        // `end` is the next byte in the text that was parsed. The reader
        // also counts the leading BOM, which that text skipped.
        let line = cursor.line_at(text, end + bom, is_atom(&value));
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

/// The value ending at `end` was completed in the last read of `bytes`, and
/// that read did not end on a newline.
///
/// `end` is the first byte after the value, or `bytes.len()` when the value
/// runs to the end of the file. A read that ends on a newline does not count.
/// A full 4095-byte read with more of the file after it does not count either.
pub(crate) fn finished_on_eof_cut(bytes: &[u8], end: usize) -> bool {
    if bytes.is_empty() || end == 0 {
        return false;
    }
    let pos = if end < bytes.len() {
        end
    } else {
        bytes.len() - 1
    };
    let mut at = 0;
    while at < bytes.len() {
        let next = read_chunk(bytes, at);
        if next == at {
            break;
        }
        if pos < next {
            let saw_nl = bytes[next - 1] == b'\n';
            return next == bytes.len() && !saw_nl;
        }
        at = next;
    }
    false
}

fn newline_count(text: &[u8]) -> usize {
    text.iter().filter(|byte| **byte == b'\n').count()
}

pub(in crate::parse) fn split_bom(bytes: &[u8]) -> (usize, &[u8]) {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        (3, &bytes[3..])
    } else {
        (0, bytes)
    }
}

fn is_atom(value: &Value) -> bool {
    matches!(value, Value::Null | Value::Bool(_) | Value::Number(_))
}

#[cfg(test)]
mod tests {
    use super::{finished_on_eof_cut, stream_line, LineCursor};

    fn bytes(len: usize, newline_every: usize) -> Vec<u8> {
        let mut text = vec![b'a'; len];
        if newline_every > 0 {
            let mut at = newline_every - 1;
            while at < text.len() {
                text[at] = b'\n';
                at += newline_every;
            }
        }
        text
    }

    fn ends_for(len: usize) -> Vec<usize> {
        let step = (len / 40).max(1);
        let mut ends: Vec<usize> = (0..=len).step_by(step).collect();
        for mark in [0, 1, 4094, 4095, 4096, 8190, 8191, len] {
            if mark <= len {
                ends.push(mark);
            }
        }
        ends.sort_unstable();
        ends.dedup();
        ends
    }

    #[test]
    fn cursor_matches_a_fresh_count() {
        let mut texts = vec![
            bytes(0, 1),
            bytes(1, 1),
            bytes(10, 1),
            bytes(4095, 10),
            bytes(4096, 4095),
            bytes(8190, 100),
            bytes(9000, 4095),
            bytes(12000, 3),
            b"\n\n\n".to_vec(),
            b"no-newline".to_vec(),
            vec![0xEF, 0xBB, 0xBF, b'1', b'\n'],
        ];
        // A newline on the first byte of a later read, then a request that
        // stops before that read starts.
        let mut boundary = vec![b'x'; 4095];
        boundary.push(b'\n');
        boundary.extend(std::iter::repeat(b'y').take(20));
        texts.push(boundary);

        for text in &texts {
            for atom in [false, true] {
                let mut cursor = LineCursor::default();
                for end in ends_for(text.len()) {
                    let got = cursor.line_at(text, end, atom);
                    let fresh = stream_line(text, end, atom);
                    assert_eq!(got, fresh, "end {end} atom {atom} len {}", text.len());
                    assert_eq!(cursor.line_at(text, end, atom), fresh);
                }
                if text.len() > 4096 {
                    let back = cursor.line_at(text, 10, !atom);
                    assert_eq!(back, stream_line(text, 10, !atom));
                    let again = cursor.line_at(text, text.len(), atom);
                    assert_eq!(again, stream_line(text, text.len(), atom));
                }
            }
        }
    }

    #[test]
    fn eof_cut_is_only_the_last_short_read() {
        assert!(finished_on_eof_cut(b"1\n2\n3", 5));
        assert!(!finished_on_eof_cut(b"1\n2\n3\n", 5));
        assert!(finished_on_eof_cut(b"1\n2\n3 ", 5));
        assert!(!finished_on_eof_cut(b"1\n2\n3\n ", 5));
        assert!(!finished_on_eof_cut(b"1\n2\n   ", 3));
        assert!(!finished_on_eof_cut(b"\n", 1));
        assert!(finished_on_eof_cut(b"3", 1));
        assert!(!finished_on_eof_cut(b"1\n", 1));
        let mut long = vec![b'1'; 4095];
        long.push(b' ');
        assert!(!finished_on_eof_cut(&long, 1));
        assert!(finished_on_eof_cut(&long, 4095));
        long.push(b'\n');
        assert!(!finished_on_eof_cut(&long, 4095));
    }
}
