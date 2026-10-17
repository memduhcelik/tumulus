//! The input shared by the main loop and by `input` / `inputs`.
//!
//! jq reads the next value when the program asks for it. A missing file is
//! reported and skipped. A value that is not valid JSON fails that read.
//! `input` turns the end of the stream into `break`. `inputs` stops there.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, Read as IoRead};

use crate::parse::{
    closer_is_detached, nul_cut, program_stream_with, pull_stream, scan_input, stream_line,
    LineCursor, StreamState,
};
use crate::value::Value;
use crate::Error;

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    static ENVIRON: RefCell<Option<Value>> = const { RefCell::new(None) };
}

/// What `input` got from the stream.
pub enum Read {
    Value(Value),
    /// The stream is finished. `inputs` stops. `input` fails with `break`.
    Break,
    /// The next value was not valid JSON. The message is jq's parse text.
    Parse(String),
    /// A file could not be read for a reason other than being missing.
    Fatal(String),
}

pub struct Source {
    pub files: Vec<String>,
    pub raw: bool,
    pub slurp: bool,
    /// `--stream`. Raw input ignores this and keeps one string per line.
    pub stream: bool,
    /// `--stream-errors`. A parse error is one more value, and parsing goes on.
    pub stream_errors: bool,
}

pub struct SourceGuard;

impl Drop for SourceGuard {
    fn drop(&mut self) {
        SESSION.with(|slot| *slot.borrow_mut() = None);
    }
}

pub fn bind_source(source: Source) -> SourceGuard {
    SESSION.with(|slot| *slot.borrow_mut() = Some(Session::open(source)));
    SourceGuard
}

pub fn set_environ(value: Value) {
    ENVIRON.with(|slot| *slot.borrow_mut() = Some(value));
}

pub fn environ_now() -> Value {
    environment()
}

pub fn environ() -> Value {
    ENVIRON.with(|slot| slot.borrow().clone().unwrap_or_else(environment))
}

pub fn here(line: usize) -> Value {
    Value::object(vec![
        ("file".to_string(), Value::String("<top-level>".to_string())),
        ("line".to_string(), Value::Number(line.to_string())),
    ])
}

pub fn input_filename() -> Value {
    with(|session| match &session.yielded {
        Some(name) => Value::String(name.clone()),
        None => Value::Null,
    })
    .unwrap_or(Value::Null)
}

pub fn set_unknown() {
    let _ = with_mut(|session| session.at = "<unknown>".to_string());
}

pub fn location() -> Option<String> {
    with(|session| session.at.clone())
}

pub fn open_failed() -> bool {
    with(|session| session.missing).unwrap_or(false)
}

/// `--stream` is on, so each value is written before the next read.
pub fn streaming() -> bool {
    with(|session| session.stream).unwrap_or(false)
}

pub fn read_one() -> Read {
    with_mut(|session| match session.pull(true) {
        Ok(Pulled::Value(value)) => Read::Value(value),
        Ok(Pulled::Parse(message)) => Read::Parse(message),
        Ok(Pulled::End) => {
            session.mark_break();
            Read::Break
        }
        Err(message) => Read::Fatal(message),
    })
    .unwrap_or(Read::Break)
}

pub fn next_main() -> Result<MainNext, Error> {
    let Some(next) = with_mut(|session| -> Result<MainNext, String> {
        if session.halt_main {
            return Ok(MainNext::Eof);
        }
        let pulled = session.pull(false)?;
        if session.arm_stop {
            session.halt_main = true;
        }
        Ok(match pulled {
            Pulled::Value(value) => MainNext::Value(value),
            Pulled::Parse(message) => MainNext::Parse(message),
            Pulled::End => MainNext::Eof,
        })
    }) else {
        return Ok(MainNext::Eof);
    };
    next.map_err(Error::new)
}

pub enum MainNext {
    Value(Value),
    Parse(String),
    Eof,
}

enum Pulled {
    Value(Value),
    Parse(String),
    End,
}

