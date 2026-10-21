//! Streaming input compared with jq.

use super::*;

#[test]
fn streaming_match_jq() {
    require_jq();

    let programs = [
        ("stream null", "null | tostream"),
        ("stream array", "[1,2] | tostream"),
        ("stream nested", "[[1,2],[3]] | tostream"),
        ("stream object", r#"{"b":1,"a":[2,{}]} | tostream"#),
        ("stream empty array", "[] | tostream"),
        ("stream empty object", "{} | tostream"),
        ("stream nested empty", "[[]] | tostream"),
        ("stream deeper", "[1,[2,[]],3] | tostream"),
        ("stream object nest", r#"{"a":{"b":{}}} | tostream"#),
        ("stream nan", "nan | tostream"),
        ("stream neg zero", "-0 | tostream"),
        ("stream decimal", "1.50 | tostream"),
        ("stream empty", "empty | tostream"),
        (
            "stream round trip",
            "[fromstream(1 | tostream), fromstream([1,2] | tostream), fromstream([[1,2],[3]] | tostream), fromstream({} | tostream), fromstream([] | tostream)]",
        ),
        ("stream rebuild", "fromstream([[0],1], [[1],2], [[1]])"),
        ("stream no closer", "fromstream([[0],1])"),
        (
            "stream closer resets",
            "fromstream([[0,0],1], [[0,1],2], [[0]], [[1,0],3], [[1,0]], [[1]])",
        ),
        ("stream root replaces", "fromstream([[0],1], [[],2])"),
        ("stream number length", "fromstream([1])"),
        ("stream extra element", "fromstream([[0],1,99], [[1]])"),
        ("stream hole", "fromstream([[2],9], [[2]])"),
        ("stream add path", "try fromstream([1,2]) catch ."),
        ("stream index number", "try fromstream(1) catch ."),
        ("stream boolean length", "try fromstream(false) catch ."),
        ("stream index array", "try fromstream([[[0]],1]) catch ."),
        ("stream string index", r#"try fromstream("ab") catch ."#),
        ("stream null path", "fromstream([null,1])"),
        (
            "stream partial then error",
            r#"try fromstream([[],1], error("z")) catch ."#,
        ),
        ("stream first stops", r#"first(fromstream([[],1], error("z")))"#),
        ("stream shadow fromstream", "def fromstream(s): 9; fromstream(1)"),
        ("stream shadow tostream", "def tostream: 7; 1 | tostream"),
        ("stream path tostream", "try path(tostream) catch ."),
        ("stream path fromstream", "try path(fromstream([[],1])) catch ."),
        ("stream path fromstream bad", "try path(fromstream(1)) catch ."),
        (
            "stream truncate one",
            "1 | truncate_stream([[1,2],[3]] | tostream)",
        ),
        (
            "stream truncate zero",
            "0 | truncate_stream([[1,2],[3]] | tostream)",
        ),
        (
            "stream truncate none",
            "2 | truncate_stream([[1,2],[3]] | tostream)",
        ),
        (
            "stream truncate negative",
            "-1 | truncate_stream([[1,2],[3]] | tostream)",
        ),
        (
            "stream truncate fraction",
            "1.5 | truncate_stream([[1,2],[3]] | tostream)",
        ),
        (
            "stream truncate null",
            "null | truncate_stream([1,2] | tostream)",
        ),
        ("stream truncate scalar", "truncate_stream(1 | tostream)"),
        (
            "stream truncate string depth",
            r#""a" | truncate_stream([1,2] | tostream)"#,
        ),
        (
            "stream truncate object item",
            r#"try (1 | truncate_stream({"a":1})) catch ."#,
        ),
        (
            "stream truncate number item",
            "try (null | truncate_stream([1,2])) catch .",
        ),
        (
            "stream truncate bool depth",
            "try (true | truncate_stream([1,2] | tostream)) catch .",
        ),
        (
            "stream truncate ignores input",
            "[1,2] | tostream | truncate_stream(.)",
        ),
        (
            "stream truncate runs on null",
            r#"{"a":1} | truncate_stream(. | tostream)"#,
        ),
        ("stream tostream arity", "tostream(1)"),
        ("stream fromstream arity", "fromstream"),
        ("stream truncate arity", "truncate_stream"),
        ("stream truncate arity two", "truncate_stream(1;2)"),
    ];
    for (name, program) in programs {
        compare_exact(name, &["-n", "-c", "--", program], b"");
    }

    let cases: Vec<(&str, &[&str], &[u8])> = vec![
        (
            "stream option nested",
            &["-c", "--stream", "."],
            b"[[1,2],[3]]",
        ),
        ("stream option scalars", &["-c", "--stream", "."], b"1\n2\n"),
        ("stream option cut off", &["-c", "--stream", "."], b"[1,"),
        (
            "stream option open number",
            &["-c", "--stream", "."],
            b"[1,2",
        ),
        ("stream option comma", &["-c", "--stream", "."], b"[1,2,]"),
        (
            "stream option object comma",
            &["-c", "--stream", "."],
            b"{\"a\":1,}",
        ),
        ("stream option top bracket", &["-c", "--stream", "."], b"1]"),
        (
            "stream option duplicates",
            &["-c", "--stream", "."],
            b"{\"a\":1,\"a\":2}",
        ),
        ("stream option pretty", &["--stream"], b"1"),
        (
            "stream slurp scalars",
            &["-c", "--stream", "-s", "."],
            b"1\n2\n",
        ),
        (
            "stream slurp detached",
            &["-c", "--stream", "-s", "."],
            b"[1,2]",
        ),
        (
            "stream slurp joined",
            &["-c", "--stream", "-s", "."],
            b"[1,2]\n",
        ),
        (
            "stream slurp partial",
            &["-c", "--stream", "-s", "."],
            b"[1,",
        ),
        (
            "stream slurp container",
            &["-c", "--stream", "-s", "."],
            b"[[],",
        ),
        (
            "stream raw wins",
            &["-c", "--stream", "-R", "."],
            b"ab\ncd\n",
        ),
        ("stream null input", &["-nc", "--stream", "."], b"[1,2]"),
        (
            "stream null inputs",
            &["-nc", "--stream", "inputs"],
            b"[[1,2],[3]]",
        ),
        (
            "stream inputs skip",
            &["-c", "--stream", "inputs"],
            b"[1,2]\n[3]",
        ),
        ("stream input break", &["-c", "--stream", "input"], b"[1,2]"),
        ("stream exit empty", &["-c", "--stream", "-e", "."], b""),
        ("stream exit null", &["-c", "--stream", "-e", "."], b"null"),
        (
            "stream error lines",
            &["-c", "--stream", r#"error("z")"#],
            b"[\n1,\n2]",
        ),
        (
            "stream error detached",
            &["-c", "--stream", r#"error("z")"#],
            b"[1,2]",
        ),
        (
            "stream error closed",
            &["-c", "--stream", r#"error("z")"#],
            b"[1,2]\n",
        ),
        (
            "stream slurp error",
            &["-c", "--stream", "-s", r#"error("z")"#],
            b"[1,2]",
        ),
        ("stream colon", &["-c", "--stream", "."], b":"),
        ("stream string tail", &["-c", "--stream", "."], b"\"\"]"),
        ("stream separator word", &["-c", "--stream", "."], b"[1["),
        ("stream bad key", &["-c", "--stream", "."], b"{1]"),
        ("stream errors cut", &["-c", "--stream-errors", "."], b"[1,"),
        (
            "stream errors top",
            &["-c", "--stream-errors", "."],
            b"true}",
        ),
        (
            "stream errors object",
            &["-c", "--stream-errors", "."],
            b"{\"a\":",
        ),
        (
            "stream errors comma",
            &["-c", "--stream-errors", "."],
            b"{\"a\":1,}",
        ),
        (
            "stream errors later",
            &["-c", "--stream-errors", "."],
            b"[1,]\n2\n",
        ),
        (
            "stream errors length",
            &["-c", "--stream-errors", "length"],
            b"[1,",
        ),
        (
            "stream errors empty",
            &["-c", "--stream-errors", "empty"],
            b"[1,",
        ),
        (
            "stream errors slurp cut",
            &["-c", "--stream-errors", "-s", "."],
            b"[1,",
        ),
        (
            "stream errors slurp hard",
            &["-c", "--stream-errors", "-s", "."],
            b"[1,2,]",
        ),
        (
            "stream errors raw",
            &["-c", "--stream-errors", "-R", "."],
            b"ab\n",
        ),
        ("stream errors exit", &["-ce", "--stream-errors", "."], b""),
        (
            "stream errors lines",
            &["-c", "--stream-errors", r#"error("z")"#],
            b"[1,",
        ),
        ("nul end", &["-c", "."], b"1\x00junk"),
        ("nul stream", &["-c", "--stream", "."], b"1\x00junk"),
        ("nul lead", &["-c", "--stream", "."], b"\x001"),
        ("nul mid", &["-c", "--stream", "."], b"[1,\x00 2]"),
        ("nul plain mid", &["-c", "."], b"[1,\x00 2]"),
        ("nul next line", &["-c", "--stream", "."], b"1\x00x\n2\n"),
        ("nul literal", &["-c", "."], b"true\x00\n"),
        ("nul array", &["-c", "--stream", "."], b"[1\x002]\n"),
        ("nul inputs", &["-nc", "--stream", "inputs"], b"[1,"),
    ];
    for (name, args, input) in cases {
        compare_exact(name, args, input);
    }

    let mut deep = vec![b'['; 300];
    deep.push(b'0');
    deep.extend(std::iter::repeat_n(b']', 300));
    compare_exact("stream deep", &["-c", "--stream", "length"], &deep);

    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("stream-span-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    let left = dir.join("left.json");
    let right = dir.join("right.json");
    fs::write(&left, b"[1,").unwrap();
    fs::write(&right, b"2]").unwrap();
    compare_exact(
        "stream spans files",
        &[
            "-c",
            "--stream",
            ".",
            left.to_str().unwrap(),
            right.to_str().unwrap(),
        ],
        b"",
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn stream_prints_before_input_ends() {
    require_jq();
    let mut payload = vec![b'['];
    for _ in 0..4000 {
        payload.extend_from_slice(b"0,");
    }
    assert!(
        saw_stream_output_before_close("jq", &payload),
        "jq did not print a stream item before stdin closed"
    );
    assert!(
        saw_stream_output_before_close(env!("CARGO_BIN_EXE_tumulus"), &payload),
        "tumulus did not print a stream item before stdin closed"
    );
}

/// Write `payload` and stop once stdout has bytes, without closing stdin first.
fn saw_stream_output_before_close(program: &str, payload: &[u8]) -> bool {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let mut child = Command::new(program)
        .args(["-c", "--stream", "."])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("could not start {program}: {err}"));
    let mut stdout = child.stdout.take().expect("stdout");
    let saw = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&saw);
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 64];
        if stdout.read(&mut buf).ok().is_some_and(|n| n > 0) {
            flag.store(true, Ordering::SeqCst);
        }
    });
    let mut stdin = child.stdin.take().expect("stdin");
    let mut sent = 0;
    let start = Instant::now();
    // Keep stdin open after the bytes are written. The item has to show up
    // before that pipe is closed.
    while !saw.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(3) {
        if sent < payload.len() {
            let end = (sent + 1024).min(payload.len());
            match stdin.write(&payload[sent..end]) {
                Ok(0) | Err(_) => {}
                Ok(n) => sent += n,
            }
            let _ = stdin.flush();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let printed = saw.load(Ordering::SeqCst);
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    printed
}
