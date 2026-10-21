//! `test`, `match`, and `capture` search with Oniguruma's Perl NG syntax.
//!
//! jq's `match` is `_match_impl(...) | .[]`. The array is what `path` tries
//! to iterate. A flag letter here is the same letter jq accepts. `g` walks
//! matches. The other letters are compile options. An empty match advances
//! one byte, which is why a multibyte character can report the same code-point
//! offset more than once.

use onig::{MatchParam, Regex, RegexOptions, Region, SearchOptions, Syntax};

use crate::value::Value;

use super::ast::Failure;
use super::eval::{self, fail};

/// Every non-overlapping copy of `pattern` in a string, replaced by `replacement`.
/// `pattern` is not empty and has no regular-expression syntax. A value that
/// is not a string fails the way `match` fails.
pub(super) fn gsub_literal(
    input: &Value,
    pattern: &str,
    replacement: &str,
) -> Result<Vec<Value>, Failure> {
    let Value::String(text) = input else {
        return Err(fail(format!(
            "{} ({}) cannot be matched, as it is not a string",
            input.type_name(),
            eval::shown(input)
        )));
    };
    let text = text.as_str();
    let Some(first) = text.find(pattern) else {
        return Ok(vec![input.clone()]);
    };
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..first]);
    out.push_str(replacement);
    let mut rest = &text[first + pattern.len()..];
    while let Some(at) = rest.find(pattern) {
        out.push_str(&rest[..at]);
        out.push_str(replacement);
        rest = &rest[at + pattern.len()..];
    }
    out.push_str(rest);
    Ok(vec![Value::String(crate::value::Text::from(out))])
}

pub(super) fn apply_pair(
    subject: &Value,
    pattern: &Value,
    flags: &Value,
    testing: bool,
) -> Result<Value, Failure> {
    let text = match subject {
        Value::String(text) => text.as_str(),
        other => {
            return Err(fail(format!(
                "{} ({}) cannot be matched, as it is not a string",
                other.type_name(),
                eval::shown(other)
            )))
        }
    };
    let pattern = match pattern {
        Value::String(text) => text.as_str(),
        other => return Err(not_string(other)),
    };
    let flags = match flags {
        Value::Null => "",
        Value::String(text) => text.as_str(),
        other => return Err(not_string(other)),
    };
    let (global, options) = flag_options(flags)?;
    let regex = compile(pattern, options)?;
    // Longest and not-empty are read while searching. `SearchOptions` does
    // not list those bits, so they have to be kept on the raw value.
    let search = SearchOptions::from_bits_retain(options.bits());
    if testing {
        return Ok(Value::Bool(has_match(&regex, text, search)));
    }
    Ok(Value::array(all_matches(&regex, text, global, search)))
}

fn not_string(value: &Value) -> Failure {
    fail(format!(
        "{} ({}) is not a string",
        value.type_name(),
        eval::shown(value)
    ))
}

fn flag_options(flags: &str) -> Result<(bool, RegexOptions), Failure> {
    let mut global = false;
    let mut options = RegexOptions::REGEX_OPTION_CAPTURE_GROUP;
    for ch in flags.chars() {
        match ch {
            'g' => global = true,
            'i' => options |= RegexOptions::REGEX_OPTION_IGNORECASE,
            'm' => options |= RegexOptions::REGEX_OPTION_MULTILINE,
            'n' => options |= RegexOptions::REGEX_OPTION_FIND_NOT_EMPTY,
            'p' => {
                options |=
                    RegexOptions::REGEX_OPTION_MULTILINE | RegexOptions::REGEX_OPTION_SINGLELINE
            }
            's' => options |= RegexOptions::REGEX_OPTION_SINGLELINE,
            'l' => options |= RegexOptions::REGEX_OPTION_FIND_LONGEST,
            'x' => options |= RegexOptions::REGEX_OPTION_EXTEND,
            _ => return Err(fail(format!("{flags} is not a valid modifier string"))),
        }
    }
    Ok((global, options))
}

fn compile(pattern: &str, options: RegexOptions) -> Result<Regex, Failure> {
    // Perl NG names groups. `(?-i)` needs the Perl option operator as well.
    let mut syntax = *Syntax::perl_ng();
    syntax.enable_operators(onig::SyntaxOperator::SYNTAX_OPERATOR_OPTION_PERL);
    match Regex::with_options(pattern, options, &syntax) {
        Ok(regex) => Ok(regex),
        Err(err) => Err(fail(format!("Regex failure: {}", err.description()))),
    }
}

