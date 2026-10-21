//! Run every program and input from the jq 1.7.1 test file.
//! The expected lines in that file only mark where a case ends.
//! Each case is run through tumulus and through jq.
//! Nothing here stands in for jq's output.
//!
//! A mismatch is printed. It does not fail the test. The other parity
//! checks still fail when tumulus and jq disagree.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Case {
    line: usize,
    program: String,
    /// `None` for a `%%FAIL` compile check, which has no input line.
    input: Option<String>,
}

struct Outcome {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    code: Option<i32>,
}

fn suite_text() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/jq.test");
    fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

fn file_lines(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines
}

fn skipline(line: &str) -> bool {
    let rest = line.trim_start_matches([' ', '\t']);
    rest.is_empty() || rest.starts_with('#')
}

/// Same cases jq's `--run-tests` reads from this file.
fn parse_suite(text: &str) -> Vec<Case> {
    let lines = file_lines(text);
    let mut cases = Vec::new();
    let mut index = 0;
    let mut must_fail = false;
    while index < lines.len() {
        let line = lines[index];
        let lineno = index + 1;
        index += 1;
        if skipline(line) {
            continue;
        }
        if line == "%%FAIL" || line == "%%FAIL IGNORE MSG" {
            must_fail = true;
            continue;
        }
        let program = line.to_string();
        let program_line = lineno;
        if must_fail {
            if index >= lines.len() {
                panic!("compile-fail case on line {program_line} has no message line");
            }
            index += 1;
            cases.push(Case {
                line: program_line,
                program,
                input: None,
            });
            must_fail = false;
            continue;
        }
        if index >= lines.len() {
            panic!("program on line {program_line} has no input line");
        }
        let input = lines[index].to_string();
        index += 1;
        while index < lines.len() && !skipline(lines[index]) {
            index += 1;
        }
        if index < lines.len() && skipline(lines[index]) {
            index += 1;
        }
        cases.push(Case {
            line: program_line,
            program,
            input: Some(input),
        });
    }
    cases
}

fn cases() -> Vec<Case> {
    let cases = parse_suite(&suite_text());
    assert_eq!(cases.len(), 447, "jq 1.7.1 tests/jq.test has 447 programs");
    assert_eq!(
        cases.iter().filter(|case| case.input.is_none()).count(),
        14,
        "jq 1.7.1 tests/jq.test has 14 compile-fail programs"
    );
    assert_eq!(cases[0].line, 8);
    assert_eq!(cases[0].program, "true");
    assert_eq!(cases[0].input.as_deref(), Some("null"));
    let dash = cases
        .iter()
        .find(|case| case.program == "-1")
        .expect("program -1");
    assert_eq!(dash.line, 25);
    assert_eq!(dash.input.as_deref(), Some("null"));
    let fail = cases
        .iter()
        .find(|case| case.program == "{(0):1}")
        .expect("compile-fail program");
    assert_eq!(fail.line, 114);
    assert!(fail.input.is_none());
    let last = cases.last().expect("last program");
    assert_eq!(last.line, 2114);
    assert!(last.program.starts_with("try ltrimstr(\"x\")"));
    assert!(cases.iter().any(|case| {
        case.program == "."
            && case
                .input
                .as_deref()
                .is_some_and(|input| input.starts_with('\u{feff}'))
    }));
    cases
}

#[test]
fn jq_test_file_shape() {
    let _ = cases();
}

