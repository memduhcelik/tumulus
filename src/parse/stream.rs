//! `--stream` reads one token at a time.
//!
//! A scalar is kept back until a comma or the closing bracket commits it.
//! An empty array or object is one pair. A container that has values ends
//! with one closer, the path of its last child. jq's streaming parser also
//! words a few errors differently from the ordinary parser, and a final
//! closer that names a scalar is split off when nothing follows it.

use super::parser::{ParseErr, Parser};
use super::{skip_rest_of_line, split_bom, LineCursor};
use crate::value::Value;

struct RawItem {
    value: Value,
    /// Parser index in the text the scanner was given.
    end: usize,
    atom: bool,
    /// A parse error jq reports at `<unknown>` rather than a file line.
    unknown: bool,
}

struct RawScan {
    items: Vec<RawItem>,
    error: Option<ParseErr>,
    path: Vec<Value>,
    incomplete: bool,
    blocked: bool,
    error_at: usize,
    resume: usize,
    resume_line: usize,
    index: usize,
    line: usize,
    column: usize,
    stack: Vec<Frame>,
}

/// One value `--stream` hands the program, with the line a runtime error names.
pub struct StreamValue {
    pub value: Value,
    /// Newlines jq's reader has consumed. Ignored when `unknown` is set.
    pub line: usize,
    /// The final scalar closer of an input that does not end in whitespace.
    pub unknown: bool,
}

/// Values the program runs on, then a parse error if the tail is bad.
pub struct StreamFeed {
    pub values: Vec<StreamValue>,
    pub error: Option<String>,
}

struct Walker<'a> {
    parser: Parser<'a>,
    items: Vec<RawItem>,
    stack: Vec<Frame>,
    err_path: Vec<Value>,
    /// End of the input, not merely the end of the bytes read so far.
    final_eof: bool,
    blocked: bool,
}

/// Where a `--stream` scan should continue.
pub(crate) struct StreamState {
    pub index: usize,
    pub line: usize,
    pub column: usize,
    stack: Vec<Frame>,
}

impl Default for StreamState {
    fn default() -> Self {
        Self {
            index: 0,
            line: 1,
            column: 0,
            stack: Vec::new(),
        }
    }
}

impl StreamState {
    pub(crate) fn stack_empty(&self) -> bool {
        self.stack.is_empty()
    }
}

pub(crate) struct StreamOut {
    pub items: Vec<(Value, usize, bool, bool)>,
    pub error: Option<String>,
    pub path: Vec<Value>,
    pub blocked: bool,
    pub incomplete: bool,
    pub resume: usize,
    pub resume_line: usize,
    pub error_at: usize,
}

enum Act {
    Keep,
    Nest(Frame),
    Done,
    /// The next token is not finished in this buffer yet.
    Block,
}

struct Arr {
    path: Vec<Value>,
    index: usize,
    held: Option<Value>,
    after_comma: bool,
    started: bool,
}

enum ObjStage {
    Key,
    /// The key and the colon are in. The value has not been read.
    Value,
    Sep,
}

struct Obj {
    path: Vec<Value>,
    after_comma: bool,
    key: String,
    held: Option<Value>,
    stage: ObjStage,
    started: bool,
}

enum Frame {
    Arr(Arr),
    Obj(Obj),
}

impl Frame {
    fn note_child(&mut self) {
        match self {
            Frame::Arr(arr) => {
                arr.index += 1;
                arr.after_comma = false;
                arr.held = None;
            }
            Frame::Obj(obj) => {
                obj.stage = ObjStage::Sep;
                obj.held = None;
            }
        }
    }
}

fn new_arr(path: Vec<Value>) -> Frame {
    Frame::Arr(Arr {
        path,
        index: 0,
        held: None,
        after_comma: false,
        started: false,
    })
}

fn new_obj(path: Vec<Value>) -> Frame {
    Frame::Obj(Obj {
        path,
        after_comma: false,
        key: String::new(),
        held: None,
        stage: ObjStage::Key,
        started: false,
    })
}

/// One piece of a `--seq` record scanned as `--stream`.
pub(crate) enum RecEvent {
    Item {
        value: Value,
        end: usize,
        atom: bool,
        failure: bool,
    },
    Soft {
        message: String,
        at: usize,
        unknown: bool,
    },
}

