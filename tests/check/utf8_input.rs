//! A bad UTF-8 byte inside a JSON string.
//!
//! jq replaces that sequence with U+FFFD and keeps going. This asks jq.
//! A missing jq fails the check.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

fn require_jq() {
    match Command::new("jq").arg("--version").output() {
        Ok(output) if output.status.success() => {}
        Ok(output) => panic!(
            "jq --version failed with status {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(err) => panic!("jq is required for parity checks and was not runnable: {err}"),
    }
}

/// A child that exits before reading closes the pipe. The check still
/// compares that child's output.
fn write_stdin(stdin: &mut impl Write, program: &str, input: &[u8]) {
    match stdin.write_all(input) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {}
        Err(err) => panic!("write stdin to {program}: {err}"),
    }
}

fn run(program: &str, args: &[String], input: &[u8]) -> std::process::Output {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("could not start {program}: {err}"));
    {
        let mut stdin = child.stdin.take().expect("stdin pipe");
        write_stdin(&mut stdin, program, input);
    }
    child
        .wait_with_output()
        .unwrap_or_else(|err| panic!("wait for {program}: {err}"))
}

fn matches_jq(name: &str, args: &[&str], input: &[u8]) {
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let got = run(bin, &owned, input);
    let expected = run("jq", &owned, input);
    assert_eq!(
        got.stdout,
        expected.stdout,
        "{name}: stdout differs\nargs: {args:?}\nstderr ours:\n{}\nstderr jq:\n{}",
        String::from_utf8_lossy(&got.stderr),
        String::from_utf8_lossy(&expected.stderr)
    );
    assert_eq!(
        got.stderr, expected.stderr,
        "{name}: stderr differs\nargs: {args:?}"
    );
    assert_eq!(
        got.status.code(),
        expected.status.code(),
        "{name}: exit status differs\nstderr ours:\n{}\nstderr jq:\n{}",
        String::from_utf8_lossy(&got.stderr),
        String::from_utf8_lossy(&expected.stderr)
    );
}

struct TempJson {
    path: PathBuf,
}

impl TempJson {
    fn new(bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!("tumulus-utf8-{}.json", std::process::id()));
        fs::write(&path, bytes).unwrap_or_else(|err| panic!("write temp json: {err}"));
        Self { path }
    }
}

