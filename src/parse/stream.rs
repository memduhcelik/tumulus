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
}

struct RawScan {
    items: Vec<RawItem>,
    error: Option<ParseErr>,
    incomplete: bool,
    error_at: usize,
    resume: usize,
    resume_line: usize,
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

/// Stream items for one buffer, in the same shape `scan_input` uses.
pub(crate) struct StreamScan {
    pub items: Vec<(Value, usize, bool)>,
    pub error: Option<String>,
    pub incomplete: bool,
    pub error_at: usize,
    pub resume: usize,
    pub resume_line: usize,
}

struct Walker<'a> {
    parser: Parser<'a>,
    items: Vec<RawItem>,
}

enum Act {
    Keep,
    Nest(Frame),
    Done,
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

/// One whole input. `slurp` packs the items the way `--stream -s` does.
pub fn program_stream(bytes: &[u8], slurp: bool) -> StreamFeed {
    let (bom, text) = split_bom(bytes);
    let scanned = walk_text(text, 0, 1);
    let mut cursor = LineCursor::default();
    let mut located = Vec::with_capacity(scanned.items.len());
    for item in &scanned.items {
        let line = cursor.line_at(bytes, item.end + bom, item.atom);
        located.push((item.value.clone(), line));
    }
    let error = scanned.error.as_ref().map(ParseErr::text);
    let values: Vec<Value> = located.iter().map(|(value, _)| value.clone()).collect();
    let detach = error.is_none() && detached(&values, bytes);
    if slurp {
        return slurp_feed(&located, detach, error, bytes);
    }
    let last = located.len();
    let values = located
        .into_iter()
        .enumerate()
        .map(|(index, (value, line))| StreamValue {
            value,
            line,
            unknown: detach && index + 1 == last,
        })
        .collect();
    StreamFeed { values, error }
}

/// Continue a scan at `start`. Indexes are positions in `bytes`.
pub(crate) fn scan_stream_input(bytes: &[u8], start: usize, line: usize) -> StreamScan {
    let bom = if start == 0 && bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        0
    };
    let origin = if start == 0 { bom } else { start };
    let scanned = walk_text(&bytes[bom..], origin - bom, line);
    let shift = |index: usize| index + bom;
    StreamScan {
        items: scanned
            .items
            .into_iter()
            .map(|item| (item.value, shift(item.end), item.atom))
            .collect(),
        error: scanned.error.as_ref().map(ParseErr::text),
        incomplete: scanned.incomplete,
        error_at: shift(scanned.error_at),
        resume: shift(scanned.resume),
        resume_line: scanned.resume_line,
    }
}

fn walk_text(text: &[u8], start: usize, line: usize) -> RawScan {
    let mut walker = Walker {
        parser: Parser::new(text),
        items: Vec::new(),
    };
    walker.parser.seek(start, line);
    walker.scan()
}

impl<'a> Walker<'a> {
    fn scan(mut self) -> RawScan {
        loop {
            self.parser.skip_ws();
            if self.parser.eof() {
                return self.finish(None);
            }
            if let Err(err) = self.one_value() {
                return self.finish(Some(err));
            }
        }
    }

    fn finish(self, error: Option<ParseErr>) -> RawScan {
        let incomplete = error.as_ref().is_some_and(ParseErr::ran_out);
        let (resume, resume_line) = if error.is_none() || incomplete || self.parser.column() == 0 {
            (self.parser.index, self.parser.line)
        } else {
            skip_rest_of_line(self.parser.src, self.parser.index, self.parser.line)
        };
        RawScan {
            items: self.items,
            error,
            incomplete,
            error_at: self.parser.index,
            resume,
            resume_line,
        }
    }