enum Opened {
    Data(Vec<u8>),
    Missing(String),
    Directory,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum More {
    Added,
    Eof,
}

struct Chunk {
    name: String,
    /// First byte of this file in the parse buffer.
    parse_start: usize,
    raw: Vec<u8>,
    /// Bytes skipped at the start of `raw` before they were parsed.
    bom: usize,
    /// Newlines already counted in `raw`. Later values continue from here.
    lines: LineCursor,
}

struct Item {
    value: Value,
    name: String,
    line: usize,
    /// A final scalar closer. jq names that error `<unknown>`.
    unknown: bool,
}

struct Fault {
    message: String,
    name: String,
    line: usize,
    /// `input` reports this parse error at `<unknown>`.
    unknown: bool,
}

struct Cur {
    name: String,
    line: usize,
    any_byte: bool,
    ended_nl: bool,
}

struct Session {
    files: Vec<String>,
    file_at: usize,
    raw: bool,
    slurp: bool,
    stream: bool,
    stream_errors: bool,
    buf: Vec<u8>,
    chunks: Vec<Chunk>,
    scan_at: usize,
    scan_line: usize,
    pending: VecDeque<Item>,
    fault: Option<Fault>,
    /// Where to continue after the program catches a bad value.
    staged: Option<(usize, usize)>,
    yielded: Option<String>,
    at: String,
    cur: Option<Cur>,
    eof_once: bool,
    missing: bool,
    dir_pending: bool,
    /// A directory noticed while the main loop slurped, not yet printed.
    dir_from_main: bool,
    /// The file that failed just before a value ran out of bytes.
    failed_here: Option<String>,
    arm_stop: bool,
    halt_main: bool,
    slurp_done: bool,
    raw_at: usize,
    dead: bool,
    input_eof: bool,
    blocked: bool,
    live: Option<Live>,
    /// Chunk that the open file is still extending.
    live_chunk: Option<usize>,
    staging: VecDeque<Staged>,
    drive: StreamState,
    /// The stream value released just before the one still held.
    prev: Option<Value>,
}

struct Staged {
    value: Value,
    end: usize,
    atom: bool,
    unknown: bool,
}

enum Live {
    Stdin,
    File(File),
}

impl Session {
    fn open(source: Source) -> Self {
        let files = if source.files.is_empty() {
            vec!["-".to_string()]
        } else {
            source.files
        };
        Self {
            files,
            file_at: 0,
            raw: source.raw,
            slurp: source.slurp,
            stream: source.stream,
            stream_errors: source.stream_errors,
            buf: Vec::new(),
            chunks: Vec::new(),
            scan_at: 0,
            scan_line: 1,
            pending: VecDeque::new(),
            fault: None,
            staged: None,
            yielded: None,
            at: "<unknown>".to_string(),
            cur: None,
            eof_once: false,
            missing: false,
            dir_pending: false,
            dir_from_main: false,
            failed_here: None,
            arm_stop: false,
            halt_main: false,
            slurp_done: false,
            raw_at: 0,
            dead: false,
            input_eof: false,
            blocked: false,
            live: None,
            live_chunk: None,
            staging: VecDeque::new(),
            drive: StreamState::default(),
            prev: None,
        }
    }

    fn pull(&mut self, from_input: bool) -> Result<Pulled, String> {
        if from_input && self.dir_pending && self.dir_from_main {
            self.flush_dir();
        }
        if self.dead {
            return Ok(Pulled::End);
        }
        if self.slurp {
            return self.pull_slurp(from_input);
        }
        if self.raw {
            return self.pull_raw();
        }
        self.pull_json(from_input)
    }

    fn pull_json(&mut self, recover: bool) -> Result<Pulled, String> {
        loop {
            if let Some(item) = self.pending.pop_front() {
                if item.unknown {
                    self.yielded = Some(item.name);
                    self.at = "<unknown>".to_string();
                } else {
                    self.note(&item.name, item.line);
                }
                return Ok(Pulled::Value(item.value));
            }
            if let Some(fault) = self.fault.take() {
                if fault.unknown {
                    self.yielded = Some(fault.name);
                    self.at = "<unknown>".to_string();
                } else {
                    self.note(&fault.name, fault.line);
                }
                if recover {
                    if let Some((at, line)) = self.staged.take() {
                        self.scan_at = at;
                        self.scan_line = line;
                    }
                } else {
                    self.dead = true;
                }
                return Ok(Pulled::Parse(fault.message));
            }
            if !self.fill_json()? {
                return Ok(Pulled::End);
            }
        }
    }