/// Scan one record. `line` and `column` already count the record separator.
/// A number at the end of the record is a value. An unfinished container
/// cut off by RS is `Truncated value`, and items read before the cut stay.
pub(crate) fn scan_seq_record(
    chunk: &[u8],
    line: usize,
    column: usize,
    ended_by_rs: bool,
    end_line: usize,
    end_column: usize,
    errors: bool,
) -> Vec<RecEvent> {
    let mut state = StreamState {
        index: 0,
        line,
        column,
        stack: Vec::new(),
    };
    let mut out = Vec::new();
    let mut prior = false;
    for _ in 0..=chunk.len() {
        let before = state.index;
        let scanned = drive(chunk, &mut state, true, &[]);
        for item in scanned.items {
            prior = true;
            out.push(RecEvent::Item {
                value: item.value,
                end: item.end,
                atom: item.atom,
                failure: false,
            });
        }
        let Some(error) = scanned.error else {
            break;
        };
        if scanned.blocked {
            break;
        }
        let unfinished = error.ran_out();
        let message = if ended_by_rs && unfinished {
            format!(
                "Truncated value at line {end_line}, column {}",
                end_column + 1
            )
        } else {
            let text = error.text();
            if text.contains(" at EOF ") || text.contains("need RS to resync") {
                text
            } else {
                format!("{text} (need RS to resync)")
            }
        };
        let unknown = error_is_unknown(&message, chunk, prior);
        // The separator is the byte that cut the value off.
        let at = if ended_by_rs && unfinished {
            chunk.len() + 1
        } else {
            scanned.error_at
        };
        if errors {
            prior = true;
            out.push(RecEvent::Item {
                value: error_value(&message, &scanned.path),
                end: at,
                atom: false,
                failure: true,
            });
        } else {
            out.push(RecEvent::Soft {
                message,
                at,
                unknown,
            });
        }
        if unfinished || state.index == before {
            break;
        }
    }
    out
}

/// One whole input. `slurp` packs the items the way `--stream -s` does.
pub fn program_stream(bytes: &[u8], slurp: bool) -> StreamFeed {
    program_stream_with(bytes, slurp, false, &[])
}

/// `errors` turns a parse error into a `[message, path]` value.
/// `bounds` are the parse indexes where a later file starts.
pub fn program_stream_with(
    bytes: &[u8],
    slurp: bool,
    errors: bool,
    bounds: &[usize],
) -> StreamFeed {
    let bytes = &bytes[..super::nul_cut(bytes)];
    let (bom, text) = split_bom(bytes);
    let bounds = shift_bounds(bounds, bom);
    let scanned = walk_all(text, errors, &bounds);
    let mut cursor = LineCursor::default();
    let mut located = Vec::with_capacity(scanned.items.len());
    for item in &scanned.items {
        let line = cursor.line_at(bytes, item.end + bom, item.atom);
        located.push((item.value.clone(), line, item.unknown));
    }
    let error = scanned.error.as_ref().map(ParseErr::text);
    let error_path = scanned.path;
    if slurp {
        return slurp_feed(&located, error, error_path, errors, bytes);
    }
    let mut values: Vec<StreamValue> = located
        .iter()
        .map(|(value, line, unknown)| StreamValue {
            value: value.clone(),
            line: *line,
            unknown: *unknown,
        })
        .collect();
    if errors {
        if let Some(message) = error.as_ref() {
            let line = cursor.line_at(bytes, scanned.error_at + bom, false);
            let unknown = error_is_unknown(message, bytes, !values.is_empty());
            values.push(StreamValue {
                value: error_value(message, &error_path),
                line,
                unknown,
            });
        }
        let detach = error.is_none() && detached_values(&values, bytes);
        if detach {
            if let Some(last) = values.last_mut() {
                last.unknown = true;
            }
        }
        return StreamFeed {
            values,
            error: None,
        };
    }
    let plain: Vec<Value> = values.iter().map(|item| item.value.clone()).collect();
    let detach = error.is_none() && detached(&plain, bytes);
    if detach {
        if let Some(last) = values.last_mut() {
            last.unknown = true;
        }
    }
    StreamFeed { values, error }
}

fn shift_bounds(bounds: &[usize], bom: usize) -> Vec<usize> {
    bounds
        .iter()
        .filter_map(|bound| bound.checked_sub(bom))
        .filter(|bound| *bound > 0)
        .collect()
}

