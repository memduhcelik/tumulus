//! Reading a program into a filter.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::math::{self, Kind};
use crate::number;
use crate::value::Value;

use super::ast::*;
use super::eval::shown;
use super::value::{apply_op, put};

thread_local! {
    static REACHED: RefCell<Option<HashSet<usize>>> = RefCell::new(None);
    /// `$name` uses that are not in scope. Collected while checking variables.
    static UNBOUND: RefCell<Vec<(usize, String)>> = RefCell::new(Vec::new());
    /// File shown in a compile error. Empty means `<top-level>`.
    static PLACE: RefCell<String> = RefCell::new(String::new());
}

struct PlaceGuard(String);

impl Drop for PlaceGuard {
    fn drop(&mut self) {
        let previous = std::mem::take(&mut self.0);
        PLACE.with(|slot| *slot.borrow_mut() = previous);
    }
}

fn enter_place(label: &str) -> PlaceGuard {
    let previous = PLACE.with(|slot| slot.replace(label.to_string()));
    PlaceGuard(previous)
}

fn place_label() -> String {
    PLACE.with(|slot| {
        let current = slot.borrow();
        if current.is_empty() {
            "<top-level>".to_string()
        } else {
            current.clone()
        }
    })
}

pub fn compile(source: &str) -> Result<Filter, ProgramError> {
    compile_with(source, &[])
}

/// True when running `filter` can call `input`, `inputs`, or `input_filename`.
/// A function that is never called does not count.
pub fn reads_input(filter: &Filter) -> bool {
    fn walk(filter: &Filter, seen: &mut HashSet<usize>) -> bool {
        match filter {
            Filter::Input | Filter::Inputs | Filter::InputFilename => true,
            Filter::Identity
            | Filter::Field(_)
            | Filter::Index { .. }
            | Filter::Iterate
            | Filter::Recurse
            | Filter::Literal(_)
            | Filter::Not
            | Filter::Empty
            | Filter::Length
            | Filter::Keys
            | Filter::KeysUnsorted
            | Filter::FromJson
            | Filter::Type
            | Filter::ToNumber
            | Filter::ToString
            | Filter::ToJson
            | Filter::Format(_)
            | Filter::AsciiDown
            | Filter::AsciiUp
            | Filter::Explode
            | Filter::Implode
            | Filter::Utf8ByteLength
            | Filter::Add
            | Filter::ToEntries
            | Filter::FromEntries
            | Filter::ToDate
            | Filter::FromDate
            | Filter::GmTime
            | Filter::LocalTime
            | Filter::MkTime
            | Filter::Now
            | Filter::Math1(_)
            | Filter::Abs
            | Filter::Pow10
            | Filter::NumTest(_)
            | Filter::Halt
            | Filter::Debug
            | Filter::Stderr
            | Filter::InputLine
            | Filter::Builtins
            | Filter::SearchList
            | Filter::ModuleMeta
            | Filter::JqOrigin
            | Filter::ProgOrigin
            | Filter::Error
            | Filter::First(None)
            | Filter::Last(None)
            | Filter::Var { .. }
            | Filter::Here(_)
            | Filter::Environ
            | Filter::Param(_)
            | Filter::Paths(None)
            | Filter::Unresolved { .. } => false,
            Filter::Order { by, .. } => by.as_deref().is_some_and(|by| walk(by, seen)),
            Filter::Pipe(left, right)
            | Filter::Comma(left, right)
            | Filter::And(left, right)
            | Filter::Or(left, right)
            | Filter::Alt(left, right)
            | Filter::Bin { left, right, .. }
            | Filter::Math2(_, left, right)
            | Filter::Limit(left, right)
            | Filter::Until(left, right)
            | Filter::While(left, right)
            | Filter::SetPath(left, right) => walk(left, seen) || walk(right, seen),
            Filter::Slice {
                base, start, end, ..
            } => {
                base.as_deref().is_some_and(|base| walk(base, seen))
                    || start.as_deref().is_some_and(|start| walk(start, seen))
                    || end.as_deref().is_some_and(|end| walk(end, seen))
            }
            Filter::Optional(inner)
            | Filter::Collect(inner)
            | Filter::Neg(inner)
            | Filter::Has(inner)
            | Filter::Select(inner)
            | Filter::Map(inner)
            | Filter::MapValues(inner)
            | Filter::Walk(inner)
            | Filter::WithEntries(inner)
            | Filter::Join(inner)
            | Filter::LTrim(inner)
            | Filter::RTrim(inner)
            | Filter::StartsWith(inner)
            | Filter::EndsWith(inner)
            | Filter::IndexOf(inner)
            | Filter::RIndexOf(inner)
            | Filter::IndicesOf(inner)
            | Filter::Split(inner)
            | Filter::Contains(inner)
            | Filter::Inside(inner)
            | Filter::IsEmpty(inner)
            | Filter::StrfTime(inner)
            | Filter::StrfLocalTime(inner)
            | Filter::StrpTime(inner)
            | Filter::Raise(inner)
            | Filter::HaltError(inner)
            | Filter::FormatBy(inner)
            | Filter::PathOf(inner)
            | Filter::GetPath(inner)
            | Filter::DelPaths(inner)
            | Filter::Repeat(inner) => walk(inner, seen),
            Filter::First(inner)
            | Filter::Last(inner)
            | Filter::Paths(inner)
            | Filter::Flatten(inner) => inner.as_deref().is_some_and(|inner| walk(inner, seen)),
            Filter::Quant { stream, cond, .. } => walk(stream, seen) || walk(cond, seen),
            Filter::In { src, stream } => {
                src.as_deref().is_some_and(|src| walk(src, seen)) || walk(stream, seen)
            }
            Filter::IndexBy { stream, key } => {
                stream.as_deref().is_some_and(|stream| walk(stream, seen)) || walk(key, seen)
            }
            Filter::RecurseDown { step, cond } => {
                walk(step, seen) || cond.as_deref().is_some_and(|cond| walk(cond, seen))
            }
            Filter::Try { body, handler } => {
                walk(body, seen)
                    || handler
                        .as_deref()
                        .is_some_and(|handler| walk(handler, seen))
            }
            Filter::Update { path, expr, .. } => walk(path, seen) || walk(expr, seen),
            Filter::Interpolate(pieces) | Filter::FormatString { pieces, .. } => {
                pieces.iter().any(|piece| match piece {
                    Piece::Text(_) => false,
                    Piece::Interp(inner) => walk(inner, seen),
                })
            }
            Filter::Object(fields) => fields.iter().any(|field| match field {
                ObjField::Pair { key, value } => walk(key, seen) || walk(value, seen),
                ObjField::Lookup(key) => walk(key, seen),
            }),
            Filter::At { base, key, .. } => walk(base, seen) || walk(key, seen),
            Filter::Math3(x, y, z) => walk(x, seen) || walk(y, seen) || walk(z, seen),
            Filter::Nth { index, expr } => {
                walk(index, seen) || expr.as_deref().is_some_and(|expr| walk(expr, seen))
            }
            Filter::Range { from, to, step } => {
                from.as_deref().is_some_and(|from| walk(from, seen))
                    || walk(to, seen)
                    || step.as_deref().is_some_and(|step| walk(step, seen))
            }
            Filter::If { cond, yes, no } => walk(cond, seen) || walk(yes, seen) || walk(no, seen),
            Filter::Label { body, .. } => walk(body, seen),
            Filter::Break { .. } => false,
            Filter::MatchImpl { regex, flags, .. } => walk(regex, seen) || walk(flags, seen),
            Filter::StreamBind { expr, body, .. } => walk(expr, seen) || walk(body, seen),
            Filter::Bind { expr, body, .. } => walk(expr, seen) || walk(body, seen),
            Filter::Reduce {
                stream,
                init,
                update,
                ..
            }
            | Filter::Foreach {
                stream,
                init,
                update,
                ..
            } => {
                walk(stream, seen)
                    || walk(init, seen)
                    || walk(update, seen)
                    || match filter {
                        Filter::Foreach {
                            extract: Some(extract),
                            ..
                        } => walk(extract, seen),
                        _ => false,
                    }
            }
            Filter::Define { rest, .. } => walk(rest, seen),
            Filter::Call { func, args } => {
                if args.iter().any(|arg| walk(arg, seen)) {
                    return true;
                }
                let Some(cell) = func.upgrade() else {
                    return false;
                };
                let ptr = Rc::as_ptr(&cell) as usize;
                if !seen.insert(ptr) {
                    return false;
                }
                let body = Rc::clone(&cell.body.borrow());
                walk(&body, seen)
            }
        }
    }
    walk(filter, &mut HashSet::new())
}

/// `scope` names variables that are already bound, ahead of the program.
pub fn compile_with(source: &str, scope: &[String]) -> Result<Filter, ProgramError> {
    super::modules::clear_cache();
    let _place = enter_place("<top-level>");
    finish_program(source, scope, true)
}

/// Compile one library file. Definitions keep their data imports.
pub(super) fn compile_module_source(
    source: &str,
    place: &str,
) -> Result<Vec<super::modules::Export>, ProgramError> {
    let _place = enter_place(place);
    let mut parser = Parser::open(source, true);
    parser.library = true;
    let filter = parser.parse_program()?;
    parser.skip_ws();
    if !parser.eof() {
        return Err(ProgramError::Syntax);
    }
    check_finished(
        source,
        &["ARGS".to_string(), "ENV".to_string()],
        &parser.data_names,
        &filter,
    )?;
    let binds = parser.data.clone();
    for exp in &parser.exports {
        let body = (**exp.func.body.borrow()).clone();
        *exp.func.body.borrow_mut() = Rc::new(super::modules::wrap_data(body, &binds));
    }
    Ok(parser.exports)
}

/// `modulemeta`. Imports are recorded and not loaded. A syntax error is returned.
pub(super) fn describe_module(source: &str, place: &str) -> Result<Value, ProgramError> {
    let _place = enter_place(place);
    let mut parser = Parser::open(source, false);
    parser.library = true;
    let filter = parser.parse_program()?;
    parser.skip_ws();
    if !parser.eof() {
        return Err(ProgramError::Syntax);
    }
    // Undefined names are not an error here. The header, imports, and defs are.
    let _ = filter;
    Ok(parser.meta_value())
}

fn finish_program(source: &str, scope: &[String], link: bool) -> Result<Filter, ProgramError> {
    let mut parser = Parser::open(source, link);
    let filter = parser.parse_program()?;
    parser.skip_ws();
    if !parser.eof() {
        return Err(parser.trailing_token());
    }
    if parser.tail_missing {
        return Err(ProgramError::Report(
            "jq: error: Top-level program not given (try \".\")\njq: 1 compile error\n".to_string(),
        ));
    }
    check_finished(source, scope, &parser.data_names, &filter)?;
    Ok(filter)
}

fn check_finished(
    source: &str,
    scope: &[String],
    data_names: &[String],
    filter: &Filter,
) -> Result<(), ProgramError> {
    let (mut errors, reached) = undefined_calls(filter);
    let _reached = set_reached(reached);
    let saved = UNBOUND.with(|slot| std::mem::take(&mut *slot.borrow_mut()));
    let mut names = scope.to_vec();
    names.extend(data_names.iter().cloned());
    let checked = check_vars(filter, &names);
    let mut unbound = UNBOUND.with(|slot| std::mem::take(&mut *slot.borrow_mut()));
    UNBOUND.with(|slot| *slot.borrow_mut() = saved);
    checked?;
    errors.append(&mut unbound);
    if errors.is_empty() {
        return Ok(());
    }
    errors.sort_by_key(|(at, _)| *at);
    Err(report_calls(source, &errors))
}

pub(super) fn check_vars(filter: &Filter, scope: &[String]) -> Result<(), ProgramError> {
    match filter {
        Filter::Identity
        | Filter::Field(_)
        | Filter::Index { .. }
        | Filter::Paths(None)
        | Filter::Iterate
        | Filter::Recurse
        | Filter::Literal(_)
        | Filter::Not
        | Filter::Empty
        | Filter::Length
        | Filter::Keys
        | Filter::KeysUnsorted
        | Filter::FromJson
        | Filter::Type
        | Filter::ToNumber
        | Filter::ToString
        | Filter::ToJson
        | Filter::Format(_)
        | Filter::AsciiDown
        | Filter::AsciiUp
        | Filter::Explode
        | Filter::Implode
        | Filter::Utf8ByteLength
        | Filter::Add
        | Filter::ToEntries
        | Filter::FromEntries
        | Filter::ToDate
        | Filter::FromDate
        | Filter::GmTime
        | Filter::LocalTime
        | Filter::MkTime
        | Filter::Now
        | Filter::Math1(_)
        | Filter::Abs
        | Filter::Pow10
        | Filter::NumTest(_)
        | Filter::Halt
        | Filter::Debug
        | Filter::Stderr
        | Filter::InputLine
        | Filter::Builtins
        | Filter::SearchList
        | Filter::ModuleMeta
        | Filter::JqOrigin
        | Filter::ProgOrigin
        | Filter::Error
        | Filter::First(None)
        | Filter::Last(None)
        | Filter::Here(_)
        | Filter::Input
        | Filter::Inputs
        | Filter::InputFilename
        | Filter::Environ => Ok(()),
        Filter::Order { by, .. } => match by {
            Some(by) => check_vars(by, scope),
            None => Ok(()),
        },
        Filter::Slice {
            base, start, end, ..
        } => {
            if let Some(base) = base {
                check_vars(base, scope)?;
            }
            if let Some(start) = start {
                check_vars(start, scope)?;
            }
            if let Some(end) = end {
                check_vars(end, scope)?;
            }
            Ok(())
        }
        Filter::Var { name, at } => {
            if scope.iter().any(|bound| bound == name) {
                Ok(())
            } else {
                UNBOUND.with(|slot| {
                    slot.borrow_mut()
                        .push((*at, format!("${name} is not defined")));
                });
                Ok(())
            }
        }
        Filter::Pipe(left, right)
        | Filter::Comma(left, right)
        | Filter::And(left, right)
        | Filter::Or(left, right)
        | Filter::Alt(left, right) => {
            check_vars(left, scope)?;
            check_vars(right, scope)
        }
        Filter::Bin { left, right, .. } => {
            check_vars(left, scope)?;
            check_vars(right, scope)
        }
        Filter::Optional(inner)
        | Filter::Collect(inner)
        | Filter::Neg(inner)
        | Filter::Has(inner)
        | Filter::Select(inner)
        | Filter::Map(inner)
        | Filter::MapValues(inner)
        | Filter::Walk(inner)
        | Filter::WithEntries(inner)
        | Filter::Join(inner)
        | Filter::LTrim(inner)
        | Filter::RTrim(inner)
        | Filter::StartsWith(inner)
        | Filter::EndsWith(inner)
        | Filter::IndexOf(inner)
        | Filter::RIndexOf(inner)
        | Filter::IndicesOf(inner)
        | Filter::Split(inner)
        | Filter::Contains(inner)
        | Filter::Inside(inner)
        | Filter::IsEmpty(inner)
        | Filter::StrfTime(inner)
        | Filter::StrfLocalTime(inner)
        | Filter::StrpTime(inner)
        | Filter::Raise(inner)
        | Filter::HaltError(inner)
        | Filter::FormatBy(inner) => check_vars(inner, scope),
        Filter::Flatten(inner) => match inner {
            Some(inner) => check_vars(inner, scope),
            None => Ok(()),
        },
        Filter::Quant { stream, cond, .. } => {
            check_vars(stream, scope)?;
            check_vars(cond, scope)
        }
        Filter::In { src, stream } => {
            if let Some(src) = src {
                check_vars(src, scope)?;
            }
            check_vars(stream, scope)
        }
        Filter::IndexBy { stream, key } => {
            if let Some(stream) = stream {
                check_vars(stream, scope)?;
            }
            check_vars(key, scope)
        }
        Filter::Try { body, handler } => {
            check_vars(body, scope)?;
            if let Some(handler) = handler {
                check_vars(handler, scope)?;
            }
            Ok(())
        }
        Filter::Update { path, expr, .. } => {
            check_vars(path, scope)?;
            check_vars(expr, scope)
        }
        Filter::PathOf(inner) | Filter::GetPath(inner) | Filter::DelPaths(inner) => {
            check_vars(inner, scope)
        }
        Filter::Paths(Some(inner)) => check_vars(inner, scope),
        Filter::SetPath(path, value) => {
            check_vars(path, scope)?;
            check_vars(value, scope)
        }
        Filter::Interpolate(pieces) | Filter::FormatString { pieces, .. } => {
            for piece in pieces {
                if let Piece::Interp(inner) = piece {
                    check_vars(inner, scope)?;
                }
            }
            Ok(())
        }
        Filter::Object(fields) => {
            for field in fields {
                match field {
                    ObjField::Pair { key, value } => {
                        check_vars(key, scope)?;
                        check_vars(value, scope)?;
                    }
                    ObjField::Lookup(key) => check_vars(key, scope)?,
                }
            }
            Ok(())
        }
        Filter::At { base, key, .. } => {
            check_vars(base, scope)?;
            check_vars(key, scope)
        }
        Filter::RecurseDown { step, cond } => {
            check_vars(step, scope)?;
            if let Some(cond) = cond {
                check_vars(cond, scope)?;
            }
            Ok(())
        }
        Filter::Math2(_, left, right) => {
            check_vars(left, scope)?;
            check_vars(right, scope)
        }
        Filter::Math3(x, y, z) => {
            check_vars(x, scope)?;
            check_vars(y, scope)?;
            check_vars(z, scope)
        }
        Filter::Limit(count, expr) | Filter::Until(count, expr) | Filter::While(count, expr) => {
            check_vars(count, scope)?;
            check_vars(expr, scope)
        }
        Filter::First(Some(inner)) | Filter::Last(Some(inner)) | Filter::Repeat(inner) => {
            check_vars(inner, scope)
        }
        Filter::Param(_) => Ok(()),
        Filter::Call { args, .. } | Filter::Unresolved { args, .. } => {
            for arg in args {
                check_vars(arg, scope)?;
            }
            Ok(())
        }
        Filter::Define { func, rest } => {
            let ptr = Rc::as_ptr(func) as usize;
            if body_reached(ptr) {
                let mut inner = scope.to_vec();
                for param in &func.params {
                    if let FnParam::Value(name) = param {
                        inner.push(name.clone());
                    }
                }
                let body = Rc::clone(&func.body.borrow());
                check_vars(&body, &inner)?;
            }
            check_vars(rest, scope)
        }
        Filter::Nth { index, expr } => {
            check_vars(index, scope)?;
            if let Some(expr) = expr {
                check_vars(expr, scope)?;
            }
            Ok(())
        }
        Filter::Range { from, to, step } => {
            if let Some(from) = from {
                check_vars(from, scope)?;
            }
            check_vars(to, scope)?;
            if let Some(step) = step {
                check_vars(step, scope)?;
            }
            Ok(())
        }
        Filter::If { cond, yes, no } => {
            check_vars(cond, scope)?;
            check_vars(yes, scope)?;
            check_vars(no, scope)
        }
        Filter::Bind {
            expr,
            pattern,
            alts,
            body,
        } => {
            check_vars(expr, scope)?;
            check_pattern(pattern, scope)?;
            for alt in alts {
                check_pattern(alt, scope)?;
            }
            let mut inner = scope.to_vec();
            bind_names(pattern, alts, &mut inner);
            check_vars(body, &inner)
        }
        Filter::Reduce {
            stream,
            pattern,
            alts,
            init,
            update,
        } => {
            check_vars(stream, scope)?;
            check_vars(init, scope)?;
            check_pattern(pattern, scope)?;
            for alt in alts {
                check_pattern(alt, scope)?;
            }
            let mut inner = scope.to_vec();
            bind_names(pattern, alts, &mut inner);
            check_vars(update, &inner)
        }
        Filter::Foreach {
            stream,
            pattern,
            alts,
            init,
            update,
            extract,
        } => {
            check_vars(stream, scope)?;
            check_vars(init, scope)?;
            check_pattern(pattern, scope)?;
            for alt in alts {
                check_pattern(alt, scope)?;
            }
            let mut inner = scope.to_vec();
            bind_names(pattern, alts, &mut inner);
            check_vars(update, &inner)?;
            if let Some(extract) = extract {
                check_vars(extract, &inner)?;
            }
            Ok(())
        }
        Filter::Label { body, .. } => check_vars(body, scope),
        Filter::Break { .. } => Ok(()),
        Filter::MatchImpl { regex, flags, .. } => {
            check_vars(regex, scope)?;
            check_vars(flags, scope)
        }
        Filter::StreamBind { expr, name, body } => {
            check_vars(expr, scope)?;
            let mut inner = scope.to_vec();
            inner.push(name.clone());
            check_vars(body, &inner)
        }
    }
}