impl Drop for TempJson {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[test]
fn invalid_utf8_in_a_string_matches_jq() {
    require_jq();
    // A whole surrogate, overlong, or out-of-range sequence is one U+FFFD.
    // An illegal lead such as C0 or F5 is one U+FFFD per byte. Columns count
    // the original bytes, so a later error does not move by U+FFFD's width.
    let cases: &[(&str, &[&str], &[u8])] = &[
        ("bad byte", &["-c", "."], b"\"a\xffb\"\n"),
        ("bad byte pretty", &["."], b"\"a\xffb\"\n"),
        ("bad byte ascii", &["-a", "."], b"\"a\xffb\"\n"),
        ("bad byte raw", &["-r", "."], b"\"a\xffb\"\n"),
        ("in an array", &["-c", "."], b"[\"a\xff\",1]\n"),
        ("object key", &["-c", "."], b"{\"a\xff\":1}\n"),
        ("object key pretty", &["."], b"{\"a\xff\":1}\n"),
        ("nested", &["-c", "."], b"{\"k\":{\"a\":\"b\xff\"}}\n"),
        ("two values", &["-c", "."], b"\"a\xff\"\n1\n"),
        ("after a bom", &["-c", "."], b"\xef\xbb\xbf\"a\xff\"\n"),
        ("surrogate", &["-c", "."], b"\"\xed\xa0\x80\"\n"),
        ("surrogate end", &["-c", "."], b"\"\xed\xbf\xbf\"\n"),
        ("overlong two-byte lead", &["-c", "."], b"\"\xc0\x80\"\n"),
        ("overlong three bytes", &["-c", "."], b"\"\xe0\x80\x80\"\n"),
        (
            "overlong four bytes",
            &["-c", "."],
            b"\"\xf0\x80\x80\x80\"\n",
        ),
        ("past unicode", &["-c", "."], b"\"\xf4\x90\x80\x80\"\n"),
        (
            "illegal four-byte lead",
            &["-c", "."],
            b"\"\xf5\x80\x80\x80\"\n",
        ),
        ("incomplete three bytes", &["-c", "."], b"\"\xe2\x82\"\n"),
        ("incomplete four bytes", &["-c", "."], b"\"\xf0\x9f\x98\"\n"),
        ("short tail before the quote", &["-c", "."], b"\"a\xe0b\"\n"),
        ("short four-byte tail", &["-c", "."], b"\"\xf0bc\"\n"),
        (
            "enough bytes before the quote",
            &["-c", "."],
            b"\"\xf0bcd\"\n",
        ),
        ("lead then ascii", &["-c", "."], b"\"\xc3(\"\n"),
        (
            "valid two bytes",
            &["-c", "."],
            "\"caf\u{e9}\"\n".as_bytes(),
        ),
        (
            "valid four bytes",
            &["-c", "."],
            "\"\u{1f600}\"\n".as_bytes(),
        ),
        (
            "bad byte between valid sequences",
            &["-c", "."],
            b"\"\xc3\xa9\xff\xc3\xa9\"\n",
        ),
        (
            "replacement character inside a string",
            &["-c", "."],
            "\"\u{fffd}\"\n".as_bytes(),
        ),
        (
            "several ill-formed sequences",
            &["-c", "."],
            b"\"\xed\xa0\x80\xe0\x80\x80\xf4\x90\x80\x80\xc0\x80\"\n",
        ),
        ("after a newline escape", &["-c", "."], b"\"\\n\xff\"\n"),
        ("after a hex escape", &["-c", "."], b"\"\\u0041\xff\"\n"),
        ("control after a bad byte", &["-c", "."], b"\"a\xff\x01\"\n"),
        (
            "control after an overlong",
            &["-c", "."],
            b"\"\xe0\x80\x80\x01\"\n",
        ),
        ("unfinished after a bad byte", &["-c", "."], b"\"a\xff"),
        ("raw nul stays a control", &["-c", "."], b"\"a\x00b\"\n"),
        ("bad escape byte", &["-c", "."], b"\"\\\xff\"\n"),
        ("bad escape sequence", &["-c", "."], b"\"\\\xc3\xa9\"\n"),
        ("escape then a short tail", &["-c", "."], b"\"\xe0\\\"\"\n"),
        (
            "short tail stops before an escape",
            &["-c", "."],
            b"\"\xf0b\\\"X\"\n",
        ),
        ("bare bad byte", &["-c", "."], b"\xff\n"),
        ("bad byte between numbers", &["-c", "."], b"1 \xff 2\n"),
        ("two bad bytes outside", &["-c", "."], b"1 \xff\xfe 2\n"),
        ("lead outside", &["-c", "."], b"1 \xc3( 2\n"),
        (
            "four bad bytes outside",
            &["-c", "."],
            b"1 \xf5\x80\x80\x80 2\n",
        ),
        ("surrogate outside", &["-c", "."], b"1 \xed\xa0\x80 2\n"),
        ("overlong outside", &["-c", "."], b"1 \xe0\x80\x80 2\n"),
        (
            "valid text outside",
            &["-c", "."],
            "1 caf\u{e9} 2\n".as_bytes(),
        ),
        (
            "replacement character outside",
            &["-c", "."],
            "1 \u{fffd} 2\n".as_bytes(),
        ),
        ("true glued to a bad byte", &["-c", "."], b"true\xff\n"),
        ("null glued to a bad byte", &["-c", "."], b"null\xff\n"),
        ("nan glued to a bad byte", &["-c", "."], b"nan\xff\n"),
    ];
    let mut jobs: Vec<(String, Vec<String>, Vec<u8>)> = cases
        .iter()
        .map(|(name, args, input)| {
            (
                (*name).to_string(),
                args.iter().map(|arg| (*arg).to_string()).collect(),
                input.to_vec(),
            )
        })
        .collect();
    // 0x00 through 0x7F are valid UTF-8. 0x80 through 0xFF cannot be a
    // scalar on their own, so each one is a replacement inside the string.
    for byte in 0x80u8..=0xFF {
        let mut input = b"\"a".to_vec();
        input.push(byte);
        input.extend_from_slice(b"b\"");
        jobs.push((
            format!("string byte {byte:02x}"),
            vec!["-c".to_string(), ".".to_string()],
            input,
        ));
    }
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        / 4;
    let cores = cores.max(1).min(jobs.len());
    let next = AtomicUsize::new(0);
    let next = &next;
    let jobs = &jobs;
    std::thread::scope(|scope| {
        for _ in 0..cores {
            scope.spawn(move || loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                if index >= jobs.len() {
                    break;
                }
                let (name, args, input) = &jobs[index];
                let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
                matches_jq(name, &borrowed, input);
            });
        }
    });
    let file = TempJson::new(b"{\"msg\":\"a\xffb\"}\n");
    let path = file.path.to_str().expect("temp path is utf-8");
    matches_jq("bad byte in a file", &[".", path], b"");
    matches_jq("bad byte in a compact file", &["-c", ".", path], b"");
}