fn walk_all(text: &[u8], errors: bool, bounds: &[usize]) -> RawScan {
    let mut state = StreamState::default();
    let mut items = Vec::new();
    loop {
        let scanned = drive(text, &mut state, true, bounds);
        items.extend(scanned.items);
        if scanned.blocked {
            return RawScan {
                items,
                error: None,
                path: Vec::new(),
                incomplete: false,
                blocked: false,
                error_at: state.index,
                resume: state.index,
                resume_line: state.line,
                index: state.index,
                line: state.line,
                column: state.column,
                stack: Vec::new(),
            };
        }
        let Some(error) = scanned.error else {
            return RawScan {
                items,
                error: None,
                path: Vec::new(),
                incomplete: false,
                blocked: false,
                error_at: scanned.error_at,
                resume: scanned.resume,
                resume_line: scanned.resume_line,
                index: scanned.index,
                line: scanned.line,
                column: scanned.column,
                stack: Vec::new(),
            };
        };
        if !errors || scanned.incomplete {
            return RawScan {
                items,
                path: scanned.path,
                error: Some(error),
                incomplete: scanned.incomplete,
                blocked: false,
                error_at: scanned.error_at,
                resume: scanned.resume,
                resume_line: scanned.resume_line,
                index: scanned.index,
                line: scanned.line,
                column: scanned.column,
                stack: Vec::new(),
            };
        }
        let message = error.text();
        let unknown = error_is_unknown(&message, text, !items.is_empty());
        items.push(RawItem {
            value: error_value(&message, &scanned.path),
            end: scanned.error_at,
            atom: false,
            unknown,
        });
        if state.index == scanned.error_at && scanned.resume == scanned.error_at {
            return RawScan {
                items,
                error: None,
                path: Vec::new(),
                incomplete: false,
                blocked: false,
                error_at: scanned.error_at,
                resume: scanned.resume,
                resume_line: scanned.resume_line,
                index: state.index,
                line: state.line,
                column: 0,
                stack: Vec::new(),
            };
        }
    }
}

pub(crate) fn error_value(message: &str, path: &[Value]) -> Value {
    Value::array(vec![
        Value::String(message.to_string()),
        Value::array(path.to_vec()),
    ])
}

/// jq names this error `<unknown>` when the whole input is one line and does
/// not end in whitespace, and a value was already emitted.
pub(crate) fn error_is_unknown(message: &str, bytes: &[u8], prior: bool) -> bool {
    prior
        && message.starts_with("Unfinished JSON term at EOF")
        && !bytes.contains(&b'\n')
        && !ends_with_ws(bytes)
}

fn detached_values(items: &[StreamValue], bytes: &[u8]) -> bool {
    let values: Vec<Value> = items.iter().map(|item| item.value.clone()).collect();
    detached(&values, bytes)
}

/// Parse what is already buffered. `final_eof` is false while more bytes may arrive.
pub(crate) fn pull_stream(
    text: &[u8],
    state: &mut StreamState,
    final_eof: bool,
    bounds: &[usize],
) -> StreamOut {
    let mut scanned = drive(text, state, final_eof, bounds);
    let error = scanned.error.as_ref().map(ParseErr::text);
    let path = std::mem::take(&mut scanned.path);
    StreamOut {
        items: scanned
            .items
            .into_iter()
            .map(|item| (item.value, item.end, item.atom, item.unknown))
            .collect(),
        error,
        path,
        blocked: scanned.blocked,
        incomplete: scanned.incomplete,
        resume: scanned.resume,
        resume_line: scanned.resume_line,
        error_at: scanned.error_at,
    }
}

fn drive(text: &[u8], state: &mut StreamState, final_eof: bool, bounds: &[usize]) -> RawScan {
    let mut walker = Walker {
        parser: Parser::new(text),
        items: Vec::new(),
        stack: std::mem::take(&mut state.stack),
        err_path: Vec::new(),
        final_eof,
        blocked: false,
    };
    walker.parser.set_boundaries(bounds);
    walker.parser.seek_at(state.index, state.line, state.column);
    let mut scanned = walker.scan();
    if scanned.blocked {
        state.index = scanned.index;
        state.line = scanned.line;
        state.column = scanned.column;
        state.stack = std::mem::take(&mut scanned.stack);
    } else if scanned.error.is_some() {
        state.index = scanned.resume;
        state.line = scanned.resume_line;
        state.column = 0;
        state.stack = Vec::new();
    } else {
        state.index = scanned.index;
        state.line = scanned.line;
        state.column = scanned.column;
        state.stack = Vec::new();
    }
    scanned
}

