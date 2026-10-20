//! Files, inputs, and line numbers compared with jq.

use super::*;

#[test]
fn missing_file_still_prints_the_rest() {
    require_jq();
    let dir = std::env::temp_dir().join(format!(
        "tumulus-missing-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _cleanup = DirGuard(dir.clone());
    fs::create_dir_all(&dir).expect("temp dir");
    let present = dir.join("present.json");
    let later = dir.join("later.json");
    fs::write(&present, br#"{"a":1}"#).expect("write");
    fs::write(&later, b"[2,3]").expect("write");
    let present = present.to_str().expect("utf-8 path");
    let later = later.to_str().expect("utf-8 path");
    let missing = dir.join("missing.json");
    let missing = missing.to_str().expect("utf-8 path");
    let also = dir.join("also-missing.json");
    let also = also.to_str().expect("utf-8 path");

    compare_exact("only missing", &[".", missing], b"");
    compare_exact("present then missing", &[".", present, missing], b"");
    compare_exact("missing then present", &[".", missing, present], b"");
    compare_exact(
        "missing between files",
        &[".", present, missing, later],
        b"",
    );
    compare_exact(
        "missing between files compact",
        &["-c", ".", present, missing, later],
        b"",
    );
    compare_exact(
        "two missing files",
        &[".", present, missing, also, later],
        b"",
    );
    compare_exact(
        "missing then stdin",
        &["-c", ".", missing, "-"],
        br#"{"from":"stdin"}"#,
    );
}

#[test]
fn truncated_tail_matches_jq() {
    require_jq();
    // A cut-off final value must not drop the values parsed before it.
    let cases: &[(&str, &[&str], &str)] = &[
        ("cut object", &["."], "1\n{\"a\":"),
        ("cut object compact", &["-c", "."], "1\n{\"a\":"),
        ("second object cut", &["."], "{\"a\":1}\n{\"b\":"),
        ("cut array", &["."], "1\n2\n["),
        ("cut string", &["."], "1\n\"abc"),
        ("string cut by newline", &["."], "\"abc\n"),
        ("unmatched after values", &["."], "1\n2\n]"),
        ("field on cut stream", &[".a"], "1\n{\"a\":"),
        ("optional field", &[".a?"], "1\n{\"a\":"),
        (
            "compact field of good value",
            &["-c", ".n"],
            "{\"n\":1}\n{\"n\":",
        ),
        ("iterate then cut", &[".[]"], "[1,2]\n{\"a\":"),
        ("glued bracket keeps earlier value", &[".a"], "1 2]"),
        ("join", &["-j", "."], "1\n{\"a\":"),
        ("lone low surrogate", &["-c", "."], "\"\\uDC00\""),
    ];
    for (name, args, input) in cases {
        compare_exact(name, args, input.as_bytes());
    }
    compare(
        "nested 256",
        &["-c", "."],
        format!("{}{}", "[".repeat(256), "]".repeat(256)).as_bytes(),
    );
    compare_exact("nested 257", &["."], "[".repeat(257).as_bytes());

    let dir = std::env::temp_dir().join(format!(
        "tumulus-cut-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _cleanup = DirGuard(dir.clone());
    fs::create_dir_all(&dir).expect("temp dir");
    let cut = dir.join("cut.json");
    fs::write(&cut, "{\"n\":1}\n{\"n\":").expect("write");
    let cut = cut.to_str().expect("utf-8 path");
    compare_exact("cut file", &[".", cut], b"");
    compare_exact("cut file field", &["-c", ".n", cut], b"");
}

#[test]
fn input_lines_match_jq() {
    require_jq();
    let samples: &[(&str, &[u8])] = &[
        ("one line", b"1\n"),
        ("no newline", b"1"),
        ("two on one line", b"1 2\n"),
        ("two on one line, no newline", b"1 2"),
        ("one value per line", b"1\n2\n"),
        ("last line has no newline", b"1\n2"),
        ("leading newline", b"\n1\n"),
        ("leading newline, no trailing newline", b"\n1"),
        ("blank lines between values", b"1\n\n\n2\n"),
        ("spaces around a value", b"  1  \n"),
        ("multi-line object", b"{\n \"a\": 1\n}\n"),
        ("multi-line object, no newline", b"{\n \"a\": 1\n}"),
        (
            "multi-line object, spaces after it",
            b"{\n \"a\": 1\n}   \n",
        ),
        ("multi-line, then a value", b"{\n \"a\": 1\n}\n2\n"),
        (
            "multi-line, then a value, no newline",
            b"{\n \"a\": 1\n}\n2",
        ),
        ("value on the closing line", b"{\n \"a\": 1\n} 2\n"),
        (
            "value on the closing line, no newline",
            b"{\n \"a\": 1\n} 2",
        ),
        ("two multi-line objects", b"{\n\"a\":1\n}\n{\n\"b\":2\n}\n"),
        (
            "second multi-line object has no newline",
            b"{\n\"a\":1\n}\n{\n\"b\":2\n}",
        ),
        ("multi-line array", b"[\n 1,\n 2\n]\n"),
        ("multi-line array, no newline", b"[\n 1,\n 2\n]"),
        ("nested object", b"{\n \"a\": {\n  \"b\": 1\n }\n}\n"),
        ("crlf object", b"{\r\n \"a\": 1\r\n}\r\n"),
        ("crlf object, no newline", b"{\r\n \"a\": 1\r\n}"),
        ("carriage return between values", b"1\r2\n"),
        ("carriage return, no newline", b"1\r2"),
        ("leading bom", b"\xef\xbb\xbf{\n \"a\": 1\n}\n"),
        ("leading bom, no newline", b"\xef\xbb\xbf1"),
        ("tab indented object", b"{\n\t\"a\": 1\n}\n"),
        ("empty object across lines", b"{\n}\n"),
        ("string value", b"\"abc\"\n"),
    ];
    for (name, input) in samples {
        compare_exact(name, &["-c", "0|.a"], input);
    }

    // A read holds 4095 bytes. Past that, the newline is a later read.
    let mut same_line = b"1".to_vec();
    same_line.extend(std::iter::repeat(b' ').take(4093));
    same_line.extend(b"2\n");
    compare_exact(
        "two values on a line longer than one read",
        &["-c", "0|.a"],
        &same_line,
    );

    let mut number_at_end = vec![b' '; 4094];
    number_at_end.extend(b"1\n");
    compare_exact(
        "number ends on the last byte of a read",
        &["-c", "0|.a"],
        &number_at_end,
    );

    let mut object_at_end = vec![b' '; 4093];
    object_at_end.extend(b"{}\n");
    compare_exact(
        "object ends on the last byte of a read",
        &["-c", "0|.a"],
        &object_at_end,
    );

    let mut bom_long = vec![0xEF, 0xBB, 0xBF];
    bom_long.extend(b"1");
    bom_long.extend(std::iter::repeat(b' ').take(4092));
    bom_long.push(b'\n');
    compare_exact(
        "bom on a line longer than one read",
        &["-c", "0|.a"],
        &bom_long,
    );
}

#[test]
fn requests_match_jq() {
    require_jq();
    let path = fixture_path("requests.ndjson");
    for program in [
        ".method",
        ".user.plan // \"none\"",
        "select(.status >= 400) | .path",
        "{method, status, ms}",
        ".user.plan + 1",
        ".tags[0] + 1",
        ".method += 1",
    ] {
        compare_exact(program, &["-c", "--", program, path.as_str()], b"");
    }
}

#[test]
fn inputs_on_requests_match_jq() {
    require_jq();
    let path = fixture_path("requests-200.ndjson");
    let file = path.as_str();
    compare_exact("[inputs] | length", &["-n", "[inputs] | length", file], b"");
    compare_exact(
        "reduce inputs",
        &["-n", "reduce inputs as $r (0; . + 1)", file],
        b"",
    );
    compare_exact(
        "raw [inputs] | length",
        &["-R", "-n", "[inputs] | length", file],
        b"",
    );
    compare_exact("input | .method", &["-n", "input | .method", file], b"");
}

#[test]
fn slurp_matches_jq() {
    require_jq();
    let (dir, _guard) = scratch("slurp");
    let one = write_bytes(&dir, "one", b"1");
    let two = write_bytes(&dir, "two", b"2\n");
    let empty = write_bytes(&dir, "empty", b"");
    let separated_a = write_bytes(&dir, "a.json", b"1\n");
    let separated_b = write_bytes(&dir, "b.json", b"2\n");
    let unf = write_bytes(&dir, "unf", b"{\"a\":");
    let close = write_bytes(&dir, "close", b"1}\n");
    let hel = write_bytes(&dir, "hel", b"\"hel");
    let lo = write_bytes(&dir, "lo", b"lo\"\n");
    let nums = write_bytes(&dir, "nums.json", b"1\n2\n");
    let obj = write_bytes(&dir, "obj.json", b"{\"a\":1}\n{\"a\":2}\n");
    let good = write_bytes(&dir, "good", b"1\n2\n3\n");
    let bad = write_bytes(&dir, "bad", b"not\n");
    let part = write_bytes(&dir, "part", b"1\nnot\n");
    let hello = write_bytes(&dir, "hello", b"hello");
    let justx = write_bytes(&dir, "justx", b"x");
    let stream = write_bytes(&dir, "stream.json", b"1\n2\n");
    let missing = dir.join("nope").to_str().expect("utf-8").to_string();
    let missing_b = dir.join("nope2").to_str().expect("utf-8").to_string();

    compare_exact("glue", &["-c", "-s", ".", &one, &two], b"");
    compare_exact(
        "glue across empty",
        &["-c", "-s", ".", &one, &empty, &two],
        b"",
    );
    compare_exact("object across files", &["-c", "-s", ".", &unf, &close], b"");
    compare_exact("string across files", &["-c", "-s", ".", &hel, &lo], b"");
    compare_exact(
        "separate values",
        &["-c", "-s", ".", &separated_a, &separated_b],
        b"",
    );
    compare_exact("pretty", &["-s", ".", &separated_a, &separated_b], b"");
    compare_exact(
        "tabs",
        &["--tab", "--slurp", ".", &separated_a, &separated_b],
        b"",
    );
    compare_exact(
        "indent",
        &["--indent", "4", "-s", ".", &separated_a, &separated_b],
        b"",
    );
    compare_exact("committed parse", &["-c", "-s", ".", &good, &bad], b"");
    compare_exact(
        "parse stops later file",
        &["-c", "-s", ".", &part, &missing],
        b"",
    );
    compare_exact(
        "missing then parse",
        &["-c", "-s", ".", &missing, &part],
        b"",
    );
    compare_exact(
        "eof parse continues",
        &["-c", "-s", ".", &hello, &missing, &justx],
        b"",
    );
    compare_exact("all missing", &["-c", "-s", ".", &missing, &missing_b], b"");
    compare_exact(
        "all missing stays 2",
        &["-e", "-c", "-s", ".", &missing, &missing_b],
        b"",
    );
    compare_exact("whitespace", &["-c", "-s", "."], b" \n \n");
    compare_exact("stdin", &["-c", "-s", "."], b"1\n2\n");
    compare_exact("flag after program", &["-c", ".", "-s"], b"1\n2\n");
    compare_exact("flags after file", &[".", &stream, "-s", "-c"], b"");
    compare_exact("end of options", &["-c", "--", ".", "-s"], b"");
    compare_exact("long name", &["--slurp", "-c", "."], b"1\n2\n");
    compare_exact("location full", &["-s", "0|.a"], b"1\n2\n3\n");
    compare_exact("location no newline", &["-s", "0|.a"], b"1");
    compare_exact("location partial", &["-s", "0|.a"], b"1\n2");
    compare_exact("location file", &["-s", "0|.a", &obj], b"");
    compare_exact("location last file", &["-s", "0|.a", &obj, &nums], b"");
    compare_exact("missing wins", &["-s", "0|.a", &nums, &missing], b"");
    compare_exact(
        "missing then later file",
        &["-s", "0|.a", &nums, &missing, &obj],
        b"",
    );
    compare_exact(
        "slurp reads every value",
        &["-c", "-s", ".", &separated_a, &missing, &stream],
        b"",
    );
    compare_exact(
        "arg with slurp",
        &["-c", "--arg", "a", "z", "-s", "[$a,.]"],
        b"1\n2\n",
    );
    compare_exact(
        "argjson with slurp",
        &["-c", "-s", "--argjson", "a", "[3]", "$a+."],
        b"1\n2\n",
    );
    compare_exact(
        "null ignores slurp",
        &["-n", "-c", "-s", ".", &stream],
        b"9\n",
    );
    compare_exact(
        "null ignores missing",
        &["-n", "-e", "-s", "-R", ".", &missing],
        b"",
    );
}

#[test]
fn raw_input_matches_jq() {
    require_jq();
    let (dir, _guard) = scratch("raw");
    let hello = write_bytes(&dir, "hello", b"hello");
    let empty = write_bytes(&dir, "emptyf", b"");
    let justx = write_bytes(&dir, "justx", b"x");
    let xnl = write_bytes(&dir, "xnl", b"x\n");
    let mid = write_bytes(&dir, "mid", b"hello\nworld");
    let zed = write_bytes(&dir, "Z", b"Z");
    let pq = write_bytes(&dir, "pq", b"p\nq\n");
    let lines = write_bytes(&dir, "lines", b"a\nb\nc\n");
    let de = write_bytes(&dir, "de", b"d\ne\n");
    let hx = write_bytes(&dir, "hx", b"hello");
    let xy = write_bytes(&dir, "xy", b"x\ny\n");
    let cr = write_bytes(&dir, "cr", b"a\r\nb\n");
    let nul = write_bytes(&dir, "nul", b"a\x00b\n");
    let badutf = write_bytes(&dir, "badutf", b"\xff\n\xff\xfe\n\x80\n\xc3\x28\n");
    let ab = write_bytes(&dir, "ab", b"a\nb\n");
    let more = write_bytes(&dir, "more", b"c\nd\n");
    let bad_line = write_bytes(&dir, "badline", b"bad\n");
    let ok_line = write_bytes(&dir, "okline", b"ok\n");
    let text = write_bytes(&dir, "text", b"ab\ncd");
    let missing = dir.join("nope").to_str().expect("utf-8").to_string();
    let missing_b = dir.join("nope2").to_str().expect("utf-8").to_string();
    let missing_c = dir.join("nope3").to_str().expect("utf-8").to_string();
    let when = "if .==\"bad\" then 0|.a else . end";

    compare_exact(
        "span empty",
        &["-c", "-R", ".", &hello, &empty, &justx],
        b"",
    );
    compare_exact("span location", &["-R", "0|.a", &hello, &empty], b"");
    compare_exact("missing location", &["-R", "0|.a", &hello, &missing], b"");
    compare_exact("carry newline", &["-c", "-R", ".", &xnl, &mid], b"");
    compare_exact("carry locations", &["-R", "0|.a", &xnl, &mid], b"");
    compare_exact("owned by next file", &["-c", "-R", ".", &zed, &pq], b"");
    compare_exact("owned locations", &["-R", "0|.a", &zed, &pq], b"");
    compare_exact("no newline", &["-c", "-R", ".", &justx], b"");
    compare_exact("no newline location", &["-R", "0|.a", &justx], b"");
    compare_exact(
        "one after missing",
        &["-c", "-R", ".", &missing, &lines],
        b"",
    );
    compare_exact(
        "line after missing",
        &["-c", "-R", ".", &lines, &missing, &de],
        b"",
    );
    compare_exact(
        "span after missing",
        &["-c", "-R", ".", &missing, &hx, &xy],
        b"",
    );
    compare_exact(
        "two missing then one line",
        &["-c", "-R", ".", &missing, &missing_b, &lines],
        b"",
    );
    compare_exact(
        "empty then missing",
        &["-c", "-R", ".", &empty, &missing, &lines],
        b"",
    );
    compare_exact(
        "span is the one line",
        &["-c", "-R", ".", &ab, &missing, &hx, &xy],
        b"",
    );
    compare_exact(
        "stop before a later missing file",
        &["-c", "-R", ".", &ab, &missing, &more, &missing_c],
        b"",
    );
    compare_exact("cr", &["-c", "-R", ".", &cr], b"");
    compare_exact("nul json", &["-c", "-R", ".", &nul], b"");
    compare_exact("nul raw", &["-Rr", ".", &nul], b"");
    compare_exact("bad utf", &["-c", "-R", ".", &badutf], b"");
    compare_exact("bad utf raw", &["-c", "-Rr", ".", &badutf], b"");
    compare_exact(
        "trailing after missing",
        &["-c", "-R", ".", &hello, &missing],
        b"",
    );
    compare_exact("trailing location", &["-R", "0|.a", &hello, &missing], b"");
    compare_exact("earlier failure clears", &["-c", "-R", when], b"bad\nok\n");
    compare_exact(
        "last failure stays",
        &["-e", "-c", "-R", when],
        b"ok\nbad\n",
    );
    compare_exact(
        "missing wins over a cleared line",
        &["-e", "-c", "-R", when, &bad_line, &missing, &ok_line],
        b"",
    );
    compare_exact("join", &["-j", "-R", "."], b"a\nb\n");
    compare_exact("ascii", &["-a", "-c", "-R", "."], "é\n".as_bytes());
    compare_exact("raw output", &["-Rr", "."], b"a\n");
    compare_exact("raw output other order", &["-rR", "."], b"a\n");
    compare_exact("long names", &["--raw-input", "--raw-output", "."], b"a\n");
    compare_exact("slurp bytes", &["-c", "-Rs", "."], b"a\n");
    compare_exact("slurp other order", &["-c", "-sR", "."], b"a\n");
    compare_exact(
        "slurp across missing",
        &["-c", "-Rs", ".", &hello, &missing, &xnl],
        b"",
    );
    compare_exact(
        "slurp all missing",
        &["-c", "-Rs", ".", &missing, &missing_b],
        b"",
    );
    compare_exact("slurp location", &["-Rs", "0|.a"], b"1\n2\n3\n");
    compare_exact("slurp location short", &["-Rs", "0|.a"], b"1");
    compare_exact("slurp missing wins", &["-Rs", "0|.a", &text, &missing], b"");
    compare_exact("stdin line", &["-c", "-R", "."], b"a\nb\n");
    compare_exact("null ignores raw", &["-n", "-c", "-R", ".", &lines], b"9\n");
}

#[test]
fn filter_files_match_jq() {
    require_jq();
    let (dir, _guard) = scratch("filter");
    let prog = write_bytes(&dir, "prog.jq", b".a\n");
    let commented = write_bytes(&dir, "comment.jq", b"# comment\n.a\n");
    let data = write_bytes(&dir, "obj.json", b"{\"a\":1}\n");
    let a = write_bytes(&dir, "a.json", b"1\n");
    let b = write_bytes(&dir, "b.json", b"2\n");
    let len = write_bytes(&dir, "len.jq", b"length\n");
    let text = write_bytes(&dir, "text.txt", b"ab\ncd");
    let second = write_bytes(&dir, "second.json", b"{\"b\":2}\n");
    let dot = write_bytes(&dir, "dot.jq", b".\n");
    let just = write_bytes(&dir, "one.jq", b"1\n");
    let fals = write_bytes(&dir, "false.jq", b"false\n");
    let none = write_bytes(&dir, "none.jq", b"empty\n");
    let argp = write_bytes(&dir, "arg.jq", b"$a\n");
    let sub = dir.join("sub");
    fs::create_dir(&sub).expect("subdir");
    let sub = sub.to_str().expect("utf-8").to_string();
    let missing = dir.join("missing.jq").to_str().expect("utf-8").to_string();
    let absent = dir.join("absent.json").to_str().expect("utf-8").to_string();

    compare_exact("file", &["-c", "-f", &prog, &data], b"");
    compare_exact("long name", &["-c", "--from-file", &prog, &data], b"");
    compare_exact("comment", &["-c", "-f", &commented, &data], b"");
    compare_exact("exit and compact", &["-cfe", &prog, &data], b"");
    compare_exact("cluster slurp", &["-c", "-nfs", &len, &a, &b], b"");
    compare_exact("flag then file", &["-c", "-f", "-s", &len, &a, &b], b"");
    compare_exact(
        "long slurp",
        &["-c", "--from-file", "--slurp", &len, &a, &b],
        b"",
    );
    compare_exact("raw slurp file", &["-c", "-Rsf", &len, &text], b"");
    compare_exact(
        "second flag is an input",
        &["-c", "-f", &prog, "-f", &second, &data],
        b"",
    );
    compare_exact("first positional", &["-c", &dot, "-f", &data], b"");
    compare_exact(
        "program text is the input",
        &["-c", &data, "-f", &prog, &second],
        b"",
    );
    compare_exact(
        "filter path is an operand",
        &["-c", ".b", "-f", &prog, &data],
        b"",
    );
    compare_exact("after end", &["-c", "-f", "--", &prog, &data], b"");
    compare_exact("stdin", &["-c", "-f", &prog], b"{\"a\":7}\n");
    compare_exact("slurp stdin", &["-c", "-f", &len, "-s"], b"1\n2\n");
    compare_exact(
        "null skips a missing input",
        &["-n", "-c", "-f", &just, &absent],
        b"",
    );
    compare_exact(
        "arg before",
        &["-n", "-c", "--arg", "a", "9", "-f", &argp],
        b"",
    );
    compare_exact(
        "arg after",
        &["-n", "-c", "-f", &argp, "--arg", "a", "9"],
        b"",
    );
    compare_exact("false file", &["-n", "-e", "-f", &fals], b"");
    compare_exact("empty result", &["-n", "-e", "-f", &none], b"");
    compare_exact("dot directory", &["-n", "-f", "."], b"");
    compare_exact("subdirectory", &["-n", "-e", "-f", &sub], b"");
    compare_exact("missing filter", &["-n", "-e", "-f", &missing], b"");
    compare_exact("empty path", &["-n", "-f", ""], b"");
    compare_exact("dash is not stdin", &["-n", "-f", "-"], b"1\n");
    compare_exact("no name opens dot", &["-f", "-s"], b"");
    compare_exact("end opens dot", &["-f", "--"], b"");
}

#[test]
fn directory_files_match_jq() {
    require_jq();
    let (dir, _guard) = scratch("dirfile");
    let folder = dir.join("folder");
    fs::create_dir(&folder).unwrap();
    let folder = folder.to_str().expect("utf-8").to_string();
    let one = write_bytes(&dir, "one.json", b"9\n");
    let missing = dir.join("missing.json");
    let missing = missing.to_str().expect("utf-8");
    compare_exact("only", &["-c", ".", &folder], b"");
    compare_exact("then file", &["-c", ".", &folder, &one], b"");
    compare_exact("file then", &["-c", ".", &one, &folder], b"");
    compare_exact("two", &["-c", ".", &folder, &folder], b"");
    compare_exact("three", &["-c", ".", &folder, &folder, &folder], b"");
    compare_exact("then missing", &["-c", ".", &folder, missing], b"");
    compare_exact("missing then", &["-c", ".", missing, &folder], b"");
    compare_exact("slurp only", &["-s", "-c", ".", &folder], b"");
    compare_exact("slurp then", &["-s", "-c", ".", &folder, &one], b"");
    compare_exact("slurp after", &["-s", "-c", ".", &one, &folder], b"");
    compare_exact("raw only", &["-R", "-c", ".", &folder], b"");
    compare_exact("raw then", &["-R", "-c", ".", &folder, &one], b"");
    compare_exact("raw after", &["-R", "-c", ".", &one, &folder], b"");
    compare_exact("raw slurp", &["-s", "-R", "-c", ".", &folder], b"");
    compare_exact(
        "raw slurp then",
        &["-s", "-R", "-c", ".", &folder, &one],
        b"",
    );
    compare_exact("stream only", &["--stream", "-c", ".", &folder], b"");
    compare_exact("stream then", &["--stream", "-c", ".", &folder, &one], b"");
    compare_exact(
        "stream two",
        &["--stream", "-c", ".", &folder, &folder],
        b"",
    );
    compare_exact("seq only", &["--seq", "-c", ".", &folder], b"");
    compare_exact("seq two", &["--seq", "-c", ".", &folder, &folder], b"");
    compare_exact(
        "seq then missing",
        &["--seq", "-c", ".", &folder, missing],
        b"",
    );
    compare_exact("null input", &["-n", "-c", ".", &folder], b"");
    compare_exact("stdin after", &["-c", ".", &folder, "-"], b"1\n");
    compare_exact("stdin before", &["-c", ".", "-", &folder], b"1\n");
    compare_exact("exit status", &["-e", "-c", ".", &folder, &one], b"");
}

#[test]
fn joined_files_match_jq() {
    require_jq();
    let (dir, _guard) = scratch("joined");
    let open_array = write_bytes(&dir, "open-array.json", b"[1,\n");
    let close_array = write_bytes(&dir, "close-array.json", b"2]\n");
    let open_object = write_bytes(&dir, "open-object.json", b"{\"a\":\n");
    let close_object = write_bytes(&dir, "close-object.json", b"1}\n");
    let nested_open = write_bytes(&dir, "nested-open.json", b"{\"a\":[1,\n");
    let nested_close = write_bytes(&dir, "nested-close.json", b"{\"b\":2}]}\n");
    let empty = write_bytes(&dir, "empty.json", b"");
    let third = write_bytes(&dir, "third.json", b"3]\n");
    let begun = write_bytes(&dir, "begun.json", b"1\n[2,\n");
    let bad = write_bytes(&dir, "bad.json", b"not\n");
    let wrong = write_bytes(&dir, "wrong.json", b"}\n");
    let alone = write_bytes(&dir, "alone.json", b"[1,\n");
    let later = write_bytes(&dir, "later.json", b"9\n");
    compare_exact("array", &["-c", ".", &open_array, &close_array], b"");
    compare_exact("object", &["-c", ".", &open_object, &close_object], b"");
    compare_exact("nested", &["-c", ".", &nested_open, &nested_close], b"");
    compare_exact(
        "empty between",
        &["-c", ".", &open_array, &empty, &close_array],
        b"",
    );
    compare_exact(
        "three files",
        &["-c", ".", &open_array, &empty, &third],
        b"",
    );
    compare_exact("value then bad", &["-c", ".", &begun, &bad], b"");
    compare_exact("wrong closer", &["-c", ".", &open_array, &wrong], b"");
    compare_exact("unfinished alone", &["-c", ".", &alone], b"");
    compare_exact(
        "bad stops the next file",
        &["-c", ".", &begun, &bad, &later],
        b"",
    );
    compare_exact(
        "error names the closing file",
        &["-c", "error(\"z\")", &open_array, &close_array],
        b"",
    );
}

/// `--seq` joins files. A number cut off at the end of one file is one
/// warning at the real end, not another warning at that file.
#[test]
fn seq_number_cut_across_files_warns_once() {
    require_jq();
    let (dir, _guard) = scratch("seq-cut");
    let head = write_bytes(&dir, "head.seq", b"\x1e12");
    let tail = write_bytes(&dir, "tail.seq", b"3");
    compare_exact(
        "still cut at the end",
        &["--seq", "-c", ".", &head, &tail],
        b"",
    );
    let done = write_bytes(&dir, "done.seq", b"3\n");
    compare_exact(
        "finished in the next file",
        &["--seq", "-c", ".", &head, &done],
        b"",
    );
    let next = write_bytes(&dir, "next.seq", b"\x1e4\n");
    compare_exact(
        "cut by the next record",
        &["--seq", "-c", ".", &head, &next],
        b"",
    );
    let one = write_bytes(&dir, "one.seq", b"\x1e1\n");
    let missing = dir.join("missing.seq");
    let missing = missing.to_str().expect("utf-8");
    compare_exact(
        "halt does not open the next file",
        &["--seq", "-c", "halt", &one, missing],
        b"",
    );
}

#[test]
fn inputs_span_files_match_jq() {
    require_jq();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("span-files-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let put = |name: &str, bytes: &[u8]| {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        path
    };
    let a = put("a.json", b"1\n2");
    let b = put("b.json", b"3");
    let c = put("c.json", b"4");
    let nl = put("nl.json", b"1\n2\n");
    let sp = put("sp.json", b"1\n2 ");
    let tru = put("tru.json", b"tru");
    let e = put("e.json", b"e");
    let left = put("left.json", b"\"ab");
    let right = put("right.json", b"c\"");
    let open = put("open.json", b"[1,");
    let close = put("close.json", b"2]");
    let dot = put("dot.json", b"1.");
    let five = put("five.json", b"5");
    let inf = put("inf.json", b"inf");
    let inity = put("inity.json", b"inity");
    let nan = put("nan.json", b"nan");
    let zero = put("zero.json", b"0");
    let yes = put("yes.json", b"true");
    let bad = put("bad.json", b"x");
    let minus = put("minus.json", b"-");
    let two = put("two.json", b"2");
    let raw_a = put("raw-a.txt", b"ab");
    let raw_b = put("raw-b.txt", b"c");
    let path = |file: &Path| file.to_str().unwrap().to_string();
    let a = path(&a);
    let b = path(&b);
    let c = path(&c);
    let nl = path(&nl);
    let sp = path(&sp);
    let tru = path(&tru);
    let e = path(&e);
    let left = path(&left);
    let right = path(&right);
    let open = path(&open);
    let close = path(&close);
    let dot = path(&dot);
    let five = path(&five);
    let inf = path(&inf);
    let inity = path(&inity);
    let nan = path(&nan);
    let zero = path(&zero);
    let yes = path(&yes);
    let bad = path(&bad);
    let minus = path(&minus);
    let two = path(&two);
    let raw_a = path(&raw_a);
    let raw_b = path(&raw_b);
    compare_exact("span", &["-n", "[inputs]", &a, &b], b"");
    compare_exact("span compact", &["-nc", "[inputs]", &a, &b], b"");
    compare_exact("span three", &["-nc", "[inputs]", &a, &b, &c], b"");
    compare_exact("span newline", &["-nc", "[inputs]", &nl, &b], b"");
    compare_exact("span space", &["-nc", "[inputs]", &sp, &b], b"");
    compare_exact("span true", &["-nc", "[inputs]", &tru, &e], b"");
    compare_exact("span string", &["-nc", "[inputs]", &left, &right], b"");
    compare_exact("span array", &["-nc", "[inputs]", &open, &close], b"");
    compare_exact("span fraction", &["-nc", "[inputs]", &dot, &five], b"");
    compare_exact("span infinity", &["-nc", "[inputs]", &inf, &inity], b"");
    compare_exact("span nan", &["-nc", "[inputs]", &nan, &zero], b"");
    compare_exact("span minus", &["-nc", "[inputs]", &minus, &two], b"");
    compare_exact("span bad literal", &["-nc", "[inputs]", &yes, &bad], b"");
    compare_exact("span raw", &["-Rnc", "[inputs]", &raw_a, &raw_b], b"");
    compare_exact(
        "span lines",
        &[
            "-nc",
            "reduce inputs as $x ([]; . + [[$x, input_line_number]]), input_line_number",
            &a,
            &b,
        ],
        b"",
    );
    let _ = fs::remove_dir_all(&dir);
}

/// `input_line_number` after `input` or `inputs` has read past the last value.
#[test]
fn input_line_after_end_match_jq() {
    require_jq();
    let cases: &[(&str, &[&str], &[u8])] = &[
        (
            "inputs cut",
            &["-n", "[inputs,input_line_number]"],
            b"1\n2\n3",
        ),
        (
            "inputs cut compact",
            &["-nc", "[inputs,input_line_number]"],
            b"1\n2\n3",
        ),
        (
            "inputs nl",
            &["-nc", "[inputs,input_line_number]"],
            b"1\n2\n3\n",
        ),
        (
            "inputs nl space",
            &["-nc", "[inputs,input_line_number]"],
            b"1\n2\n3\n ",
        ),
        (
            "inputs space",
            &["-nc", "[inputs,input_line_number]"],
            b"1\n2\n3 ",
        ),
        (
            "inputs ws tail",
            &["-nc", "[inputs,input_line_number]"],
            b"1\n2\n   ",
        ),
        (
            "inputs only nl",
            &["-nc", "[inputs,input_line_number]"],
            b"\n",
        ),
        (
            "inputs only space",
            &["-nc", "[inputs,input_line_number]"],
            b" ",
        ),
        ("inputs empty", &["-nc", "[inputs,input_line_number]"], b""),
        (
            "inputs pipe",
            &["-nc", "inputs|[.,input_line_number]"],
            b"1\n2\n3",
        ),
        (
            "inputs reduce",
            &[
                "-nc",
                "reduce inputs as $x ([]; . + [[$x, input_line_number]]), input_line_number",
            ],
            b"1\n2\n3",
        ),
        (
            "inputs reduce nl",
            &[
                "-nc",
                "reduce inputs as $x ([]; . + [[$x, input_line_number]]), input_line_number",
            ],
            b"1\n2\n3\n",
        ),
        (
            "inputs foreach",
            &[
                "-nc",
                "[foreach inputs as $x (0; .+1; input_line_number)], input_line_number",
            ],
            b"1\n2\n3",
        ),
        (
            "input try",
            &[
                "-nc",
                "[try (input,input,input,input) catch input_line_number]",
            ],
            b"1\n2\n3",
        ),
        (
            "input try nl",
            &[
                "-nc",
                "[try (input,input,input,input) catch input_line_number]",
            ],
            b"1\n2\n3\n",
        ),
        (
            "main and inputs",
            &["-c", "[.,inputs,input_line_number]"],
            b"1\n2\n3",
        ),
        ("raw cut", &["-Rnc", "[inputs,input_line_number]"], b"a\nb"),
        ("raw nl", &["-Rnc", "[inputs,input_line_number]"], b"a\nb\n"),
        (
            "raw space",
            &["-Rnc", "[inputs,input_line_number]"],
            b"a\n ",
        ),
        (
            "raw reduce",
            &[
                "-Rnc",
                "reduce inputs as $x ([]; . + [[$x, input_line_number]]), input_line_number",
            ],
            b"a\nb",
        ),
        (
            "slurp cut",
            &["-snc", "[inputs,input_line_number]"],
            b"1\n2",
        ),
        (
            "slurp nl",
            &["-snc", "[inputs,input_line_number]"],
            b"1\n2\n",
        ),
        (
            "slurp reduce",
            &[
                "-snc",
                "reduce inputs as $x ([]; . + [[$x, input_line_number]]), input_line_number",
            ],
            b"1\n2\n",
        ),
        (
            "slurp only nl",
            &["-snc", "[inputs,input_line_number]"],
            b"\n",
        ),
        (
            "string cut",
            &["-nc", "[inputs,input_line_number]"],
            b"\"a\"\n\"b\"",
        ),
        (
            "array cut",
            &["-nc", "[inputs,input_line_number]"],
            b"[1,\n2]",
        ),
        (
            "stream cut",
            &["-nc", "--stream", "[inputs], input_line_number"],
            b"1\n2\n3",
        ),
        (
            "stream nl",
            &["-nc", "--stream", "[inputs], input_line_number"],
            b"1\n2\n3\n",
        ),
        (
            "seq nl",
            &["-nc", "--seq", "[inputs,input_line_number]"],
            b"\x1e1\n\x1e2\n\x1e3\n",
        ),
        (
            "seq cut",
            &["-nc", "--seq", "[inputs,input_line_number]"],
            b"\x1e1\n\x1e2\n\x1e3",
        ),
    ];
    for (name, args, input) in cases {
        compare_exact(name, args, input);
    }

    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("line-end-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    let done = dir.join("done.json");
    let full = dir.join("full.json");
    let empty = dir.join("empty.json");
    fs::write(&done, b"3\n").unwrap();
    fs::write(&full, b"1\n2\n").unwrap();
    fs::write(&empty, b"").unwrap();
    let prog = "reduce inputs as $x ([]; . + [[$x, input_line_number]]), input_line_number";
    compare_exact(
        "two files",
        &["-nc", prog, full.to_str().unwrap(), done.to_str().unwrap()],
        b"",
    );
    compare_exact(
        "nl then empty",
        &["-nc", prog, full.to_str().unwrap(), empty.to_str().unwrap()],
        b"",
    );
    let _ = fs::remove_dir_all(&dir);
}