/// Keys see the surrounding scope only, not names this pattern binds.
pub(super) fn check_pattern(pattern: &Pattern, scope: &[String]) -> Result<(), ProgramError> {
    match pattern {
        Pattern::Var(_) => Ok(()),
        Pattern::Array(items) => {
            for item in items {
                check_pattern(item, scope)?;
            }
            Ok(())
        }
        Pattern::Object(fields) => {
            for field in fields {
                check_vars(&field.key, scope)?;
                check_pattern(&field.pattern, scope)?;
            }
            Ok(())
        }
    }
}

fn body_reached(ptr: usize) -> bool {
    REACHED.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|reached| reached.contains(&ptr))
    })
}

struct Reached;

impl Drop for Reached {
    fn drop(&mut self) {
        REACHED.with(|slot| *slot.borrow_mut() = None);
    }
}

fn set_reached(reached: HashSet<usize>) -> Reached {
    REACHED.with(|slot| *slot.borrow_mut() = Some(reached));
    Reached
}

pub(super) fn reset_compile_state() {
    REACHED.with(|slot| *slot.borrow_mut() = None);
    PLACE.with(|slot| slot.borrow_mut().clear());
}

/// Calls that nobody defined, and the functions a call actually reaches.
/// An argument of an undefined call is not itself reported: jq names the call.
fn undefined_calls(filter: &Filter) -> (Vec<(usize, String)>, HashSet<usize>) {
    let mut errors = Vec::new();
    let mut seen = HashSet::new();
    walk_defs(filter, &mut seen, &mut errors);
    errors.sort_by_key(|(at, _)| *at);
    (errors, seen)
}

fn walk_defs(filter: &Filter, seen: &mut HashSet<usize>, errors: &mut Vec<(usize, String)>) {
    match filter {
        Filter::Identity
        | Filter::Field(_)
        | Filter::Index { .. }
        | Filter::Iterate
        | Filter::Recurse
        | Filter::Literal(_)
        | Filter::Not
        | Filter::Empty
        | Filter::Length
        | Filter::Keys
        | Filter::KeysUnsorted
        | Filter::FromJson
        | Filter::Type
        | Filter::ToNumber
        | Filter::ToString
        | Filter::ToJson
        | Filter::Format(_)
        | Filter::AsciiDown
        | Filter::AsciiUp
        | Filter::Explode
        | Filter::Implode
        | Filter::Utf8ByteLength
        | Filter::Add
        | Filter::ToEntries
        | Filter::FromEntries
        | Filter::ToDate
        | Filter::FromDate
        | Filter::GmTime
        | Filter::LocalTime
        | Filter::MkTime
        | Filter::Now
        | Filter::Math1(_)
        | Filter::Abs
        | Filter::Pow10
        | Filter::NumTest(_)
        | Filter::Halt
        | Filter::Debug
        | Filter::Stderr
        | Filter::InputLine
        | Filter::Builtins
        | Filter::SearchList
        | Filter::ModuleMeta
        | Filter::JqOrigin
        | Filter::ProgOrigin
        | Filter::Error
        | Filter::First(None)
        | Filter::Last(None)
        | Filter::Var { .. }
        | Filter::Here(_)
        | Filter::Input
        | Filter::Inputs
        | Filter::InputFilename
        | Filter::Environ
        | Filter::Param(_)
        | Filter::Paths(None)
        | Filter::Flatten(None) => {}
        Filter::Slice {
            base, start, end, ..
        } => {
            if let Some(base) = base {
                walk_defs(base, seen, errors);
            }
            if let Some(start) = start {
                walk_defs(start, seen, errors);
            }
            if let Some(end) = end {
                walk_defs(end, seen, errors);
            }
        }
        Filter::Order { by, .. } => {
            if let Some(by) = by {
                walk_defs(by, seen, errors);
            }
        }
        Filter::Pipe(left, right)
        | Filter::Comma(left, right)
        | Filter::And(left, right)
        | Filter::Or(left, right)
        | Filter::Alt(left, right)
        | Filter::Bin { left, right, .. }
        | Filter::Math2(_, left, right)
        | Filter::Limit(left, right)
        | Filter::Until(left, right)
        | Filter::While(left, right) => {
            walk_defs(left, seen, errors);
            walk_defs(right, seen, errors);
        }
        Filter::Optional(inner)
        | Filter::Collect(inner)
        | Filter::Neg(inner)
        | Filter::Has(inner)
        | Filter::Select(inner)
        | Filter::Map(inner)
        | Filter::MapValues(inner)
        | Filter::Walk(inner)
        | Filter::WithEntries(inner)
        | Filter::Join(inner)
        | Filter::LTrim(inner)
        | Filter::RTrim(inner)
        | Filter::StartsWith(inner)
        | Filter::EndsWith(inner)
        | Filter::IndexOf(inner)
        | Filter::RIndexOf(inner)
        | Filter::IndicesOf(inner)
        | Filter::Split(inner)
        | Filter::Contains(inner)
        | Filter::Inside(inner)
        | Filter::IsEmpty(inner)
        | Filter::StrfTime(inner)
        | Filter::StrfLocalTime(inner)
        | Filter::StrpTime(inner)
        | Filter::Raise(inner)
        | Filter::HaltError(inner)
        | Filter::FormatBy(inner)
        | Filter::PathOf(inner)
        | Filter::GetPath(inner)
        | Filter::DelPaths(inner)
        | Filter::First(Some(inner))
        | Filter::Last(Some(inner))
        | Filter::Repeat(inner)
        | Filter::Flatten(Some(inner)) => walk_defs(inner, seen, errors),
        Filter::Quant { stream, cond, .. } => {
            walk_defs(stream, seen, errors);
            walk_defs(cond, seen, errors);
        }
        Filter::In { src, stream } => {
            if let Some(src) = src {
                walk_defs(src, seen, errors);
            }
            walk_defs(stream, seen, errors);
        }
        Filter::IndexBy { stream, key } => {
            if let Some(stream) = stream {
                walk_defs(stream, seen, errors);
            }
            walk_defs(key, seen, errors);
        }
        Filter::RecurseDown { step, cond } => {
            walk_defs(step, seen, errors);
            if let Some(cond) = cond {
                walk_defs(cond, seen, errors);
            }
        }
        Filter::Paths(Some(inner)) => walk_defs(inner, seen, errors),
        Filter::Try { body, handler } => {
            walk_defs(body, seen, errors);
            if let Some(handler) = handler {
                walk_defs(handler, seen, errors);
            }
        }
        Filter::Update { path, expr, .. } => {
            walk_defs(path, seen, errors);
            walk_defs(expr, seen, errors);
        }
        Filter::SetPath(path, value) => {
            walk_defs(path, seen, errors);
            walk_defs(value, seen, errors);
        }
        Filter::Interpolate(pieces) | Filter::FormatString { pieces, .. } => {
            for piece in pieces {
                if let Piece::Interp(inner) = piece {
                    walk_defs(inner, seen, errors);
                }
            }
        }
        Filter::Object(fields) => {
            for field in fields {
                match field {
                    ObjField::Pair { key, value } => {
                        walk_defs(key, seen, errors);
                        walk_defs(value, seen, errors);
                    }
                    ObjField::Lookup(key) => walk_defs(key, seen, errors),
                }
            }
        }
        Filter::At { base, key, .. } => {
            walk_defs(base, seen, errors);
            walk_defs(key, seen, errors);
        }
        Filter::Math3(x, y, z) => {
            walk_defs(x, seen, errors);
            walk_defs(y, seen, errors);
            walk_defs(z, seen, errors);
        }
        Filter::Nth { index, expr } => {
            walk_defs(index, seen, errors);
            if let Some(expr) = expr {
                walk_defs(expr, seen, errors);
            }
        }
        Filter::Range { from, to, step } => {
            if let Some(from) = from {
                walk_defs(from, seen, errors);
            }
            walk_defs(to, seen, errors);
            if let Some(step) = step {
                walk_defs(step, seen, errors);
            }
        }
        Filter::If { cond, yes, no } => {
            walk_defs(cond, seen, errors);
            walk_defs(yes, seen, errors);
            walk_defs(no, seen, errors);
        }
        Filter::Bind { expr, body, .. } => {
            walk_defs(expr, seen, errors);
            walk_defs(body, seen, errors);
        }
        Filter::Reduce {
            stream,
            init,
            update,
            ..
        }
        | Filter::Foreach {
            stream,
            init,
            update,
            ..
        } => {
            walk_defs(stream, seen, errors);
            walk_defs(init, seen, errors);
            walk_defs(update, seen, errors);
            if let Filter::Foreach {
                extract: Some(extract),
                ..
            } = filter
            {
                walk_defs(extract, seen, errors);
            }
        }
        Filter::Define { rest, .. } => walk_defs(rest, seen, errors),
        Filter::Call { func, args } => {
            for arg in args {
                walk_defs(arg, seen, errors);
            }
            let Some(cell) = func.upgrade() else {
                return;
            };
            let ptr = Rc::as_ptr(&cell) as usize;
            if seen.insert(ptr) {
                let body = Rc::clone(&cell.body.borrow());
                walk_defs(&body, seen, errors);
            }
        }
        Filter::Unresolved {
            at, name, arity, ..
        } => errors.push((*at, format!("{name}/{arity} is not defined"))),
        Filter::Label { body, .. } => walk_defs(body, seen, errors),
        Filter::MatchImpl { regex, flags, .. } => {
            walk_defs(regex, seen, errors);
            walk_defs(flags, seen, errors);
        }
        Filter::StreamBind { expr, body, .. } => {
            walk_defs(expr, seen, errors);
            walk_defs(body, seen, errors);
        }
        Filter::Break {
            slot: None,
            at,
            name,
        } => errors.push((*at, format!("$*label-{name} is not defined"))),
        Filter::Break { slot: Some(_), .. } => {}
    }
}

/// `None` when the metadata has no `search` key. `Some` replaces the path,
/// including a null or a number, which then finds nothing.
fn search_override(meta: Option<&Value>) -> Option<Value> {
    let Some(Value::Object(items)) = meta else {
        return None;
    };
    items
        .iter()
        .find(|(key, _)| key == "search")
        .map(|(_, value)| value.clone())
}

fn dep_record(meta: Option<Value>, alias: Option<&str>, data: bool, rel: &str) -> Value {
    let mut pairs = match &meta {
        Some(Value::Object(items)) => items.iter().cloned().collect(),
        _ => Vec::new(),
    };
    if let Some(alias) = alias {
        super::value::put(
            &mut pairs,
            "as".to_string(),
            Value::String(crate::value::Text::from(alias.to_string())),
        );
    }
    super::value::put(&mut pairs, "is_data".to_string(), Value::Bool(data));
    super::value::put(
        &mut pairs,
        "relpath".to_string(),
        Value::String(crate::value::Text::from(rel.to_string())),
    );
    Value::object(pairs)
}

fn report_calls(source: &str, errors: &[(usize, String)]) -> ProgramError {
    if errors.len() == 1 {
        return report_at(source, errors[0].0, &errors[0].1);
    }
    let mut text = String::new();
    for (at, message) in errors {
        let (line_no, line, column) = line_at(source, *at);
        text.push_str(&format!(
            "jq: error: {message} at {}, line {line_no}:\n{line}",
            place_label()
        ));
        text.push_str(&" ".repeat(column));
        text.push('\n');
    }
    text.push_str(&format!("jq: {} compile errors\n", errors.len()));
    ProgramError::Report(text)
}

fn reserved(name: &str) -> bool {
    matches!(
        name,
        "if" | "then"
            | "else"
            | "elif"
            | "end"
            | "and"
            | "or"
            | "def"
            | "as"
            | "reduce"
            | "foreach"
            | "try"
            | "catch"
            | "import"
            | "include"
            | "module"
            | "break"
            | "label"
    )
}

fn unbox(args: Vec<Box<Filter>>) -> Vec<Filter> {
    args.into_iter().map(|arg| *arg).collect()
}

fn is_builtin(name: &str, arity: usize) -> bool {
    match name {
        "not" | "empty" | "length" | "keys" | "keys_unsorted" | "type" | "tonumber"
        | "tostring" | "tojson" | "fromjson" | "ascii_downcase" | "ascii_upcase" | "explode"
        | "implode" | "utf8bytelength" | "todate" | "fromdate" | "gmtime" | "localtime"
        | "mktime" | "now" | "add" | "to_entries" | "from_entries" | "input" | "inputs"
        | "input_filename" | "env" | "sort" | "unique" | "min" | "max" | "values" | "nulls"
        | "booleans" | "numbers" | "strings" | "arrays" | "objects" | "iterables" | "scalars"
        | "tostream" => arity == 0,
        "sort_by" | "group_by" | "unique_by" | "min_by" | "max_by" => arity == 1,
        "has" | "select" | "map" | "map_values" | "join" | "ltrimstr" | "rtrimstr"
        | "startswith" | "endswith" | "index" | "rindex" | "indices" | "contains" | "inside"
        | "isempty" | "strftime" | "strflocaltime" | "strptime" | "path" | "getpath"
        | "delpaths" | "del" | "repeat" | "walk" | "with_entries" | "fromstream"
        | "truncate_stream" => arity == 1,
        "split" => arity == 1 || arity == 2,
        "scan" | "splits" => arity == 1 || arity == 2,
        "sub" | "gsub" => arity == 2 || arity == 3,
        "paths" | "first" | "last" | "error" | "flatten" | "halt_error" | "combinations" => {
            arity <= 1
        }
        "pick" => arity == 1,
        "isnan" | "isinfinite" | "isnormal" | "isfinite" | "halt" | "transpose" | "stderr"
        | "input_line_number" | "builtins" | "finites" | "normals" | "get_jq_origin"
        | "get_prog_origin" | "reverse" | "fromdateiso8601" | "todateiso8601" | "modulemeta"
        | "get_search_list" => arity == 0,
        "debug" => arity <= 1,
        "in" | "bsearch" | "format" => arity == 1,
        "match" | "test" | "capture" => arity == 1 || arity == 2,
        "JOIN" => (2..=4).contains(&arity),
        "any" | "all" => arity <= 2,
        "IN" | "INDEX" => arity == 1 || arity == 2,
        "recurse" => arity <= 2,
        "nth" => arity == 1 || arity == 2,
        "range" => (1..=3).contains(&arity),
        "limit" | "until" | "while" | "setpath" => arity == 2,
        _ => math_arity(name, arity),
    }
}

