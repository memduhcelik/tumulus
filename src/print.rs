use std::cell::RefCell;

use crate::value::Value;

thread_local! {
    static PALETTE: RefCell<[String; 8]> = RefCell::new(default_colors());
}

/// Replace the SGR parameters used when `style.color` is set.
pub fn set_palette(colors: [String; 8]) {
    PALETTE.with(|slot| *slot.borrow_mut() = colors);
}

pub(crate) fn reset_palette() {
    set_palette(default_colors());
}

/// How `.` is printed. Later layout flags replace earlier ones, as in jq.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub layout: Layout,
    pub raw: bool,
    pub join: bool,
    pub ascii: bool,
    pub sort_keys: bool,
    /// `-C`, or a terminal when `-M` was not given.
    pub color: bool,
    /// Prefix each JSON text with an ASCII record separator.
    pub seq: bool,
    /// `--raw-output0`: end each output with a NUL instead of a newline.
    pub nul: bool,
    /// `--unbuffered`: flush after each output.
    pub flush: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Compact,
    Pretty(Indent),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indent {
    Spaces(u8),
    Tabs,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            layout: Layout::Pretty(Indent::Spaces(2)),
            raw: false,
            join: false,
            ascii: false,
            sort_keys: false,
            color: false,
            seq: false,
            nul: false,
            flush: false,
        }
    }
}

/// jq's palette when `JQ_COLORS` is unset or empty.
pub fn default_colors() -> [String; 8] {
    [
        "0;90", "0;39", "0;39", "0;39", "0;32", "1;39", "1;39", "1;34",
    ]
    .map(str::to_string)
}

/// A colon-separated list. A trailing colon does not add an empty slot.
/// An empty string keeps the default palette. Digits and `;` are the only
/// characters a slot may hold.
pub fn parse_colors(text: &str) -> Option<[String; 8]> {
    if text.is_empty() {
        return Some(default_colors());
    }
    let mut parts: Vec<&str> = text.split(':').collect();
    if text.ends_with(':') {
        parts.pop();
    }
    let mut colors = default_colors();
    for (index, part) in parts.iter().enumerate() {
        if index >= colors.len() {
            break;
        }
        if !part
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b';')
        {
            return None;
        }
        colors[index] = (*part).to_string();
    }
    Some(colors)
}

impl Style {
    fn compact(self) -> bool {
        matches!(self.layout, Layout::Compact)
    }

    /// `-a` keeps JSON string quoting even when `-r` or `-j` is set.
    fn raw_strings(self) -> bool {
        self.raw && !self.ascii
    }
}

/// Empty arrays and objects stay on one line. Anything else breaks across
/// lines, indented the way the style asks.
/// jq replaces a value nested deeper than this. The top-level value is depth 0.
const DUMP_DEPTH: usize = 256;

pub fn render(value: &Value, style: Style) -> String {
    let mut out = String::new();
    // Raw output applies only to a top-level string, not to strings nested in JSON.
    write_value(&mut out, value, style, 0, 0, true);
    out
}

fn write_value(
    out: &mut String,
    value: &Value,
    style: Style,
    level: usize,
    depth: usize,
    raw_here: bool,
) {
    if depth > DUMP_DEPTH {
        out.push_str("<skipped: too deep>");
        return;
    }
    match value {
        Value::Null => paint_text(out, style, 0, "null"),
        Value::Bool(true) => paint_text(out, style, 2, "true"),
        Value::Bool(false) => paint_text(out, style, 1, "false"),
        Value::Number(raw) => {
            paint(out, style, 3);
            crate::number::write_number(out, raw);
            reset(out, style);
        }
        Value::String(text) if raw_here && style.raw_strings() => out.push_str(text),
        Value::String(text) => {
            paint(out, style, 4);
            push_string(out, text, style.ascii);
            reset(out, style);
        }
        Value::Array(items) => write_array(out, items, style, level, depth),
        Value::Object(items) => write_object(out, items, style, level, depth),
    }
}