impl<'a> Walker<'a> {
    fn scan(mut self) -> RawScan {
        loop {
            if !self.stack.is_empty() {
                if self.parser.eof() && !self.final_eof {
                    self.blocked = true;
                    return self.finish(None);
                }
                let Some(mut frame) = self.stack.pop() else {
                    continue;
                };
                match self.step(&mut frame) {
                    Ok(Act::Keep) => self.stack.push(frame),
                    Ok(Act::Nest(child)) => {
                        self.stack.push(frame);
                        self.stack.push(child);
                    }
                    Ok(Act::Done) => {
                        if let Some(parent) = self.stack.last_mut() {
                            parent.note_child();
                        }
                    }
                    Ok(Act::Block) => {
                        self.stack.push(frame);
                        self.blocked = true;
                        return self.finish(None);
                    }
                    Err(err) => return self.finish(Some(err)),
                }
                continue;
            }
            self.parser.skip_ws();
            if self.parser.eof() {
                if !self.final_eof {
                    self.blocked = true;
                }
                return self.finish(None);
            }
            self.err_path.clear();
            match self.begin_value() {
                Ok(()) if self.blocked => return self.finish(None),
                Ok(()) => {}
                Err(err) => return self.finish(Some(err)),
            }
        }
    }

    fn finish(self, error: Option<ParseErr>) -> RawScan {
        let incomplete = error.as_ref().is_some_and(ParseErr::ran_out);
        let (resume, resume_line) =
            if self.blocked || error.is_none() || incomplete || self.parser.column() == 0 {
                (self.parser.index, self.parser.line)
            } else {
                skip_rest_of_line(self.parser.src, self.parser.index, self.parser.line)
            };
        RawScan {
            items: self.items,
            path: self.err_path,
            error,
            incomplete,
            blocked: self.blocked,
            error_at: self.parser.index,
            resume,
            resume_line,
            index: self.parser.index,
            line: self.parser.line,
            column: self.parser.column(),
            stack: self.stack,
        }
    }

    fn begin_value(&mut self) -> Result<(), ParseErr> {
        match self.parser.peek() {
            None => Err(self.parser.fail("Unfinished JSON term", true)),
            Some(']') => Err(self.parser.bump_fail("Unmatched ']' at the top-level")),
            Some('}') => Err(self.parser.bump_fail("Unmatched '}' at the top-level")),
            Some(',') => Err(self.parser.bump_fail("Expected value before ','")),
            Some(':') => Err(self.parser.bump_fail("':' not as part of an object")),
            Some('[') => {
                self.parser.bump();
                self.stack.push(new_arr(Vec::new()));
                Ok(())
            }
            Some('{') => {
                self.parser.bump();
                self.stack.push(new_obj(Vec::new()));
                Ok(())
            }
            Some('"') => {
                if !self.string_ready() {
                    self.blocked = true;
                    return Ok(());
                }
                // A string is finished at its closing quote. A bad byte after
                // it is a separate error, and the string is still a value.
                let value = Value::String(self.parser.parse_string()?);
                self.emit_pair(&[], value, false);
                self.top_glue()?;
                Ok(())
            }
            Some(_) => {
                if !self.atom_ready() {
                    self.blocked = true;
                    return Ok(());
                }
                let value = self.parser.scan_atom()?;
                self.top_glue()?;
                let atom = is_atom_value(&value);
                self.emit_pair(&[], value, atom);
                Ok(())
            }
        }
    }

    fn string_ready(&self) -> bool {
        if self.final_eof {
            return true;
        }
        let bytes = self.parser.src;
        let mut index = self.parser.index + 1;
        while index < bytes.len() {
            match bytes[index] {
                b'"' => return true,
                b'\\' => index += 2,
                _ => index += 1,
            }
        }
        false
    }