fn order_kind(name: &str) -> SortKind {
    match name {
        "sort" => SortKind::Sort,
        "sort_by" => SortKind::SortBy,
        "group_by" => SortKind::Group,
        "unique" => SortKind::Unique,
        "unique_by" => SortKind::UniqueBy,
        "min" => SortKind::Min,
        "max" => SortKind::Max,
        "min_by" => SortKind::MinBy,
        "max_by" => SortKind::MaxBy,
        _ => unreachable!("order builtin"),
    }
}

fn math_arity(name: &str, arity: usize) -> bool {
    match math::kind(name) {
        Some(Kind::Unary(_) | Kind::Abs | Kind::Pow10) => arity == 0,
        Some(Kind::Binary(_)) => arity == 2,
        Some(Kind::Fma) => arity == 3,
        None => false,
    }
}

fn keep_null(equal: bool) -> Filter {
    Filter::Select(Box::new(Filter::Bin {
        op: if equal { Op::Eq } else { Op::Ne },
        left: Box::new(Filter::Identity),
        right: Box::new(Filter::Literal(Value::Null)),
    }))
}

fn keep_type(kind: &str) -> Filter {
    Filter::Select(Box::new(type_is(kind)))
}

fn type_is(kind: &str) -> Filter {
    Filter::Bin {
        op: Op::Eq,
        left: Box::new(Filter::Type),
        right: Box::new(Filter::Literal(Value::String(crate::value::Text::from(
            kind.to_string(),
        )))),
    }
}

fn type_is_not(kind: &str) -> Filter {
    Filter::Bin {
        op: Op::Ne,
        left: Box::new(Filter::Type),
        right: Box::new(Filter::Literal(Value::String(crate::value::Text::from(
            kind.to_string(),
        )))),
    }
}

/// `match`, `test`, and `capture` as jq defines them. One argument binds
/// `$val` and may pass an array through. Two arguments call the search.
/// `match` then iterates the array of match objects.
fn regex_builtin(name: &str, args: Vec<Box<Filter>>) -> Filter {
    let mut args = args.into_iter();
    let regex = args.next().expect("regex");
    match args.next() {
        Some(flags) => regex_two(name, regex, flags),
        None => regex_one(name, regex),
    }
}

fn regex_two(name: &str, regex: Box<Filter>, flags: Box<Filter>) -> Filter {
    let search = Filter::MatchImpl {
        regex,
        flags,
        testing: name == "test",
    };
    if name == "test" {
        return search;
    }
    let matched = Filter::Pipe(Box::new(search), Box::new(Filter::Iterate));
    if name == "match" {
        return matched;
    }
    Filter::Pipe(Box::new(matched), Box::new(capture_reduce()))
}

fn regex_one(name: &str, arg: Box<Filter>) -> Filter {
    let val = || {
        Box::new(Filter::Var {
            name: "val".to_string(),
            at: 0,
        })
    };
    let vt = || {
        Box::new(Filter::Var {
            name: "vt".to_string(),
            at: 0,
        })
    };
    let string_type = Filter::Literal(Value::String(crate::value::Text::from(
        "string".to_string(),
    )));
    let array_type = Filter::Literal(Value::String(crate::value::Text::from("array".to_string())));
    let is_string = Filter::Bin {
        op: Op::Eq,
        left: vt(),
        right: Box::new(string_type),
    };
    let is_array = Filter::Bin {
        op: Op::Eq,
        left: vt(),
        right: Box::new(array_type),
    };
    let length = Filter::Pipe(val(), Box::new(Filter::Length));
    let longer = Filter::Bin {
        op: Op::Gt,
        left: Box::new(length.clone()),
        right: Box::new(Filter::Literal(Value::Number(crate::value::Text::from(
            "1".to_string(),
        )))),
    };
    let nonempty = Filter::Bin {
        op: Op::Gt,
        left: Box::new(length),
        right: Box::new(Filter::Literal(Value::Number(crate::value::Text::from(
            "0".to_string(),
        )))),
    };
    let none = || Box::new(Filter::Literal(Value::Null));
    let at = |index: &str| {
        Box::new(Filter::At {
            base: val(),
            key: Box::new(Filter::Literal(Value::Number(crate::value::Text::from(
                index.to_string(),
            )))),
            optional: false,
        })
    };
    let call = |regex, flags| regex_two(name, regex, flags);
    let body = Filter::If {
        cond: Box::new(is_string),
        yes: Box::new(call(val(), none())),
        no: Box::new(Filter::If {
            cond: Box::new(Filter::And(Box::new(is_array.clone()), Box::new(longer))),
            yes: Box::new(call(at("0"), at("1"))),
            no: Box::new(Filter::If {
                cond: Box::new(Filter::And(Box::new(is_array), Box::new(nonempty))),
                yes: Box::new(call(at("0"), none())),
                no: Box::new(Filter::Raise(Box::new(Filter::Bin {
                    op: Op::Add,
                    left: vt(),
                    right: Box::new(Filter::Literal(Value::String(crate::value::Text::from(
                        " not a string or array".to_string(),
                    )))),
                }))),
            }),
        }),
    };
    let typed = Filter::Bind {
        expr: Box::new(Filter::Pipe(val(), Box::new(Filter::Type))),
        pattern: Pattern::Var("vt".to_string()),
        alts: Vec::new(),
        body: Box::new(body),
    };
    Filter::StreamBind {
        expr: arg,
        name: "val".to_string(),
        body: Box::new(typed),
    }
}

/// Named captures, in the order jq's `capture` reduce walks them.
fn capture_reduce() -> Filter {
    let named = Filter::Select(Box::new(Filter::Bin {
        op: Op::Ne,
        left: Box::new(Filter::Field("name".to_string())),
        right: Box::new(Filter::Literal(Value::Null)),
    }));
    let pair = Filter::Object(vec![ObjField::Pair {
        key: Box::new(Filter::Field("name".to_string())),
        value: Box::new(Filter::Field("string".to_string())),
    }]);
    let stream = Filter::Pipe(
        Box::new(Filter::Pipe(
            Box::new(Filter::Pipe(
                Box::new(Filter::Field("captures".to_string())),
                Box::new(Filter::Iterate),
            )),
            Box::new(named),
        )),
        Box::new(pair),
    );
    Filter::Reduce {
        stream: Box::new(stream),
        pattern: Pattern::Var("pair".to_string()),
        alts: Vec::new(),
        init: Box::new(Filter::Literal(Value::object(Vec::new()))),
        update: Box::new(Filter::Bin {
            op: Op::Add,
            left: Box::new(Filter::Identity),
            right: Box::new(Filter::Var {
                name: "pair".to_string(),
                at: 0,
            }),
        }),
    }
}

/// `scan` prepends `g`. Null flags add nothing, because `"g" + null` is `"g"`.
const SCAN: &str = r#"match($re; "g" + $flags) | if (.captures|length > 0) then [ .captures | .[] | .string ] else .string end"#;

/// `splits` prepends `g` and cuts the input at each match. `_nwise` is jq's.
const SPLITS: &str = r#"def splits(flags): def _nwise($n): def n: if length <= $n then . else .[0:$n], (.[$n:] | n) end; n; . as $s | [ match($re; "g" + flags) | (.offset, .offset + .length) ] | [0] + . + [$s|length] | _nwise(2) | $s[.[0]:.[1]]; splits(null)"#;

/// One argument of `split` stays a string split. Two arguments collect `splits`.
const SPLIT_RE: &str = r#"def split(flags): [ splits($re; flags) ]; split(null)"#;

/// `sub` replaces the first match unless the flags already ask for more.
const SUB: &str = r#"def sub(s): . as $in | (reduce match($re; $flags) as $edit ({result: [], previous: 0}; $in[.previous:($edit | .offset)] as $gap | [reduce ($edit | .captures | .[] | select(.name != null) | {(.name): .string}) as $pair ({}; . + $pair) | s] as $inserts | reduce range(0; $inserts|length) as $ix (.; .result[$ix] += $gap + $inserts[$ix]) | .previous = ($edit | .offset + .length)) | .result[] + $in[.previous:]) // $in; sub(null)"#;

/// `gsub` is `sub` with `g` added at the end of the flags.
const GSUB: &str = r#"def gsub(s; flags): sub($re; s; flags + "g"); gsub(null; null)"#;

fn scan_call(re: Box<Filter>, flags: Option<Box<Filter>>) -> Filter {
    let flags = match flags {
        Some(flags) => *flags,
        None => Filter::Literal(Value::Null),
    };
    bind_lazy(
        vec![("re", *re), ("flags", flags)],
        compile_scoped(SCAN, &["re", "flags"]),
    )
}

fn splits_call(re: Box<Filter>, flags: Option<Box<Filter>>) -> Filter {
    let flags = match flags {
        Some(flags) => *flags,
        None => Filter::Literal(Value::Null),
    };
    bind_lazy(
        vec![("re", *re)],
        call_with(compile_scoped(SPLITS, &["re"]), vec![flags]),
    )
}

fn split_regex(re: Filter, flags: Filter) -> Filter {
    bind_lazy(
        vec![("re", re)],
        call_with(compile_scoped(SPLIT_RE, &["re"]), vec![flags]),
    )
}

fn sub_call(re: Box<Filter>, replacement: Box<Filter>, flags: Option<Box<Filter>>) -> Filter {
    // Two arguments use an empty flags string. Null is a real flags value.
    let flags = match flags {
        Some(flags) => *flags,
        None => Filter::Literal(Value::String(crate::value::Text::from(String::new()))),
    };
    bind_lazy(
        vec![("re", *re), ("flags", flags)],
        call_with(compile_scoped(SUB, &["re", "flags"]), vec![*replacement]),
    )
}

fn gsub_call(re: Box<Filter>, replacement: Box<Filter>, flags: Option<Box<Filter>>) -> Filter {
    match flags {
        Some(flags) => bind_lazy(
            vec![("re", *re)],
            call_with(compile_scoped(GSUB, &["re"]), vec![*replacement, *flags]),
        ),
        None => sub_call(
            re,
            replacement,
            Some(Box::new(Filter::Literal(Value::String(
                crate::value::Text::from("g".to_string()),
            )))),
        ),
    }
}

fn bind_lazy(pairs: Vec<(&str, Filter)>, body: Filter) -> Filter {
    let mut body = body;
    for (name, expr) in pairs.into_iter().rev() {
        body = Filter::StreamBind {
            expr: Box::new(expr),
            name: name.to_string(),
            body: Box::new(body),
        };
    }
    body
}

fn compile_scoped(source: &str, scope: &[&str]) -> Filter {
    let names: Vec<String> = scope.iter().copied().map(str::to_string).collect();
    match compile_with(source, &names) {
        Ok(filter) => filter,
        Err(ProgramError::Report(text)) => panic!("regex builtin failed to compile: {text}"),
        Err(ProgramError::Syntax) => panic!("regex builtin failed to compile"),
    }
}

fn call_with(mut filter: Filter, call_args: Vec<Filter>) -> Filter {
    let Filter::Define { rest, .. } = &mut filter else {
        panic!("regex builtin is not a definition");
    };
    let Filter::Call { args, .. } = rest.as_mut() else {
        panic!("regex builtin is not a call");
    };
    args.clear();
    args.extend(call_args);
    filter
}

fn build_builtin(name: &str, args: Vec<Box<Filter>>) -> Filter {
    let mut args = args.into_iter();
    match name {
        "not" => Filter::Not,
        "input" => Filter::Input,
        "inputs" => Filter::Inputs,
        "input_filename" => Filter::InputFilename,
        "env" => Filter::Environ,
        "sort" | "unique" | "min" | "max" | "sort_by" | "group_by" | "unique_by" | "min_by"
        | "max_by" => Filter::Order {
            kind: order_kind(name),
            by: args.next(),
        },
        "empty" => Filter::Empty,
        "length" => Filter::Length,
        "keys" => Filter::Keys,
        "keys_unsorted" => Filter::KeysUnsorted,
        "fromjson" => Filter::FromJson,
        "type" => Filter::Type,
        "tonumber" => Filter::ToNumber,
        "tostring" => Filter::ToString,
        "tojson" => Filter::ToJson,
        "add" => Filter::Add,
        "to_entries" => Filter::ToEntries,
        "from_entries" => Filter::FromEntries,
        "walk" => Filter::Walk(args.next().expect("walk")),
        "with_entries" => Filter::WithEntries(args.next().expect("with_entries")),
        "recurse" => match args.len() {
            0 => Filter::Recurse,
            1 => Filter::RecurseDown {
                step: args.next().expect("step"),
                cond: None,
            },
            _ => Filter::RecurseDown {
                step: args.next().expect("step"),
                cond: Some(args.next().expect("cond")),
            },
        },
        "todate" => Filter::ToDate,
        "fromdate" => Filter::FromDate,
        "gmtime" => Filter::GmTime,
        "localtime" => Filter::LocalTime,
        "mktime" => Filter::MkTime,
        "now" => Filter::Now,
        "has" => Filter::Has(args.next().expect("arg")),
        "select" => Filter::Select(args.next().expect("arg")),
        "map" => Filter::Map(args.next().expect("arg")),
        "map_values" => Filter::MapValues(args.next().expect("arg")),
        "split" => match args.next() {
            Some(sep) => match args.next() {
                Some(flags) => split_regex(*sep, *flags),
                None => Filter::Split(sep),
            },
            None => unreachable!("split arity"),
        },
        "contains" => Filter::Contains(args.next().expect("arg")),
        "inside" => Filter::Inside(args.next().expect("arg")),
        "isempty" => Filter::IsEmpty(args.next().expect("arg")),
        "values" => keep_null(false),
        "nulls" => keep_null(true),
        "booleans" => keep_type("boolean"),
        "numbers" => keep_type("number"),
        "strings" => keep_type("string"),
        "arrays" => keep_type("array"),
        "objects" => keep_type("object"),
        "iterables" => Filter::Select(Box::new(Filter::Or(
            Box::new(type_is("array")),
            Box::new(type_is("object")),
        ))),
        "scalars" => Filter::Select(Box::new(Filter::And(
            Box::new(type_is_not("array")),
            Box::new(type_is_not("object")),
        ))),
        "IN" => {
            let first = args.next().expect("in");
            match args.next() {
                None => Filter::In {
                    src: None,
                    stream: first,
                },
                Some(stream) => Filter::In {
                    src: Some(first),
                    stream,
                },
            }
        }
        "INDEX" => {
            let first = args.next().expect("index");
            match args.next() {
                None => Filter::IndexBy {
                    stream: None,
                    key: first,
                },
                Some(key) => Filter::IndexBy {
                    stream: Some(first),
                    key,
                },
            }
        }
        "flatten" => Filter::Flatten(args.next()),
        "any" | "all" => {
            let all_form = name == "all";
            let (stream, cond) = match (args.next(), args.next()) {
                (None, None) => (Box::new(Filter::Iterate), Box::new(Filter::Identity)),
                (Some(cond), None) => (Box::new(Filter::Iterate), cond),
                (Some(stream), Some(cond)) => (stream, cond),
                (None, Some(_)) => unreachable!("quantifier arity"),
            };
            Filter::Quant {
                all: all_form,
                stream,
                cond,
            }
        }
        "join" => Filter::Join(args.next().expect("arg")),
        "ltrimstr" => Filter::LTrim(args.next().expect("arg")),
        "rtrimstr" => Filter::RTrim(args.next().expect("arg")),
        "ascii_downcase" => Filter::AsciiDown,
        "ascii_upcase" => Filter::AsciiUp,
        "explode" => Filter::Explode,
        "implode" => Filter::Implode,
        "utf8bytelength" => Filter::Utf8ByteLength,
        "startswith" => Filter::StartsWith(args.next().expect("arg")),
        "endswith" => Filter::EndsWith(args.next().expect("arg")),
        "index" => Filter::IndexOf(args.next().expect("arg")),
        "rindex" => Filter::RIndexOf(args.next().expect("arg")),
        "indices" => Filter::IndicesOf(args.next().expect("arg")),
        "strftime" => Filter::StrfTime(args.next().expect("arg")),
        "strflocaltime" => Filter::StrfLocalTime(args.next().expect("arg")),
        "strptime" => Filter::StrpTime(args.next().expect("arg")),
        "path" => Filter::PathOf(args.next().expect("arg")),
        "getpath" => Filter::GetPath(args.next().expect("arg")),
        "delpaths" => Filter::DelPaths(args.next().expect("arg")),
        "del" => Filter::Update {
            op: UpdateOp::Modify,
            path: args.next().expect("path"),
            expr: Box::new(Filter::Empty),
        },
        "repeat" => Filter::Repeat(args.next().expect("expr")),
        "paths" => Filter::Paths(args.next()),
        "first" => Filter::First(args.next()),
        "last" => Filter::Last(args.next()),
        "error" => match args.next() {
            None => Filter::Error,
            Some(arg) => Filter::Raise(arg),
        },
        "nth" => Filter::Nth {
            index: args.next().expect("index"),
            expr: args.next(),
        },
        "range" => match args.len() {
            1 => Filter::Range {
                from: None,
                to: args.next().expect("to"),
                step: None,
            },
            2 => Filter::Range {
                from: Some(args.next().expect("from")),
                to: args.next().expect("to"),
                step: None,
            },
            _ => Filter::Range {
                from: Some(args.next().expect("from")),
                to: args.next().expect("to"),
                step: Some(args.next().expect("step")),
            },
        },
        "limit" => Filter::Limit(args.next().expect("count"), args.next().expect("expr")),
        "until" => Filter::Until(args.next().expect("cond"), args.next().expect("next")),
        "while" => Filter::While(args.next().expect("cond"), args.next().expect("update")),
        "setpath" => Filter::SetPath(args.next().expect("path"), args.next().expect("value")),
        "tostream" => compile_stream(TOSTREAM),
        "isnan" => Filter::NumTest(NumTest::Nan),
        "isinfinite" => Filter::NumTest(NumTest::Infinite),
        "isnormal" => Filter::NumTest(NumTest::Normal),
        "isfinite" => Filter::NumTest(NumTest::Finite),
        "halt" => Filter::Halt,
        "debug" => match args.next() {
            None => Filter::Debug,
            Some(msgs) => with_stream_arg(DEBUG_MSGS, msgs),
        },
        "stderr" => Filter::Stderr,
        "input_line_number" => Filter::InputLine,
        "builtins" => Filter::Builtins,
        "get_search_list" => Filter::SearchList,
        "modulemeta" => Filter::ModuleMeta,
        "finites" => compile_stream(FINITES),
        "normals" => compile_stream(NORMALS),
        "get_jq_origin" => Filter::JqOrigin,
        "get_prog_origin" => Filter::ProgOrigin,
        "reverse" => compile_stream(REVERSE),
        "in" => with_stream_arg(IN_VALUE, args.next().expect("in")),
        "bsearch" => with_stream_arg(BSEARCH, args.next().expect("bsearch")),
        "format" => Filter::FormatBy(args.next().expect("format")),
        "fromdateiso8601" => compile_stream(FROM_ISO),
        "todateiso8601" => compile_stream(TO_ISO),
        "JOIN" => with_call_args(join_source(args.len()), unbox_keep(args)),
        "match" | "test" | "capture" => regex_builtin(name, args.collect()),
        "scan" => scan_call(args.next().expect("scan"), args.next()),
        "splits" => splits_call(args.next().expect("splits"), args.next()),
        "sub" => sub_call(
            args.next().expect("sub"),
            args.next().expect("sub"),
            args.next(),
        ),
        "gsub" => gsub_call(
            args.next().expect("gsub"),
            args.next().expect("gsub"),
            args.next(),
        ),
        "halt_error" => match args.next() {
            None => compile_stream(HALT_ERROR),
            Some(code) => Filter::HaltError(code),
        },
        "pick" => with_stream_arg(PICK, args.next().expect("pick")),
        "transpose" => compile_stream(TRANSPOSE),
        "combinations" => match args.next() {
            None => compile_stream(COMBINATIONS),
            Some(count) => with_stream_arg(COMBINATIONS_N, count),
        },
        "fromstream" => with_stream_arg(FROMSTREAM, args.next().expect("fromstream")),
        "truncate_stream" => with_stream_arg(TRUNCATE_STREAM, args.next().expect("truncate")),
        _ => match math::kind(name).expect("math builtin") {
            Kind::Unary(op) => Filter::Math1(op),
            Kind::Abs => Filter::Abs,
            Kind::Pow10 => Filter::Pow10,
            Kind::Binary(op) => Filter::Math2(op, args.next().expect("x"), args.next().expect("y")),
            Kind::Fma => Filter::Math3(
                args.next().expect("x"),
                args.next().expect("y"),
                args.next().expect("z"),
            ),
        },
    }
}

