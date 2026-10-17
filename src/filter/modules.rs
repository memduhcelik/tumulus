//! Module search.
//!
//! The working directory is tried first and is not part of `get_search_list`.
//! `-L` replaces the default list. `~` and `$ORIGIN` are expanded while searching.
//! A path component `..` is rejected before any directory is opened.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::value::Value;

use super::ast::{Failure, Filter, FnCell, Pattern, ProgramError};
use super::eval::fail;

/// One definition exported by a module file.
pub(super) struct Export {
    pub name: String,
    pub arity: usize,
    pub func: Rc<FnCell>,
}

struct Cache {
    loading: Vec<PathBuf>,
    done: HashMap<PathBuf, Rc<Vec<Export>>>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache {
        loading: Vec::new(),
        done: HashMap::new(),
    });
}

pub(super) fn clear_cache() {
    CACHE.with(|slot| {
        let mut cache = slot.borrow_mut();
        cache.loading.clear();
        cache.done.clear();
    });
}

pub(super) enum Hit {
    File(PathBuf),
    Missing { rel: String, note: Option<String> },
    Escape(String),
}

pub(super) fn locate(rel: &str, ext: &str) -> Hit {
    if rel.split('/').any(|part| part == "..") {
        return Hit::Escape(rel.to_string());
    }
    let (dirs, note) = roots();
    for dir in dirs {
        let direct = dir.join(format!("{rel}.{ext}"));
        if direct.is_file() {
            return Hit::File(direct);
        }
        if let Some(base) = rel.rsplit('/').next() {
            if !base.is_empty() {
                let nested = dir.join(rel).join(format!("{base}.{ext}"));
                if nested.is_file() {
                    return Hit::File(nested);
                }
            }
        }
    }
    Hit::Missing {
        rel: rel.to_string(),
        note,
    }
}

pub(super) fn missing_text(rel: &str, note: Option<&str>) -> String {
    match note {
        Some(note) => format!("module not found: {rel} ({note})"),
        None => format!("module not found: {rel}"),
    }
}

pub(super) fn escape_text(rel: &str) -> String {
    format!("Relative paths to modules may not traverse to parent directories ({rel})")
}

/// A linker failure. jq prints a blank line before the compile-error count.
pub(super) fn linker(message: &str) -> ProgramError {
    ProgramError::Report(format!("jq: error: {message}\n\njq: 1 compile error\n"))
}

pub(super) fn load_code(rel: &str) -> Result<Rc<Vec<Export>>, ProgramError> {
    let path = match locate(rel, "jq") {
        Hit::File(path) => path,
        Hit::Missing { rel, note } => {
            return Err(linker(&missing_text(&rel, note.as_deref())));
        }
        Hit::Escape(rel) => return Err(linker(&escape_text(&rel))),
    };
    let key = fs::canonicalize(&path).unwrap_or(path.clone());
    if let Some(found) = cached(&key) {
        return Ok(found);
    }
    if loading(&key) {
        return Err(linker(&format!("circular import: {rel}")));
    }
    let _guard = LoadGuard::push(key.clone());
    let bytes = fs::read(&path).map_err(|_| linker(&missing_text(rel, None)))?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let place = key.to_string_lossy().into_owned();
    let exports = super::parse::compile_module_source(&text, &place)?;
    let loaded = Rc::new(exports);
    store(key, Rc::clone(&loaded));
    Ok(loaded)
}

pub(super) fn load_data(rel: &str) -> Result<Value, ProgramError> {
    let path = match locate(rel, "json") {
        Hit::File(path) => path,
        Hit::Missing { rel, note } => {
            return Err(linker(&missing_text(&rel, note.as_deref())));
        }
        Hit::Escape(rel) => return Err(linker(&escape_text(&rel))),
    };
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => return Err(linker(&missing_text(rel, None))),
    };
    let parsed = match crate::parse_stream(&bytes) {
        Ok(parsed) => parsed,
        Err(err) => {
            return Err(ProgramError::Report(format!(
                "jq: error loading data file {}: {err}\n\njq: 1 compile error\n",
                path.display()
            )));
        }
    };
    if let Some(message) = parsed.error {
        return Err(ProgramError::Report(format!(
            "jq: error loading data file {}: {message}\n\njq: 1 compile error\n",
            path.display()
        )));
    }
    Ok(Value::array(
        parsed.values.into_iter().map(|item| item.value).collect(),
    ))
}