    fn atom_ready(&self) -> bool {
        if self.final_eof {
            return true;
        }
        let mut index = self.parser.index;
        while index < self.parser.src.len() {
            let byte = self.parser.src[index];
            if matches!(
                byte,
                b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'[' | b']' | b'{' | b'}' | b',' | b':'
            ) {
                return true;
            }
            index += 1;
        }
        false
    }

    /// A top-level atom glued to a bracket or a comma is not a value.
    fn top_glue(&mut self) -> Result<(), ParseErr> {
        match self.parser.peek() {
            Some(']') => Err(self.parser.bump_fail("Unmatched ']' at the top-level")),
            Some('}') => Err(self.parser.bump_fail("Unmatched '}' at the top-level")),
            Some(',') => Err(self.parser.bump_fail("Expected value before ','")),
            Some(':') => Err(self.parser.bump_fail("':' not as part of an object")),
            _ => Ok(()),
        }
    }

    fn step(&mut self, frame: &mut Frame) -> Result<Act, ParseErr> {
        match frame {
            Frame::Arr(arr) => self.step_array(arr),
            Frame::Obj(obj) => self.step_object(obj),
        }
    }

    fn step_array(&mut self, arr: &mut Arr) -> Result<Act, ParseErr> {
        self.parser.skip_ws();
        if !arr.started {
            if self.parser.peek().is_none() && !self.final_eof {
                return Ok(Act::Block);
            }
            arr.started = true;
            if self.parser.peek() == Some(']') {
                self.parser.bump();
                self.emit_pair(&arr.path, Value::array(Vec::new()), false);
                return Ok(Act::Done);
            }
        }
        self.mark_arr(arr);
        match self.parser.peek() {
            None => {
                if !self.final_eof {
                    return Ok(Act::Block);
                }
                Err(self.parser.fail("Unfinished JSON term", true))
            }
            Some(']') => {
                if arr.after_comma && arr.held.is_none() {
                    return Err(self.parser.bump_fail("Expected another array element"));
                }
                self.parser.bump();
                if let Some(value) = arr.held.take() {
                    let child = with_index(&arr.path, arr.index);
                    let atom = is_atom_value(&value);
                    self.emit_pair(&child, value, atom);
                    self.emit_closer(&child);
                } else if arr.index == 0 {
                    return Err(self.parser.fail("Unfinished JSON term", true));
                } else {
                    self.emit_closer(&with_index(&arr.path, arr.index - 1));
                }
                Ok(Act::Done)
            }
            Some('}') => {
                if arr.after_comma && arr.held.is_none() {
                    return Err(self.parser.bump_fail("Expected another key:value pair"));
                }
                Err(self
                    .parser
                    .bump_fail("Unmatched '}' in the middle of an array"))
            }
            Some(',') => {
                if let Some(value) = arr.held.take() {
                    self.parser.bump();
                    let child = with_index(&arr.path, arr.index);
                    let atom = is_atom_value(&value);
                    self.emit_pair(&child, value, atom);
                    arr.index += 1;
                    arr.after_comma = true;
                    Ok(Act::Keep)
                } else if arr.index > 0 && !arr.after_comma {
                    self.parser.bump();
                    arr.after_comma = true;
                    Ok(Act::Keep)
                } else {
                    Err(self.parser.bump_fail("Expected value before ','"))
                }
            }
            Some(':') => Err(self.parser.bump_fail("':' not as part of an object")),
            Some(_) => {
                if arr.held.is_some() || (arr.index > 0 && !arr.after_comma) {
                    return Err(self.parser.expected_stream_separator());
                }
                let child = with_index(&arr.path, arr.index);
                match self.parser.peek() {
                    Some('[') => {
                        self.parser.bump();
                        Ok(Act::Nest(new_arr(child)))
                    }
                    Some('{') => {
                        self.parser.bump();
                        Ok(Act::Nest(new_obj(child)))
                    }
                    Some('"') => {
                        if !self.string_ready() {
                            return Ok(Act::Block);
                        }
                        arr.held = Some(Value::String(self.parser.parse_string()?));
                        arr.after_comma = false;
                        Ok(Act::Keep)
                    }
                    Some(_) => {
                        if !self.atom_ready() {
                            return Ok(Act::Block);
                        }
                        arr.held = Some(self.parser.scan_atom()?);
                        arr.after_comma = false;
                        Ok(Act::Keep)
                    }
                    None => {
                        if !self.final_eof {
                            return Ok(Act::Block);
                        }
                        Err(self.parser.fail("Unfinished JSON term", true))
                    }
                }
            }
        }
    }