const REVERSE: &str = "def reverse: [.[length - 1 - range(0;length)]]; reverse";

const IN_VALUE: &str = "def in(xs): . as $x | xs | has($x); in(null)";

const BSEARCH: &str = "def bsearch($target): if length == 0 then -1 elif length == 1 then if $target == .[0] then 0 elif $target < .[0] then -1 else -2 end else . as $in | [0, length-1, null] | until(.[0] > .[1]; if .[2] != null then (.[1] = -1) else (((.[1] + .[0]) / 2) | floor) as $mid | $in[$mid] as $monkey | if $monkey == $target then (.[2] = $mid) elif .[0] == .[1] then (.[1] = -1) elif $monkey < $target then (.[0] = ($mid + 1)) else (.[1] = ($mid - 1)) end end) | if .[2] == null then if $in[.[0]] < $target then (-2 - .[0]) else (-1 - .[0]) end else .[2] end end; bsearch(null)";

const JOIN2: &str = "def JOIN($idx; idx_expr): [.[] | [., $idx[idx_expr]]]; JOIN(null; null)";
const JOIN3: &str =
    "def JOIN($idx; stream; idx_expr): stream | [., $idx[idx_expr]]; JOIN(null; null; null)";
const JOIN4: &str = "def JOIN($idx; stream; idx_expr; join_expr): stream | [., $idx[idx_expr]] | join_expr; JOIN(null; null; null; null)";

const FROM_ISO: &str =
    "def fromdateiso8601: strptime(\"%Y-%m-%dT%H:%M:%SZ\")|mktime; fromdateiso8601";
const TO_ISO: &str = "def todateiso8601: strftime(\"%Y-%m-%dT%H:%M:%SZ\"); todateiso8601";

fn join_source(arity: usize) -> &'static str {
    match arity {
        2 => JOIN2,
        3 => JOIN3,
        _ => JOIN4,
    }
}

fn unbox_keep(args: std::vec::IntoIter<Box<Filter>>) -> Vec<Filter> {
    args.map(|arg| *arg).collect()
}

fn with_call_args(source: &str, call_args: Vec<Filter>) -> Filter {
    let mut filter = compile_stream(source);
    let Filter::Define { rest, .. } = &mut filter else {
        panic!("builtin is not a definition");
    };
    let Filter::Call { args, .. } = rest.as_mut() else {
        panic!("builtin is not a call");
    };
    args.clear();
    args.extend(call_args);
    filter
}

/// `debug(msgs)` prints each message and then yields the input.
const DEBUG_MSGS: &str = "def debug(msgs): (msgs | debug | empty), .; debug(null)";

/// `finites` keeps numbers that are not infinite. `nan` stays.
const FINITES: &str = "def finites: select(isfinite); finites";

/// `normals` keeps finite nonzero normal numbers.
const NORMALS: &str = "def normals: select(isnormal); normals";

/// `halt_error` with no argument is exit status 5.
const HALT_ERROR: &str = "def halt_error: halt_error(5); halt_error";

/// jq's `pick`. Each path is copied onto a value that starts as null.
const PICK: &str = "def pick(pathexps): . as $in | reduce path(pathexps) as $a (null; setpath($a; $in|getpath($a))); pick(null)";

/// jq's `transpose`. Short rows are padded with null.
const TRANSPOSE: &str =
    "def transpose: [range(0; map(length)|max // 0) as $i | [.[][$i]]]; transpose";

/// jq's `combinations`. One array from each input array, the first array varying slowest.
const COMBINATIONS: &str = "def combinations: if length == 0 then [] else .[0][] as $x | (.[1:] | combinations) as $y | [$x] + $y end; combinations";

/// jq's `combinations(n)`. `n` is a filter. The input is repeated once per value `range` yields.
const COMBINATIONS_N: &str =
    "def combinations(n): . as $dot | [range(n) | $dot] | combinations; combinations(null)";

/// jq's `tostream`. Paths of scalars come out first, then one closer for each container.
const TOSTREAM: &str =
    "path(def r: (.[]?|r), .; r) as $p | getpath($p) | reduce path(.[]?) as $q ([$p, .]; [$p+$q])";

/// jq's `fromstream`. The call at the end is replaced with the real argument.
const FROMSTREAM: &str = "def fromstream(i): {x: null, e: false} as $init | foreach i as $i ($init; if .e then $init else . end | if $i|length == 2 then setpath([\"e\"]; $i[0]|length==0) | setpath([\"x\"]+$i[0]; $i[1]) else setpath([\"e\"]; $i[0]|length==1) end; if .e then .x else empty end); fromstream(null)";

/// jq's `truncate_stream`. The argument runs on `null`, and `.` is only the depth.
const TRUNCATE_STREAM: &str = "def truncate_stream(stream): . as $n | null | stream | . as $input | if (.[0]|length) > $n then setpath([0];$input[0][$n:]) else empty end; truncate_stream(null)";

fn compile_stream(source: &str) -> Filter {
    match compile(source) {
        Ok(filter) => filter,
        Err(ProgramError::Report(text)) => panic!("stream builtin failed to compile: {text}"),
        Err(ProgramError::Syntax) => panic!("stream builtin failed to compile"),
    }
}

fn with_stream_arg(source: &str, arg: Box<Filter>) -> Filter {
    let mut filter = compile_stream(source);
    let Filter::Define { rest, .. } = &mut filter else {
        panic!("stream builtin is not a definition");
    };
    let Filter::Call { args, .. } = rest.as_mut() else {
        panic!("stream builtin is not a call");
    };
    args.clear();
    args.push(*arg);
    filter
}

pub(super) fn bind_names(pattern: &Pattern, alts: &[Pattern], out: &mut Vec<String>) {
    pattern_names(pattern, out);
    for alt in alts {
        pattern_names(alt, out);
    }
}

pub(super) fn pattern_names(pattern: &Pattern, out: &mut Vec<String>) {
    match pattern {
        Pattern::Var(name) => out.push(name.clone()),
        Pattern::Array(items) => {
            for item in items {
                pattern_names(item, out);
            }
        }
        Pattern::Object(fields) => {
            for field in fields {
                if let Some(name) = &field.also {
                    out.push(name.clone());
                }
                pattern_names(&field.pattern, out);
            }
        }
    }
}

pub(super) struct Parser<'a> {
    src: &'a str,
    index: usize,
    /// `if`, `reduce`, and `foreach` are not terms. A fold stream uses this
    /// so `reduce if` stays a syntax error while `(if ...)` is still a term.
    reject_block: bool,
    /// Object values reject `try`. Parentheses and arguments clear it.
    reject_try: bool,
    /// Inside `"\( ... )"`. A `)` there ends the interpolation.
    interp: u32,
    /// The leading minus of a `try` operand. `?` there marks only the last suffix.
    suffix_question: bool,
    /// Inner scopes are at the back. Each `def` body gets its own scope.
    scopes: Vec<PScope>,
    /// Load `import` and `include`. `modulemeta` only records them.
    link: bool,
    /// This source is a library. A filter after the definitions is rejected.
    library: bool,
    /// `module` header, if the file has one.
    meta: Option<Value>,
    /// Import records, in source order.
    deps: Vec<Value>,
    /// Top-level definitions, in source order.
    exports: Vec<super::modules::Export>,
    /// Data imports. `$name::name` reads these.
    data_names: Vec<String>,
    /// Values bound around a linked module's definitions.
    data: Vec<(String, Value)>,
    /// The file ended on a top-level `def` or directive, with no filter after it.
    tail_missing: bool,
    /// Imports and includes, loaded after the whole preamble is read.
    pending: Vec<PendingImport>,
    /// Visible `label` names, inner ones at the back. The number is the slot.
    labels: Vec<(String, u32)>,
    next_slot: u32,
}

struct PendingImport {
    path: String,
    /// `None` is `include`.
    alias: Option<String>,
    data: bool,
    /// `None` keeps the working directory and the library path.
    search: Option<Value>,
}

enum Ready {
    Data(Value),
    Code(Rc<Vec<super::modules::Export>>),
}

impl<'a> Parser<'a> {
    fn open(src: &'a str, link: bool) -> Self {
        Parser {
            src,
            index: 0,
            reject_block: false,
            reject_try: false,
            interp: 0,
            suffix_question: false,
            scopes: vec![PScope {
                funcs: Vec::new(),
                params: Vec::new(),
            }],
            link,
            library: false,
            meta: None,
            deps: Vec::new(),
            exports: Vec::new(),
            data_names: Vec::new(),
            data: Vec::new(),
            tail_missing: false,
            pending: Vec::new(),
            labels: Vec::new(),
            next_slot: 0,
        }
    }

    fn meta_value(&self) -> Value {
        let mut pairs = match &self.meta {
            Some(Value::Object(items)) => items.iter().cloned().collect(),
            _ => Vec::new(),
        };
        super::value::put(
            &mut pairs,
            "deps".to_string(),
            Value::array(self.deps.clone()),
        );
        super::value::put(
            &mut pairs,
            "defs".to_string(),
            Value::array(
                self.exports
                    .iter()
                    .map(|exp| {
                        Value::String(crate::value::Text::from(format!(
                            "{}/{}",
                            exp.name, exp.arity
                        )))
                    })
                    .collect(),
            ),
        );
        Value::object(pairs)
    }
}

struct PScope {
    funcs: Vec<Sig>,
    params: Vec<String>,
}

enum Found {
    Func(Rc<FnCell>),
    Param,
}

struct Sig {
    name: String,
    arity: usize,
    func: Rc<FnCell>,
}

impl<'a> Parser<'a> {
    /// Optional `module`, then `import` and `include`, then the program.
    fn parse_program(&mut self) -> Result<Filter, ProgramError> {
        self.skip_ws();
        if self.at_keyword("module") {
            self.parse_module_directive()?;
        }
        loop {
            self.skip_ws();
            if self.at_keyword("import") {
                self.parse_import_directive()?;
            } else if self.at_keyword("include") {
                self.parse_include_directive()?;
            } else {
                break;
            }
        }
        // A library that ends in a filter is rejected even when an import
        // would have failed. The import errors are kept when the file itself
        // is only definitions. The main program still reports the import
        // error before it parses the rest.
        let import_err = self.resolve_imports().err();
        if import_err.is_some() && !self.library {
            return Err(import_err.expect("import error"));
        }
        self.skip_ws();
        let mut body = if self.eof() {
            if self.meta.is_none() && self.deps.is_empty() {
                return Err(import_err.unwrap_or(ProgramError::Syntax));
            }
            self.tail_missing = true;
            Filter::Empty
        } else {
            self.parse_pipe()?
        };
        if self.library && !self.tail_missing {
            return Err(super::modules::library_main(self.link));
        }
        if let Some(err) = import_err {
            return Err(err);
        }
        for (name, value) in self.data.iter().rev() {
            body = Filter::Bind {
                expr: Box::new(Filter::Literal(value.clone())),
                pattern: Pattern::Var(name.clone()),
                alts: Vec::new(),
                body: Box::new(body),
            };
        }
        Ok(body)
    }

    fn parse_module_directive(&mut self) -> Result<(), ProgramError> {
        let at = self.index;
        if !self.eat_keyword("module") {
            return Err(ProgramError::Syntax);
        }
        self.skip_ws();
        let expr = self.parse_pipe()?;
        let value = match const_one(&expr) {
            Some(value) if matches!(value, Value::Object(_)) => value,
            Some(_) => return Err(report_at(self.src, at, "Module metadata must be an object")),
            None => return Err(report_at(self.src, at, "Module metadata must be constant")),
        };
        if !self.eat(';') {
            return Err(ProgramError::Syntax);
        }
        self.meta = Some(value);
        Ok(())
    }

    fn parse_import_directive(&mut self) -> Result<(), ProgramError> {
        let at = self.index;
        if !self.eat_keyword("import") {
            return Err(ProgramError::Syntax);
        }
        let (path, _path_at) = self.parse_module_path()?;
        if !self.eat_keyword("as") {
            return Err(ProgramError::Syntax);
        }
        self.skip_ws();
        let (alias, data) = if self.eat('$') {
            (self.ident()?, true)
        } else if self.starts_ident() {
            let name = self.ident()?;
            if reserved(&name) {
                return Err(ProgramError::Syntax);
            }
            (name, false)
        } else {
            return Err(ProgramError::Syntax);
        };
        let meta = self.parse_meta(at)?;
        if !self.eat(';') {
            return Err(ProgramError::Syntax);
        }
        let search = search_override(meta.as_ref());
        self.deps
            .push(dep_record(meta.clone(), Some(&alias), data, &path));
        if !self.link && data {
            self.data_names.push(alias.clone());
        }
        self.pending.push(PendingImport {
            path,
            alias: Some(alias),
            data,
            search,
        });
        Ok(())
    }