#[test]
fn feature_labels() {
    assert_eq!(bucket("map(.+1)"), "function calls");
    assert_eq!(bucket("length"), "builtin filters");
    assert_eq!(bucket("@text"), "format filters");
    assert_eq!(bucket("1+1"), "operators");
    assert_eq!(bucket(".+1"), "operators");
    assert_eq!(bucket("1 == 2"), "operators");
    assert_eq!(bucket("1 and 2"), "operators");
    assert_eq!(bucket("-1"), "literals and constructors");
    assert_eq!(bucket("true"), "literals and constructors");
    assert_eq!(bucket("{a:1}"), "literals and constructors");
    assert_eq!(bucket("{a,b,(.d):.a,e:.b}"), "literals and constructors");
    assert_eq!(bucket("[.foo]"), "literals and constructors");
    assert_eq!(bucket("[.[]|tojson|fromjson]"), "builtin filters");
    assert_eq!(bucket(r#""inter\("pol" + "ation")""#), "operators");
    assert_eq!(
        bucket("{if:0,and:1,or:2,def:8,import:14,module:16}"),
        "literals and constructors"
    );
    assert_eq!(bucket("def f: 1; f"), "function definitions");
    assert_eq!(bucket("import \"a\" as a; a::a"), "modules");
    assert_eq!(bucket("include \"c\"; c"), "modules");
    assert_eq!(bucket(". as $x | $x"), "variables and bindings");
    assert_eq!(bucket("try . catch 1"), "try and catch");
    assert_eq!(bucket("if . then 1 else 2 end"), "conditionals");
    assert_eq!(bucket("reduce .[] as $x (0; . + $x)"), "reduce and foreach");
    assert_eq!(bucket(".a = 1"), "updates");
    assert_eq!(bucket(".a |= ."), "updates");
    assert_eq!(bucket(".a += 1"), "updates");
    assert_eq!(bucket(". // 1"), "alternatives");
}

fn require_jq() -> String {
    let output = Command::new("jq")
        .arg("--version")
        .output()
        .unwrap_or_else(|err| panic!("jq is required and was not runnable: {err}"));
    assert!(
        output.status.success(),
        "jq --version failed with status {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn run_filter(bin: &str, program: &str, input: &[u8], home: &Path) -> Outcome {
    // `--` ends option parsing. The program is the next argument.
    let mut child = Command::new(bin)
        .args(["--", program])
        .env("HOME", home)
        .env_remove("JQ_LIBRARY_PATH")
        .env_remove("JQ_COLORS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("could not start {bin}: {err}"));
    {
        let mut stdin = child.stdin.take().expect("stdin pipe");
        // A program that exits before reading closes this pipe. The
        // comparison is the child's output, so a broken pipe is not a failure.
        match stdin.write_all(input) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {}
            Err(err) => panic!("write stdin to {bin}: {err}"),
        }
    }
    let output = child
        .wait_with_output()
        .unwrap_or_else(|err| panic!("wait for {bin}: {err}"));
    Outcome {
        stdout: output.stdout,
        stderr: output.stderr,
        code: output.status.code(),
    }
}

fn same(ours: &Outcome, jq: &Outcome) -> bool {
    ours.stdout == jq.stdout && ours.stderr == jq.stderr && ours.code == jq.code
}

fn syntax_error(outcome: &Outcome) -> bool {
    outcome.code == Some(3)
        && outcome.stdout.is_empty()
        && outcome.stderr == b"tumulus: syntax error\n"
}

enum Class {
    Match(usize, String),
    Rejected {
        line: usize,
        program: String,
        label: &'static str,
    },
    Value(Diff),
    Status(Diff),
    Both(Diff),
}

fn classify(case: &Case, bin: &str, home: &Path) -> Class {
    let input = case.input.as_deref().unwrap_or("").as_bytes();
    // This binary is the only one running, so the two tools share the wait.
    let (ours, jq) = std::thread::scope(|scope| {
        let ours = scope.spawn(|| run_filter(bin, &case.program, input, home));
        let jq = scope.spawn(|| run_filter("jq", &case.program, input, home));
        (ours.join().unwrap(), jq.join().unwrap())
    });
    if same(&ours, &jq) {
        return Class::Match(case.line, case.program.clone());
    }
    // Exit 3 is jq's compile error. Any other status means jq compiled
    // the program and ran it, even when the value then failed.
    if syntax_error(&ours) && jq.code != Some(3) {
        return Class::Rejected {
            line: case.line,
            program: case.program.clone(),
            label: bucket(&case.program),
        };
    }
    let row = Diff {
        line: case.line,
        program: case.program.clone(),
        ours,
        jq,
    };
    if syntax_error(&row.ours) {
        Class::Both(row)
    } else if row.ours.stdout != row.jq.stdout {
        Class::Value(row)
    } else {
        Class::Status(row)
    }
}

/// Every case still runs jq. The cases run together.
fn classify_all(cases: &[Case], bin: &str, home: &Path) -> Vec<Class> {
    let mut slots = Vec::with_capacity(cases.len());
    for _ in 0..cases.len() {
        slots.push(None);
    }
    let slots = std::sync::Mutex::new(slots);
    // The parity tests run in this same process, so leave most cores to them.
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        / 4;
    let cores = cores.max(1).min(cases.len());
    let next = AtomicUsize::new(0);
    let next = &next;
    let cases = &cases;
    let slots_ref = &slots;
    std::thread::scope(|scope| {
        for _ in 0..cores {
            scope.spawn(move || loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                if index >= cases.len() {
                    break;
                }
                let class = classify(&cases[index], bin, home);
                slots_ref.lock().unwrap()[index] = Some(class);
            });
        }
    });
    slots
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|class| class.expect("case"))
        .collect()
}

#[test]
fn compare_jq_test_file() {
    let version = require_jq();
    let cases = cases();
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let home = empty_home();
    let _cleanup = DirGuard(home.clone());

    let classes = classify_all(&cases, bin, &home);
    let mut matched = Vec::new();
    let mut syntax: BTreeMap<&'static str, Bucket> = BTreeMap::new();
    let mut both_reject = Vec::new();
    let mut values = Vec::new();
    let mut status = Vec::new();
    for class in classes {
        match class {
            Class::Match(line, program) => matched.push((line, program)),
            Class::Rejected {
                line,
                program,
                label,
            } => {
                let entry = syntax.entry(label).or_default();
                if entry.examples.len() < 3 {
                    entry.examples.push((line, program));
                }
                entry.count += 1;
            }
            Class::Value(row) => values.push(row),
            Class::Status(row) => status.push(row),
            Class::Both(row) => both_reject.push(row),
        }
    }

    let differed = cases.len() - matched.len();
    let mut lines = Vec::new();
    lines.push(format!("jq: {version}"));
    lines.push("suite: jq 1.7.1 tests/jq.test".to_string());
    lines.push(format!("{} programs", cases.len()));
    lines.push(format!("{} come out the same", matched.len()));
    lines.push(format!("{differed} differ"));
    lines.push(String::new());
    lines.push(
        "A run comes out the same when stdout, stderr, and the exit status match.".to_string(),
    );
    for (line, program) in &matched {
        lines.push(format!("  line {line}: {}", short(program)));
    }
    lines.push(String::new());
    lines.push(
        "14 programs have no input line. They are compile-fail checks and run on empty stdin."
            .to_string(),
    );
    let syntax_count: usize = syntax.values().map(|bucket| bucket.count).sum();
    lines.push(String::new());
    lines.push(format!(
        "{syntax_count} differ because tumulus rejects the program and jq runs it"
    ));
    lines.push("Each of those programs is counted under one feature.".to_string());
    let mut buckets: Vec<_> = syntax.into_iter().collect();
    buckets.sort_by(|left, right| right.1.count.cmp(&left.1.count).then(left.0.cmp(right.0)));
    for (label, bucket) in &buckets {
        lines.push(format!("  {label}: {}", bucket.count));
        for (line, program) in &bucket.examples {
            lines.push(format!("    line {line}: {}", short(program)));
        }
    }
    push_diffs(&mut lines, "differ in the values they print", &values);
    push_diffs(
        &mut lines,
        "match on stdout and differ in stderr or the exit status",
        &status,
    );
    let (missing_mod, both_reject): (Vec<_>, Vec<_>) = both_reject
        .into_iter()
        .partition(|diff| String::from_utf8_lossy(&diff.jq.stderr).contains("module not found"));
    push_diffs(
        &mut lines,
        "use import or include, and jq reports that the module is not found",
        &missing_mod,
    );
    push_diffs(
        &mut lines,
        "are rejected by both, and the text differs",
        &both_reject,
    );
    println!("{}", lines.join("\n"));
}

#[derive(Default)]
struct Bucket {
    count: usize,
    examples: Vec<(usize, String)>,
}


struct Diff {
    line: usize,
    program: String,
    ours: Outcome,
    jq: Outcome,
}

fn push_diffs(lines: &mut Vec<String>, title: &str, diffs: &[Diff]) {
    lines.push(String::new());
    lines.push(format!("{} {title}", diffs.len()));
    for diff in diffs {
        lines.push(format!("  line {}: {}", diff.line, short(&diff.program)));
        lines.push(format!(
            "    tumulus exit {} stdout {} stderr {}",
            code_label(diff.ours.code),
            preview(&diff.ours.stdout),
            preview(&diff.ours.stderr)
        ));
        lines.push(format!(
            "    jq exit {} stdout {} stderr {}",
            code_label(diff.jq.code),
            preview(&diff.jq.stdout),
            preview(&diff.jq.stderr)
        ));
    }
}

fn code_label(code: Option<i32>) -> String {
    match code {
        Some(code) => code.to_string(),
        None => "none".to_string(),
    }
}

fn short(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(120).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

fn preview(bytes: &[u8]) -> String {
    let flat = String::from_utf8_lossy(bytes).replace('\n', "\\n");
    format!("[{}]", short(&flat))
}

/// One label per program. Earlier checks win.
fn bucket(program: &str) -> &'static str {
    let source = strip_strings(program);
    if has_keyword(&source, "def") {
        return "function definitions";
    }
    if has_keyword(&source, "import")
        || has_keyword(&source, "include")
        || has_keyword(&source, "module")
    {
        return "modules";
    }
    if has_keyword(&source, "reduce") || has_keyword(&source, "foreach") {
        return "reduce and foreach";
    }
    if has_keyword(&source, "try") || has_keyword(&source, "catch") || has_keyword(&source, "label")
    {
        return "try and catch";
    }
    if has_keyword(&source, "if") {
        return "conditionals";
    }
    if source.contains('$') {
        return "variables and bindings";
    }
    if has_assignment(&source) {
        return "updates";
    }
    if source.contains("//") {
        return "alternatives";
    }
    if has_call(&source) {
        return "function calls";
    }
    if has_format(&source) {
        return "format filters";
    }
    if has_operator(&source) {
        return "operators";
    }
    if has_bare_builtin(&source) {
        return "builtin filters";
    }
    "literals and constructors"
}

fn strip_strings(program: &str) -> String {
    let chars: Vec<char> = program.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '"' {
            index = skip_string(&chars, index, &mut out);
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    out
}

/// `index` points at the opening quote. Interpolation code is kept.
/// The string itself becomes `0` so a following operator still has a value on its left.
fn skip_string(chars: &[char], index: usize, out: &mut String) -> usize {
    out.push('0');
    let mut cursor = index + 1;
    while cursor < chars.len() {
        if chars[cursor] == '\\' {
            if chars.get(cursor + 1) == Some(&'(') {
                cursor = copy_interpolation(chars, cursor + 2, out);
                continue;
            }
            cursor += 1;
            if cursor < chars.len() {
                cursor += 1;
            }
            continue;
        }
        if chars[cursor] == '"' {
            return cursor + 1;
        }
        cursor += 1;
    }
    cursor
}

fn copy_interpolation(chars: &[char], mut cursor: usize, out: &mut String) -> usize {
    let mut depth = 1;
    while cursor < chars.len() && depth > 0 {
        if chars[cursor] == '"' {
            cursor = skip_string(chars, cursor, out);
            continue;
        }
        if chars[cursor] == '(' {
            depth += 1;
            out.push('(');
            cursor += 1;
            continue;
        }
        if chars[cursor] == ')' {
            depth -= 1;
            if depth == 0 {
                return cursor + 1;
            }
            out.push(')');
            cursor += 1;
            continue;
        }
        out.push(chars[cursor]);
        cursor += 1;
    }
    cursor
}

fn has_keyword(source: &str, word: &str) -> bool {
    let bytes = source.as_bytes();
    let word = word.as_bytes();
    let mut index = 0;
    while index + word.len() <= bytes.len() {
        if &bytes[index..index + word.len()] == word {
            let prev_ok = index == 0 || !is_ident_cont(bytes[index - 1]);
            let after = index + word.len();
            let next_ok = after >= bytes.len() || !is_ident_cont(bytes[after]);
            if prev_ok && next_ok {
                let mut cursor = after;
                while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                    cursor += 1;
                }
                if cursor >= bytes.len() || bytes[cursor] != b':' {
                    return true;
                }
            }
        }
        index += 1;
    }
    false
}

fn has_assignment(source: &str) -> bool {
    let bytes = source.as_bytes();
    for index in 0..bytes.len() {
        if bytes[index] != b'=' {
            continue;
        }
        let prev = if index > 0 { bytes[index - 1] } else { 0 };
        let next = if index + 1 < bytes.len() {
            bytes[index + 1]
        } else {
            0
        };
        let comparison =
            next == b'=' || prev == b'=' || prev == b'!' || prev == b'<' || prev == b'>';
        if !comparison && prev != 0 {
            return true;
        }
    }
    false
}

fn has_call(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !is_ident_start(bytes[index]) {
            index += 1;
            continue;
        }
        index += 1;
        while index < bytes.len() && is_ident_cont(bytes[index]) {
            index += 1;
        }
        let mut cursor = index;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor < bytes.len() && bytes[cursor] == b'(' {
            return true;
        }
    }
    false
}