    /// Parse what is buffered. Returns true when a value or an error is waiting.
    fn fill_json(&mut self) -> Result<bool, String> {
        if !self.pending.is_empty() || self.fault.is_some() {
            return Ok(true);
        }
        if self.stream {
            return self.fill_stream();
        }
        loop {
            let scan = scan_input(&self.buf, self.scan_at, self.scan_line);
            for (value, end) in scan.values {
                let (name, line) = self.place(end, is_atom(&value));
                self.pending.push_back(Item {
                    value,
                    name,
                    line,
                    unknown: false,
                });
            }
            if scan.incomplete {
                self.scan_at = scan.restart;
                self.scan_line = scan.restart_line;
                if self.append()? == More::Added {
                    continue;
                }
                let (name, line) = if let Some(name) = self.failed_here.clone() {
                    (name, 0)
                } else {
                    self.place(scan.error_at, false)
                };
                self.fault = Some(Fault {
                    message: scan.error.unwrap_or_else(|| "parse error".to_string()),
                    name,
                    line,
                    unknown: false,
                });
                return Ok(true);
            }
            if let Some(message) = scan.error {
                let (name, line) = self.place(scan.error_at, false);
                self.fault = Some(Fault {
                    message,
                    name,
                    line,
                    unknown: false,
                });
                self.staged = Some((scan.resume, scan.resume_line));
                return Ok(true);
            }
            self.scan_at = scan.resume;
            self.scan_line = scan.resume_line;
            if !self.pending.is_empty() {
                return Ok(true);
            }
            if self.append()? == More::Eof {
                return Ok(false);
            }
        }
    }

    /// `--stream` parses the bytes already read and returns as soon as one
    /// item can be printed. A later read can still change a line that jq's
    /// 4095-byte read has not finished, so that item stays staged.
    fn fill_stream(&mut self) -> Result<bool, String> {
        loop {
            self.pull_available();
            self.release();
            if !self.pending.is_empty() || self.fault.is_some() {
                return Ok(true);
            }
            if self.input_eof {
                return Ok(false);
            }
            match self.append()? {
                More::Added => {}
                More::Eof => self.input_eof = true,
            }
        }
    }

    fn pull_available(&mut self) {
        if self.fault.is_some() {
            return;
        }
        let end = nul_cut(&self.buf);
        if self.drive.index > end {
            self.drive.index = end;
        }
        let bounds: Vec<usize> = self
            .chunks
            .iter()
            .skip(1)
            .map(|chunk| chunk.parse_start)
            .filter(|start| *start < end)
            .collect();
        let out = pull_stream(&self.buf[..end], &mut self.drive, self.input_eof, &bounds);
        for (value, at, atom, unknown) in out.items {
            self.staging.push_back(Staged {
                value,
                end: at,
                atom,
                unknown,
            });
        }
        self.blocked = out.blocked;
        if out.blocked {
            return;
        }
        let Some(message) = out.error else {
            return;
        };
        if self.stream_errors {
            let unknown = message.starts_with("Unfinished JSON term at EOF")
                && !self.buf[..end].contains(&b'\n')
                && !matches!(self.buf[..end].last(), Some(b' ' | b'\t' | b'\n' | b'\r'))
                && (self.prev.is_some() || !self.staging.is_empty() || !self.pending.is_empty());
            let path = Value::array(out.path);
            self.staging.push_back(Staged {
                value: Value::array(vec![Value::String(message), path]),
                end: out.error_at.min(end),
                atom: false,
                unknown,
            });
            return;
        }
        self.release();
        let (name, line) = if out.incomplete {
            if let Some(name) = self.failed_here.clone() {
                (name, 0)
            } else {
                self.place(out.error_at.min(end), false)
            }
        } else {
            self.place(out.error_at.min(end), false)
        };
        let visible = &self.buf[..nul_cut(&self.buf)];
        let prior = self.prev.is_some() || !self.pending.is_empty();
        let unknown = out.incomplete
            && prior
            && message.starts_with("Unfinished JSON term at EOF")
            && !visible.contains(&b'\n')
            && !matches!(visible.last(), Some(b' ' | b'\t' | b'\n' | b'\r'));
        self.fault = Some(Fault {
            message,
            name,
            line,
            unknown,
        });
        if !out.incomplete {
            self.staged = Some((out.resume, out.resume_line));
        }
        self.blocked = true;
    }