    fn parse_include_directive(&mut self) -> Result<(), ProgramError> {
        let at = self.index;
        if !self.eat_keyword("include") {
            return Err(ProgramError::Syntax);
        }
        let (path, _path_at) = self.parse_module_path()?;
        let meta = self.parse_meta(at)?;
        if !self.eat(';') {
            return Err(ProgramError::Syntax);
        }
        let search = search_override(meta.as_ref());
        self.deps.push(dep_record(meta, None, false, &path));
        self.pending.push(PendingImport {
            path,
            alias: None,
            data: false,
            search,
        });
        Ok(())
    }

    /// Imports are loaded from the end. Each library that ends in a filter
    /// adds one error and the walk continues. Any other error stops it, and
    /// the library errors already seen stay in front of that one. Successful
    /// imports are installed in source order.
    fn resolve_imports(&mut self) -> Result<(), ProgramError> {
        if !self.link {
            return Ok(());
        }
        let pending = std::mem::take(&mut self.pending);
        let mut ready = Vec::with_capacity(pending.len());
        for _ in 0..pending.len() {
            ready.push(None);
        }
        let mut mains = 0usize;
        let mut stop: Option<ProgramError> = None;
        for index in (0..pending.len()).rev() {
            let item = &pending[index];
            let loaded = if item.data {
                super::modules::load_data(&item.path, item.search.as_ref()).map(Ready::Data)
            } else {
                super::modules::load_code(&item.path, item.search.as_ref()).map(Ready::Code)
            };
            match loaded {
                Ok(value) => ready[index] = Some(value),
                Err(ProgramError::Report(text)) => {
                    if let Some(count) = super::modules::library_main_count(&text) {
                        mains += count;
                    } else {
                        stop = Some(ProgramError::Report(text));
                        break;
                    }
                }
                Err(err) => {
                    stop = Some(err);
                    break;
                }
            }
        }
        if mains > 0 || stop.is_some() {
            return Err(super::modules::combine_import_errors(mains, stop));
        }
        for (item, value) in pending.into_iter().zip(ready) {
            let value = value.expect("import");
            match value {
                Ready::Data(value) => {
                    let name = item.alias.unwrap_or_default();
                    self.data_names.push(name.clone());
                    self.data.push((name, value));
                }
                Ready::Code(exports) => self.install(item.alias.as_deref(), &exports),
            }
        }
        Ok(())
    }

    /// A string literal. Interpolation is rejected. The index is the quote.
    fn parse_module_path(&mut self) -> Result<(String, usize), ProgramError> {
        self.skip_ws();
        let at = self.index;
        if self.peek() != Some('"') {
            return Err(ProgramError::Syntax);
        }
        match self.parse_string_expr()? {
            Filter::Literal(Value::String(ref text)) => Ok((text.to_string(), at)),
            _ => Err(report_at(self.src, at, "Import path must be constant")),
        }
    }

    /// Nothing when the next token is `;`. Otherwise one constant object.
    fn parse_meta(&mut self, at: usize) -> Result<Option<Value>, ProgramError> {
        self.skip_ws();
        if self.peek() == Some(';') || self.eof() {
            return Ok(None);
        }
        let expr = self.parse_pipe()?;
        match const_one(&expr) {
            Some(value) if matches!(value, Value::Object(_)) => Ok(Some(value)),
            Some(_) => Err(report_at(self.src, at, "Module metadata must be an object")),
            None => Err(report_at(self.src, at, "Module metadata must be constant")),
        }
    }

    fn install(&mut self, alias: Option<&str>, exports: &[super::modules::Export]) {
        let scope = &mut self.scopes[0].funcs;
        for exp in exports {
            let name = match alias {
                Some(alias) => format!("{alias}::{}", exp.name),
                None => exp.name.clone(),
            };
            scope.push(Sig {
                name,
                arity: exp.arity,
                func: Rc::clone(&exp.func),
            });
        }
    }

    fn at_keyword(&self, word: &str) -> bool {
        let rest = &self.src[self.index..];
        if rest.len() < word.len() || !rest.starts_with(word) {
            return false;
        }
        match rest[word.len()..].chars().next() {
            None => true,
            Some(ch) if ch.is_ascii_alphanumeric() || ch == '_' => false,
            Some(_) => true,
        }
    }

    /// `foo::bar` with no space around `::`.
    fn qualify(&mut self, mut name: String) -> Result<String, ProgramError> {
        while self.followed_by_colon_name() {
            self.index += 2;
            let next = self.ident()?;
            name.push_str("::");
            name.push_str(&next);
        }
        Ok(name)
    }

