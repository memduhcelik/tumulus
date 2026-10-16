use crate::value::Value;

/// How `.` is printed. Later layout flags replace earlier ones, as in jq.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub layout: Layout,
    pub raw: bool,
    pub join: bool,
    pub ascii: bool,
    pub sort_keys: bool,
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
        }
    }
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
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(raw) => crate::number::write_number(out, raw),
        Value::String(text) if raw_here && style.raw_strings() => out.push_str(text),
        Value::String(text) => push_string(out, text, style.ascii),
        Value::Array(items) => write_array(out, items, style, level, depth),
        Value::Object(items) => write_object(out, items, style, level, depth),
    }
}

fn write_array(out: &mut String, items: &[Value], style: Style, level: usize, depth: usize) {
    if items.is_empty() {
        out.push_str("[]");
        return;
    }
    if style.compact() {
        out.push('[');
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            write_value(out, item, style, 0, depth + 1, false);
        }
        out.push(']');
        return;
    }
    out.push_str("[\n");
    for (index, item) in items.iter().enumerate() {
        push_indent(out, style, level + 1);
        write_value(out, item, style, level + 1, depth + 1, false);
        if index + 1 != items.len() {
            out.push(',');
        }
        out.push('\n');
    }
    push_indent(out, style, level);
    out.push(']');
}

fn write_object(
    out: &mut String,
    items: &[(String, Value)],
    style: Style,
    level: usize,
    depth: usize,
) {
    if items.is_empty() {
        out.push_str("{}");
        return;
    }
    let mut order: Vec<usize> = (0..items.len()).collect();
    if style.sort_keys {
        order.sort_by(|&left, &right| items[left].0.cmp(&items[right].0));
    }
    if style.compact() {
        out.push('{');
        for (index, slot) in order.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let (key, value) = &items[*slot];
            push_string(out, key, style.ascii);
            out.push(':');
            write_value(out, value, style, 0, depth + 1, false);
        }
        out.push('}');
        return;
    }
    out.push_str("{\n");
    for (index, slot) in order.iter().enumerate() {
        push_indent(out, style, level + 1);
        let (key, value) = &items[*slot];
        push_string(out, key, style.ascii);
        out.push_str(": ");
        write_value(out, value, style, level + 1, depth + 1, false);
        if index + 1 != order.len() {
            out.push(',');
        }
        out.push('\n');
    }
    push_indent(out, style, level);
    out.push('}');
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