    fn release(&mut self) {
        let finished = self.input_eof && !self.blocked && self.drive.stack_empty();
        // A parse error stops the input. A closer staged before that error
        // has already happened, so it is not still waiting to see EOF.
        let flush = finished || self.fault.is_some();
        while let Some(front) = self.staging.front() {
            let hold = !flush && self.staging.len() == 1 && self.might_detach(&front.value);
            if hold || !self.item_ready(front.end, front.atom) {
                break;
            }
            let item = self.staging.pop_front().expect("staged item");
            let mut unknown = item.unknown;
            if finished && self.staging.is_empty() {
                unknown = unknown || self.pair_detach(&item.value);
            }
            let (name, line) = self.place(item.end, item.atom);
            self.prev = Some(item.value.clone());
            self.pending.push_back(Item {
                value: item.value,
                name,
                line,
                unknown,
            });
        }
    }

    fn item_ready(&self, end: usize, atom: bool) -> bool {
        let Some(index) = self.chunk_index(end) else {
            return true;
        };
        let chunk = &self.chunks[index];
        let raw_end = chunk.bom + end.saturating_sub(chunk.parse_start);
        let file_done = self.input_eof || self.live_chunk != Some(index);
        chunk.lines.ready(&chunk.raw, raw_end, atom, file_done)
    }

    fn might_detach(&self, value: &Value) -> bool {
        let Value::Array(closer) = value else {
            return false;
        };
        closer.len() == 1
    }

    fn pair_detach(&self, value: &Value) -> bool {
        let Some(prev) = &self.prev else {
            return false;
        };
        let end = nul_cut(&self.buf);
        closer_is_detached(&[prev.clone(), value.clone()], &self.buf[..end])
    }

    fn pull_raw(&mut self) -> Result<Pulled, String> {
        let mut pending = Vec::new();
        loop {
            if self.raw_at >= self.buf.len() {
                if self.append()? == More::Eof {
                    if pending.is_empty() {
                        return Ok(Pulled::End);
                    }
                    let line = self.cur.as_ref().map(|cur| cur.line).unwrap_or(0);
                    let name = self.name_at(self.raw_at);
                    self.note(&name, line);
                    return Ok(Pulled::Value(Value::String(decode(&pending))));
                }
                continue;
            }
            let cut = self.buf[self.raw_at..]
                .iter()
                .position(|byte| *byte == b'\n');
            let Some(cut) = cut else {
                pending.extend_from_slice(&self.buf[self.raw_at..]);
                self.raw_at = self.buf.len();
                continue;
            };
            pending.extend_from_slice(&self.buf[self.raw_at..self.raw_at + cut]);
            self.raw_at += cut + 1;
            let (name, line) = self.place(self.raw_at, false);
            let text = decode(&pending);
            self.note(&name, line);
            return Ok(Pulled::Value(Value::String(text)));
        }
    }