fn has_match(regex: &Regex, text: &str, search: SearchOptions) -> bool {
    let mut region = Region::new();
    let found = regex
        .search_with_param(
            text,
            0,
            text.len(),
            search,
            Some(&mut region),
            MatchParam::default(),
        )
        .ok()
        .flatten();
    let Some(_) = found else {
        return false;
    };
    match region.pos(0) {
        Some((from, to))
            if from == to
                && search.bits() & RegexOptions::REGEX_OPTION_FIND_NOT_EMPTY.bits() != 0 =>
        {
            false
        }
        Some(_) => true,
        None => false,
    }
}

fn all_matches(regex: &Regex, text: &str, global: bool, search: SearchOptions) -> Vec<Value> {
    let names = group_names(regex);
    let mut region = Region::new();
    let mut start = 0usize;
    let len = text.len();
    let mut out = Vec::new();
    loop {
        if start > len {
            break;
        }
        let found = regex.search_with_param(
            text,
            start,
            len,
            search,
            Some(&mut region),
            MatchParam::default(),
        );
        let Ok(Some(_)) = found else {
            break;
        };
        let Some((from, to)) = region.pos(0) else {
            break;
        };
        // Longest-match remembers an empty hit before not-empty can reject
        // it. Drop that hit. With `g`, keep walking. Without `g`, it was the
        // only hit the search was going to return.
        if from == to && search.bits() & RegexOptions::REGEX_OPTION_FIND_NOT_EMPTY.bits() != 0 {
            if !global {
                break;
            }
            start = from.saturating_add(1);
            continue;
        }
        out.push(match_object(regex, text, &names, &region, from, to));
        if !global {
            break;
        }
        let next = if to == from {
            from.saturating_add(1)
        } else {
            to
        };
        if next <= start {
            start = start.saturating_add(1);
        } else {
            start = next;
        }
    }
    out
}

fn group_names(regex: &Regex) -> Vec<Option<String>> {
    let mut names = vec![None; regex.captures_len().saturating_add(1)];
    regex.foreach_name(|name, groups| {
        for group in groups {
            let index = *group as usize;
            if let Some(slot) = names.get_mut(index) {
                *slot = Some(name.to_string());
            }
        }
        true
    });
    names
}

fn match_object(
    regex: &Regex,
    text: &str,
    names: &[Option<String>],
    region: &Region,
    from: usize,
    to: usize,
) -> Value {
    // An unset group inside an empty match is an empty capture there.
    // Inside a match that took characters, it stays unset.
    let whole_empty = from == to;
    let mut captures = Vec::new();
    for index in 1..=regex.captures_len() {
        let name = names.get(index).and_then(|name| name.clone());
        let (offset, length, string) = match region.pos(index) {
            None if whole_empty => (
                codepoints_before(text, from) as i64,
                0,
                Value::String(crate::value::Text::from(String::new())),
            ),
            None => (-1, 0, Value::Null),
            Some((start, end)) => {
                let offset = codepoints_before(text, start) as i64;
                let length =
                    codepoints_before(text, end).saturating_sub(codepoints_before(text, start));
                (
                    offset,
                    length,
                    Value::String(crate::value::Text::from(slice_between(text, start, end))),
                )
            }
        };
        captures.push(capture_object(offset, length, string, name));
    }
    let offset = codepoints_before(text, from);
    let length = codepoints_before(text, to).saturating_sub(offset);
    Value::object(vec![
        ("offset".to_string(), number(offset)),
        ("length".to_string(), number(length)),
        (
            "string".to_string(),
            Value::String(crate::value::Text::from(slice_between(text, from, to))),
        ),
        ("captures".to_string(), Value::array(captures)),
    ])
}

/// A capture jq left unmatched uses `string` before `length`. A capture that
/// took no characters does too, with an empty string. A capture that took
/// characters puts `length` first.
fn capture_object(offset: i64, length: usize, string: Value, name: Option<String>) -> Value {
    let name = match name {
        Some(name) => Value::String(crate::value::Text::from(name)),
        None => Value::Null,
    };
    let offset_value = Value::Number(crate::value::Text::from(offset.to_string()));
    let length_value = number(length);
    if offset < 0 || length == 0 {
        Value::object(vec![
            ("offset".to_string(), offset_value),
            ("string".to_string(), string),
            ("length".to_string(), length_value),
            ("name".to_string(), name),
        ])
    } else {
        Value::object(vec![
            ("offset".to_string(), offset_value),
            ("length".to_string(), length_value),
            ("string".to_string(), string),
            ("name".to_string(), name),
        ])
    }
}

fn codepoints_before(text: &str, byte: usize) -> usize {
    text.char_indices()
        .take_while(|(index, _)| *index < byte)
        .count()
}

fn slice_between(text: &str, start: usize, end: usize) -> String {
    if start > end || end > text.len() {
        return String::new();
    }
    if text.is_char_boundary(start) && text.is_char_boundary(end) {
        return text[start..end].to_string();
    }
    String::new()
}

fn number(value: usize) -> Value {
    Value::Number(crate::value::Text::from(value.to_string()))
}
