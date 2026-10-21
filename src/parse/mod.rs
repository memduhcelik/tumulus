//! Read a JSON text into values, and report a jq parse error for a bad tail.

mod atom;
mod parser;
mod stream;
mod string;
mod value;

use std::io::Write;

#[cfg(test)]
use std::cell::Cell;

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
    /// Byte just after this value in the buffer that was parsed. `0` when
    /// the caller does not need a file boundary.
    pub end: usize,
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
    // Tests count every byte this search is asked to walk. `--stream` used to
    // ask for the whole file again on each read.
    #[cfg(test)]
    NUL_CUT_BYTES.with(|cell| {
        cell.set(cell.get().saturating_add(bytes.len() as u64));
    });
    #[cfg(target_os = "macos")]
    {
        nul_cut_at(bytes)
    }
    #[cfg(not(target_os = "macos"))]
    {
        bytes.len()
    }
}

/// Where a repeated NUL check can resume.
///
/// The bytes already searched stay a prefix. A later check looks only at
/// the new tail, so a file with no NUL is not reread once per chunk.
#[derive(Clone, Copy, Default)]
pub(crate) struct NulScan {
    checked: usize,
    last_nul: Option<usize>,
    /// A newline has been seen after `last_nul` in the checked prefix.
    nl_after: bool,
}

impl NulScan {
    /// The index [`nul_cut`] would return on macOS for this buffer.
    pub(crate) fn at(&mut self, bytes: &[u8]) -> usize {
        if self.checked > bytes.len() {
            *self = Self::default();
        }
        if self.checked < bytes.len() {
            let start = self.checked;
            if let Some(rel) = bytes[start..].iter().rposition(|byte| *byte == 0) {
                let index = start + rel;
                self.last_nul = Some(index);
                self.nl_after = bytes[index + 1..].contains(&b'\n');
            } else if self.last_nul.is_some() && !self.nl_after {
                self.nl_after = bytes[start..].contains(&b'\n');
            }
            self.checked = bytes.len();
        }
        match self.last_nul {
            Some(index) if !self.nl_after => index,
            _ => bytes.len(),
        }
    }

    /// [`nul_cut`] for this buffer, continued from the previous call.
    pub(crate) fn cut(&mut self, bytes: &[u8]) -> usize {
        let at = self.at(bytes);
        #[cfg(target_os = "macos")]
        {
            at
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = at;
            bytes.len()
        }
    }
}

#[cfg(test)]
thread_local! {
    static NUL_CUT_BYTES: Cell<u64> = const { Cell::new(0) };
}

/// Bytes [`nul_cut`] has been asked to walk on this thread since the last reset.
#[cfg(test)]
pub(crate) fn nul_cut_bytes() -> u64 {
    NUL_CUT_BYTES.with(|cell| cell.get())
}

/// Start a fresh count for [`nul_cut_bytes`].
#[cfg(test)]
pub(crate) fn reset_nul_cut_bytes() {
    NUL_CUT_BYTES.with(|cell| cell.set(0));
}

fn nul_cut_at(bytes: &[u8]) -> usize {
    match bytes.iter().rposition(|byte| *byte == 0) {
        Some(index) if !bytes[index + 1..].contains(&b'\n') => index,
        _ => bytes.len(),
    }
}

/// One JSON text from a `--seq` input, or an error jq ignores and keeps reading.
pub enum SeqItem {
    Value(LocatedValue),
    Error(SeqFault),
}

/// A `--seq` parse error. `at` is the byte in the buffer where it was noticed.
pub struct SeqFault {
    pub message: String,
    pub at: usize,
}

fn seq_fault(message: impl Into<String>, at: usize) -> SeqItem {
    SeqItem::Error(SeqFault {
        message: message.into(),
        at,
    })
}

/// `application/json-seq`. Bytes before the first ASCII RS are skipped.
/// A value cut off by the next separator is reported. An input with no RS
/// is abandoned text. A final record with no newline, after one that had
/// a newline, is dropped unless the caller is slurping.
pub fn parse_seq(bytes: &[u8]) -> Vec<SeqItem> {
    // A record with no newline is dropped once an earlier record had one,
    // unless the caller is slurping. `-s` reports that record instead.
    parse_seq_keep(bytes, false)
}