    fn pull_slurp(&mut self, from_input: bool) -> Result<Pulled, String> {
        if self.slurp_done {
            return self.pop_slurp();
        }
        while self.append()? == More::Added {}
        if from_input {
            self.flush_dir();
        } else if self.dir_pending {
            self.dir_from_main = true;
        }
        self.slurp_done = true;
        if self.raw {
            let name = self.cur.as_ref().map(|cur| cur.name.clone());
            let line = self.cur.as_ref().map(|cur| cur.line).unwrap_or(0);
            if let Some(name) = name {
                self.note(&name, line);
            }
            return Ok(Pulled::Value(Value::String(decode(&self.buf))));
        }
        if self.stream {
            self.load_stream_slurp();
            return self.pop_slurp();
        }
        let scan = scan_input(&self.buf, 0, 1);
        if let Some(message) = scan.error {
            let (name, line) = if scan.incomplete {
                if let Some(name) = self.failed_here.clone() {
                    (name, 0)
                } else {
                    self.place(scan.error_at, false)
                }
            } else {
                self.place(scan.error_at, false)
            };
            self.note(&name, line);
            return Ok(Pulled::Parse(message));
        }
        let items: Vec<Value> = scan.values.into_iter().map(|(value, _)| value).collect();
        if let Some((name, line)) = self.cur.as_ref().map(|cur| (cur.name.clone(), cur.line)) {
            self.note(&name, line);
        }
        Ok(Pulled::Value(Value::array(items)))
    }

    fn load_stream_slurp(&mut self) {
        let bounds: Vec<usize> = self
            .chunks
            .iter()
            .skip(1)
            .map(|chunk| chunk.parse_start)
            .collect();
        let fed = program_stream_with(&self.buf, true, self.stream_errors, &bounds);
        let name = self
            .cur
            .as_ref()
            .map(|cur| cur.name.clone())
            .unwrap_or_else(|| "<stdin>".to_string());
        for value in fed.values {
            self.pending.push_back(Item {
                value: value.value,
                name: name.clone(),
                line: value.line,
                unknown: value.unknown,
            });
        }
        if let Some(message) = fed.error {
            self.fault = Some(Fault {
                message,
                name,
                line: 0,
                unknown: false,
            });
        }
    }

    fn pop_slurp(&mut self) -> Result<Pulled, String> {
        if let Some(item) = self.pending.pop_front() {
            if item.unknown {
                self.yielded = Some(item.name);
                self.at = "<unknown>".to_string();
            } else {
                self.note(&item.name, item.line);
            }
            return Ok(Pulled::Value(item.value));
        }
        if let Some(fault) = self.fault.take() {
            self.note(&fault.name, fault.line);
            return Ok(Pulled::Parse(fault.message));
        }
        Ok(Pulled::End)
    }

    fn append(&mut self) -> Result<More, String> {
        if self.stream && !self.raw && !self.slurp {
            return self.append_stream();
        }
        self.append_whole()
    }

    /// Read the next piece of a file. A value can still run on into the next file.
    fn append_stream(&mut self) -> Result<More, String> {
        loop {
            if self.live.is_none() {
                if self.file_at >= self.files.len() {
                    return Ok(More::Eof);
                }
                let path = self.files[self.file_at].clone();
                self.file_at += 1;
                let name = source_name(&path);
                match open_live(&path)? {
                    LiveOpen::Reader(live) => self.live = Some(live),
                    LiveOpen::Missing(reason) => {
                        eprintln!("jq: error: Could not open file {name}: {reason}");
                        self.missing = true;
                        self.failed_here = Some(name.clone());
                        self.remember_failed(&name);
                        continue;
                    }
                    LiveOpen::Directory => {
                        if self.dir_pending {
                            self.flush_dir();
                        }
                        self.dir_pending = true;
                        self.missing = true;
                        self.failed_here = Some(name.clone());
                        self.remember_failed(&name);
                        continue;
                    }
                }
            }
            let mut tmp = [0u8; 4096];
            let read = match self.live.as_mut().expect("open stream") {
                Live::Stdin => IoRead::read(&mut io::stdin(), &mut tmp),
                Live::File(file) => IoRead::read(file, &mut tmp),
            };
            let name = self.live_name();
            let n = read.map_err(|err| format!("cannot read {name}: {err}"))?;
            if n == 0 {
                if let Some(index) = self.live_chunk {
                    let name = self.chunks[index].name.clone();
                    let raw = self.chunks[index].raw.clone();
                    self.remember(&name, &raw);
                }
                self.live = None;
                self.live_chunk = None;
                if self.file_at >= self.files.len() {
                    return Ok(More::Eof);
                }
                continue;
            }
            let name = match self.live_chunk {
                Some(index) => self.chunks[index].name.clone(),
                None => self.live_name(),
            };
            self.push_stream_bytes(&name, &tmp[..n]);
            return Ok(More::Added);
        }
    }