    fn step_object(&mut self, obj: &mut Obj) -> Result<Act, ParseErr> {
        match obj.stage {
            ObjStage::Sep => return self.object_sep(obj),
            ObjStage::Value => return self.object_value(obj),
            ObjStage::Key => {}
        }
        self.mark_obj(obj, false);
        self.parser.skip_ws();
        if !obj.started {
            if self.parser.peek().is_none() && !self.final_eof {
                return Ok(Act::Block);
            }
            obj.started = true;
            if self.parser.peek() == Some('}') {
                self.parser.bump();
                self.emit_pair(&obj.path, Value::object(Vec::new()), false);
                return Ok(Act::Done);
            }
        }
        if self.hold_for_token() || !self.colon_ready() {
            return Ok(Act::Block);
        }
        let key = self.parse_key(obj.after_comma)?;
        self.parser.expect_colon()?;
        obj.key = key;
        obj.stage = ObjStage::Value;
        self.object_value(obj)
    }

    fn object_value(&mut self, obj: &mut Obj) -> Result<Act, ParseErr> {
        self.mark_obj(obj, true);
        self.parser.skip_ws();
        match self.parser.peek() {
            None => {
                if !self.final_eof {
                    return Ok(Act::Block);
                }
                Err(self.parser.fail("Unfinished JSON term", true))
            }
            Some('}') => Err(self.parser.bump_fail("Missing value in key:value pair")),
            Some(']') => Err(self
                .parser
                .bump_fail("Unmatched ']' in the middle of an object")),
            Some('[') => {
                self.parser.bump();
                Ok(Act::Nest(new_arr(with_key(&obj.path, &obj.key))))
            }
            Some('{') => {
                self.parser.bump();
                Ok(Act::Nest(new_obj(with_key(&obj.path, &obj.key))))
            }
            Some('"') => {
                if !self.string_ready() {
                    return Ok(Act::Block);
                }
                obj.held = Some(Value::String(self.parser.parse_string()?));
                obj.stage = ObjStage::Sep;
                Ok(Act::Keep)
            }
            Some(_) => {
                if !self.atom_ready() {
                    return Ok(Act::Block);
                }
                obj.held = Some(self.parser.scan_atom()?);
                obj.stage = ObjStage::Sep;
                Ok(Act::Keep)
            }
        }
    }

    /// The key and the colon that follows it are both in this buffer.
    fn colon_ready(&self) -> bool {
        if self.final_eof {
            return true;
        }
        let bytes = self.parser.src;
        let mut index = self.parser.index;
        while index < bytes.len() && matches!(bytes[index], b' ' | b'\t' | b'\n' | b'\r') {
            index += 1;
        }
        if index >= bytes.len() {
            return false;
        }
        if bytes[index] == b'"' {
            index += 1;
            let mut closed = false;
            while index < bytes.len() {
                match bytes[index] {
                    b'"' => {
                        index += 1;
                        closed = true;
                        break;
                    }
                    b'\\' => index += 2,
                    _ => index += 1,
                }
            }
            if !closed {
                return false;
            }
        } else if matches!(bytes[index], b'{' | b'}' | b'[' | b']' | b',' | b':') {
            return true;
        } else {
            while index < bytes.len()
                && !matches!(
                    bytes[index],
                    b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'[' | b']' | b'{' | b'}' | b',' | b':'
                )
            {
                index += 1;
            }
            if index >= bytes.len() {
                return false;
            }
        }
        while index < bytes.len() && matches!(bytes[index], b' ' | b'\t' | b'\n' | b'\r') {
            index += 1;
        }
        index < bytes.len() && bytes[index] == b':'
    }

    /// The next key or atom is cut off at the end of this buffer.
    fn hold_for_token(&self) -> bool {
        match self.parser.peek() {
            Some('"') => !self.string_ready(),
            Some(c) if !matches!(c, '"' | '}' | ']' | ',' | ':' | '[' | '{') => !self.atom_ready(),
            _ => false,
        }
    }

