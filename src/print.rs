use crate::value::Value;

/// Empty arrays and objects stay on one line. Anything else breaks across
/// lines with a two-space indent, matching jq's default layout.
pub fn render(value: &Value, compact: bool) -> String {
    let mut out = String::new();
    write_value(&mut out, value, compact, 0);
    out
}

fn write_value(out: &mut String, value: &Value, compact: bool, indent: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(raw) => out.push_str(raw),
        Value::String(text) => push_string(out, text),
        Value::Array(items) => write_array(out, items, compact, indent),
        Value::Object(items) => write_object(out, items, compact, indent),
    }
}

fn write_array(out: &mut String, items: &[Value], compact: bool, indent: usize) {
    if items.is_empty() {
        out.push_str("[]");
        return;
    }
    if compact {
        out.push('[');
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            write_value(out, item, true, 0);
        }
        out.push(']');
        return;
    }
    out.push_str("[\n");
    for (index, item) in items.iter().enumerate() {
        push_indent(out, indent + 2);
        write_value(out, item, false, indent + 2);
        if index + 1 != items.len() {
            out.push(',');
        }
        out.push('\n');
    }
    push_indent(out, indent);
    out.push(']');
}

fn write_object(out: &mut String, items: &[(String, Value)], compact: bool, indent: usize) {
    if items.is_empty() {
        out.push_str("{}");
        return;
    }
    if compact {
        out.push('{');
        for (index, (key, value)) in items.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            push_string(out, key);
            out.push(':');
            write_value(out, value, true, 0);
        }
        out.push('}');
        return;
    }
    out.push_str("{\n");
    for (index, (key, value)) in items.iter().enumerate() {
        push_indent(out, indent + 2);
        push_string(out, key);
        out.push_str(": ");
        write_value(out, value, false, indent + 2);
        if index + 1 != items.len() {
            out.push(',');
        }
        out.push('\n');
    }
    push_indent(out, indent);
    out.push('}');
}

fn push_indent(out: &mut String, spaces: usize) {
    out.extend(std::iter::repeat_n(' ', spaces));
}

fn push_string(out: &mut String, text: &str) {
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
            c if (c as u32) < 0x20 || c == '\u{007f}' => {
                out.push_str("\\u");
                out.push_str(&format!("{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