    fn followed_by_colon_name(&self) -> bool {
        let rest = &self.src[self.index..];
        if !rest.starts_with("::") {
            return false;
        }
        rest[2..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
    }

    fn parse_pipe(&mut self) -> Result<Filter, ProgramError> {
        let mut filter = self.parse_comma()?;
        while self.sees_pipe() {
            let token_at = self.index;
            self.bump();
            let token_end = self.index;
            if let Some(err) = self.after_operator(token_at, token_end) {
                return Err(err);
            }
            let right = self.parse_comma()?;
            filter = Filter::Pipe(Box::new(filter), Box::new(right));
        }
        Ok(filter)
    }

    fn parse_comma(&mut self) -> Result<Filter, ProgramError> {
        let mut filter = self.parse_alt()?;
        while self.sees(',') {
            let token_at = self.index;
            self.bump();
            let token_end = self.index;
            if let Some(err) = self.after_operator(token_at, token_end) {
                return Err(err);
            }
            let right = self.parse_alt()?;
            filter = Filter::Comma(Box::new(filter), Box::new(right));
        }
        Ok(filter)
    }

    /// `//` is right-associative. It is looser than an update and tighter than `,`.
    fn parse_alt(&mut self) -> Result<Filter, ProgramError> {
        let left = self.parse_update()?;
        if self.eat_alt() {
            let token_end = self.index;
            let token_at = token_end - 2;
            if let Some(err) = self.after_operator(token_at, token_end) {
                return Err(err);
            }
            let right = self.parse_alt()?;
            return Ok(Filter::Alt(Box::new(left), Box::new(right)));
        }
        Ok(left)
    }

    /// `=` does not chain. `//=` is one operator, not `//` followed by `=`.
    fn parse_update(&mut self) -> Result<Filter, ProgramError> {
        let left = self.parse_or()?;
        let Some(op) = self.take_update() else {
            return Ok(left);
        };
        let right = self.parse_or()?;
        if self.sees_update() {
            return Err(ProgramError::Syntax);
        }
        Ok(Filter::Update {
            op,
            path: Box::new(left),
            expr: Box::new(right),
        })
    }

    fn parse_or(&mut self) -> Result<Filter, ProgramError> {
        let mut left = self.parse_and()?;
        while self.eat_keyword("or") {
            let token_end = self.index;
            let token_at = token_end - 2;
            if let Some(err) = self.after_operator(token_at, token_end) {
                return Err(err);
            }
            let right = self.parse_and()?;
            left = Filter::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Filter, ProgramError> {
        let mut left = self.parse_cmp()?;
        while self.eat_keyword("and") {
            let token_end = self.index;
            let token_at = token_end - 3;
            if let Some(err) = self.after_operator(token_at, token_end) {
                return Err(err);
            }
            let right = self.parse_cmp()?;
            left = Filter::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// Comparisons do not chain. `1<2<3` is a syntax error.
    fn parse_cmp(&mut self) -> Result<Filter, ProgramError> {
        let left = self.parse_add()?;
        self.skip_ws();
        let Some((op, len)) = cmp_at(&self.src[self.index..]) else {
            return Ok(left);
        };
        let token_at = self.index;
        self.index += len;
        let token_end = self.index;
        if let Some(err) = self.after_operator(token_at, token_end) {
            return Err(err);
        }
        let right = self.parse_add()?;
        if self.cmp_op_pending() {
            return Err(ProgramError::Syntax);
        }
        Ok(Filter::Bin {
            op,
            left: Box::new(left),
            right: Box::new(right),
        })
    }

    fn parse_add(&mut self) -> Result<Filter, ProgramError> {
        let mut left = self.parse_signed()?;
        loop {
            self.skip_ws();
            // `+=` and `-=` are updates, not addition followed by assignment.
            let op = match (self.peek(), self.peek_at(1)) {
                (Some('+'), Some('=')) | (Some('-'), Some('=')) => break,
                (Some('+'), _) => Op::Add,
                (Some('-'), _) => Op::Sub,
                _ => break,
            };
            // `1 +` has nothing after the operator. jq names that as the
            // end of the file, at the operator.
            let token_at = self.index;
            self.bump();
            let token_end = self.index;
            if let Some(err) = self.after_operator(token_at, token_end) {
                return Err(err);
            }
            let right = self.parse_signed()?;
            left = Filter::Bin {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// A leading `-` binds looser than `*` `/` `%`, so `-1/2` is `-(1/2)`.
    fn parse_signed(&mut self) -> Result<Filter, ProgramError> {
        self.skip_ws();
        if self.peek() == Some('-') {
            self.bump();
            let operand = self.parse_signed()?;
            return Ok(Filter::Neg(Box::new(operand)));
        }
        self.parse_mul()
    }

    fn parse_mul(&mut self) -> Result<Filter, ProgramError> {
        let mut left = self.parse_binding()?;
        while let Some(op) = self.mul_op() {
            let token_end = self.index;
            let token_at = token_end - 1;
            if let Some(err) = self.after_operator(token_at, token_end) {
                return Err(err);
            }
            // A minus here is the start of the operand, and that operand
            // includes the tighter `*` `/` `%` that follow it: `2/-3*4`
            // is `2/-(3*4)`. Without the minus, `8/2/2` stays left-associative.
            let right = if self.sees('-') {
                self.parse_signed()?
            } else {
                self.parse_binding()?
            };
            left = Filter::Bin {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// `as` binds tighter than `*` and a leading `-`. The body is the rest of
    /// the expression, and `.` there is still the input of the `as`.
    fn parse_binding(&mut self) -> Result<Filter, ProgramError> {
        let left = self.parse_juxt()?;
        if !self.eat_keyword("as") {
            return Ok(left);
        }
        let (pattern, alts) = self.parse_pattern_alts()?;
        if !self.eat('|') {
            return Err(ProgramError::Syntax);
        }
        let body = self.parse_pipe()?;
        Ok(Filter::Bind {
            expr: Box::new(left),
            pattern,
            alts,
            body: Box::new(body),
        })
    }

    fn parse_juxt(&mut self) -> Result<Filter, ProgramError> {
        let mut filter = self.parse_postfix()?;
        loop {
            self.skip_ws();
            if matches!(self.peek(), Some('.' | '(')) {
                // `length(1)` and `map(f)(1)` are calls. A call this version
                // does not take is a syntax error, not a pipe.
                if self.peek() == Some('(') && forbids_call(&filter) {
                    return Err(ProgramError::Syntax);
                }
                let right = self.parse_postfix()?;
                filter = Filter::Pipe(Box::new(filter), Box::new(right));
            } else {
                break;
            }
        }
        Ok(filter)
    }

    fn parse_postfix(&mut self) -> Result<Filter, ProgramError> {
        let mut filter = self.parse_primary()?;
        // `?` just after `]` skips a failed index. `?` after `)` catches the
        // whole expression, including an error in a bound.
        let mut bracket = false;
        loop {
            self.skip_ws();
            if self.eat('?') {
                filter = if self.suffix_question || bracket {
                    optional_last(filter)
                } else {
                    Filter::Optional(Box::new(filter))
                };
                bracket = false;
                continue;
            }
            if self.peek() == Some('[') {
                filter = self.parse_index(Some(filter))?;
                bracket = true;
                continue;
            }
            if self.peek() == Some('.') {
                let bytes = self.src.as_bytes();
                // `..` is recurse, and only when it is the whole term.
                if bytes.get(self.index + 1) == Some(&b'.') {
                    break;
                }
                self.bump();
                bracket = false;
                if self.peek() == Some('"') || self.spaced_quote() {
                    let key = self.parse_quoted_key()?;
                    filter = at_key(Some(filter), key);
                    continue;
                }
                if self.starts_ident() {
                    let name = self.ident()?;
                    filter = chain(Some(filter), Filter::Field(name));
                    continue;
                }
                if self.peek() == Some('[') {
                    filter = self.parse_index(Some(filter))?;
                    bracket = true;
                    continue;
                }
                return Err(ProgramError::Syntax);
            }
            break;
        }
        Ok(filter)
    }

    fn parse_primary(&mut self) -> Result<Filter, ProgramError> {
        self.skip_ws();
        if let Some(err) = self.lone_closing_brace() {
            return Err(err);
        }
        if self.eat('(') {
            let saved_block = self.reject_block;
            let saved_try = self.reject_try;
            self.reject_block = false;
            self.reject_try = false;
            let inner = self.parse_pipe();
            self.reject_block = saved_block;
            self.reject_try = saved_try;
            let inner = inner?;
            let token_end = self.index;
            self.skip_ws();
            if self.eof() {
                return Err(self.unexpected_eof(token_start(self.src, token_end), token_end));
            }
            if !self.eat(')') {
                return Err(ProgramError::Syntax);
            }
            return Ok(inner);
        }
        if self.peek() == Some('$') {
            return self.parse_dollar();
        }
        if self.peek() == Some('[') {
            return self.parse_array();
        }
        if self.peek() == Some('{') {
            return self.parse_object();
        }
        if self.peek() == Some('@') {
            return self.parse_format();
        }
        if self.peek() == Some('"') {
            return self.parse_string_expr();
        }
        if self.peek() == Some('.') {
            let bytes = self.src.as_bytes();
            if bytes
                .get(self.index + 1)
                .is_some_and(|b| b.is_ascii_digit())
            {
                return self.parse_number();
            }
            return self.parse_path();
        }
        if self.peek().is_some_and(|c| c.is_ascii_digit()) {
            return self.parse_number();
        }
        if self.starts_ident() {
            let at = self.index;
            let name = self.ident()?;
            return self.parse_name(at, name);
        }
        Err(ProgramError::Syntax)
    }

    /// A name after keywords that start a form have been set aside.
    /// The first `(` is the call. A second `(` is juxtaposition, and
    /// `forbids_call` rejects it.
    fn parse_name(&mut self, at: usize, name: String) -> Result<Filter, ProgramError> {
        let qualified = self.followed_by_colon_name();
        if !qualified {
            match name.as_str() {
                "def" if !self.reject_block => return self.parse_def(),
                "if" if !self.reject_block => return self.parse_if(),
                "reduce" if !self.reject_block => return self.parse_fold(false),
                "foreach" if !self.reject_block => return self.parse_fold(true),
                "try" if !self.reject_block && !self.reject_try => return self.parse_try(),
                "label" if !self.reject_block && !self.reject_try => return self.parse_label(),
                "break" => return self.parse_break(at),
                "true" => return Ok(Filter::Literal(Value::Bool(true))),
                "false" => return Ok(Filter::Literal(Value::Bool(false))),
                "null" => return Ok(Filter::Literal(Value::Null)),
                _ => {}
            }
            if reserved(&name) {
                return Err(ProgramError::Syntax);
            }
        }
        let name = self.qualify(name)?;
        self.skip_ws();
        let args = if self.peek() == Some('(') {
            self.parse_call_args()?
        } else {
            Vec::new()
        };
        let arity = args.len();
        // An inner function hides a parameter of the same name. A parameter
        // hides an outer function. `$name` is also the filter `name`.
        if let Some(found) = self.resolve(&name, arity) {
            return Ok(match found {
                Found::Func(func) => Filter::Call {
                    func: Rc::downgrade(&func),
                    args: unbox(args),
                },
                Found::Param => Filter::Param(name),
            });
        }
        if arity == 0 && matches!(name.as_str(), "nan" | "infinite") {
            let text = if name == "nan" {
                number::nan_text()
            } else {
                number::pos_inf()
            };
            return Ok(Filter::Literal(Value::Number(crate::value::Text::from(
                text,
            ))));
        }
        if is_builtin(&name, arity) {
            return Ok(build_builtin(&name, args));
        }
        Ok(Filter::Unresolved {
            at,
            name,
            arity,
            args: unbox(args),
        })
    }

    /// `def name: body; rest` or `def name(params): body; rest`.
    /// The name is visible in the body, so the function can call itself.
    /// It is not visible in the argument expressions of that call.
    fn parse_def(&mut self) -> Result<Filter, ProgramError> {
        self.skip_ws();
        let ident = self.ident()?;
        let name = self.qualify(ident)?;
        if !name.contains("::") && reserved(&name) {
            return Err(ProgramError::Syntax);
        }
        self.skip_ws();
        let params = if self.peek() == Some('(') {
            self.parse_params()?
        } else {
            Vec::new()
        };
        if !self.eat(':') {
            return Err(ProgramError::Syntax);
        }
        let func = Rc::new(FnCell {
            params: params.clone(),
            body: RefCell::new(Rc::new(Filter::Empty)),
        });
        if self.scopes.len() == 1 {
            self.exports.push(super::modules::Export {
                name: name.clone(),
                arity: params.len(),
                func: Rc::clone(&func),
            });
        }
        // The name is visible in the body and the rest, then it drops out so
        // a `def` inside parentheses does not shadow the code after them.
        let slot = self.define(&name, Rc::clone(&func));
        let mut names = Vec::with_capacity(params.len());
        for param in &params {
            match param {
                FnParam::Filter(name) | FnParam::Value(name) => names.push(name.clone()),
            }
        }
        self.scopes.push(PScope {
            funcs: Vec::new(),
            params: names,
        });
        let body = self.parse_pipe();
        self.scopes.pop();
        let body = body?;
        if !self.eat(';') {
            return Err(ProgramError::Syntax);
        }
        *func.body.borrow_mut() = Rc::new(body);
        self.skip_ws();
        let rest = if self.eof() {
            // A top-level definition with nothing after it is not a program.
            // The same file is a valid library. A nested definition that runs
            // out of input is a syntax error when its outer `def` is closed.
            if self.scopes.len() == 1 {
                self.tail_missing = true;
            }
            Ok(Filter::Empty)
        } else {
            self.parse_pipe()
        };
        self.scopes[slot.0].funcs.remove(slot.1);
        let rest = rest?;
        Ok(Filter::Define {
            func,
            rest: Box::new(rest),
        })
    }

    /// `(name; $name; ...)`. Empty `()` and a trailing `;` are syntax errors.
    fn parse_params(&mut self) -> Result<Vec<FnParam>, ProgramError> {
        if !self.eat('(') {
            return Err(ProgramError::Syntax);
        }
        self.skip_ws();
        if self.peek() == Some(')') {
            return Err(ProgramError::Syntax);
        }
        let mut params = Vec::new();
        loop {
            self.skip_ws();
            if self.eat('$') {
                let name = self.ident()?;
                // `$__loc__` is not a name a parameter can take.
                if name == "__loc__" {
                    return Err(ProgramError::Syntax);
                }
                params.push(FnParam::Value(name));
            } else if self.starts_ident() {
                let name = self.ident()?;
                if reserved(&name) {
                    return Err(ProgramError::Syntax);
                }
                params.push(FnParam::Filter(name));
            } else {
                return Err(ProgramError::Syntax);
            }
            self.skip_ws();
            if self.eat(';') {
                self.skip_ws();
                if self.peek() == Some(')') {
                    return Err(ProgramError::Syntax);
                }
                continue;
            }
            if self.eat(')') {
                break;
            }
            return Err(ProgramError::Syntax);
        }
        Ok(params)
    }

    fn define(&mut self, name: &str, func: Rc<FnCell>) -> (usize, usize) {
        let arity = func.params.len();
        let scope = self.scopes.len() - 1;
        let funcs = &mut self.scopes[scope].funcs;
        let index = funcs.len();
        funcs.push(Sig {
            name: name.to_string(),
            arity,
            func,
        });
        (scope, index)
    }

    /// The innermost scope wins. In one scope a function hides a parameter,
    /// and the later parameter of a name hides the earlier one.
    fn resolve(&self, name: &str, arity: usize) -> Option<Found> {
        for scope in self.scopes.iter().rev() {
            if let Some(sig) = scope
                .funcs
                .iter()
                .rev()
                .find(|sig| sig.name == name && sig.arity == arity)
            {
                return Some(Found::Func(Rc::clone(&sig.func)));
            }
            if arity == 0 && scope.params.iter().rev().any(|param| param == name) {
                return Some(Found::Param);
            }
        }
        None
    }

    /// Arguments of one call, separated by `;`. The count is how many
    /// expressions parsed, which is the arity jq reports.
    fn parse_call_args(&mut self) -> Result<Vec<Box<Filter>>, ProgramError> {
        if !self.eat('(') {
            return Err(ProgramError::Syntax);
        }
        let saved_block = self.reject_block;
        let saved_try = self.reject_try;
        self.reject_block = false;
        self.reject_try = false;
        let parsed = self.parse_separated_args();
        self.reject_block = saved_block;
        self.reject_try = saved_try;
        parsed
    }

    fn parse_separated_args(&mut self) -> Result<Vec<Box<Filter>>, ProgramError> {
        let mut args = vec![Box::new(self.parse_pipe()?)];
        while self.eat(';') {
            args.push(Box::new(self.parse_pipe()?));
        }
        if !self.eat(')') {
            return Err(ProgramError::Syntax);
        }
        Ok(args)
    }

    /// One juxtaposition term. `if`, `reduce`, and `foreach` need parentheses.
    fn parse_stream_term(&mut self) -> Result<Filter, ProgramError> {
        let saved = self.reject_block;
        self.reject_block = true;
        let term = self.parse_juxt();
        self.reject_block = saved;
        term
    }

    fn parse_fold(&mut self, foreach: bool) -> Result<Filter, ProgramError> {
        let stream = self.parse_stream_term()?;
        if !self.eat_keyword("as") {
            return Err(ProgramError::Syntax);
        }
        let (pattern, alts) = self.parse_pattern_alts()?;
        if !self.eat('(') {
            return Err(ProgramError::Syntax);
        }
        let init = self.parse_pipe()?;
        if !self.eat(';') {
            return Err(ProgramError::Syntax);
        }
        let update = self.parse_pipe()?;
        let extract = if foreach && self.eat(';') {
            Some(Box::new(self.parse_pipe()?))
        } else if !foreach && self.sees(';') {
            return Err(ProgramError::Syntax);
        } else {
            None
        };
        if !self.eat(')') {
            return Err(ProgramError::Syntax);
        }
        let stream = Box::new(stream);
        let init = Box::new(init);
        let update = Box::new(update);
        if foreach {
            Ok(Filter::Foreach {
                stream,
                pattern,
                alts,
                init,
                update,
                extract,
            })
        } else {
            Ok(Filter::Reduce {
                stream,
                pattern,
                alts,
                init,
                update,
            })
        }
    }

    /// The first pattern, then each `?//` pattern.
    fn parse_pattern_alts(&mut self) -> Result<(Pattern, Vec<Pattern>), ProgramError> {
        let pattern = self.parse_pattern()?;
        let mut alts = Vec::new();
        while self.eat_question_alt() {
            alts.push(self.parse_pattern()?);
        }
        Ok((pattern, alts))
    }

    /// `?//` between patterns. A lone `?` or `//` stays what it was.
    fn eat_question_alt(&mut self) -> bool {
        self.skip_ws();
        let rest = &self.src[self.index..];
        if rest.starts_with("?//") {
            self.index += 3;
            true
        } else {
            false
        }
    }

    /// `label $name | body`. The name is visible in the body and nowhere after it.
    fn parse_label(&mut self) -> Result<Filter, ProgramError> {
        self.skip_ws();
        if !self.eat('$') {
            return Err(ProgramError::Syntax);
        }
        let name = self.ident()?;
        if name == "__loc__" {
            return Err(ProgramError::Syntax);
        }
        if !self.eat('|') {
            return Err(ProgramError::Syntax);
        }
        let slot = self.next_slot;
        self.next_slot += 1;
        self.labels.push((name, slot));
        let body = self.parse_pipe();
        self.labels.pop();
        Ok(Filter::Label {
            slot,
            body: Box::new(body?),
        })
    }

    /// `break $name`. The slot is filled when a label of that name is visible.
    fn parse_break(&mut self, at: usize) -> Result<Filter, ProgramError> {
        self.skip_ws();
        if !self.eat('$') {
            return Err(ProgramError::Syntax);
        }
        let name = self.ident()?;
        if name == "__loc__" {
            return Err(ProgramError::Syntax);
        }
        let slot = self
            .labels
            .iter()
            .rev()
            .find(|(bound, _)| bound == &name)
            .map(|(_, slot)| *slot);
        Ok(Filter::Break { slot, at, name })
    }

    fn parse_pattern(&mut self) -> Result<Pattern, ProgramError> {
        self.skip_ws();
        if self.eat('$') {
            let name = self.ident()?;
            // `as $__loc__` is rejected. The name is not a binding.
            if name == "__loc__" {
                return Err(ProgramError::Syntax);
            }
            return Ok(Pattern::Var(name));
        }
        if self.eat('[') {
            self.skip_ws();
            if self.peek() == Some(']') {
                return Err(ProgramError::Syntax);
            }
            let mut items = Vec::new();
            loop {
                items.push(self.parse_pattern()?);
                self.skip_ws();
                if self.eat(',') {
                    self.skip_ws();
                    if self.peek() == Some(']') {
                        return Err(ProgramError::Syntax);
                    }
                    continue;
                }
                if self.eat(']') {
                    break;
                }
                return Err(ProgramError::Syntax);
            }
            return Ok(Pattern::Array(items));
        }
        if self.eat('{') {
            self.skip_ws();
            if self.peek() == Some('}') {
                return Err(ProgramError::Syntax);
            }
            let mut fields = Vec::new();
            loop {
                fields.push(self.parse_pat_field()?);
                self.skip_ws();
                if self.eat(',') {
                    self.skip_ws();
                    if self.peek() == Some('}') {
                        return Err(ProgramError::Syntax);
                    }
                    continue;
                }
                if self.eat('}') {
                    break;
                }
                return Err(ProgramError::Syntax);
            }
            return Ok(Pattern::Object(fields));
        }
        Err(ProgramError::Syntax)
    }

    fn parse_pat_field(&mut self) -> Result<PatField, ProgramError> {
        self.skip_ws();
        if self.eat('$') {
            let name = self.ident()?;
            if name == "__loc__" {
                return Err(ProgramError::Syntax);
            }
            self.skip_ws();
            if self.eat(':') {
                let pattern = self.parse_pattern()?;
                return Ok(PatField {
                    key: Box::new(Filter::Literal(Value::String(crate::value::Text::from(
                        name.clone(),
                    )))),
                    pattern,
                    also: Some(name),
                });
            }
            return Ok(PatField {
                key: Box::new(Filter::Literal(Value::String(crate::value::Text::from(
                    name.clone(),
                )))),
                pattern: Pattern::Var(name),
                also: None,
            });
        }
        if self.peek() == Some('(') {
            let pos = self.index;
            self.bump();
            let key = self.parse_pipe()?;
            self.skip_ws();
            if !self.eat(')') || !self.eat(':') {
                return Err(ProgramError::Syntax);
            }
            if let Some(constant) = const_one(&key) {
                if !matches!(constant, Value::String(_)) {
                    return Err(report_at(self.src, pos, &key_message(&constant)));
                }
            }
            let pattern = self.parse_pattern()?;
            return Ok(PatField {
                key: Box::new(key),
                pattern,
                also: None,
            });
        }
        if self.peek() == Some('"') {
            let key = self.parse_string_expr()?;
            if !self.eat(':') {
                return Err(ProgramError::Syntax);
            }
            let pattern = self.parse_pattern()?;
            return Ok(PatField {
                key: Box::new(key),
                pattern,
                also: None,
            });
        }
        if self.starts_ident() {
            let name = self.ident_key()?;
            if !self.eat(':') {
                return Err(ProgramError::Syntax);
            }
            let pattern = self.parse_pattern()?;
            return Ok(PatField {
                key: Box::new(Filter::Literal(Value::String(crate::value::Text::from(
                    name,
                )))),
                pattern,
                also: None,
            });
        }
        Err(ProgramError::Syntax)
    }

    /// Pipe and unary minus, without comma or the arithmetic operators.
    /// Object values use this, so `{a:1+2}` stays a syntax error.
    /// `try` is rejected here. `(try ...)` and a call argument still accept it.
    fn parse_expd(&mut self) -> Result<Filter, ProgramError> {
        let saved = self.reject_try;
        self.reject_try = true;
        let parsed = self.parse_expd_pipe();
        self.reject_try = saved;
        parsed
    }

    fn parse_expd_pipe(&mut self) -> Result<Filter, ProgramError> {
        let mut filter = self.parse_expd_unary()?;
        while self.eat('|') {
            let right = self.parse_expd_unary()?;
            filter = Filter::Pipe(Box::new(filter), Box::new(right));
        }
        Ok(filter)
    }

    /// `try` and `catch` stop before `+`, `*`, `|`, and `,`.
    /// A leading `-` still takes the product that follows it.
    fn parse_try(&mut self) -> Result<Filter, ProgramError> {
        let body = self.parse_try_operand()?;
        let handler = if self.eat_keyword("catch") {
            Some(Box::new(self.parse_try_operand()?))
        } else {
            None
        };
        Ok(Filter::Try {
            body: Box::new(body),
            handler,
        })
    }

    fn parse_try_operand(&mut self) -> Result<Filter, ProgramError> {
        self.skip_ws();
        if self.peek() == Some('-') && self.peek_at(1) != Some('=') {
            let saved = self.suffix_question;
            self.suffix_question = true;
            let operand = self.parse_signed();
            self.suffix_question = saved;
            return operand;
        }
        let primary = self.parse_primary()?;
        let filter = self.parse_try_tail(primary)?;
        if !self.eat_keyword("as") {
            return Ok(filter);
        }
        let (pattern, alts) = self.parse_pattern_alts()?;
        if !self.eat('|') {
            return Err(ProgramError::Syntax);
        }
        let body = self.parse_pipe()?;
        Ok(Filter::Bind {
            expr: Box::new(filter),
            pattern,
            alts,
            body: Box::new(body),
        })
    }

    /// Indexing and fields after the primary. `?` applies only once a suffix
    /// has been written, and only to that last suffix.
    fn parse_try_tail(&mut self, mut filter: Filter) -> Result<Filter, ProgramError> {
        let mut extended = false;
        loop {
            self.skip_ws();
            if self.peek() == Some('[') {
                filter = self.parse_index(Some(filter))?;
                extended = true;
                continue;
            }
            if self.peek() == Some('.') {
                let bytes = self.src.as_bytes();
                if bytes.get(self.index + 1) == Some(&b'.') {
                    break;
                }
                self.bump();
                if self.peek() == Some('"') || self.spaced_quote() {
                    let key = self.parse_quoted_key()?;
                    filter = at_key(Some(filter), key);
                    extended = true;
                    continue;
                }
                if self.starts_ident() {
                    let name = self.ident()?;
                    filter = chain(Some(filter), Filter::Field(name));
                    extended = true;
                    continue;
                }
                if self.peek() == Some('[') {
                    filter = self.parse_index(Some(filter))?;
                    extended = true;
                    continue;
                }
                return Err(ProgramError::Syntax);
            }
            if extended && self.eat('?') {
                filter = optional_last(filter);
                continue;
            }
            break;
        }
        Ok(filter)
    }

    fn parse_expd_unary(&mut self) -> Result<Filter, ProgramError> {
        self.skip_ws();
        if self.peek() == Some('-') {
            self.bump();
            let operand = self.parse_expd_unary()?;
            return Ok(Filter::Neg(Box::new(operand)));
        }
        self.parse_postfix()
    }

    fn parse_if(&mut self) -> Result<Filter, ProgramError> {
        let cond = self.parse_pipe()?;
        if !self.eat_keyword("then") {
            return Err(ProgramError::Syntax);
        }
        let yes = self.parse_pipe()?;
        let no = self.parse_else()?;
        Ok(Filter::If {
            cond: Box::new(cond),
            yes: Box::new(yes),
            no: Box::new(no),
        })
    }

    fn parse_else(&mut self) -> Result<Filter, ProgramError> {
        if self.eat_keyword("elif") {
            let cond = self.parse_pipe()?;
            if !self.eat_keyword("then") {
                return Err(ProgramError::Syntax);
            }
            let yes = self.parse_pipe()?;
            let no = self.parse_else()?;
            return Ok(Filter::If {
                cond: Box::new(cond),
                yes: Box::new(yes),
                no: Box::new(no),
            });
        }
        let no = if self.eat_keyword("else") {
            self.parse_pipe()?
        } else {
            // A missing else yields the input, which is null under -n.
            Filter::Identity
        };
        let token_end = self.index;
        if !self.eat_keyword("end") {
            if self.eof() {
                return Err(self.unterminated_if(token_start(self.src, token_end), token_end));
            }
            return Err(ProgramError::Syntax);
        }
        Ok(no)
    }

    fn eat_keyword(&mut self, word: &str) -> bool {
        self.skip_ws();
        let rest = &self.src[self.index..];
        if rest.len() < word.len() || !rest.starts_with(word) {
            return false;
        }
        let boundary = match rest[word.len()..].chars().next() {
            None => true,
            Some(ch) if ch.is_ascii_alphanumeric() || ch == '_' => false,
            Some(_) => true,
        };
        if !boundary {
            return false;
        }
        self.index += word.len();
        true
    }

    fn eat_alt(&mut self) -> bool {
        self.skip_ws();
        let rest = &self.src[self.index..];
        // `//=` is an update, not an alternative followed by assignment.
        if rest.starts_with("//") && !rest.starts_with("//=") {
            self.index += 2;
            true
        } else {
            false
        }
    }

    fn take_update(&mut self) -> Option<UpdateOp> {
        self.skip_ws();
        let rest = &self.src[self.index..];
        let (op, len) = if rest.starts_with("|=") {
            (UpdateOp::Modify, 2)
        } else if rest.starts_with("+=") {
            (UpdateOp::Add, 2)
        } else if rest.starts_with("-=") {
            (UpdateOp::Sub, 2)
        } else if rest.starts_with("*=") {
            (UpdateOp::Mul, 2)
        } else if rest.starts_with("/=") {
            (UpdateOp::Div, 2)
        } else if rest.starts_with("%=") {
            (UpdateOp::Rem, 2)
        } else if rest.starts_with("//=") {
            (UpdateOp::Alt, 3)
        } else if rest.starts_with('=') && !rest.starts_with("==") {
            (UpdateOp::Assign, 1)
        } else {
            return None;
        };
        self.index += len;
        Some(op)
    }

    fn sees_update(&mut self) -> bool {
        let saved = self.index;
        let found = self.take_update().is_some();
        self.index = saved;
        found
    }

    fn sees(&mut self, expected: char) -> bool {
        self.skip_ws();
        self.peek() == Some(expected)
    }

    fn cmp_op_pending(&mut self) -> bool {
        self.skip_ws();
        cmp_at(&self.src[self.index..]).is_some()
    }

    /// `*` `/` `%`, but not `//`.
    fn mul_op(&mut self) -> Option<Op> {
        self.skip_ws();
        let rest = &self.src[self.index..];
        if rest.starts_with("//") {
            return None;
        }
        // `*=`, `/=`, and `%=` are updates.
        if rest.len() > 1
            && rest.as_bytes()[1] == b'='
            && matches!(rest.as_bytes()[0], b'*' | b'/' | b'%')
        {
            return None;
        }
        let (op, len) = if rest.starts_with('*') {
            (Op::Mul, 1)
        } else if rest.starts_with('/') {
            (Op::Div, 1)
        } else if rest.starts_with('%') {
            (Op::Rem, 1)
        } else {
            return None;
        };
        self.index += len;
        Some(op)
    }

    fn parse_path(&mut self) -> Result<Filter, ProgramError> {
        self.bump();
        if self.peek() == Some('.') {
            self.bump();
            if self.peek() == Some('.') || self.starts_ident() {
                return Err(ProgramError::Syntax);
            }
            return self.finish_path(Some(Filter::Recurse));
        }
        let mut filter = None;
        if self.peek() == Some('"') || self.spaced_quote() {
            let key = self.parse_quoted_key()?;
            filter = Some(at_key(None, key));
        } else if self.starts_ident() {
            filter = Some(Filter::Field(self.ident()?));
        }
        self.finish_path(filter)
    }

    fn finish_path(&mut self, mut filter: Option<Filter>) -> Result<Filter, ProgramError> {
        loop {
            self.skip_ws();
            if self.eat('?') {
                let base = filter.unwrap_or(Filter::Identity);
                filter = Some(optional_last(base));
                continue;
            }
            self.skip_ws();
            if self.peek() == Some('[') {
                filter = Some(self.parse_index(filter)?);
                continue;
            }
            if self.peek() == Some('.') {
                let bytes = self.src.as_bytes();
                if bytes.get(self.index + 1) == Some(&b'.') {
                    return Err(ProgramError::Syntax);
                }
                self.bump();
                if self.peek() == Some('"') || self.spaced_quote() {
                    let key = self.parse_quoted_key()?;
                    filter = Some(at_key(filter, key));
                    continue;
                }
                if self.starts_ident() {
                    let name = self.ident()?;
                    filter = Some(chain(filter, Filter::Field(name)));
                    continue;
                }
                if self.peek() == Some('[') {
                    filter = Some(self.parse_index(filter)?);
                    continue;
                }
                return Err(ProgramError::Syntax);
            }
            break;
        }
        Ok(filter.unwrap_or(Filter::Identity))
    }

    /// A space is allowed between `.` and a quoted name.
    fn spaced_quote(&self) -> bool {
        let mut index = self.index;
        let bytes = self.src.as_bytes();
        while index < bytes.len() && matches!(bytes[index], b' ' | b'\t' | b'\n' | b'\r') {
            index += 1;
        }
        bytes.get(index) == Some(&b'"')
    }

    fn parse_quoted_key(&mut self) -> Result<Filter, ProgramError> {
        if self.peek() != Some('"') {
            self.skip_ws();
        }
        self.parse_string_expr()
    }

    /// `[` has not been consumed. A colon in the body is a slice.
    fn parse_index(&mut self, base: Option<Filter>) -> Result<Filter, ProgramError> {
        if !self.eat('[') {
            return Err(ProgramError::Syntax);
        }
        let token_end = self.index;
        self.skip_ws();
        if self.eat(']') {
            return Ok(chain(base, Filter::Iterate));
        }
        if self.eof() {
            return Err(self.unexpected_eof(token_start(self.src, token_end), token_end));
        }
        if self.bracket_has_slice() {
            let mut segment = self.parse_slice_body()?;
            if let Filter::Slice { base: slot, .. } = &mut segment {
                *slot = base.map(Box::new);
            }
            return Ok(segment);
        }
        let key = self.parse_pipe()?;
        self.skip_ws();
        if !self.eat(']') {
            return Err(ProgramError::Syntax);
        }
        Ok(at_key(base, key))
    }

    fn bracket_has_slice(&self) -> bool {
        let bytes = self.src.as_bytes();
        let mut index = self.index;
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escape = false;
        while index < bytes.len() {
            let byte = bytes[index];
            if in_string {
                if escape {
                    escape = false;
                } else if byte == b'\\' {
                    escape = true;
                } else if byte == b'"' {
                    in_string = false;
                }
                index += 1;
                continue;
            }
            match byte {
                b'"' => in_string = true,
                b'[' | b'(' | b'{' => depth += 1,
                b']' if depth == 0 => return false,
                b']' | b')' | b'}' => depth -= 1,
                b':' if depth == 0 => return true,
                _ => {}
            }
            index += 1;
        }
        false
    }

    /// The opening `[` is already consumed, and the body contains a `:`.
    /// Each side is an expression. A comma yields several bounds. An empty
    /// side is left off. Both sides empty is not a slice.
    fn parse_slice_body(&mut self) -> Result<Filter, ProgramError> {
        self.skip_ws();
        let start = if self.peek() == Some(':') {
            None
        } else {
            let start = self.parse_pipe()?;
            self.skip_ws();
            Some(Box::new(start))
        };
        if !self.eat(':') {
            return Err(ProgramError::Syntax);
        }
        self.skip_ws();
        let end = if self.peek() == Some(']') {
            None
        } else {
            let end = self.parse_pipe()?;
            self.skip_ws();
            Some(Box::new(end))
        };
        if start.is_none() && end.is_none() {
            return Err(ProgramError::Syntax);
        }
        if !self.eat(']') {
            return Err(ProgramError::Syntax);
        }
        Ok(Filter::Slice {
            base: None,
            start,
            end,
            optional: false,
        })
    }

    fn ident(&mut self) -> Result<String, ProgramError> {
        let start = self.index;
        if !self.starts_ident() {
            return Err(ProgramError::Syntax);
        }
        self.bump();
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            self.bump();
        }
        Ok(self.src[start..self.index].to_string())
    }

    fn starts_ident(&self) -> bool {
        matches!(self.peek(), Some(c) if c.is_ascii_alphabetic() || c == '_')
    }

    fn eat(&mut self, expected: char) -> bool {
        self.skip_ws();
        if self.peek() == Some(expected) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn skip_ws(&mut self) {
        loop {
            match self.peek() {
                Some(' ' | '\t' | '\n' | '\r') => {
                    self.bump();
                }
                Some('#') => {
                    self.bump();
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                _ => break,
            }
        }
    }

    fn parse_number(&mut self) -> Result<Filter, ProgramError> {
        let start = self.index;
        let bytes = self.src.as_bytes();
        let mut index = start;
        if bytes.get(index) == Some(&b'.') {
            index += 1;
            let frac = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if index == frac {
                return Err(ProgramError::Syntax);
            }
        } else {
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            if bytes.get(index) == Some(&b'.') {
                index += 1;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
            }
        }
        if index < bytes.len() && (bytes[index] == b'e' || bytes[index] == b'E') {
            let exp = index + 1;
            let mut digit_at = exp;
            if bytes
                .get(digit_at)
                .is_some_and(|b| *b == b'+' || *b == b'-')
            {
                digit_at += 1;
            }
            if bytes.get(digit_at).is_some_and(|b| b.is_ascii_digit()) {
                index = digit_at + 1;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
            }
        }
        // A leading dot with no digits is a path, not a number.
        let token = &self.src[start..index];
        if token == "." || token.is_empty() {
            return Err(ProgramError::Syntax);
        }
        self.index = index;
        let value = number_value(token)?;
        Ok(Filter::Literal(value))
    }

    fn parse_array(&mut self) -> Result<Filter, ProgramError> {
        if !self.eat('[') {
            return Err(ProgramError::Syntax);
        }
        self.skip_ws();
        if self.eat(']') {
            return Ok(Filter::Literal(Value::array(Vec::new())));
        }
        let inner = self.parse_pipe()?;
        // `[1,2` parsed the elements and then the file ended.
        let token_end = self.index;
        self.skip_ws();
        if self.eof() {
            let token_at = token_start(self.src, token_end);
            return Err(self.unexpected_eof(token_at, token_end));
        }
        if !self.eat(']') {
            return Err(ProgramError::Syntax);
        }
        if let Some(items) = const_outputs(&inner) {
            return Ok(Filter::Literal(Value::array(items)));
        }
        Ok(Filter::Collect(Box::new(inner)))
    }

    fn parse_object(&mut self) -> Result<Filter, ProgramError> {
        if !self.eat('{') {
            return Err(ProgramError::Syntax);
        }
        self.skip_ws();
        if self.eat('}') {
            return Ok(Filter::Literal(Value::object(Vec::new())));
        }
        let mut fields = Vec::new();
        loop {
            fields.push(self.parse_field()?);
            self.skip_ws();
            if self.eat(',') {
                self.skip_ws();
                if self.eat('}') {
                    break;
                }
                continue;
            }
            if self.eat('}') {
                break;
            }
            if let Some(err) = self.object_operator() {
                return Err(err);
            }
            return Err(ProgramError::Syntax);
        }
        if let Some(value) = const_object(&fields) {
            return Ok(Filter::Literal(value));
        }
        Ok(Filter::Object(fields))
    }

    fn parse_field(&mut self) -> Result<ObjField, ProgramError> {
        self.skip_ws();
        if self.peek() == Some('(') {
            let pos = self.index;
            self.bump();
            let key = self.parse_pipe()?;
            self.skip_ws();
            if !self.eat(')') || !self.eat(':') {
                return Err(ProgramError::Syntax);
            }
            let value = self.parse_expd()?;
            if let Some(constant) = const_one(&key) {
                if !matches!(constant, Value::String(_)) {
                    return Err(report_at(self.src, pos, &key_message(&constant)));
                }
            }
            return Ok(ObjField::Pair {
                key: Box::new(key),
                value: Box::new(value),
            });
        }
        if self.peek() == Some('"') {
            let key = self.parse_string_expr()?;
            self.skip_ws();
            if self.eat(':') {
                let value = self.parse_expd()?;
                return Ok(ObjField::Pair {
                    key: Box::new(key),
                    value: Box::new(value),
                });
            }
            return Ok(ObjField::Lookup(Box::new(key)));
        }
        if self.peek() == Some('$') {
            return self.parse_dollar_field();
        }
        if self.starts_ident() {
            let name = self.ident_key()?;
            self.skip_ws();
            if self.eat(':') {
                let value = self.parse_expd()?;
                return Ok(ObjField::Pair {
                    key: Box::new(Filter::Literal(Value::String(crate::value::Text::from(
                        name,
                    )))),
                    value: Box::new(value),
                });
            }
            return Ok(ObjField::Pair {
                key: Box::new(Filter::Literal(Value::String(crate::value::Text::from(
                    name.clone(),
                )))),
                value: Box::new(Filter::Field(name)),
            });
        }
        Err(ProgramError::Syntax)
    }

    /// `$name` or `$__loc__`. `at` is the `$`.
    fn parse_dollar(&mut self) -> Result<Filter, ProgramError> {
        let at = self.index;
        self.bump();
        let name = self.ident()?;
        if self.followed_by_colon_name() {
            self.index += 2;
            let rest = self.ident()?;
            if rest == name && self.data_names.iter().any(|have| have == &name) {
                return Ok(Filter::Var { name, at });
            }
            return Err(report_at(
                self.src,
                at,
                &format!("${name}::{rest} is not defined"),
            ));
        }
        // `$__loc__` is the source of that `$`, not a binding. `-f` keeps
        // the file name `<top-level>`.
        if name == "__loc__" {
            return Ok(Filter::Here(line_at(self.src, at).0));
        }
        Ok(Filter::Var { name, at })
    }

    /// `{$name}` copies the binding under that name. `{$name: value}` uses
    /// the binding's value as the key. `{$__loc__: ...}` is not that form.
    fn parse_dollar_field(&mut self) -> Result<ObjField, ProgramError> {
        let value = self.parse_dollar()?;
        self.skip_ws();
        let colon =
            self.src[self.index..].starts_with(':') && !self.src[self.index..].starts_with("::");
        if colon {
            self.bump();
            if matches!(value, Filter::Here(_)) {
                return Err(ProgramError::Syntax);
            }
            let expr = self.parse_expd()?;
            return Ok(ObjField::Pair {
                key: Box::new(value),
                value: Box::new(expr),
            });
        }
        let name = match &value {
            Filter::Here(_) => "__loc__".to_string(),
            Filter::Var { name, .. } => name.clone(),
            _ => return Err(ProgramError::Syntax),
        };
        Ok(ObjField::Pair {
            key: Box::new(Filter::Literal(Value::String(crate::value::Text::from(
                name,
            )))),
            value: Box::new(value),
        })
    }

    /// `foo::bar` is one key.
    fn ident_key(&mut self) -> Result<String, ProgramError> {
        let mut name = self.ident()?;
        while self.src[self.index..].starts_with("::") {
            let after = self.index + 2;
            if after >= self.src.len() {
                return Err(ProgramError::Syntax);
            }
            let saved = self.index;
            self.index = after;
            if !self.starts_ident() {
                self.index = saved;
                break;
            }
            name.push_str("::");
            name.push_str(&self.ident()?);
        }
        Ok(name)
    }

    /// `@name` or `@name "text \(expr)"`. The name may start with a digit.
    /// A space before the name is a syntax error. Space before the string is not.
    fn parse_format(&mut self) -> Result<Filter, ProgramError> {
        self.bump();
        let name = self.format_name()?;
        self.skip_ws();
        if self.peek() == Some('"') {
            let pieces = self.parse_string_pieces()?;
            return Ok(Filter::FormatString { name, pieces });
        }
        Ok(Filter::Format(name))
    }

    fn format_name(&mut self) -> Result<String, ProgramError> {
        let start = self.index;
        let Some(ch) = self.peek() else {
            return Err(ProgramError::Syntax);
        };
        if !ch.is_ascii_alphanumeric() && ch != '_' {
            return Err(ProgramError::Syntax);
        }
        self.bump();
        while self
            .peek()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            self.bump();
        }
        Ok(self.src[start..self.index].to_string())
    }

    fn parse_string_expr(&mut self) -> Result<Filter, ProgramError> {
        let pieces = self.parse_string_pieces()?;
        if pieces.iter().all(|piece| matches!(piece, Piece::Text(_))) {
            let mut combined = String::new();
            for piece in pieces {
                if let Piece::Text(part) = piece {
                    combined.push_str(&part);
                }
            }
            return Ok(Filter::Literal(Value::String(crate::value::Text::from(
                combined,
            ))));
        }
        Ok(Filter::Interpolate(pieces))
    }

    fn parse_string_pieces(&mut self) -> Result<Vec<Piece>, ProgramError> {
        if !self.eat('"') {
            return Err(ProgramError::Syntax);
        }
        let mut pieces = Vec::new();
        let mut text = String::new();
        loop {
            let Some(ch) = self.peek() else {
                return Err(ProgramError::Syntax);
            };
            if ch == '"' {
                self.bump();
                break;
            }
            if ch == '\\' {
                let slash = self.index;
                let next = self.src[self.index + 1..].chars().next();
                if next == Some('(') {
                    if !text.is_empty() {
                        pieces.push(Piece::Text(std::mem::take(&mut text)));
                    }
                    self.bump();
                    self.bump();
                    let saved_try = self.reject_try;
                    self.reject_try = false;
                    self.interp += 1;
                    let inner = self.parse_pipe();
                    self.interp -= 1;
                    self.reject_try = saved_try;
                    let inner = inner?;
                    if !self.eat(')') {
                        return Err(ProgramError::Syntax);
                    }
                    pieces.push(Piece::Interp(Box::new(inner)));
                    continue;
                }
                if !self.take_escape_run() {
                    return Err(ProgramError::Syntax);
                }
                let run = &self.src[slash..self.index];
                let wrapped = format!("\"{run}\"");
                match decode_string_token(&wrapped) {
                    Ok(decoded) => text.push_str(&decoded),
                    Err(message) => return Err(report_at(self.src, slash, &message)),
                }
                continue;
            }
            self.bump();
            text.push(ch);
        }
        if !text.is_empty() {
            pieces.push(Piece::Text(text));
        }
        Ok(pieces)
    }

    /// One or more jq escape chunks, starting at a backslash.
    fn take_escape_run(&mut self) -> bool {
        let bytes = self.src.as_bytes();
        let mut index = self.index;
        let mut any = false;
        while index < bytes.len() && bytes[index] == b'\\' {
            if index + 1 >= bytes.len() {
                break;
            }
            let next = bytes[index + 1];
            if next == b'(' {
                break;
            }
            if next == b'u' {
                let mut cursor = index + 2;
                let mut count = 0;
                while count < 4 && cursor < bytes.len() && bytes[cursor].is_ascii_alphanumeric() {
                    cursor += 1;
                    count += 1;
                }
                index = cursor;
                any = true;
                continue;
            }
            index += 2;
            any = true;
        }
        if !any {
            return false;
        }
        self.index = index;
        true
    }

    fn peek(&self) -> Option<char> {
        self.src[self.index..].chars().next()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.src[self.index..].chars().nth(offset)
    }

    fn bump(&mut self) {
        if let Some(c) = self.peek() {
            self.index += c.len_utf8();
        }
    }

    fn eof(&self) -> bool {
        self.index >= self.src.len()
    }
}

pub(super) fn chain(left: Option<Filter>, right: Filter) -> Filter {
    match left {
        Some(left) => Filter::Pipe(Box::new(left), Box::new(right)),
        None => right,
    }
}

pub(super) fn optional_last(filter: Filter) -> Filter {
    match filter {
        Filter::Pipe(left, right) => Filter::Pipe(left, Box::new(question(*right))),
        other => question(other),
    }
}

/// `?` written on a bracket skips a failed index. It does not catch an error
/// raised while the key or a slice bound is computed. A second `?`, or `?`
/// after parentheses, does catch that error.
fn question(filter: Filter) -> Filter {
    match filter {
        Filter::Slice {
            optional: false,
            base,
            start,
            end,
        } => Filter::Slice {
            base,
            start,
            end,
            optional: true,
        },
        Filter::At {
            optional: false,
            base,
            key,
        } => Filter::At {
            base,
            key,
            optional: true,
        },
        // `.a?` and `.["a"]?` skip a failed field. `( .a )?` is the
        // general form and still catches an invalid path.
        Filter::Field(name) => Filter::At {
            base: Box::new(Filter::Identity),
            key: Box::new(Filter::Literal(Value::String(crate::value::Text::from(
                name,
            )))),
            optional: true,
        },
        other => Filter::Optional(Box::new(other)),
    }
}

pub(super) fn cmp_at(rest: &str) -> Option<(Op, usize)> {
    if rest.starts_with("==") {
        Some((Op::Eq, 2))
    } else if rest.starts_with("!=") {
        Some((Op::Ne, 2))
    } else if rest.starts_with("<=") {
        Some((Op::Le, 2))
    } else if rest.starts_with(">=") {
        Some((Op::Ge, 2))
    } else if rest.starts_with('<') {
        Some((Op::Lt, 1))
    } else if rest.starts_with('>') {
        Some((Op::Gt, 1))
    } else {
        None
    }
}

pub(super) fn at_key(base: Option<Filter>, key: Filter) -> Filter {
    if let Filter::Literal(Value::String(name)) = &key {
        return chain(base, Filter::Field(name.to_string()));
    }
    Filter::At {
        base: Box::new(base.unwrap_or(Filter::Identity)),
        key: Box::new(key),
        optional: false,
    }
}

/// The number text jq prints for a literal. Slice bounds stay raw in the
/// step so the slice math is unchanged, and `path` asks for this spelling.
pub(super) fn canonical_number(literal: &str) -> String {
    let literal = literal.strip_prefix('\u{1}').unwrap_or(literal);
    match crate::parse_stream(literal.as_bytes()) {
        Ok(parsed) if parsed.error.is_none() => match parsed.values.as_slice() {
            [one] => match &one.value {
                Value::Number(text) => return text.to_string(),
                _ => {}
            },
            _ => {}
        },
        _ => {}
    }
    literal.to_string()
}

pub(super) fn number_value(token: &str) -> Result<Value, ProgramError> {
    let parsed = crate::parse_stream(token.as_bytes()).map_err(|_| ProgramError::Syntax)?;
    if parsed.error.is_some() {
        return Err(ProgramError::Syntax);
    }
    match parsed.values.as_slice() {
        [one] => Ok(one.value.clone()),
        _ => Err(ProgramError::Syntax),
    }
}

pub(super) fn decode_string_token(text: &str) -> Result<String, String> {
    crate::parse::decode_string_token(text)
}

/// Spaces, tabs, and newlines. A comment is not whitespace here.
fn only_ws(text: &str) -> bool {
    text.bytes()
        .all(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
}

/// The start of the token that ends at `token_end`.
fn token_start(src: &str, token_end: usize) -> usize {
    let bytes = src.as_bytes();
    if token_end == 0 || token_end > bytes.len() {
        return 0;
    }
    let mut index = token_end - 1;
    if bytes[index].is_ascii_digit() {
        while index > 0 && (bytes[index - 1].is_ascii_digit() || bytes[index - 1] == b'.') {
            index -= 1;
        }
        return index;
    }
    if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
        while index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_') {
            index -= 1;
        }
        return index;
    }
    index
}

impl<'a> Parser<'a> {
    /// The operand after an operator is missing, or the next token cannot
    /// be that operand. `1 *` is the end of the file. `"\(1 +)"` ends the
    /// interpolation. `def f: 1 +; f` hits the semicolon.
    fn after_operator(&mut self, token_at: usize, token_end: usize) -> Option<ProgramError> {
        self.skip_ws();
        if self.eof() {
            return Some(self.unexpected_eof(token_at, token_end));
        }
        if self.interp > 0 && self.peek() == Some(')') {
            return Some(self.unexpected_token(self.index, "unexpected QQSTRING_INTERP_END"));
        }
        if self.peek() == Some(';') {
            return Some(self.unexpected_token(self.index, "unexpected ';'"));
        }
        None
    }

    /// `|` that is not `|=`.
    fn sees_pipe(&mut self) -> bool {
        self.skip_ws();
        self.peek() == Some('|') && self.peek_at(1) != Some('=')
    }

    /// A finished value with something still written after it.
    /// `1 2` is a literal. `.a }` is not a token jq names.
    fn trailing_token(&self) -> ProgramError {
        match self.peek() {
            Some('}' | ']' | ')') => self.unexpected_token(
                self.index,
                "unexpected INVALID_CHARACTER, expecting end of file",
            ),
            Some(ch) if ch.is_ascii_digit() => {
                self.unexpected_token(self.index, "unexpected LITERAL, expecting end of file")
            }
            _ => ProgramError::Syntax,
        }
    }

    /// `{a:1+2}`. The value stopped, and jq expected the object to close.
    fn object_operator(&self) -> Option<ProgramError> {
        let ch = self.peek()?;
        if !matches!(ch, '+' | '-' | '*' | '/' | '%') || self.peek_at(1) == Some('=') {
            return None;
        }
        let detail = match ch {
            '+' => "unexpected '+', expecting '}'",
            '-' => "unexpected '-', expecting '}'",
            '*' => "unexpected '*', expecting '}'",
            '/' => "unexpected '/', expecting '}'",
            '%' => "unexpected '%', expecting '}'",
            _ => return None,
        };
        Some(self.unexpected_token(self.index, detail))
    }

    fn unexpected_token(&self, at: usize, detail: &str) -> ProgramError {
        if place_label() != "<top-level>" {
            return ProgramError::Syntax;
        }
        let (line_no, line, column) = line_at(self.src, at);
        jq_syntax(detail, line_no, &line, column)
    }

    /// `if 1 then 2` with no `end`. The end of the file is one error, and
    /// the open `if` is another.
    fn unterminated_if(&self, token_at: usize, token_end: usize) -> ProgramError {
        if place_label() != "<top-level>" {
            return ProgramError::Syntax;
        }
        let (line_no, line, column) = line_at(self.src, token_at);
        let pad = column + usize::from(token_end < self.src.len());
        let place = place_label();
        let mut text = format!(
            "jq: error: syntax error, unexpected end of file (Unix shell quoting issues?) at {place}, line {line_no}:\n{line}"
        );
        text.extend(std::iter::repeat(' ').take(pad));
        text.push('\n');
        text.push_str(&format!(
            "jq: error: Possibly unterminated 'if' statement at {place}, line {line_no}:\n{line}\n"
        ));
        text.push_str("jq: 2 compile errors\n");
        ProgramError::Report(text)
    }

    /// `1 +` and `[1,2`. The column is the last token. Whitespace after
    /// that token adds one more space, which is what jq prints.
    fn unexpected_eof(&self, token_at: usize, token_end: usize) -> ProgramError {
        if place_label() != "<top-level>" {
            return ProgramError::Syntax;
        }
        let (line_no, line, column) = line_at(self.src, token_at);
        let pad = column + usize::from(token_end < self.src.len());
        jq_syntax("unexpected end of file", line_no, &line, pad)
    }

    /// A `}` with nothing else in the program. jq calls that character
    /// invalid and says it expected the end of the file.
    fn lone_closing_brace(&self) -> Option<ProgramError> {
        if self.peek() != Some('}') {
            return None;
        }
        let after = self.index + 1;
        if !only_ws(&self.src[..self.index]) || !only_ws(&self.src[after..]) {
            return None;
        }
        if place_label() != "<top-level>" {
            return Some(ProgramError::Syntax);
        }
        let (line_no, line, column) = line_at(self.src, self.index);
        Some(jq_syntax(
            "unexpected INVALID_CHARACTER, expecting end of file",
            line_no,
            &line,
            column,
        ))
    }
}

fn jq_syntax(detail: &str, line_no: usize, line: &str, pad: usize) -> ProgramError {
    let mut text = format!(
        "jq: error: syntax error, {detail} (Unix shell quoting issues?) at {}, line {line_no}:\n{line}",
        place_label()
    );
    text.extend(std::iter::repeat(' ').take(pad));
    text.push_str("\njq: 1 compile error\n");
    ProgramError::Report(text)
}

pub(super) fn report_at(source: &str, pos: usize, message: &str) -> ProgramError {
    let (line_no, line, column) = line_at(source, pos);
    let mut text = format!(
        "jq: error: {message} at {}, line {line_no}:\n{line}",
        place_label()
    );
    text.push_str(&" ".repeat(column));
    text.push_str("\njq: 1 compile error\n");
    ProgramError::Report(text)
}

pub(super) fn line_at(source: &str, pos: usize) -> (usize, String, usize) {
    let mut line_no = 1usize;
    let mut start = 0usize;
    for (index, byte) in source.bytes().enumerate() {
        if index == pos {
            break;
        }
        if byte == b'\n' {
            line_no += 1;
            start = index + 1;
        }
    }
    let end = source[start..]
        .bytes()
        .position(|byte| byte == b'\n')
        .map(|offset| start + offset)
        .unwrap_or(source.len());
    (
        line_no,
        source[start..end].to_string(),
        pos.saturating_sub(start),
    )
}

pub(super) fn key_message(value: &Value) -> String {
    format!(
        "Cannot use {} ({}) as object key",
        value.type_name(),
        shown(value)
    )
}

pub(super) fn const_one(filter: &Filter) -> Option<Value> {
    match const_outputs(filter)?.as_slice() {
        [one] => Some(one.clone()),
        _ => None,
    }
}

pub(super) fn const_outputs(filter: &Filter) -> Option<Vec<Value>> {
    match filter {
        Filter::Literal(value) => Some(vec![value.clone()]),
        Filter::Here(line) => Some(vec![crate::feed::here(*line)]),
        Filter::Pipe(left, right) if is_identity(right) => const_outputs(left),
        Filter::Pipe(left, right) if is_identity(left) => const_outputs(right),
        Filter::Comma(left, right) => {
            let mut out = const_outputs(left)?;
            out.extend(const_outputs(right)?);
            Some(out)
        }
        Filter::Collect(inner) => {
            let items = const_outputs(inner)?;
            Some(vec![Value::array(items)])
        }
        Filter::Object(fields) => const_object(fields).map(|value| vec![value]),
        Filter::Optional(inner) => const_outputs(inner),
        Filter::Bin { op, left, right } => {
            let lefts = const_outputs(left)?;
            let rights = const_outputs(right)?;
            let mut out = Vec::new();
            for right in &rights {
                for left in &lefts {
                    out.push(apply_op(*op, left, right).ok()?);
                }
            }
            Some(out)
        }
        _ => None,
    }
}

pub(super) fn is_identity(filter: &Filter) -> bool {
    matches!(filter, Filter::Identity)
}

/// `not(1)`, `length(1)`, and a second call such as `map(f)(1)`.
pub(super) fn forbids_call(filter: &Filter) -> bool {
    match filter {
        Filter::Not
        | Filter::Empty
        | Filter::Length
        | Filter::Keys
        | Filter::KeysUnsorted
        | Filter::FromJson
        | Filter::Has(_)
        | Filter::Select(_)
        | Filter::Map(_)
        | Filter::MapValues(_)
        | Filter::Split(_)
        | Filter::Contains(_)
        | Filter::Inside(_)
        | Filter::Flatten(_)
        | Filter::Quant { .. }
        | Filter::IsEmpty(_)
        | Filter::In { .. }
        | Filter::IndexBy { .. }
        | Filter::Type
        | Filter::ToNumber
        | Filter::ToString
        | Filter::ToJson
        | Filter::Format(_)
        | Filter::FormatString { .. }
        | Filter::AsciiDown
        | Filter::AsciiUp
        | Filter::Explode
        | Filter::Implode
        | Filter::Utf8ByteLength
        | Filter::Add
        | Filter::ToEntries
        | Filter::FromEntries
        | Filter::WithEntries(_)
        | Filter::Walk(_)
        | Filter::RecurseDown { .. }
        | Filter::Join(_)
        | Filter::LTrim(_)
        | Filter::RTrim(_)
        | Filter::StartsWith(_)
        | Filter::EndsWith(_)
        | Filter::IndexOf(_)
        | Filter::RIndexOf(_)
        | Filter::IndicesOf(_)
        | Filter::ToDate
        | Filter::FromDate
        | Filter::GmTime
        | Filter::LocalTime
        | Filter::MkTime
        | Filter::StrfTime(_)
        | Filter::StrfLocalTime(_)
        | Filter::StrpTime(_)
        | Filter::Now
        | Filter::Math1(_)
        | Filter::Math2(_, _, _)
        | Filter::Math3(_, _, _)
        | Filter::Abs
        | Filter::Pow10
        | Filter::Error
        | Filter::Raise(_)
        | Filter::Try { .. }
        | Filter::PathOf(_)
        | Filter::Paths(_)
        | Filter::GetPath(_)
        | Filter::SetPath(_, _)
        | Filter::DelPaths(_)
        | Filter::Limit(_, _)
        | Filter::First(_)
        | Filter::Last(_)
        | Filter::Nth { .. }
        | Filter::Range { .. }
        | Filter::Until(_, _)
        | Filter::While(_, _)
        | Filter::Repeat(_)
        | Filter::Order { .. }
        | Filter::Define { .. }
        | Filter::Call { .. }
        | Filter::Param(_)
        | Filter::Unresolved { .. }
        | Filter::MatchImpl { .. }
        | Filter::StreamBind { .. } => true,
        Filter::Optional(inner) => forbids_call(inner),
        Filter::Pipe(left, right) => regex_shape(left) || call_blocked(right),
        _ => false,
    }
}

/// `match`, `test`, and `capture` do not return a function.
fn regex_shape(filter: &Filter) -> bool {
    match filter {
        Filter::MatchImpl { .. } | Filter::StreamBind { .. } => true,
        Filter::Pipe(left, _) => regex_shape(left),
        _ => false,
    }
}

/// A pipe forbids a following call only when its rightmost filter is
/// `error`, `error(expr)`, or `try`. Other pipes stay as they are.
pub(super) fn call_blocked(filter: &Filter) -> bool {
    match filter {
        Filter::Error | Filter::Raise(_) | Filter::Try { .. } => true,
        Filter::Optional(inner) | Filter::Pipe(_, inner) => call_blocked(inner),
        _ => false,
    }
}

pub(super) fn const_object(fields: &[ObjField]) -> Option<Value> {
    let mut pairs = Vec::new();
    for field in fields {
        let ObjField::Pair { key, value } = field else {
            return None;
        };
        let name = const_one(key)?;
        let Ok(name) = name.into_string() else {
            return None;
        };
        let value = const_one(value)?;
        put(&mut pairs, name, value);
    }
    Some(Value::object(pairs))
}