    fn object_sep(&mut self, obj: &mut Obj) -> Result<Act, ParseErr> {
        self.mark_obj(obj, true);
        self.parser.skip_ws();
        match self.parser.peek() {
            None => {
                if !self.final_eof {
                    return Ok(Act::Block);
                }
                Err(self.parser.fail("Unfinished JSON term", true))
            }
            Some(',') => {
                self.parser.bump();
                if let Some(value) = obj.held.take() {
                    let child = with_key(&obj.path, &obj.key);
                    let atom = is_atom_value(&value);
                    self.emit_pair(&child, value, atom);
                }
                obj.after_comma = true;
                obj.stage = ObjStage::Key;
                Ok(Act::Keep)
            }
            Some('}') => {
                self.parser.bump();
                if let Some(value) = obj.held.take() {
                    let child = with_key(&obj.path, &obj.key);
                    let atom = is_atom_value(&value);
                    self.emit_pair(&child, value, atom);
                }
                self.emit_closer(&with_key(&obj.path, &obj.key));
                Ok(Act::Done)
            }
            Some(']') => Err(self
                .parser
                .bump_fail("Unmatched ']' in the middle of an object")),
            Some(_) => Err(self.parser.expected_stream_separator()),
        }
    }

    fn parse_key(&mut self, after_comma: bool) -> Result<String, ParseErr> {
        self.parser.skip_ws();
        match self.parser.peek() {
            None => Err(self.parser.fail("Unfinished JSON term", true)),
            Some('}') => Err(self.parser.bump_fail(if after_comma {
                "Expected another key:value pair"
            } else {
                "Unmatched '}' in the middle of an object"
            })),
            // After a comma, `]` is named as a missing array element.
            Some(']') => Err(self.parser.bump_fail(if after_comma {
                "Expected another array element"
            } else {
                "Unmatched ']' in the middle of an object"
            })),
            Some(',') => Err(self.parser.bump_fail("Expected value before ','")),
            Some(':') => Err(self.parser.bump_fail("Expected string key before ':'")),
            Some('[') => Err(self.key_open(after_comma, false)),
            Some('{') => Err(self.key_open(after_comma, true)),
            Some('"') => self.parser.parse_string(),
            Some(_) => {
                self.parser.parse_value()?;
                Err(self.stream_bad_key())
            }
        }
    }

    fn stream_bad_key(&mut self) -> ParseErr {
        self.parser.skip_ws();
        match self.parser.peek() {
            Some(':') => self.parser.bump_fail("Object keys must be strings"),
            Some('}' | ',') => self
                .parser
                .bump_fail("Objects must consist of key:value pairs"),
            Some(']') => self
                .parser
                .bump_fail("Unmatched ']' in the middle of an object"),
            None => self.parser.fail("Unfinished JSON term", true),
            Some(_) => self.parser.expected_stream_separator(),
        }
    }

    fn key_open(&mut self, after_comma: bool, brace: bool) -> ParseErr {
        self.parser.bump();
        let phrase = match (after_comma, brace) {
            (false, false) => "Expected string key after '{', not '['",
            (false, true) => "Expected string key after '{', not '{'",
            (true, false) => "Expected string key after ',' in object, not '['",
            (true, true) => "Expected string key after ',' in object, not '{'",
        };
        self.parser.fail(phrase, false)
    }

    fn emit_pair(&mut self, path: &[Value], value: Value, atom: bool) {
        let item = Value::array(vec![Value::array(path.to_vec()), value]);
        self.items.push(RawItem {
            value: item,
            end: self.parser.index,
            atom,
            unknown: false,
        });
    }

    fn emit_closer(&mut self, path: &[Value]) {
        let item = Value::array(vec![Value::array(path.to_vec())]);
        self.items.push(RawItem {
            value: item,
            end: self.parser.index,
            atom: false,
            unknown: false,
        });
    }

    fn mark_arr(&mut self, arr: &Arr) {
        self.err_path = with_index(&arr.path, arr.index);
    }

    /// `keyed` is the key whose value is being read. Otherwise the slot is null.
    fn mark_obj(&mut self, obj: &Obj, keyed: bool) {
        let mut path = obj.path.clone();
        if keyed {
            path.push(Value::String(obj.key.clone()));
        } else {
            path.push(Value::Null);
        }
        self.err_path = path;
    }
}

fn with_index(path: &[Value], index: usize) -> Vec<Value> {
    let mut child = path.to_vec();
    child.push(Value::Number(index.to_string()));
    child
}