    fn live_name(&self) -> String {
        let index = self.file_at.saturating_sub(1);
        self.files
            .get(index)
            .map(|path| source_name(path))
            .unwrap_or_else(|| "<stdin>".to_string())
    }

    fn push_stream_bytes(&mut self, name: &str, bytes: &[u8]) {
        if self.live_chunk.is_none() {
            if self.dir_pending {
                self.flush_dir();
            }
            self.failed_here = None;
            let bom = if self.buf.is_empty() && bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
                3
            } else {
                0
            };
            let bom = bom.min(bytes.len());
            let parse_start = self.buf.len();
            self.buf.extend_from_slice(&bytes[bom..]);
            self.chunks.push(Chunk {
                name: name.to_string(),
                parse_start,
                raw: bytes.to_vec(),
                bom,
                lines: LineCursor::default(),
            });
            self.live_chunk = Some(self.chunks.len() - 1);
            return;
        }
        let index = self.live_chunk.expect("live chunk");
        self.chunks[index].raw.extend_from_slice(bytes);
        self.buf.extend_from_slice(bytes);
    }

    fn append_whole(&mut self) -> Result<More, String> {
        loop {
            if self.file_at >= self.files.len() {
                return Ok(More::Eof);
            }
            let path = self.files[self.file_at].clone();
            self.file_at += 1;
            let name = source_name(&path);
            match read_path(&path)? {
                Opened::Data(bytes) => {
                    if self.dir_pending {
                        self.flush_dir();
                    }
                    self.failed_here = None;
                    self.remember(&name, &bytes);
                    let bom = if self.buf.is_empty() && bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
                        3
                    } else {
                        0
                    };
                    // Raw input keeps every byte. JSON parsing skips one leading BOM.
                    let bom = if self.raw { 0 } else { bom };
                    let parse_start = self.buf.len();
                    self.buf.extend_from_slice(&bytes[bom..]);
                    self.chunks.push(Chunk {
                        name,
                        parse_start,
                        raw: bytes,
                        bom,
                        lines: LineCursor::default(),
                    });
                    return Ok(More::Added);
                }
                Opened::Missing(reason) => {
                    eprintln!("jq: error: Could not open file {name}: {reason}");
                    self.missing = true;
                    self.failed_here = Some(name.clone());
                    self.remember_failed(&name);
                    if self.raw {
                        self.arm_stop = true;
                    }
                }
                Opened::Directory => {
                    if self.dir_pending {
                        self.flush_dir();
                    }
                    self.dir_pending = true;
                    self.missing = true;
                    self.failed_here = Some(name.clone());
                    self.remember_failed(&name);
                    if self.raw {
                        self.arm_stop = true;
                    }
                }
            }
        }
    }

    fn flush_dir(&mut self) {
        if self.dir_pending {
            eprintln!("jq: error: Is a directory");
            self.dir_pending = false;
            self.dir_from_main = false;
        }
    }

    fn remember(&mut self, name: &str, bytes: &[u8]) {
        self.cur = Some(Cur {
            name: name.to_string(),
            line: stream_line(bytes, bytes.len(), false),
            any_byte: !bytes.is_empty(),
            ended_nl: bytes.last() == Some(&b'\n'),
        });
    }

    fn remember_failed(&mut self, name: &str) {
        self.cur = Some(Cur {
            name: name.to_string(),
            line: 0,
            any_byte: false,
            ended_nl: false,
        });
    }

    fn place(&mut self, end: usize, atom: bool) -> (String, usize) {
        let Some(index) = self.chunk_index(end) else {
            return ("<stdin>".to_string(), 0);
        };
        let chunk = &mut self.chunks[index];
        let raw_end = chunk.bom + end.saturating_sub(chunk.parse_start);
        let line = chunk.lines.line_at(&chunk.raw, raw_end, atom);
        (chunk.name.clone(), line)
    }

    fn name_at(&self, end: usize) -> String {
        let name = match self.chunk_index(end) {
            Some(index) => self.chunks[index].name.clone(),
            None => "<stdin>".to_string(),
        };
        // An empty path leaves this byte with no name. Use the file this
        // read already recorded.
        if !name.is_empty() {
            return name;
        }
        self.cur
            .as_ref()
            .map(|cur| cur.name.clone())
            .unwrap_or_else(|| "<stdin>".to_string())
    }

    fn chunk_index(&self, end: usize) -> Option<usize> {
        if self.chunks.is_empty() {
            return None;
        }
        let found = self
            .chunks
            .iter()
            .enumerate()
            .rev()
            .find(|(_, chunk)| end > chunk.parse_start && end <= chunk.parse_start + chunk.stored())
            .map(|(index, _)| index);
        Some(found.unwrap_or(self.chunks.len() - 1))
    }

    fn note(&mut self, name: &str, line: usize) {
        self.yielded = Some(name.to_string());
        self.at = format!("{name}:{line}");
    }

    fn mark_break(&mut self) {
        self.at = if self.eof_once {
            "<unknown>".to_string()
        } else {
            self.eof_once = true;
            match &self.cur {
                Some(cur) if !(cur.any_byte && !cur.ended_nl) => {
                    format!("{}:{}", cur.name, cur.line)
                }
                _ => "<unknown>".to_string(),
            }
        };
    }
}

