//! Module search.
//!
//! An import with no `search` key tries the working directory, then each
//! library path. `modulemeta` tries the library path only. The working
//! directory is not part of `get_search_list`. `-L` replaces the default list.
//! `~` and `$ORIGIN` are expanded on that list. Inside `search`, `~/` and
//! `$ORIGIN/` expand the same way. A bare `~`, and `$ORIGIN` not followed by
//! `/`, are ordinary relative names.
//!
//! A `search` value replaces both the working directory and the library path.
//! The string `"."` is the working directory. Any other string is relative to
//! the file that contains the import. A command-line program has no file, so
//! that string is relative to the working directory. `..` in a module path is
//! rejected before a directory is opened. `..` in a search directory is left
//! for the filesystem, which follows a symlink before applying `..`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::eprint;
use crate::eprintln;
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
    /// Directory of the file whose imports are being resolved.
    /// `None` is a command-line program.
    static IMPORT_ORIGIN: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Directory of the file whose imports are being resolved.
/// `None` is a command-line program: `search` then uses the working directory.
pub fn set_import_origin(origin: Option<PathBuf>) {
    IMPORT_ORIGIN.with(|slot| *slot.borrow_mut() = origin);
}

fn import_origin() -> Option<PathBuf> {
    IMPORT_ORIGIN.with(|slot| slot.borrow().clone())
}

/// Restores the caller's file directory when a nested module has finished.
struct OriginGuard {
    previous: Option<PathBuf>,
}

impl OriginGuard {
    fn set(origin: Option<PathBuf>) -> Self {
        let previous =
            IMPORT_ORIGIN.with(|slot| std::mem::replace(&mut *slot.borrow_mut(), origin));
        OriginGuard { previous }
    }
}

impl Drop for OriginGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        IMPORT_ORIGIN.with(|slot| *slot.borrow_mut() = previous);
    }
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

pub(super) fn locate(rel: &str, ext: &str, search: Option<&Value>) -> Hit {
    if let Some(hit) = escape(rel) {
        return hit;
    }
    let (dirs, note) = match search {
        Some(value) => override_dirs(value),
        None => roots(),
    };
    found(rel, ext, &dirs, note)
}

/// `modulemeta` uses the library path and does not look in the working directory.
fn locate_library(rel: &str, ext: &str) -> Hit {
    if let Some(hit) = escape(rel) {
        return hit;
    }
    let (dirs, note) = library_dirs();
    found(rel, ext, &dirs, note)
}

fn escape(rel: &str) -> Option<Hit> {
    if rel.split('/').any(|part| part == "..") {
        Some(Hit::Escape(rel.to_string()))
    } else {
        None
    }
}

fn found(rel: &str, ext: &str, dirs: &[PathBuf], note: Option<String>) -> Hit {
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

pub(super) fn load_code(
    rel: &str,
    search: Option<&Value>,
) -> Result<Rc<Vec<Export>>, ProgramError> {
    let path = match locate(rel, "jq", search) {
        Hit::File(path) => path,
        Hit::Missing { rel, note } => {
            return Err(linker(&missing_text(&rel, note.as_deref())));
        }
        Hit::Escape(rel) => return Err(linker(&escape_text(&rel))),
    };
    let key = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
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
    // Imports inside this file are relative to the path that found it,
    // not to its canonical path. `..` then follows a symlink the way the
    // filesystem does.
    let _origin = OriginGuard::set(parent_dir(&path));
    let exports = super::parse::compile_module_source(&text, &place)?;
    let loaded = Rc::new(exports);
    store(key, Rc::clone(&loaded));
    Ok(loaded)
}

pub(super) fn load_data(rel: &str, search: Option<&Value>) -> Result<Value, ProgramError> {
    let path = match locate(rel, "json", search) {
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
            .map(|path| Value::String(crate::value::Text::from(path)))
            .collect(),
    )
}

pub(super) fn eval_module_meta(input: &Value) -> Result<Vec<Value>, Failure> {
    let Value::String(name) = input else {
        return Err(fail(
            "modulemeta input module name must be a string".to_string(),
        ));
    };
    let path = match locate_library(name, "jq") {
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
            alts: Vec::new(),
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
    let (libs, note) = library_dirs();
    dirs.extend(libs);
    (dirs, note)
}

fn library_dirs() -> (Vec<PathBuf>, Option<String>) {
    let mut dirs = Vec::new();
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

/// One directory per search entry. `"."` is always the working directory.
/// Every other entry is relative to the importing file, or to the working
/// directory when the program has no file. A `~/` or `$ORIGIN/` prefix is
/// expanded first, and a failed `~/` is skipped. The first such failure is
/// reported when no entry names the module.
fn override_dirs(search: &Value) -> (Vec<PathBuf>, Option<String>) {
    let cwd = std::env::current_dir().ok();
    let origin = import_origin();
    let mut dirs = Vec::new();
    let mut note = None;
    for entry in search_entries(search) {
        let entry = match expand_search(&entry) {
            Ok(entry) => entry,
            Err(text) => {
                if note.is_none() {
                    note = Some(text);
                }
                continue;
            }
        };
        if entry == "." {
            if let Some(cwd) = &cwd {
                dirs.push(cwd.clone());
            }
            continue;
        }
        let Some(base) = origin.as_ref().or(cwd.as_ref()) else {
            continue;
        };
        dirs.push(join_entry(base, &entry));
    }
    (dirs, note)
}

/// `~/...` and `$ORIGIN/...` only. A bare `~` or a `$ORIGIN` without `/` stays
/// as written, which is what jq does for a search string.
fn expand_search(entry: &str) -> Result<String, String> {
    if let Some(rest) = entry.strip_prefix("~/") {
        let home = std::env::var("HOME")
            .map_err(|_| format!("Could not expand {entry}. (Could not find home directory.)"))?;
        return Ok(if rest.is_empty() {
            home
        } else {
            PathBuf::from(home)
                .join(rest)
                .to_string_lossy()
                .into_owned()
        });
    }
    if let Some(rest) = entry.strip_prefix("$ORIGIN/") {
        let origin = crate::feed::jq_origin();
        return Ok(if rest.is_empty() {
            origin
        } else {
            PathBuf::from(origin)
                .join(rest)
                .to_string_lossy()
                .into_owned()
        });
    }
    Ok(entry.to_string())
}

fn search_entries(search: &Value) -> Vec<String> {
    match search {
        Value::String(text) => vec![text.to_string()],
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(text) => Some(text.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn join_entry(base: &Path, entry: &str) -> PathBuf {
    if entry.is_empty() {
        base.to_path_buf()
    } else {
        base.join(entry)
    }
}

/// Parent of the path as it was opened. A relative parent is joined to the
/// working directory and is not canonicalized, so a symlink stays in the path
/// and `..` is applied by the filesystem.
fn parent_dir(path: &Path) -> Option<PathBuf> {
    let parent = path.parent().filter(|dir| !dir.as_os_str().is_empty())?;
    if parent.is_absolute() {
        return Some(parent.to_path_buf());
    }
    match std::env::current_dir() {
        Ok(cwd) => Some(cwd.join(parent)),
        Err(_) => Some(parent.to_path_buf()),
    }
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