pub(crate) fn parse_seq_keep(bytes: &[u8], keep_short: bool) -> Vec<SeqItem> {
    let mut items = Vec::new();
    let mut index = 0;
    let mut line = 1usize;
    let mut column = 0usize;
    while index < bytes.len() && bytes[index] != b'\x1e' {
        step(&mut line, &mut column, bytes[index]);
        index += 1;
    }
    if index == bytes.len() {
        items.push(seq_fault(
            format!("Unfinished abandoned text at EOF at line {line}, column {column}"),
            index,
        ));
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
        // A record with no newline is dropped only at the real end, after
        // an earlier record that had one. One cut off by the next separator
        // is reported.
        if saw_newline && !chunk_newline && !keep_short && !ended_by_rs {
            continue;
        }
        saw_newline |= chunk_newline;
        push_chunk(
            &mut items,
            chunk,
            start,
            start_line,
            start_column,
            ended_by_rs,
            line,
            column,
        );
    }
    items
}

/// One RS-separated record. `start` is the first byte after that RS.
pub(crate) struct SeqRecord {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
    pub ended_by_rs: bool,
    pub end_line: usize,
    pub end_column: usize,
}

pub(crate) enum SeqLayout {
    Abandoned { message: String, at: usize },
    Records(Vec<SeqRecord>),
}

/// The same record split `parse_seq` uses. A final record with no newline,
/// after one that had a newline, is left out unless `keep_short` is set.
pub(crate) fn seq_layout_keep(bytes: &[u8], keep_short: bool) -> SeqLayout {
    let mut index = 0;
    let mut line = 1usize;
    let mut column = 0usize;
    while index < bytes.len() && bytes[index] != b'\x1e' {
        step(&mut line, &mut column, bytes[index]);
        index += 1;
    }
    if index == bytes.len() {
        return SeqLayout::Abandoned {
            message: format!("Unfinished abandoned text at EOF at line {line}, column {column}"),
            at: index,
        };
    }
    let mut records = Vec::new();
    let mut saw_newline = false;
    while index < bytes.len() {
        step(&mut line, &mut column, bytes[index]);
        index += 1;
        let start = index;
        let start_line = line;
        let start_column = column;
        let mut has_newline = false;
        while index < bytes.len() && bytes[index] != b'\x1e' {
            if bytes[index] == b'\n' {
                has_newline = true;
            }
            step(&mut line, &mut column, bytes[index]);
            index += 1;
        }
        let ended_by_rs = index < bytes.len() && bytes[index] == b'\x1e';
        if saw_newline && !has_newline && !keep_short && !ended_by_rs {
            continue;
        }
        saw_newline |= has_newline;
        records.push(SeqRecord {
            start,
            end: index,
            line: start_line,
            column: start_column,
            ended_by_rs,
            end_line: line,
            end_column: column,
        });
    }
    SeqLayout::Records(records)
}

/// One value `--stream` produced from a `--seq` record, or a warning.
pub(crate) enum SeqBit {
    Item(SeqStreamItem),
    Soft {
        message: String,
        at: usize,
        unknown: bool,
    },
}

pub(crate) struct SeqStreamItem {
    pub value: Value,
    pub end: usize,
    pub atom: bool,
    pub failure: bool,
}

pub(crate) struct SeqStream {
    pub events: Vec<SeqBit>,
    /// The last failure is an unfinished value that ended on a comma, so
    /// `-s` emits it after the array.
    pub partial: bool,
}

/// `--stream` over application/json-seq. A cut-off number stays a value.
/// Other failures are warnings, or `[message, path]` when `errors` is set.
pub(crate) fn seq_stream_keep(bytes: &[u8], errors: bool, keep_short: bool) -> SeqStream {
    let mut events = Vec::new();
    let records = match seq_layout_keep(bytes, keep_short) {
        SeqLayout::Abandoned { message, at } => {
            if errors {
                events.push(SeqBit::Item(SeqStreamItem {
                    value: stream::error_value(&message, &[]),
                    end: at,
                    atom: false,
                    failure: true,
                }));
            } else {
                events.push(SeqBit::Soft {
                    message,
                    at,
                    unknown: false,
                });
            }
            return SeqStream {
                events,
                partial: false,
            };
        }
        SeqLayout::Records(records) => records,
    };
    let mut partial = false;
    for record in records {
        let chunk = &bytes[record.start..record.end];
        let bits = stream::scan_seq_record(
            chunk,
            record.line,
            record.column,
            record.ended_by_rs,
            record.end_line,
            record.end_column,
            errors,
        );
        for bit in bits {
            match bit {
                stream::RecEvent::Item {
                    value,
                    end,
                    atom,
                    failure,
                } => {
                    partial = failure
                        && chunk.ends_with(b",")
                        && value_message(&value)
                            .is_some_and(|text| text.starts_with("Unfinished JSON term at EOF"));
                    events.push(SeqBit::Item(SeqStreamItem {
                        value,
                        end: record.start + end,
                        atom,
                        failure,
                    }));
                }
                stream::RecEvent::Soft {
                    message,
                    at,
                    unknown,
                } => {
                    partial = false;
                    events.push(SeqBit::Soft {
                        message,
                        at: record.start + at,
                        unknown,
                    });
                }
            }
        }
    }
    // Only a failure that is still the last event splits off under `-s`.
    if partial {
        if let Some(SeqBit::Item(item)) = events.last() {
            partial = item.failure;
        } else {
            partial = false;
        }
    }
    SeqStream { events, partial }
}