impl Chunk {
    fn stored(&self) -> usize {
        self.raw.len().saturating_sub(self.bom)
    }
}

fn with<T>(read: impl FnOnce(&Session) -> T) -> Option<T> {
    SESSION.with(|slot| slot.borrow().as_ref().map(read))
}

fn with_mut<T>(write: impl FnOnce(&mut Session) -> T) -> Option<T> {
    SESSION.with(|slot| slot.borrow_mut().as_mut().map(write))
}

fn source_name(path: &str) -> String {
    if path == "-" {
        "<stdin>".to_string()
    } else {
        path.to_string()
    }
}

enum LiveOpen {
    Reader(Live),
    Missing(String),
    Directory,
}

fn open_live(path: &str) -> Result<LiveOpen, String> {
    if path == "-" {
        return Ok(LiveOpen::Reader(Live::Stdin));
    }
    match File::open(path) {
        Ok(file) => {
            if fs::metadata(path).is_ok_and(|meta| meta.is_dir()) {
                return Ok(LiveOpen::Directory);
            }
            Ok(LiveOpen::Reader(Live::File(file)))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(LiveOpen::Missing(os_reason(&err))),
        Err(err) if is_directory(path, &err) => Ok(LiveOpen::Directory),
        Err(err) => Err(format!("cannot read {path}: {err}")),
    }
}

fn read_path(path: &str) -> Result<Opened, String> {
    if path == "-" {
        let mut buf = Vec::new();
        io::Read::read_to_end(&mut io::stdin(), &mut buf)
            .map_err(|err| format!("cannot read -: {err}"))?;
        return Ok(Opened::Data(buf));
    }
    match fs::read(path) {
        Ok(bytes) => Ok(Opened::Data(bytes)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Opened::Missing(os_reason(&err))),
        Err(err) if is_directory(path, &err) => Ok(Opened::Directory),
        Err(err) => Err(format!("cannot read {path}: {err}")),
    }
}

fn is_directory(path: &str, err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::IsADirectory
        || err.raw_os_error() == Some(21)
        || fs::metadata(path).is_ok_and(|meta| meta.is_dir())
}

fn os_reason(err: &io::Error) -> String {
    let text = err.to_string();
    match text.rfind(" (os error") {
        Some(end) => text[..end].to_string(),
        None => text,
    }
}

fn decode(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn is_atom(value: &Value) -> bool {
    matches!(value, Value::Null | Value::Bool(_) | Value::Number(_))
}

fn environment() -> Value {
    let fields = std::env::vars()
        .map(|(key, value)| (key, Value::String(value)))
        .collect();
    Value::object(fields)
}
