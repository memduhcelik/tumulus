//! Run the same inputs through tumulus and jq.
//! jq has to be installed. A missing jq fails this check.
//! Nothing here stands in for jq's output.

mod builtins;
mod dates;
mod deep;
mod defs;
mod filters;
mod generators;
mod input;
mod math;
mod modules;
mod options;
mod ordering;
mod paths;
mod regex;
mod stream;
mod text;
mod variables;

pub(crate) use std::fs;
pub(crate) use std::io::{Read, Write};
pub(crate) use std::os::unix::process::ExitStatusExt;
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::process::{Command, Stdio};

pub(crate) fn require_jq() {
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

pub(crate) fn run(program: &str, args: &[String], input: &[u8]) -> std::process::Output {
    // No stdin pipe when the program reads nothing. A pipe per case is
    // most of the time spent comparing with jq.
    if input.is_empty() {
        return Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap_or_else(|err| panic!("could not start {program}: {err}"));
    }
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

/// A child that exits before reading closes the pipe. The check still
/// compares that child's output.
fn write_stdin(stdin: &mut impl Write, program: &str, input: &[u8]) {
    match stdin.write_all(input) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {}
        Err(err) => panic!("write stdin to {program}: {err}"),
    }
}

pub(crate) fn compare(name: &str, args: &[&str], input: &[u8]) {
    let expected = compare_exact(name, args, input);
    assert!(
        expected.status.success(),
        "{name}: jq failed with status {}\n{}",
        expected.status,
        String::from_utf8_lossy(&expected.stderr)
    );
}

pub(crate) fn compare_exact(name: &str, args: &[&str], input: &[u8]) -> std::process::Output {
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    // The program name is argv[0], so $ORIGIN matches the real binary.
    // jq still runs as its own process.
    let mut argv = Vec::with_capacity(1 + owned.len());
    argv.push(bin.to_string());
    argv.extend(owned.iter().cloned());
    // jq starts before tumulus runs, so the two overlap. jq is still a
    // separate process for this exact program.
    let mut child = Command::new("jq")
        .args(&owned)
        .stdin(if input.is_empty() {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("could not start jq: {err}"));
    if !input.is_empty() {
        let mut stdin = child.stdin.take().expect("stdin pipe");
        write_stdin(&mut stdin, "jq", input);
    }
    let got = tumulus::execute(&argv, input);
    let expected = child
        .wait_with_output()
        .unwrap_or_else(|err| panic!("wait for jq: {err}"));
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
        Some(got.code),
        expected.status.code(),
        "{name}: exit status differs\nstderr ours:\n{}\nstderr jq:\n{}",
        String::from_utf8_lossy(&got.stderr),
        String::from_utf8_lossy(&expected.stderr)
    );
    expected
}

/// Run `job` for each index, one after another.
///
/// The tests in this file already run side by side. Starting more jq
/// processes inside one test makes the whole run slower. Every index still
/// runs jq itself.
fn each(count: usize, job: impl Fn(usize)) {
    for index in 0..count {
        job(index);
    }
}

pub(crate) fn fixture(name: &str) -> Vec<u8> {
    fs::read(fixture_path(name)).unwrap_or_else(|err| panic!("read fixture {name}: {err}"))
}

pub(crate) fn fixture_path(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .to_str()
        .expect("fixture path is utf-8")
        .to_string()
}

pub(crate) struct DirGuard(std::path::PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn scratch(label: &str) -> (std::path::PathBuf, DirGuard) {
    let dir = std::env::temp_dir().join(format!(
        "tumulus-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(&dir).expect("temp dir");
    let guard = DirGuard(dir.clone());
    (dir, guard)
}

pub(crate) fn write_bytes(dir: &std::path::Path, name: &str, bytes: &[u8]) -> String {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap_or_else(|err| panic!("write {name}: {err}"));
    path.to_str().expect("utf-8 path").to_string()
}

pub(crate) fn assert_rejects(name: &str, args: &[&str], input: &[u8], stderr: &[u8], code: i32) {
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let got = run(bin, &owned, input);
    assert!(
        got.stdout.is_empty(),
        "{name}: stdout {}",
        String::from_utf8_lossy(&got.stdout)
    );
    assert_eq!(
        got.stderr,
        stderr,
        "{name}: stderr {}",
        String::from_utf8_lossy(&got.stderr)
    );
    assert_eq!(got.status.code(), Some(code), "{name}");
}