fn has_format(source: &str) -> bool {
    source.contains('@')
}

fn has_operator(source: &str) -> bool {
    if has_keyword(source, "and") || has_keyword(source, "or") || has_keyword(source, "not") {
        return true;
    }
    let chars: Vec<char> = source.chars().collect();
    for (index, ch) in chars.iter().copied().enumerate() {
        match ch {
            '+' | '-' => {
                if preceded_by_value(&chars, index) {
                    return true;
                }
            }
            '*' | '%' => return true,
            '/' => {
                let doubled =
                    chars.get(index + 1) == Some(&'/') || (index > 0 && chars[index - 1] == '/');
                if !doubled {
                    return true;
                }
            }
            '=' | '!' | '<' | '>' => return true,
            _ => {}
        }
    }
    false
}

fn preceded_by_value(chars: &[char], index: usize) -> bool {
    let mut cursor = index;
    while cursor > 0 {
        cursor -= 1;
        if chars[cursor] == ' ' || chars[cursor] == '\t' {
            continue;
        }
        return matches!(
            chars[cursor],
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | ')' | ']' | '"' | '$' | '.'
        );
    }
    false
}

fn has_bare_builtin(source: &str) -> bool {
    let chars: Vec<char> = source.chars().collect();
    let mut frames = vec![Frame::Seq];
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if ch == ' ' || ch == '\t' {
            index += 1;
            continue;
        }
        match ch {
            '{' => frames.push(Frame::Key),
            '(' | '[' => frames.push(Frame::Seq),
            '}' | ')' | ']' => {
                frames.pop();
                if frames.is_empty() {
                    frames.push(Frame::Seq);
                }
            }
            ':' => {
                if frames.last() == Some(&Frame::Key) {
                    *frames.last_mut().expect("frame") = Frame::Val;
                }
            }
            ',' => {
                if matches!(frames.last(), Some(Frame::Key | Frame::Val)) {
                    *frames.last_mut().expect("frame") = Frame::Key;
                }
            }
            _ if ident_start(ch) => {
                let start = index;
                index += 1;
                while index < chars.len() && ident_cont(chars[index]) {
                    index += 1;
                }
                let word: String = chars[start..index].iter().collect();
                if previous_significant(&chars, start) == Some('.') {
                    continue;
                }
                let mut cursor = index;
                while cursor < chars.len() && matches!(chars[cursor], ' ' | '\t') {
                    cursor += 1;
                }
                if matches!(chars.get(cursor), Some('(' | ':')) {
                    continue;
                }
                if frames.last() == Some(&Frame::Key) || is_syntax_word(&word) {
                    continue;
                }
                return true;
            }
            _ => {}
        }
        index += 1;
    }
    false
}

#[derive(Clone, Copy, PartialEq)]
enum Frame {
    Seq,
    Key,
    Val,
}

fn ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn ident_cont(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn previous_significant(chars: &[char], index: usize) -> Option<char> {
    let mut cursor = index;
    while cursor > 0 {
        cursor -= 1;
        if chars[cursor] != ' ' && chars[cursor] != '\t' {
            return Some(chars[cursor]);
        }
    }
    None
}

fn is_syntax_word(word: &str) -> bool {
    matches!(
        word,
        "true"
            | "false"
            | "null"
            | "and"
            | "or"
            | "not"
            | "def"
            | "import"
            | "include"
            | "module"
            | "try"
            | "catch"
            | "label"
            | "if"
            | "then"
            | "else"
            | "elif"
            | "end"
            | "reduce"
            | "foreach"
            | "as"
            | "break"
    )
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident_cont(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn empty_home() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "tumulus-jq-suite-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(&path).expect("temp home");
    path
}

struct DirGuard(PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