    fn one_value(&mut self) -> Result<(), ParseErr> {
        let mut stack = Vec::new();
        match self.parser.peek() {
            None => return Err(self.parser.fail("Unfinished JSON term", true)),
            Some(']') => return Err(self.parser.bump_fail("Unmatched ']' at the top-level")),
            Some('}') => return Err(self.parser.bump_fail("Unmatched '}' at the top-level")),
            Some(',') => return Err(self.parser.bump_fail("Expected value before ','")),
            Some(':') => {
                return Err(self.parser.bump_fail("':' not as part of an object"));
            }
            Some('[') => {
                self.parser.bump();
                stack.push(new_arr(Vec::new()));
            }
            Some('{') => {
                self.parser.bump();
                stack.push(new_obj(Vec::new()));
            }
            Some('"') => {
                // A string is finished at its closing quote. A bad byte after
                // it is a separate error, and the string is still a value.
                let value = Value::String(self.parser.parse_string()?);
                self.emit_pair(&[], value, false);
                self.top_glue()?;
                return Ok(());
            }
            Some(_) => {
                let value = self.parser.scan_atom()?;
                self.top_glue()?;
                let atom = is_atom_value(&value);
                self.emit_pair(&[], value, atom);
                return Ok(());
            }
        }
        loop {
            let Some(mut frame) = stack.pop() else {
                return Ok(());
            };
            match self.step(&mut frame)? {
                Act::Keep => stack.push(frame),
                Act::Nest(child) => {
                    stack.push(frame);
                    stack.push(child);
                }
                Act::Done => {
                    if let Some(parent) = stack.last_mut() {
                        parent.note_child();
                    }
                }
            }
        }
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
            arr.started = true;
            if self.parser.peek() == Some(']') {
                self.parser.bump();
                self.emit_pair(&arr.path, Value::array(Vec::new()), false);
                return Ok(Act::Done);
            }
        }
        match self.parser.peek() {
            None => Err(self.parser.fail("Unfinished JSON term", true)),
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
                        arr.held = Some(Value::String(self.parser.parse_string()?));
                        arr.after_comma = false;
                        Ok(Act::Keep)
                    }
                    Some(_) => {
                        arr.held = Some(self.parser.scan_atom()?);
                        arr.after_comma = false;
                        Ok(Act::Keep)
                    }
                    None => Err(self.parser.fail("Unfinished JSON term", true)),
                }
            }
        }
    }

    fn step_object(&mut self, obj: &mut Obj) -> Result<Act, ParseErr> {
        if matches!(obj.stage, ObjStage::Sep) {
            return self.object_sep(obj);
        }
        self.parser.skip_ws();
        if !obj.started {
            obj.started = true;
            if self.parser.peek() == Some('}') {
                self.parser.bump();
                self.emit_pair(&obj.path, Value::object(Vec::new()), false);
                return Ok(Act::Done);
            }
        }
        let key = self.parse_key(obj.after_comma)?;
        self.parser.expect_colon()?;
        self.parser.skip_ws();
        match self.parser.peek() {
            None => Err(self.parser.fail("Unfinished JSON term", true)),
            Some('}') => Err(self.parser.bump_fail("Missing value in key:value pair")),
            Some(']') => Err(self
                .parser
                .bump_fail("Unmatched ']' in the middle of an object")),
            Some('[') => {
                obj.key = key;
                self.parser.bump();
                Ok(Act::Nest(new_arr(with_key(&obj.path, &obj.key))))
            }
            Some('{') => {
                obj.key = key;
                self.parser.bump();
                Ok(Act::Nest(new_obj(with_key(&obj.path, &obj.key))))
            }
            Some('"') => {
                obj.held = Some(Value::String(self.parser.parse_string()?));
                obj.key = key;
                obj.stage = ObjStage::Sep;
                Ok(Act::Keep)
            }
            Some(_) => {
                obj.held = Some(self.parser.scan_atom()?);
                obj.key = key;
                obj.stage = ObjStage::Sep;
                Ok(Act::Keep)
            }
        }
    }

    fn object_sep(&mut self, obj: &mut Obj) -> Result<Act, ParseErr> {
        self.parser.skip_ws();
        match self.parser.peek() {
            None => Err(self.parser.fail("Unfinished JSON term", true)),
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
        });
    }

    fn emit_closer(&mut self, path: &[Value]) {
        let item = Value::array(vec![Value::array(path.to_vec())]);
        self.items.push(RawItem {
            value: item,
            end: self.parser.index,
            atom: false,
        });
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
    located: &[(Value, usize)],
    detach: bool,
    error: Option<String>,
    bytes: &[u8],
) -> StreamFeed {
    let line = bytes.iter().filter(|byte| **byte == b'\n').count();
    if let Some(message) = error {
        if show_partial(bytes, located, &message) {
            let items = located.iter().map(|(value, _)| value.clone()).collect();
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
    if detach {
        let mut items: Vec<Value> = located.iter().map(|(value, _)| value.clone()).collect();
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
    let items = located.iter().map(|(value, _)| value.clone()).collect();
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
fn show_partial(bytes: &[u8], located: &[(Value, usize)], message: &str) -> bool {
    bytes.last() == Some(&b',')
        && message.starts_with("Unfinished JSON term at EOF")
        && !located.is_empty()
        && located.iter().all(|(value, _)| scalar_pair(value))
}

fn scalar_pair(value: &Value) -> bool {
    let Value::Array(items) = value else {
        return false;
    };
    items.len() == 2 && matches!(items[0], Value::Array(_)) && is_scalar(&items[1])
}