fn with_key(path: &[Value], key: &str) -> Vec<Value> {
    let mut child = path.to_vec();
    child.push(Value::String(key.to_string()));
    child
}

fn is_atom_value(value: &Value) -> bool {
    matches!(value, Value::Null | Value::Bool(_) | Value::Number(_))
}

fn is_scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

/// The last closer names a scalar, and the input does not end in whitespace.
pub(crate) fn closer_is_detached(items: &[Value], bytes: &[u8]) -> bool {
    detached(items, bytes)
}

fn detached(items: &[Value], bytes: &[u8]) -> bool {
    if ends_with_ws(bytes) || items.len() < 2 {
        return false;
    }
    let Some(Value::Array(closer)) = items.last() else {
        return false;
    };
    let Some(Value::Array(pair)) = items.get(items.len() - 2) else {
        return false;
    };
    closer.len() == 1 && pair.len() == 2 && is_scalar(&pair[1]) && same_path(&pair[0], &closer[0])
}

fn ends_with_ws(bytes: &[u8]) -> bool {
    matches!(bytes.last(), Some(b' ' | b'\t' | b'\n' | b'\r'))
}

fn same_path(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|(left, right)| match (left, right) {
                        (Value::Number(left), Value::Number(right))
                        | (Value::String(left), Value::String(right)) => left == right,
                        _ => false,
                    })
        }
        _ => false,
    }
}

fn slurp_feed(
    located: &[(Value, usize, bool)],
    error: Option<String>,
    error_path: Vec<Value>,
    errors: bool,
    bytes: &[u8],
) -> StreamFeed {
    let line = bytes.iter().filter(|byte| **byte == b'\n').count();
    if let Some(message) = error {
        let partial = show_partial(bytes, located, &message);
        if errors && partial {
            let items = located.iter().map(|(value, _, _)| value.clone()).collect();
            return StreamFeed {
                values: vec![
                    StreamValue {
                        value: Value::array(items),
                        line,
                        unknown: false,
                    },
                    StreamValue {
                        value: error_value(&message, &error_path),
                        line: 0,
                        unknown: error_is_unknown(&message, bytes, !located.is_empty()),
                    },
                ],
                error: None,
            };
        }
        if errors {
            let mut items: Vec<Value> = located.iter().map(|(value, _, _)| value.clone()).collect();
            items.push(error_value(&message, &error_path));
            return StreamFeed {
                values: vec![StreamValue {
                    value: Value::array(items),
                    line,
                    unknown: false,
                }],
                error: None,
            };
        }
        if partial {
            let items = located.iter().map(|(value, _, _)| value.clone()).collect();
            return StreamFeed {
                values: vec![StreamValue {
                    value: Value::array(items),
                    line,
                    unknown: false,
                }],
                error: Some(message),
            };
        }
        return StreamFeed {
            values: Vec::new(),
            error: Some(message),
        };
    }
    let values: Vec<Value> = located.iter().map(|(value, _, _)| value.clone()).collect();
    let detach = detached(&values, bytes);
    if detach {
        let mut items: Vec<Value> = located.iter().map(|(value, _, _)| value.clone()).collect();
        let last = items.pop().expect("detached closer");
        return StreamFeed {
            values: vec![
                StreamValue {
                    value: Value::array(items),
                    line,
                    unknown: false,
                },
                StreamValue {
                    value: last,
                    line: 0,
                    unknown: true,
                },
            ],
            error: None,
        };
    }
    let items = located.iter().map(|(value, _, _)| value.clone()).collect();
    StreamFeed {
        values: vec![StreamValue {
            value: Value::array(items),
            line,
            unknown: false,
        }],
        error: None,
    }
}

/// A cut-off value that ends on a comma still runs, when every item is a scalar pair.
fn show_partial(bytes: &[u8], located: &[(Value, usize, bool)], message: &str) -> bool {
    bytes.last() == Some(&b',')
        && message.starts_with("Unfinished JSON term at EOF")
        && !located.is_empty()
        && located.iter().all(|(value, _, _)| scalar_pair(value))
}

fn scalar_pair(value: &Value) -> bool {
    let Value::Array(items) = value else {
        return false;
    };
    items.len() == 2 && matches!(items[0], Value::Array(_)) && is_scalar(&items[1])
}