pub(super) fn search_list_value() -> Value {
    Value::array(
        crate::feed::library_paths()
            .into_iter()
            .map(Value::String)
            .collect(),
    )
}

pub(super) fn eval_module_meta(input: &Value) -> Result<Vec<Value>, Failure> {
    let Value::String(name) = input else {
        return Err(fail(
            "modulemeta input module name must be a string".to_string(),
        ));
    };
    let path = match locate(name, "jq") {
        Hit::File(path) => path,
        Hit::Missing { rel, note } => return Err(fail(missing_text(&rel, note.as_deref()))),
        Hit::Escape(rel) => return Err(fail(escape_text(&rel))),
    };
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => return Err(fail(missing_text(name, None))),
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let place = fs::canonicalize(&path)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned());
    match super::parse::describe_module(&text, &place) {
        Ok(value) => Ok(vec![value]),
        Err(ProgramError::Syntax) => {
            eprintln!("tumulus: syntax error");
            Ok(vec![Value::Null])
        }
        Err(ProgramError::Report(text)) => {
            eprint!("{text}");
            Ok(vec![Value::Null])
        }
    }
}

/// Bind each data import around `body`, first import outermost.
pub(super) fn wrap_data(mut body: Filter, binds: &[(String, Value)]) -> Filter {
    for (name, value) in binds.iter().rev() {
        body = Filter::Bind {
            expr: Box::new(Filter::Literal(value.clone())),
            pattern: Pattern::Var(name.clone()),
            body: Box::new(body),
        };
    }
    body
}

fn roots() -> (Vec<PathBuf>, Option<String>) {
    let mut dirs = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd);
    }
    let mut note = None;
    for entry in crate::feed::library_paths() {
        match expand(&entry) {
            Ok(dir) => dirs.push(dir),
            Err(text) => {
                if note.is_none() {
                    note = Some(text);
                }
            }
        }
    }
    (dirs, note)
}

fn expand(entry: &str) -> Result<PathBuf, String> {
    if entry == "~" || entry.starts_with("~/") {
        let home = std::env::var("HOME")
            .map_err(|_| format!("Could not expand {entry}. (Could not find home directory.)"))?;
        let rest = entry.trim_start_matches('~').trim_start_matches('/');
        return Ok(if rest.is_empty() {
            PathBuf::from(home)
        } else {
            PathBuf::from(home).join(rest)
        });
    }
    let origin = crate::feed::jq_origin();
    Ok(PathBuf::from(entry.replace("$ORIGIN", &origin)))
}

fn cached(path: &Path) -> Option<Rc<Vec<Export>>> {
    CACHE.with(|slot| slot.borrow().done.get(path).cloned())
}

fn loading(path: &Path) -> bool {
    CACHE.with(|slot| slot.borrow().loading.iter().any(|item| item == path))
}

fn store(path: PathBuf, loaded: Rc<Vec<Export>>) {
    CACHE.with(|slot| {
        slot.borrow_mut().done.insert(path, loaded);
    });
}

struct LoadGuard(PathBuf);

impl LoadGuard {
    fn push(path: PathBuf) -> Self {
        CACHE.with(|slot| slot.borrow_mut().loading.push(path.clone()));
        LoadGuard(path)
    }
}

impl Drop for LoadGuard {
    fn drop(&mut self) {
        CACHE.with(|slot| {
            let loading = &mut slot.borrow_mut().loading;
            if let Some(index) = loading.iter().rposition(|path| path == &self.0) {
                loading.remove(index);
            }
        });
    }
}