fn value_message(value: &Value) -> Option<&str> {
    let Value::Array(items) = value else {
        return None;
    };
    let Some(Value::String(text)) = items.first() else {
        return None;
    };
    Some(text)
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
    origin: usize,
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
                    push_kept(items, &values, &chunk[base..], line, origin + base);
                    if ended_by_rs {
                        let column = end_column + 1;
                        let phrase = if matches!(value, Value::Number(_)) {
                            format!(
                                "Potentially truncated top-level numeric value at line {end_line}, column {column}"
                            )
                        } else {
                            format!("Truncated value at line {end_line}, column {column}")
                        };
                        // The separator that cut the value off is the noticed byte.
                        items.push(seq_fault(phrase, origin + chunk.len() + 1));
                    } else if matches!(value, Value::Number(_)) {
                        items.push(seq_fault(
                            format!(
                                "Potentially truncated top-level numeric value at EOF at line {end_line}, column {end_column}"
                            ),
                            origin + chunk.len(),
                        ));
                    } else {
                        push_kept(items, &[(value, end)], &chunk[base..], line, origin + base);
                    }
                    return;
                }
            }
        }
        push_kept(items, &values, &chunk[base..], line, origin + base);
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
            items.push(seq_fault(
                format!("Truncated value at line {end_line}, column {column}"),
                origin + chunk.len() + 1,
            ));
            return;
        }
        let at = origin + base + scanned.error_at;
        if message.contains(" at EOF ") {
            items.push(seq_fault(message, at));
            return;
        }
        items.push(seq_fault(format!("{message} (need RS to resync)"), at));
        // jq names the following line as well when the bad token did not
        // already end there, then keeps parsing.
        if scanned.resume_line > error_line && base + scanned.resume < chunk.len() {
            items.push(seq_fault(
                format!(
                    "Invalid numeric literal at line {}, column 0 (need RS to resync)",
                    scanned.resume_line
                ),
                origin + base + scanned.resume,
            ));
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

fn push_kept(
    items: &mut Vec<SeqItem>,
    values: &[(Value, usize)],
    chunk: &[u8],
    start_line: usize,
    origin: usize,
) {
    for (value, end) in values {
        let line = start_line + chunk[..*end].iter().filter(|byte| **byte == b'\n').count();
        items.push(SeqItem::Value(LocatedValue {
            value: value.clone(),
            line,
            end: origin + *end,
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
    let scanned = scan_text(text, 0, 1, false);
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

pub fn scan_input(bytes: &[u8], start: usize, line: usize, open_ended: bool) -> InputScan {
    let bytes = &bytes[..nul_cut(bytes)];
    let start = start.min(bytes.len());
    let bom = if start == 0 && bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        0
    };
    let origin = if start == 0 { bom } else { start };
    let scanned = scan_text(&bytes[bom..], origin - bom, line, open_ended);
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

fn scan_text(text: &[u8], start: usize, line: usize, open_ended: bool) -> Scanned {
    let mut parser = Parser::new(text);
    parser.open_ended = open_ended;
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
    pub(crate) fn ready(&mut self, text: &[u8], end: usize, atom: bool, eof: bool) -> bool {
        if atom && end >= text.len() && !eof {
            return false;
        }
        let need = if atom && end < text.len() {
            end
        } else {
            end.saturating_sub(1)
        };
        // This read already finished: more bytes follow it, so a later read
        // cannot change the line. The tail at the end of the buffer still
        // waits, because another read can lengthen it.
        if self.filled > need && need >= self.last_start && self.filled < text.len() {
            return true;
        }
        if self.filled > 0 && need < self.last_start {
            *self = Self::default();
        }
        while self.filled <= need {
            if self.filled >= text.len() {
                return eof;
            }
            let start = self.filled;
            let next = read_chunk(text, start);
            if next == start {
                return eof;
            }
            let saw_nl = text[next - 1] == b'\n';
            if next == text.len() && !saw_nl && !eof && next - start < FGETS_MAX {
                return false;
            }
            // A finished read contains at most one newline, and it is the
            // last byte. Counting the slice again walked the file per value.
            if saw_nl {
                self.line += 1;
            }
            self.last_start = start;
            self.filled = next;
        }
        true
    }

    /// `cut` bytes left the front of the buffer. `base_line` is the newline
    /// count of the reads that left with them.
    pub(crate) fn shift(&mut self, cut: usize, base_line: usize) {
        if cut == 0 {
            return;
        }
        if self.filled > cut {
            self.filled -= cut;
            if self.last_start >= cut {
                self.last_start -= cut;
            } else {
                self.last_start = 0;
            }
            return;
        }
        *self = Self {
            filled: 0,
            line: base_line,
            last_start: 0,
        };
    }

    pub(crate) fn line_at(&mut self, text: &[u8], end: usize, atom: bool) -> usize {
        let need = if atom && end < text.len() {
            end
        } else {
            end.saturating_sub(1)
        };
        // Values are stamped in order. A later value in the same read has
        // this line: jq counts every newline that read already consumed.
        if self.filled > need && need >= self.last_start {
            return self.line;
        }
        if self.filled > 0 && need < self.last_start {
            *self = Self::default();
        }
        while self.filled <= need && self.filled < text.len() {
            let start = self.filled;
            let next = read_chunk(text, start);
            if next > start && text[next - 1] == b'\n' {
                self.line += 1;
            }
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
        out.push(LocatedValue {
            value,
            line,
            end: 0,
        });
    }
    out
}

fn read_chunk(text: &[u8], start: usize) -> usize {
    let limit = start.saturating_add(FGETS_MAX).min(text.len());
    if start >= limit {
        return start;
    }
    match find_newline(&text[start..limit]) {
        Some(at) => start + at + 1,
        None => limit,
    }
}

/// fgets reads already finished in a buffer. A short tail with no newline is
/// left out so a later append can lengthen that same read.
#[derive(Clone, Default)]
pub(crate) struct Fgets {
    ends: Vec<usize>,
    lines: Vec<u32>,
    at: usize,
    /// Newlines in reads already dropped from the front of the buffer.
    base: u32,
}

impl Fgets {
    /// Record reads in `raw`. `eof` finishes a short tail that has no newline.
    /// Without it, that tail stays open so a later append can lengthen the
    /// same read.
    pub(crate) fn push(&mut self, raw: &[u8], eof: bool) {
        let mut at = self.ends.last().copied().unwrap_or(0);
        let mut line = self.lines.last().copied().unwrap_or(self.base);
        while at < raw.len() {
            let next = read_chunk(raw, at);
            if next == at {
                break;
            }
            let saw_nl = raw[next - 1] == b'\n';
            let full = next - at == FGETS_MAX;
            if next == raw.len() && !saw_nl && !full && !eof {
                break;
            }
            if saw_nl {
                line = line.saturating_add(1);
            }
            self.ends.push(next);
            self.lines.push(line);
            at = next;
        }
    }

    /// Line number once the read containing `need` has finished.
    /// `None` while that read can still grow.
    pub(crate) fn line(&mut self, need: usize) -> Option<u32> {
        if self.at > 0 && need < self.ends[self.at - 1] {
            self.at = 0;
        }
        while self.at < self.ends.len() && self.ends[self.at] <= need {
            self.at += 1;
        }
        if self.at < self.ends.len() {
            Some(self.lines[self.at])
        } else {
            None
        }
    }

    /// The greatest finished read that ends at or before `limit`.
    pub(crate) fn prefix_end(&self, limit: usize) -> usize {
        self.ends
            .iter()
            .rposition(|end| *end <= limit)
            .map(|index| self.ends[index])
            .unwrap_or(0)
    }

    /// Forget finished reads at or before `cut`. Later reads stay, shifted.
    pub(crate) fn discard_prefix(&mut self, cut: usize) {
        if cut == 0 {
            return;
        }
        let mut drop_n = 0;
        while drop_n < self.ends.len() && self.ends[drop_n] <= cut {
            drop_n += 1;
        }
        if drop_n > 0 {
            self.base = self.lines[drop_n - 1];
        }
        let mut ends = self.ends.split_off(drop_n);
        let lines = self.lines.split_off(drop_n);
        for end in &mut ends {
            *end -= cut;
        }
        self.ends = ends;
        self.lines = lines;
        self.at = 0;
    }

    /// Newlines in the dropped prefix.
    pub(crate) fn base_line(&self) -> u32 {
        self.base
    }

    /// Newlines once every finished read, including a sealed tail, is counted.
    pub(crate) fn sealed_line(&self) -> u32 {
        self.lines.last().copied().unwrap_or(self.base)
    }
}

/// First `\n` in `slice`.
fn find_newline(slice: &[u8]) -> Option<usize> {
    if slice.is_empty() {
        return None;
    }
    let found = unsafe { memchr(slice.as_ptr(), b'\n' as i32, slice.len()) };
    if found.is_null() {
        None
    } else {
        Some(unsafe { found.offset_from(slice.as_ptr()) } as usize)
    }
}

unsafe extern "C" {
    fn memchr(haystack: *const u8, needle: i32, len: usize) -> *const u8;
}

/// Where the next end-of-file check can start.
///
/// Each value used to walk from the first byte of the file. A file of many
/// values then reread every earlier line once per value. A check continues
/// from the last read it has passed. A read is passed only when it ended
/// before the end of the buffer, so more bytes can still lengthen a short
/// read at the end.
#[derive(Clone, Copy, Default)]
pub(crate) struct EofCut {
    /// Length of `bytes` this answer was classified for.
    seen: usize,
    /// First byte of the read that may still grow, or `seen` when every
    /// read is finished.
    resume: usize,
    /// Start of the unfinished tail read. `None` when that tail is not an
    /// end-of-file cut.
    open: Option<usize>,
}

#[cfg(test)]
static EOF_SCANNED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl EofCut {
    /// The value ending at `end` was completed in the last read of `bytes`,
    /// and that read did not end on a newline.
    ///
    /// `end` is the first byte after the value, or `bytes.len()` when the
    /// value runs to the end of the file. A read that ends on a newline does
    /// not count. A full 4095-byte read with more of the file after it does
    /// not count either.
    pub(crate) fn at(&mut self, bytes: &[u8], end: usize) -> bool {
        if bytes.is_empty() || end == 0 {
            return false;
        }
        if self.seen != bytes.len() {
            self.classify(bytes);
        }
        let Some(open) = self.open else {
            return false;
        };
        let pos = if end < bytes.len() {
            end
        } else {
            bytes.len() - 1
        };
        pos >= open
    }

    /// One pass over the bytes added since the last call. A value inside a
    /// finished read does not walk that read again.
    fn classify(&mut self, bytes: &[u8]) {
        let len = bytes.len();
        if self.resume > len {
            self.resume = 0;
            self.open = None;
        }
        self.seen = len;
        // A buffer that ends on a newline has no end-of-file cut.
        if bytes.last() == Some(&b'\n') {
            self.open = None;
            self.resume = len;
            return;
        }
        let mut at = match self.open {
            Some(open) => open,
            None => self.resume.min(len),
        };
        while at < len {
            let next = read_chunk(bytes, at);
            #[cfg(test)]
            EOF_SCANNED.fetch_add((next - at) as u64, std::sync::atomic::Ordering::Relaxed);
            if next == at {
                break;
            }
            if next == len {
                self.open = Some(at);
                self.resume = at;
                return;
            }
            at = next;
        }
        self.open = None;
        self.resume = at;
    }
}

impl NulScan {
    /// A NUL with no newline after it ends input on macOS.
    pub(crate) fn truncates(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.last_nul.is_some() && !self.nl_after
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    /// `cut` bytes have left the front of the buffer.
    pub(crate) fn shift(&mut self, cut: usize) {
        if cut == 0 {
            return;
        }
        self.checked = self.checked.saturating_sub(cut);
        if let Some(index) = self.last_nul {
            if index >= cut {
                self.last_nul = Some(index - cut);
            } else {
                self.last_nul = None;
                self.nl_after = false;
            }
        }
    }
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
    use super::{nul_cut, stream_line, EofCut, Fgets, LineCursor, NulScan};

    fn finished_on_eof_cut(bytes: &[u8], end: usize) -> bool {
        EofCut::default().at(bytes, end)
    }

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
        boundary.extend(std::iter::repeat_n(b'y', 20));
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

    /// A finished file with no trailing newline still has a line. The table
    /// used to leave that tail out, so the value at the end had no line.
    #[test]
    fn fgets_sealed_tail_matches_line_cursor() {
        let text = b"12345";
        let mut table = Fgets::default();
        table.push(text, true);
        let mut cursor = LineCursor::default();
        assert!(cursor.ready(text, text.len(), false, true));
        let line = cursor.line_at(text, text.len(), false);
        assert_eq!(table.line(text.len() - 1), Some(line as u32));

        let mut samples = vec![
            b"abc".to_vec(),
            b"abc\n".to_vec(),
            b"\n\n\n".to_vec(),
            b"no-newline-at-all".to_vec(),
            bytes(10, 1),
            bytes(80, 7),
            bytes(4096, 10),
            bytes(9000, 4095),
            vec![0xEF, 0xBB, 0xBF, b'1', b'\n'],
        ];
        let mut boundary = vec![b'x'; 4095];
        boundary.push(b'\n');
        boundary.extend(std::iter::repeat_n(b'y', 20));
        samples.push(boundary);
        for text in samples {
            let step = if text.len() > 500 { 128 } else { 1 };
            let mut table = Fgets::default();
            let mut built = 0;
            while built < text.len() {
                let next = (built + step).min(text.len());
                let eof = next == text.len();
                table.push(&text[..next], eof);
                agree(&mut table, &text[..next], eof);
                built = next;
            }
        }
    }

    fn agree(table: &mut Fgets, text: &[u8], eof: bool) {
        for end in ends_for(text.len()) {
            for atom in [false, true] {
                if atom && end >= text.len() && !eof {
                    continue;
                }
                let mut cursor = LineCursor::default();
                let ready = cursor.ready(text, end, atom, eof);
                let need = if atom && end < text.len() {
                    end
                } else {
                    end.saturating_sub(1)
                };
                let got = table.line(need);
                if ready {
                    let mut fresh = LineCursor::default();
                    let line = fresh.line_at(text, end, atom);
                    assert_eq!(
                        got,
                        Some(line as u32),
                        "end {end} atom {atom} eof {eof} len {}",
                        text.len()
                    );
                } else {
                    assert_eq!(
                        got,
                        None,
                        "end {end} atom {atom} eof {eof} len {}",
                        text.len()
                    );
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

    /// The read loop `finished_on_eof_cut` used before it continued forward.
    fn slow_eof_cut(bytes: &[u8], end: usize) -> bool {
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
            let next = super::read_chunk(bytes, at);
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

    fn agrees(walk: &mut EofCut, bytes: &[u8], end: usize) {
        assert_eq!(
            walk.at(bytes, end),
            slow_eof_cut(bytes, end),
            "end {end} len {}",
            bytes.len()
        );
    }

    #[test]
    fn eof_cut_continues_from_the_read_it_passed() {
        let mut texts = vec![
            b"".to_vec(),
            b"\n".to_vec(),
            b"3".to_vec(),
            b"1\n".to_vec(),
            b"1\n2\n3".to_vec(),
            b"1\n2\n3\n".to_vec(),
            b"1\n2\n3 ".to_vec(),
            b"1\n2\n3\n ".to_vec(),
            b"1\n2\n   ".to_vec(),
            vec![b'1'; 4095],
            vec![b'1'; 4096],
            vec![b'a'; 8190],
        ];
        let mut nl_then = vec![b'\n'];
        nl_then.extend(std::iter::repeat_n(b'a', 5000));
        texts.push(nl_then);
        let mut boundary = vec![b'a'; 4094];
        boundary.push(b'\n');
        boundary.push(b'b');
        texts.push(boundary);

        for text in &texts {
            let mut walk = EofCut::default();
            for end in 0..=text.len() {
                agrees(&mut walk, text, end);
            }
            if text.len() > 4 {
                agrees(&mut walk, text, 1);
                agrees(&mut walk, text, text.len());
            }
        }

        // Bytes arriving later extend a short read. The same walk has to
        // agree after each extra byte, including a newline that ends it.
        let mut buf = Vec::new();
        let mut walk = EofCut::default();
        for &byte in b"12\n3 4\n5\n6789" {
            buf.push(byte);
            for end in 0..=buf.len() {
                agrees(&mut walk, &buf, end);
            }
        }

        // A short tail that grows past a 4095-byte read is a new last read.
        let mut grown = vec![b'\n'];
        grown.extend(std::iter::repeat_n(b'a', 100));
        let mut walk = EofCut::default();
        agrees(&mut walk, &grown, grown.len());
        grown.extend(std::iter::repeat_n(b'a', 4090));
        agrees(&mut walk, &grown, 50);
        agrees(&mut walk, &grown, grown.len());
        grown.push(b'\n');
        agrees(&mut walk, &grown, grown.len());
        grown.push(b'z');
        agrees(&mut walk, &grown, grown.len());

        let mut many = Vec::new();
        for _ in 0..200_000 {
            many.extend_from_slice(b"1\n");
        }
        many.pop();
        let marks = [
            0,
            1,
            2,
            4094,
            4095,
            4096,
            many.len() / 2,
            many.len().saturating_sub(1),
            many.len(),
        ];
        let mut walk = EofCut::default();
        for end in marks {
            agrees(&mut walk, &many, end);
        }
        let started = std::time::Instant::now();
        let mut walk = EofCut::default();
        for end in 0..=many.len() {
            let _ = walk.at(&many, end);
        }
        assert!(
            started.elapsed().as_millis() < 3_000,
            "reread the file for every value"
        );
    }

    /// A long line is one fgets window after another. Asking for every value
    /// must not read that window again for each value.
    #[test]
    fn eof_cut_does_not_reread_a_long_line() {
        let many = vec![b'1'; 200_000];
        super::EOF_SCANNED.store(0, std::sync::atomic::Ordering::Relaxed);
        let mut walk = EofCut::default();
        for end in 0..=many.len() {
            let _ = walk.at(&many, end);
        }
        let scanned = super::EOF_SCANNED.load(std::sync::atomic::Ordering::Relaxed);
        assert!(
            scanned < many.len() as u64 * 8,
            "scanned {scanned} bytes of a {}-byte line",
            many.len()
        );
    }

    fn reference_nul(bytes: &[u8]) -> usize {
        match bytes.iter().rposition(|byte| *byte == 0) {
            Some(index) if !bytes[index + 1..].contains(&b'\n') => index,
            _ => bytes.len(),
        }
    }

    #[test]
    fn nul_scan_matches_a_fresh_search() {
        let patterns: &[&[u8]] = &[
            b"",
            b"\0",
            b"a",
            b"\0b",
            b"a\0",
            b"a\0b",
            b"a\0\nb",
            b"a\nb\0c",
            b"\0\n",
            b"a\0b\0c",
            b"a\0b\nc\0d",
            b"1\n2\n3\n",
        ];
        for pattern in patterns {
            let mut scan = NulScan::default();
            let mut buf = Vec::new();
            for &byte in *pattern {
                buf.push(byte);
                assert_eq!(scan.at(&buf), reference_nul(&buf), "{buf:?}");
                assert_eq!(scan.cut(&buf), nul_cut(&buf));
            }
            assert_eq!(scan.at(&buf), reference_nul(&buf));
        }

        let mut buf = Vec::new();
        let mut scan = NulScan::default();
        for _ in 0..2_000 {
            buf.extend_from_slice(b"1\n{}");
            assert_eq!(scan.at(&buf), reference_nul(&buf));
        }
        buf.extend_from_slice(b"\0tail");
        assert_eq!(scan.at(&buf), reference_nul(&buf));
        buf.push(b'\n');
        assert_eq!(scan.at(&buf), reference_nul(&buf));
        buf.extend_from_slice(b"more\0");
        assert_eq!(scan.at(&buf), reference_nul(&buf));

        let mut big = Vec::new();
        let mut scan = NulScan::default();
        let started = std::time::Instant::now();
        for _ in 0..2_000 {
            big.extend(std::iter::repeat_n(b'a', 4096));
            let _ = scan.at(&big);
        }
        assert!(
            started.elapsed().as_millis() < 2_000,
            "reread the file looking for a NUL"
        );
        assert_eq!(scan.at(&big), reference_nul(&big));
    }
}