fn write_array(out: &mut String, items: &[Value], style: Style, level: usize, depth: usize) {
    if items.is_empty() {
        paint(out, style, 5);
        out.push_str("[]");
        reset(out, style);
        return;
    }
    if style.compact() {
        paint(out, style, 5);
        out.push('[');
        for (index, item) in items.iter().enumerate() {
            write_value(out, item, style, 0, depth + 1, false);
            if index + 1 != items.len() {
                paint(out, style, 5);
                out.push(',');
            }
        }
        // jq sets the array color again before the closing bracket.
        paint(out, style, 5);
        paint(out, style, 5);
        out.push(']');
        reset(out, style);
        return;
    }
    paint(out, style, 5);
    out.push_str("[\n");
    for (index, item) in items.iter().enumerate() {
        push_indent(out, style, level + 1);
        write_value(out, item, style, level + 1, depth + 1, false);
        paint(out, style, 5);
        if index + 1 != items.len() {
            out.push(',');
        }
        out.push('\n');
    }
    push_indent(out, style, level);
    paint(out, style, 5);
    out.push(']');
    reset(out, style);
}

fn write_object(
    out: &mut String,
    items: &[(String, Value)],
    style: Style,
    level: usize,
    depth: usize,
) {
    if items.is_empty() {
        paint(out, style, 6);
        out.push_str("{}");
        reset(out, style);
        return;
    }
    let mut order: Vec<usize> = (0..items.len()).collect();
    if style.sort_keys {
        order.sort_by(|&left, &right| items[left].0.cmp(&items[right].0));
    }
    if style.compact() {
        paint(out, style, 6);
        out.push('{');
        reset(out, style);
        for (index, slot) in order.iter().enumerate() {
            if index > 0 {
                paint(out, style, 6);
                out.push(',');
                reset(out, style);
            }
            let (key, value) = &items[*slot];
            paint(out, style, 7);
            push_string(out, key, style.ascii);
            reset(out, style);
            paint(out, style, 6);
            out.push(':');
            reset(out, style);
            write_value(out, value, style, 0, depth + 1, false);
        }
        paint(out, style, 6);
        paint(out, style, 6);
        out.push('}');
        reset(out, style);
        return;
    }
    paint(out, style, 6);
    out.push_str("{\n");
    for (index, slot) in order.iter().enumerate() {
        push_indent(out, style, level + 1);
        reset(out, style);
        let (key, value) = &items[*slot];
        paint(out, style, 7);
        push_string(out, key, style.ascii);
        reset(out, style);
        paint(out, style, 6);
        out.push_str(": ");
        reset(out, style);
        write_value(out, value, style, level + 1, depth + 1, false);
        paint(out, style, 6);
        if index + 1 != order.len() {
            out.push(',');
        }
        out.push('\n');
    }
    push_indent(out, style, level);
    paint(out, style, 6);
    out.push('}');
    reset(out, style);
}

fn paint_text(out: &mut String, style: Style, slot: usize, text: &str) {
    paint(out, style, slot);
    out.push_str(text);
    reset(out, style);
}

fn paint(out: &mut String, style: Style, slot: usize) {
    if style.color {
        let code = PALETTE.with(|palette| palette.borrow()[slot].clone());
        out.push('\u{1b}');
        out.push('[');
        out.push_str(&code);
        out.push('m');
    }
}

fn reset(out: &mut String, style: Style) {
    if style.color {
        out.push_str("\u{1b}[0m");
    }
}

fn push_indent(out: &mut String, style: Style, level: usize) {
    match style.layout {
        Layout::Compact => {}
        Layout::Pretty(Indent::Spaces(width)) => {
            out.extend(std::iter::repeat_n(' ', usize::from(width) * level));
        }
        Layout::Pretty(Indent::Tabs) => {
            out.extend(std::iter::repeat_n('\t', level));
        }
    }
}

fn push_string(out: &mut String, text: &str, ascii: bool) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{000c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c == '\u{007f}' || (ascii && !c.is_ascii()) => {
                push_u_escape(out, c);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn push_u_escape(out: &mut String, c: char) {
    let mut buf = [0u16; 2];
    for unit in c.encode_utf16(&mut buf) {
        out.push_str(&format!("\\u{unit:04x}"));
    }
}
