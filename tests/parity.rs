//! Run the same inputs through tumulus and jq.
//! jq has to be installed. A missing jq fails this check.
//! Nothing here stands in for jq's output.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, Stdio};

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

/// A child that exits before reading closes the pipe. The check still
/// compares that child's output.
fn write_stdin(stdin: &mut impl Write, program: &str, input: &[u8]) {
    match stdin.write_all(input) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {}
        Err(err) => panic!("write stdin to {program}: {err}"),
    }
}

fn compare(name: &str, args: &[&str], input: &[u8]) {
    let expected = compare_exact(name, args, input);
    assert!(
        expected.status.success(),
        "{name}: jq failed with status {}\n{}",
        expected.status,
        String::from_utf8_lossy(&expected.stderr)
    );
}

fn compare_exact(name: &str, args: &[&str], input: &[u8]) -> std::process::Output {
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
    expected
}

fn fixture(name: &str) -> Vec<u8> {
    fs::read(fixture_path(name)).unwrap_or_else(|err| panic!("read fixture {name}: {err}"))
}

fn fixture_path(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .to_str()
        .expect("fixture path is utf-8")
        .to_string()
}

#[test]
fn jq_parity() {
    require_jq();

    let texts = [
        ("null", "null"),
        ("true", "true"),
        ("false", "false"),
        ("zero", "0"),
        ("one", "1"),
        ("minus", "-1"),
        ("neg zero", "-0"),
        ("zero decimal", "0.0"),
        ("one decimal", "1.0"),
        ("trailing zero", "1.50"),
        ("fraction", "0.10"),
        ("kept fraction", "1.2300"),
        ("burst", "150.0"),
        ("wide int", "9007199254740993"),
        ("long int", "123456789012345678901234567890"),
        ("leading zero", "01"),
        ("zeros", "00"),
        ("plus", "+1"),
        ("plus zero", "+0.0"),
        ("plus decimal", "+1.50"),
        ("trailing dot", "1."),
        ("padded fraction", "00.10"),
        ("neg leading zero", "-01"),
        ("neg zeros", "-00"),
        ("neg padded zero", "-00.0"),
        ("leading dot", ".1"),
        ("neg leading dot", "-.5"),
        ("plus leading dot", "+.5"),
        ("frac zeros", "000.100"),
        ("exp", "1e2"),
        ("exp cap", "1E+2"),
        ("exp plus", "1e+2"),
        ("exp neg", "1e-2"),
        ("exp shift", "1.5e1"),
        ("exp fraction", "1.5e-1"),
        ("exp keep zero", "2.50e1"),
        ("exp one decimal", "10e-1"),
        ("exp zero", "0e0"),
        ("exp zero pos", "0e5"),
        ("exp zero neg", "0e-5"),
        ("exp dot", "1.0e2"),
        ("exp two zeros", "1.00e2"),
        ("exp coef", "100e2"),
        ("fixed floor", "1e-6"),
        ("sci floor", "1e-7"),
        ("sci zero digit", "10e-8"),
        ("tiny", "5e-324"),
        ("beyond double", "1e309"),
        ("double max", "1.7976931348623157e+308"),
        ("neg exp", "-1.5e-1"),
        ("odd exp", "01e2"),
        ("dot exp", ".1e2"),
        ("bare exp", "1.e2"),
        ("plus exp", "+1e2"),
        ("exp pad", "1e+01"),
        ("long exp", "123456789012345678901234567890e-1"),
        (
            "odd mix",
            r#"[01,00,.5,-.5,+1.50,1.,-0,1e2,1.5e-1,2.50e1,1e-7,5e-324]"#,
        ),
        ("empty string", "\"\""),
        ("space string", "\" \""),
        ("slash", "\"a/b\""),
        ("escaped slash", "\"a\\/b\""),
        ("quoted", "\"say \\\"hello\\\"\""),
        ("backslash", "\"c\\\\d\""),
        ("short escapes", "\"\\b\\f\\n\\r\\t\""),
        ("controls", "\"\\u0000\\u0001\\u000b\\u001f\\u007f\""),
        ("unicode escapes", "\"\\u0041\\u00e9\\u2028\\u2029\""),
        ("surrogate", "\"\\uD83D\\uDE00\""),
        ("raw unicode", "\"café 🙂 潮\""),
        ("empty array", "[]"),
        ("empty object", "{}"),
        ("one array", "[1]"),
        ("array", "[1, 2, 3]"),
        ("nested empties", "[{}, [], {\"a\": []}]"),
        ("mixed array", "[1, {\"a\": 2}, 3]"),
        ("key order", "{\"b\": 1, \"a\": 2}"),
        ("nested", "{\"a\": {\"b\": [1, {\"c\": true}]}}"),
        ("duplicate", "{\"a\": 1, \"a\": 2}"),
        (
            "duplicate order",
            "{\"b\": 1, \"a\": 2, \"b\": 3, \"a\": 4}",
        ),
        (
            "duplicate nested",
            "{\"a\": {\"a\": 1, \"a\": 2}, \"a\": 3}",
        ),
        ("duplicate inner", "{\"x\": 1, \"x\": {\"y\": 1, \"y\": 2}}"),
        ("same decoded key", "{\"\\u0041\": 1, \"A\": 2}"),
        (
            "escaped key",
            "{\"a\\\"b\": \"c\\\\d\", \"ü\": 1, \"a/b\": \"/\"}",
        ),
        ("padded", "  { \"b\" : 1 , \"a\" : [ 2 , 3 ] }  "),
        ("scalars", "true false null"),
        ("numbers", "0 0.0 -0 1.50"),
        ("adjacent strings", "\"\"\"\""),
        ("adjacent values", "1\"a\""),
        ("keyword container", "true[]"),
        ("null object", "null{}"),
        ("string number", "\"a\"-1"),
        ("false string", "false\"b\""),
        ("two arrays", "[1][2]"),
        ("two objects", "{\"a\":1}{\"b\":2}"),
        ("crlf pair", "1\r\n2\r\n"),
        ("blank lines", "true\n\nfalse"),
        ("empty input", ""),
        ("whitespace input", " \n\t\r\n"),
    ];

    for (name, input) in texts {
        compare(&format!("{name} pretty"), &["."], input.as_bytes());
        compare(&format!("{name} compact"), &["-c", "."], input.as_bytes());
    }

    let mut bom = vec![0xEF, 0xBB, 0xBF];
    bom.extend_from_slice(br#"{"a":[1,2]}"#);
    compare("bom pretty", &["."], &bom);
    compare("bom compact", &["-c", "."], &bom);

    let sample = br#"{"z":[true,false,null,"x",1.50,{"k":[]}]} "#;
    compare("default filter", &[], sample);
    compare("default compact", &["-c"], sample);
    compare("compact after filter", &[".", "-c"], sample);
    compare("long compact", &["--compact-output", "."], sample);
    compare("end of options", &["-c", "--", "."], sample);

    for name in [
        "package.json",
        "openapi.json",
        "record.json",
        "stream.json",
        "empty.json",
        "blank.json",
    ] {
        let bytes = fixture(name);
        compare(&format!("{name} stdin pretty"), &["."], &bytes);
        compare(&format!("{name} stdin compact"), &["-c", "."], &bytes);
        let path = fixture_path(name);
        compare(&format!("{name} file pretty"), &[".", path.as_str()], b"");
        compare(
            &format!("{name} file compact"),
            &["-c", ".", path.as_str()],
            b"",
        );
    }

    let package = fixture_path("package.json");
    let openapi = fixture_path("openapi.json");
    let record = fixture_path("record.json");
    let stream = fixture_path("stream.json");
    compare("two files", &[".", package.as_str(), openapi.as_str()], b"");
    compare(
        "two files compact",
        &["-c", ".", stream.as_str(), record.as_str()],
        b"",
    );
    compare(
        "compact between filter and file",
        &[".", "-c", "--", record.as_str()],
        b"",
    );
    compare(
        "file then stdin",
        &["-c", ".", package.as_str(), "-"],
        br#"{"from":"stdin","n":1.50}"#,
    );
    compare(
        "stdin then file",
        &[".", "-", stream.as_str()],
        br#"{"first":true}"#,
    );
}

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
fn output_flags_match_jq() {
    require_jq();
    let input =
        r#"{"b":"café","a":"🙂","A":"x\n","e":"","n":"ü","":true} "ü" {"z":1,"y":{"b":1,"a":2}}"#;
    let flags: &[&[&str]] = &[
        &["-r", "."],
        &["-j", "."],
        &["-a", "."],
        &["-S", "."],
        &["--tab", "."],
        &["--indent", "0", "."],
        &["--indent", "1", "."],
        &["--indent", "4", "."],
        &["--indent", "7", "."],
        &["--indent", "-1", "."],
        &["--raw-output", "."],
        &["--join-output", "."],
        &["--ascii-output", "."],
        &["--sort-keys", "."],
        &["-c", "."],
        &["-c", "--indent", "4", "."],
        &["--indent", "4", "-c", "."],
        &["-c", "--tab", "."],
        &["--tab", "-c", "."],
        &["--indent", "0", "--tab", "."],
        &["--tab", "--indent", "0", "."],
        &["--indent", "-1", "-c", "."],
        &["-c", "--indent", "-1", "."],
        &["--indent", "4", "--tab", "."],
        &["--tab", "--indent", "4", "."],
        &["-r", "-a", "."],
        &["-a", "-r", "."],
        &["-j", "-a", "."],
        &["-r", "-S", "."],
        &["-S", "-a", "."],
        &["-j", "-S", "--indent", "4", "."],
        &["-r", "-a", "-S", "--tab", "."],
        &["-j", "-a", "-S", "--indent", "3", "."],
        &["-c", "-a", "-S", "."],
        &["--raw-output", "--ascii-output", "--sort-keys", "."],
        &[
            "--join-output",
            "--ascii-output",
            "--sort-keys",
            "--indent",
            "4",
            ".",
        ],
        &[".", "-S", "-a", "--indent", "1"],
        &["-rj", "."],
        &["-jr", "."],
        &["-ra", "."],
        &["-ar", "."],
        &["-rS", "."],
        &["-ja", "."],
        &["-aS", "."],
        &["-rjSa", "."],
        &["-crj", "."],
        &["-crjaS", "."],
        &["-cS", "."],
        &["-Sc", "."],
        &["-raj", "."],
        &["--indent", "4abc", "."],
        &["--indent", "+4", "."],
        &["--indent", "4.9", "."],
        &["--indent", "foo", "."],
        &["--indent", "-1abc", "."],
        &["--indent", "4294967295", "."],
        &["--indent", "4294967296", "."],
        &["--indent", "4294967297", "."],
    ];
    for args in flags {
        compare(&format!("{args:?}"), args, input.as_bytes());
    }

    let nonfinite = "NaN nan NAN nAn +NaN -nan Infinity infinity INFINITY InFiNiTy inf Inf INF +inf -Inf -infinity +Infinity";
    for args in [
        &["."][..],
        &["-c", "."],
        &["-r", "."],
        &["-a", "."],
        &["-S", "."],
        &["--indent", "4", "."],
        &["-j", "-a", "."],
    ] {
        compare(&format!("nonfinite {args:?}"), args, nonfinite.as_bytes());
    }

    let python = r#"{"z": -Infinity, "y": Infinity, "x": NaN, "s": "café"}"#;
    compare("python object", &["."], python.as_bytes());
    compare("python object compact", &["-c", "."], python.as_bytes());
    compare("python array", &["-c", "."], b"[NaN, Infinity, -Infinity]");

    compare_exact("indent 8", &["--indent", "8", "."], b"1");
    compare_exact("indent 08", &["--indent", "08", "."], b"1");
    compare_exact("indent -2", &["--indent", "-2", "."], b"1");
    compare_exact("indent missing", &["--indent"], b"1");
    compare_exact("indent missing after filter", &[".", "--indent"], b"1");
}

#[test]
fn filters_match_jq() {
    require_jq();
    let doc = r#"{"a":[{"b":1,"c":[10]},2,{"b":3,"c":[]}],"b":[9],"a b":{"z":true},"n":null}"#;
    let cases = [
        (".a", doc),
        (".a.b", r#"{"a":{"b":1}}"#),
        (".a.b", r#"{"a":1}"#),
        (".a.b", "null"),
        (".a.b?", r#"{"a":1}"#),
        (".a?.b", "1"),
        (".a?.b", r#"{"a":1}"#),
        ("(.a.b)?", "1"),
        (r#".["a b"].z"#, doc),
        (r#".["a"].b"#, r#"{"a":{"b":4}}"#),
        (".[0]", "[10,20,30]"),
        (".[-1]", "[10,20,30]"),
        (".[1.9]", "[10,20,30]"),
        (".[-1.1]", "[10,20,30]"),
        (".[10]", "[10,20,30]"),
        (".[0]", "null"),
        (".[0]", "1"),
        (".[0]?", "1"),
        (".[]", "[10,20,30]"),
        (".[]", r#"{"b":1,"a":2}"#),
        (".[]", "[]"),
        (".[]", "1"),
        (".[]", "null"),
        (".[]", r#""abc""#),
        (".[]?", "1"),
        (".[]?", "null"),
        (".a[0].b", doc),
        (".a[].b", doc),
        (".a[].b?", doc),
        (".a[] | .b, .c[]?", doc),
        (".a, .b | .[0]", doc),
        (".[] | .a, .b", r#"[{"a":1,"b":2},{"a":3}]"#),
        ("(.a | .b), .n", r#"{"a":{"b":1},"n":2}"#),
        (".[] | .a?", r#"[1,{"a":2},true]"#),
        ("(.[] | .a)?", r#"[{"a":1},2]"#),
        (".a.b?", "1"),
        (".?", "1"),
        (".a??", r#"{"a":1}"#),
        ("._a", r#"{"_a":1}"#),
        (".a .b", r#"{"a":{"b":1}}"#),
        (".a [0]", r#"{"a":[10,20]}"#),
        (".[][]", "[[1,2],[3]]"),
        (".", "NaN"),
        (".a", "NaN"),
        (".[]", "NaN"),
        (".", "Infinity"),
        (".[]", "Infinity"),
        (".[]", "-Infinity"),
        (".[]", r#""xxxxxxxxxxxx""#),
        (".a", "1\n{\"a\":2}\ntrue\n"),
        (".a", "1\n{\"a\":2}\n"),
        (".a", "{\"a\":2}\n1\n"),
        (".[]", "1\n[]\n"),
        (".[] | .a", r#"[{"a":1},2]"#),
        (".a, .b", "1"),
    ];
    for (expr, input) in cases {
        compare_exact(expr, &[expr], input.as_bytes());
    }
    let slices = [
        (".[2:5]", "[0,1,2,3,4,5,6]"),
        (".[2:5]", "\"abcdef\""),
        (".[:-1]", "[0,1,2,3,4,5,6]"),
        (".[:-1]", "\"abcdef\""),
        (".[2:]", "[0,1,2,3,4,5,6]"),
        (".[:3]", "\"abcdef\""),
        (".[-3:-1]", "[0,1,2,3,4,5,6]"),
        (".[-2:]", "\"abcdef\""),
        (".[5:2]", "[0,1,2,3,4,5,6]"),
        (".[-10:2]", "[0,1,2,3,4,5,6]"),
        (".[1.9:3.2]", "[0,1,2,3,4,5,6]"),
        (".[-1.9:]", "[0,1,2,3,4,5,6]"),
        (".[1.1:2.1]", "[0,1,2,3,4,5,6]"),
        (".[-1.1:-0.1]", "[0,1,2,3,4,5,6]"),
        (".[-0.1:2]", "[0,1,2,3,4,5,6]"),
        (".[0:0.1]", "[0,1,2,3,4,5,6]"),
        (".[1:3]", "\"caf\u{e9}\u{1f642}\""),
        (".[0:1]", "\"\u{1f642}ab\""),
        (".[-2:]", "\"\u{1f642}ab\""),
        (".[:2]", "\"\u{1f642}ab\""),
        (".[2:5]", "null"),
        (".[:-1]", "1"),
        (".[2:5]", "true"),
        (".[1:3]", "{\"a\":1}"),
        (".[2:5]?", "1"),
        (".a[1:3]", "{\"a\":[10,20,30,40]}"),
        (".a[1:3]", "{\"a\":\"abcd\"}"),
        (".[2:5,0]", "[0,1,2,3,4,5,6]"),
        (".[0,1:3]", "[0,1,2,3,4,5,6]"),
        (".[:1,3]", "[0,1,2,3,4,5,6]"),
        (".[2,3:]", "[0,1,2,3,4,5,6]"),
        (".[1,2:4]", "\"abcdef\""),
        (".[1:3]", "1\n\"abcd\"\n"),
        (".[1:3]", "\"abcd\"\n1\n"),
        (".\"a b\"", "{\"a b\":1,\"a\":2}"),
        (".a.\"b c\"", "{\"a\":{\"b c\":4}}"),
        (".\"a\".\"b\"", "{\"a\":{\"b\":5}}"),
        (".\"a b\"[0]", "{\"a b\":[9,8]}"),
        (".[\"a\",\"b\"]", "{\"a\":1,\"b\":2,\"c\":3}"),
        (".[\"b\",\"a\",\"a\"]", "{\"a\":1,\"b\":2}"),
        (".[0,2,1]", "[0,1,2,3,4,5,6]"),
        (".[1,10,-1]", "[0,1,2,3,4,5,6]"),
        (".[\"a\",\"b\"]", "[0,1,2]"),
        (".[0,\"a\"]", "[0,1,2]"),
        (".[\"a\",0]", "[10,20,30]"),
        (".[\"a\",0]?", "[10,20,30]"),
        ("(.[\"a\",0])?", "[10,20,30]"),
        (".[\"a\",0,\"b\",1]?", "[10,20,30]"),
        (".[\"a\",0]??", "[10,20,30]"),
        (".[\"a\",0] ?", "[10,20,30]"),
        (".[\"missing\",\"a\"]", "{\"a\":1}"),
        (".[\"a\",\"b\"]", "null"),
        ("..", "{\"b\":1,\"a\":{\"c\":[2,3]}}"),
        ("..", "[0,1,2]"),
        ("..", "\"abcdef\""),
        ("..", "null"),
        ("..", "[]"),
        ("..", "{}"),
        (".. | .a?", "{\"a\":{\"a\":1},\"b\":2}"),
        ("..[]?", "{\"a\":[1],\"b\":2}"),
        ("..[0]", "[0,1,2]"),
    ];
    for (expr, input) in slices {
        compare_exact(expr, &[expr], input.as_bytes());
    }
    compare_exact(
        "long iterate",
        &[".[]"],
        format!("\"{}\"", "x".repeat(13)).as_bytes(),
    );
    compare_exact("compact field", &["-c", ".a[].b"], doc.as_bytes());
    compare_exact(
        "raw names",
        &["-r", ".a[].b"],
        r#"{"a":[{"b":"caf\u00e9"},{"b":"x"}]}"#.as_bytes(),
    );

    let dir = std::env::temp_dir().join(format!(
        "tumulus-filter-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let _cleanup = DirGuard(dir.clone());
    fs::create_dir_all(&dir).expect("temp dir");
    let ended = dir.join("ended.json");
    let open = dir.join("open.json");
    fs::write(&ended, "1\n{\"a\":2}\n").expect("write");
    fs::write(&open, "1\ntrue").expect("write");
    compare_exact(
        "file lines",
        &[
            ".a",
            ended.to_str().expect("utf-8"),
            open.to_str().expect("utf-8"),
        ],
        b"",
    );
    let bad = dir.join("bad.json");
    let good = dir.join("good.json");
    fs::write(&bad, "1\n").expect("write");
    fs::write(&good, "{\"a\":2}\n").expect("write");
    let bad = bad.to_str().expect("utf-8");
    let good = good.to_str().expect("utf-8");
    compare_exact("file error then value", &[".a", bad, good], b"");
    compare_exact("file value then error", &[".a", good, bad], b"");
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
fn closed_pipe_stops_quietly() {
    require_jq();
    let input = "1\n".repeat(50_000);
    let ours = closed_stdout(
        env!("CARGO_BIN_EXE_tumulus"),
        &["-c", "."],
        input.as_bytes(),
    );
    let jq = closed_stdout("jq", &["-c", "."], input.as_bytes());
    assert_eq!(
        ours,
        jq,
        "closed stdout\nours signal {:?} stderr {}\njq signal {:?} stderr {}",
        ours.0,
        String::from_utf8_lossy(&ours.1),
        jq.0,
        String::from_utf8_lossy(&jq.1),
    );
}

fn closed_stdout(program: &str, args: &[&str], input: &[u8]) -> (Option<i32>, Vec<u8>) {
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
    let mut stdout = child.stdout.take().expect("stdout pipe");
    let mut byte = [0u8; 1];
    let _ = stdout.read(&mut byte);
    drop(stdout);
    let output = child
        .wait_with_output()
        .unwrap_or_else(|err| panic!("wait for {program}: {err}"));
    (output.status.signal(), output.stderr)
}

#[test]
fn values_match_jq() {
    require_jq();
    let programs = [
        "0",
        "1",
        "-1",
        "--1",
        "---1",
        "- 1",
        "- -1",
        "-(1)",
        "1.5",
        "1.",
        ".5",
        "-.5",
        "01",
        "1e2",
        "1E+2",
        "1e-1",
        "00.10",
        "1.2300",
        "true",
        "false",
        "null",
        "\"hi\"",
        "\"\"",
        "\"a\\nb\"",
        "\"\\u0041\"",
        "\"caf\\u00e9\"",
        "\"say \\\"hi\\\"\"",
        "\"a\\/b\"",
        "\"\\\\(1)\"",
        "\"x\\(.)y\"",
        "\"\\(1)\"",
        "\"\\(true)\"",
        "\"\\(null)\"",
        "\"\\(.a)\"",
        "\"a\\(1,2)b\"",
        "\"\\(\"hi\")\"",
        "\"\\(.a,.b)\"",
        "\"\\({a:1})\"",
        "\"\\([1,2])\"",
        "\"a\\(1)b\\(2)\"",
        "[]",
        "[1]",
        "[1,2,3]",
        "[1,\"a\",true,null,.]",
        "[.a,.b]",
        "[[1],[2]]",
        "[(1,2)]",
        "[1,2]",
        "[1|2]",
        "{}",
        "{a:1}",
        "{a:.a}",
        "{a}",
        "{a,b}",
        "{\"a\":1}",
        "{\"a b\":1}",
        "{\"a\"}",
        "{\"a\",b}",
        "{\"a\\(1)\"}",
        "{(.a):1}",
        "{a:1,b:2}",
        "{a:1,b}",
        "{a:(1,2)}",
        "{(\"a\",\"b\"):1}",
        "{a:1,a:2}",
        "{a:1,b:2,a:3}",
        "{if:1,and:2,or:3,not:4}",
        "{null:null}",
        "{true:false}",
        "{empty:1}",
        "{empty}",
        "{\"\":1}",
        "{(\"\"):1}",
        "{a:1,}",
        "{a,}",
        "{a: {b: 1}}",
        "{a:1|2}",
        "{foo::bar:1}",
        "1 # hi",
        "# c\n1",
        "\"a#b\"",
        "{a:1, # c\nb:2}",
        "[1, #c\n2]",
        "{ #c\na: 1 }",
        "\"\\(.a)x\\(.b)\"",
    ];
    for program in programs {
        compare_exact(
            &format!("-n {program}"),
            &["-n", "-c", "--", program],
            b"not json",
        );
    }
    compare_exact("pretty object", &["-n", "--", "{a:1,b:2}"], b"");
    compare_exact("pretty array", &["-n", "--", "[1,2]"], b"");
    compare_exact("raw string", &["-n", "-r", "--", "\"hi\""], b"");
    compare_exact("join string", &["-n", "-j", "--", "\"hi\""], b"");
    compare_exact("cluster nc", &["-nc", "--", "1"], b"ignored");
    compare_exact("long null input", &["--null-input", "-c", "--", "1"], b"");
    compare_exact("n after program", &[".", "-n"], b"{\"z\":1}");
    compare_exact("n dot ignores stdin", &["-n", "-c", "."], b"{\"z\":1}");
    compare_exact(
        "n ignores missing file",
        &["-n", ".", "no-such-file.json"],
        b"",
    );
    compare_exact("n alone", &["-n"], b"{\"z\":1}");

    let input = br#"{"a":1,"b":2,"c":3,"a b":4,"a$2":9,"d":"c","":7,"x":[10,20]}"#;
    let with_input = [
        "\"x\\(.)y\"",
        "\"\\(.a)\"",
        "\"\\(.a,.b)\"",
        "{a}",
        "{a,b}",
        "{\"a\"}",
        "{a,b,(.d):.a,e:.b}",
        "{(.a):.b}",
        "[.a,.b]",
        "[(.a,.b)]",
        "{a:(1,2)}",
        "{(\"a\",\"b\"):1}",
        "{\"a\\(1)\":.b}",
        "{a:1,b:.b}",
        "\"\\(.x)\"",
        ".\"a\\(1)\"",
        ".[\"a\\(1,2)\"]",
        ".a[.d]",
        "(.a,.b)[\"a\",\"b\"]",
        "{a:(1,2),b:(3,4)}",
        "{(.a,.b):1}",
        "{\"\\(1,2)\"}",
        "{a:(1,.b)}",
        "1, [2, .a]",
    ];
    for program in with_input {
        compare_exact(program, &["-c", "--", program], input);
    }

    let bad_strings = [
        "\"\\q\"",
        "\"\\v\"",
        "\"\\u12\"",
        "\"\\uXXXX\"",
        "\"\\uD800\"",
        "\"ok\\q\"",
        "  \"\\q\"",
        "{a:\"\\q\"}",
        "\"\\u12zz\"",
        "\"\\uD800\\uZZZZ\"",
    ];
    for program in bad_strings {
        compare_exact(
            &format!("string {program}"),
            &["-n", "-c", "--", program],
            b"",
        );
    }
    let bad_keys = [
        "{(0):1}",
        "{(1):1}",
        "{([]):1}",
        "{({}):1}",
        "{(null):1}",
        "{(true):1}",
        "{(false):1}",
        "{(1.50):1}",
        "{(1e2):1}",
        "{(01):1}",
        "{([1,2]):1}",
        "{({a:1,b:[2]}):1}",
        "{(1|.):1}",
        "{(.|1):2}",
        "{(1|.|.):2}",
        "{\n(0):1}",
        "#c\n{(1):1}",
    ];
    for program in bad_keys {
        compare_exact(&format!("key {program}"), &["-n", "-c", "--", program], b"");
    }

    let runtime = [
        ("{(.):1}", "1\n"),
        ("{(.):1}", "\"k\"\n"),
        ("{(.):1}", "null\n"),
        ("{(.):1}", "true\n"),
        ("{(.):1}", "false\n"),
        ("{(.):1}", "[]\n"),
        ("{(.):1}", "{}\n"),
        ("{(.):1}", "1.50\n"),
        ("{(.):1}", "{\"a\":1}\n"),
        ("{(.):1}", "{\"b\":1,\"a\":2}\n"),
        ("{a:.a}", "1\n"),
        ("[.a]", "1\n"),
        ("[1,.a]", "1\n"),
        ("{a:1,b:.b}", "1\n"),
        ("\"\\(.a)\"", "1\n"),
        ("\"pre\\(1,.a)post\"", "1\n"),
        ("\"\\(1,.a)\"", "1\n"),
        ("{(\"a\",\"b\"):.a}", "{\"a\":1}\n"),
        ("{a:(1,.b)}", "1\n"),
        ("{a:(1,.b), c:3}", "1\n"),
        ("{c:3, a:(1,.b)}", "1\n"),
        ("{a:(1,2), b:(3,.x)}", "1\n"),
        ("{a:(1,2), b:(3,.x)}", "{\"x\":9}\n"),
        ("{(1,2):1}", "null\n"),
        ("{(1|2):1}", "null\n"),
        ("{([1, .]):1}", "2\n"),
        ("-n .[]", ""),
    ];
    for (program, input) in runtime {
        if program == "-n .[]" {
            compare_exact("n iterate", &["-n", ".[]"], b"");
            continue;
        }
        compare_exact(
            &format!("rt {program} {input}"),
            &["-c", "--", program],
            input.as_bytes(),
        );
    }
    compare_exact("n bad key", &["-n", "-c", "{(.):1}"], b"");
    compare_exact("n bad key pretty location", &["-n", "{(.a):1}"], b"");
}

#[test]
fn leading_minus_program_matches_jq() {
    require_jq();
    let object = br#"{"a":1,"foo":2}"#;
    let cases: &[(&str, &[&str], &[u8])] = &[
        ("bare -1", &["-1"], b"null\n"),
        ("bare -.a", &["-.a"], object),
        ("bare -.", &["-."], b"5\n"),
        ("bare -.5", &["-n", "-.5"], b""),
        ("bare -0", &["-n", "-0"], b""),
        ("null input -1", &["-n", "-1"], b""),
        ("compact before", &["-c", "-1,[2]"], b"1\n"),
        ("compact after", &["-1,[2]", "-c"], b"1\n"),
        ("flags then program", &["-nc", "-1"], b""),
        (
            "indent value stays indent",
            &["--indent", "-1", "."],
            object,
        ),
        ("end of options", &["-n", "--", "-1"], b""),
        ("later minus is a file", &["-1", "-2"], b""),
        ("later dotted minus is a file", &["-.a", "-.b"], object),
    ];
    for (name, args, input) in cases {
        compare_exact(name, args, input);
    }
}

#[test]
fn operators_match_jq() {
    require_jq();
    let programs = [
        "1+2*3",
        "1+2*3+4",
        "8/2/2",
        "8/2*2",
        "1-2-3",
        "-1-2",
        "-1+2",
        "-1*2",
        "2*-3",
        "1--2",
        "-(1+2)",
        "-2*-3",
        "2/-3*4",
        "2/-3/4",
        "7%-2*3",
        "7%2*3",
        "8/2/-2",
        "--1",
        "---1",
        "1---2",
        "-0",
        "-(-0)",
        "0*-1",
        "-1*0",
        "0-0",
        "(-0)-0",
        "0-(-0)",
        "(-0)+(-0)",
        "1/3",
        "0.1+0.2",
        "1.2300+0",
        "1.50*1",
        "4/2",
        "5/2",
        "1e20+1",
        "1e16+0",
        "1.1e16+0",
        "1e15+0",
        "1e-4+0",
        "1e-5+0",
        "1e-7+0",
        "1e-12+0",
        "1e308*10",
        "-1e308*10",
        "12345678901234567+0",
        "-0/1",
        "0/-1",
        "1e-19 + 1e-20 - 5e-21",
        "1 / 1e-17",
        "1 + 2 * 2 + 10 / 2",
        "[16 / 4 / 2, 16 / 4 * 2, 16 - 4 - 2, 16 - 4 + 2]",
        "5%2",
        "(-5)%2",
        "5%-2",
        "5.5%2",
        "(-5.5)%2",
        "1%0.5",
        "(-8)%3",
        "(-4)%2",
        "25%7",
        "49732%472",
        "1e20%3",
        "(-1e19)%3",
        "(9223372036854775808)%10",
        "[(infinite, -infinite) % (1, -1, infinite)]",
        "(infinite)%(-infinite)",
        "(-infinite)%infinite",
        "infinite==infinite",
        "infinite==1.7976931348623157e+308",
        "infinite>1.7976931348623157e+308",
        "-infinite<-1.7976931348623157e+308",
        "1e999999999==infinite",
        "1e999999999>1",
        "5E500000000 > 5E-5000000000",
        "nan==nan",
        "nan<nan",
        "nan<1",
        "1>nan",
        "1<nan",
        "-(nan)",
        "nan%0",
        "0/nan",
        "nan/0",
        "1/0",
        "-1/0",
        "(-1)/0",
        "1/-0",
        "1%0",
        "1%-0",
        "1.50/0",
        "null+1",
        "1+null",
        "null+\"a\"",
        "\"a\"+null",
        "null+[1]",
        "[1]+null",
        "null+{}",
        "null+true",
        "null+null",
        "\"ab\"*3",
        "\"ab\"*2.9",
        "\"ab\"*0.9",
        "\"ab\"*-1",
        "\"a\"*-0",
        "3*\"ab\"",
        "\"a\"*nan",
        "\"é\"*2",
        "\"\"/\"\"",
        "\"ab\"/\"\"",
        "\"éé\"/\"\"",
        "\"aaa\"/\"aa\"",
        "\"a,,b,\"/\",\"",
        "\"abc\"/\"z\"",
        "[1,2,1]-[1]",
        "[1,1.0]-[1]",
        "[nan]-[nan]",
        "\"ab\"+\"cd\"",
        "[1]+[2,3]",
        "{\"b\":1,\"a\":2}+{\"a\":3,\"c\":4}",
        "{\"a\":{\"b\":1,\"c\":2},\"d\":3}*{\"a\":{\"c\":4,\"e\":5},\"f\":6}",
        "{\"a\":{\"b\":1}}*{\"a\":2}",
        "{\"a\":1}*{\"a\":{\"b\":2}}",
        "{\"a\":[1]}*{\"a\":[2]}",
        "{\"b\":1,\"a\":{\"x\":1}}*{\"a\":{\"y\":2},\"c\":3}",
        "null<false",
        "false<true",
        "true<0",
        "0<\"\"",
        "\"\"<[]",
        "[]<{}",
        "1==1.0",
        "-0==0",
        "-0<0",
        "1.50==1.5",
        "\"b\"<\"aa\"",
        "[1]<[1,2]",
        "[1,3]<[1,2]",
        "[1]==[1.0]",
        "{}<{\"a\":1}",
        "{\"a\":1,\"b\":1}<{\"a\":2}",
        "{\"a\":1}<{\"a\":1,\"b\":1}",
        "{\"a\":1,\"c\":1}<{\"a\":1,\"b\":1}",
        "{\"b\":1,\"a\":2}=={\"a\":2,\"b\":1}",
        "{\"a\":2,\"b\":1}<{\"a\":1,\"b\":9}",
        "{\"a\":nan}<{\"a\":nan}",
        "[nan]<[0]",
        "1 and 2",
        "0 and 1",
        "false and 1",
        "null and 1",
        "\"\" and 1",
        "[] and 1",
        "false or 0",
        "null or null",
        "1 and (1,null,false)",
        "false or (1,null,0)",
        "(1,null) and (2,3)",
        "(1,2) and (3,4)",
        "(1,null,false,0)//\"x\"",
        "(null,false)//\"x\"",
        "(null,false)//(1,2)",
        "0//1",
        "\"\"//1",
        "null//null//1",
        "false//1//2",
        "1, 2 and 3",
        "1 and 2, 3",
        "1 // 2 and 3",
        "1 and 2 // 3",
        "null, false // \"x\"",
        "1<2 and 3>1",
        "if false then 1 end",
        "if true then 1 end",
        "if null then 1 else 2 end",
        "if 0 then 1 else 2 end",
        "if (false,true,null,0) then \"t\" else \"f\" end",
        "if (true,true) then (1,2) else 3 end",
        "if false then 1 elif true then 2 else 3 end",
        "if false then 1 elif false then 2 end",
        "if false then 1 elif false then 2 elif true then 3 else 4 end",
        "1 + if true then 2 else 3 end",
        "if true then 1 else 2 end + 3",
        "not",
        "0|not",
        "not|not",
        "if not then 1 else 2 end",
        "1 and not",
        "\"\\(1+2)\"",
        "1 + # hi\n 2",
        "[1+2]",
        "{a:(1+2)}",
        "{a:-1}",
        "{a:-1.50}",
        "1+\"a\"",
        "\"a\"+1",
        "true+false",
        "[]*2",
        "\"a\"/1",
        "\"a\"%\"b\"",
        "-true",
        "-\"a\"",
        "-{}",
        "{}-{}",
        "null-1",
        "-\"ab\"*2",
        "-2*\"a\"",
        "\"abcdefghijklmno\"+1",
        "\"éééééééé\"+1",
        "\"🙂🙂🙂🙂\"+1",
        "infinite/0",
        "{(1+2):1}",
        "{(1-2):1}",
        "{(2*3):1}",
        "{(1/2):1}",
        "{(1<2):1}",
        "{(1==1):1}",
        "{([1]+[2]):1}",
        "{(\"a\"+\"b\"):1}",
        "{(-(1)):1}",
        "{(-1):1}",
        "{(true and false):1}",
        "{(null // 1):1}",
        "{(if true then 1 else 2 end):1}",
        "{(1/0):1}",
        "{(1%0):1}",
        "{(1+\"a\"):1}",
        "{(null+1):1}",
        "{(null+\"a\"):1}",
        "(1,2)-(3,4)",
        "(1,\"a\",2)+3",
        "(1,2)+(3,\"a\")",
        "1or 2",
        "1and 2",
        "13911860366432393 == 13911860366432392",
        "13911860366432393 > 13911860366432392",
        "(13911860366432393 + 0) == 13911860366432393",
        "[13911860366432393] - [13911860366432392]",
        "1 == 1.0",
        "1.50 == 1.5",
        "1e2 == 100",
        "-0 == 0",
        "(0.1+0.2) > 0.3",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
    let with_input = [
        (".+1", "1\n"),
        (".+1", "\"a\"\n"),
        ("-.", "5\n"),
        ("(1,.a)-(3,4)", "1\n"),
        ("(1,2)-(3,.a)", "1\n"),
        ("(1,.a)-(2,.b)", "1\n"),
        ("(1,.a)//0", "1\n"),
        ("(null,.a)//5", "1\n"),
        ("(null,1,.a)//0", "1\n"),
        ("1//.a", "1\n"),
        ("false//.a", "1\n"),
        ("false and .a", "1\n"),
        ("1 or .a", "1\n"),
        ("0 or .a", "1\n"),
        ("null or .a", "1\n"),
        ("(1,.a) and 2", "1\n"),
        ("(null,1) and .a", "1\n"),
        ("(1,null) or .a", "1\n"),
        ("if false then .a else 2 end", "1\n"),
        ("if true then 1 else .a end", "1\n"),
        ("if (true,.a) then 1 else 2 end", "1\n"),
        ("if (false,.a) then .b else 1 end", "1\n"),
        ("if true then 1 elif .a then 2 else 3 end", "1\n"),
        ("if false then .a elif true then 2 else .b end", "1\n"),
        ("(1,.a)|.+1", "1\n"),
        ("[.[] * 3]", "[\"a\",\"ab\",\"abc\"]\n"),
        ("[.[] * \"abc\"]", "[-1.0,-0.5,0.0,0.5,1.0,1.5,3.7,10]\n"),
        ("[. * (nan,-nan)]", "\"abc\"\n"),
        ("[.[] / \",\"]", "[\"a,b\",\"c,d\"]\n"),
        ("[.[] / \", \"]", "[\"a, b\",\"c, d\"]\n"),
        ("(1,null,false,0,.[])|not", "null\n"),
        ("(.[] )//(1,2)", "[]\n"),
        ("false // (.[] ) // 1", "[]\n"),
        ("1 and not", "1\n"),
        ("[if false then 3 end]", "7\n"),
        ("[if false then 3 elif false then 4 end]", "7\n"),
        ("[if true then 3 end]", "7\n"),
        ("[if false then 3 else . end]", "7\n"),
        (". == 13911860366432392", "13911860366432393\n"),
        ("(.+0) == .", "13911860366432393\n"),
    ];
    for (program, input) in with_input {
        compare_exact(program, &["-c", "--", program], input.as_bytes());
    }
}

#[test]
fn operator_syntax_stays_generic() {
    require_jq();
    let bin = env!("CARGO_BIN_EXE_tumulus");
    for program in [
        "1<2<3", "1==1==1", "{a:1+2}", "not true", "1++2", ".a=.b=1", ".a|=.|=.", ".a+=1+=2",
    ] {
        let args: Vec<String> = ["-n", "-c", "--", program]
            .into_iter()
            .map(str::to_string)
            .collect();
        let got = run(bin, &args, b"");
        assert!(
            got.stdout.is_empty(),
            "{program}: stdout {}",
            String::from_utf8_lossy(&got.stdout)
        );
        assert_eq!(
            got.stderr,
            b"tumulus: syntax error\n",
            "{program}: stderr {}",
            String::from_utf8_lossy(&got.stderr)
        );
        assert_eq!(got.status.code(), Some(3), "{program}");
    }
}

#[test]
fn updates_match_jq() {
    require_jq();
    let programs = [
        "null | .a = 1",
        "null | .a.b = 1",
        "null | .[0] = 1",
        "null | .a[0].b = 1",
        "null | .[2].a = 1",
        "null | .[1:2] = [9]",
        "null | .[1:3] = []",
        "null | .a += 1",
        "null | .a //= 1",
        "null | .a |= empty",
        "null | del(.a)",
        "null | .[]? = 1",
        "null | .[1:3] |= empty",
        "{} | .a.b = 1",
        "{} | .a.b += 1",
        "{} | .a -= 1",
        "{\"a\":1,\"b\":2} | .a = .b",
        "{\"a\":1} | .a = (1,2)",
        "{\"a\":1,\"b\":2} | (.a,.b) = 9",
        "{\"a\":1,\"b\":2} | (.a,.b) = (7,8)",
        "{\"a\":1,\"b\":2} | .a, .b = 9",
        "{\"a\":1} | .b.c = 4",
        "{\"a\":null} | .a.b = 4",
        "{\"a\":1} | .a.b = 4",
        "\"abc\" | .a = 1",
        "\"abc\" | .[0] = \"Z\"",
        "1 | .a = 2",
        "1 | .[0] = 2",
        "true | .a = 1",
        "true | .[0] += 1",
        "[1,2] | .a = 1",
        "[1,2,3] | .[1] = 9",
        "[1,2,3] | .[-1] = 9",
        "[1,2,3] | .[5] = 9",
        "[] | .[-1] = 9",
        "[1,2] | .[-3] = 1",
        "[1,2,3] | .[1.9] = 8",
        "[1,2,3] | .[-1.2] = 8",
        "[1,2,3,4] | .[1:3] = [9]",
        "[1,2,3,4] | .[1:3] = [8,8,8]",
        "[1,2,3,4] | .[1:3] = []",
        "[1,2,3,4] | .[1:3] = 9",
        "[1,2] | .[0:0] = [9,9]",
        "[1,2,3] | .[1:1] = [9]",
        "[1,2,3] | .[] = 0",
        "[1,2,3] | .[] = (7,8)",
        "[] | .[] = 1",
        "{\"b\":1,\"a\":2} | .[] = 0",
        "{\"a\":1} | .[\"b\",\"c\"] = 4",
        "{\"a\":1} | .[0] = 2",
        "[1,2,3] | .[0,2] = 9",
        "{\"a\":{\"b\":1}} | (.a.b, .a) = 5",
        "{\"a\":{\"b\":1}} | (.a, .a.b) = 5",
        "[1,2,3] | (.[0], .[1]) = .[2]",
        "[1,2,3] | (.[0], .[2]) = .[0]",
        "{\"a\":{\"b\":1,\"c\":2},\"d\":3} | .a | .b = 1",
        "{\"a\":{\"b\":1,\"c\":2}} | (.a | .b) = 9",
        "[{\"a\":0},{\"a\":1}] | (.[] | .a) = 7",
        ".a = 1 + 2",
        ".a = 1, 2",
        ".a = .b // .c",
        "1 // .a = 2",
        "{\"b\":false,\"c\":3} | .a = .b // .c",
        "1 = 2",
        "(1+2) = 3",
        ".a + 1 = 2",
        "length = 1",
        "empty = 1",
        "empty = empty",
        "[1,2,3] | empty = 9",
        "[1] | .a = empty",
        "del(1)",
        "(.a, 1) = 2",
        "(1,2) = 3",
        "{\"a\":1} | .a |= .+1",
        "{\"a\":1,\"b\":2} | .a |= . + .b",
        "{\"a\":1,\"b\":2} | .a += .b",
        "{\"a\":{\"b\":1},\"b\":9} | .a.b += .b",
        "{\"a\":1} | .a += (1,10)",
        "{\"a\":1} | .a |= (1,2,3)",
        "{\"a\":1} | .a |= (1, .b)",
        "[1,2,3] | .[] += 1",
        "[1,2,3] | .[] |= .+1",
        "{\"b\":1,\"a\":2} | .[] |= .+1",
        "[0,1,2,3,4,5] | .[] |= select(. % 2 == 0)",
        "[1,2] | .[] |= (., .+10)",
        "{\"a\":1,\"b\":2} | (.a,.b) |= .+1",
        "{\"a\":1} | (.a,.a) |= .+1",
        "{\"a\":1} | (.a,.a) += 1",
        "[10] | (.[0], .[0]) |= .+1",
        "[1,2,3] | (.[0], .[0]) += 1",
        "[1,2,3,4] | (.[0], .[0]) |= empty",
        "[1,2,3,4] | .[0,1] |= empty",
        "[1,2,3,4] | .[1:3] |= . + [8]",
        "[1,2,3,4] | .[1:3] |= [.[] + 1]",
        "[1,2,3,4] | .[1:3] |= 9",
        "[1,2,3,4] | .[1:3] |= empty",
        "[1,5,3,0,7] | (.[] | select(. >= 2)) |= empty",
        "[1,5,3,0,7] | (.[] | select(. >= 2)) = 9",
        "[1,null,2] | .[] |= select(. != null)",
        "{\"a\":1,\"b\":0,\"c\":2} | .[] |= select(. > 0)",
        ". |= 5",
        "[1,2] | . |= . + [3]",
        ". |= empty",
        "{\"a\":1} | . |= empty",
        "{\"a\":1} | .a |= empty",
        "{\"a\":1,\"b\":2} | .a = empty",
        "[1,2,3,4] | del(.[1])",
        "[1,2,3,4] | del(.[1:3])",
        "[1,2,3,4] | del(.[1], .[3])",
        "[1,2,3,4] | del(.[3], .[1])",
        "[1,2,3,4] | del(.[0], .[0])",
        "[1,2,3,4] | del(.[-1], .[3])",
        "[1,2,3,4] | del(.[-1], .[0])",
        "[1,2,3,4] | .[1,-1] |= empty",
        "[1,2,3,4] | .[-1,-1] = 9",
        "[1,2,3] | del(.[-5])",
        "[1,2] | .[-5] |= empty",
        "[1,2,3,4] | del(.[10])",
        "del(.[2:4],.[0],.[-2:])",
        "del(.[1], .[-6], .[2], .[-3:9])",
        "{\"a\":1,\"b\":2,\"c\":3} | del(.b)",
        "{\"a\":1,\"b\":2,\"c\":3} | del(.a,.c)",
        "{\"a\":1,\"b\":[1,2,3]} | del(.b[1])",
        "[1,2,3] | del(.[] )",
        "{\"a\":1,\"b\":2} | del(.[] )",
        "{\"a\":1} | del(.)",
        "1 | del(.)",
        "[1,2,3] | del(empty)",
        "{\"a\":1} | del(.b)",
        "{\"a\":1} | del(.a.b)",
        "{\"a\":1} | del(.a.b?)",
        "\"abcd\" | del(.[1:3])",
        "\"abcd\" | del(.[0])",
        "\"abcd\" | .[1:3] = \"XY\"",
        "\"abcd\" | .[1:3] |= empty",
        "\"abcd\" | .[1:3] |= .",
        "{\"a\":\"x\"} | .a += 1",
        "{\"a\":[1]} | .a += [2]",
        "{\"a\":{\"b\":1}} | .a += {\"c\":2}",
        "{\"a\":1} | .a -= 2",
        "{\"a\":[1,2,3]} | .a -= [2]",
        "{\"a\":2} | .a *= 3",
        "{\"a\":\"ab\"} | .a *= 2",
        "{\"a\":6} | .a /= 2",
        "{\"a\":\"a,b,c\"} | .a /= \",\"",
        "{\"a\":7} | .a %= 3",
        "{\"a\":\"x\"} | .a *= 2",
        "{\"a\":\"x\"} | .a /= 2",
        "{\"a\":\"x\"} | .a %= 2",
        "{\"a\":1} | .a /= 0",
        "{\"a\":1} | .a %= 0",
        "{\"a\":true} | .a += 1",
        "{\"a\":{\"b\":1}} | .a *= {\"b\":2,\"c\":3}",
        "{\"a\":null} | .a //= 5",
        "{\"a\":false} | .a //= 5",
        "{\"a\":0} | .a //= 5",
        "{\"a\":\"\"} | .a //= 5",
        "{\"a\":1} | .a //= 5",
        "{\"a\":null,\"b\":9} | .a //= .b",
        "{\"a\":null,\"b\":9} | .a |= . // .b",
        "{\"a\":null} | .a //= (5,6)",
        "{\"a\":1} | .a //= (5,6)",
        "{\"a\":false} | .a //= (5,6)",
        "{\"a\":null,\"b\":false,\"c\":4} | .a //= .b // .c",
        ".a //= .b // .c",
        ".a += 1 + 2",
        ".a *= 1+2",
        "{\"a\":1} | .b += 1",
        "{\"a\":1} | .a.b.c += 1",
        "[null] | .[0].a = 1",
        "[1] | .[0].a = 1",
        "{\"a\":[]} | .a[0] = 1",
        "{\"a\":[]} | .a[-1] = 1",
        ".foo[2].bar = 1",
        ".foo[1,4,2,3] |= empty",
        "1 as $x | .a = $x",
        "\"a\" as $k | .[$k] = 7",
        "{\"k\":\"a\",\"a\":1} | .[.k] = 9",
        "{\"n\":2,\"a\":[0,0,0]} | .a[.n] = 9",
        ".a.b? = 1",
        "{\"a\":1} | .a.b? = 2",
        "{\"a\":{\"b\":1}} | .a.b? = 2",
        "1 | .a? = 2",
        "{\"a\":\"x\"} | .a.b? = 1",
        "\"x\" | .a.b? = 1",
        "{\"a\":1} | .b.c? = 2",
        "{\"a\":1,\"b\":{}} | (.a.c?, .b.d) = 2",
        "[1,2,3] | .[1]? = 9",
        "\"abc\" | .[0]? = \"Z\"",
        "select(.>1) = 9",
        "{\"a\":1} | (.a |= .+1) | .a",
        "{\"a\":1,\"b\":2} | .a = .b | .b = 0",
        "null | .z = 1 | .a = 2",
        "{\"a\":{\"b\":1}} | .. = 1",
        "{\"a\":1,\"b\":[2,{\"c\":3}]} | .. |= .",
        "[1,2,3] | .[5] |= .+1",
        "[1,2] | .[-5] |= .+1",
        "[1,2] | .[-5] += 1",
        "1 | .[0:1] = [2]",
        "[1,2,3,4] | .[1:3][0] = 9",
        "{\"a\":1} | .a += (1, \"x\")",
        "{\"a\":1,\"b\":2} | (.a // .b) = 9",
        "{\"a\":null,\"b\":2} | (.a // .b) = 9",
        "{\"a\":false,\"b\":2} | (.a // .b) = 9",
        "{\"a\":0,\"b\":2} | (.a // .b) = 9",
        "{\"a\":\"\",\"b\":2} | (.a // .b) = 9",
        "{\"b\":2} | (.a // .b) = 9",
        "{\"a\":1,\"b\":2} | (if .a then .a else .b end) = 9",
        "{\"a\":null,\"b\":2} | (if .a then .a else .b end) = 9",
        "{\"b\":2} | (if .a then .a else .b end) = 9",
        "{} | (if .a then .a end) = 9",
        "{\"a\":false} | (if .a then .b end) = 9",
        "{\"a\":1,\"b\":2} | (if .a then .a elif .b then .b else .c end) = 9",
        "{\"b\":2} | (if .a then .a elif .b then .b else .c end) = 9",
        "{} | (if .a then .a elif .b then .b end) = 9",
        "{\"a\":1,\"b\":2} | del(if .a then .a else .b end)",
        "{\"a\":[1,2,3]} | (if false then .b else .a end)[] = 9",
        "(if empty then .a else .b end) = 9",
        "{\"a\":1,\"b\":2} | (if (true,false) then .a else .b end) = 9",
        "{\"a\":1} | (if .a then .a else (1/0) end) = 9",
        "{\"a\":null,\"b\":2} | (if .a then .a else (1/0) end) = 9",
        "{\"a\":1,\"b\":2} | (if (.a, 1/0) then .a else .b end) = 9",
        "false | (if true then false else 1 end) = 9",
        "{\"a\":false} | (if .a then .b else false end) = 9",
        "{\"a\":false} | (if .a then .b else .a end | false) = 9",
        "{\"a\":1} | (.a // (1/0)) = 9",
        "{\"a\":null} | (.a // (1/0)) = 9",
        "{\"a\":1,\"b\":2,\"c\":3} | ((.a,.b)//.c) = 9",
        "{\"a\":null,\"b\":false,\"c\":3} | ((.a,.b)//.c) = 9",
        "{\"a\":1,\"b\":null,\"c\":3} | ((.a,.b)//.c) = 9",
        "{\"a\":1} | ((.a, .[0]) // .b) = 9",
        "{\"a\":false,\"b\":4} | ((.a, .[0]) // .b) = 9",
        "{\"a\":1,\"b\":2} | ((.a, .[0]) // .b)? = 9",
        "{\"a\":false,\"b\":7} | ((.a, .[0]) // .b)? = 7",
        "{\"a\":1} | (empty // .a) = 9",
        "{\"a\":1} | (.a // empty) = 9",
        "{\"a\":null} | (.a // empty) = 9",
        "1 | (.a? // .b?) = 9",
        "{\"a\":1,\"b\":[1,2]} | (.a[0]? // .b[0]) = 9",
        "{\"a\":null,\"b\":[1,2,3]} | (.a[0] // .b[1]) = 9",
        "{\"a\":[1,2]} | (.a[0] // .b[1]) = 9",
        "{\"a\":1} | (.a // .b).c = 9",
        "{\"a\":null,\"b\":{\"c\":1}} | (.a // .b).c = 9",
        "[null,2,3] | (.[0] // .[1]) = 9",
        "[null,2,3] | (.[0] // .[1]) += 1",
        "{\"a\":1,\"b\":2} | (.a // .b) += (1,10)",
        "{\"a\":null,\"b\":2} | (.a // .b) += (1,10)",
        "{\"a\":1,\"b\":2} | ((.a // .b), .b) = 8",
        "{\"a\":1,\"b\":2} | (.a // .b) |= .+1",
        "{\"a\":null,\"b\":2} | (.a // .b) |= .+1",
        "{\"a\":null,\"b\":false} | (.a // .b) //= 5",
        "{\"a\":1,\"b\":false} | (.a // .b) //= 5",
        "{\"a\":1,\"b\":2} | del(.a // .b)",
        "{\"a\":null,\"b\":2} | del(.a // .b)",
        "{\"a\":false,\"b\":2,\"c\":3} | del((.a // .b), .c)",
        "null = 9",
        "false = 9",
        "true = 9",
        "0 = 9",
        "null | (null | .a) = 9",
        "null | (null | .[0]) = 9",
        "{\"a\":false} | (.a | false) = 9",
        "{\"a\":null} | (.a | null) = 9",
        "{\"a\":false} | (.a | false | .b) = 9",
        "{\"a\":1} | (.a | 1 | false | .c) = 9",
        "{\"a\":false} | (.a | 1 | false | .c) = 9",
        "{\"a\":false} | (.a | not | not) = 9",
        "true | (not | not) = 9",
        "false | (not | not) = 9",
        "null | (. | not | not) = 9",
        "false | not = 9",
        "1 | not = 9",
        "{\"a\":false} | (.a | not) = 9",
        "1 | .. = 9",
        "(1 | ..) = 9",
        "(1 | .[]) = 9",
        "{\"a\":1} | (.a | ..) = 9",
        "{\"a\":1} | (.. | .a?) = 9",
        "[1,{\"a\":2}] | (.. | .a?) = 9",
        "false | ((false | true) and .a) = 9",
        "false | ((false | true) | .a) = 9",
        "false | ((false | false) and .a) = 9",
        "false | ((false | false) | .a) = 9",
        "{\"a\":false} | ((false | true) and .a) = 9",
        "{\"a\":1} | ((.a | true) and .b) = 9",
        "{\"a\":1} | ((.a | true) | .b) = 9",
        "{\"a\":false} | ((.a | false) and .b) = 9",
        "{\"a\":false} | (.a and .b) = 9",
        "{\"a\":false} | (.a and true) = 9",
        "{\"a\":null} | (.a and .b) = 9",
        "{\"a\":1} | (.a and .b) = 9",
        "{\"a\":{\"b\":false}} | (.a.b and .c) = 9",
        "{\"a\":false,\"b\":false,\"c\":1} | (.a and .b and .c) = 9",
        "1 | (1 and .a) = 9",
        "{\"a\":1} | ((1 | true) and .a) = 9",
        "1 | ((1 | true) and .a) = 9",
        "{\"a\":true} | (.a or .b) = 9",
        "{\"a\":true} | (.a or false) = 9",
        "{\"a\":0} | (.a or .b) = 9",
        "{\"a\":\"\"} | (.a or .b) = 9",
        "{\"a\":[]} | (.a or .b) = 9",
        "{\"a\":false} | (.a or false) = 9",
        "{\"a\":false,\"b\":1} | (.a or .b) = 9",
        "{\"a\":false,\"b\":false,\"c\":1} | (.a or .b or .c) = 9",
        "false | (false or .a) = 9",
        "{\"a\":1} | (false or .a) = 9",
        "{\"a\":true} | (false or .a) = 9",
        "{\"a\":false} | (false or .a) = 9",
        "{\"a\":false} | ((.a and .b)?) = 9",
        "{\"a\":1} | ((.a and .b)?) = 9",
        "{\"a\":false} | ((.a and .[])?) = 9",
        "{\"a\":1,\"b\":2} | ((.a,.b) and .c) = 9",
        "{\"a\":false,\"b\":2} | ((.a,.b) and .c) = 9",
        "{\"a\":true,\"b\":true} | (select(.a) and .b) = 9",
        "{\"a\":1} | (select(.a) and .b) = 9",
        "select(true,true) and .a = 9",
        "{\"a\":true,\"b\":true} | (select(true,true) and .b) = 9",
        "{\"a\":1} | (map(.+1)[]) = 9",
        "(1, .[0]) = 9",
        "((.a, .[0])?) = 9",
        "1 | (1 | .a)? = 9",
        ".[1/0]? = 9",
        "(.[1/0])? = 9",
        "1 | .a[0]? = 9",
        "{\"a\":1} | (.a[0]?) = 9",
        ".[\"a\",(1/0)]? = 9",
        "(.[\"a\",(1/0)])? = 9",
        "{\"a\":1} | .[\"no\",\"a\"]? = 9",
        "[0,1,2] | .[1,\"a\",2]? = 9",
        "[0,1,2,3,4] | (.[1:3] | .[0]) = 9",
        "[0,1,2,3,4] | (.[1:3][0]) = 9",
        "\"abcd\" | (.[1:3] | .[0]) = 9",
        "null | (.[1:2] | .[0]) = 9",
        "[1,2,3,4] | .[{\"start\":1,\"end\":3}] = [9]",
        "[1,2,3,4] | .[{\"start\":1,\"end\":null}] = [9]",
        "[1,2,3,4] | .[{\"start\":null,\"end\":2}] = [9]",
        "null | .[{\"start\":1,\"end\":2}] = [9]",
        "[1,2,3,4] | .[{\"start\":1}] = [9]",
        "null | .[{}] = 9",
        "[1,2] | .[[]] = 9",
        "1 | .[true] = 9",
        "{\"a\":1} | (.a and .[1e20]) = 9",
        "{\"a\":1} | (.a and .[\"€€€€€\"]) = 9",
        "{\"a\":1} | (.a and .[\"kkkkkkkkkkkkk\"]) = 9",
        "[1,2,3] | (.[0] and .[1:3]) = 9",
        "{\"k\":\"a\",\"a\":1} | (.k as $x | .[$x]) = 9",
        "{\"a\":[1,2],\"b\":10} | (.a[] as $x | .b) |= .+1",
        "[1,2,3] | (. as $x | .[1]) = 9",
        "[1,2,3] | (.[] as $x | .[0]) = 9",
        "false | (false as $x | $x) = 9",
        "1 | (1 as $x | $x) = 9",
        "{\"a\":1} | (.a as $x | $x) = 9",
        "1 | (. as $x | $x) = 9",
        "[1,2,3] | (. as $x | $x) = 9",
        "[1,2,3] | (. as $x | $x[1]) = 9",
        "{\"a\":1} | (. as $x | $x.a) = 9",
        "[false] | (. as [$x] | false) = 9",
        "{\"a\":false,\"b\":1} | (. as {a:$x} | false) = 9",
        "[[false]] | (. as [[$x]] | false) = 9",
        "{\"a\":{\"b\":1}} | (. as {a: {$b}} | $b) = 9",
        "{\"a\":[1]} | (. as {a: [$x]} | $x) = 9",
        "{\"a\":false,\"b\":false} | (. as {(\"a\",\"b\"): $x} | false) = 9",
        "{\"a\":false,\"b\":true} | (. as {(\"a\",\"b\"): $x} | false) = 9",
        "[1,2] | (. as [$x,$y] | false) = 9",
        "{\"a\":false,\"b\":false} | (. as {a:$x, b:$y} | false) = 9",
        "1 | (. as [$x] | .) = 9",
        "{\"a\":[false]} | (.a as [$x] | false) = 9",
        "[1,2] | (.[0] as $x | $x) = 9",
        "false | (false as $x | ($x | .a)) = 9",
        "1 | (1 as $x | ($x | .a)) = 9",
        "{\"a\":false,\"b\":false} | (. as {(\"a\",1): $x} | .c) = 9",
        "[[false,true]] | (. as [[$x,$y]] | false) = 9",
        "[1,2,3] | (.[0,1] as $x | .[0]) = 9",
        "{\"a\":1,\"b\":2} | ((.a,.b) as $x | .a) = 9",
        "[{\"a\":1},{\"a\":2}] | (.[0,1] | .a) = 9",
        "[1,2,3] | (.[0,1] | .[0]) = 9",
        "false | length = 1",
        "null | length = 1",
        "{\"a\":1} | has(\"a\") = 9",
        "[1,2] | (foreach .[] as $x (0; .+1)) = 9",
        "{\"a\":1} | (reduce .a as $x (0; .+$x)) = 9",
        "1 as $x | ($x) = 9",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
}

#[test]
fn settings_updates_match_jq() {
    require_jq();
    let path = fixture_path("settings.json");
    for program in [
        ".listen.port = 443",
        ".regions[0].weight |= .+1",
        ".limits.rate_per_minute += 60",
        ".regions[0].weight -= 1",
        ".limits.burst *= 2",
        ".limits.max_body_bytes /= 1024",
        ".listen.port %= 1000",
        ".regions[].weight //= 1",
        "del(.labels.nul)",
        "del(.regions[] | select(.enabled == false))",
        "(.regions[] | select(.enabled)).name = \"on\"",
        ".nested.a.b.c.d.e[1][1][1][0] = 9",
        ".empty.arr[0] = 1",
        ".ids.long += 1",
        ".labels.emoji |= . + \"!\"",
        // The value is the wrong type for the operator.
        ".listen.host += 1",
        ".listen.host -= 1",
        ".listen.tls *= 2",
        ".listen.host /= 2",
        ".listen.host %= 2",
        ".empty.arr += 1",
        ".empty.obj += 1",
        ".listen.port += \"x\"",
        ".regions[2].weight *= 2",
        ".listen.host |= .+1",
        ".regions[].name += 1",
        ".empty.str -= \"\"",
        ".labels += 1",
    ] {
        compare_exact(program, &["-c", "--", program, path.as_str()], b"");
    }
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
fn citylots_paths_match_jq() {
    require_jq();
    let path = fixture_path("citylots.json");
    for program in [
        "path(.features[0].properties.BLKLOT)",
        "path(.features[].properties.STREET)",
        "[paths | select(.[-1] == \"BLKLOT\")]",
        "getpath([\"features\", 1, \"properties\", \"MAPBLKLOT\"])",
        "getpath([\"features\", 0, \"geometry\", \"coordinates\", 0, 0, 0])",
        "setpath([\"features\", 0, \"properties\", \"ODD_EVEN\"]; \"O\")",
        "delpaths([[\"features\", 2, \"geometry\", \"coordinates\"]])",
        // The mapped coordinates were computed, so this path is refused.
        "path(.features[0].geometry.coordinates | map(.[0][0]))",
    ] {
        compare_exact(program, &["-c", "--", program, path.as_str()], b"");
    }
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
fn variables_match_jq() {
    require_jq();
    let programs = [
        "1 as $x | 2 as $y | [$x,$y,$x]",
        "1 as $x | [$x,$x,$x as $x | $x]",
        "[1,2,3] | .[] as $x | [$x, .]",
        "1 as $x | (2 as $x | $x), $x",
        "[1, {c:3, d:4}] as [$a, {c:$b, b:$c}] | $a, $b, $c",
        "[1,2,3][] as $x | [[4,5,6,7][$x]]",
        r#"{"a":1,"b":[2,{"d":3}]} | . as {$a, b: [$c, {$d}]} | [$a,$c,$d]"#,
        r#"{"a":1,"b":[2,{"d":3}]} | . as {$a, $b:[$c, $d]} | [$a,$b,$c,$d]"#,
        "[1,2,3] | . as [$a,$b,$a] | $a",
        r#"{"a":1,"b":2} | . as {a:$x, b:$x} | $x"#,
        r#"{"a":1,"b":2,"c":3} | . as {c:$x, a:$x, b:$x} | $x"#,
        r#"{"a":[1,2]} | . as {$a: [$b,$a]} | [$a,$b]"#,
        r#"{"a":[1,2]} | . as {$a:[$b,$c]} | [$a,$b,$c]"#,
        r#"{"a":1} | . as {$a:$a} | $a"#,
        r#"{"a":1} | . as { $a } | $a"#,
        r#"{"b":[2,3],"a":1} | . as {b:[$x,$x], a:$x} | $x"#,
        r#"{"a":1,"b":[2,3]} | . as {a:$x, b:[$x,$x]} | $x"#,
        r#"[1, {"a":2}] | . as [$a, {a:$a}] | $a"#,
        r#"[{"a":1,"b":2}, {"a":3,"b":4}] | . as [{("a","b"):$x}, {("a","b"):$y}] | [$x,$y]"#,
        r#"[{"a":1,"b":2}, 9] | . as [{("a","b"):$x}, $y] | [$x,$y]"#,
        r#"{"a":1,"b":2} | . as {("a","b"):$x, ("a","b"):$y} | [$x,$y]"#,
        "null | . as [$a,$b] | [$a,$b]",
        "null | . as {a:[$x]} | $x",
        "[] | . as [{a:$x}] | $x",
        "[] | . as [$a,$b] | [$a,$b]",
        "[1] | . as [$a,$b] | [$a,$b]",
        r#"{"a":1} | . as {a:$x, b:$y} | [$x,$y]"#,
        r#"{"a":null} | . as {a:[$x]} | $x"#,
        r#""b" as $k | {"b":7} | . as {($k):$v} | $v"#,
        r#"{"name":"a","a":5} | . as {(.name): $x} | $x"#,
        r#"{"ab":4} | . as {("a"+"b"):$x} | $x"#,
        r#"{"a1":9} | . as {"a\(1)":$x} | $x"#,
        r#"{"a\nb":7} | . as {"a\nb":$x} | $x"#,
        "{foo::bar:1} | . as {foo::bar: $x} | $x",
        "1 as $x | 2 + $x | $x",
        "1 | 2 + 3 as $x | $x",
        "4 * 2 as $x | $x + 1",
        "1, 2 as $x | $x + 10, $x",
        "1, 2 as $x | [$x]",
        "(1, 2) as $x | [$x]",
        "1 + 2 as $x | $x, 3",
        "false and 2 as $x | [7]",
        "false or 1 as $x | [$x]",
        "0 // 2 as $x | [7]",
        "null // 2 as $x | [$x]",
        "3 < 2 as $x | [$x]",
        "-1 as $x | $x",
        "true | not as $x | $x",
        "1 | not as $x | [$x]",
        "123 as $label | $label",
        "1 as $foreach | $foreach",
        "1as$x|$x",
        "1 as$x|$x",
        "[0,1,2,3,4] as $a | $a[1:3]",
        "[0,1,2] as $a | $a[-1]",
        r#"{"a":1} as $o | $o .a"#,
        r#"1 as $x | "\($x)""#,
        r#"1 as $n | "\($n+1)""#,
        r#"{"a":{"b":1}} as $o | "\($o.a.b)""#,
        "1 as $x | if $x > 0 then $x else 0 end",
        "{a: (1 as $x | $x)}",
        "[] | .[] as $x | 1",
        r#"{"a":1} | . as {(([]|.[])):$x} | 7"#,
        r#"[[2,{"j":1}],[5,{"j":3}],[6,{"j":4}]] | reduce .[] as [$i, {j:$j}] (0; . + $i - $j)"#,
        "reduce [[1,2,10],[3,4,10]][] as [$i,$j] (0; . + $i * $j)",
        "null | reduce . as $n (.; .)",
        "[1,2,3] | reduce .[] as $then (4 as $else | $else; . as $elif | . + $then * $elif)",
        "reduce (1,2) as $x ((10,20); .+$x)",
        "[1,2,3] | reduce . as $x ((10,20,30); $x)",
        r#"{"a":3} | reduce .a as $x ((10,20); .+$x)"#,
        "[1,2] | reduce ([1,2]|.[]) as $x ((10,20); .+$x)",
        "[] | reduce .[] as $x (0; .+$x)",
        "[] | [foreach .[] as $x (0; .+$x)]",
        "reduce (1,2) as $x (([]|.[]); .+$x)",
        "[foreach (1,2) as $x (([]|.[]); .+$x)]",
        "reduce (1,2) as $x (0; .+$x, .+10)",
        "[foreach (1,2) as $x (0; .+$x, .+10)]",
        "[foreach (1,2,3) as $x (0; if $x==2 then []|.[] else .+$x end)]",
        "reduce (1,2) as $x (5; if $x==1 then []|.[] else .+$x end)",
        "reduce (1,2,3) as $x (0; .+$x, .)",
        "[foreach (1,2,3) as $x (0; .+$x; .*10)]",
        "[foreach 1 as $x (0; ., .+$x; .*10)]",
        "[foreach 1 as $x (0; ., .+$x; ., .+1)]",
        "[foreach (1,2) as $x (0; .+$x; [$x,.])]",
        "foreach (1,2) as $x (0; .+1; $x)",
        "[foreach (1,2) as $x (0; if $x==1 then []|.[] else .+$x end; .)]",
        r#"{"a":1,"b":2} | reduce . as {("a","b"):$x} (0; .+$x)"#,
        r#"{"a":1,"b":2} | [foreach . as {("a","b"):$x, ("a","b"):$y} (0; .+$x)]"#,
        r#"{"a":1,"b":2} | reduce . as {("a","b"):$x, ("a","b"):$y} (0; .+$x+$y)"#,
        r#"{"a":1} | reduce . as {(([]|.[])):$x} (5; .+$x)"#,
        r#"[1,{"a":1}] | [foreach .[] as {(([]|.[])):$x} (0; .+1)]"#,
        "[[1,2],[3]] | [reduce .[] as $row (0; reduce $row[] as $n (.; .+$n))]",
        "1 | reduce .[]? as $x (0; .+$x)",
        "[1,2,3] | reduce .[]? as $x (0; .+$x)",
        "[1,2,3] | reduce .[] as $x (0; .+$x)+1",
        "[1,2,3] | 1+reduce .[] as $x (0; .+$x)",
        "[[1],[2,3]] | [foreach .[] as $x (0; . + $x[])]",
        "[1,2,3] | foreach .[] as $x (.; . + [$x])",
        "[1,2,3] | reduce .[] as $x (0; . as $a | $a + $x)",
        "1 as $x | reduce 2 as $x ($x; . + $x)",
        "1 as $a | reduce (1,2) as $x (2 as $a | $a; . + $x + $a)",
        "1 as $y | [1,2] | reduce .[] as $x (1 as $y | $y; . + $x + $y)",
        "reduce (if true then 1 else 2 end) as $x (0; $x)",
        "reduce (1 as $x | $x) as $y (0; $y)",
        "1 | reduce not as $x (0; $x)",
        "[1,2,4] | reduce.[]as$x(0;.+ $x)",
        r#""b" as $k | {"a":{"b":4}} | .a as {($k):$x} | $x"#,
        "[1,[2]] | [reduce .. as $x ([]; . + [$x])]",
        "1 | . as [$a] | $a",
        "false | . as [$a] | $a",
        r#""ab" | . as [$a] | $a"#,
        "{} | . as [$a] | $a",
        r#"{"a":1} | . as {a:[$x]} | $x"#,
        "1 | . as {a:$x} | $x",
        r#"[1,2,3] | . as {("0"):$x} | $x"#,
        r#""ab" | . as {("0"):$x} | $x"#,
        r#"[1, {"a":2}] | . as [{a:$a}, $a] | $a"#,
        r#"{"a":1} | . as {$a:[$b]} | $a"#,
        "([1],2) as [$a] | $a",
        "(1,[2]) as [$a] | $a",
        r#"{"a":1,"b":2} | . as {("a", 1): $x} | $x"#,
        r#"{"a":1,"b":2} | . as {("a", 1): $x} | (1|.a)"#,
        r#"{"a":1,"b":2} | . as {("a", .nope, "b"): $x} | $x"#,
        r#"{"a":1,"b":2,"c":3} | . as {("a","b"):$x, ("c", 1):$y} | [$x,$y]"#,
        r#"{"a":5} | . as {(.name): $x} | $x"#,
        r#"{"a":1} | . as {(1|.+0):$x} | $x"#,
        "1 + 2 as $x | [$x]",
        "1 + 2 as $x | [$x] + 3",
        "2 * 3 as $x | [$x]",
        "4 * 2 as $x | [$x] + 1",
        "-1 as $x | [$x]",
        "2 * -1 as $x | [$x]",
        "[1,2,3] | reduce .[] as $x ((0,100); .+$x)",
        "[1,2,3] | foreach .[] as $x ((0,100); .+$x)",
        r#"[1,"a",3] | foreach .[] as $x (0; .+$x)"#,
        r#"[1,"a",3] | reduce .[] as $x (0; .+$x)"#,
        r#"[1,"a",3] | [foreach .[] as $x (0; .+$x)]"#,
        r#"[1,"a"] | foreach .[] as $x (0; .+$x; .*2)"#,
        r#"["a",1] | foreach .[] as $x (0; .+$x)"#,
        "1 | foreach (2, .a, 3) as $x (0; .+$x)",
        "1 | reduce (2, .a) as $x (0; .+$x)",
        "foreach ([1], 2) as [$a] (0; .+$a)",
        "reduce ([1], 2) as [$a] (0; .+$a)",
        "1 | reduce 1 as $x ((2, .a); .+$x)",
        "1 | foreach 1 as $x ((2, .a); .+$x)",
        "1 | reduce .a as $x (.b; .)",
        "1 | foreach .a as $x (0; .)",
        "1 | foreach 1 as $x ((2, .a); .b)",
        "1 | reduce 1 as $x ((2, .a); .b)",
        "null | reduce .[] as $x ((1,2); .)",
        "[] | reduce .[] as $x ((1,2); .+$x)",
        "1 | reduce .[] as $x (10; .+$x)",
        "1 | foreach 1 as $x (0; (5, .a))",
        "foreach (1,2) as $x (0; .+$x; if .>1 then .a else . end)",
        r#"{"a":1,"b":2} | foreach . as {("a", 1):$x} (0; .+$x)"#,
        r#"{"a":1,"b":2} | reduce . as {("a", 1):$x} (0; .+$x)"#,
        "[1,2] | . as [$a, {a:$b}] | $a",
        ". as {(1):$x} | $x",
        ". as {(1+2):$x} | $x",
        ". as {(true):$x} | 1",
        ". as {(null):$x} | 1",
        r#". as {({"a":1}):$x} | 1"#,
        "foreach 1 as $x (0; (5, .a); if .==5 then .b else . end)",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
    let with_input = [
        ("42 as $x | . | . | . + 432 | $x + 1", "34324\n"),
        ("reduce .[] as $x (0; . + $x)", "[1,2,4]\n"),
        (
            "[foreach .[] as [$i, $j] (0; . + $i - $j)]",
            "[[2,1],[5,3],[6,4]]\n",
        ),
        (
            "[foreach .[] as {a:$a} (0; . + $a; -.)]",
            r#"[{"a":1},{"b":2},{"a":3,"b":4}]"#,
        ),
        (
            "[ foreach .[] as $try (1 as $catch | $catch - 1; . + $try; .) ]",
            "[10,9,8,7]\n",
        ),
        (".[] as [$a, $b] | [$b, $a]", "[[1],[1,2,3]]\n"),
        (". as $i | . as [$i] | $i", "[0]\n"),
        (". as [$i] | . as $i | $i", "[0]\n"),
        (
            r#". as {as: $kw, "str": $str, ("e"+"x"+"p"): $exp} | [$kw,$str,$exp]"#,
            r#"{"as":1,"str":2,"exp":3}"#,
        ),
        (
            "[1,2,4] | reduce .[] as $x (#i
0; .+$x)",
            "null\n",
        ),
        ("1 | reduce .[] as $x (10; .+$x)", "[1,2,3]\n"),
        (
            "reduce (reduce .[] as $x (0; .+$x)) as $y (0; $y)",
            "[1,2,3]\n",
        ),
    ];
    for (program, input) in with_input {
        compare_exact(program, &["-c", "--", program], input.as_bytes());
    }
}

#[test]
fn variable_syntax_stays_generic() {
    require_jq();
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let programs = [
        "$x",
        "1 as $x | $y",
        "reduce 1 as $x ($x; $x)",
        "1 as $x",
        ". as []",
        ". as {}",
        ". as [$a,]",
        ". as {a:$a,}",
        "1 as $x | {$x}",
        "1 as $x | {$x: 1}",
        "1 as $x | {$x, a:1}",
        "reduce 1+2 as $x (0; .)",
        "reduce if true then 1 else 2 end as $x (0; $x)",
        "(1 as $x | $x) | $x",
        "reduce 1 as $x (0; .+$x) | $x",
        r#"{"a":1} | . as {a:$k, ($k):$v}"#,
        "1 as $x + 1",
        "reduce reduce .[] as $x (0; $x) as $y (0; $y)",
        ". as [$a]? | $a",
        ".[] as $a ?// 1 | $a",
        "if 1 as $x | $x > 0 then $x else 0 end",
        "{a: 1 as $x | $x}",
        "foreach .[] as $x (0)",
        "reduce .[] as $x (0; .; .)",
        r#"{"a":{"b":4}} | . as {$a: {($a.b): $x}} | $x"#,
        r#"[{k:1}, {"1":9}] | . as [{k:$k}, {($k):$v}] | [$k,$v]"#,
        "reduce 1 as $x (1 as $y | $y; $y)",
        "foreach .[] as $x (0; .; .; .)",
    ];
    for program in programs {
        let args: Vec<String> = ["-n", "-c", "--", program]
            .into_iter()
            .map(str::to_string)
            .collect();
        let got = run(bin, &args, b"");
        assert!(
            got.stdout.is_empty(),
            "{program}: stdout {}",
            String::from_utf8_lossy(&got.stdout)
        );
        assert_eq!(
            got.stderr,
            b"tumulus: syntax error\n",
            "{program}: stderr {}",
            String::from_utf8_lossy(&got.stderr)
        );
        assert_eq!(got.status.code(), Some(3), "{program}");
    }
}

#[test]
fn builtins_match_jq() {
    require_jq();
    let programs = [
        "empty",
        "[empty]",
        "[1, empty, 2]",
        "1, empty, 2",
        "empty | 1",
        "empty .a",
        "length",
        "infinite | length",
        "-infinite | length",
        "nan | length",
        "-(nan) | length",
        "-0 | length",
        "0.10 | length",
        "-0.10 | length",
        "150.0 | length",
        "1.2300 | length",
        "-1.50 | length",
        "-1e-1 | length",
        "1e2 | length",
        "1e308 | length",
        "1e309 | length",
        "-1.7976931348623157e+308 | length",
        "9007199254740993 | length",
        "-9007199254740993 | length",
        "1000000000000000002 | length",
        "-123456789012345678901234567890 | length",
        "(1000000000000000002 | length) == 1000000000000000000",
        "(-1.50 | length) == 1.5",
        "nan | length == null",
        "1.7976931348623157e+308 | length == infinite",
        r#""asdf" | length"#,
        r#""\u03bc" | length"#,
        r#""a\u0301" | length"#,
        r#""\uD83D\uDC4B" | length"#,
        r#""\u0000\u0000" | length"#,
        r#""a\nb" | length"#,
        r#""é" | length"#,
        "[] | length",
        "[1,2,3] | length",
        "{} | length",
        "{a:1,b:2} | length",
        "{a:1} + {b:2,a:3} | length",
        "true | length",
        "false | length",
        "(1, \"ab\", true) | length",
        "keys",
        "{b:1,a:2,b:3} | keys",
        r#"{"":1,"a":2} | keys"#,
        r#"{"\u0000":1,"a":2} | keys"#,
        r#"{"β":1,"α":2,"a":3} | keys"#,
        r#"{"10":1,"2":2,"1":3} | keys"#,
        "[10,20,30] | keys",
        "[] | keys",
        "{z:1,a:2,m:3} | keys",
        "{a:1} + {b:2,a:3} | keys",
        "[1,2,3] | keys[0]",
        "[1,2,3] | keys[-1]",
        "[1,2,3] | keys | length",
        "1 | keys",
        r#""abc" | keys"#,
        "true | keys",
        "null | keys",
        "nan | keys",
        "infinite | keys",
        r#"("y"*30) | keys"#,
        r#"{"a":1} | has("a")"#,
        r#"{"a":1} | has("b")"#,
        r#"{"a":null} | has("a")"#,
        r#"{"":1} | has("")"#,
        r#"{"a":1,"b":2} | has("a","b","c")"#,
        r#"{"a":1} | has("a", 1)"#,
        r#"{"a":1} | has("a", "b", 1, "a")"#,
        "[1,2,3] | has(0)",
        "[1,2,3] | has(2)",
        "[1,2,3] | has(3)",
        "[1,2,3] | has(-1)",
        "[1,2,3] | has(-3)",
        "[1] | has(-0.5)",
        "[1] | has(-0.1)",
        "[1,2,3] | has(-1.5)",
        "[1,2,3] | has(1.9)",
        "[1,2,3] | has(1.0)",
        "[1] | has(0.5)",
        "[1] | has(1.0)",
        "[] | has(0)",
        "[] | has(-0.1)",
        "[1] | has(1e20)",
        "[1] | has(-1e20)",
        "[1] | has(1e309)",
        "[0,1,2] | has(nan)",
        "[0,1,2] | has(infinite)",
        "[0,1,2] | has(-infinite)",
        "[10,20] | has(0,1,2,-1)",
        "[1,2,3] | has(0, \"a\")",
        "[1,2,3] | has(1+1)",
        r#"{"ab":1} | has("a"+"b")"#,
        "null | has(\"a\")",
        "null | has(0)",
        "null | has(null)",
        "null | has(true)",
        "null | has([])",
        "null | has({})",
        "null | has(nan)",
        "null | has(\"a\", 1, [], {})",
        "1 | has(0)",
        "1 | has(\"a\")",
        r#""hi" | has(0)"#,
        "true | has(\"a\")",
        "{} | has(1)",
        "{} | has(true)",
        "{} | has([])",
        "{} | has(nan)",
        "[] | has(\"a\")",
        "[] | has(true)",
        "[] | has({})",
        "[] | has(null)",
        "({a:1}, 1) | has(\"a\")",
        "empty | has(0)",
        r#"{"a":1} | has(empty)"#,
        "has (\"a\")",
        "select(true)",
        "select(false)",
        "select(null)",
        "select(0)",
        "select(\"\")",
        "select([])",
        "select({})",
        "select(empty)",
        "select(true, false, true)",
        "select(false, false)",
        "select(1, null, false, 2, empty, \"x\")",
        "select(.)",
        "1 | select(true, .a)",
        "1 | select(.a)?",
        "2 | . as $x | select($x > 1)",
        "[1,2,3] | select(length > 2)",
        "1 | select(length)",
        r#""ab" | select(length)"#,
        "true | select(length)",
        "null | select(length)",
        "[] | select(length)",
        "map(.+1)",
        "map(empty)",
        "map(., .)",
        "map(1,2,3)",
        "map(select(. > 2))",
        "map(select(select(. % 2 == 0)))",
        "[1,2,3] | map(.+1) | length",
        "map(. as $x | $x + 1)",
        "{b:1,a:2} | map(.)",
        "{b:1,a:2} | map(., .)",
        "{z:1,a:2,m:3} | map(.)",
        "{} | map(.+1)",
        "null | map(.)",
        "1 | map(.)",
        r#""abc" | map(.)"#,
        "true | map(.)",
        r#""hi" | map(.)"#,
        "123456789012345 | map(.)",
        r#""abcdefghijklmnopqrstuvwxyz" | map(.)"#,
        "[1,\"a\",3] | map(.+1)",
        "{\"a\":1,\"b\":\"x\"} | map(.+1)",
        "[1] | map(1, .[0])",
        "([1,2] | map(.+1)), (1 | map(.))",
        "(1 | length), (true | length), 3",
        "([1]|keys), (1|keys)",
        "({\"a\":1}|has(\"a\")), (1|has(\"a\"))",
        "[[1],[]] | map(has(0))",
        "[[1,2],[3,4,5]] | map(has(2))",
        "[{}, {abcd:1,abc:2,abcde:3}, {x:1,z:3,y:2}] | map(keys)",
        "[[], {}, [1,2], {a:42}, \"asdf\", \"μ\"] | [.[] | length]",
        "[1,2,empty,3,empty,4]",
        "[[1,2],[3]] | map(keys)",
        "[[1,2],[3]] | map(length)",
        "[\"ab\",\"c\"] | map(length)",
        "[{a:1},{b:2},{a:3}] | map(select(has(\"a\")))",
        "[1,2,3,4] | map(select(. % 2 == 0))",
        "[1,2,3,4] | keys | map(select(.>1))",
        "[[1,2],[3]] | map(reduce .[] as $x (0; .+$x))",
        "[1,-2,3] | reduce map(if .>0 then . else empty end) as $x (0; .+$x)",
        "[1,2,3] | foreach .[] as $x (0; .+$x) | select(.>3)",
        "[1,2,3] | foreach .[] as $x (0; .+$x; length)",
        "reduce (1,empty,2) as $x (0; .+$x)",
        "[1,empty,2] | length",
        "true | length?",
        "1 | keys?",
        "map(.)?",
        "[true] | map(length)?",
        r#"{"a":1} | has("a")?"#,
        "1 | has(\"a\")?",
        "{a: length}",
        "{a: empty}",
        "{a: keys}",
        "{length}",
        "{empty}",
        "{keys}",
        "length #x",
        "[1,2] | map #c\n(.+1)",
        "has #c\n(\"a\")",
        "\"\\(length)\"",
        "\"\\([1,2]|map(.+1))\"",
        "[1,2,3] | keys[]",
        "empty | length",
        "empty | map(.)",
        "select(empty) | length",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
    let with_input = [
        ("length", "null\n"),
        ("length", "true\n"),
        ("length", "1.2300\n"),
        ("length", "-123456789012345678901234567890\n"),
        ("length", "\"\\u03bc\"\n"),
        ("length", "\"a\\u0301\"\n"),
        ("length", "[1,2,3]\n"),
        ("length", "{\"a\":1,\"b\":2}\n"),
        ("length", "{\"a\":1,\"a\":2}\n"),
        ("keys", "{\"b\":1,\"a\":2}\n"),
        ("keys", "{\"a\":1,\"a\":2}\n"),
        ("keys", "[42,3,35]\n"),
        ("keys", "null\n"),
        ("keys", "\"abcdefghijklmnopqrstuvwxyz\"\n"),
        (
            "[.[] | length]",
            "[[], {}, [1,2], {\"a\":42}, \"asdf\", \"μ\"]\n",
        ),
        (
            "map(keys)",
            "[{}, {\"abcd\":1,\"abc\":2,\"abcde\":3}, {\"x\":1, \"z\": 3, \"y\":2}]\n",
        ),
        ("[1,2,empty,3,empty,4]", "null\n"),
        ("map(has(\"foo\"))", "[{\"foo\": 42}, {}]\n"),
        ("map(has(2))", "[[0,1], [\"a\",\"b\",\"c\"]]\n"),
        ("has(nan)", "[0,1,2]\n"),
        ("map(.+1)", "[1,2,3]\n"),
        ("map(.+1)", "[1]\n[2,3]\n"),
        ("map(.+1)", "[1]\ntrue\n[2]\n"),
        ("map(select(. > 2))", "[1,2,3,4]\n"),
        (".[] | select(. > 2)", "[1,2,3,4]\n"),
        ("select(has(\"a\"))", "{\"a\":1}\n{}\n"),
        ("has(\"a\")", "{\"a\":1}\n1\n"),
        ("{a: map(.+1)}", "[1,2]\n"),
        ("map (.+1)", "[1,2,3]\n"),
        ("select (. > 0)", "5\n"),
        ("has (0)", "[1,2]\n"),
        ("length[]", "[1,2,3]\n"),
        ("[] | map(.)", "[]\n"),
        ("{\"a\":1} | map(.)", "{\"a\":1}\n"),
    ];
    for (program, input) in with_input {
        compare_exact(program, &["-c", "--", program], input.as_bytes());
    }
    compare_exact("pretty keys", &["--", "keys"], b"{\"b\":1,\"a\":2}\n");
    compare_exact("pretty map", &["--", "map(.+1)"], b"[1,2]\n");
}

#[test]
fn builtin_syntax_stays_generic() {
    require_jq();
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let programs = [
        "empty()",
        "empty .",
        "length()",
        "length .",
        "length?(1)",
        "keys()",
        "(length)(1)",
        "(empty)(1)",
        "has()",
        "select()",
        "map()",
        "map(.+1)(2)",
        "(map(.+1))(2)",
        "todate()",
        "todate ()",
        "fromdate()",
        "gmtime()",
        "mktime()",
        "(todate)(0)",
        "(fromdate)(\"\")",
        "(gmtime)(0)",
        "(mktime)(0)",
        "strftime()",
        "strftime ()",
        "(strftime(\"%Y\"))(0)",
        "strptime()",
        "strptime ()",
        "(strptime(\"%Y\"))(0)",
        "now()",
        "now ()",
        "(now)(0)",
        "localtime()",
        "localtime ()",
        "(localtime)(0)",
        "strflocaltime()",
        "strflocaltime ()",
        "(strflocaltime(\"%Y\"))(0)",
        "error()",
        "error( )",
        "error .",
        "(error)(1)",
        "error(1)(2)",
        "(1 | error)(2)",
        "(try error catch .)(1)",
        "try",
        "catch .",
        "try catch .",
        "try error catch",
        "try ?",
        "try error catch ?",
        "error catch .",
        "reduce try 1 catch 2 as $x (0; .+$x)",
        "try (1)(2)",
        "try (1)(2) catch 3",
        "try (.)? catch 9",
        "try error? catch",
        "{a: try 1 catch 2}",
        r#"{"a": try error(1) catch 2}"#,
        "{try: try 1 catch 2}",
        "try 1, 2 catch 3",
        "try 1 | error catch .",
        "try 1 + 2 catch 9",
        "try .a // .b catch 1",
        "try 1 < 2 catch 3",
        "try .a = 1 catch .",
        "try -=1",
        "{a: 1 | try error(1) catch 2}",
        "try 1 as $x | error($x) catch $x",
        "type()",
        "type .",
        "type?(1)",
        "(type)(1)",
        "tonumber()",
        "tonumber .",
        "tonumber?(1)",
        "(tonumber)(1)",
        "tostring()",
        "tostring .",
        "tostring?(1)",
        "(tostring)(1)",
        "join()",
        "join(\",\")(1)",
        "(join(\",\"))(1)",
        "ltrimstr()",
        "ltrimstr(\"x\")(1)",
        "(ltrimstr(\"x\"))(1)",
        "rtrimstr()",
        "rtrimstr(\"x\")(1)",
        "(rtrimstr(\"x\"))(1)",
        "add()",
        "add .",
        "tojson()",
        "tojson .",
        "to_entries()",
        "from_entries()",
        "walk()",
        "with_entries()",
        "recurse()",
    ];
    for program in programs {
        let args: Vec<String> = ["-n", "-c", "--", program]
            .into_iter()
            .map(str::to_string)
            .collect();
        let got = run(bin, &args, b"");
        assert!(
            got.stdout.is_empty(),
            "{program}: stdout {}",
            String::from_utf8_lossy(&got.stdout)
        );
        assert_eq!(
            got.stderr,
            b"tumulus: syntax error\n",
            "{program}: stderr {}",
            String::from_utf8_lossy(&got.stderr)
        );
        assert_eq!(got.status.code(), Some(3), "{program}");
    }
    let undefined = [
        "empty(1)",
        "empty (1)",
        "empty(1;2)",
        "length(1)",
        "length (1)",
        "length(1;2)",
        "keys(1)",
        "keys (1)",
        "has",
        "has(1;2)",
        "select",
        "select(1;2)",
        "map",
        "map(1;2)",
        ". | map",
        "lengthx",
        "hasx(1)",
        "todate(1)",
        "fromdate(1)",
        "gmtime(1)",
        "mktime(1)",
        "strftime",
        "strftime(1;2)",
        "strptime",
        "strptime(1;2)",
        "now(1)",
        "localtime(1)",
        "strflocaltime",
        "strflocaltime(1;2)",
        "error(1;2)",
        "try length(1)",
        "type(1)",
        "type (1)",
        "type(1;2)",
        "tonumber(1)",
        "tonumber (1)",
        "tonumber(1;2)",
        "tostring(1)",
        "tostring (1)",
        "tostring(1;2)",
        "join",
        "join(1;2)",
        "ltrimstr",
        "ltrimstr(1;2)",
        "rtrimstr",
        "rtrimstr(1;2)",
        "not(1)",
        "tryerror",
        "try errorcatch",
        "add(1)",
        "add(1;2)",
        "tojson(1)",
        "to_entries(1)",
        "from_entries(1)",
        "walk",
        "walk(1;2)",
        "with_entries",
        "with_entries(1;2)",
        "recurse(1;2;3)",
    ];
    for program in undefined {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
}

#[test]
fn entries_walk_recurse_add_match_jq() {
    require_jq();
    let programs = [
        "limit(5; 0|recurse(.+1))",
        "limit(1; 0|recurse(.+1))",
        "limit(0; 0|recurse(.+1))",
        "first(0|recurse(.+1))",
        "nth(2; 0|recurse(.+1))",
        "limit(10000; 0|recurse(.+1)) | last",
        "0 | recurse(.+1; .<3)",
        "limit(3; 0|recurse(.+1; (true, error(\"z\"))))",
        "limit(4; 0|recurse(1, error(\"z\")))",
        "limit(4; (1,2)|recurse(.+10))",
        "limit(4; null|recurse(.+1))",
        "limit(6; {}|recurse(1,2))",
        "limit(5; 0|recurse(.+1, .+2))",
        "limit(3; 0|recurse(.+1; 0))",
        "0 | recurse(.+1; false)",
        "0 | recurse(.+1; null)",
        "0 | recurse(.+1; empty)",
        "def f: recurse(.+1); limit(3; 0|f)",
        "def f: limit(3; 0|recurse(.+1)); 0|f",
        "def f: recurse(.+1); limit(10000; 0|f) | last",
        "def f: . + 1; limit(4; 0|recurse(f))",
        "def f($n): . + $n; limit(4; 0|recurse(f(1)))",
        "def apply(g): g; limit(3; 0|recurse(apply(.+1)))",
        "def f(g): recurse(g); limit(3; 0|f(.+1))",
        "0 | 5 as $n | limit(3; recurse(. + $n))",
        "[1,null] | recurse(.[]?)",
        "[1,null] | recurse(.[]?; . != null)",
        "{\"a\":1} | recurse(.[]?; (1,2))",
        "0 | [recurse(if .<2 then (.+1, .+10) else empty end)]",
        "[1,[2,[3]]] | [limit(3; recurse)]",
        "[1,[2,[3]]] | [limit(3; ..)]",
        "empty | recurse",
        "1 | recurse",
        "null | recurse",
        "\"ab\" | recurse",
        "true | recurse",
        "{\"a\":null} | recurse",
        "{\"b\":1,\"a\":2} | recurse",
        "[1,[2]] | recurse",
        "1 | recurse(.a)",
        "try (1|recurse(.a)) catch .",
        "limit(8; [1,[2]]|recurse(.[]))",
        "[1] | recurse(.a)",
        "0 | try recurse(.+1; .<1, error(\"z\")) catch .",
        "[1,[2]] | recurse |= .+1",
        "[1,[2]] | recurse(.[]?) |= .+1",
        "[1,[2]] | (recurse(.[]?) | select(type==\"number\")) |= .+1",
        "[1,null,2] | (recurse(.[]?; . != null) | select(type==\"number\")) |= .+1",
        "1 | recurse(.+1) |= .+1",
        "1 | recurse(.a?) |= .+1",
        "try (1 | path(recurse(.+1))) catch .",
        "[1,[2]] | [path(recurse(.[]?))]",
        "{\"a\":1} | [path(recurse(.a?; (1,2)))]",
        "{\"a\":1} | (recurse(.a?; (1,2)) | select(type==\"number\")) |= .+1",
        "try ([1,null] | path(recurse(.[]?; . != null, error(\"z\")))) catch .",
        "try ([1,2] | path(recurse((.[0], error(\"z\"))))) catch .",
        "def f: .[]?; [1,[2]] | (recurse(f) | select(type==\"number\")) |= .+1",
        "1 | [path(recurse(empty))]",
        "[1,null] | [path(recurse(.[]?; . != null))]",
        "try ([1,2] | path(recurse(.[]))) catch .",
        "limit(1; 0|walk(recurse(.+1)))",
        "[1,[2]] | walk(if type==\"array\" then .+[9] else . end)",
        "{\"a\":10} | walk(if type==\"number\" then (., .+1, .+2) else . end)",
        "[10] | walk(if type==\"number\" then (., .+1, .+2) else . end)",
        "{\"a\":10,\"b\":20} | walk(if type==\"number\" then (., .+1) else . end)",
        "{\"a\":1,\"b\":20} | walk(if type==\"number\" and .<10 then empty else . end)",
        "{\"a\":1} | walk(if type==\"number\" then empty else . end)",
        "{\"a\":{\"b\":1,\"c\":20}} | walk(if type==\"number\" and .<10 then empty else . end)",
        "[1,\"a\",[2]] | walk(select(type==\"number\"))",
        "1 | walk(empty)",
        "{\"a\":1} | walk(empty)",
        "{\"x\":0} | walk(1)",
        "{\"x\":0} | [walk(.,1)]",
        "1 | walk(.+1)",
        "try (1|walk(error(\"z\"))) catch .",
        "{\"b\":1,\"a\":{\"c\":2}} | walk(. + if type==\"number\" then 1 else {} end)",
        "{\"b\":1,\"a\":2} | walk(.)",
        "[] | walk(.)",
        "{} | walk(.)",
        "{\"a\":1} | walk(select(type==\"object\"))",
        "try ([0,1]|walk(if .==1 then error(\"z\") else . end)) catch .",
        "try ({\"a\":1}|walk(if type==\"number\" then (., error(\"z\")) else . end)) catch .",
        "{\"a\":0}|walk(if type==\"number\" then recurse(.+1) else . end)",
        "{\"a\":{\"b\":0}}|walk(if type==\"number\" then (., error(\"z\")) else . end)",
        "try ([0]|walk(if type==\"number\" then (., error(\"z\")) else . end)) catch .",
        "try (1|walk((., error(\"z\")))) catch .",
        "{\"b\":1,\"a\":2} | to_entries",
        "[10,20] | to_entries",
        "[] | to_entries",
        "{} | to_entries",
        "null | to_entries",
        "1 | to_entries",
        "\"ab\" | to_entries",
        "true | to_entries",
        "{\"z\":1,\"a\":{\"b\":2}} | to_entries",
        "{\"a\":null} | to_entries",
        "[[{\"key\":\"a\",\"value\":1},{\"Key\":\"b\",\"Value\":2},{\"name\":\"c\",\"value\":3},{\"Name\":\"d\",\"Value\":4}]] | .[] | from_entries",
        "[{\"key\":null,\"name\":\"a\",\"value\":1}] | from_entries",
        "[{\"Key\":null,\"name\":\"a\",\"value\":1}] | from_entries",
        "[{\"name\":null,\"Name\":\"z\",\"value\":1}] | from_entries",
        "[{\"key\":\"a\",\"value\":null,\"Value\":9}] | from_entries",
        "[{\"Key\":\"a\",\"value\":null,\"Value\":3}] | from_entries",
        "[{\"key\":\"a\",\"Value\":9}] | from_entries",
        "[{\"name\":\"a\",\"Value\":8}] | from_entries",
        "[{\"k\":\"a\",\"v\":1}] | from_entries",
        "[{\"k\":\"a\",\"value\":5}] | from_entries",
        "[{\"key\":\"a\",\"v\":1}] | from_entries",
        "[{\"key\":\"a\",\"val\":1}] | from_entries",
        "[{\"key\":\"g\"}] | from_entries",
        "[{\"name\":\"a\"}] | from_entries",
        "[null] | from_entries",
        "[{}] | from_entries",
        "null | from_entries",
        "[1] | from_entries",
        "[[1]] | from_entries",
        "[\"a\"] | from_entries",
        "[true] | from_entries",
        "[{\"key\":\"a\",\"value\":1},{\"name\":\"b\"}] | from_entries",
        "[{\"key\":\"a\",\"value\":1},{\"key\":\"a\",\"value\":2}] | from_entries",
        "[{\"key\":\"a\",\"value\":1,\"name\":\"b\"}] | from_entries",
        "[{\"key\":false,\"name\":\"a\",\"value\":1}] | from_entries",
        "[{\"key\":0,\"name\":\"a\",\"value\":1}] | from_entries",
        "[{\"key\":\"\",\"value\":1}] | from_entries",
        "[{\"key\":\"a\",\"value\":false,\"Value\":1}] | from_entries",
        "[{\"key\":\"a\",\"VALUE\":1}] | from_entries",
        "[{\"NAME\":\"a\",\"value\":1}] | from_entries",
        "[{\"key\":1,\"value\":2}] | from_entries",
        "[{\"key\":{\"a\":1},\"value\":1}] | from_entries",
        "[{\"key\":[1],\"value\":1}] | from_entries",
        "[{\"key\":true,\"value\":1}] | from_entries",
        "{\"z\":{\"key\":\"a\",\"value\":1},\"y\":{\"name\":\"b\",\"Value\":2}} | from_entries",
        "{\"a\":1} | from_entries",
        "[] | from_entries",
        "{} | from_entries",
        "{\"a\":1,\"b\":2} | with_entries(.key |= \"K_\" + .)",
        "{\"a\":1} | with_entries(.)",
        "{\"a\":1,\"b\":2} | with_entries(select(.key==\"a\"))",
        "[10,20] | with_entries(.)",
        "null | with_entries(.)",
        "{\"a\":1} | with_entries(1)",
        "{\"a\":1} | with_entries(.value)",
        "{\"a\":1} | with_entries(.value |= . + 1)",
        "{\"a\":1} | with_entries((., .))",
        "\"ab\" | tojson",
        "\"ab\" | tostring",
        "\"\" | tojson",
        "\"a\\nb\" | tojson",
        "\"a\\nb\" | tostring",
        "1 | tojson",
        "01 | tojson",
        "01 | tostring",
        "nan | tojson",
        "infinite | tojson",
        "-infinite | tojson",
        "{\"b\":1,\"a\":2} | tojson",
        "[1,\"a\"] | tojson",
        "null | add",
        "[] | add",
        "{} | add",
        "[1,2,3] | add",
        "[1] | add",
        "[\"a\",\"b\"] | add",
        "[1,\"a\"] | add",
        "\"ab\" | add",
        "1 | add",
        "true | add",
        "[null] | add",
        "[null,1] | add",
        "[1,2,null] | add",
        "[false] | add",
        "[true,false] | add",
        "[{a:1},{b:2}] | add",
        "[{a:1},{a:2,b:3}] | add",
        "[{a:{b:1}},{a:{c:2}}] | add",
        "[[1],[2]] | add",
        "{a:1,b:2} | add",
        "{b:\"y\",a:\"x\"} | add",
        "[{},{}] | add",
        "try (null | add) catch .",
        "try (1,2 | add) catch .",
        "def f: add; [1,2,3] | f",
        "def add(a;b): a*b; add(3;4)",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
    compare_exact(
        "tojson ignores sort",
        &["-n", "-S", "-c", "--", "{\"b\":1,\"a\":2} | tojson"],
        b"",
    );
}

#[test]
fn small_builtins_match_jq() {
    require_jq();
    let programs = [
        "null | type",
        "true | type",
        "false | type",
        "1 | type",
        "1.50 | type",
        "-0 | type",
        "nan | type",
        "infinite | type",
        "-infinite | type",
        "\"x\" | type",
        "[] | type",
        "{} | type",
        "try error(nan) catch type",
        "try error(1) catch . | type",
        "1 | type?",
        "{type:1} | .type",
        "{type:1} | type",
        "{type:1} | {type}",
        "{tostring:1} | .tostring",
        "{tonumber:2, tostring:1} | keys",
        "1 | tonumber",
        "1.50 | tonumber",
        "1.2300 | tonumber",
        "1.2300 | tonumber == 1.2300",
        "-0 | tonumber",
        "nan | tonumber | type",
        "infinite | tonumber | type",
        "\"1\" | tonumber",
        "(\"1\" | tonumber) == 1",
        "\"01\" | tonumber",
        "(\"01\" | tonumber) == 1",
        "\"00\" | tonumber",
        "\"1e2\" | tonumber",
        "\"1E+2\" | tonumber",
        "1e2 | tonumber",
        "\"1.2300\" | tonumber",
        "\"1.\" | tonumber",
        "\".5\" | tonumber",
        "\"  +1 \" | tonumber",
        "\"1 \" | tonumber",
        "\" 1\" | tonumber",
        "\"1\\n\" | tonumber",
        "\"\\n1\\n\" | tonumber",
        "\"\\t1\" | tonumber",
        "\"1\\t\" | tonumber",
        "\"1e400\" | tonumber",
        "\"-0\" | tonumber",
        "\"-0\" | tonumber | tostring",
        "\"+0\" | tonumber",
        "\"nan\" | tonumber | type",
        "\"+nan\" | tonumber | tostring",
        "\"Infinity\" | tonumber | type",
        "\"\\uFEFF1\" | tonumber",
        "\"abc\" | tonumber",
        "\"\" | tonumber",
        "\"   \" | tonumber",
        "\"1 2\" | tonumber",
        "\"1\\n2\" | tonumber",
        "\"true\" | tonumber",
        "\"false\" | tonumber",
        "\"null\" | tonumber",
        "\"[]\" | tonumber",
        "\"{}\" | tonumber",
        "\"\\\"1\\\"\" | tonumber",
        "\"[1]\" | tonumber",
        "\"1]\" | tonumber",
        "\"1,\" | tonumber",
        "\"1 abc\" | tonumber",
        "\"1 true\" | tonumber",
        "\"1 []\" | tonumber",
        "\"true 1\" | tonumber",
        "\"infinite\" | tonumber",
        "\"-infinite\" | tonumber",
        "\"2a\" | tonumber",
        "\"1e\" | tonumber",
        "\"0x10\" | tonumber",
        "\"1.2.3\" | tonumber",
        "\"1e2.0\" | tonumber",
        "\"--1\" | tonumber",
        "\"+\" | tonumber",
        "\".\" | tonumber",
        "\"a\\\"b\" | tonumber",
        "\"a'b\" | tonumber",
        "\"it is not a number\" | tonumber",
        "\"café\" | tonumber",
        "\"abcdefghijklmnopqrstuvwxyz\" | tonumber",
        "null | tonumber",
        "true | tonumber",
        "false | tonumber",
        "[] | tonumber",
        "{} | tonumber",
        "[1] | tonumber",
        "[1,2,3,4,5,6,7,8] | tonumber",
        "{\"abcdefg\":1,\"h\":2} | tonumber",
        "try (\"abc\" | tonumber) catch .",
        "try (1 | tonumber) catch \"no\"",
        "\"abc\" | tonumber?",
        "[try (\"1\", \"2a\", \"3\") | tonumber]",
        "1 | tostring",
        "1.50 | tostring",
        "-0 | tostring",
        "nan | tostring",
        "infinite | tostring",
        "-infinite | tostring",
        "true | tostring",
        "false | tostring",
        "null | tostring",
        "\"already\" | tostring",
        "[nan] | tostring",
        "{b:1,a:2} | tostring",
        "{\"a\":1,\"a\":2} | tostring",
        "{a:[1,{\"b\":true}]} | tostring",
        "[13911860366432393] | .[0] | tostring",
        "\"ab\" | tostring",
        "1 | tostring?",
        "join(\",\")",
        "type",
        "tonumber",
        "tostring",
        "ltrimstr(\"a\")",
        "rtrimstr(\"a\")",
        "[\"a\",\"b\",\"c\",\"d\"] | join(\",\",\"/\")",
        "[[], [\"\"], [\"\",\"\"], [\"\",\"\",\"\"]] | [.[] | join(\"a\")]",
        "[\"1\",2,true,false,3.4] | join(\",\")",
        "[[], [null], [null,null], [null,null,null]] | .[] | join(\",\")",
        "[[\"a\",null], [null,\"a\"]] | .[] | join(\",\")",
        "[\"1\",\"2\",{\"a\":{\"b\":{\"c\":33}}}] | try join(\",\") catch .",
        "[\"1\",\"2\",[3,4,5]] | try join(\",\") catch .",
        "[1] | join(\",\")",
        "[1] | join([])",
        "[1] | join(1)",
        "[1] | join(error(\"z\"))",
        "[1,2] | join([])",
        "[1,2] | join(\"\",\"/\")",
        "[null] | join(\",\")",
        "[null,null] | join(\",\")",
        "[null,null,null] | join(\",\")",
        "[1,null,2] | join(\",\")",
        "[\"a\",null,\"b\"] | join(\",\")",
        "[\"a\",\"b\"] | join(\"\")",
        "[\"a\",\"b\"] | join(null)",
        "[null,null] | join(null)",
        "[nan,infinite,-infinite] | join(\",\")",
        "[] | join(\",\")",
        "{} | join(\",\")",
        "[] | join(1)",
        "[] | join({})",
        "[] | join(error(\"z\"))",
        "[] | join(\",\", error(\"z\"))",
        "[] | join(empty)",
        "[\"a\",\"b\"] | join(empty)",
        "[1,2,3] | join(empty, \",\")",
        "{a:1,b:2} | join(\",\")",
        "{b:1,a:2} | join(\"-\")",
        "{a:null} | join(\",\")",
        "{a:1} | join([])",
        "{a:1,b:2} | join(empty)",
        "\"abc\" | join(\",\")",
        "1 | join(\",\")",
        "1 | join(error(\"z\"))",
        "1 | join(\",\", error(\"z\"))",
        "null | join(\",\")",
        "true | join(\",\")",
        "1 | join(\",\")?",
        "[\"a\",\"b\"] | join(1)",
        "[\"a\",\"b\"] | join(true)",
        "[\"a\",\"b\"] | join([])",
        "[\"a\",\"b\"] | join({})",
        "[\"a\",\"b\"] | join(1, \",\")",
        "[\"a\"] | join(1, \",\")",
        "[\"a\",\"b\"] | join(\",\", error(\"e\"))",
        "[\"a\",\"b\",\"c\"] | join(\"x\", error(\"e\"), \"y\")",
        "[[1],[2]] | join(\",\")",
        "[1,[2]] | join(\",\")",
        "[\"a\",{},\"b\"] | join(\",\")",
        "[{}] | join(\",\")",
        "[{a:1}] | join(\"-\")",
        "[\"a\",\"b\"] | join (\",\")",
        "[\"fo\", \"foo\", \"barfoo\", \"foobar\", \"afoo\"] | [.[] | ltrimstr(\"foo\")]",
        "[\"fo\", \"foo\", \"barfoo\", \"foobar\", \"foob\"] | [.[] | rtrimstr(\"foo\")]",
        "\"foobar\" | ltrimstr(\"foo\")",
        "\"foobar\" | ltrimstr(\"bar\")",
        "\"foobar\" | rtrimstr(\"bar\")",
        "\"foobar\" | rtrimstr(\"foo\")",
        "\"foo\" | ltrimstr(\"foo\")",
        "\"foo\" | ltrimstr(\"foobar\")",
        "\"fff\" | ltrimstr(\"f\")",
        "\"aaa\" | ltrimstr(\"aa\")",
        "\"foofoo\" | ltrimstr(\"foo\")",
        "\"foofoo\" | rtrimstr(\"foo\")",
        "\"\" | ltrimstr(\"\")",
        "\"\" | ltrimstr(\"a\")",
        "\"\" | rtrimstr(\"a\")",
        "\"a\" | ltrimstr(\"\")",
        "\"abc\" | rtrimstr(\"\")",
        "\"café\" | ltrimstr(\"caf\")",
        "\"café\" | rtrimstr(\"é\")",
        "\"éé\" | ltrimstr(\"é\")",
        "\"\\u0301a\" | ltrimstr(\"\\u0301\")",
        "\"\\u0000foo\" | ltrimstr(\"\\u0000\")",
        "\"foo\\u0000\" | rtrimstr(\"\\u0000\")",
        "\"  a\" | ltrimstr(\" \")",
        "\"a  \" | rtrimstr(\" \")",
        "\" a \" | ltrimstr(\" \")",
        "1 | ltrimstr(\"x\")",
        "1 | ltrimstr(1)",
        "1 | rtrimstr(\"x\")",
        "null | ltrimstr(\"x\")",
        "null | rtrimstr(\"x\")",
        "true | ltrimstr(\"x\")",
        "[] | ltrimstr(\"x\")",
        "{} | ltrimstr(\"x\")",
        "{} | rtrimstr(1)",
        "\"hi\" | ltrimstr(1)",
        "\"hi\" | ltrimstr(null)",
        "\"hi\" | ltrimstr(true)",
        "\"hi\" | ltrimstr([])",
        "\"hi\" | ltrimstr({})",
        "\"hi\" | rtrimstr(1)",
        "\"hi\" | rtrimstr(null)",
        "\"hi\" | rtrimstr(true)",
        "\"hi\" | rtrimstr([])",
        "\"hi\" | rtrimstr({})",
        "{\"hey\":[]} | ltrimstr(\"x\")",
        "{\"hey\":[]} | rtrimstr(\"x\")",
        "\"xxhello\" | ltrimstr(\"x\",\"xx\")",
        "\"xxhello\" | ltrimstr(\"x\",1)",
        "\"hi\" | ltrimstr(1,\"h\")",
        "\"hi\" | rtrimstr(1,\"i\")",
        "(1,\"xxhello\") | ltrimstr(\"x\")",
        "\"hi\" | ltrimstr(\"h\", error(\"e\"))",
        "\"foo\" | ltrimstr(empty)",
        "\"foo\" | ltrimstr(\"f\", empty)",
        "\"hi\" | ltrimstr (\"h\")",
        "try ltrimstr(1) catch \"x\", try rtrimstr(1) catch \"x\" | \"ok\"",
        "try ltrimstr(\"x\") catch \"x\", try rtrimstr(\"x\") catch \"x\" | \"ok\"",
        "{\"a\": {\"b\": [1, {\"b\": 3}]}} | (.. | select(type == \"object\" and has(\"b\") and (.b | type) == \"array\") | .b) |= .[0]",
        "[-9007199254740993, -9007199254740992, 9007199254740992, 9007199254740993, 13911860366432393] | .[] as $n | $n+0 | [., tostring, . == $n]",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
    compare_exact(
        "sort keys stays out of tostring",
        &["-n", "-c", "-S", "--", "{b:1,a:2} | tostring"],
        b"",
    );
    compare_exact(
        "ascii stays out of tostring",
        &["-n", "-c", "-a", "--", "{\"é\":\"ü\"} | tostring"],
        b"",
    );
    compare_exact("raw tostring", &["-n", "-r", "--", "1 | tostring"], b"");
    compare_exact(
        "join output tostring",
        &["-n", "-j", "--", "\"ab\" | tostring"],
        b"",
    );
    compare_exact(
        "pretty tostring",
        &["-n", "--", "{b:1,a:2} | tostring"],
        b"",
    );
    compare_exact(
        "tostring of input ignores sort",
        &["-S", "-c", "--", "tostring"],
        b"{\"b\":1,\"a\":2}\n",
    );
    let with_input = [
        (".[] |= try tonumber", "[\"1\", \"2a\", \"3\", 4]\n"),
        ("join(\",\")", "[\"a\",\"b\"]\n"),
        ("type", "{\"b\":1}\n"),
        ("tonumber", "\"01\"\n"),
        ("tostring", "{\"b\":1,\"a\":2}\n"),
        ("ltrimstr(\"foo\")", "\"foobar\"\n"),
        ("rtrimstr(\"bar\")", "\"foobar\"\n"),
        (
            "try ltrimstr(1) catch \"x\", try rtrimstr(1) catch \"x\" | \"ok\"",
            "\"hi\"\n",
        ),
        (
            "try ltrimstr(\"x\") catch \"x\", try rtrimstr(\"x\") catch \"x\" | \"ok\"",
            "{\"hey\":[]}\n",
        ),
    ];
    for (program, input) in with_input {
        compare_exact(program, &["-c", "--", program], input.as_bytes());
    }
}

#[test]
fn errors_match_jq() {
    require_jq();
    let programs = [
        "try error",
        "try (1, error(\"x\"), 2)",
        "try (1, error(\"x\"), 2) catch \"c\"",
        "1, try error(\"x\"), 2",
        "[try (1, error(\"a\"), 3) catch .]",
        "try try try error(1) catch 2 catch 3 catch 4",
        "try try error(1) catch error(2) catch 3",
        "try error(1) catch error",
        "try error(\"a\") catch error(\"b\")",
        "try error(1) catch (3, error(4))",
        "error",
        "error(\"m\")",
        "error(1)",
        "error([1,2])",
        "error({\"b\":1,\"a\":2})",
        "error(true)",
        "error(false)",
        "error(1.0)",
        "error(1.5)",
        "error(0)",
        "error(-0)",
        "error(1e21)",
        "error(1e-7)",
        "error(infinite)",
        "error(-infinite)",
        "error(nan)",
        "try error(nan) catch (.[0])",
        "error(\"a\\nb\")",
        "error(\"say \\\"hi\\\"\")",
        "error(\"a\\u0000b\")",
        "error(\"\\u0000x\")",
        "try error(\"a\\u0000b\") catch .",
        "try error(\"a\\u0000b\") catch length",
        "try error(\"a\\u0000b\") catch .[0:1]",
        "try error catch .",
        "try error(\"m\") catch .",
        "try error(1) catch . + 1",
        "try error([1]) catch .[0]",
        "try error({\"a\":1}) catch .a",
        "try (1/0) catch .",
        "try (1%0) catch .",
        "try keys catch .",
        "try length catch .",
        "error(empty)",
        "try error(empty) catch .",
        "error(1,2)",
        "error(1, error(2), 3)",
        "(1,2,3) | error",
        "error(1 | .a)",
        "error(error(\"inner\"))",
        "try {a: error(1)} catch .",
        "try [error(1)] catch .",
        "try \"x\\(error(1))y\" catch .",
        "try [1, error(\"a\"), 3] catch .",
        "try {a:1, b: error(2)} catch .",
        "try error(1) catch . | .",
        "try (error(1) | .) catch .",
        "try 1 catch 2 * 3",
        "try 1 catch 2 + 3",
        "try 1 catch 2, 3",
        "try 1 catch 2 | .+3",
        "try 1 * 2",
        "try 1 + 2",
        "try error(\"a\") // 1",
        "try true or error(1)",
        "try false or error(1)",
        "try false and error(1)",
        "try 1 and error(\"b\")",
        "try -1 * error(9)",
        "try 1 * error(9)",
        "try -2 * 3 catch 4",
        "try -2 * 3",
        "try 1 catch -2 * 3",
        "try error(1) catch -2 * 3",
        "try 1 catch -2 + 3",
        "try error(1) catch -2 + 3",
        "try 1 catch 2 / -3 * 4",
        "try -1",
        "try - 1",
        "try error(1) catch -2",
        "try -error? catch 1",
        "try 2 as $x | $x catch 0",
        "try 2 as $x | error($x) catch .",
        "try error(1) as $x | $x catch 0",
        "try error(\"a\") catch . as $e | $e",
        "try 1 as $x | $x * 2 catch 0",
        "try -1 as $x | $x catch 0",
        "try error(\"a\")[0] catch .",
        "try error(\"ab\")[1] catch .",
        "try error(\"abc\") catch .[0]",
        "try error(\"a\") catch .[0:1]",
        "reduce (try 1 catch 2) as $x (0; .+$x)",
        "foreach (try error(1) catch 5) as $x (0; .+$x)",
        "-try 4",
        "try(-4)",
        "try .a? catch 9",
        "try .[]? catch 9",
        "try error.a catch 1",
        "try error.a",
        "try length.a catch 1",
        "try .a.b catch 1",
        "try .[\"a\"] catch 1",
        "try empty.a",
        "try (error).a catch 1",
        "try error catch .a",
        "try error catch (.)?",
        "try error.a? catch 1",
        "try error.a?",
        "try (.).a.b? catch 4",
        "try length.a? catch 7",
        "try -error.a? catch 1",
        "try -length.a? catch 7",
        "{a: (try 1 catch 2)}",
        "{a: (try error(1) catch 2)}",
        "{a: error}",
        "{a: error(1)}",
        "{a: has(try error(1) catch 2)}",
        "{a: \"\\(try error(1) catch 2)\"}",
        "{a: error(try 1 catch 2)}",
        "reduce error as $x (null; .)",
        "error?",
        "length.a?",
        "try 1 catch 2?",
        "try .a = 1",
        "try .a catch .b = 1",
        "try error(\"a\") = 1",
        "try error(\"a\") catch .b = 1",
        "try error({\"a\":1}) catch .a = 9",
        "try error({\"a\":1}) catch . = 9",
        "try error([1,2,3]) catch .[1] = 9",
        "try error([1,2]) catch .[] = 9",
        "try error(\"abcd\") catch .[1:3] = \"XY\"",
        "try error(1) catch \"x\" = 2",
        "try error(null) catch . = 7",
        "try .foo[-1] = 0",
        "try (.foo[-1] = 0) catch .",
        "try (.a[-2] = 0) catch .",
        "(try .a)? = 1",
        "try (.a)? = 1",
        ".a? = 1",
        "try (1|.a) catch .b = 1",
        "error = 1",
        "error(1) = 2",
        ". = error(1)",
        "try empty = 1",
        "{try:1, catch:2, error:3}",
        "1 as $try | 2 as $catch | $try + $catch",
        "1, error(\"x\")",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
    compare_exact("err ascii", &["-n", "-a", "--", "error(1)"], b"");
    compare_exact("err raw", &["-n", "-r", "--", "error(\"m\")"], b"");
    compare_exact(
        "err sort",
        &["-n", "-S", "--", "error({\"b\":1,\"a\":2})"],
        b"",
    );
    compare_exact(
        "catch sort",
        &["-n", "-S", "--", "try error({\"b\":1,\"a\":2}) catch ."],
        b"",
    );
    compare_exact("err pretty", &["-n", "--", "error({\"b\":1,\"a\":2})"], b"");
    let with_input = [
        ("error", "\"hi\"\n"),
        ("try error catch .", "\"hi\"\n"),
        ("try (.a) catch .", "1\n"),
        ("try .[] catch .", "null\n"),
        ("try -. catch .", "\"very-long-string\"\n"),
        ("try map(error(.)) catch .", "[1]\n"),
        ("try error as $x | $x catch 0", "\"z\"\n"),
        ("try .[0] catch 9", "1\n"),
        ("try .a? catch 9", "1\n"),
        ("try .[0]? catch 9", "1\n"),
        ("try .[0]? catch 9", "\"ab\"\n"),
        ("try length.a", "null\n"),
        ("try (.).a? catch 9", "1\n"),
        ("try (.).a catch 9", "1\n"),
        ("try (.).a catch 9", "{\"a\":1}\n"),
        ("try .a.b? catch 9", "1\n"),
        ("try (.).a.b? catch 4", "1\n"),
        ("try (.).a.b? catch 4", "null\n"),
        ("try -(.).a.b? catch 4", "1\n"),
        ("(.).a?", "1\n"),
        (".a.b?", "1\n"),
        ("(.a)?", "1\n"),
        ("(try .a.b catch .c) = 9", "{}\n"),
        ("(try .[0] catch .[1]) = 8", "[1,2]\n"),
        ("(try .[5] catch .[1]) = 8", "[1,2]\n"),
        ("(try .a catch .b) |= .+10", "{\"a\":1,\"b\":2}\n"),
        ("(try .z catch .b) |= .+10", "{\"a\":1,\"b\":2}\n"),
        ("try .[0] = 5", "1\n"),
        ("try error(\"x\") = 2", "1\n"),
        ("try (.a, error(\"x\")) = 1", "{}\n"),
        ("try (.a, error(\"x\"), .b) = 1", "{}\n"),
        ("try (.a, error(\"x\")) catch empty = 1", "{}\n"),
        ("try (.a, error(\"x\")) catch . = 1", "{}\n"),
        ("try (.a, error(\"x\")) catch .c = 1", "{}\n"),
        ("(try .a catch .b) = 9", "1\n"),
        ("try error(null) catch . = 7", "1\n"),
        ("try error(null) catch . = 7", "null\n"),
        ("try .a catch . = 9", "1\n"),
        ("try .a catch error(\"z\") = 5", "1\n"),
        ("try .a catch error(\"z\") = 5", "{}\n"),
        ("try (.a, .nope, .b) = 9", "{\"a\":1,\"b\":2}\n"),
        ("try (.a, .b) = 1", "{}\n"),
        ("(try .a, .b) = 1", "{}\n"),
        ("try (.foo[-1] = 0) catch .", "{\"foo\":[1]}\n"),
        ("try (.foo[-1] = 0) catch .", "{\"foo\":1}\n"),
        ("try (.foo[-1] = 0) catch .", "1\n"),
        (".a |= try error(\"z\") catch \"ok\"", "{\"a\":1}\n"),
        (". |= try error(2)", "1\n"),
        ("(.a, error(1)) = 2", "{}\n"),
        ("error", "1\n\"z\"\ntrue\n"),
        (
            "map(try .a[] catch ., try .a.[] catch ., .a[]?, .a.[]?)",
            "[{\"a\": [1,2]}, {\"a\": 123}]\n",
        ),
        (
            "[.[]|try if . == 0 then error(\"foo\") elif . == 1 then .a elif . == 2 then empty else . end catch .]",
            "[0,1,2,3]\n",
        ),
        (".[] | try error catch .", "[1,null,2]\n"),
        (". |= try . catch .", "1\n"),
        (". |= try 2", "1\n"),
        (". |= try 2 catch 3", "1\n"),
    ];
    for (program, input) in with_input {
        compare_exact(program, &["-c", "--", program], input.as_bytes());
    }
}

/// `?` after a chain still wraps the whole chain. jq marks only the last suffix.
#[test]
fn question_mark_keeps_wrapping_the_chain() {
    require_jq();
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let programs = ["1 | (.).a.b?", "length.a.b?"];
    for program in programs {
        let args = ["-n", "-c", "--", program];
        let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
        let got = run(bin, &owned, b"");
        assert_eq!(got.stdout, b"", "{program}");
        assert_eq!(got.stderr, b"", "{program}");
        assert_eq!(got.status.code(), Some(0), "{program}");
        let jq = run("jq", &owned, b"");
        assert_eq!(jq.status.code(), Some(5), "{program}");
        let err = String::from_utf8_lossy(&jq.stderr);
        assert!(
            err.contains("Cannot index number with string \"a\""),
            "{program}: {err}"
        );
    }
}

#[test]
fn dates_match_jq() {
    require_jq();
    let programs = [
        "0 | todate",
        "1 | todate",
        "1.9 | todate",
        "-0 | todate",
        "-1 | todate",
        "-1.9 | todate",
        "-0.1 | todate",
        "nan | todate",
        "86400 | todate",
        "1600000000 | todate",
        "1600000000 | todate | fromdate",
        "1e10 | todate",
        "1e16 | todate",
        "-1e16 | todate",
        "-315619200 | todate",
        "-2208988800 | todate",
        "[1970,0,1,0,0,0,4,0] | todate",
        "[2015,2,5,23,51,47,4,63] | strftime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "0 | gmtime",
        "1 | gmtime",
        "1.9 | gmtime",
        "-0 | gmtime",
        "-0.1 | gmtime",
        "-0.9 | gmtime",
        "-1 | gmtime",
        "-1.1 | gmtime",
        "-1.9 | gmtime",
        "nan | gmtime",
        "nan | gmtime | .[5] == null",
        "nan | gmtime | .[5] | length",
        "86400 | gmtime",
        "1425599507 | gmtime",
        "1425599507 | gmtime | .[0] == 2015",
        "1600000000.9 | gmtime",
        "1e16 | gmtime",
        "1e16 | gmtime | mktime",
        "-1e16 | gmtime",
        "9007199254740993 | gmtime | .[0]",
        "0 | gmtime | mktime",
        "1.9 | gmtime | mktime",
        "1600000000 | gmtime | mktime",
        "86400 | gmtime | mktime | todate",
        "[1970,0,1,0,0,0,4,0] | mktime",
        "[1970,0,1,0,0,0,0,0] | mktime",
        "[1970,0,1,0,0,0,4,99] | mktime",
        "[1970,0,1,0,0,60,4,0] | mktime",
        "[1970,0,1,0,0,61,4,0] | mktime",
        "[1970,12,1,0,0,0,4,0] | mktime",
        "[1970,0,0,0,0,0,4,0] | mktime",
        "[1970,0,1,24,0,0,4,0] | mktime",
        "[1970.9,0.9,1.9,0.9,0.9,0.9,4.9,0.9] | mktime",
        "[1970,0,1,0,0,1.9,4,0] | mktime",
        "[1970,0,1,0,0,nan,4,0] | mktime",
        "[1970,0,1,0,0,infinite,4,0] | mktime",
        "[1970,0,1,0,0,0,4,0,\"x\"] | mktime",
        "[1970,0,1,0,0,0,4,0,null] | mktime",
        "[infinite,0,1,0,0,0,4,0] | mktime",
        "\"1970-01-01T00:00:00Z\" | fromdate",
        "\"1970-01-01T00:00:01Z\" | fromdate",
        "\"2000-01-01T00:00:00Z\" | fromdate",
        "\"2000-02-29T00:00:00Z\" | fromdate",
        "\"2016-12-31T23:59:59Z\" | fromdate | todate",
        "\"1960-01-01T00:00:00Z\" | fromdate",
        "\"1900-01-01T00:00:00Z\" | fromdate",
        "\"1970-01-01T00:00:00Z \" | fromdate",
        "\"1970-01-01T00:00:00Z\\n\" | fromdate",
        "\"1970-01-01T00:00:00Z\\t\" | fromdate",
        "\"1970-01-01T00:00:00Z\\r\" | fromdate",
        "\"1970-01-01T00:00:00Z\\u000b\" | fromdate",
        "\"1970-01-01T00:00:00Z\\u000c\" | fromdate",
        "\"1970-01-01T00:00:00Z junk\" | fromdate",
        "\"1970-01-01T00:00:00Z\\u00a0\" | fromdate",
        "0 | strftime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "0 | strftime(\"%Y %m %d %H %M %S\")",
        "0 | strftime(\"%Z\")",
        "0 | strftime(\"year %Y\")",
        "0 | strftime(\"年%Y月\")",
        "0 | strftime(\"hello\")",
        "0 | strftime(\"%Y\", \"%m\")",
        "(0, 86400) | strftime(\"%d\")",
        "1.9 | strftime(\"%S\")",
        "1.9 | gmtime | strftime(\"%S\")",
        "[1970,0,1,0,0,1.9,4,0] | strftime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "[1970,0,1,0,0,1.9,4,0] | strftime(\"%S\")",
        "nan | strftime(\"%Y\")",
        "0 | strftime(empty)",
        "\"x\" | strftime(\"%Y\")?",
        "true | todate?",
        "(\"x\" | gmtime)?, 1",
        "{todate: 1}",
        "{todate}",
        "{gmtime: 1}",
        "{strftime: 1}",
        "[0, 1.9, 86400] | map(todate)",
        "0 | strftime (\"%Y\")",
        "strftime(\"%Y\")",
        "0 | gmtime | strftime(\"%Y-%m-%d\")",
        "2 | gmtime | .[0]",
        "[0] | .[0] |= todate",
        "infinite | todate",
        "-infinite | todate",
        "true | todate",
        "\"x\" | todate",
        "null | todate",
        "[1970,0,1,0,0,0] | todate",
        "infinite | gmtime",
        "-infinite | gmtime",
        "true | gmtime",
        "\"0\" | gmtime",
        "null | gmtime",
        "[0] | gmtime",
        "1e17 | gmtime",
        "1e18 | gmtime",
        "-1 | gmtime | mktime",
        "[1969,11,31,23,59,59,3,364] | mktime",
        "[1969,11,31,23,59,58,3,364] | mktime",
        "[1970,0,1,0,0] | mktime",
        "[1970,0,1,0,0,0] | mktime",
        "[1970,0,1,0,0,0,4] | mktime",
        "[1970,0,1,0,0,0,4,null] | mktime",
        "[1970,0,1,0,0,0,4,\"0\"] | mktime",
        "[true,0,1,0,0,0,4,0] | mktime",
        "true | mktime",
        "0 | mktime",
        "{} | mktime",
        "[] | mktime",
        "\"1970-01-01T00:00:00Zjunk\" | fromdate",
        "\"1970-01-01T00:00:00ZZ\" | fromdate",
        "\"1970-01-01T00:00:00\" | fromdate",
        "\"1970-01-01t00:00:00Z\" | fromdate",
        "\"1970-01-01T00:00:00.5Z\" | fromdate",
        "\"1970-01-01T00:00:00+00:00\" | fromdate",
        "\"bad\" | fromdate",
        "0 | fromdate",
        "true | fromdate",
        "null | fromdate",
        "[] | fromdate",
        "\"1969-12-31T23:59:59Z\" | fromdate",
        "\"1969-12-31T23:59:58Z\" | fromdate",
        "\"0001-01-01T00:00:00Z\" | fromdate",
        "\"1970\\n-01\" | fromdate",
        "\"say \\\"hi\\\"\" | fromdate",
        "true | strftime(\"%Y\")",
        "\"x\" | strftime(\"%Y\")",
        "null | strftime(\"%Y\")",
        "[1970,0,1,0,0,0] | strftime(\"%Y\")",
        "[true,0,1,0,0,0,4,0] | strftime(\"%Y\")",
        "[1970,0,1,0,0,0,4,0] | strftime(1)",
        "[1970,0,1] | strftime(1)",
        "[true,0,1,0,0,0,4,0] | strftime(1)",
        "infinite | strftime(\"%Y\")",
        "infinite | strftime(1)",
        "infinite | strftime(\"%Y\", \"%m\")",
        "0 | strftime(\"\")",
        "0 | strftime(\"%Y\", 1/0)",
        "0 | strftime(1/0)",
        "0 | strftime(1/0, \"%Y\")",
        "1, (\"x\" | todate)",
        "(\"x\" | todate), 1",
        "1, (\"x\" | gmtime)",
        "(\"no\" | fromdate), 1",
        "\"1970-01-01T00:00:00Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:01Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"2000-02-29T12:30:45Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1900-03-01T00:00:00Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"2000-03-01T00:00:00Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1969-12-31T23:59:59Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"2020-07-01T00:00:00Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z \" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z junk\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z\\n\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z\\tmore\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z\\r\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z\\u000b\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z\\u000c\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z\\u00a0\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"\" | strptime(\"\")",
        "\" \" | strptime(\"\")",
        "\"1970\" | strptime(\"\")",
        "\"1970\" | strptime(\"%Y\")",
        "\"1970-01\" | strptime(\"%Y-%m\")",
        "\"70\" | strptime(\"%y\")",
        "\"69\" | strptime(\"%y\")",
        "\"68\" | strptime(\"%y\")",
        "\"00\" | strptime(\"%y\")",
        "\"99\" | strptime(\"%y\")",
        "\"Jan\" | strptime(\"%b\")",
        "\"Feb\" | strptime(\"%b\")",
        "\"Dec\" | strptime(\"%b\")",
        "\"jan\" | strptime(\"%b\")",
        "\"Thu\" | strptime(\"%a\")",
        "\" 1\" | strptime(\"%e\")",
        "\"1\" | strptime(\"%e\")",
        "\"10\" | strptime(\"%e\")",
        "\" 5\" | strptime(\"%e\")",
        "\"1970-01-01 001\" | strptime(\"%Y-%m-%d %j\")",
        "\"2000-02-29 060\" | strptime(\"%Y-%m-%d %j\")",
        "\"2000-03-01 061\" | strptime(\"%Y-%m-%d %j\")",
        "\"1970 extra\" | strptime(\"%Y\")",
        "\"1970 \\\"z\\\"\" | strptime(\"%Y\")",
        "\"1970\\n01\" | strptime(\"%Y %m\")",
        "0 | gmtime == (\"1970-01-01T00:00:00Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\"))",
        "0 | strftime(\"%Y-%m-%dT%H:%M:%SZ\") | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00Z\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\") | mktime",
        "\"1970-01-01T00:00:00Z junk\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\") | mktime",
        "\"1970\" | strptime(\"%Y\", \"%m\")",
        "\"1970-01\" | strptime(\"%Y-%m\", 1/0)",
        "\"\" | strptime(1/0, \"%Y\")",
        "1 | strptime(1/0)",
        "\"bad\" | strptime(\"%Y\")?",
        "(\"bad\" | strptime(\"%Y\")), 1",
        "1, (\"bad\" | strptime(\"%Y\"))",
        "\"1970\" | strptime (\"%Y\")",
        "1 | strptime(\"%Y\")",
        "\"x\" | strptime(1)",
        "1 | strptime(1)",
        "true | strptime(\"%Y\")",
        "null | strptime(\"%Y\")",
        "[] | strptime(\"%Y\")",
        "\"1970\" | strptime(null)",
        "\"1970\" | strptime(true)",
        "\"bad\" | strptime(\"%Y-%m-%d\")",
        "\"1970-01-01T00:00:00Zjunk\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"1970-01-01T00:00:00ZZ\" | strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "\"\" | strptime(\"%Y\")",
        "\"1970\" | strptime(\"%Y-%m-%d\")",
        "\"%Y\" | strptime(\"%Y\")",
        "\"1970\" | strptime(\"%Y extra\")",
        "0 | strftime(\"%j\")",
        "0 | strftime(\"%a\")",
        "0 | strftime(\"%b\")",
        "0 | strftime(\"%e\")",
        "0 | strftime(\"%y\")",
        "0 | strftime(\"%z\")",
        "0 | strftime(\"%Y %j %a %b %e %y\")",
        "86400 | strftime(\"%j %a %e\")",
        "[1970,0,1,0,0,0,4,0] | strftime(\"%a %b %d %e %y %j\")",
        "[1970,0,5,0,0,0,4,4] | strftime(\"%e\")",
        "[1970,0,10,0,0,0,4,9] | strftime(\"%e\")",
        "1593561600 | strftime(\"%z %Z %a %j\")",
        "\"1970-01-01T00:00:00+0000\" | strptime(\"%Y-%m-%dT%H:%M:%S%z\")",
        "\"1970-01-01T00:00:00-0700\" | strptime(\"%Y-%m-%dT%H:%M:%S%z\")",
        "empty | now",
        "now > 0",
        "now > 1600000000",
        "1 | now > 1600000000",
        "true | now > 1600000000",
        "now | gmtime | length",
        "now | gmtime | .[0] > 2020",
        "1,2 | now > 0",
        "{now: 1}",
        "{now}",
        "{strptime: 1}",
        "{strptime}",
        "0 | localtime",
        "1 | localtime",
        "1.9 | localtime",
        "-0 | localtime",
        "-0.1 | localtime",
        "-0.9 | localtime",
        "-1 | localtime",
        "-1.1 | localtime",
        "-1.9 | localtime",
        "nan | localtime",
        "nan | localtime | .[5] == null",
        "nan | localtime | .[5] | length",
        "86400 | localtime",
        "1425599507 | localtime",
        "1600000000.9 | localtime",
        "1e16 | localtime",
        "1e16 | strflocaltime(\"%Y\")",
        "-1e16 | localtime",
        "-1e16 | strflocaltime(\"%Y\")",
        "9007199254740993 | localtime | .[0]",
        "9007199254740993 | strflocaltime(\"%Y\")",
        "0 | localtime | .[5]",
        "0 | gmtime == localtime",
        "1593561600 | localtime",
        "localtime",
        "0 | strflocaltime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "0 | strflocaltime(\"%Y %m %d %H %M %S\")",
        "0 | strflocaltime(\"%Z\")",
        "0 | strflocaltime(\"%z\")",
        "0 | strflocaltime(\"%j\")",
        "0 | strflocaltime(\"%a\")",
        "0 | strflocaltime(\"%b\")",
        "0 | strflocaltime(\"%e\")",
        "0 | strflocaltime(\"%y\")",
        "0 | strflocaltime(\"%Y %m %d %H %M %S %Z %z %j %a %b %e %y\")",
        "0 | strflocaltime(\"%Y\", \"%m\")",
        "(0, 86400) | strflocaltime(\"%d\")",
        "1.9 | strflocaltime(\"%S\")",
        "1.9 | localtime | strflocaltime(\"%S\")",
        "[1970,0,1,0,0,1.9,4,0] | strflocaltime(\"%Y-%m-%dT%H:%M:%SZ\")",
        "[1970,0,1,0,0,1.9,4,0] | strflocaltime(\"%S\")",
        "[1970,0,1,0,0,0,4,0] | strflocaltime(\"%Y-%m-%dT%H:%M:%S %Z %z\")",
        "[1970,0,1,0,0,0,4,0] | strftime(\"%Z %z\") == ([1970,0,1,0,0,0,4,0] | strflocaltime(\"%Z %z\"))",
        "0 | strflocaltime(empty)",
        "\"x\" | localtime?",
        "(\"x\" | localtime)?, 1",
        "{localtime: 1}",
        "{localtime}",
        "{strflocaltime: 1}",
        "{strflocaltime}",
        "[0, 1.9, 86400] | map(localtime)",
        "0 | strflocaltime (\"%Y\")",
        "strflocaltime(\"%Y\")",
        "0 | localtime | strflocaltime(\"%Y-%m-%d\")",
        "1593561600 | strflocaltime(\"%Y-%m-%d %H:%M:%S %Z %z %a %j\")",
        "1593561600 | localtime | strflocaltime(\"%Y-%m-%d %H:%M:%S\")",
        "86400 | strflocaltime(\"%j %a %e\")",
        "[1970,0,1,0,0,0,4,0] | strflocaltime(\"%a %b %d %e %y %j\")",
        "[1970,0,5,0,0,0,4,4] | strflocaltime(\"%e\")",
        "[1970,0,10,0,0,0,4,9] | strflocaltime(\"%e\")",
        "infinite | localtime",
        "-infinite | localtime",
        "true | localtime",
        "\"0\" | localtime",
        "null | localtime",
        "[0] | localtime",
        "1e17 | localtime",
        "1e18 | localtime",
        "true | strflocaltime(\"%Y\")",
        "\"x\" | strflocaltime(\"%Y\")",
        "null | strflocaltime(\"%Y\")",
        "[1970,0,1,0,0,0] | strflocaltime(\"%Y\")",
        "[true,0,1,0,0,0,4,0] | strflocaltime(\"%Y\")",
        "[1970,0,1,0,0,0,4,0] | strflocaltime(1)",
        "[1970,0,1] | strflocaltime(1)",
        "[true,0,1,0,0,0,4,0] | strflocaltime(1)",
        "0 | strflocaltime(\"\")",
        "0 | strflocaltime(\"%Y\", 1/0)",
        "0 | strflocaltime(1/0)",
        "0 | strflocaltime(1/0, \"%Y\")",
        "1, (\"x\" | localtime)",
        "(\"x\" | localtime), 1",
        "empty | localtime",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
    let streams = [
        ("todate", "0\n1\n\"x\"\n"),
        ("gmtime", "0\n1\n\"x\"\n"),
        ("todate", "0\n\"x\"\n1\n"),
        ("fromdate", "\"1970-01-01T00:00:00Z\"\n\"bad\"\n"),
        ("gmtime", "0\ntrue\n1\n"),
        (
            "strptime(\"%Y-%m-%dT%H:%M:%SZ\")",
            "\"1970-01-01T00:00:00Z\"\n\"bad\"\n",
        ),
        ("strptime(\"%Y\")", "\"bad\"\n\"1970\"\n"),
        ("now > 0", "1\ntrue\n"),
        ("localtime", "0\n1\n\"x\"\n"),
        ("localtime", "0\ntrue\n1\n"),
        ("strflocaltime(\"%Y\")", "0\n\"x\"\n1\n"),
        ("strflocaltime(\"%Y\")", "0\ntrue\n"),
    ];
    for (program, input) in streams {
        compare_exact(program, &["-c", "--", program], input.as_bytes());
    }
    for tz in [
        "UTC",
        "GMT",
        "America/Los_Angeles",
        "Asia/Kolkata",
        "Europe/Istanbul",
    ] {
        compare_tz("epoch zone", "0 | strftime(\"%Z\")", tz);
        compare_tz("summer zone", "1593561600 | strftime(\"%Z\")", tz);
        compare_tz(
            "utc fields",
            "1593561600 | strftime(\"%Y-%m-%dT%H:%M:%SZ\")",
            tz,
        );
        compare_tz("epoch offset", "0 | strftime(\"%z\")", tz);
        compare_tz("summer offset", "1593561600 | strftime(\"%z %a %b\")", tz);
        compare_tz(
            "parsed offset",
            "\"1970-01-01T00:00:00+0000\" | strptime(\"%Y-%m-%dT%H:%M:%S%z\")",
            tz,
        );
        compare_tz(
            "parsed negative offset",
            "\"1970-01-01T00:00:00-0700\" | strptime(\"%Y-%m-%dT%H:%M:%S%z\")",
            tz,
        );
        compare_tz("local epoch", "0 | localtime", tz);
        compare_tz("local summer", "1593561600 | localtime", tz);
        compare_tz("local fraction", "1.9 | localtime", tz);
        compare_tz("local before epoch", "-1.1 | localtime", tz);
        compare_tz(
            "local format epoch",
            "0 | strflocaltime(\"%Y-%m-%dT%H:%M:%S %Z %z %a %j\")",
            tz,
        );
        compare_tz(
            "local format summer",
            "1593561600 | strflocaltime(\"%Y-%m-%d %H:%M:%S %Z %z %a %j\")",
            tz,
        );
        compare_tz(
            "local format letters",
            "1593561600 | strflocaltime(\"%Y %m %d %H %M %S %Z %z %j %a %b %e %y\")",
            tz,
        );
        compare_tz(
            "array local zone",
            "[1970,0,1,0,0,0,4,0] | strflocaltime(\"%Z %z\")",
            tz,
        );
        compare_tz("same instant", "0 | gmtime == localtime", tz);
        compare_tz(
            "epoch names",
            "0 | strftime(\"%A %B %I %p %u %w %s %D %F %T %R %c %%\")",
            tz,
        );
        compare_tz(
            "local names",
            "0 | strflocaltime(\"%A %B %I %p %u %w %s %D %F %T %R %c %%\")",
            tz,
        );
        compare_tz(
            "summer names",
            "1593561600 | strftime(\"%A %B %I %p %u %w %s %c\")",
            tz,
        );
        compare_tz(
            "local summer names",
            "1593561600 | strflocaltime(\"%A %B %I %p %u %w %s %c\")",
            tz,
        );
        compare_tz("parsed seconds", "\"0\" | strptime(\"%s\")", tz);
        compare_tz("parsed afternoon", "\"1 PM\" | strptime(\"%I %p\")", tz);
    }
}

#[test]
fn date_letters_match_jq() {
    require_jq();
    let programs = [
        "0 | strftime(\"%A\")",
        "0 | strftime(\"%B\")",
        "0 | strftime(\"%I\")",
        "0 | strftime(\"%p\")",
        "0 | strftime(\"%u\")",
        "0 | strftime(\"%w\")",
        "0 | strftime(\"%s\")",
        "0 | strftime(\"%D\")",
        "0 | strftime(\"%F\")",
        "0 | strftime(\"%T\")",
        "0 | strftime(\"%R\")",
        "0 | strftime(\"%c\")",
        "0 | strftime(\"%%\")",
        "0 | strflocaltime(\"%A %B %I %p %u %w %s %D %F %T %R %c %%\")",
        "3600 | strftime(\"%I %p %R %T\")",
        "43200 | strftime(\"%I %p\")",
        "46800 | strftime(\"%I %p\")",
        "86399 | strftime(\"%I %p %H\")",
        "1.9 | strftime(\"%s %S\")",
        "1600000000 | strftime(\"%A %B %I %p %u %w %s %D %F\")",
        "1600000000 | strflocaltime(\"%A %B %I %p %u %w %s %c\")",
        "[1970,0,1,0,0,0,4,0] | strftime(\"%A %a %B %b %u %w %j %s\")",
        "[1970,0,1,0,0,0,0,0] | strftime(\"%A %u %w\")",
        "[1970,0,4,0,0,0,0,3] | strftime(\"%A %u %w\")",
        "[1970,0,1,0,30,0,4,0] | strftime(\"%I %p %R %T\")",
        "[1970,0,1,12,0,0,4,0] | strftime(\"%I %p\")",
        "[1970,0,1,15,0,0,4,0] | strftime(\"%I %p %H\")",
        "[1970,6,1,0,0,0,3,181] | strftime(\"%A %B %u %w\")",
        "[1970,0,1,0,0,1.9,4,0] | strftime(\"%s %S %c\")",
        "[2015,2,5,23,51,47,4,63] | strftime(\"%s\")",
        "[2015,2,5,23,51,47,4,63] | strflocaltime(\"%s\")",
        "0 | strftime(\"100%%\")",
        "0 | strftime(\"%Y%%\")",
        "0 | strftime(\"%%Y\")",
        "0 | strftime(\"%A%B%I%p%u%w%s%D%F%T%R%c%%\")",
        "0 | strftime(\"%Y\", \"%A\")",
        "[1970,0,1,0,0,0,4,0] | strflocaltime(\"%Y\", \"%A\")",
        "0 | gmtime | strftime(\"%c\")",
        "0 | localtime | strflocaltime(\"%c\")",
        "0 | strftime(\"%c\") | strptime(\"%c\")",
        "0 | strflocaltime(\"%c\") | strptime(\"%c\")",
        "\"Thursday\" | strptime(\"%A\")",
        "\"January\" | strptime(\"%B\")",
        "\"Thu\" | strptime(\"%A\")",
        "\"Jan\" | strptime(\"%B\")",
        "\"12 AM\" | strptime(\"%I %p\")",
        "\"12 PM\" | strptime(\"%I %p\")",
        "\"01\" | strptime(\"%I\")",
        "\"4\" | strptime(\"%u\")",
        "\"4\" | strptime(\"%w\")",
        "\"0\" | strptime(\"%s\")",
        "\"1600000000\" | strptime(\"%s\")",
        "\"01/02/70\" | strptime(\"%D\")",
        "\"1/2/70\" | strptime(\"%D\")",
        "\"1970-01-02\" | strptime(\"%F\")",
        "\"1970-01-02 junk\" | strptime(\"%F\")",
        "\"13:05:09\" | strptime(\"%T\")",
        "\"13:05\" | strptime(\"%R\")",
        "\"%\" | strptime(\"%%\")",
        "\"1970%\" | strptime(\"%Y%%\")",
        "\"x%\" | strptime(\"x%%\")",
        "\"Thursday, January 1, 1970\" | strptime(\"%A, %B %d, %Y\")",
        "\"nope\" | strptime(\"%A\")",
        "\"\" | strptime(\"%A\")",
        "\"1970\" | strptime(\"%A\")",
        "\"x\" | strptime(\"%%\")",
        "\"1970\" | strptime(\"%Y\", \"%A\")",
        "true | strftime(\"%A\")",
        "null | strftime(\"%s\")",
        "0 | strptime(\"%F\")",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
}

/// jq aborts when `strftime` is given a number and a format that is not a
/// string. The message here is the one jq prints for an array. Letters
/// other than the ones these filters accept are not compared with jq.
#[test]
fn strftime_letters_are_limited() {
    let cases = [
        (r#"0 | strftime("%")"#, "strftime does not support %"),
        (r#"0 | strftime("%Y%")"#, "strftime does not support %"),
        (r#"0 | strftime("%n")"#, "strftime does not support %n"),
        (r#"0 | strftime("%t")"#, "strftime does not support %t"),
        (r#"0 | strftime("%EY")"#, "strftime does not support %E"),
        (r#"0 | strftime("%-Y")"#, "strftime does not support %-"),
        (r#"0 | strftime("%10Y")"#, "strftime does not support %1"),
        (r#"0 | strftime("%Q%Y")"#, "strftime does not support %Q"),
        (r#"0 | strftime(1)"#, "strftime/1 requires a string format"),
        (
            r#"0 | strftime(null)"#,
            "strftime/1 requires a string format",
        ),
        (
            r#"0 | strftime(true)"#,
            "strftime/1 requires a string format",
        ),
        (
            r#"0 | strftime(["%Y"])"#,
            "strftime/1 requires a string format",
        ),
    ];
    for (program, message) in cases {
        let stderr = format!("jq: error (at <unknown>): {message}\n");
        assert_rejects(
            program,
            &["-n", "-c", "--", program],
            b"",
            stderr.as_bytes(),
            5,
        );
    }
}

/// jq aborts when `strflocaltime` converts a number and the format is not a
/// string, and when that conversion fails. The message is the one jq prints
/// for an array, or the converting error. Letters other than the ones these
/// filters accept are not compared with jq.
#[test]
fn strflocaltime_letters_are_limited() {
    let cases = [
        (
            r#"0 | strflocaltime("%")"#,
            "strflocaltime does not support %",
        ),
        (
            r#"0 | strflocaltime("%Y%")"#,
            "strflocaltime does not support %",
        ),
        (
            r#"0 | strflocaltime("%n")"#,
            "strflocaltime does not support %n",
        ),
        (
            r#"0 | strflocaltime("%t")"#,
            "strflocaltime does not support %t",
        ),
        (
            r#"0 | strflocaltime("%EY")"#,
            "strflocaltime does not support %E",
        ),
        (
            r#"0 | strflocaltime("%-Y")"#,
            "strflocaltime does not support %-",
        ),
        (
            r#"0 | strflocaltime("%10Y")"#,
            "strflocaltime does not support %1",
        ),
        (
            r#"0 | strflocaltime("%Q%Y")"#,
            "strflocaltime does not support %Q",
        ),
        (
            r#"0 | strflocaltime(1)"#,
            "strflocaltime/1 requires a string format",
        ),
        (
            r#"0 | strflocaltime(null)"#,
            "strflocaltime/1 requires a string format",
        ),
        (
            r#"0 | strflocaltime(true)"#,
            "strflocaltime/1 requires a string format",
        ),
        (
            r#"0 | strflocaltime(["%Y"])"#,
            "strflocaltime/1 requires a string format",
        ),
        (
            r#"infinite | strflocaltime("%Y")"#,
            "error converting number of seconds since epoch to datetime",
        ),
        (
            r#"infinite | strflocaltime(1)"#,
            "error converting number of seconds since epoch to datetime",
        ),
        (
            r#"-infinite | strflocaltime("%Y")"#,
            "error converting number of seconds since epoch to datetime",
        ),
        (
            r#"-infinite | strflocaltime(1)"#,
            "error converting number of seconds since epoch to datetime",
        ),
    ];
    for (program, message) in cases {
        let stderr = format!("jq: error (at <unknown>): {message}\n");
        assert_rejects(
            program,
            &["-n", "-c", "--", program],
            b"",
            stderr.as_bytes(),
            5,
        );
    }
}

/// `strptime` rejects the same letters `strftime` rejects. jq would hand
/// those letters to the C library, so the message is not compared with jq.
#[test]
fn strptime_letters_are_limited() {
    let cases = [
        (r#""1970" | strptime("%")"#, "strptime does not support %"),
        (r#""1970" | strptime("%Y%")"#, "strptime does not support %"),
        (r#""1970" | strptime("%n")"#, "strptime does not support %n"),
        (r#""1970" | strptime("%t")"#, "strptime does not support %t"),
        (
            r#""1970" | strptime("%Ey")"#,
            "strptime does not support %E",
        ),
        (
            r#""1970" | strptime("%-y")"#,
            "strptime does not support %-",
        ),
        (
            r#""1970" | strptime("%3j")"#,
            "strptime does not support %3",
        ),
        (r#""1970" | strptime("%Q")"#, "strptime does not support %Q"),
    ];
    for (program, message) in cases {
        let stderr = format!("jq: error (at <unknown>): {message}\n");
        assert_rejects(
            program,
            &["-n", "-c", "--", program],
            b"",
            stderr.as_bytes(),
            5,
        );
    }
}

/// The two processes read the clock a moment apart. The number itself is
/// not compared with jq.
#[test]
fn now_is_close_to_jq() {
    require_jq();
    let args = ["-n", "-c", "--", "now"];
    let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let got = run(env!("CARGO_BIN_EXE_tumulus"), &owned, b"");
    let expected = run("jq", &owned, b"");
    let ours = one_stamp(&got);
    let theirs = one_stamp(&expected);
    let gap = (ours - theirs).abs();
    assert!(
        gap < 5.0,
        "now differs by {gap}: tumulus {ours}, jq {theirs}"
    );
}

fn one_stamp(output: &std::process::Output) -> f64 {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let text = std::str::from_utf8(&output.stdout).expect("utf-8");
    let stamp: f64 = text
        .trim()
        .parse()
        .unwrap_or_else(|err| panic!("now printed {text:?}: {err}"));
    assert!(stamp.is_finite() && stamp > 1_600_000_000.0, "{stamp}");
    stamp
}

fn compare_tz(name: &str, program: &str, tz: &str) {
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let args = ["-n", "-c", "--", program];
    let got = Command::new(bin)
        .args(args)
        .env("TZ", tz)
        .output()
        .unwrap_or_else(|err| panic!("could not start tumulus: {err}"));
    let expected = Command::new("jq")
        .args(args)
        .env("TZ", tz)
        .output()
        .unwrap_or_else(|err| panic!("could not start jq: {err}"));
    assert_eq!(
        got.stdout,
        expected.stdout,
        "{name}: stdout differs under TZ={tz}\nstderr ours:\n{}\nstderr jq:\n{}",
        String::from_utf8_lossy(&got.stderr),
        String::from_utf8_lossy(&expected.stderr)
    );
    assert_eq!(
        got.stderr, expected.stderr,
        "{name}: stderr differs under TZ={tz}"
    );
    assert_eq!(
        got.status.code(),
        expected.status.code(),
        "{name}: exit status differs under TZ={tz}\nstderr ours:\n{}\nstderr jq:\n{}",
        String::from_utf8_lossy(&got.stderr),
        String::from_utf8_lossy(&expected.stderr)
    );
}

struct DirGuard(std::path::PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scratch(label: &str) -> (std::path::PathBuf, DirGuard) {
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

fn write_bytes(dir: &std::path::Path, name: &str, bytes: &[u8]) -> String {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap_or_else(|err| panic!("write {name}: {err}"));
    path.to_str().expect("utf-8 path").to_string()
}

fn assert_rejects(name: &str, args: &[&str], input: &[u8], stderr: &[u8], code: i32) {
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

#[test]
fn exit_status_matches_jq() {
    require_jq();
    let cases: &[(&str, &[&str], &[u8])] = &[
        ("no input", &["-e", "."], b""),
        ("false", &["-e", "."], b"false\n"),
        ("null", &["-e", "."], b"null\n"),
        ("zero", &["-e", "."], b"0\n"),
        ("empty string", &["-e", "."], b"\"\"\n"),
        ("empty array", &["-e", "."], b"[]\n"),
        ("empty object", &["-e", "."], b"{}\n"),
        ("iterate empty", &["-e", ".[]"], b"[]\n"),
        ("iterate false last", &["-e", ".[]"], b"[true,false]\n"),
        ("iterate true last", &["-e", ".[]"], b"[false,true]\n"),
        ("select false", &["-e", "select(false)"], b"1\n"),
        ("select on false", &["-e", "select(.)"], b"false\n"),
        ("select on zero", &["-e", "select(.)"], b"0\n"),
        ("select false then one", &["-e", "select(.)"], b"false\n1\n"),
        ("null input empty", &["-n", "-e", "empty"], b""),
        ("null input dot", &["-n", "-e", "."], b""),
        ("null input false", &["-n", "-e", "false"], b""),
        ("one then false", &["-n", "-e", "1,false"], b""),
        ("false then one", &["-n", "-e", "false,1"], b""),
        ("one then empty", &["-n", "-e", "1,empty"], b""),
        ("empty then false", &["-n", "-e", "empty,false"], b""),
        ("false then empty", &["-n", "-e", "false,empty"], b""),
        ("empty then empty", &["-n", "-e", "empty,empty"], b""),
        ("join false", &["-je", "-n", "false"], b""),
        ("join empty string", &["-je", "-n", "\"\""], b""),
        ("slurp empty", &["-e", "-s", "."], b""),
        ("raw slurp empty", &["-e", "-Rs", "."], b""),
        ("raw empty", &["-e", "-R", "."], b""),
        ("raw blank line", &["-e", "-R", "."], b"\n"),
        ("raw text false", &["-e", "-R", "."], b"false\n"),
        ("raw string false", &["-er", "-n", "\"false\""], b""),
        ("null wins", &["-n", "-e", "-s", "-R", "."], b"1\n2\n"),
        ("later length clears", &["-e", "length"], b"true\n\"ab\"\n"),
        ("last length fails", &["-e", "length"], b"\"ab\"\ntrue\n"),
        (
            "error then empty",
            &["-e", "if .==true then .a else empty end"],
            b"true\n1\n",
        ),
        (
            "error then false",
            &["-e", "if .==true then .a else false end"],
            b"true\n1\n",
        ),
        (
            "empty then value",
            &["-e", "if .==1 then empty else . end"],
            b"1\n2\n",
        ),
        ("one then false values", &["-e", "."], b"1\nfalse\n"),
        ("false then one values", &["-e", "."], b"false\n1\n"),
        ("partial then error", &["-n", "-e", "1,(0|.a)"], b""),
        ("value then parse error", &["-e", "."], b"false\n{\n"),
        (
            "nan is truthy",
            &["--argjson", "a", "nan", "-ne", "$a"],
            b"",
        ),
        (
            "null arg is false",
            &["--argjson", "a", "null", "-ne", "$a"],
            b"",
        ),
        (
            "nan is not null",
            &["--argjson", "a", "nan", "-n", "-c", "$a == null"],
            b"",
        ),
        (
            "slurp false is truthy",
            &["-e", "-c", "-s", "."],
            b"false\n",
        ),
        ("slurp empty filter", &["-e", "-s", "empty"], b"1\n"),
        ("join raw blank", &["-ej", "-R", "."], b"\n"),
        ("join raw slurp empty", &["-ej", "-Rs", "."], b""),
        (
            "long names null",
            &["--exit-status", "--slurp", "--null-input", "."],
            b"1\n",
        ),
        ("cluster", &["-ncesRajS", "--", "1"], b"ignored"),
        ("se empty", &["-se", "."], b""),
        ("runtime stays", &["-n", "-e", "0|.a"], b""),
        (
            "missing stays",
            &["-e", "-c", ".", "no-such-exit-status.json"],
            b"",
        ),
    ];
    for (name, args, input) in cases {
        compare_exact(name, args, input);
    }

    let (dir, _guard) = scratch("exit");
    let one = write_bytes(&dir, "one.json", b"1\n");
    let two = write_bytes(&dir, "two.json", b"2\n");
    let yes = write_bytes(&dir, "true.json", b"true\n");
    let text = write_bytes(&dir, "ab.json", b"\"ab\"\n");
    let blank = write_bytes(&dir, "blank.json", b"");
    let present = write_bytes(&dir, "present.json", b"{\"a\":1}\n");
    let missing = dir.join("missing.json");
    let missing = missing.to_str().expect("utf-8 path");
    compare_exact(
        "empty across files",
        &["-e", "-c", "if .==1 then empty else . end", &one, &two],
        b"",
    );
    compare_exact(
        "length across files",
        &["-e", "-c", "length", &yes, &blank, &text],
        b"",
    );
    compare_exact(
        "present then missing",
        &["-e", "-c", ".", &present, missing],
        b"",
    );
    compare_exact(
        "missing then present",
        &["-e", "-c", ".", missing, &present],
        b"",
    );
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
fn named_arguments_match_jq() {
    require_jq();
    let cases: &[(&str, &[&str], &[u8])] = &[
        ("no args", &["-n", "-c", "$ARGS"], b""),
        (
            "pretty",
            &["--arg", "b", "1", "--arg", "a", "2", "-n", "$ARGS"],
            b"",
        ),
        (
            "sorted",
            &["--arg", "b", "1", "--arg", "a", "2", "-n", "-S", "$ARGS"],
            b"",
        ),
        (
            "compact",
            &["--arg", "b", "1", "--arg", "a", "2", "-n", "-c", "$ARGS"],
            b"",
        ),
        (
            "args name",
            &["--arg", "ARGS", "z", "-n", "-c", "$ARGS"],
            b"",
        ),
        (
            "args field",
            &["--arg", "ARGS", "z", "-n", "-c", "$ARGS.named.ARGS"],
            b"",
        ),
        (
            "string wins over later json",
            &["--arg", "a", "1", "--argjson", "a", "{", "-n", "-c", "$a"],
            b"",
        ),
        (
            "json wins over later string",
            &["--argjson", "a", "1", "--arg", "a", "2", "-n", "-c", "$a"],
            b"",
        ),
        (
            "first string wins",
            &["--arg", "a", "1", "--arg", "a", "2", "-n", "-c", "$a"],
            b"",
        ),
        (
            "repeat json is not parsed",
            &[
                "--argjson",
                "a",
                "1",
                "--argjson",
                "a",
                "{",
                "-n",
                "-c",
                "$a",
            ],
            b"",
        ),
        (
            "later name still binds",
            &[
                "--arg",
                "a",
                "1",
                "--argjson",
                "a",
                "{",
                "--arg",
                "b",
                "2",
                "-n",
                "-c",
                "$ARGS",
            ],
            b"",
        ),
        (
            "bad json first",
            &["--argjson", "a", "{", "--arg", "a", "1", "-n", "$a"],
            b"",
        ),
        (
            "compact is a value",
            &["--arg", "a", "--compact-output", "-n", "$ARGS"],
            b"",
        ),
        (
            "dash dash value",
            &["--arg", "a", "--", "-n", "-c", "$a"],
            b"",
        ),
        (
            "slurp is a value",
            &["--arg", "a", "--slurp", "$a"],
            b"1\n2\n",
        ),
        (
            "slurp name",
            &["--arg", "--slurp", "1", "-n", "-c", "$ARGS"],
            b"",
        ),
        ("dash name", &["--arg", "--", "a", "-n", "-c", "$ARGS"], b""),
        ("keyword", &["--arg", "if", "x", "-n", "-c", "$if"], b""),
        (
            "hyphen name",
            &["--arg", "a-b", "x", "-n", "-c", "$ARGS"],
            b"",
        ),
        ("empty name", &["--arg", "", "z", "-n", "-c", "$ARGS"], b""),
        (
            "digit name",
            &["--arg", "1a", "y", "-n", "-c", "$ARGS"],
            b"",
        ),
        (
            "unicode name",
            &["--arg", "é", "x", "-n", "-c", "$ARGS"],
            b"",
        ),
        (
            "padded",
            &["--argjson", "a", "  1  ", "-n", "-c", "$a"],
            b"",
        ),
        (
            "padded lines",
            &["--argjson", "a", "\n 1 \n", "-n", "-c", "$a"],
            b"",
        ),
        (
            "trailing space",
            &["--argjson", "a", "true ", "-n", "-c", "$a"],
            b"",
        ),
        (
            "kept fraction",
            &["--argjson", "a", "1.50", "-n", "$a"],
            b"",
        ),
        (
            "leading zero",
            &["--argjson", "a", "01", "-n", "-c", "$a"],
            b"",
        ),
        (
            "dot fraction",
            &["--argjson", "a", ".5", "-n", "-c", "$a"],
            b"",
        ),
        ("plus", &["--argjson", "a", "+1", "-n", "-c", "$a"], b""),
        ("infinity", &["--argjson", "a", "Infinity", "-n", "$a"], b""),
        (
            "negative infinity",
            &["--argjson", "a", "-Infinity", "-n", "$a"],
            b"",
        ),
        ("inf", &["--argjson", "a", "+inf", "-n", "$a"], b""),
        ("nan", &["--argjson", "a", "nan", "-n", "$a"], b""),
        (
            "bom",
            &["--argjson", "a", "\u{feff}1", "-n", "-c", "$a"],
            b"",
        ),
        ("two values", &["--argjson", "a", "1 2", "-n", "$a"], b""),
        ("two lines", &["--argjson", "a", "1\n2", "-n", "$a"], b""),
        ("trailing comma", &["--argjson", "a", "1,", "-n", "$a"], b""),
        ("comment", &["--argjson", "a", "1 #x", "-n", "$a"], b""),
        ("dot", &["--argjson", "a", ".", "-n", "$a"], b""),
        ("empty text", &["--argjson", "a", "", "-n", "$a"], b""),
        ("blank text", &["--argjson", "a", "   ", "-n", "$a"], b""),
        ("open brace", &["--argjson", "a", "{", "-n", "$a"], b""),
        ("arg missing", &["--arg"], b""),
        ("arg one", &["--arg", "a"], b""),
        ("argjson missing", &["--argjson"], b""),
        ("argjson one", &["--argjson", "a"], b""),
        (
            "shadow",
            &["--arg", "a", "hello", "-n", "-c", "1 as $a | $a"],
            b"",
        ),
        (
            "reduce",
            &[
                "--argjson",
                "a",
                "10",
                "-c",
                "reduce .[] as $x ($a; . + $x)",
            ],
            b"[1,2,3]\n",
        ),
        (
            "interpolate",
            &["--arg", "a", "world", "-n", "-c", r#""hello \($a)""#],
            b"",
        ),
        ("zero is truthy", &["--argjson", "a", "0", "-ne", "$a"], b""),
        ("false arg", &["--argjson", "a", "false", "-ne", "$a"], b""),
        (
            "empty string arg",
            &["--argjson", "a", "\"\"", "-ne", "$a"],
            b"",
        ),
        (
            "empty array arg",
            &["--argjson", "a", "[]", "-ne", "$a"],
            b"",
        ),
        (
            "object",
            &["--argjson", "a", "{\"z\":1,\"y\":2}", "-n", "-S", "$a"],
            b"",
        ),
    ];
    for (name, args, input) in cases {
        compare_exact(name, args, input);
    }
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
fn input_and_arguments_match_jq() {
    require_jq();
    let (dir, _guard) = scratch("feed");
    let one = write_bytes(&dir, "one.json", b"1\n");
    let two = write_bytes(&dir, "two.json", b"2\n3\n");
    let bad = write_bytes(&dir, "bad.json", b"not json\n");
    let partial = write_bytes(&dir, "partial.json", b"{\"a\":");
    let empty = write_bytes(&dir, "empty", b"");
    let spanned = write_bytes(&dir, "span-a.json", b"{\"a\":");
    let span_b = write_bytes(&dir, "span-b.json", b"1}\n2\n");
    let early = write_bytes(&dir, "early.json", b"1\n2\n3\n");
    let later = write_bytes(&dir, "later.json", b"not\n");
    let after = write_bytes(&dir, "after.json", b"4\n");
    let lines = write_bytes(&dir, "lines.txt", b"a\nb\nc\n");
    let raw = write_bytes(&dir, "raw.txt", b"hello\nworld");
    let slurp = write_bytes(&dir, "slurp.json", b"1\n{\"a\":2}\n\"x\"\n");
    let missing = dir
        .join("missing.json")
        .to_str()
        .expect("utf-8")
        .to_string();
    let folder = dir.join("folder");
    fs::create_dir(&folder).expect("dir");
    let folder = folder.to_str().expect("utf-8").to_string();
    let prog = write_bytes(&dir, "loc.jq", b"# c\n\n$__loc__\n");

    let cases: &[(&str, &[&str], &[u8])] = &[
        ("input empty", &["-n", "input"], b""),
        ("inputs empty", &["-n", "inputs"], b""),
        ("filename empty", &["-n", "input_filename"], b""),
        ("input one", &["-n", "-c", "input"], b"1\n"),
        ("input no newline", &["-n", "-c", "input"], b"42"),
        ("inputs three", &["-n", "-c", "[inputs]"], b"1\n2\n3\n"),
        ("input twice", &["-n", "-c", "[input, input]"], b"1\n2\n"),
        (
            "input runs out",
            &["-n", "-c", "[input, input, input]"],
            b"1\n2\n",
        ),
        (
            "inputs then input",
            &["-n", "-c", "inputs, input"],
            b"1\n2\n",
        ),
        ("try input", &["-n", "-c", "try input catch ."], b""),
        ("try inputs", &["-n", "-c", "try inputs catch ."], b""),
        ("try input type", &["-n", "-c", "try input catch type"], b""),
        ("dot input", &["-c", "input"], b"1\n2\n3\n"),
        ("dot input one", &["-c", "input"], b"1\n"),
        ("dot inputs", &["-c", "inputs"], b"1\n2\n3\n"),
        ("dot and input", &["-c", "., input"], b"1\n2\n3\n"),
        (
            "limit stops",
            &["-n", "-c", "limit(1; inputs), input"],
            b"1\n2\n3\n",
        ),
        (
            "limit zero",
            &["-n", "-c", "limit(0; inputs), input"],
            b"1\n2\n",
        ),
        (
            "first stops",
            &["-n", "-c", "first(inputs), input"],
            b"1\n2\n3\n",
        ),
        (
            "bad caught",
            &["-n", "-c", "try inputs catch ."],
            b"1\nnot\n2\n",
        ),
        ("bad stops inputs", &["-n", "-c", "inputs"], b"1\nnot\n2\n"),
        (
            "bad resumes",
            &["-n", "-c", "(try inputs catch .), inputs"],
            b"1\nnot\n2\n3\n",
        ),
        (
            "same line skipped",
            &["-n", "-c", "(try inputs catch .), inputs"],
            b"1 not 2\n3\n",
        ),
        ("main parse", &["-c", "."], b"1\nnot\n2\n"),
        (
            "filename after",
            &["-n", "-c", "[input_filename, input, input_filename]"],
            b"1\n",
        ),
        ("filename stdin", &["-c", "input_filename"], b"1\n"),
        ("break line", &["-n", "input, input"], b"1\n"),
        ("break unknown", &["-n", "input, input"], b"1"),
        (
            "break again",
            &["-n", "-c", "try input catch ., input"],
            b"",
        ),
        ("empty lines", &["-n", "-c", "[inputs]"], b"\n\n"),
        ("optional", &["-n", "-c", "input?"], b""),
        ("def input", &["-n", "-c", "def f: input; f, f"], b"1\n2\n"),
        (
            "def inputs limit",
            &["-n", "-c", "def f: inputs; limit(1; f), input"],
            b"1\n2\n3\n",
        ),
        ("shadow input", &["-n", "-c", "def input: 7; input"], b""),
        ("input arity", &["-n", "input(1)"], b""),
        (
            "reduce inputs",
            &["-n", "-c", "reduce inputs as $x (0; . + $x)"],
            b"1\n2\n3\n",
        ),
        ("error line", &["-n", "inputs | .a"], b"1\n2\n"),
        ("raw lines", &["-n", "-R", "-c", "[inputs]"], b"a\nb\nc"),
        ("raw break", &["-n", "-R", "input, input, input"], b"a\nb\n"),
        (
            "raw no newline",
            &["-n", "-R", "input, input, input"],
            b"a\nb",
        ),
        ("slurp inputs", &["-n", "-s", "-c", "[inputs]"], b"1\n2\n"),
        (
            "slurp break",
            &["-s", "-c", "[., try input catch .]"],
            b"1\n2\n",
        ),
        ("slurp name", &["-s", "-c", "input_filename"], b"1\n2\n"),
        ("null slurp name", &["-n", "-s", "input_filename"], b"1\n"),
        ("raw slurp", &["-n", "-Rs", "-c", "input"], b"a\nb\n"),
        ("exit empty", &["-n", "-e", "inputs"], b""),
        ("exit break", &["-n", "-e", "input"], b""),
        ("exit false", &["-n", "-e", "input"], b"false\n"),
        ("loc", &["-n", "-c", "$__loc__"], b""),
        ("loc line", &["-n", "-c", "1 |\n$__loc__"], b""),
        ("loc def", &["-n", "-c", "def f:\n$__loc__;\nf"], b""),
        ("loc string", &["-n", "-c", r#""\($__loc__)""#], b""),
        ("loc field", &["-n", "-c", "{x: $__loc__}"], b""),
        ("loc bind", &["-n", "-c", "$__loc__ as $l | $l.line"], b""),
        (
            "loc arg",
            &["-n", "-c", "--arg", "__loc__", "hi", "$__loc__"],
            b"",
        ),
        ("env same", &["-n", "-c", "$ENV == env"], b""),
        ("env type", &["-n", "-c", "[($ENV|type), (env|type)]"], b""),
        (
            "env shadow",
            &["-n", "-c", "1 as $ENV | [$ENV, (env|type)]"],
            b"",
        ),
        (
            "env arg",
            &[
                "-n",
                "-c",
                "--arg",
                "ENV",
                "hi",
                "[($ENV|type), $ARGS.named.ENV]",
            ],
            b"",
        ),
        (
            "env path",
            &["-n", "-c", "--arg", "PATH", "nope", "$ENV.PATH == env.PATH"],
            b"",
        ),
        ("env arity", &["-n", "env(1)"], b""),
        (
            "env def",
            &["-n", "-c", "def env: 3; [env, ($ENV|type)]"],
            b"",
        ),
        (
            "args none",
            &["-n", "-c", "--args", "$ARGS.positional"],
            b"",
        ),
        (
            "args two",
            &["-n", "-c", "--args", "$ARGS.positional", "a", "b"],
            b"",
        ),
        (
            "args after",
            &["-n", "-c", "$ARGS.positional", "--args", "a", "b"],
            b"",
        ),
        (
            "args option",
            &["-n", "--args", "-c", "$ARGS.positional", "a"],
            b"",
        ),
        (
            "args full",
            &["-n", "-c", "--arg", "b", "1", "--args", "$ARGS", "a", "c"],
            b"",
        ),
        (
            "json args",
            &[
                "-n",
                "-c",
                "--jsonargs",
                "$ARGS.positional",
                "1",
                "true",
                "null",
            ],
            b"",
        ),
        (
            "json bad",
            &["-n", "--jsonargs", "$ARGS.positional", "{"],
            b"",
        ),
        (
            "json two",
            &["-n", "--jsonargs", "$ARGS.positional", "1 2"],
            b"",
        ),
        (
            "json blank",
            &["-n", "--jsonargs", "$ARGS.positional", ""],
            b"",
        ),
        (
            "json space",
            &["-n", "-c", "--jsonargs", "$ARGS.positional", "1 "],
            b"",
        ),
        (
            "json odd",
            &[
                "-n",
                "-c",
                "--jsonargs",
                "$ARGS.positional",
                "01",
                ".5",
                "nan",
            ],
            b"",
        ),
        (
            "json last",
            &["-n", "-c", "--args", "--jsonargs", "$ARGS.positional", "1"],
            b"",
        ),
        (
            "text last",
            &["-n", "-c", "--jsonargs", "--args", "$ARGS.positional", "1"],
            b"",
        ),
        (
            "args empty text",
            &["-n", "-c", "--args", "$ARGS.positional", ""],
            b"",
        ),
        (
            "pos and named",
            &["-n", "-c", "--arg", "a", "z", "--jsonargs", "$ARGS", "true"],
            b"",
        ),
    ];
    for (name, args, input) in cases {
        compare_exact(name, args, input);
    }

    compare_exact("files", &["-n", "-c", "[inputs]"], &[]);
    compare_exact(
        "file values",
        &["-n", "-c", "[inputs | [., input_filename]]", &one, &two],
        b"",
    );
    compare_exact("file dot", &["-c", "[., input_filename]", &one, &two], b"");
    compare_exact("span", &["-n", "-c", "[inputs]", &spanned, &span_b], b"");
    compare_exact(
        "span name",
        &["-n", "-c", "[inputs | input_filename]", &spanned, &span_b],
        b"",
    );
    compare_exact(
        "later file",
        &[
            "-n",
            "-c",
            "(try inputs catch .), inputs",
            &early,
            &later,
            &after,
        ],
        b"",
    );
    compare_exact("bad file", &["-n", "input", &bad], b"");
    compare_exact("partial file", &["-n", "input", &partial], b"");
    compare_exact("missing", &["-n", "inputs", &missing], b"");
    compare_exact("missing then", &["-n", "-c", "inputs", &missing, &one], b"");
    compare_exact("missing input", &["-n", "input", &missing], b"");
    compare_exact(
        "empty then",
        &["-n", "-c", "input, input", &empty, &one],
        b"",
    );
    compare_exact("directory", &["-n", "input", &folder], b"");
    compare_exact(
        "directory then",
        &["-n", "-c", "inputs", &folder, &one],
        b"",
    );
    compare_exact(
        "directory last",
        &["-n", "-c", "inputs", &one, &folder],
        b"",
    );
    compare_exact("directory slurp", &["-n", "-s", "inputs", &folder], b"");
    compare_exact("raw files", &["-n", "-R", "-c", "[inputs]", &lines], b"");
    compare_exact(
        "raw missing",
        &["-n", "-R", "-c", "[inputs]", &missing, &lines],
        b"",
    );
    compare_exact("raw one", &["-R", "-c", ".", &missing, &lines], b"");
    compare_exact(
        "raw inside",
        &["-R", "-c", "., input", &missing, &lines],
        b"",
    );
    compare_exact("loc file", &["-n", "-c", "-f", &prog], b"");
    compare_exact(
        "slurpfile",
        &["-n", "-c", "--slurpfile", "a", &slurp, "$a"],
        b"",
    );
    compare_exact(
        "slurp empty",
        &["-n", "-c", "--slurpfile", "a", &empty, "$a"],
        b"",
    );
    compare_exact("slurp bad", &["-n", "--slurpfile", "a", &bad, "$a"], b"");
    compare_exact(
        "slurp missing",
        &["-n", "--slurpfile", "a", &missing, "$a"],
        b"",
    );
    compare_exact("slurp dir", &["-n", "--slurpfile", "a", &folder, "$a"], b"");
    compare_exact(
        "slurp first",
        &[
            "-n",
            "-c",
            "--slurpfile",
            "a",
            &one,
            "--slurpfile",
            "a",
            &bad,
            "$a",
        ],
        b"",
    );
    compare_exact(
        "slurp named",
        &["-n", "-c", "--slurpfile", "a", &one, "$ARGS.named.a"],
        b"",
    );
    compare_exact("rawfile", &["-n", "-c", "--rawfile", "t", &raw, "$t"], b"");
    compare_exact(
        "rawfile empty",
        &["-n", "-c", "--rawfile", "t", &empty, "$t"],
        b"",
    );
    compare_exact(
        "rawfile missing",
        &["-n", "--rawfile", "t", &missing, "$t"],
        b"",
    );
    compare_exact("rawfile dir", &["-n", "--rawfile", "t", &folder, "$t"], b"");
    compare_exact("slurp usage", &["--slurpfile"], b"");
    compare_exact("raw usage", &["--rawfile", "t"], b"");
    compare_exact(
        "slurp keeps input",
        &["-c", "--slurpfile", "t", &one, ".", &two],
        b"",
    );
    compare_exact(
        "args not files",
        &["-n", "-c", "--args", "$ARGS.positional", &one],
        b"",
    );
    compare_exact("file then args", &["-c", ".", &one, "--args", "z"], b"");
}

#[test]
fn rejected_options_stay_ours() {
    require_jq();
    let unknown = [
        "--arg=a",
        "--argjson=a",
        "--from-file=prog.jq",
        "--indent=4",
        "-fprog.jq",
        "--slurpfile=a",
        "--args=foo",
        "-eQ",
    ];
    for option in unknown {
        let stderr = format!("tumulus: unknown option: {option}\n");
        assert_rejects(option, &[option], b"", stderr.as_bytes(), 1);
    }

    let (dir, _guard) = scratch("reject");
    let empty = write_bytes(&dir, "empty.jq", b"");
    let bad = write_bytes(&dir, "bad.jq", b"{\n");
    let badb = write_bytes(&dir, "badb.jq", b"\xff");
    let syntax = b"tumulus: syntax error\n";
    assert_rejects("empty filter", &["-n", "-e", "-f", &empty], b"", syntax, 3);
    assert_rejects("bad filter", &["-n", "-f", &bad], b"", syntax, 3);
    assert_rejects("bad byte", &["-n", "-f", &badb], b"", syntax, 3);
    assert_rejects(
        "unbound",
        &["--arg", "a", "1", "-n", "-e", "$b"],
        b"",
        syntax,
        3,
    );
    compare_exact("dash f", &["-n", "--", "-f"], b"");
    compare_exact("dash arg", &["-n", "--", "--arg"], b"");
}

#[test]
fn math_matches_jq() {
    require_jq();
    let values = r#"[
      (0.5|floor), ((-0.5)|floor), (1.5|floor), ((-1.5)|floor), (1.1|floor), ((-1.1)|floor),
      ((-0)|floor), (0|floor), (nan|floor), (infinite|floor), ((-infinite)|floor),
      (9007199254740993|floor), (1e309|floor),
      (0.5|ceil), ((-0.5)|ceil), (1.1|ceil),
      (0.5|round), ((-0.5)|round), (1.5|round), ((-1.5)|round), (2.5|round), ((-2.5)|round), (1.1|round),
      ((-0.5)|trunc), ((-0)|trunc), (1.1|trunc),
      (0.5|nearbyint), (2.5|nearbyint), ((-2.5)|nearbyint), ((-0.5)|rint),
      (4|fabs), ((-3.2)|fabs), (nan|fabs), ((-nan)|fabs),
      (4|sqrt), (9|sqrt), ((-1)|sqrt), ((-0)|sqrt), (infinite|sqrt), ((-infinite)|sqrt),
      (9|cbrt), ((-8)|cbrt), (4|cbrt),
      (1|exp), (1|exp2), (1|exp10), (2|exp10), ((-1)|exp10), (1|expm1),
      (1|log), (0|log), ((-1)|log), (100|log10), (0|log10), (8|log2), ((-1)|log2),
      (0|log1p), ((-0)|log1p), ((-1)|log1p), ((-2)|log1p),
      (1.5|logb), (1024|logb), (0|logb), ((-0)|logb), ((-8)|logb), (infinite|logb), ((-infinite)|logb),
      (1|sin), (1|cos), (1|tan), (nan|sin), (infinite|cos),
      (0.5|asin), (2|asin), ((-1)|asin), ((-2)|asin), (0.5|acos), (1|atan),
      (1|sinh), (1|cosh), (0.5|tanh), (1|asinh), (2|acosh), (0|acosh), (0.5|atanh), (1|atanh), (2|atanh),
      (5|gamma), (0.5|gamma), ((-0.5)|gamma), (0|gamma), ((-1)|gamma), ((-infinite)|gamma), (1.5|gamma),
      (5|tgamma), ((-2)|tgamma), ((-infinite)|tgamma),
      (5|lgamma), (0|lgamma), ((-1)|lgamma), ((-infinite)|lgamma),
      (0|erf), (1|erf), (infinite|erfc),
      (0|j0), (1|j0), (2|j0), (1|j1), (2|j1), (0|y0), (1|y0), ((-1)|y0), (1|y1),
      (0|frexp), ((-0)|frexp), (1|frexp), ((-1)|frexp), (1.5|frexp), (8|frexp), (0.5|frexp), (0.1|frexp),
      (1024|frexp), (nan|frexp), (infinite|frexp), ((-infinite)|frexp), (5e-324|frexp),
      (1.7976931348623157e+308|frexp), ((-2.5)|frexp),
      (0|modf), ((-0)|modf), (1|modf), ((-1)|modf), (1.5|modf), ((-1.5)|modf), (8|modf), (0.1|modf),
      (nan|modf), (infinite|modf), ((-infinite)|modf), (5e-324|modf), ((-2.5)|modf),
      (0|significand), ((-0)|significand), (1|significand), (1.5|significand), (0.1|significand),
      (8|significand), ((-4)|significand), ((-1.5)|significand), (2.5|significand), (0.5|significand),
      (3|significand), (7|significand), (5e-324|significand), (1e-320|significand),
      (nan|significand), (infinite|significand), ((-infinite)|significand),
      (nan|floor|type), (infinite|floor|type)
    ]"#;
    compare_exact("math values", &["-n", "-c", "--", values], b"");

    let calls = r#"[
      pow(2;10), pow(0;0), pow(-0;0), pow(-0;1), pow(-0;2), pow(-2;3), pow(-2;-1),
      pow(-2;0.5), pow(2;-1), pow(8;1/3), pow(1.5;2.5), pow(10;-2), pow(-1.5;2),
      pow(nan;1), pow(1;nan), pow(infinite;0), pow(0;infinite), pow(-1;infinite),
      pow(2;infinite), pow(0.5;infinite),
      atan2(0;-1), atan2(-0;-1), atan2(1;0), atan2(-1;-1),
      ldexp(1;0), ldexp(1;1), ldexp(1;-1), ldexp(1;1.9), ldexp(1;-1.9), ldexp(1;2.2),
      ldexp(1;10), ldexp(1;-10), ldexp(1;0.5), ldexp(1;-0.5), ldexp(1;1023), ldexp(1;1024),
      ldexp(1;nan), ldexp(1;infinite), ldexp(1;-infinite), ldexp(1;1e20), ldexp(1;-1e20),
      ldexp(1;2147483647), ldexp(1;2147483648), ldexp(1;-2147483648),
      fma(2;3;4), fma(1e-20;1e-20;1), fma(nan;1;1), fma(1;nan;1), fma(1;1;nan), fma(-nan;1;1),
      hypot(3;4), hypot(infinite;1), hypot(nan;infinite),
      fmod(5;2), fmod(-5;2), fmod(5;-2), fmod(-5;-2), fmod(5.5;2), fmod(1;0), fmod(nan;1),
      remainder(5;2), remainder(-5;2), remainder(5.5;2), remainder(1;0),
      drem(5;2), drem(-5;2), drem(3;2), drem(-3;2), drem(1;0), drem(-0;1),
      copysign(1;-2), copysign(-1;0), copysign(1;-0), copysign(-0;1), copysign(nan;-1), copysign(1;nan),
      copysign(1; copysign(nan; -1)),
      fmax(1;2), fmax(nan;1), fmax(1;nan), fmax(nan;nan), fmax(-0;0), fmax(0;-0),
      fmin(nan;1), fmin(1;nan), fmin(-0;0), fmin(0;-0), fmin(-0;-0), fmax(-0;-0),
      fdim(3;1), fdim(1;3), fdim(nan;1), fdim(1;nan),
      nextafter(1;2), nextafter(0;-1), nextafter(1;1), nextafter(nan;1), nextafter(1;nan), nextafter(-0;1),
      scalb(1;10), scalb(1;-1), scalb(1;1.9), scalb(1;nan), scalb(1;infinite), scalb(1;-infinite), scalb(-2;3),
      scalbln(1;10), scalbln(1;1.9), scalbln(1;nan), scalbln(1;infinite), scalbln(1;-infinite),
      scalbln(1;1e20), scalbln(1;-1e20),
      jn(0;1), jn(1;1), jn(-1;1), jn(1.9;1), jn(-1.9;1), jn(2;1), jn(10;1),
      yn(0;1), yn(1;1), yn(2;0.5), yn(1;0.5), yn(-1;1), yn(1.9;1), yn(0;0)
    ]"#;
    compare_exact("math calls", &["-n", "-c", "--", calls], b"");

    let signs = r#"[
      ((-(nan)) | copysign(1; .)),
      ((-(-(nan))) | copysign(1; .)),
      (((-nan)|floor) | copysign(1; .)),
      (((-nan)|fabs) | copysign(1; .)),
      (((-nan)|sin) | copysign(1; .)),
      (((-nan)|gamma) | copysign(1; .)),
      (fma(-nan;1;1) | copysign(1; .)),
      (((-2)|asin) | copysign(1; .))
    ]"#;
    compare_exact("nan signs", &["-n", "-c", "--", signs], b"");

    compare_exact("pow grid", &["-n", "-c", "--", "[pow(10,100; 2,3)]"], b"");
    compare_exact(
        "fma grid",
        &["-n", "-c", "--", "[fma(1,10; 2,20; 3,30)]"],
        b"",
    );
    compare_exact("pow uses the input", &["-c", "--", "pow(.; .+1)"], b"5\n");
    compare_exact("pow ignores the input", &["-c", "--", "pow(2;3)"], b"9\n");
    compare_exact("floor and sqrt", &["-c", "--", "floor, sqrt"], b"4\n");
    compare_exact(
        "nested ldexp",
        &["-n", "-c", "--", "pow(ldexp(1; 3); 2)"],
        b"",
    );
    compare_exact("comment in pow", &["-n", "-c", "--", "pow(2; #c\n3)"], b"");
    compare_exact("spaced pow", &["-n", "-c", "--", "pow ( 2 ; 3 )"], b"");
    compare_exact("spaced fma", &["-n", "-c", "--", "fma( 2 ; 3 ; 4 )"], b"");
    compare_exact(
        "if inside pow",
        &["-n", "-c", "--", "pow(if true then 2 else 9 end; 3)"],
        b"",
    );
    compare_exact("pretty frexp", &["-n", "--", "8 | frexp"], b"");
    compare_exact("pretty modf", &["-n", "--", "-1.5 | modf"], b"");
    compare_exact(
        "divide negated zero",
        &["-n", "-c", "--", "1 / ((-0)|floor)"],
        b"",
    );
    compare_exact(
        "floor is not a path",
        &["-n", "-c", "--", "1 | (floor) = 2"],
        b"",
    );

    compare_exact(
        "pow stops after a bad value",
        &["-n", "-c", "--", r#"pow(1,2; 3,"a",4)"#],
        b"",
    );
    compare_exact(
        "pow stops when an argument fails",
        &["-n", "-c", "--", r#"pow(1, error("x"); 3, 4)"#],
        b"",
    );
    compare_exact(
        "pow bad first",
        &["-n", "-c", "--", r#"pow("a",1; 2,3)"#],
        b"",
    );
    compare_exact(
        "first bad of two",
        &["-n", "-c", "--", r#"pow(1,"a",2; 3,4)"#],
        b"",
    );
    compare_exact("floor stream", &["-n", "-c", "--", r#"1,"a",2|floor"#], b"");
    compare_exact(
        "try floor",
        &["-n", "-c", "--", r#"try (1,"a",2|floor) catch ."#],
        b"",
    );
    compare_exact(
        "caught floor",
        &["-n", "-c", "--", r#"try ("hello"|floor) catch ."#],
        b"",
    );
    compare_exact("optional pow", &["-n", "-c", "--", r#"pow(1;"z")?"#], b"");
    compare_exact("optional success", &["-n", "-c", "--", "pow(2;3)?"], b"");
    compare_exact("optional floor", &["-c", "floor?"], b"\"a\"\n");
    compare_exact(
        "empty arguments",
        &["-n", "-c", "--", "[pow(empty;1), pow(1;empty)]"],
        b"",
    );
    compare_exact("sin keeps going", &["-c", "sin"], b"1\n\"no\"\n2\n");

    compare_exact("string floor", &["-c", "floor"], b"\"hello\"\n");
    compare_exact("string at eof", &["-c", "floor"], b"\"hello\"");
    compare_exact(
        "later value clears the failure",
        &["-c", "floor"],
        b"1.5\n\"a\"\n2.5\n",
    );
    compare_exact(
        "long string",
        &["-c", "floor"],
        b"\"abcdefghijklmnopqrstuvwxyz\"\n",
    );
    compare_exact("object floor", &["-c", "floor"], b"{\"abcdefghij\":1}\n");
    compare_exact(
        "long array",
        &["-c", "floor"],
        b"[1,2,3,4,5,6,7,8,9,10,11,12]\n",
    );
    compare_exact("bool and null", &["-c", "sqrt"], b"true\nfalse\nnull\n");
    compare_exact("pow string", &["-n", "-c", "--", r#"pow("a"; 1)"#], b"");
    compare_exact("pow second", &["-n", "-c", "--", r#"pow(1; "b")"#], b"");
    compare_exact("pow both bad", &["-n", "-c", "--", r#"pow("a"; "b")"#], b"");
    compare_exact("pow null", &["-n", "-c", "--", "pow(null; 1)"], b"");
    compare_exact("pow null second", &["-n", "-c", "--", "pow(1; null)"], b"");
    compare_exact("pow false", &["-n", "-c", "--", "pow(1; false)"], b"");
    compare_exact("fma false", &["-n", "-c", "--", "fma(1; 2; false)"], b"");
    compare_exact("fma first", &["-n", "-c", "--", "fma(false; 2; 3)"], b"");
    compare_exact(
        "ldexp string",
        &["-n", "-c", "--", r#"ldexp(1; "no")"#],
        b"",
    );
    compare_exact(
        "nan floor status",
        &["-n", "-e", "-c", "--", "nan|floor"],
        b"",
    );
    compare_exact(
        "bad floor status",
        &["-n", "-e", "-c", "--", r#""a"|floor"#],
        b"",
    );

    compare_exact("floor arity", &["-n", "--", "floor(1)"], b"");
    compare_exact("indented floor", &["-n", "--", "  floor(1)"], b"");
    compare_exact("floor on the next line", &["-n", "--", "\nfloor(1)"], b"");
    compare_exact("pow missing", &["-n", "--", "pow"], b"");
    compare_exact("pow one", &["-n", "--", "pow(1)"], b"");
    compare_exact("pow three", &["-n", "--", "pow(1;2;3)"], b"");
    compare_exact("pow five", &["-n", "--", "pow(1;2;3;4;5)"], b"");
    compare_exact("fma two", &["-n", "--", "fma(1;2)"], b"");
    compare_exact("fma four", &["-n", "--", "fma(1;2;3;4)"], b"");
    compare_exact("sqrt call", &["-n", "--", "sqrt(-1)"], b"");
    compare_exact("jn three", &["-n", "--", "jn(1;2;3)"], b"");

    let abs_values = r#"[0,-0,1,-1,1.5,-1.5,0.5,-0.5,nan,-nan,infinite,-infinite,"a","",[],{},[1],{"a":1}] | .[] | abs"#;
    compare_exact("abs values", &["-n", "-c", "--", abs_values], b"");
    compare_exact(
        "abs signs",
        &[
            "-n",
            "-c",
            "--",
            "[nan,-nan,-1.5] | .[] | abs | copysign(1; .)",
        ],
        b"",
    );
    compare_exact("abs null", &["-n", "-c", "--", "null | abs"], b"");
    compare_exact("abs true", &["-n", "-c", "--", "true | abs"], b"");
    compare_exact("abs false", &["-n", "-c", "--", "false | abs"], b"");
    compare_exact(
        "abs stream",
        &["-n", "-c", "--", r#"1, "a", -2, null, 3 | abs"#],
        b"",
    );
    compare_exact(
        "abs caught",
        &["-n", "-c", "--", "try (-1, null, 2 | abs) catch ."],
        b"",
    );
    compare_exact("abs arity", &["-n", "--", "abs(1)"], b"");
    compare_exact("abs json", &["-c", "map(abs)"], b"[-0, 0, -10, -1.1]\n");

    let lgamma = "[0,-0,1,-1,1.5,-1.5,0.5,-0.5,2,3,-2,4.5,nan,-nan,infinite,-infinite,5e-324] | .[] | lgamma_r";
    compare_exact("lgamma_r", &["-n", "-c", "--", lgamma], b"");
    compare_exact("lgamma_r string", &["-c", "lgamma_r"], b"\"a\"\n");
    compare_exact("lgamma_r bool", &["-n", "-c", "--", "true | lgamma_r"], b"");
    compare_exact("lgamma_r null", &["-n", "-c", "--", "null | lgamma_r"], b"");
    compare_exact("lgamma_r array", &["-n", "-c", "--", "[1] | lgamma_r"], b"");
    compare_exact(
        "lgamma_r object",
        &["-n", "-c", "--", r#"{"a":1} | lgamma_r"#],
        b"",
    );
    compare_exact("lgamma_r arity", &["-n", "--", "lgamma_r(1)"], b"");
    compare_exact(
        "lgamma_r caught",
        &["-n", "-c", "--", r#"try ("z" | lgamma_r) catch ."#],
        b"",
    );

    compare_exact(
        "nexttoward",
        &[
            "-n",
            "-c",
            "--",
            "[nexttoward(0;-1), nexttoward(-0;1), nexttoward(1;2), nexttoward(1;1), nexttoward(-1;2), nexttoward(0.1;1), nexttoward(nan;1), nexttoward(1;nan), nexttoward(infinite;-1), nexttoward(-infinite;1), nexttoward(5e-324;-1)]",
        ],
        b"",
    );
    compare_exact(
        "nexttoward string",
        &["-n", "-c", "--", r#"nexttoward("a"; 1)"#],
        b"",
    );
    compare_exact(
        "nexttoward second",
        &["-n", "-c", "--", r#"nexttoward(1; "b")"#],
        b"",
    );
    compare_exact(
        "nexttoward null",
        &["-n", "-c", "--", "nexttoward(null; 1), nexttoward(1; null)"],
        b"",
    );
    compare_exact(
        "nexttoward partial",
        &["-n", "-c", "--", r#"nexttoward(1,2; 3,"a",4)"#],
        b"",
    );
    compare_exact("nexttoward arity", &["-n", "--", "nexttoward(1;2;3)"], b"");

    compare_exact("pow10 number", &["-n", "-c", "--", "1 | pow10"], b"");
    compare_exact("pow10 string", &["-n", "-c", "--", r#""a" | pow10"#], b"");
    compare_exact("pow10 null", &["-n", "-c", "--", "null | pow10"], b"");
    compare_exact("pow10 stream", &["-n", "-c", "--", "1, 2 | pow10"], b"");
    compare_exact("pow10 empty", &["-n", "-c", "--", "empty | pow10"], b"");
    compare_exact(
        "pow10 caught",
        &["-n", "-c", "--", "try pow10 catch ."],
        b"",
    );
    compare_exact("pow10 arity", &["-n", "--", "pow10(1)"], b"");
}

#[test]
fn paths_match_jq() {
    require_jq();

    let programs = [
        ("path dot", "path(.)", "{\"a\":1}"),
        ("path field", "path(.a)", "{\"a\":1}"),
        ("path chain", "path(.a[0].b)", "{\"a\":[{\"b\":7}]}"),
        ("path comma", "path(.foo[0,1])", "{\"foo\":[10,20]}"),
        ("path select", "path(.[] | select(.>3))", "[1,5,3]"),
        ("path empty", "path(empty)", "{\"a\":1}"),
        ("path recurse", "path(..)", "{\"a\":[1]}"),
        ("path missing optional", "path(.missing?)", "{\"a\":1}"),
        ("path optional type", "path(.a?)", "1"),
        ("path null match", "path(null)", "null"),
        ("path true match", "path(true)", "true"),
        ("path false match", "path(false)", "false"),
        ("path null mismatch", "path(null)", "1"),
        ("path literal", "try path(1) catch .", "{\"a\":1}"),
        ("path string", "try path(\"a\") catch .", "1"),
        ("path add", "try path(.+1) catch .", "1"),
        ("path array", "try path([.]) catch .", "[1]"),
        ("path object", "try path({a:.}) catch .", "1"),
        ("path partial broken", "try path(.a, 1, .b) catch .", "{\"a\":1,\"b\":2}"),
        ("path broken first", "try path(1, .a) catch .", "{\"a\":2}"),
        ("path index error", "try path(.a, .[0]) catch .", "{\"a\":1}"),
        (
            "path map",
            "try path(.a | map(select(.b == 0))) catch .",
            "{\"a\":[{\"b\":0}]}",
        ),
        (
            "path map index",
            "try path(.a | map(select(.b == 0)) | .[0]) catch .",
            "{\"a\":[{\"b\":0}]}",
        ),
        (
            "path map field",
            "try path(.a | map(select(.b == 0)) | .c) catch .",
            "{\"a\":[{\"b\":0}]}",
        ),
        (
            "path map iterate",
            "try path(.a | map(select(.b == 0)) | .[]) catch .",
            "{\"a\":[{\"b\":0}]}",
        ),
        ("path nested", "path(.a[path(.b)[0]])", "{\"a\":{\"b\":0}}"),
        ("path negative", "path(.[-1])", "[10,20,30]"),
        ("path negative far", "path(.[-4])", "[10,20]"),
        ("path fraction", "path(.[1.5])", "[10,20,30]"),
        ("path one decimal", "path(.[1.0])", "[10,20,30]"),
        ("path neg zero", "path(.[-0])", "[10,20,30]"),
        ("path spellings", "path(.[1.5, 2.0])", "[10,20,30]"),
        ("path slice", "path(.[1:3])", "[10,20,30,40]"),
        ("path slice open", "path(.[:2])", "[10,20,30,40]"),
        ("path slice tail", "path(.[1:])", "[10,20,30,40]"),
        ("path slice decimal", "path(.[1.0:2.0])", "[10,20,30,40]"),
        ("path slice zero", "path(.[01:2])", "[10,20,30,40]"),
        ("path slice then index", "path(.[0:2][0])", "[10,20,30]"),
        ("path huge", "path(.[1e20])", "[1,2,3]"),
        ("path string index", "try path(.[0]) catch .", "\"ab\""),
        ("path number field", "try path(.a) catch .", "1"),
        ("path null field", "path(.a.b)", "null"),
        ("path getpath", "path(getpath([\"a\",0,\"b\"]))", "{\"a\":[{\"b\":0}]}"),
        ("path getpath missing", "path(getpath([\"missing\"]))", "{\"a\":1}"),
        ("path getpath far", "path(getpath([\"a\",5]))", "{\"a\":[1]}"),
        ("path getpath negative", "path(getpath([\"a\",-1]))", "{\"a\":[9,8]}"),
        ("path getpath fraction", "path(getpath([1.50]))", "[10,20,30]"),
        ("path getpath decimal", "path(getpath([1.0]))", "[10,20,30]"),
        ("path getpath zero", "path(getpath([-0]))", "[10,20,30]"),
        ("path getpath huge", "path(getpath([1e20]))", "[1,2,3]"),
        (
            "path getpath slice",
            "path(getpath([{\"start\":1.0,\"end\":2}]))",
            "[10,20,30,40]",
        ),
        (
            "path getpath open slice",
            "path(getpath([{\"start\":1,\"end\":null}]))",
            "[10,20,30]",
        ),
        ("path getpath empty", "path(getpath([]))", "{\"a\":1}"),
        ("path getpath array", "path(getpath([[]]))", "[1,2,3]"),
        ("path getpath nested array", "path(getpath([[[0]]]))", "[1,2,3]"),
        ("path getpath empty object", "path(getpath([{}]))", "null"),
        (
            "path getpath key order",
            "path(getpath([{\"end\":2,\"start\":1}]))",
            "[1,2,3,4]",
        ),
        (
            "path getpath extra",
            "path(getpath([{\"start\":1,\"end\":3,\"x\":9}]))",
            "[1,2,3,4]",
        ),
        ("path field huge", "path(.a[1e20])", "{\"a\":[1,2,3]}"),
        ("getpath slice index", "getpath([{\"start\":0,\"end\":2}, 0])", "[1,2,3,4]"),
        (
            "setpath slice index",
            "setpath([{\"start\":0,\"end\":2}, 0]; 9)",
            "[1,2,3,4]",
        ),
        (
            "delpaths slice index",
            "delpaths([[{\"start\":0,\"end\":2}, 0]])",
            "[1,2,3,4]",
        ),
        (
            "paths condition error",
            "try paths(if . == 2 then error(\"z\") else true end) catch .",
            "[1,2]",
        ),
        ("path getpath pipe", "path(getpath([\"a\"]) | getpath([\"b\"]))", "{\"a\":{\"b\":1}}"),
        ("path getpath from null", "null | path(null | getpath([\"a\"]))", "null"),
        ("path built array", "try path([1,2] | getpath([0])) catch .", "[1,2]"),
        ("path bad getpath", "try path(getpath(1)) catch .", "{\"a\":1}"),
        ("path null key", "try path(getpath([null])) catch .", "{\"a\":1}"),
        ("path setpath", "try path(setpath([\"a\"]; 1)) catch .", "{\"b\":2}"),
        ("path delpaths", "try path(delpaths([[\"a\"]])) catch .", "{\"a\":1,\"b\":2}"),
        ("path paths", "try path(paths) catch .", "{\"a\":1}"),
        ("path error stops", "try path(.a, error(\"z\"), .b) catch .", "{\"a\":1,\"b\":2}"),
        ("paths scalar", "[paths]", "null"),
        ("paths number", "[paths]", "1"),
        ("paths string", "[paths]", "\"ab\""),
        ("paths empty object", "[paths]", "{}"),
        ("paths empty array", "[paths]", "[]"),
        ("paths nested", "[paths]", "[1,{\"a\":2,\"b\":[3]}]"),
        ("paths keys", "[paths]", "{\"b\":1,\"a\":{\"c\":2}}"),
        ("paths truth", "[paths(.)]", "{\"a\":1,\"b\":null,\"c\":false,\"d\":0}"),
        ("paths type", "[paths(type==\"number\")]", "[1,{\"a\":2,\"b\":[3]}]"),
        ("paths compare", "paths(. > 1)", "[1,2,3]"),
        ("paths select", "[paths(select(.>0))]", "[0,1,2]"),
        ("paths many", "[paths(1, false, 2, null, empty, true)]", "[1]"),
        ("paths variable", ". as $x | [paths(. == $x[0])]", "[1,2]"),
        ("paths false", "[paths(false)]", "[0,1]"),
        ("paths empty filter", "[paths(empty)]", "[1]"),
        ("paths root error", "try paths(tonumber) catch .", "[1,\"a\",3]"),
        ("paths later error", "try paths(has(\"x\")) catch .", "{\"a\":1}"),
        (
            "paths element error",
            "try paths(if . == 2 then error(\"z\") else true end) catch .",
            "[1,2]",
        ),
        ("getpath field", "getpath([\"a\"])", "{\"a\":1,\"b\":2}"),
        ("getpath missing", "getpath([\"missing\"])", "{\"a\":1}"),
        ("getpath index", "getpath([1])", "[10,20,30]"),
        ("getpath negative", "getpath([-1])", "[10,20,30]"),
        ("getpath negative far", "getpath([-4])", "[10,20,30]"),
        ("getpath beyond", "getpath([5])", "[10,20,30]"),
        ("getpath fraction", "getpath([1.5])", "[10,20,30]"),
        ("getpath decimal", "getpath([1.0])", "[10,20,30]"),
        ("getpath neg zero", "getpath([-0])", "[10,20,30]"),
        ("getpath huge", "getpath([1e20])", "[1,2,3]"),
        ("getpath infinite", "getpath([1e309])", "[1,2,3]"),
        ("getpath nan", "getpath([nan])", "[1,2,3]"),
        ("getpath empty", "getpath([])", "{\"a\":1}"),
        ("getpath nested", "getpath([\"b\",\"c\"])", "{\"a\":1,\"b\":{\"c\":2}}"),
        ("getpath missing nested", "getpath([\"a\",\"b\",\"c\"])", "{}"),
        ("getpath null chain", "getpath([\"a\",0])", "null"),
        ("getpath slice", "getpath([{\"start\":1,\"end\":3}])", "[1,2,3,4]"),
        ("getpath slice string", "getpath([{\"start\":1,\"end\":3}])", "\"abcd\""),
        ("getpath slice nulls", "getpath([{\"start\":null,\"end\":null}])", "\"ab\""),
        ("getpath slice fraction", "getpath([{\"start\":1.5,\"end\":3}])", "[1,2,3,4]"),
        ("getpath slice order", "getpath([{\"end\":3,\"start\":1,\"end\":2}])", "[1,2,3,4,5]"),
        ("getpath bad slice", "try getpath([{}]) catch .", "[1,2,3]"),
        ("getpath bad slice null", "getpath([{}])", "null"),
        ("getpath array key", "getpath([[]])", "[1,2,3]"),
        ("getpath bool", "try getpath([true]) catch .", "[1]"),
        ("getpath null key", "try getpath([null]) catch .", "{\"a\":1}"),
        ("getpath string", "try getpath(\"a\") catch .", "{\"a\":1}"),
        ("getpath number", "try getpath(0) catch .", "[1]"),
        ("getpath on string", "try getpath([0]) catch .", "\"ab\""),
        ("getpath on number", "try getpath([\"a\"]) catch .", "1"),
        ("getpath stream", "try getpath(([\"a\"], 0, [\"b\"])) catch .", "{\"a\":1,\"b\":2}"),
        ("getpath empty arg", "getpath(empty)", "{\"a\":1}"),
        ("setpath replace", "setpath([]; 5)", "{\"a\":1}"),
        ("setpath field", "setpath([\"a\"]; 9)", "{\"a\":1,\"b\":2}"),
        ("setpath missing", "setpath([\"missing\"]; 9)", "{\"a\":1}"),
        ("setpath create", "setpath([\"a\",0,\"b\"]; 5)", "null"),
        ("setpath index create", "setpath([0,\"a\"]; 5)", "null"),
        ("setpath extend", "setpath([2]; 1)", "[]"),
        ("setpath gap", "setpath([5]; 9)", "[1,2,3]"),
        ("setpath negative", "setpath([-1]; 9)", "[0,1,2]"),
        ("setpath negative empty", "try setpath([-1]; 9) catch .", "[]"),
        ("setpath negative far", "try setpath([-5]; 9) catch .", "[1,2,3]"),
        ("setpath fraction", "setpath([1.5]; 7)", "[10,20,30]"),
        ("setpath neg zero", "setpath([-0]; 7)", "[10,20,30]"),
        ("setpath neg zero null", "setpath([-0]; 9)", "null"),
        ("setpath slice", "setpath([{\"start\":1,\"end\":3}]; [8,9])", "[1,2,3,4]"),
        ("setpath slice value", "try setpath([{\"start\":1,\"end\":3}]; 9) catch .", "[1,2,3,4]"),
        ("setpath slice string", "try setpath([{\"start\":1,\"end\":3}]; \"XY\") catch .", "\"abcd\""),
        ("setpath insert", "setpath([{\"start\":1,\"end\":1}]; [9,8])", "[1,2,3]"),
        ("setpath slice tail", "setpath([{\"start\":5,\"end\":6}]; [9])", "[1,2,3]"),
        ("setpath null slice", "setpath([{\"start\":0,\"end\":1}]; [9])", "null"),
        ("setpath bad slice", "try setpath([{}]; 9) catch .", "null"),
        ("setpath array key", "try setpath([[]]; 9) catch .", "[1,2]"),
        ("setpath pairs", "setpath(([\"a\"],[\"b\"]); 1)", "{\"a\":0}"),
        ("setpath values", "setpath([\"a\"]; 1,2)", "{\"a\":0}"),
        ("setpath product", "setpath(([\"a\"],[\"b\"]); (1,2))", "{\"a\":0}"),
        ("setpath error after", "try setpath(([\"a\"], error(\"z\")); 1) catch .", "{\"a\":0}"),
        ("setpath error first", "try setpath((error(\"z\"), [\"a\"]); 1) catch .", "{\"a\":0}"),
        ("setpath value error", "try setpath([\"a\"]; (1, error(\"z\"))) catch .", "{\"a\":0}"),
        ("setpath outer error", "try setpath(error(\"z\"); 1) catch .", "{\"a\":0}"),
        ("setpath value first", "try setpath(0; error(\"z\")) catch .", "{\"a\":0}"),
        ("setpath empty path", "setpath(empty; 1)", "{\"a\":1}"),
        ("setpath empty value", "setpath([\"a\"]; empty)", "{\"a\":1}"),
        ("setpath type", "try setpath(false; 1) catch .", "{}"),
        ("setpath number on object", "try setpath([0]; 1) catch .", "{}"),
        ("setpath string on array", "try setpath([\"a\"]; 1) catch .", "[]"),
        ("setpath nested number", "try setpath([0,\"a\"]; 1) catch .", "[1]"),
        ("delpaths field", "delpaths([[\"a\"]])", "{\"a\":1,\"b\":2}"),
        ("delpaths missing", "delpaths([[\"missing\"]])", "{\"a\":1}"),
        ("delpaths index", "delpaths([[1]])", "[1,2,3,4]"),
        ("delpaths two", "delpaths([[1],[3]])", "[1,2,3,4]"),
        ("delpaths order", "delpaths([[3],[1]])", "[1,2,3,4]"),
        ("delpaths same", "delpaths([[1],[1]])", "[1,2,3,4]"),
        ("delpaths nested", "delpaths([[0],[1,0]])", "[[1,2],[3,4]]"),
        ("delpaths nested order", "delpaths([[1,0],[0]])", "[[1,2],[3,4]]"),
        ("delpaths parent", "delpaths([[\"a\"],[\"a\",\"b\"]])", "{\"a\":{\"b\":1},\"c\":2}"),
        ("delpaths child first", "delpaths([[\"a\",\"b\"],[\"a\"]])", "{\"a\":{\"b\":1},\"c\":2}"),
        ("delpaths negative", "delpaths([[-1],[0]])", "[1,2,3,4]"),
        ("delpaths negative far", "delpaths([[-200]])", "[1,2,3]"),
        ("delpaths beyond", "delpaths([[4]])", "[1,2,3,4]"),
        ("delpaths empty list", "delpaths([])", "[1,2,3]"),
        ("delpaths root", "delpaths([[]])", "{\"a\":1}"),
        ("delpaths root and index", "delpaths([[0],[]])", "[1,2,3]"),
        ("delpaths slice", "delpaths([[{\"start\":1,\"end\":3}]])", "[1,2,3,4]"),
        ("delpaths slice fraction", "delpaths([[{\"start\":1.5,\"end\":3}]])", "[1,2,3,4]"),
        ("delpaths slice string", "try delpaths([[{\"start\":1,\"end\":3}]]) catch .", "\"abcd\""),
        ("delpaths none", "delpaths([[{\"start\":1,\"end\":1}]])", "[1,2,3]"),
        ("delpaths number field", "try delpaths([[0]]) catch .", "{\"a\":1}"),
        ("delpaths string element", "try delpaths([[\"a\"]]) catch .", "[1,2]"),
        ("delpaths bool element", "try delpaths([[true]]) catch .", "[1]"),
        ("delpaths null element", "try delpaths([[null]]) catch .", "[]"),
        ("delpaths from number", "try delpaths([[0]]) catch .", "1"),
        ("delpaths from string", "try delpaths([[\"a\"]]) catch .", "\"ab\""),
        ("delpaths from bool", "try delpaths([[0]]) catch .", "true"),
        ("delpaths from null", "delpaths([[true]])", "null"),
        ("delpaths nested delete", "try delpaths([[\"a\",0]]) catch .", "{\"a\":1}"),
        ("delpaths nested missing", "delpaths([[\"missing\",0]])", "{\"a\":1}"),
        ("delpaths through null", "delpaths([[0,0]])", "[null]"),
        ("delpaths bad element", "try delpaths([[1], 2]) catch .", "[1,2,3,4]"),
        ("delpaths not array", "try delpaths(0) catch .", "{}"),
        ("delpaths string", "try delpaths(\"ab\") catch .", "{}"),
        ("delpaths null", "try delpaths(null) catch .", "{}"),
        ("delpaths object", "try delpaths({}) catch .", "{}"),
        ("delpaths bool", "try delpaths(false) catch .", "{}"),
        ("delpaths inner string", "try delpaths([\"a\"]) catch .", "[1]"),
        ("delpaths inner null", "try delpaths([null]) catch .", "{}"),
        ("delpaths stream", "delpaths(([[0]], [[1]]))", "[1,2,3]"),
        ("delpaths stream error", "try delpaths(([[0]], 0)) catch .", "[1,2,3]"),
        ("delpaths error first", "try delpaths((0, [[0]])) catch .", "[1,2,3]"),
        ("delpaths mid error", "try delpaths([[1], [true]]) catch .", "[1,2,3,4]"),
        ("delpaths index first", "try delpaths([[true, 0]]) catch .", "[1,2,3]"),
        ("delpaths array key", "delpaths([[[0], 0]])", "[1,2,3]"),
        ("update getpath", "getpath([1]) |= .+10", "[1,2,3]"),
        ("update getpath add", "getpath([\"a\"]) += 2", "{\"a\":1}"),
        ("update getpath create", "getpath([\"a\",0,\"b\"]) = 5", "null"),
        ("update getpath bounds", "try (getpath([-5]) = 9) catch .", "[1,2,3]"),
        ("update getpath extend", "getpath([5]) = 9", "[1,2,3]"),
        (
            "update getpath slice",
            "getpath([{\"start\":1,\"end\":3}]) = [8,9]",
            "[1,2,3,4]",
        ),
        (
            "update getpath cases",
            ".[] | try (getpath([\"a\",0,\"b\"]) |= 5) catch .",
            "[null,{\"b\":0},{\"a\":0},{\"a\":null},{\"a\":[0,1]},{\"a\":{\"b\":1}},{\"a\":[{}]},{\"a\":[{\"c\":3}]}]",
        ),
        (
            "setpath no leak",
            "try [\"ok\", setpath([1]; 1)] catch [\"ko\", .]",
            "{\"hi\":\"hello\"}",
        ),
        ("suite get set del", "[\"foo\",1] as $p | getpath($p), setpath($p; 20), delpaths([$p])", "{\"bar\":42,\"foo\":[\"a\",\"b\",\"c\",\"d\"]}"),
        ("suite missing", "[\"foo\",1] as $p | getpath($p), setpath($p; 20), delpaths([$p])", "{\"bar\":false}"),
        ("empty input path", "empty | path(.)", "null"),
        ("empty input paths", "empty | paths", "null"),
        ("empty input get", "empty | getpath([\"a\"])", "null"),
        ("empty input set", "empty | setpath([\"a\"]; 1)", "null"),
        ("empty input del", "empty | delpaths([[\"a\"]])", "null"),
    ];
    for (name, program, input) in programs {
        compare_exact(name, &["-c", "--", program], input.as_bytes());
    }

    compare_exact("path arity", &["-n", "--", "path"], b"");
    compare_exact("path arity two", &["-n", "--", "path(1;2)"], b"");
    compare_exact("paths arity", &["-n", "--", "paths(1;2)"], b"");
    compare_exact("getpath arity", &["-n", "--", "getpath"], b"");
    compare_exact("getpath arity two", &["-n", "--", "getpath(1;2)"], b"");
    compare_exact("setpath arity", &["-n", "--", "setpath"], b"");
    compare_exact("setpath arity one", &["-n", "--", "setpath([0])"], b"");
    compare_exact(
        "setpath arity three",
        &["-n", "--", "setpath([0];1;2)"],
        b"",
    );
    compare_exact("delpaths arity", &["-n", "--", "delpaths"], b"");
    compare_exact("delpaths arity two", &["-n", "--", "delpaths([0];1)"], b"");

    compare_exact(
        "paths stream",
        &["-c", "--", "paths"],
        b"1\n[1,{\"a\":2}]\n",
    );
    compare_exact(
        "path stream",
        &["-c", "--", "path(.a)"],
        b"[1]\n{\"a\":2}\n",
    );
    compare_exact(
        "getpath stream",
        &["-c", "--", "getpath([0])"],
        b"{\"a\":1}\n[9,8]\n",
    );
}

#[test]
fn generators_match_jq() {
    require_jq();

    let programs = [
        "limit",
        "limit(1)",
        "limit(1;2;3)",
        "first(1;2)",
        "last(1;2)",
        "nth",
        "nth(1;2;3)",
        "range",
        "range(1;2;3;4)",
        "until",
        "until(1)",
        "until(1;2;3)",
        "while",
        "while(1)",
        "while(1;2;3)",
        "repeat",
        "repeat(1;2)",
        "first",
        "last",
        "nth(0)",
        "[10,20,30] | first",
        "[10,20,30] | last",
        "[10,20,30] | nth(1)",
        "[10,20,30] | nth(1.5)",
        "[10,20,30] | nth(0.9)",
        "[10,20,30] | nth(-1.2)",
        "[10,20,30] | nth(-0)",
        "[10,20,30] | first(.[])",
        "[10,20,30] | last(.[])",
        "[10,20,30] | nth(1; .[])",
        "[10,20,30] | nth(0; .[])",
        "[10,20,30] | nth(3; .[])",
        "try ([10,20,30] | nth(-1; .[])) catch .",
        "[] | first",
        "[] | last",
        "null | first",
        "null | last",
        "try (\"abc\" | first) catch .",
        "try (1 | first) catch .",
        "try ({} | first) catch .",
        "try (true | last) catch .",
        "[10,20,30] | first, last",
        "[10,20,30,40] | limit(2; .[])",
        "[10,20,30] | limit(0; .[])",
        "[10,20,30] | limit(-1; .[])",
        "[10,20,30] | limit(10; .[])",
        "[10,20,30] | limit(0.1; .[])",
        "[10,20,30] | limit(0.9; .[])",
        "[10,20,30] | limit(1; .[])",
        "[10,20,30] | limit(1.1; .[])",
        "[10,20,30] | limit(1.9; .[])",
        "[10,20,30] | limit(2.0; .[])",
        "[10,20,30] | limit(2.1; .[])",
        "[10,20,30] | limit(2.9; .[])",
        "[10,20,30] | limit(-0.1; .[])",
        "[10,20,30] | limit(-0; .[])",
        "[10,20,30] | limit(-0.0; .[])",
        "[10,20,30] | limit(nan; .[])",
        "[10,20,30] | limit(infinite; .[])",
        "[10,20,30] | limit(null; .[])",
        "[10,20,30] | limit(false; .[])",
        "[10,20,30] | limit(true; .[])",
        "try ([10,20,30] | limit(\"a\"; .[])) catch .",
        "try limit(\"a\"; 10,20) catch .",
        "try limit({}; 10,20) catch .",
        "try limit([]; 10) catch .",
        "try limit(\"\"; 1,2) catch .",
        "try limit([1]; 1,2) catch .",
        "try limit(\"a\"; empty) catch .",
        "try limit(\"a\"; error(\"z\")) catch .",
        "try limit(1; error(\"z\")) catch .",
        "try limit(0; error(\"z\")) catch .",
        "try limit(-1; error(\"z\")) catch .",
        "try limit(nan; 1, error(\"z\")) catch .",
        "try limit(1; 1, error(\"z\"), 3) catch .",
        "try limit(2; 1, error(\"z\"), 3) catch .",
        "try limit(1; (1, error(\"z\")) | .) catch .",
        "try limit(1; 1 | ., error(\"z\")) catch .",
        "try limit(2; 1, [error(\"z\")]) catch .",
        "try limit(1, error(\"z\"); 5) catch .",
        "limit(2; 1,2,3,4)",
        "limit(2; empty)",
        "1,2,3 | limit(2; .)",
        "[1,2,3] | limit(1; .[]) , 9",
        "limit(2; 1,2,3), 9",
        "[1,2,3,4] | limit(1,2; .[])",
        "[1,2,3,4] | limit(1,2,0,-1; .[])",
        "limit(infinite; 1,2,3)",
        "first(1,2,3)",
        "last(1,2,3)",
        "nth(0; 1,2,3)",
        "nth(2; 1,2,3)",
        "nth(5; 1,2,3)",
        "first(empty)",
        "last(empty)",
        "nth(0; empty)",
        "first(1, error(\"z\"), 3)",
        "try first(1, error(\"z\"), 3) catch .",
        "try last(1, error(\"z\"), 3) catch .",
        "try nth(1; 1, error(\"z\"), 3) catch .",
        "try nth(0; error(\"z\"), 1) catch .",
        "last(1, error(\"z\"), 3)",
        "try first((1, error(\"z\")) | .) catch .",
        "try nth(0; 1, error(\"z\"), 3) catch .",
        "try nth(1; 1, 2, error(\"z\")) catch .",
        "try nth(5; 1, error(\"z\")) catch .",
        "try nth(0, -1; 10,20) catch .",
        "try nth(-1, 0; 10,20) catch .",
        "nth(0.1; 10,20,30,40,50)",
        "nth(0.9; 10,20,30,40,50)",
        "nth(1.0; 10,20,30,40,50)",
        "nth(1.1; 10,20,30,40,50)",
        "nth(2.1; 10,20,30,40,50)",
        "nth(2.9; 10,20,30,40,50)",
        "nth(4; 10,20,30,40,50)",
        "nth(4.1; 10,20,30,40,50)",
        "nth(5; 10,20,30,40,50)",
        "nth(-0; 10,20,30)",
        "try nth(-0.1; 10,20) catch .",
        "try nth(nan; 1,2,3) catch .",
        "nth(infinite; 1,2,3)",
        "try nth(-infinite; 1,2) catch .",
        "try nth(\"a\"; 1,2) catch .",
        "try nth(\"a\"; error(\"z\")) catch .",
        "try nth(null; 1,2) catch .",
        "try nth(false; 1,2) catch .",
        "try nth(true; 1,2) catch .",
        "try nth([1]; 1,2) catch .",
        "try nth(1.5, \"a\"; 1,2,3) catch .",
        "nth(0,1,5; 10,20,30)",
        "first(true)",
        "1 | first(.)",
        "null | first(.)",
        "empty | first",
        "empty | last",
        "empty | first(.)",
        "1 | first(empty)",
        "first(empty, 4, error(\"z\"))",
        "try first(error(\"z\"), 1) catch .",
        "try last(error(\"z\"), 1) catch .",
        "last(1,2,empty,3)",
        "first(range(3))",
        "last(range(3))",
        "nth(1; range(5))",
        "nth(10; range(3))",
        "nth(1.5; range(4))",
        "nth(2.0; range(4))",
        "nth(2.5; range(6))",
        "[1,2] | nth(1)",
        "[[1,2],[3]] | nth(0)",
        "[] | nth(0)",
        "[10] | nth(0)",
        "[10] | nth(1)",
        "try (\"abcd\" | nth(2)) catch .",
        "try ({\"a\":1,\"b\":2} | nth(0)) catch .",
        "null | nth(1.5)",
        "[range(4)]",
        "[range(0)]",
        "[range(-1)]",
        "[range(1.5)]",
        "[range(2.0)]",
        "[range(0.5)]",
        "[range(-0.5)]",
        "[range(-0)]",
        "[range(0; -0)]",
        "[range(-0; 0)]",
        "[range(-0; 3)]",
        "[range(1;4)]",
        "[range(1;1)]",
        "[range(4;1)]",
        "[range(1;5;2)]",
        "[range(5;1;-2)]",
        "[range(0;1;0.3)]",
        "[range(1;0;-0.4)]",
        "[range(0;1;0)]",
        "[range(1;2;-1)]",
        "[range(0;1;0.1)]",
        "[range(2; 2.1; 0.05)]",
        "[range(5;0;-1)]",
        "[range(2;2)]",
        "[range(10;0;1)]",
        "[range(0;10;-1)]",
        "[range(1;1;1)]",
        "[range(1;1;-1)]",
        "[range(-2;2)]",
        "[range(-2;-5;-1)]",
        "[range(3; 3; 1)]",
        "[range(3; 3; -1)]",
        "[range(1; 4; -0)]",
        "[range(1; 3; -0.0)]",
        "[range(3; 1; -0.0)]",
        "try range(1;2;0) catch .",
        "try [range(1;2;0)] catch .",
        "try range(\"a\") catch .",
        "try range(1; \"b\") catch .",
        "try range(1; 2; \"c\") catch .",
        "try [range(1; 2; \"c\")] catch .",
        "try range(1; 4; \"c\") catch .",
        "try range(null) catch .",
        "try range(true) catch .",
        "try range(1; 2; null) catch .",
        "try range(1; 2; false) catch .",
        "try range(1; 2; true) catch .",
        "try range(1; 2; []) catch .",
        "try range(1; 2; -(nan)) catch .",
        "range(\"a\")",
        "range(1,2)",
        "[range(3), range(1;3), range(0;6;2)]",
        "1 | [range(.)]",
        "1,2,4 | [range(.)]",
        "1.5 | [range(.)]",
        "[range(1; 4, 2)]",
        "range(1,2; 3,4)",
        "try range(1, error(\"z\"); 3) catch .",
        "try range(1; 2, error(\"z\")) catch .",
        "try range(1, \"a\"; 3) catch .",
        "try range(1; 3, \"a\") catch .",
        "try [range(1; 3, \"a\")] catch .",
        "try range(1, 2; \"a\", 4) catch .",
        "try range({\"a\":1}) catch .",
        "try (range(1; 5; 2, error(\"z\"))) catch .",
        "[range(empty; 3)]",
        "[range(0; empty)]",
        "[range(0; 3; empty)]",
        "[range(1, empty, 2)]",
        "limit(empty; 1,2,3)",
        "nth(empty; 1,2,3)",
        "[limit(5; range(nan))]",
        "[limit(5; range(0; nan))]",
        "[limit(5; range(3; nan))]",
        "[limit(5; range(infinite))]",
        "[limit(3; range(-infinite))]",
        "[limit(4; range(0; 3; nan))]",
        "[limit(4; range(0; 3; infinite))]",
        "[limit(6; range(5; 0; nan))]",
        "[limit(6; range(0; 5; nan))]",
        "[limit(6; range(nan; 5))]",
        "[limit(6; range(nan; 5; -1))]",
        "[limit(6; range(5; nan; -1))]",
        "[limit(4; range(nan; nan))]",
        "[limit(4; range(nan; nan; -1))]",
        "[limit(4; range(0; -5; nan))]",
        "[limit(4; range(-5; nan))]",
        "[limit(5; range(0; nan; 1))]",
        "[limit(5; range(0; nan; 2))]",
        "[limit(5; range(0; nan; 0.5))]",
        "[limit(3; range(1.0; nan))]",
        "[limit(3; range(1.5; nan))]",
        "[limit(3; range(1.5; nan; 1))]",
        "[limit(4; range(1; nan; -0.5))]",
        "[limit(3; range(-(nan)))]",
        "[limit(3; range(9007199254740993; nan))]",
        "[range(1.5; 4.5)]",
        "1 | until(.>3; .+1)",
        "5 | until(.>3; .+1)",
        "0 | until(.==3; .+1)",
        "0 | until(.>=3; .+1)",
        "1 | until(.; .+1)",
        "0 | until(.; .+1)",
        "1 | [until(true, true; .+1)]",
        "1 | until(empty; .+1)",
        "false | until(.; 1)",
        "null | until(.; 1)",
        "\"\" | until(.; 1)",
        "[] | until(.; 1)",
        "{} | until(.; 1)",
        "1 | until(. == 1; error(\"z\"))",
        "try (5 | until(.>3; error(\"z\"))) catch .",
        "try (1 | until(.>2; error(\"z\"))) catch .",
        "try (1 | until(error(\"z\"); .+1)) catch .",
        "try (1 | until(.>3, error(\"z\"); .+1)) catch .",
        "4 | until(.>3, error(\"z\"); .+1)",
        "try (4 | until(.>3, error(\"z\"); .+1)) catch .",
        "try (1 | until(false; error(\"z\"))) catch .",
        "1 | [until(.>=3; .+1, .+2)]",
        "10 | [until(.<5; .-3, .-1)]",
        "1 | [limit(10; until(.>3; .+1, .+10))]",
        "{\"a\":1} | until(.a > 3; .a += 1)",
        "\"aaa\" | until(length < 2; .[1:])",
        "\"abc\" | until(. == \"c\"; .[1:])",
        "\"ab\" | until(. == \"b\"; .[1:])",
        "3 | until(.>10 and .>0; .+1)",
        "try (1 | until(.[0]; .+1)) catch .",
        "1 | until(.>2; . as $x | $x+1)",
        "1 | [while(. < 4; .+1)]",
        "5 | [while(. < 4; .+1)]",
        "0 | [while(. < 3; . + 1)]",
        "0 | [limit(5; while(true; .+1))]",
        "1 | [while(.<2; .+1)]",
        "false | [while(.; 1)]",
        "1 | [while(empty; .+1)]",
        "1 | [while(.>5; .+1)]",
        "1 | [while(.<3; .+1, .+2)]",
        "1 | [while(.<4; .+1, .+10)]",
        "1 | [while(.<5; .+1, .+2)]",
        "1 | [while(.>5, .<3; .+1)]",
        "1 | [while(. < 0, . < 3; .+1)]",
        "1 | [while(. < 3, . < 0; .+1)]",
        "1 | [limit(4; while(true, false; .+1))]",
        "1 | [limit(6; while(.<3, true; .+1))]",
        "2 | [while(.>0; .-1, empty)]",
        "5 | [while(.>0; . - 2)]",
        "[1,2,3,4] | [while(length > 0; .[1:])]",
        "try (1 | while(. < 3; error(\"z\"))) catch .",
        "try (1 | while(error(\"z\"); .+1)) catch .",
        "try (1 | while(false, error(\"z\"); .+1)) catch .",
        "try (1 | [while(. < 4; error(\"z\"), .+1)]) catch .",
        "0 | [limit(1; while(true; error(\"z\"), .+1))]",
        "try (0 | [limit(2; while(true; .+1, error(\"z\")))]) catch .",
        "1 | [limit(0; while(true; .+1))]",
        "try (1 | while(. < 0; error(\"z\"))) catch .",
        "1 | [limit(4; repeat(.+1))]",
        "0 | [limit(4; repeat(.+1))]",
        "1 | [limit(3; repeat(1,2))]",
        "1 | [limit(3; repeat(.))]",
        "1 | [limit(5; repeat(1, empty, 2))]",
        "1 | [limit(4; repeat(.+1, .+2))]",
        "0 | [limit(3; repeat(empty, 1))]",
        "\"ab\" | [limit(5; repeat(.[1:]))]",
        "[10,20] | [limit(3; repeat(.[0]))]",
        "2 as $n | [limit(3; repeat($n))]",
        "try [limit(2; repeat(error(\"z\")))] catch .",
        "try (1 | [limit(3; repeat(1, error(\"z\"), 2))]) catch .",
        "try (1 | limit(1; repeat(error(\"z\"), 1))) catch .",
        "1 | limit(3; repeat(.)) | select(. == 1)",
        "1,2 | [limit(2; repeat(.+1))]",
        "[limit(3; 1,2,3,4)]",
        "[limit(2; limit(10; range(5)))]",
        "[limit(2; limit(-1; range(4)))]",
        "first(first(1, error(\"z\"), 2), error(\"q\"))",
        "try first(first(error(\"z\"), 1), 2) catch .",
        "5 as $n | limit($n; range(10))",
        "2 as $n | [range($n; $n+3)]",
        "1 as $x | 2 as $y | [range($x; $y; -1)]",
        "1,2 | [range(.; .+2)]",
        "[1,2,3] | first = 9",
        "[1,2,3] | last = 9",
        "[1,2,3] | nth(1) = 9",
        "[1,2,3] | nth(1+1) = 7",
        "[1,2,3] | nth(0, 2) = 8",
        "[1,2,3,4] | nth(1, 0) = 8",
        "[1,2,3] | first |= .+10",
        "[1,2,3] | last |= .+1",
        "[1,2,3] | nth(1) |= .+10",
        "[1,2,3] | first += 4",
        "[1,2,3] | last *= 2",
        "[1,2,3] | nth(1) -= 1",
        "[] | first = 9",
        "null | first = 9",
        "null | first = 1 | last = 2 | nth(3) = 4",
        "[] | nth(2) = 5",
        "[1] | last = 6",
        "[1,2] | nth(-1) = 7",
        "[1,2] | nth(-2) = 7",
        "try ([1,2] | nth(-3) = 7) catch .",
        "try ([] | last = 6) catch .",
        "try (\"ab\" | first = \"z\") catch .",
        "try ([1,2] | nth(nan) = 7) catch .",
        "try ([1,2,3] | nth(\"a\") = 1) catch .",
        "[1,2,3] | nth(5) = 9",
        "null | nth(5) = 9",
        "[1,2,3] | first(.[]) = 9",
        "[1,2,3] | first(.[1]) = 9",
        "[1,2,3] | first(.[0], .[2]) = 9",
        "[1,2,3] | first(empty, .[1]) = 8",
        "try ([1,2,3] | first(.[], error(\"z\")) = 9) catch .",
        "try ([1,2,3] | first(1) = 9) catch .",
        "try ([1,2,3] | last(.[]) = 9) catch .",
        "[1,2,3] | nth(1; .[]) = 9",
        "[1,2,3] | nth(0; .[]) = 4",
        "[1,2,3] | nth(1.5; .[]) = 9",
        "try ([1,2,3] | nth(5; .[]) = 9) catch .",
        "try ([1,2,3] | nth(1; 1) = 9) catch .",
        "try ([1,2,3] | nth(-1; .[]) = 9) catch .",
        "[1,2,3] | nth(1; .[0], .[2]) = 9",
        "try ([1,2,3] | nth(0; .[1], error(\"z\")) = 9) catch .",
        "try ([1,2,3] | nth(1; .[1], error(\"z\")) = 9) catch .",
        "[1,2,3] | limit(2; .[]) = 9",
        "[1,2,3] | limit(0; .[]) = 9",
        "try ([1,2,3] | limit(0; error(\"z\")) = 9) catch .",
        "try ([1,2,3] | limit(1; .[], error(\"z\")) = 9) catch .",
        "try ([1,2,3] | limit(2; .[], error(\"z\")) = 9) catch .",
        "try ([1,2,3] | limit(-1; .[], error(\"z\")) = 9) catch .",
        "[1,2,3] | limit(1; .[1], .[0]) = 8",
        "[1,2,3,4] | (first, last) = 9",
        "[1,2,3] | del(first)",
        "[1,2,3] | del(last)",
        "[1,2,3] | del(nth(1))",
        "[1,2,3] | del(first(.[]))",
        "[1,2,3,4] | del(first, last)",
        "[1,2,3,4] | del(nth(1), nth(2))",
        "null | del(first)",
        "[] | del(first)",
        "[1] | del(first)",
        "[1] | del(last)",
        "[1,2,3] | path(first)",
        "[1,2,3] | path(last)",
        "[1,2,3] | path(nth(1))",
        "[1,2,3] | path(nth(-1))",
        "[1,2,3] | path(nth(1.5))",
        "[1,2,3] | path(nth(1.0))",
        "[1,2,3] | path(nth(1.50))",
        "[1,2,3] | path(nth(-1.2))",
        "[1,2,3] | path(nth(-0))",
        "[1,2,3] | path(nth(-0.0))",
        "[1,2,3] | path(nth(1+1))",
        "[1,2,3] | path(nth(2*1.5))",
        "[1,2,3] | path(nth(nan))",
        "[1,2,3] | path(nth(infinite))",
        "null | path(first)",
        "null | path(last)",
        "null | path(nth(0))",
        "null | path(nth(1.5))",
        "[] | path(first)",
        "[] | path(last)",
        "[1,2,3] | path(first, last)",
        "[1,2,3] | path(first(.[1]))",
        "[1,2,3] | path(first(.[]))",
        "try ([1,2,3] | path(last(.[1]))) catch .",
        "try ([1,2,3] | path(last(.[]))) catch .",
        "[1,2,3] | path(nth(1; .[]))",
        "[1,2,3] | path(nth(0; .[]))",
        "[1,2,3] | path(nth(1; .[0], .[2]))",
        "[1,2,3] | path(first(.[0], .[2]))",
        "[1,2,3] | path(first(empty, .[2]))",
        "try ([1,2,3] | path(first(1))) catch .",
        "[1,2,3] | path(limit(1; .[]))",
        "[1,2,3] | path(limit(2; .[]))",
        "[1,2,3] | path(limit(0; .[]))",
        "[1,2,3] | path(limit(-1; .[]))",
        "[1,2,3] | path(limit(1; .[1], .[0]))",
        "try ([1,2,3] | path(range(2))) catch .",
        "[1,2,3] | path(first(.[1:]))",
        "try ([1,2,3] | first(.[1:]) = 9) catch .",
        "[1,2,3] | first(.[1:]) = [9]",
        "[1,2,3] | .[first]",
        "[1,2,3] | .[nth(1)]",
        "[1,2,3,4] | .[first] = 9",
        "[1,2,3] | (first | .) = 9",
        "[1,2,3] | (. | first) = 9",
        "[1,2,3] | first(.) = 9",
        "[1,2,3] | first(.[]) |= .+1",
        "try ([1,2,3] | nth(1; .[]) |= .+5) catch .",
        "[1,2,3] | first(.[] | select(.>1)) = 9",
        "try ([1,2,3] | first(select(.>0)) = 9) catch .",
        "{\"a\":1} | .a |= until(.>3; .+1)",
        "[0,0,0] | nth(0,1,2) = 1",
        "reduce range(4) as $i (0; . + $i)",
        "[foreach range(3) as $i (0; .+$i)]",
        "nth(-1; 1)",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }

    let fed = [
        ("[range(0;10)]", "null"),
        ("[range(0,1;3,4)]", "null"),
        ("[range(0;10;3)]", "null"),
        ("[range(0;10;-1)]", "null"),
        ("[range(0;-5;-1)]", "null"),
        ("[range(0,1;4,5;1,2)]", "null"),
        ("[while(.<100; .*2)]", "1"),
        (
            "[.[]|[.,1]|until(.[0] < 1; [.[0] - 1, .[1] * .[0]])|.[1]]",
            "[1,2,3,4,5]",
        ),
        ("[foreach range(5) as $item (0; $item)]", "null"),
        ("[limit(3; .[])]", "[11,22,33,44,55,66,77,88,99]"),
        ("[limit(0; error)]", "\"badness\""),
        ("[limit(1; 1, error)]", "\"badness\""),
        ("[first(range(.)), last(range(.))]", "10"),
        (
            "[nth(0,5,9,10,15; range(.)), try nth(-1; range(.)) catch .]",
            "10",
        ),
        ("first(1,error(\"foo\"))", "null"),
        ("[limit(5,7; range(9))]", "null"),
        ("[nth(5,7; range(9;0;-1))]", "null"),
        ("[range(0,1,2;4,3,2;2,3)]", "null"),
        ("[range(3,5)]", "null"),
        ("[1,2,3] | first(empty, .[1])", "[1,2,3]"),
        ("try (1 | [limit(3; repeat(.[0]))]) catch .", "1"),
    ];
    for (program, input) in fed {
        compare_exact(program, &["-c", "--", program], input.as_bytes());
    }
}

#[test]
fn defs_match_jq() {
    require_jq();
    let programs = [
        "def f: 1; f",
        "def f: 1; def f: 2; f",
        "def f: 1; def g: def f: 2; f; [g, f]",
        "1, def f: 2; f",
        "def f: 1, 2; f",
        "def f: 1 | .+10; f",
        "10 | def f: .+1; f",
        "(def f: 1; f), 2",
        "def f: 1; (def f: 2; f), f",
        "def f: 1; [def f: 2; f], f",
        "def a: 0; . | a",
        "if true then def f: 4; f else 0 end",
        "reduce (def f: 1,2; f) as $x (0; .+$x)",
        "def f: 1; 2 as $x | f + $x",
        "def floor: 9; floor",
        "def empty: 4; [empty]",
        "def nan: 1; nan",
        "def infinite: 2; infinite",
        "def true: 1; true",
        "def false: 1; false",
        "def null: 1; null",
        "def not: 7; not",
        "def add(a;b): a*b; add(3;4)",
        "def map(f): f+1; map(10)",
        "def f(a): 1 | a; 10 | f(.+5)",
        "def f(a): [1 | a]; 10 | f(.+5)",
        "def f(a): [a, a]; f(1,2)",
        "def f(a;b): [a,b]; f(1,2; 3,4)",
        "def f(a;b): a+b; [f(1,2; 10,20)]",
        "def f(a): a | a; [f(1,2)]",
        "def f(a): 1; f(error(\"z\"))",
        "def f(a): a; try f(1, error(\"z\")) catch .",
        "def f(a): [a]; try f(1, error(\"z\")) catch .",
        "def f(a): [limit(1; a)]; try f(1, error(\"z\"), 3) catch .",
        "def f($a): $a; [f(1,2)]",
        "def f($a;$b): [$a,$b]; [f(1,2; 3,4)]",
        "def f($a;$b): $a+$b; [f(1,2; 10,20)]",
        "7 as $a | def f($a; $b): [$a, $b]; f(1; $a)",
        "def f($a): 1 | $a; 10 | f(.+5)",
        "def f($a): 1; try f(error(\"z\")) catch .",
        "def f($a): $a; try f(1, error(\"z\"), 3) catch .",
        "def f($a): [$a,$a]; [f(1,2)]",
        "def f($a): $a; [f(empty), 1]",
        "def f($a): 1; [limit(1; f(1, error(\"z\"), 3))]",
        "def f($a): $a; [limit(2; f(1,2,3,4))]",
        "def f($a; b; $c): [$a, b, $c]; [f(1,2; 10,20; 3,4)]",
        "def fac($n): if $n <= 1 then 1 else $n * fac($n-1) end; fac(5)",
        "def f($a): a; f(5)",
        "def f($a): a + 1; f(5)",
        "def f($a): 1 | a; f(5)",
        "def f($a): 1 | a; 10 | f(.+5)",
        "10 as $x | def f($a): 1 | a; f($x)",
        "def f($a): a; f(1,2)",
        "def f($a): [a, a]; [f(1,2)]",
        "def f($a): [a, $a]; f(5)",
        "def f($a): [a, $a]; [f(1,2)]",
        "def f($a): a+1; [f(1,2)]",
        "def f($a): a, a; [f(1,2)]",
        "def f($a; $b): [a, b]; f(1,2; 3,4)",
        "def f($a): . + a; 10 | f(1)",
        "def f($a): a | . + 1; f(5)",
        "def f($a): a(.); f(5)",
        "5 as $a | a",
        "def f($a): a; try f(1, error(\"z\")) catch .",
        "def f($nan): nan; f(1)",
        "def f(nan): nan; f(1)",
        "def f($length): length; f(3)",
        "def f(length): length; f(3)",
        "def f(g): def g: 5; g; f(1)",
        "def g: 1; def f(g): g; f(2)",
        "def f(g): [g, def g: 5; g]; f(1)",
        "def f($a): def a: 9; a; f(5)",
        "def a: 9; def f($a): a; f(5)",
        "def f($a): def g: a; g; f(3)",
        "def f(a): def g: a; def a: 4; g; f(3)",
        "def f($a; a): [$a, a]; f(1; 2)",
        "def f(a; $a): [$a, a]; f(2; 1)",
        "def f($a; $a): [$a, a]; f(1; 2)",
        "def f($a; $b; $a): [$a, $b, a]; f(1; 3; 2)",
        "def f(a; a): a; f(1; 2)",
        "def f: if . == 0 then 0 else .-1 | f end; 100000 | f",
        "def f: if . == 0 then 0 else (.-1 | f) + 0 end; 100000 | f",
        "def f: [1,2]; f",
        "def f: {a:1,b:2}; f",
        "def f: empty // 3; f",
        "def f: 0 // 3; f",
        "def f: null // 4; f",
        "def f: try error(\"z\") catch .; f",
        "def f: try (1, error(\"z\")) catch .; f",
        "def f: 1 as $x | $x + 2; f",
        "def f: reduce (1,2) as $x (0; . + $x); f",
        "def f: first(1,2,3); f",
        "def f: [first(1, error(\"z\"))]; f",
        "def f: [limit(1; 1, error(\"z\"), 3)]; f",
        "def f: [1,2] | map(.+1); f",
        "def f: [10,20][0]; f",
        "def f: if . == 0 then 0 else [(.-1|f)][0] end; 10000 | f",
        "def f: if . == 0 then 0 else {a:(.-1|f)}.a end; 10000 | f",
        "def f: if . == 0 then 0 else (.-1|f) // 1 end; 10000 | f",
        "def f: if . == 0 then 0 else try (.-1|f) end; 10000 | f",
        "def f: if . == 0 then 0 else reduce (.-1|f) as $x (null; $x) end; 10000 | f",
        "def f: if . == 0 then 0 else (.-1|f) as $x | $x end; 10000 | f",
        "def f: if . == 0 then 0 else first(.-1|f) end; 10000 | f",
        "def f: if . == 0 then 0 else limit(1; .-1|f) end; 10000 | f",
        "def f: if . == 0 then 0 else [.-1] | map(f) | .[0] end; 10000 | f",
        "def f($n): if $n<=0 then 0 else f($n-1) end; f(100000)",
        "def f($n): if $n<=0 then 0 else f($n-1)+0 end; f(100000)",
        "def f: 1,2; [limit(1; f)]",
        "def f: 1, error(\"z\"), 3; try f catch .",
        "[limit(3; def f: 1, f; f)]",
        "def f($a): def g: $a; g; f(3)",
        "def f(a): def g: a; g; [f(1,2)]",
        "def f($a): def g($b): $a+$b; g(4); f(3)",
        "def f($a): def g: def h: $a; h; g; f(8)",
        "def f(a): def g(b): [a,b]; g(9); f(8)",
        "5 as $n | def f: def g: $n; g; f",
        "def f($a): def g($a): $a; g(2); f(1)",
        "def f(a): def g(a): a; g(7); f(3)",
        "def f(a): def g: a; [g,g]; f(1,2)",
        "def f($a): def g: $a+1; g; f(4)",
        "def f(a): def g: a+1; g; [f(10,20)]",
        "def f($a): def g(b): b+$a; g(10,20); [f(3)]",
        "def f(a): a; def g: f(1,2); [g]",
        "def a: 9; def f(a): a; f(2)",
        "def f: .+1; def g: def h: f; def f: .+10; h; g",
        "foo",
        "def f: 1; f(1)",
        "length(1)",
        "map",
        "not(1)",
        "def f(g): g(1); def h(x): x+1; f(h)",
        "lengthx",
        "hasx(1)",
        "pow",
        "def f: foo(bar); f",
        "def f: a; 1",
        "def f: $missing; 1",
        "def f: def g: a; 1; f",
        "def f: 1; def g: f; 1",
        "def f: if .==0 then a else 1 end; 1 | f",
        "def f($a): $a; f(1|2)",
        "def f(a;b;c;d;e;f;g;h;i;j): [j,i,h,g,f,e,d,c,b,a]; f(0;1;2;3;4;5;6;7;8;9)",
        "def id(x):x; 2000 as $x | def f(x):1 as $x | id([$x, x, x]); def g(x): 100 as $x | f($x,$x+x); g($x)",
        "def f: 1; def g: f, def f: 2; def g: 3; f, def f: g; f, g; def f: 4; [f, def f: g; def g: 5; f, g]+[f,g]",
        "def f: 1; f.a",
        "def f: empty; [f, 1]",
        "def type: 1; type",
        "def error: 4; try error catch .",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
    let fed = [
        ("def f: . + 1; f", "3"),
        ("def f($a): . + a; f(5)", "10"),
        (
            "def f(g): def g: if . == 0 then 0 else .-1 | g end; g; f(1)",
            "4",
        ),
        (
            "def f: .+1; def g: f; def f: .+100; def f(a):a+.+11; [(g|f(20)), f]",
            "1",
        ),
        ("def length: 3; [length]", "\"abcd\""),
        ("def length(x): x; [length, length(5)]", "\"ab\""),
        ("def f(a): a; f(.+5)", "10"),
        (
            "def fac: if . == 1 then 1 else . * (. - 1 | fac) end; [.[] | fac]",
            "[1,2,3,4]",
        ),
        (
            "def fac(n): if n <= 1 then 1 else n * ((n-1)|fac(.)) end; fac(.)",
            "5",
        ),
        ("def f: if . > 3 then . else .+1 | f end; f", "0"),
        ("def x: .[1,2]; x=10", "[0,1,2]"),
        (
            "def inc(x): x |= .+1; inc(.[].a)",
            "[{\"a\":1,\"b\":2},{\"a\":2}]",
        ),
        ("def f($i): .[$i]; f(1)=9", "[0,1,2]"),
        ("def f($i): .[$i]; f(0,2)=8", "[0,1,2]"),
        ("def f(a): a; f(.[1])=9", "[0,1,2]"),
        ("def f(a): a |= .+1; f(.[1])", "[0,1,2]"),
        ("def f($a; b): b; f(1; .+9)", "10"),
        ("def f(a; $b): a+$b; f(.+1; 3,4)", "10"),
        (
            "try (def f($i): .[$i]; f(0, error(\"z\")) = 8) catch .",
            "[0,1,2]",
        ),
        ("def f: .a; f", "{\"a\":1}"),
        ("def a: 0; . | a, 1", "5"),
        ("def f: 5 as $n | $n + .; f", "3"),
        (
            "def f(a;b;c;d;e;f): [a+1,b,c,d,e,f]; f(.[0];.[1];.[0];.[0];.[0];.[0])",
            "[1,2]",
        ),
        (
            "def x(a;b): a as $a | b as $b | $a + $b; def y($a;$b): $a + $b; def check(a;b): [x(a;b)] == [y(a;b)]; check(.[];.[]*2)",
            "[1,2,3]",
        ),
        ("def f(x): x | x; f([.], . + [42])", "[1,2,3]"),
        (
            "def f: . + 1; def g: def g: . + 100; f | g | f; (f | g), g",
            "3",
        ),
        ("def f: (1000,2000); f", "123412345"),
        (
            "[[20,10][1,0] as $x | def f: (100,200) as $y | def g: [$x + $y, .]; . + $x | g; f[0] | [f][0][1] | f]",
            "999999999",
        ),
        (
            "def f: 1; def g: f, def f: 2; def g: 3; f, def f: g; f, g; def f: 4; [f, def f: g; def g: 5; f, g]+[f,g]",
            "null",
        ),
    ];
    for (program, input) in fed {
        compare_exact(program, &["-c", "--", program], input.as_bytes());
    }
}

#[test]
fn ordering_match_jq() {
    require_jq();

    let filters = [
        "sort",
        "unique",
        "min",
        "max",
        "sort_by(.)",
        "group_by(.)",
        "unique_by(.)",
        "min_by(.)",
        "max_by(.)",
        "sort_by(error(\"z\"))",
        "group_by(error(\"z\"))",
        "unique_by(error(\"z\"))",
        "min_by(error(\"z\"))",
        "max_by(error(\"z\"))",
    ];
    let values = [
        "null",
        "false",
        "true",
        "0",
        "1",
        "1.50",
        "-0",
        "nan",
        "infinite",
        "-infinite",
        "\"\"",
        "\"a\"",
        "\"bac\"",
        "\"abcdefghijklmnopqrstuvwxyz\"",
        "[]",
        "[1]",
        "{}",
        "{\"a\":1}",
        "{\"b\":1,\"a\":2}",
        "{\"a\":1,\"b\":1}",
        "{\"\":1}",
        "{\"a\":\"abcdefghijklmnopqrstuvwxyz\"}",
        "{\"abcdefghijklmnop\":[1,2,3,4]}",
    ];
    for filter in filters {
        for value in values {
            let program = format!("{value} | {filter}");
            compare_exact(&program, &["-n", "-c", "--", &program], b"");
        }
    }

    let caught = [
        "sort",
        "unique",
        "min",
        "max",
        "sort_by(.)",
        "group_by(.)",
        "unique_by(.)",
        "min_by(.)",
        "max_by(.)",
    ];
    let caught_on = [
        "null",
        "1.50",
        "-0",
        "nan",
        "infinite",
        "-infinite",
        "\"abcdefghijklmnopqrstuvwxyz\"",
        "{}",
        "{\"b\":3,\"a\":1}",
        "{\"a\":\"abcdefghijklmnopqrstuvwxyz\"}",
    ];
    for filter in caught {
        for value in caught_on {
            let program = format!("try ({value} | {filter}) catch .");
            compare_exact(&program, &["-n", "-c", "--", &program], b"");
        }
    }

    let programs = [
        "[3,1,2] | sort",
        "[3,1,2] | sort_by(.)",
        "[3,1,2] | unique",
        "[3,1,2] | group_by(.)",
        "[3,1,2] | min",
        "[3,1,2] | max",
        "[3,1,2] | min_by(.)",
        "[3,1,2] | max_by(.)",
        "[1,2,1,3,2] | unique",
        "[1,1,1] | [sort, unique, group_by(.), min, max]",
        "[2,1,2,1] | [sort, unique, group_by(.)]",
        "[] | [sort, unique, group_by(.), min, max, min_by(.), max_by(.)]",
        "[5] | [min, max, min_by(.), max_by(.)]",
        "[null,false,true,0,1,\"a\",[],{}] | [sort, unique, group_by(.), min, max]",
        "[1,\"a\",true,null,[]] | [min, max]",
        "[null,1,null] | [min, max]",
        "[0,-0,1,-1] | sort",
        "[0,-0] | [min, max, unique, group_by(.)]",
        "[-0,0] | [min, max, unique]",
        "[0,-0,0] | unique",
        "[1.0, 1, 1.00, 2] | unique",
        "[1.50, 1.5, 2] | unique",
        "[01, 1] | sort",
        "[9007199254740993, 9007199254740992, -1] | sort",
        "[nan,1,nan,-nan,0,-0] | [sort, unique, group_by(.), min, max]",
        "[nan,-nan,nan] | unique | length",
        "[nan, -nan] | [min | copysign(1; .), max | copysign(1; .)]",
        "[nan, -nan, 1] | [min | copysign(1; .), max | copysign(1; .)]",
        "[1,nan,0] | [min | copysign(1; .), max]",
        "[infinite, -infinite, 1, nan] | sort",
        "[infinite, -infinite, 1] | [min, max]",
        "[[nan,1],[nan,2]] | [min, max, unique, group_by(.)]",
        "[[nan,1],[nan,1]] | [unique | length, group_by(.) | length]",
        "[[2,1],[1],[1,2],[]] | [sort, unique, min, max]",
        "[[3],[1,2],[1]] | [sort, min, max, min_by(.), max_by(.)]",
        "[[1,2],[1,1],[1],[1,2,0]] | sort",
        "[[1],[2,3]] | sort_by(.[1])",
        "[[5,1],[4,2],[3]] | [min_by(.[1]), max_by(.[1])]",
        "[[3,1],[2,0]] | sort_by(sort)",
        "[[3,1],[1,0,4]] | map(sort)",
        "[\"b\",\"a\",\"b\",\"aa\",\"ab\"] | [sort, unique, group_by(.), min, max]",
        "[\"é\",\"e\",\"A\",\"a\",\"b\",\"😀\"] | sort",
        "[\"A\",\"a\",\"b\",\"e\",\"é\"] | sort",
        "[[],[1]] | min",
        "[1,2,3] | sort_by(empty)",
        "[1,2] | sort_by(1, empty)",
        "[1,2] | group_by(empty)",
        "[10,20,30] | [min_by(empty), max_by(empty)]",
        "[1,2,3] | unique_by(empty)",
        "[1,2,3,4] | sort_by(select(. % 2 == 0))",
        "[1,2,3,4] | unique_by(select(. % 2 == 0))",
        "[1,2,3,4] | group_by(select(. % 2 == 0))",
        "[1,2,3,4] | group_by(. < 3)",
        "[1,2,3] | sort_by(if .==1 then 1,2 else . end)",
        "[1,2] | group_by(range(.))",
        "[2,1,2] | unique_by(range(.))",
        "[2,1] | min_by(range(.; .+1))",
        "[2,1,3] | sort_by(range(0; .))",
        "[1,2,3] | sort_by(. as $x | 3-$x)",
        "1 as $x | [3,1,2] | sort_by(. + $x)",
        "[[1,3],[2,0],[1,0]] | [min_by(.[0], .[1]), max_by(.[0], .[1])]",
        "[{\"a\":1,\"b\":2},{\"a\":1,\"b\":1},{\"a\":2,\"b\":0}] | sort_by(.a, .b)",
        "[{\"a\":1,\"b\":1},{\"a\":1,\"b\":2},{\"a\":2,\"b\":1}] | group_by(.b, .a)",
        "[1,2,3,4] | unique_by(. % 2, . > 2)",
        "[1,\"a\",2,true,\"b\",false] | unique_by(type)",
        "[{\"a\":1,\"i\":1},{\"a\":2,\"i\":2},{\"a\":1,\"i\":3},{\"a\":2,\"i\":4}] | sort_by(.a)",
        "[{\"a\":1,\"i\":1},{\"a\":1,\"i\":2},{\"a\":2,\"i\":3}] | unique_by(.a)",
        "[{\"a\":1,\"i\":2},{\"a\":2,\"i\":1},{\"a\":1,\"i\":3}] | group_by(.a)",
        "[{\"k\":1,\"n\":2},{\"k\":1,\"n\":1},{\"k\":0,\"n\":3}] | group_by(.k)",
        "[{\"a\":1,\"n\":2},{\"a\":0,\"n\":1},{\"a\":1,\"n\":0}] | unique_by(.a)",
        "[{\"a\":1,\"n\":1},{\"a\":1,\"n\":2},{\"a\":1,\"n\":3}] | sort_by(.a) | map(.n)",
        "[{\"a\":1,\"n\":1},{\"a\":2,\"n\":2},{\"a\":1,\"n\":3}] | min_by(.a)",
        "[{\"a\":2},{\"a\":1},{\"a\":2}] | [min_by(.a), max_by(.a)]",
        "[{\"a\":1,\"n\":1},{\"a\":2,\"n\":2},{\"a\":2,\"n\":3}] | max_by(.a)",
        "[{\"z\":1,\"a\":2},{\"a\":2,\"z\":1},{\"a\":1}] | sort",
        "[{\"z\":1,\"a\":2},{\"a\":2,\"z\":1}] | [unique, group_by(.)]",
        "[{\"b\":1,\"a\":2} < {\"a\":2,\"b\":1}, {\"a\":1} < {\"b\":1}]",
        "[{\"a\":1},{\"a\":1,\"b\":2},{\"b\":1},{}] | [sort, unique, min, max]",
        "[{\"a\":null},{\"a\":1},{\"a\":null}] | sort_by(.a)",
        "[{\"a\":nan},{\"a\":1},{\"a\":nan}] | sort_by(.a)",
        "[{\"i\":1,\"n\":nan},{\"i\":2,\"n\":-nan},{\"i\":3,\"n\":1}] | sort_by(.n) | map(.i)",
        "[{\"i\":1,\"a\":nan,\"b\":1},{\"i\":2,\"a\":nan,\"b\":2}] | [sort_by(.) | map(.i), min_by(.) | .i, max_by(.) | .i, unique_by(.) | map(.i), group_by(.) | map(map(.i))]",
        "[{\"a\":2},{\"b\":1},{\"a\":1}] | sort_by(.a)",
        "[{\"a\":2},{\"a\":1},{\"a\":2}] | min_by(.a)",
        "[3,1,2], [9,8] | sort",
        "[1,3,2] | sort # comment",
        "[3,1] | sort_by (.)",
        "def f: sort; [3,1,2] | f",
        "def k: .a; [{\"a\":2},{\"a\":1}] | sort_by(k)",
        "def g: .a; def f: sort_by(g); [{\"a\":2},{\"a\":1}] | f",
        "def k(f): sort_by(f); [3,1,2] | k(.)",
        "def sort: 9; [3,1] | sort",
        "def sort: 9; [3,1] | sort_by(.)",
        "def sort(f): f; [3,1] | [sort, sort(.)]",
        "def min: 4; [1,2] | min",
        "def min(f): f; [1,2] | [min, min(.)]",
        "def sort_by(f): 7; [3,1] | sort_by(.)",
        "def unique: sort; [3,1,1] | unique",
        "def insort: reduce .[] as $x ([]; . + [$x] | sort); [3,1,4,2] | insort",
        "[3,1,4,2] | reduce .[] as $x ([]; . + [$x] | sort)",
        "[1,2,3] | sort_by(if .==2 then error(\"z\") else . end)",
        "[1,2,3] | sort_by(. , if .==2 then error(\"z\") else 0 end)",
        "[1,2] | sort_by(1, error(\"z\"))",
        "[1,2,3] | sort_by(if .==1 then error(\"z\") else . end)",
        "[1,2] | sort_by(.a)",
        "[1,2] | sort_by(.a.b)",
        "[1,2] | min_by(.a)",
        "[1,2] | group_by(if .==2 then .a else . end)",
        "try ([1,2,3] | sort_by(if .==2 then error(\"z\") else . end)) catch .",
        "try ([1,2] | sort_by(1, error(\"z\"))) catch .",
        "try ([1,2] | group_by(if .==2 then .a else . end)) catch .",
        "{\"a\":1,\"b\":2} | sort_by(if .==2 then error(\"z\") else . end)",
        "{\"a\":1,\"b\":2} | sort_by(if .==1 then error(\"z\") else . end)",
        "{\"b\":3,\"a\":1} | sort_by(.+1)",
        "{\"b\":3,\"a\":1} | [unique, group_by(.), unique_by(.+1), min_by(.+1), max_by(.)]",
        "{\"a\":1} | sort_by(1,2)",
        "{\"a\":1} | sort_by(empty)",
        "1 | sort?",
        "[1 | sort?]",
        "(1 | sort?), 2",
        "[1] | sort_by(.a)?",
        "limit(1; ([3,1], 1) | sort)",
        "first(([3,1], 1) | sort)",
        "([1] | sort_by(error(\"a\"))), ([3,1] | sort)",
        "([3,1] | sort), ([1] | sort)",
        "1, ([2,1] | sort)",
        "sort, unique",
        "SORT",
        "sort(1)",
        "unique(1)",
        "min(1)",
        "max(1)",
        "sort_by",
        "group_by",
        "unique_by",
        "min_by",
        "max_by",
        "sort_by(.;.)",
        "group_by(.;.)",
        "min_by(1;2)",
        "[3,1,2] | . |= sort",
        "[3,1,2] | . |= sort_by(.)",
        "[[3,1],[2,0]] | .[] |= sort",
        "[[3,1],[2,0]] | .[0] |= sort",
        "[3,1,2] | sort_by(.) |= .",
        "try ([3,1,2] | sort |= .) catch .",
        "try ([] | sort |= .) catch .",
        "[] | sort_by(error(\"z\"))",
        "[] | min_by(error(\"z\"))",
        "[] | max_by(error(\"z\"))",
        "\"abc\" | sort_by(error(\"z\"))",
        "false | unique_by(error(\"z\"))",
        "null | min_by(error(\"z\"))",
        "1 | group_by(error(\"z\"))",
        "true | max",
        "\"bac\" | [sort, unique, min, max]",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }

    compare_exact("raw min", &["-n", "-r", "--", "[\"b\",\"a\"] | min"], b"");
    compare_exact("join min", &["-n", "-j", "--", "[\"b\",\"a\"] | min"], b"");
    compare_exact(
        "ascii long",
        &["-n", "-a", "--", "\"éééééééééé\" | sort"],
        b"",
    );
    compare_exact(
        "empty min status",
        &["-n", "-e", "-c", "--", "[] | min"],
        b"",
    );
    compare_exact(
        "empty max status",
        &["-n", "-e", "-c", "--", "[] | max"],
        b"",
    );
    compare_exact(
        "empty sort status",
        &["-n", "-e", "-c", "--", "[] | sort"],
        b"",
    );
    compare_exact(
        "false min status",
        &["-n", "-e", "-c", "--", "[false, true] | min"],
        b"",
    );
    compare_exact("error status", &["-n", "-e", "--", "1 | sort"], b"");
    compare_exact("pretty sort", &["-n", "--", "[3,1,2] | sort"], b"");
    compare_exact(
        "pretty group",
        &["-n", "--", "[3,1,2,1] | group_by(.)"],
        b"",
    );

    let fed = [
        ("sort", "[3,1,2]\n"),
        ("sort", "1.50"),
        ("sort", "-0\n"),
        ("min", "nan\n"),
        ("sort", "\"abcdefghijklmnopqrstuvwxyz\"\n"),
        ("unique", "{\"b\":1,\"a\":2}\n"),
        ("sort_by(.)", "{\"b\":3,\"a\":1}\n"),
        ("min_by(.)", "{\"a\":1}\n"),
        ("[3,1,2] | sort", "ignored\n"),
        ("sort", "[3,1]\n[2,0]\n"),
        ("sort", "1\n[3,1]\n"),
        ("try sort catch .", "1\n[3,1]\n"),
        ("sort_by(input)", "[3,1,2]\n30\n10\n20\n"),
        ("sort_by(input)", "[1,2,3]\n10\n20\n"),
        ("sort_by(input)", "[1,2]\n"),
        ("sort_by(inputs)", "[1,2,3]\n10\n20\n"),
        ("sort_by(inputs)", "[1,2,3]\n"),
        (". |= sort", "[3,1,2]\n"),
        (".[] |= sort", "[[3,1],[2,0]]\n"),
    ];
    for (program, input) in fed {
        compare_exact(program, &["-c", "--", program], input.as_bytes());
    }
    compare_exact("slurp", &["-s", "-c", "--", "sort"], b"3\n1\n2\n");
    compare_exact("pretty slurp", &["-s", "--", "sort"], b"3\n1\n2\n");

    let suite = [
        (
            "sort",
            "[42,[2,5,3,11],10,{\"a\":42,\"b\":2},{\"a\":42},true,2,[2,6],\"hello\",null,[2,5,6],{\"a\":[],\"b\":1},\"abc\",\"ab\",[3,10],{},false,\"abcd\",null]",
        ),
        (
            "(sort_by(.b) | sort_by(.a)), sort_by(.a, .b), sort_by(.b, .c), group_by(.b), group_by(.a + .b - .c == 2)",
            "[{\"a\": 1, \"b\": 4, \"c\": 14}, {\"a\": 4, \"b\": 1, \"c\": 3}, {\"a\": 1, \"b\": 4, \"c\": 3}, {\"a\": 0, \"b\": 2, \"c\": 43}]",
        ),
        ("unique", "[1,2,5,3,5,3,1,3]"),
        ("unique", "[]"),
        (
            "[min, max, min_by(.[1]), max_by(.[1]), min_by(.[2]), max_by(.[2])]",
            "[[4,2,\"a\"],[3,1,\"a\"],[2,4,\"a\"],[1,3,\"a\"]]",
        ),
        ("[min,max,min_by(.),max_by(.)]", "[]"),
    ];
    for (program, input) in suite {
        compare_exact(program, &["--", program], input.as_bytes());
    }
}

#[test]
fn reduce_build_matches_jq() {
    require_jq();
    let programs = [
        "reduce range(80) as $i ([]; . + [$i])",
        "reduce range(40) as $i ([]; . + [$i, $i + 1])",
        "reduce range(5) as $i ([1]; . + [$i])",
        "reduce range(4) as $i (null; . + [$i])",
        "reduce range(0) as $i ([9]; . + [$i])",
        "reduce range(3) as $i (0; . + [$i])",
        "reduce range(4) as $i ([0]; . + [.[-1] + $i])",
        "reduce range(3) as $i ([]; . + [])",
        "reduce range(3) as $i ([]; . + [if $i == 1 then error(\"z\") else $i end])",
        "[1,2,3] | reduce .[] as $x ([]; . + [$x])",
        "reduce (1,2) as $x ((10,20); . + [$x])",
        "reduce range(3) as $i ([]; . + [$i], . + [10])",
        "reduce range(80) as $i ({}; .[$i | tostring] = $i)",
        "reduce range(80) as $i ([]; .[$i] = $i)",
        "reduce range(5) as $i (null; .[$i | tostring] = $i)",
        "reduce range(5) as $i (null; .[$i] = $i)",
        "reduce (\"b\",\"a\",\"b\") as $k ({c:0}; .[$k] = 1)",
        "reduce range(4) as $i ({a:1}; .a = $i)",
        "reduce range(4) as $i ([9,8,7]; .[0] = $i)",
        "reduce range(3) as $i ([]; .[$i] = ($i, $i + 10))",
        "reduce range(3) as $i ({}; .[\"a\",\"b\"] = $i)",
        "reduce (0,1,-1) as $i ([9,8,7]; .[$i] = $i)",
        "reduce range(1) as $i ([]; .[-1] = $i)",
        "reduce range(1) as $i (null; .[-1] = 1)",
        "reduce range(4) as $i ([]; .[$i + 2] = $i)",
        "reduce range(5) as $i ([]; .[length] = $i)",
        "reduce range(2) as $i (1; .[$i] = $i)",
        "reduce range(2) as $i (\"ab\"; .[0] = $i)",
        "reduce range(2) as $i ({}; .[0] = $i)",
        "reduce range(1) as $i ([]; .[\"a\"] = 1)",
        "reduce range(2) as $i ([1,2,3]; .[1:2] = [$i])",
        "reduce range(3) as $i ({}; .a.b = $i)",
        "reduce range(4) as $i ([]; .[0] |= . + [$i])",
        "reduce range(4) as $i ({}; .[($i|tostring)] += 1)",
        "[foreach range(5) as $i (0; . + $i)]",
        "[foreach range(5) as $i ([]; . + [$i])]",
        "[foreach range(4) as $i ({}; .[$i|tostring] = $i)]",
        "def f: reduce range(40) as $i ([]; . + [$i]); f",
        "def f: reduce range(40) as $i ({}; .[$i|tostring] = $i); f",
        "def f: reduce range(40) as $i ([]; .[$i] = $i); f",
        "reduce range(3) as $i ([]; .[empty] = $i)",
        "reduce range(2) as $i ([1]; .[0] = empty)",
        "reduce range(1) as $i ({}; .[error(\"z\")] = 1)",
        "reduce range(1) as $i ([]; .[0] = (1, error(\"z\")))",
        "reduce range(1) as $i ([]; .[nan] = $i)",
        "{\"a\":1,\"a\":2} | reduce range(1) as $i (.; .a = 9)",
        "reduce range(3) as $i ({}; .[$i|tostring] = .)",
        "reduce range(80) as $i ([]; . + [$i]) | [.[0], .[-1], length]",
        "reduce range(80) as $i ({}; .[$i|tostring] = $i) | [.[\"0\"], .[\"79\"], length]",
        "reduce range(80) as $i ([]; .[$i] = $i) | [.[0], .[-1], length]",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
    compare_exact(
        "append input",
        &["-n", "-c", "--", "reduce range(3) as $i ([]; . + [input])"],
        b"10\n20\n30\n",
    );
}

#[test]
fn update_build_matches_jq() {
    require_jq();
    let programs = [
        "reduce range(30) as $i ([]; .[$i] |= $i)",
        "reduce range(5) as $i (null; .[$i] |= $i)",
        "reduce range(4) as $i ([9,8,7]; .[0] |= $i)",
        "reduce range(4) as $i ({a:1}; .a |= $i)",
        "reduce (\"b\",\"a\",\"b\") as $k ({c:0}; .[$k] |= 1)",
        "reduce range(3) as $i ({}; .[$i|tostring] |= $i)",
        "reduce range(3) as $i ([]; .[$i] |= ($i, $i + 10))",
        "reduce range(4) as $i ([1,2,3,4]; .[0] |= empty)",
        "reduce range(1) as $i ([1,2,3,4]; .[0,2] |= empty)",
        "reduce range(1) as $i ([1,2,3]; .[0,1] |= . + 10)",
        "reduce range(1) as $i ([1,2,3,4]; .[1,0] |= empty)",
        "reduce range(2) as $i (1; .[$i] |= $i)",
        "reduce range(2) as $i (\"ab\"; .[0] |= $i)",
        "reduce range(1) as $i ([]; .[-1] |= $i)",
        "reduce range(1) as $i (null; .[-1] |= 1)",
        "reduce range(1) as $i ([]; .[nan] |= $i)",
        "reduce range(1) as $i ({}; .[error(\"z\")] |= 1)",
        "reduce range(1) as $i ([]; .[0] |= (1, error(\"z\")))",
        "reduce range(1) as $i ([]; .[empty] |= 5)",
        "reduce range(3) as $i ([]; .[0] |= . + [$i])",
        "reduce range(2) as $i ([1,2,3]; .[1:2] |= [$i])",
        "{\"a\":1,\"a\":2} | reduce range(1) as $i (.; .a |= 9)",
        "reduce range(3) as $i ({}; .a.b |= $i)",
        "reduce (0,1,-1) as $i ([9,8,7]; .[$i] |= $i)",
        "reduce range(1) as $i ({}; .[0] |= $i)",
        "reduce range(1) as $i ([]; .[\"a\"] |= 1)",
        "reduce range(2) as $i ([]; .[$i + 2] |= $i)",
        "reduce range(3) as $i ([]; .[length] |= $i)",
        "reduce range(4) as $i ([]; .[$i] |= .)",
        "def f: reduce range(20) as $i ([]; .[$i] |= $i); f",
        "reduce range(30) as $i ([]; setpath([$i]; $i))",
        "reduce range(5) as $i (null; setpath([$i]; $i))",
        "reduce range(4) as $i ({}; setpath([$i|tostring]; $i))",
        "reduce range(3) as $i ([1,2,3]; setpath([0]; $i))",
        "reduce range(1) as $i ([1,2,3]; setpath([]; 9))",
        "reduce range(1) as $i ([1,2,3]; setpath(empty; 9))",
        "reduce range(1) as $i ([]; setpath([0]; empty))",
        "reduce range(1) as $i ([]; setpath(\"no\"; 1))",
        "reduce range(1) as $i (null; setpath([0,0]; $i))",
        "reduce range(2) as $i ([]; setpath([0]; $i, $i + 10))",
        "reduce range(1) as $i ([0]; setpath([1], []; 9, 8))",
        "reduce range(1) as $i ([0]; setpath([1], empty; 9, 8))",
        "reduce range(1) as $i (1; setpath([0]; $i))",
        "reduce range(1) as $i (\"ab\"; setpath([0]; 1))",
        "reduce range(1) as $i ({}; setpath([0]; 1))",
        "reduce range(3) as $i ([]; setpath([$i, 0]; $i))",
        "reduce range(1) as $i ([1,2,3]; setpath([0],[2]; 9))",
        "reduce range(1) as $i ([]; setpath([-1]; 1))",
        "reduce range(1) as $i (null; setpath([-1]; 1))",
        "reduce range(2) as $i ({a:1}; setpath([\"a\"]; $i))",
        "reduce range(1) as $i ([]; setpath([0]; 1, error(\"z\")))",
        "reduce range(1) as $i ([]; setpath(error(\"p\"); 1, error(\"z\")))",
        "def f: reduce range(20) as $i ([]; setpath([$i]; $i)); f",
        "foreach range(8) as $i ([]; . + [$i])",
        "[foreach range(8) as $i ([]; . + [$i])]",
        "foreach range(8) as $i ([]; . + [$i]; .[-1])",
        "foreach range(8) as $i ([]; . + [$i]; .)",
        "foreach range(8) as $i ([]; . + [$i]) | .[-1]",
        "[foreach range(6) as $i ([]; . + [$i]) | length]",
        "foreach range(4) as $i ([]; .[$i] |= $i)",
        "foreach range(4) as $i (null; . + [$i])",
        "foreach range(3) as $i (0; . + [$i])",
        "foreach range(4) as $i ([0]; . + [.[-1] + $i])",
        "limit(3; foreach range(8) as $i ([]; . + [$i]))",
        "first(foreach range(5) as $i ([]; . + [$i]))",
        "last(foreach range(5) as $i ([]; . + [$i]))",
        "nth(2; foreach range(5) as $i ([]; . + [$i]))",
        "foreach range(4) as $i ([]; . + [$i]; empty)",
        "def f: foreach range(5) as $i ([]; . + [$i]); f",
        "foreach range(3) as $i ([]; . + [if $i == 1 then error(\"z\") else $i end])",
        "[1,2,3] | foreach .[] as $x (.; . + [$x])",
        "foreach (1,2) as $x ((10,20); . + [$x])",
        "foreach range(3) as $i ([]; . + [$i, $i + 1])",
        "foreach range(3) as $i ([]; . + [])",
        "foreach range(3) as $i ([]; .[$i] |= $i; .[-1])",
        "limit(1; foreach (1,2) as $i (0; . + $i))",
        "limit(2; foreach (1,2) as $i (0; if $i == 2 then error(\"z\") else $i end))",
        "first(foreach (1,2) as $i (0; if $i == 2 then error(\"z\") else $i end))",
        "foreach range(3) as $i ([]; . + [$i], . + [10])",
        "reduce range(3) as $i ([]; setpath([$i]; $i), setpath([$i]; $i + 1))",
        "reduce range(1) as $i ([1]; setpath([\"a\"]; 1))",
        "reduce range(1) as $i (null; setpath([\"a\"]; 1))",
        "reduce range(1) as $i ({}; .[\"a\", \"b\"] |= $i)",
        "foreach range(4) as $i ({}; .[$i|tostring] |= $i)",
        "foreach range(4) as $i ([]; .[$i] |= empty)",
        "[foreach range(4) as $i ([]; . + [$i]; empty)]",
        "foreach range(3) as $i ([]; . + [$i]) | select(length > 1)",
        "def f: foreach range(4) as $i ([]; . + [$i]); f | length",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
    compare_exact(
        "foreach input",
        &[
            "-n",
            "-c",
            "--",
            "limit(1; foreach (1, input) as $i (0; . + $i))",
        ],
        b"10\n20\n",
    );
    compare_exact(
        "setpath input",
        &[
            "-c",
            "--",
            "reduce range(2) as $i (.; setpath([$i]; input))",
        ],
        b"[1,2,3]\n10\n20\n",
    );
}

#[test]
fn odd_indexes_match_jq() {
    require_jq();
    let programs = [
        "[1,2,3] | setpath([nan]; 9)",
        "null | setpath([nan]; 9)",
        "{} | setpath([nan]; 9)",
        "\"ab\" | setpath([nan]; 9)",
        "[1,2,3] | setpath([-nan]; 9)",
        "[1,2,3] | setpath([nan, 0]; 9)",
        "[1,2,3] | .[nan:1]",
        "[1,2,3] | .[1:nan]",
        "[1,2,3] | .[nan:nan]",
        "[1,2,3] | .[nan:]",
        "[1,2,3] | .[:nan]",
        "[1,2,3] | .[nan:1,2]",
        "[1,2,3] | .[-nan:2]",
        "[1,2,3] | .[0:-nan]",
        "\"abcd\" | .[nan:2]",
        "\"abcd\" | .[1:nan]",
        "null | .[nan:1]",
        "{} | .[nan:1]",
        "[1,2,3] | .[infinite:1]",
        "[1,2,3] | .[1:infinite]",
        "[1,2,3] | .[-infinite:2]",
        "[1,2,3] | .[0:-infinite]",
        "[1,2,3] | .[nan:infinite]",
        "[1,2,3] | path(.[nan:1])",
        "[1,2,3] | path(.[1:nan])",
        "null | path(.[nan:1])",
        "\"abc\" | path(.[1:nan])",
        "[1,2,3] | .[nan:1] = [9]",
        "[1,2,3] | .[1:nan] = [9]",
        "null | .[nan:1] = [9]",
        "\"abcd\" | .[nan:2] = \"Z\"",
        "[1,2,3] | del(.[nan:1])",
        "[1,2,3] | .[nan:1] |= map(.+10)",
        "[range(3)] | .[nan:1]",
        "[range(3)] | .[1:nan]",
        "[1,2,3] | .[nan:-1]",
        "[1,2,3] | .[-2:nan]",
        "[1,2,3] | .[nan:1.5]",
        "[1,2,3] | .[0.5:nan]",
        "[1,2,3] | setpath([-infinite]; 1)",
        "[1,2,3] | setpath([-1e300]; 1)",
        "[1,2,3] | .[-1e300] = 1",
        "[1,2,3] | .[-infinite] = 1",
        "null | .[-1e300] = 1",
        "null | setpath([-infinite]; 1)",
        "[1,2,3] | .[-1e300] |= 1",
        "[1,2,3] | .[-1e300] |= empty",
        "[1,2,3] | del(.[-1e300])",
        "[1,2,3] | .[-1e300] += 1",
        "[1,2,3] | .[-1e300]",
        "[1,2,3] | .[-1e300][0]",
        "[1,2,3] | .[-1e300][0] = 1",
        "[1,2,3] | .[-1e300].a = 1",
        "null | .[-1e300].a = 1",
        "[1,2,3] | del(.[-1e300].a)",
        "[1,2,3] | .[0, -1e300] = 9",
        "[1,2,3] | setpath([-infinite, 0]; 1)",
        "[1,2,3] | setpath([-1e309]; 1)",
        "[1,2,3] | .[-1e18] = 1",
        "[1,2,3] | .[-9223372036854775808] = 1",
        "\"ab\" | .[-1e300] = 1",
        "{} | .[-1e300] = 1",
        "1 | .[-1e300] = 1",
        "[1,2,3] | .[-4] = 1",
        "[1,2,3] | del(.[-4])",
        "[1,2,3] | .[1:2]",
        "[1,2,3] | path(.[1:2])",
        "[1,2,3] | path(.[-infinite])",
        "[1,2,3] | path(.[nan])",
        "[1,2,3] | .[nan] = 9",
        "[1,2,3] | getpath([nan])",
        "[1,2,3] | getpath([-infinite])",
        "[1,2,3] | .[1e2:2]",
        "reduce range(1) as $i ([1,2,3]; .[-1e300] = 1)",
        "reduce range(1) as $i ([1,2,3]; setpath([nan]; 9))",
        "reduce range(1) as $i ([1,2,3]; setpath([-infinite]; 1))",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
}

#[test]
fn text_functions_match_jq() {
    require_jq();
    let programs = [
        r#""AbCéÉßİ@[`{" | ascii_downcase"#,
        r#""AbCéÉßİ@[`{" | ascii_upcase"#,
        "\"\" | ascii_downcase",
        r#""A\u0000b" | ascii_downcase | explode"#,
        "null | ascii_downcase",
        "1 | ascii_upcase",
        "[] | explode",
        r#""a" | explode"#,
        r#""é" | explode"#,
        r#""😀" | explode"#,
        "\"\" | explode",
        r#""a\nb\u0000" | explode"#,
        "true | explode",
        "[65,66] | implode",
        "[] | implode",
        "[-1,0,1,2,3,1114111,1114112,55295,55296,57343,57344,1.1,1.9] | implode | explode",
        "123 | implode",
        r#"["a"] | implode"#,
        "[nan] | implode",
        "[true] | implode",
        "[null] | implode",
        "[{}] | implode",
        "[[]] | implode",
        r#"[1,"a"] | implode"#,
        "[infinite] | implode | explode",
        "[-infinite] | implode | explode",
        "[1.9,-1.2,-0.1,0.9] | implode | explode",
        "[1e20] | implode | explode",
        "[256] | implode",
        "[1114111.9] | implode | explode",
        "[-1.9] | implode | explode",
        r#""foo" | startswith("foo")"#,
        r#""foobar" | startswith("foo")"#,
        r#""barfoo" | endswith("foo")"#,
        "\"\" | startswith(\"\")",
        r#""a" | startswith("")"#,
        "\"\" | startswith(\"a\")",
        r#""aé" | endswith("é")"#,
        r#"null | startswith("a")"#,
        r#""a" | startswith(1)"#,
        r#""abc" | startswith("a","ab","x")"#,
        r#""abc" | startswith("a", error("z"))"#,
        r#""abc" | endswith(1)"#,
        r#"1 | endswith("a")"#,
        r#""abc" | index("b")"#,
        r#""abc" | index("x")"#,
        r#""abc" | index("")"#,
        r#""abc" | rindex("")"#,
        r#""abc" | indices("")"#,
        "\"\" | indices(\"\")",
        r#""aaa" | indices("aa")"#,
        r#""aaa" | rindex("aa")"#,
        r#""xababababax" | indices("aba")"#,
        r#""xababababax" | rindex("aba")"#,
        r#""éab" | index("a")"#,
        r#""éab" | indices("é")"#,
        r#""😀a" | index("a")"#,
        r#""aéb" | index("b")"#,
        r#""ababa" | indices("aba")"#,
        r#""a,b|c" | index(",","|")"#,
        r#""a,b|c" | indices(",","|")"#,
        r#"null | index("a")"#,
        "null | indices(1)",
        "null | indices(true)",
        r#""a" | index(1)"#,
        r#""a" | index(null)"#,
        r#""a" | index([])"#,
        r#"1 | index("a")"#,
        r#"1 | indices("a")"#,
        "[] | index(1)",
        "[0,1,1,2,3,4,1,5] | indices(1)",
        "[0,1,1,2,3,4,1,5] | rindex(1)",
        "[0,1,2,3,1,4,2,5,1,2,6,7] | indices([1,2])",
        "[0,1,2,3,1,4,2,5,1,2,6,7] | rindex([1,2])",
        "[1] | indices([1,2])",
        "[1,2,3] | index([])",
        "[1,2,3] | indices([])",
        "[] | index([])",
        "[1,null,1] | indices(null)",
        r#"[{"a":1},{"b":2},{"a":1}] | rindex({"a":1})"#,
        "[1,2,1,2,1] | indices([1,2,1])",
        "[true,false,true] | indices(true)",
        r#"[1,2] | index("1")"#,
        r#""abc" | index(["ab"])"#,
        "[1,2,3,1,2] | index(1,2,[1,2])",
        "{} | index(1)",
        r#"{"a":1,"b":2} | indices("a")"#,
        r#"{"a":1,"b":2} | index("a")"#,
        r#"{"a":1,"b":2} | rindex("a")"#,
        r#"{"a":"xy"} | indices("a")"#,
        r#"{"a":"xy"} | index("a")"#,
        r#"{"a":"xy"} | rindex("a")"#,
        r#"{"a":[10,20,30]} | index("a")"#,
        r#"{"a":[10,20,30]} | rindex("a")"#,
        r#"{"a":null} | index("a")"#,
        r#"{"a":null} | rindex("a")"#,
        r#"{"a":true} | index("a")"#,
        r#"{"a":{"x":1}} | rindex("a")"#,
        r#"{"a":1,"a":9} | indices("a")"#,
        "[[1,2],[3],[1,2]] | index([1,2])",
        "[[1,2],[3],[1,2]] | indices([1,2])",
        "[nan] | indices(nan)",
        "[1,1.0] | indices(1)",
        r#""abc" | index(empty)"#,
        r#"[1,2] | index(1, "x")"#,
        r#""ab" | index("a", 1)"#,
        "false | index(true)",
        "\"\" | utf8bytelength",
        r#""a" | utf8bytelength"#,
        r#""é" | utf8bytelength"#,
        r#""😀" | utf8bytelength"#,
        r#""asdfμ" | utf8bytelength"#,
        r#""\u0000" | utf8bytelength"#,
        "null | utf8bytelength",
        "0 | utf8bytelength",
        "1.5 | utf8bytelength",
        "true | utf8bytelength",
        "[] | utf8bytelength",
        "[1,2] | utf8bytelength",
        "{} | utf8bytelength",
        r#"{"a":1} | utf8bytelength"#,
        "[1,2,3,4,5,6,7,8,9] | utf8bytelength",
        r#"{"abcdefghij":1} | utf8bytelength"#,
        "nan | utf8bytelength",
        r#"["hello world!!"] | implode"#,
        r#"def f: explode; "ab" | f"#,
        r#"def f($x): index($x); "abc" | f("b")"#,
        r#""AbC" | ascii_upcase | ascii_downcase"#,
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
}

#[test]
fn expression_slices_match_jq() {
    require_jq();
    let programs = [
        "(2 as $i | [0,1,2,3,4] | .[$i:])",
        "(2 as $i | [0,1,2,3,4] | .[$i:] = [9])",
        "(2 as $i | [0,1,2,3,4] | del(.[$i:]))",
        "(2 as $i | [0,1,2,3,4] | .[$i:] |= map(.+10))",
        "(2 as $i | [0,1,2,3,4] | .[$i:] += [9])",
        "(2 as $i | [0,1,2,3,4] | .[$i:] |= empty)",
        "(2 as $i | [0,1,2,3,4] | path(.[$i:]))",
        "(2 as $i | [0,1,2,3,4] | .[$i : ])",
        "(null as $i | [0,1,2] | .[$i:])",
        "(null as $i | [0,1,2] | .[:$i])",
        "(null as $i | [0,1,2] | path(.[$i:]))",
        "[0,1,2,3,4] | .[:length-1]",
        "[0,1,2,3,4] | .[:length-1] = [9]",
        "[0,1,2,3,4] | del(.[:length-1])",
        "[0,1,2,3,4] | .[:length-1] |= map(.+1)",
        "[0,1,2,3,4] | path(.[:length-1])",
        "\"abcde\" | .[:length-1]",
        "[0,1,2,3,4] | .[length-3:]",
        "[0,1,2,3,4] | .[-length:]",
        "[0,1,2,3,4] | .[-(1):]",
        "[0,1,2,3,4] | .[1+1:]",
        "[0,1,2,3,4] | .[1+2*2:]",
        "[0,1,2,3,4] | ((-1) as $i | .[$i:])",
        "[0,1,2,3,4] | ((-2) as $i | .[:$i])",
        "[0,1,2,3,4] | (1.9 as $i | .[$i:3.2])",
        "[0,1,2,3,4] | .[1.9:3.2]",
        "\"abcde\" | .[index(\"b\"):]",
        "\"abcde\" | .[index(\"b\"):] = \"Z\"",
        "\"abcde\" | del(.[index(\"b\"):])",
        "\"abcde\" | path(.[index(\"b\"):])",
        "\"abcde\" | .[index(\"b\"):] |= .",
        "\"ébcde\" | .[2:]",
        "\"ébcde\" | .[index(\"b\"):]",
        "\"ébcde\" | ((-2) as $i | .[$i:])",
        "\"abcde\" | .[index(\"b\"):index(\"d\")]",
        "[0,1,2,3,4] | .[index(2):]",
        "[0,1,2,3,4] | .[index(2):] = [9]",
        "[0,1,2,3,4] | del(.[index(2):])",
        "[0,1,2,3,4] | path(.[index(2):])",
        "[0,1,2,3,4] | .[index(1),index(3):]",
        "[0,1,2,3,4] | .[index(2):index(4)]",
        "[\"a\",\"b\",\"c\",\"d\"] | .[index(\"b\"):]",
        "null | (1 as $i | .[$i:])",
        "null | (1 as $i | .[$i:] = [7,8])",
        "null | (1 as $i | del(.[$i:]))",
        "null | .[:length-1] = [7]",
        "null | .[index(\"b\"):]",
        "null | .[:1] = [1,2]",
        "null | del(.[:1])",
        "null | .[true:]",
        "null | path(.[true:])",
        "null | path(.[1.50:true])",
        "null | .[:true] = [1,2]",
        "null | .[1,true:] = [9]",
        "null | del(.[:true])",
        "null | .[:true] |= [9]",
        "[0,1,2,3,4] | .[1,2:]",
        "[0,1,2,3,4] | .[1,2:3,4]",
        "[0,1,2,3,4] | .[1,2:] = [9]",
        "[0,1,2,3,4] | .[1:2,4] = [9]",
        "[0,1,2,3,4] | .[1,3:2,4] = [9]",
        "[0,1,2,3,4] | .[1,2:] = [9,8]",
        "[0,1,2,3,4] | del(.[1,3:])",
        "[0,1,2,3,4] | .[1,2:] |= map(.+10)",
        "[0,1,2,3,4] | .[1,2:] |= empty",
        "[0,1,2,3,4] | path(.[1,2:3,4])",
        "[0,1,2,3,4] | .[1,error(\"z\"):]",
        "[0,1,2,3,4] | .[error(\"z\"),1:]",
        "[0,1,2,3,4] | .[1,error(\"z\"):] = [9]",
        "[0,1,2,3,4] | .[1,error(\"z\"):] = 9",
        "[0,1,2,3,4] | del(.[1,error(\"z\"):])",
        "[0,1,2,3,4] | path(.[1,error(\"z\"):])",
        "[0,1,2,3,4] | try .[1,error(\"z\"):]",
        "[0,1,2,3,4] | try (.[1,error(\"z\"):] = [9])",
        "[0,1,2,3,4] | (.[1,error(\"z\"):])?",
        "[0,1,2,3,4] | .[1,true:]",
        "[0,1,2,3,4] | .[1,true:]?",
        "[0,1,2,3,4] | .[:true]?",
        "[0,1,2,3,4] | .[1,true:] = [9]",
        "[0,1,2,3,4] | .[1,true:] = 9",
        "[0,1,2,3,4] | .[true:] = 9",
        "[0,1,2,3,4] | .[1,2:true] = 9",
        "[0,1,2,3,4] | .[1,2:error(\"z\")] = 9",
        "[0,1,2,3,4] | .[1:2,error(\"z\")] = 9",
        "[0,1,2,3,4] | .[1,true:] += 1",
        "[0,1,2,3,4] | .[1,true:] += [9]",
        "[0,1,2,3,4] | .[1,error(\"z\"):] |= map(.+1)",
        "[0,1,2,3,4] | .[1,error(\"z\"):] |= error(\"y\")",
        "\"ab\" | .[:true]",
        "\"ab\" | .[:true] = \"Z\"",
        "\"ab\" | .[1,true:] = \"Z\"",
        "\"ab\" | del(.[:true])",
        "\"ab\" | del(.[1,true:])",
        "\"ab\" | .[1,error(\"z\"):] = \"Z\"",
        "\"ab\" | .[1,error(\"z\"):] |= empty",
        "\"abcd\" | .[1,error(\"z\"):] |= .",
        "\"abcd\" | .[1,true:]",
        "{} | .[1:]",
        "{} | .[:true]",
        "{} | .[error(\"z\"):]",
        "{} | .[1,error(\"z\"):]",
        "{} | .[1:]?",
        "{} | (1 as $i | .[$i:] = [1])",
        "1 | .[1:]",
        "1 | .[:true]",
        "1 | .[error(\"z\"):]",
        "true | .[1:]",
        "path(1 | .[1:])",
        "path(1 | .[1+1:])",
        "path(1 | .[error(\"z\"):])",
        "path(1 | .[true:])",
        "1 | path(.[1:])",
        "1 | path(.[error(\"z\"):])",
        "{} | path(.[1:])",
        "path(null | .[true:])",
        "[0,1,2,3,4] | path(.[1:])",
        "[0,1,2,3,4] | path(.[1.0:])",
        "[0,1,2,3,4] | path(.[01:])",
        "[0,1,2,3,4] | path(.[1.50:2])",
        "[0,1,2,3,4] | path(.[-1:])",
        "[0,1,2,3,4] | path(.[-1.50:])",
        "[0,1,2,3,4] | path(.[-0:])",
        "[0,1,2,3,4] | path(.[-0.0:])",
        "[0,1,2,3,4] | path(.[nan:1])",
        "[0,1,2,3,4] | path(.[-nan:1])",
        "[0,1,2,3,4] | path(.[infinite:])",
        "[0,1,2,3,4] | path(.[:-infinite])",
        "[0,1,2,3,4] | path(.[1e2:])",
        "[0,1,2,3,4] | path(.[1+0.5:])",
        "[0,1,2,3,4] | .[-2:]",
        "[0,1,2,3,4] | .[:-2]",
        "[1,2,3] | .[nan:1]",
        "[1,2,3] | .[1:nan]",
        "[1,2,3] | .[nan:1,2]",
        "[1,2,3] | .[nan:1,2] = [9]",
        "[1,2,3] | path(.[nan:1,2])",
        "[0,1,2,3,4] | .[empty:]",
        "[0,1,2,3,4] | .[1:empty]",
        "[0,1,2,3,4] | .[empty:error(\"z\")]",
        "[0,1,2,3,4] | .[1,empty:3]",
        "[0,1,2,3,4] | .[empty:] = [9]",
        "[0,1,2,3,4] | del(.[empty:])",
        "[0,1,2,3,4] | .[(1|2):]",
        "[0,1,2,3,4] | .[1|2:]",
        "(1 as $a | 4 as $b | [0,1,2,3,4] | .[$a:$b])",
        "(1 as $a | 4 as $b | [0,1,2,3,4] | .[$a:$b] = [9])",
        "(1 as $a | 4 as $b | [0,1,2,3,4] | del(.[$a:$b]))",
        "def f($i): .[$i:]; [0,1,2,3,4] | f(2)",
        "def f($i): .[$i:] = [9]; [0,1,2,3,4] | f(2)",
        "def f($i): del(.[$i:]); [0,1,2,3,4] | f(2)",
        "def f: .[index(\"b\"):]; \"abcde\" | f",
        "def f: .[:length-1] = [7]; [0,1,2,3,4] | f",
        "[[0,1,2,3],[4,5,6]] | .[0][:length-1]",
        "[[0,1,2,3],[4,5,6]] | .[0][:length-1] = [9]",
        "[[0,1,2,3],[4,5,6]] | .[0] | .[:length-1]",
        "[[0,1,2,3],[4,5,6]] | del(.[0][:length-1])",
        "[[0,1,2,3],[4,5,6]] | path(.[0][:length-1])",
        "{\"a\":[0,1,2,3,4]} | .a[:length-1]",
        "{\"a\":[0,1,2,3,4]} | .a[:length-1] = [9]",
        "{\"a\":[0,1,2,3,4]} | del(.a[:length-1])",
        "{\"a\":[0,1,2,3,4]} | path(.a[:length-1])",
        "{\"a\":[0,1,2,3,4]} | (.a)[:length-1]",
        "{\"a\":[0,1,2,3,4]} | (.a)|.[:length-1]",
        "[\"ab\",\"x\"] | .[0][index(\"b\"):]",
        "[\"ab\",\"x\"] | .[0] | .[index(\"b\"):]",
        "[0,1,2,3,4] | .[:3][:length-1]",
        "[0,1,2,3,4] | .[:3] | .[:length-1]",
        "[0,1,2,3,4] | path(.[:3][:length-1])",
        "([0,1,2,3],[10,11,12])[1:2]",
        "([0,1,2,3],[10,11,12])[1,2:]",
        "([0,1,2,3,4], error(\"a\"))[1,2:]",
        "([0,1,2,3,4], error(\"a\"))[1:error(\"b\")]",
        "([0,1,2],[10,11], error(\"a\"))[error(\"b\"):]",
        "[0,1,2,3,4] | (.[1:4], .[2]) = [9]",
        "[0,1,2,3,4] | (.[1:4], .[3]) = [9]",
        "[0,1,2,3,4] | del(.[1:3], .[3])",
        "[0,1,2,3,4] | del(.[1:3], .[1])",
        "[0,1,2,3,4] | (.[1:4], .[2]) |= if type == \"array\" then [9] else empty end",
        "[] | (0 as $i | .[$i:] = [1,2])",
        "[0,1,2,3,4] | .[range(3):]",
        "(-1e300 as $i | [1,2,3] | .[$i:])",
        "(nan as $i | [1,2,3] | .[$i:2])",
        "(nan as $i | [1,2,3] | .[0:$i])",
        "(nan as $i | [1,2,3] | path(.[$i:2]))",
        "(infinite as $i | [1,2,3] | .[1:$i])",
        "[0,1,2,3,4] | (2 as $i | .[$i:] = 9)",
        "reduce range(3) as $i ([0,1,2,3,4]; .[$i:] = [$i])",
        "[0,1,2,3,4] | .[{start:1, end:3}] = [9]",
        "[0,1,2,3,4] | del(.[{start:1, end:3}])",
        "[0,1,2,3,4] | path(.[{start:1, end:3}])",
        "[0,1,2,3,4] | getpath([{start:1, end:3}])",
        "(2 as $i | [0,1,2,3,4] | getpath([{start:$i, end:4}]))",
        "(2 as $i | [0,1,2,3,4] | setpath([{start:$i, end:4}]; [9]))",
        "\"abc\" | .[index(1):]",
        "[0,1,2] | .[index(\"a\"):]",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
}

#[test]
fn formats_match_jq() {
    require_jq();
    let programs = [
        r#""hi" | @text"#,
        r#""hi" | @json"#,
        r#""hi" | @text, @json"#,
        "1 | @text",
        "1 | @json",
        "1.50 | @text",
        "1.50 | @json",
        "null | @text",
        "null | @json",
        "true | @text",
        "false | @json",
        r#"[1, "a"] | @text"#,
        r#"[1, "a"] | @json"#,
        r#"{"z":1,"a":2} | @json"#,
        r#"{"z":1,"a":2} | @text"#,
        r#"{"z":1,"a":2} | @html"#,
        "nan | @text",
        "nan | @json",
        "-nan | @text",
        "infinite | @json",
        "-infinite | @text",
        r#""<>&'\"\t" | @html"#,
        r#""plain" | @html"#,
        r#""" | @html"#,
        r#""a<b&c>d'e\"f" | @html"#,
        "1 | @html",
        "null | @html",
        "true | @html",
        "[] | @html",
        "{} | @html",
        r#""é<>" | @html"#,
        r#""\u0000<" | @html"#,
        r#""\u0001" | @html"#,
        r#""\u0008" | @html"#,
        r#""\u000b" | @html"#,
        r#""\u000c" | @html"#,
        r#""&lt;" | @html"#,
        r#"["<"] | @html"#,
        r#""a\u0000b" | @html"#,
        r#""hello" | @uri"#,
        r#""a b" | @uri"#,
        r#""!()<>&'\"\t" | @uri"#,
        r#""μ" | @uri"#,
        r#""😀" | @uri"#,
        r#""é" | @uri"#,
        r#""ÿ" | @uri"#,
        r#""" | @uri"#,
        r#""-._~AZ09az" | @uri"#,
        r#""a/b?c=d&e#f" | @uri"#,
        r#""\u0000" | @uri"#,
        r#""\u0001" | @uri"#,
        r#""a+b" | @uri"#,
        r#"" ~" | @uri"#,
        "1 | @uri",
        "null | @uri",
        "[] | @uri",
        "true | @uri",
        r#"["<"] | @uri"#,
        r#""hello" | @sh"#,
        r#""a b" | @sh"#,
        r#""a'b" | @sh"#,
        r#""" | @sh"#,
        r#""a\nb" | @sh"#,
        r#""a\tb" | @sh"#,
        r#""$" | @sh"#,
        r#""*" | @sh"#,
        r#""'" | @sh"#,
        r#""''" | @sh"#,
        r#""a b'c$d" | @sh"#,
        r#""\n" | @sh"#,
        r#""\u0000" | @sh"#,
        r#""\u0001" | @sh"#,
        r#""\u0008" | @sh"#,
        r#""\u000c" | @sh"#,
        r#""'\u0000" | @sh"#,
        r#""\u0000'" | @sh"#,
        r#""é" | @sh"#,
        "1 | @sh",
        "1.50 | @sh",
        "-0 | @sh",
        "nan | @sh",
        "-nan | @sh",
        "infinite | @sh",
        "null | @sh",
        "true | @sh",
        "false | @sh",
        r#"["a","b c","d'e"] | @sh"#,
        "[] | @sh",
        r#"[""] | @sh"#,
        r#"["a b", "c"] | @sh"#,
        r#"["", " ", "a"] | @sh"#,
        r#"["a", 1] | @sh"#,
        "[null] | @sh",
        r#"[null, ""] | @sh"#,
        "[nan] | @sh",
        r#"[nan, "a"] | @sh"#,
        "[-0] | @sh",
        "[true, false] | @sh",
        r#"["a", 1, true, null, false] | @sh"#,
        "[1,2,3,4,5,6,7,8,9,10] | @sh",
        r#"["a b"] | @sh"#,
        "{} | @sh",
        r#"{"abcdefghijklmnop":1} | @sh"#,
        r#"{"abcdefghi":1,"b":2} | @sh"#,
        "[[]] | @sh",
        r#"["a", ["b"]] | @sh"#,
        r#"["a", {}, "c"] | @sh"#,
        r#"["ok", {}] | @sh"#,
        "[1, {}] | @sh",
        "1 | @sh?",
        "{} | @sh?",
        r#"[1, "a,b", true, false, null] | @csv"#,
        r#"[1, "a,b", true, false, null] | @tsv"#,
        "[] | @csv",
        "[] | @tsv",
        "[null] | @csv",
        "[null,null] | @csv",
        "[null,null] | @tsv",
        r#"[""] | @csv"#,
        r#"[""] | @tsv"#,
        r#"["a\"b"] | @csv"#,
        r#"["a\"b"] | @tsv"#,
        r#"["a\nb"] | @csv"#,
        r#"["a\nb"] | @tsv"#,
        r#"["a\tb"] | @tsv"#,
        r#"["a\rb"] | @tsv"#,
        r#"["a\rb"] | @csv"#,
        r#"["a\\b"] | @tsv"#,
        r#"["\\"] | @csv"#,
        r#"["a,b"] | @tsv"#,
        r#"["a\n\t\r\\b"] | @tsv"#,
        r#"["\\\n"] | @tsv"#,
        r#"["\n\\"] | @tsv"#,
        "[1.50, 1.0, 1] | @csv",
        "[01, 1e2, 1.50] | @csv",
        "[1e2] | @tsv",
        "[nan] | @csv",
        "[-nan] | @csv",
        r#"[nan, "null", null] | @csv"#,
        r#"[nan, "null", null] | @tsv"#,
        "[1.50, nan, -nan, true] | @csv",
        "[infinite] | @csv",
        "[-infinite] | @tsv",
        "[-0, 0] | @csv",
        "[-0, 0] | @tsv",
        "[9007199254740993] | @csv",
        r#"["é", "😀"] | @csv"#,
        r#"[1, "a\"b,c\n"] | @csv"#,
        "[true, false, null, 0, -1] | @csv",
        "[true, false, null, 0, -1] | @tsv",
        r#"["\u000c"] | @tsv"#,
        r#"["\u000c"] | @csv"#,
        r#"["a\u0000b"] | @csv"#,
        r#"["\u0000"] | @csv"#,
        r#"["a\u0001b"] | @csv"#,
        r#"["a\u0008b"] | @csv"#,
        r#"["a\u000bb"] | @csv"#,
        r#"["a\u0008b"] | @tsv"#,
        r#"["\u000b"] | @tsv"#,
        r#"["\"\u0000"] | @csv"#,
        "[[1,2],[3]] | @csv",
        r#"[{"a":1}] | @csv"#,
        "[1, {}] | @csv",
        "[1, [2]] | @tsv",
        "[1, {}, 2] | @csv",
        r#"["ok", {}] | @tsv"#,
        "[{}] | @csv",
        "[[[]]] | @tsv",
        r#""hi" | @csv"#,
        "1 | @csv",
        "null | @csv",
        "true | @tsv",
        "{} | @tsv",
        r#""abcdefghijklmnopqr" | @csv"#,
        "[[1,2,3,4,5,6,7,8]] | @csv",
        "1 | @csv?",
        r#""!" | @base64d?"#,
        r#""" | @base64"#,
        r#""a" | @base64"#,
        r#""ab" | @base64"#,
        r#""abc" | @base64"#,
        r#""foóbar\n" | @base64"#,
        r#""\u0000" | @base64"#,
        r#""😀" | @base64"#,
        r#""é" | @base64"#,
        "1 | @base64",
        "null | @base64",
        "[] | @base64",
        "true | @base64",
        "{} | @base64",
        r#"["<"] | @base64"#,
        "1 | @base64 | @base64d",
        "null | @base64 | @base64d",
        "[] | @base64 | @base64d",
        "{} | @base64 | @base64d",
        "true | @base64 | @base64d",
        r#""\u0000" | @base64 | @base64d | explode"#,
        r#""😀" | @base64 | @base64d"#,
        r#""foóbar\n" | @base64 | @base64d"#,
        r#""" | @base64d"#,
        r#""YQ==" | @base64d"#,
        r#""YWI=" | @base64d"#,
        r#""YWJj" | @base64d"#,
        r#""Zm/Ds2Jhcgo=" | @base64d"#,
        r#""YQ" | @base64d"#,
        r#""YQ=" | @base64d"#,
        r#""YWI" | @base64d"#,
        r#""YWJjZA" | @base64d"#,
        r#""YWJjZA=" | @base64d"#,
        r#""YWJjYQ" | @base64d"#,
        r#""YWJjYQ=" | @base64d"#,
        r#""YWJjYQ==" | @base64d"#,
        r#""YWJj====" | @base64d"#,
        r#""YQ==!!!!" | @base64d"#,
        r#""====YQ==" | @base64d"#,
        r#""YQ==YQ==" | @base64d"#,
        r#""YQ=\n=" | @base64d"#,
        r#""YW=Jj" | @base64d"#,
        r#""====" | @base64d"#,
        r#""=" | @base64d"#,
        r#""==" | @base64d"#,
        r#""YQ== " | @base64d"#,
        r#""YQ==\t" | @base64d"#,
        r#""YQ==\r\n" | @base64d"#,
        r#""YQ== \t" | @base64d"#,
        r#""YQ==\n" | @base64d"#,
        r#""Y" | @base64d"#,
        r#""Y===" | @base64d"#,
        r#""A" | @base64d"#,
        r#""+" | @base64d"#,
        r#""/" | @base64d"#,
        r#""ABCDE" | @base64d"#,
        r#""false" | @base64d"#,
        r#""AB" | @base64d | explode"#,
        r#""YY" | @base64d | explode"#,
        r#""YYY" | @base64d | explode"#,
        r#""YYYY" | @base64d | explode"#,
        r#""ABC" | @base64d | explode"#,
        r#""ABCD" | @base64d | explode"#,
        r#""!!!!" | @base64d"#,
        r#""*" | @base64d"#,
        r#""-" | @base64d"#,
        r#""_" | @base64d"#,
        r#""AB " | @base64d"#,
        r#""ABC " | @base64d"#,
        r#""ABCD " | @base64d"#,
        r#""AB\n" | @base64d"#,
        r#""AB\t" | @base64d"#,
        r#""ABCD\n" | @base64d"#,
        r#""\tAB" | @base64d"#,
        r#""AB\nCD" | @base64d"#,
        r#""YWJj " | @base64d"#,
        r#"" YQ==" | @base64d"#,
        r#""   " | @base64d"#,
        r#""\n" | @base64d"#,
        r#""\nYQ==" | @base64d"#,
        r#""Y Q==" | @base64d"#,
        r#""YQ == " | @base64d"#,
        "1 | @base64d",
        "1.50 | @base64d",
        "null | @base64d",
        "null | @base64d | length",
        "null | @base64d | explode",
        "null | @base64d | utf8bytelength",
        "null | @base64d | @base64",
        "[] | @base64d",
        "true | @base64d",
        "nan | @base64d",
        "{} | @base64d",
        r#""abcdefghijklmnop" | @base64d"#,
        r#""!!!!!!!!!!!!!!" | @base64d"#,
        r#""gA==" | @base64d | explode"#,
        r#""/w==" | @base64d | explode"#,
        r#""wA==" | @base64d | explode"#,
        r#""wg==" | @base64d | explode"#,
        r#""woA=" | @base64d | [length, explode, @base64]"#,
        r#""wiA=" | @base64d | explode"#,
        r#""4ICA" | @base64d | explode"#,
        r#""7aCA" | @base64d | explode"#,
        r#""9YCAgA==" | @base64d | explode"#,
        r#""8ICAgA==" | @base64d | explode"#,
        r#""AA==" | @base64d | explode"#,
        r#""woBB" | @base64d | explode"#,
        r#""/0E=" | @base64d | explode"#,
        r#""gEE=" | @base64d | explode"#,
        r#""6WU=" | @base64d | explode"#,
        r#""null" | @base64d | [length, explode, @base64]"#,
        r#""QQ==" | @base64d | explode"#,
        r#""wv8=" | @base64d | explode"#,
        r#""8JCAgA==" | @base64d | explode"#,
        r#""7b+/" | @base64d | explode"#,
        r#""9I+/vw==" | @base64d | explode"#,
        r#""9JCAgA==" | @base64d | explode"#,
        r#""gIA=" | @base64d | explode"#,
        r#""wK8=" | @base64d | explode"#,
        r#""4IA=" | @base64d | explode"#,
        r#""8IA=" | @base64d | explode"#,
        r#""/g==" | @base64d | explode"#,
        r#""wb8=" | @base64d | explode"#,
        r#""378=" | @base64d | explode"#,
        r#""4KCA" | @base64d | explode"#,
        r#""<script>hax</script>" | @html "<b>\(.)</b>""#,
        r#""<script>hax</script>" | "<b>\(.)</b>" | @html"#,
        r#""a b" | @sh "cmd \(.)""#,
        r#""a b" | "cmd \(.)" | @sh"#,
        r#"["a b","c"] | @sh "cmd \(.)""#,
        r#"["a b","c"] | "cmd \(.)" | @sh"#,
        r#"["a b","c"] | @sh"#,
        r#"["a b", "c"] | @sh "pre \(.) post""#,
        r#"[1, "a,b"] | @csv "row: \(.)""#,
        r#"[1, "a,b"] | "row: \(.)" | @csv"#,
        r#"[1, "a,b"] | @csv"#,
        r#"1 | @json "n=\(.)""#,
        r#"1 | "n=\(.)" | @json"#,
        r#""hi" | @text "x\(.)y""#,
        r#""hi" | @json "x\(.)y""#,
        r#""hi" | @json "[\(.)]""#,
        r#"1 | @json "[\(.)]""#,
        r#""<>" | @html "a\(.)b\(.)c""#,
        r#""<>&" | @html "\(.)\(.)""#,
        "(1,2) | @html \"n\\(.)\"",
        r#"[1,2] | @csv "\(.[])""#,
        r#""a<b" | @html "\(.)\("x>y")""#,
        r#"null | @html "\(.)""#,
        r#"null | @text "v=\(.)""#,
        r#"1 | @csv "\(.)""#,
        r#"@html """#,
        r#"@csv """#,
        r#"@csv "ab""#,
        r#"@csv"ab""#,
        r#"@foo "x""#,
        r#""<>" | @html "\(.)""#,
        r#"@html "\(error("z"))""#,
        r#""<>" | @html "\(., error("z"))""#,
        r#"@text "\(1,2)\(3,4)""#,
        r#"@text "\(1,2)\(3,4)\(5,6)""#,
        r#"@text "A\(1,2)B\(3,4)C""#,
        r#"@text "x\(1,2)y""#,
        r#"@html "\(1,2)-\("a","b")""#,
        r#"@html "\(1,2)\(empty)""#,
        r#"@html "\(empty)\("a","b")""#,
        r#"@foo "\(empty)""#,
        r#"@foo "\(1)""#,
        r#"@foo "a\(empty)b""#,
        r#"@foo "a\(1)b""#,
        r#"@foo "x\(empty)\(1)""#,
        r#"@foo "\(1)\(empty)""#,
        r#"@foo "\(empty)\(1)""#,
        r#"@foo "\(empty)\(empty)""#,
        r#"@foo "\(empty)\("a")""#,
        r#"@foo "\("a")\(empty)""#,
        r#"@csv "\([1], 2)""#,
        r#"@csv "\(1, [2])""#,
        r#"@csv "pre\([1], 2)post""#,
        r#"@csv "\([1], 2)\([3])""#,
        r#"@csv "\([1])\(2)""#,
        r#"@csv "\([1], 2, error("z"))""#,
        r#"@csv "\([1], error("z"))""#,
        r#"@html "\([1], error("z"))""#,
        r#"@text "\(1, error("z"))""#,
        r#"@text "A\(1, error("z"))B""#,
        r#"@html "\(1)\(error("z"))""#,
        r#"@foo "\(error("z"))\(1)""#,
        r#"@text "\(error("z"))\(1)""#,
        r#"@html "\(1,error("z"))\(3)""#,
        r#"@text "\(1, error("z"))\(3,4)""#,
        r#"@text "\(1,2)\(3, error("z"))""#,
        r#"@csv "\([1], error("z"))\([3])""#,
        r#"@foo "\(error("z"))\(1)""#,
        r#"@foo "\(1)\(error("z"))""#,
        "(1, error(\"z\")) | @text \"n\\(.)\"",
        r#""ab" | @base64 "xx\(.)yy""#,
        r#""YQ==" | @base64d "zz\(.)""#,
        r#"1 | @uri "n\(.)""#,
        r#"{} | @sh "k \(.)""#,
        r#"@html "<\(.)>""#,
        r#""&" | @html "&\(.)""#,
        r#"@base64 "\(.)""#,
        "nan | @json \"v=\\(.)\"",
        r#"@csv"#,
        r#"@text"#,
        r#"@json"#,
        r#"@Html"#,
        r#"@HTML"#,
        r#"@1"#,
        r#"@_"#,
        r#"@_foo"#,
        r#"@csv1"#,
        r#"@a"#,
        r#"@A"#,
        r#"@a0"#,
        r#"@a_"#,
        r#"@base64d"#,
        "def csv: 1; @csv",
        "def f: @foo; 1",
        "def f: @foo; f",
        "def f: @foo; empty",
        r#"def html: 1; "<" | @html"#,
        r#"def csv: 1; [1] | @csv"#,
        r#"def f: @csv "\(.)"; [1,2] | f"#,
        "empty | @foo",
        "if false then @foo else 1 end",
        "1 | @csv?",
        "path(@text)",
        "path(@html)",
        r#""<>" | path(@html "\(.)")"#,
        "@csv.a",
        "[1,2] | @csv.a",
        r#"@html "x".a"#,
        "[1,2] | @csv[]",
        "@text?",
        "try (1 | @csv) catch .",
        "try ([1, [2]] | @csv) catch .",
        "try ({} | @sh) catch .",
        r#"try ("!" | @base64d) catch ."#,
        r#"try (1 | @csv "\(.)") catch ."#,
        "([1], \"no\") | @csv",
        "(1, \"a<\") | @html",
        "1 | @text, @json",
        r#""a\u0000b" | @json"#,
        r#""a\u0000b" | @text"#,
        r#"@csv # hi
"ab""#,
        "\"<>\" | @html # c\n\"\\(.)\"",
        r#"[1,2] | map(@text)"#,
        r#"[1,"a,b"] | @csv | length"#,
        r#""hi" | @text | length"#,
        "1 | @base64 | @base64d | @json",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
    let with_input = [
        (r#"@text "\(input)\(1,2)""#, &b"10\n20\n30\n"[..]),
        (r#"@text "\(input)\(1)""#, &b"10\n20\n"[..]),
        (r#"@text "\(input)\(1,2,3)""#, &b"10\n20\n30\n40\n"[..]),
        (r#"@text "x\(input)y\(1,2)""#, &b"10\n20\n30\n"[..]),
        (r#"@text "\(1,2)x\(input)""#, &b"10\n20\n"[..]),
        (
            r#"@text "\(input,input)x\(1,2)""#,
            &b"1\n2\n3\n4\n5\n6\n"[..],
        ),
        (r#"@text "\(1,2)x\(input,input)""#, &b"1\n2\n3\n4\n"[..]),
        (r#"@text "\(input)\(input)""#, &b"10\n20\n30\n"[..]),
        (r#"@text "\(input)\(input,input)""#, &b"1\n2\n3\n4\n"[..]),
        (r#"@csv "\(input)""#, &b"[1]\n2\n[3]\n"[..]),
        (r#"@html "pre\(input)post""#, &b"\"<\"\n\"&\"\n"[..]),
    ];
    for (program, input) in with_input {
        compare_exact(program, &["-n", "-c", "--", program], input);
    }
    // The jq.test programs read the value as `.`, not through `input`.
    let stdin_cases = [
        (
            r#"@text,@json,([1,.]|@csv,@tsv),@html,@uri,@sh,(@base64|.,@base64d)"#,
            "\"!()<>&'\\\"\\t\"\n",
        ),
        ("@base64", "\"foóbar\\n\"\n"),
        ("@base64d", "\"Zm/Ds2Jhcgo=\"\n"),
        ("@uri", "\"\\u03bc\"\n"),
        (r#"@html "<b>\(.)</b>""#, "\"<script>hax</script>\"\n"),
    ];
    for (program, input) in stdin_cases {
        compare_exact(program, &["-c", "--", program], input.as_bytes());
    }
}

#[test]
fn deep_values_match_jq() {
    require_jq();
    let programs = [
        "reduce range(0) as $i (0; [.])",
        "reduce range(1) as $i (0; [.])",
        "reduce range(4) as $i (0; [.])",
        "reduce range(4) as $i (0; [.]) | tojson",
        "reduce range(4) as $i (0; [.]) | tostring",
        "reduce range(10) as $i (0; [., $i])",
        "reduce range(8) as $i ({}; {a:.})",
        "reduce range(3) as $i (0; {b:1, a:.})",
        "reduce range(3) as $i (0; [., 9])",
        "reduce range(3) as $i (0; [9, .])",
        "def f: reduce range(6) as $i (0; [.]); f",
        "reduce range(256) as $i (0; [.]) | tojson",
        "reduce range(256) as $i (0; [.]) | tostring",
        "reduce range(257) as $i (0; [.]) | tojson",
        "reduce range(257) as $i (0; [.]) | tostring",
        "reduce range(257) as $i (0; [.]) | @json",
        "reduce range(257) as $i (0; [.]) | @text",
        "reduce range(300) as $i (0; [.]) | tojson",
        "reduce range(300) as $i (0; [.]) | tostring",
        "reduce range(512) as $i (0; [.]) | (tojson == tostring)",
        "reduce range(257) as $i (null; [.]) | tojson",
        "reduce range(257) as $i (true; [.]) | tojson",
        "reduce range(257) as $i (\"x\"; [.]) | tojson",
        "reduce range(257) as $i ([1,2]; [.]) | tojson",
        "reduce range(257) as $i ({b:1}; [.]) | tojson",
        "reduce range(256) as $i ({}; {a:.}) | tojson",
        "reduce range(257) as $i ({}; {a:.}) | tojson",
        "reduce range(257) as $i (0; {a:.}) | tojson",
        "reduce range(257) as $i (0; {a:., b:1}) | tojson",
        "reduce range(257) as $i (0; {b:1, a:.}) | tojson",
        "reduce range(257) as $i (0; [., 9]) | tojson",
        "reduce range(257) as $i (0; [9, .]) | tojson",
        "[range(5)] | reduce range(252) as $i (.; [.]) | tojson",
        "[range(5)] | reduce range(253) as $i (.; [.]) | tojson",
        "reduce range(256) as $i (0; [., $i]) | tojson",
        "reduce range(257) as $i (0; [., $i]) | tojson",
        "reduce range(255) as $i (0; [.]) == reduce range(255) as $i (0; [.])",
        "reduce range(256) as $i (0; [.]) == reduce range(256) as $i (0; [.])",
        "reduce range(257) as $i (0; [.]) == reduce range(257) as $i (0; [.])",
        "reduce range(300) as $i (0; [.]) == reduce range(299) as $i (0; [.])",
        "reduce range(300) as $i (0; [.]) == reduce range(300) as $i (1; [.])",
        "reduce range(300) as $i (0; [.]) != reduce range(300) as $i (1; [.])",
        "reduce range(257) as $i (0; [.]) < reduce range(257) as $i (1; [.])",
        "reduce range(257) as $i (1; [.]) < reduce range(257) as $i (0; [.])",
        "reduce range(257) as $i (0; [.]) > reduce range(256) as $i (0; [.])",
        "reduce range(399) as $i (0; [.]) < reduce range(400) as $i (0; [.])",
        "reduce range(400) as $i (0; [.]) < reduce range(399) as $i (0; [.])",
        "reduce range(400) as $i (0; {z:.}) < reduce range(400) as $i (1; {z:.})",
        "reduce range(400) as $i (0; {b:1, a:.}) == reduce range(400) as $i (0; {a:., b:1})",
        "reduce range(300) as $i ({}; {a:.}) == reduce range(300) as $i ({}; {a:.})",
        "reduce range(300) as $i ({}; {a:.}) == reduce range(299) as $i ({}; {a:.})",
        "reduce range(400) as $i ([1,2,3]; [.]) == reduce range(400) as $i ([1,2,3]; [.])",
        "reduce range(400) as $i ([1,2,3]; [.]) == reduce range(400) as $i ([1,9,3]; [.])",
        "reduce range(400) as $i ([1,2]; [.]) < reduce range(400) as $i ([1,2,3]; [.])",
        "[[range(3)]] == [[0,1,2]]",
        "[1,2,3] == [1,2]",
        "{\"b\":1,\"a\":2} == {\"a\":2,\"b\":1}",
        "reduce range(2000) as $i (0; [.]) == reduce range(2000) as $i (0; [.])",
        "reduce range(2000) as $i (0; [.]) == reduce range(2000) as $i (1; [.])",
        "reduce range(2000) as $i (0; [.]) == reduce range(1999) as $i (0; [.])",
        "reduce range(2000) as $i (0; [.]) | tojson | length",
        "reduce range(2000) as $i (0; [.]) | tostring | length",
        "reduce range(400) as $i ({}; {a:.}) == reduce range(400) as $i ({}; {a:.})",
        "reduce range(400) as $i ({}; {a:.}) == reduce range(399) as $i ({}; {a:.})",
        "reduce range(400) as $i ({}; {a:.}) | tojson | length",
        "reduce range(0) as $i ({}; {a:.})",
        "reduce range(0) as $i (0; [., 1])",
        "reduce range(0) as $i (0; [[.]])",
        "reduce range(4) as $i (0; [., 1])",
        "reduce range(4) as $i (0; [[.]])",
        "reduce range(4) as $i (0; [1, .])",
        "reduce range(3) as $i (0; [1, ., 2])",
        "reduce range(3) as $i (0; [[.], 1])",
        "reduce range(3) as $i (0; [1, [.]])",
        "reduce range(3) as $i (0; {a:[., 1]})",
        "reduce range(3) as $i (0; [{a:.}])",
        "reduce range(3) as $i ({x:1}; {a:.})",
        "reduce range(3) as $i (0; {b:., a:1})",
        "reduce range(3) as $i (0; {a:., a:1})",
        "reduce range(3) as $i (0; {a:1, a:.})",
        "reduce range(3) as $i (0; {a:., b:2, a:3})",
        "reduce range(3) as $i (0; {a:[1, .]})",
        "reduce range(3) as $i (0; {a:{b:.}})",
        "reduce range(4) as $i (0; [., \"x\", true, null])",
        "reduce range(3) as $i (0; [., {\"k\":1}])",
        "reduce range(3) as $i (0, {}; [., 1])",
        "reduce range(3) as $i (0; {(\"a\"):.})",
        "reduce range(3) as $i (0; {(\"b\" + \"b\"):.})",
        "def f: reduce range(6) as $i ({}; {a:.}); f",
        "def f: reduce range(6) as $i (0; [., 1]); f",
        "def f: reduce range(6) as $i (0; [[.]]); f",
        "def g: {a:.}; reduce range(4) as $i ({}; g)",
        "reduce range(6) as $i (0; [., .])",
        "reduce range(4) as $i (0; {a:., b:.})",
        "reduce range(4) as $i (0; [., [.]])",
        "reduce range(4) as $i (0; {a:., b:[.]})",
        "reduce range(2) as $i (0; {a: .foo})",
        "reduce range(3) as $i (7; {a})",
        "reduce range(4) as $i (0; [., $i])",
        "reduce range(4) as $i (0; . | {a:.})",
        "foreach range(4) as $i (0; [., 1])",
        "foreach range(4) as $i ({}; {a:.})",
        "foreach range(3) as $i (0; [[.]])",
        "[foreach range(3) as $i (0; {a:.})]",
        "reduce range(128) as $i (0; [[.]]) | tojson",
        "reduce range(129) as $i (0; [[.]]) | tojson",
        "reduce range(256) as $i (0; [., 1]) | tojson",
        "reduce range(257) as $i (0; [., 1]) | tojson",
        "reduce range(20) as $i (0; [[.]]) == reduce range(40) as $i (0; [.])",
        "reduce range(20) as $i (0; [., 1]) == reduce range(20) as $i (0; [., 2])",
        "reduce range(20) as $i ({}; {a:.}) == reduce range(20) as $i ({}; {b:.})",
        "reduce range(2000) as $i ({}; {a:.}) == reduce range(2000) as $i ({}; {a:.})",
        "reduce range(2000) as $i ({}; {a:.}) == reduce range(1999) as $i ({}; {a:.})",
        "reduce range(2000) as $i (0; [., 1]) == reduce range(2000) as $i (0; [., 1])",
        "reduce range(2000) as $i (0; [., 1]) == reduce range(2000) as $i (0; [., 2])",
        "reduce range(2000) as $i (0; [[.]]) == reduce range(2000) as $i (0; [[.]])",
        "reduce range(2000) as $i (0; [[.]]) == reduce range(1999) as $i (0; [[.]])",
        "reduce range(2000) as $i (0; [[.]]) == reduce range(4000) as $i (0; [.])",
        "reduce range(2000) as $i ({}; {a:.}) | tojson | length",
        "reduce range(2000) as $i (0; [., 1]) | tojson | length",
        "reduce range(2000) as $i (0; [[.]]) | tojson | length",
        "reduce range(0) as $i (0; [., .])",
        "reduce range(1) as $i (0; [., .])",
        "reduce range(4) as $i (0; [., .])",
        "reduce range(4) as $i (1; [., .])",
        "reduce range(6) as $i (0; [., .]) == reduce range(6) as $i (0; [., .])",
        "reduce range(6) as $i (0; [., .]) == reduce range(6) as $i (1; [., .])",
        "reduce range(6) as $i (0; [., .]) == reduce range(5) as $i (0; [., .])",
        "reduce range(12) as $i (0; [., .]) | length",
        "reduce range(20) as $i (0; [., .]) | length",
        "reduce range(4) as $i (0; [., .]) | .[0] == .[1]",
        "reduce range(4) as $i (0; [., .]) | .[0][0] = 9",
        "reduce range(3) as $i (0; {a:., b:.})",
        "reduce range(3) as $i (0; {a:., b:.}) | .a = 1",
        "reduce range(4) as $i (0; [.] | [.])",
        "reduce range(4) as $i (0; [.] | [.] | [.])",
        "reduce range(8) as $i (0; [.] | [.]) == reduce range(8) as $i (0; [[.]])",
        "reduce range(6) as $i (0; [.] | [., 1])",
        "reduce range(0) as $i (0; [.]) | [..]",
        "reduce range(1) as $i (0; [.]) | [..]",
        "reduce range(4) as $i (0; [.]) | [..]",
        "reduce range(6) as $i (0; [.]) | [..] | length",
        "reduce range(4) as $i ({}; {a:.}) | [..]",
        "reduce range(4) as $i (0; [., 1]) | [..]",
        "[range(8)] | [..] | length",
        "reduce range(4) as $i (0; [.]) | [.[]]",
        "def f: reduce range(5) as $i (0; [., .]); f | length",
        "def f: reduce range(5) as $i (0; [.] | [.]); f",
        "reduce range(2000) as $i (0; [.]) | [..] | length",
        "reduce range(500) as $i (0; [.] | [.]) | length",
        "reduce range(500) as $i (0; [.] | [.]) == reduce range(500) as $i (0; [[.]])",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
    compare_exact(
        "pretty 4",
        &["-n", "--", "reduce range(4) as $i (0; [.])"],
        b"",
    );
    compare_exact(
        "pretty 257",
        &["-n", "--", "reduce range(257) as $i (0; [.])"],
        b"",
    );
    compare_exact(
        "pretty object 257",
        &["-n", "--", "reduce range(257) as $i ({}; {a:.})"],
        b"",
    );
    compare_exact(
        "pretty comma 257",
        &["-n", "--", "reduce range(257) as $i (0; [., 1])"],
        b"",
    );
    compare_exact(
        "pretty double 129",
        &["-n", "--", "reduce range(129) as $i (0; [[.]])"],
        b"",
    );
    compare_exact(
        "wrap input object",
        &["-c", "--", "reduce range(4) as $i (.; {a:.})"],
        br#"{"z":1}"#,
    );
    compare_exact(
        "wrap input array",
        &["-c", "--", "reduce range(4) as $i (.; [., 1])"],
        b"[9]",
    );
    compare_exact(
        "wrap input double",
        &["-c", "--", "reduce range(3) as $i (.; [[.]])"],
        b"\"x\"",
    );
    compare_exact(
        "tab 257",
        &["-n", "--tab", "--", "reduce range(257) as $i (0; [.])"],
        b"",
    );
    compare_exact(
        "indent 1",
        &[
            "-n",
            "--indent",
            "1",
            "--",
            "reduce range(257) as $i (0; [.])",
        ],
        b"",
    );
    compare_exact(
        "compact 257",
        &["-n", "-c", "--", "reduce range(257) as $i (0; [.])"],
        b"",
    );
    compare_exact(
        "raw tostring",
        &[
            "-n",
            "-r",
            "--",
            "reduce range(257) as $i (0; [.]) | tostring",
        ],
        b"",
    );
    compare_exact(
        "sort keys 257",
        &[
            "-n",
            "-c",
            "-S",
            "--",
            "reduce range(257) as $i ({}; {b:1, a:.})",
        ],
        b"",
    );
    compare_exact(
        "sort keys shallow",
        &[
            "-n",
            "-c",
            "-S",
            "--",
            "reduce range(4) as $i ({}; {b:1, a:.})",
        ],
        b"",
    );
}

#[test]
fn ten_filters_match_jq() {
    require_jq();
    let programs = [
        r#""a,b,c" | split(",")"#,
        r#""a,b,c" | split("")"#,
        r#""aaa" | split("aa")"#,
        r#""aaa" | split("a")"#,
        "\"\" | split(\"\")",
        "\"\" | split(\",\")",
        r#""a," | split(",")"#,
        "\",\" | split(\",\")",
        r#""é,é" | split(",")"#,
        r#""é" | split("")"#,
        r#""😀" | split("")"#,
        r#""a😀b" | split("😀")"#,
        r#""a\u0000b\u0000c" | split("\u0000")"#,
        r#""ab\u0000cd" | split("b\u0000")"#,
        r#""abc" | split(",","/","b")"#,
        r#"try ("abc" | split(",", error("z"))) catch ."#,
        r#"try ("abc" | split(1)) catch ."#,
        r#"try ("abc" | split(null)) catch ."#,
        r#"try ("abc" | split(true)) catch ."#,
        r#"try ("abc" | split(false)) catch ."#,
        r#"try ("abc" | split([])) catch ."#,
        r#"try ("abc" | split({})) catch ."#,
        r#"try ("abc" | split(nan)) catch ."#,
        r#"try (1 | split(",")) catch ."#,
        r#"try (null | split("")) catch ."#,
        r#"try (true | split("a")) catch ."#,
        r#"try ([] | split(",")) catch ."#,
        r#"try ({} | split(",")) catch ."#,
        r#"try (nan | split("a")) catch ."#,
        "split",
        r#""1" | fromjson"#,
        r#""true" | fromjson"#,
        r#""\"a\"" | fromjson"#,
        r#""[1,2]" | fromjson"#,
        r#""{\"a\":1}" | fromjson"#,
        r#"" 1 " | fromjson"#,
        r#""nan" | fromjson | type"#,
        r#""NaN" | fromjson | type"#,
        r#""nan1234" | fromjson | type"#,
        r#""NaN1234" | fromjson | type"#,
        r#""nan0" | fromjson | type"#,
        r#""nan00" | fromjson | type"#,
        r#""nan1" | fromjson | type"#,
        r#""nan0001" | fromjson | type"#,
        r#""+nan0" | fromjson | type"#,
        r#""+nan" | fromjson | type"#,
        r#""-nan" | fromjson | type"#,
        r#""+nan99" | fromjson | type"#,
        r#""nan" | fromjson == null"#,
        r#""null" | fromjson == null"#,
        r#""Infinity" | fromjson"#,
        r#""-Infinity" | fromjson"#,
        r#""+Infinity" | fromjson"#,
        r#""Inf" | fromjson"#,
        r#""-Inf" | fromjson"#,
        r#""infinity" | fromjson"#,
        r#""01" | fromjson"#,
        r#""+1" | fromjson"#,
        r#""1." | fromjson"#,
        r#"".1" | fromjson"#,
        r#""1e2" | fromjson"#,
        r#"try ("nanabc" | fromjson) catch ."#,
        r#"try ("x" | fromjson) catch ."#,
        r#"try ("" | fromjson) catch ."#,
        r#"try (" " | fromjson) catch ."#,
        r#"try ("1 2" | fromjson) catch ."#,
        r#"try ("{}" | fromjson) catch ."#,
        r#"try ("{" | fromjson) catch ."#,
        r#"try ("[1,]" | fromjson) catch ."#,
        "try (\"# c\\n1\" | fromjson) catch .",
        r#"try (1 | fromjson) catch ."#,
        r#"try (null | fromjson) catch ."#,
        r#"try (true | fromjson) catch ."#,
        r#"try (false | fromjson) catch ."#,
        r#"try ([] | fromjson) catch ."#,
        r#"try ({} | fromjson) catch ."#,
        r#"try (nan | fromjson) catch ."#,
        r#"try ("abcdefghijklmno" | fromjson) catch ."#,
        "fromjson(1)",
        r#""foo" | contains("foo")"#,
        r#""foobar" | contains("foo")"#,
        r#""foo" | contains("foobar")"#,
        r#""foo" | contains("")"#,
        "\"\" | contains(\"\")",
        "\"\" | contains(\"a\")",
        r#""ab\u0000cd" | contains("\u0000")"#,
        r#""ab\u0000cd" | contains("b\u0000c")"#,
        r#""ébc" | contains("é")"#,
        r#""a😀b" | contains("😀")"#,
        r#"try ("abc" | contains(1)) catch ."#,
        r#"try ("abc" | contains(null)) catch ."#,
        r#"try ("abc" | contains(true)) catch ."#,
        r#"try ("abc" | contains([])) catch ."#,
        r#"try ("abc" | contains({})) catch ."#,
        r#""abc" | contains("a","x","bc")"#,
        r#"try ("abc" | contains("a", error("z"))) catch ."#,
        "[] | contains([])",
        "[1,2,3] | contains([1,2])",
        "[1,2,3] | contains([3,1])",
        "[1,2,3] | contains([4])",
        "[1,2,3] | contains([1,4])",
        "[1] | contains([1,1])",
        "[] | contains([1])",
        "[1,2,3] | contains([])",
        r#"["foobar","foobaz"] | contains(["baz","bar"])"#,
        r#"["foobar","foobaz"] | contains(["blap"])"#,
        "[[1,2],[3,4]] | contains([[1]])",
        "[[1,2,3]] | contains([[1,2]])",
        "[[1,2]] | contains([1])",
        "[[[1]]] | contains([[1]])",
        "[1,[2,3]] | contains([[2]])",
        "[1,[2,3]] | contains([2])",
        r#"try ([1,2,3] | contains(1)) catch ."#,
        r#"try ([[1,2]] | contains(1)) catch ."#,
        "[true] | contains([false])",
        "[true,false] | contains([false])",
        r#"try (true | contains(false)) catch ."#,
        r#"try (false | contains(true)) catch ."#,
        "true | contains(true)",
        "false | contains(false)",
        "1 | contains(1)",
        "1 | contains(1.0)",
        "1 | contains(2)",
        "nan | contains(nan)",
        "nan | contains(1)",
        "null | contains(null)",
        r#"try (null | contains(false)) catch ."#,
        r#"try (1 | contains("1")) catch ."#,
        r#"{"foo":12,"bar":13} | contains({"foo":12})"#,
        r#"{"foo":12} | contains({})"#,
        r#"{"foo":12,"bar":13} | contains({"baz":14})"#,
        r#"{"a":[1,2,3]} | contains({"a":[1]})"#,
        r#"{"a":"foo"} | contains({"a":["f"]})"#,
        r#"{"a":"foobar"} | contains({"a":"foo"})"#,
        r#"{"a":1} | contains({"a":2})"#,
        r#"{"a":1} | contains({"a":null})"#,
        r#"{"a":null} | contains({"a":null})"#,
        r#"{"a":true} | contains({"a":false})"#,
        r#"{"foo":{"baz":12,"blap":{"bar":13}},"bar":14} | contains({"bar":14,"foo":{"blap":{}}})"#,
        r#"{"foo":{"baz":12,"blap":{"bar":13}},"bar":14} | contains({"bar":14,"foo":{"blap":{"bar":14}}})"#,
        r#"try ({"a":1} | contains(1)) catch ."#,
        r#"try ("abcdefghijklmno" | contains(1)) catch ."#,
        r#""foo" | inside("foobar")"#,
        r#""foobar" | inside("foo")"#,
        "2 | inside([1,2])",
        "[1,2] | inside([1,2,3])",
        r#"try (2 | inside([1,2,3])) catch ."#,
        r#""a" | inside("abc","a","x")"#,
        r#"try ("a" | inside("abc", error("z"))) catch ."#,
        "1 as $x | 2 | inside($x)",
        "contains",
        "inside",
        "[0,[1],[[2]],[[[3]]]] | flatten",
        "[] | flatten",
        "[[]] | flatten",
        "[[[]]] | flatten",
        "[1,2,3] | flatten",
        r#"{"a":[1]} | flatten"#,
        r#"{"a":1} | flatten"#,
        r#"[{"a":[2]},1] | flatten"#,
        "[0,[1],[[2]],[[[3]]]] | flatten(0)",
        "[0,[1],[[2]],[[[3]]]] | flatten(1)",
        "[0,[1],[[2]],[[[3]]]] | flatten(2)",
        "[0,[1],[[2]],[[[3]]]] | flatten(3)",
        "[0,[1],[[2]],[[[3]]]] | flatten(1.0)",
        "[0,[1],[[2]],[[[3]]]] | flatten(2.0)",
        "[0,[1],[[2]],[[[3]]]] | flatten(0.5)",
        "[0,[1],[[2]],[[[3]]]] | flatten(1.5)",
        "[0,[1],[[2]],[[[3]]]] | flatten(1.9)",
        "[0,[1],[[2]],[[[3]]]] | flatten(2.9)",
        "[0,[1],[[2]],[[[3]]]] | flatten(1e-10)",
        "[0,[1],[[2]],[[[3]]]] | flatten(infinite)",
        "[0,[1],[[2]],[[[3]]]] | flatten(-0)",
        r#"try ([0,[1],[[2]],[[[3]]]] | flatten(-1)) catch ."#,
        r#"try ([0,[1]] | flatten(-0.5)) catch ."#,
        r#"try ([1,2] | flatten(nan)) catch ."#,
        r#"try ([1,2] | flatten(null)) catch ."#,
        r#"try ([1,2] | flatten(true)) catch ."#,
        r#"try ([1,2] | flatten(false)) catch ."#,
        r#"try (1 | flatten(-1)) catch ."#,
        r#"try (1 | flatten(1)) catch ."#,
        r#"try (null | flatten) catch ."#,
        r#"try ("a" | flatten) catch ."#,
        r#"try (true | flatten) catch ."#,
        "[1,2,3] | flatten(\"a\")",
        r#"try ([1,[2]] | flatten("a")) catch ."#,
        r#"try ([1,[2]] | flatten("1")) catch ."#,
        r#"try ([1,[2]] | flatten([])) catch ."#,
        r#"try ([1,[2]] | flatten({})) catch ."#,
        r#"{"a":[1,[2]]} | flatten(1)"#,
        r#"{"a":[1]} | flatten(0)"#,
        r#"try ({"a":1} | flatten("x")) catch ."#,
        "[0,[1],[[2]]] | flatten(1,2,0)",
        r#"try ([0,[1]] | flatten(1, error("z"), 0)) catch ."#,
        "flatten(1;2)",
        "[] | any",
        "[] | all",
        "[true] | any",
        "[true] | all",
        "[false] | any",
        "[false] | all",
        "[true,false] | [any, all]",
        "[null,null,true] | [any, all]",
        "[0] | any",
        "[0] | all",
        r#"[""] | any"#,
        "[[]] | any",
        "[{}] | all",
        "[null] | all",
        "[] | any(not)",
        "[] | all(not)",
        "[false] | any(not)",
        "[false] | all(not)",
        "[1,2,3] | any(.>2)",
        "[1,2,3] | all(.>0)",
        "[1,2,3] | all(.>2)",
        r#"try any(true, error("z"); .) catch ."#,
        r#"try all(false, error("z"); .) catch ."#,
        "any(1,2,3; .>10)",
        "all(1,2,3; .<10)",
        "any(empty; .)",
        "all(empty; .)",
        "any(1; empty)",
        "all(1; empty)",
        "any(1; false, true)",
        "all(1; true, false)",
        "all(1; true, true)",
        r#"{"a":0,"b":2} | any(.>1)"#,
        r#"{"a":1,"b":2} | all(.>0)"#,
        r#"{"a":false,"b":1} | any"#,
        r#"{"a":1,"b":false} | all"#,
        r#"try (1 | any) catch ."#,
        r#"try ("ab" | all) catch ."#,
        r#"try (null | any) catch ."#,
        r#"try (true | all) catch ."#,
        r#"try (nan | any) catch ."#,
        r#"try ([1,2] | any(error("z"))) catch ."#,
        r#"try ([1] | any(.<0, error("z"))) catch ."#,
        r#"try ([1] | any(.>0, error("z"))) catch ."#,
        r#"try ([1,2] | all(.>0, error("z"))) catch ."#,
        r#"try ([1,0] | all(.>0, error("z"))) catch ."#,
        "any(select(.>2))",
        "[1,2,3] | any(select(.>2))",
        "[1,2] | any(select(.>9))",
        "[1,2] | all(select(.>9))",
        "any(3)",
        "all(1;2;3)",
        "isempty(empty)",
        "isempty(range(3))",
        r#"try isempty(1, error("z")) catch ."#,
        "isempty(1)",
        "isempty(false)",
        "isempty(null)",
        "isempty(0)",
        r#"isempty("")"#,
        "isempty([])",
        "isempty({})",
        "[] | isempty(.[] )",
        "[1] | isempty(.[] )",
        "{} | isempty(.[] )",
        r#"{"a":1} | isempty(.a)"#,
        r#"try isempty(error("z")) catch ."#,
        r#"try isempty(empty, error("z")) catch ."#,
        r#"try (1 | isempty(.[])) catch ."#,
        r#"try ("a" | isempty(.[])) catch ."#,
        "isempty",
        "[0,1,2] | map_values(.+1)",
        "[] | map_values(.+1)",
        r#"{"a":1,"b":2} | map_values(.+1)"#,
        r#"{"b":1,"a":2} | map_values(.+1)"#,
        "{} | map_values(.+1)",
        "[1,2,3] | map_values(select(.>1))",
        r#"{"a":1,"b":2} | map_values(select(.>1))"#,
        "[1,2,3] | map_values(empty)",
        r#"{"a":1,"c":3,"b":2} | map_values(if .==2 then empty else . end)"#,
        "[1,2] | map_values(., .+10)",
        r#"{"a":1} | map_values(., .+1)"#,
        r#"try ([1] | map_values(1, error("z"))) catch ."#,
        r#"try ([1,2] | map_values(if .==1 then (1, error("z")) else .+1 end)) catch ."#,
        r#"try ([1,2] | map_values(if .==1 then error("z") else .+1 end)) catch ."#,
        r#"{"a":null,"b":1} | map_values(.+1)"#,
        "[[1],[2]] | map_values(map(.+1))",
        r#"try (null | map_values(.+1)) catch ."#,
        r#"try (1 | map_values(.+1)) catch ."#,
        r#"try ("abc" | map_values(.)) catch ."#,
        r#"try (true | map_values(.)) catch ."#,
        r#"try (nan | map_values(.)) catch ."#,
        "map_values",
        r#"{"b":1,"a":2} | keys_unsorted"#,
        r#"{"z":1,"a":2,"m":3} | keys_unsorted"#,
        r#"{"b":1,"a":2} | keys"#,
        r#"{"10":1,"2":2,"a":3} | keys_unsorted"#,
        r#"{"é":1,"a":2} | keys_unsorted"#,
        "{} | keys_unsorted",
        "[] | keys_unsorted",
        "[10,20,30] | keys_unsorted",
        r#"try (null | keys_unsorted) catch ."#,
        r#"try (1 | keys_unsorted) catch ."#,
        r#"try (true | keys_unsorted) catch ."#,
        r#"try ("abc" | keys_unsorted) catch ."#,
        r#"try ("" | keys_unsorted) catch ."#,
        r#"try (nan | keys_unsorted) catch ."#,
        r#"try ("abcdefghijklmno" | keys_unsorted) catch ."#,
        r#"{"b":1,"a":2} + {"b":3} | keys_unsorted"#,
        "keys_unsorted(1)",
        r#"[{"b":1,"a":2},{"a":1},{"c":1,"a":0,"b":2}] | sort_by(keys_unsorted)"#,
        r#"def f($s): split($s); "a,b" | f(",")"#,
        r#"def f: fromjson; "\"1\"" | f"#,
        "def f($x): contains($x); [1,2,3] | f([2])",
        "def f($x): inside($x); [1] | f([[1],[2]])",
        "def f: flatten; [[1],[2]] | f",
        "def f: any(.>1); [0,2] | f",
        "def f: all(.>0); [1,2] | f",
        "def f: isempty(.[1]); [1] | f",
        "def f: map_values(.+1); [1,2] | f",
        r#"def f: keys_unsorted; {"b":1,"a":2} | f"#,
        "try path(any) catch .",
        "[true,false] | try path(any) catch .",
        "[false,true] | try path(any) catch .",
        "[1] | try path(any) catch .",
        "[1,true] | try path(any) catch .",
        "[false,1,true] | try path(any) catch .",
        r#"{"a":true,"b":false} | try path(any) catch ."#,
        "[true,false] | (any = 9)",
        "[false,true,true] | (any = 9)",
        "[1,0,2] | try (any = 9) catch .",
        "[true] | del(any)",
        "[true,false,true] | del(any)",
        "[false] | try path(all) catch .",
        "[true,false] | try path(all) catch .",
        "[true,false] | (all = 9)",
        "[true] | try path(all) catch .",
        "[] | try path(all) catch .",
        "[true,false] | try path(any(.)) catch .",
        "[true,false] | try path(any(.; .)) catch .",
        "[1,2,3] | try path(any(.>1)) catch .",
        "[2,0,3] | try path(any(.>1)) catch .",
        "[[true],[false]] | try path(any(any)) catch .",
        "[[false],[true]] | try path(any(any)) catch .",
        r#"[{"a":true}] | try path(any(.a)) catch ."#,
        "[[false]] | try path(all(all)) catch .",
        "[[true],[false]] | (all(all) = 9)",
        r#"[{"a":false}] | try path(all(.a)) catch ."#,
        "[[true],[false]] | try path(any(.[]; any)) catch .",
        "[1] | try path(isempty(.)) catch .",
        "[1] | try path(isempty(empty)) catch .",
        "[] | try path(isempty(.[])) catch .",
        r#""a,b" | try path(split(",")) catch ."#,
        r#""abc" | try path(contains("a")) catch ."#,
        "[1] | try path(contains(1)) catch .",
        "[1,[2]] | try path(flatten) catch .",
        r#""1" | try path(fromjson) catch ."#,
        r#"{"b":1,"a":2} | try path(keys_unsorted) catch ."#,
        "[true] | (any |= not)",
        "1 | try (contains([])?) catch .",
        r#"{"a":nan} | tojson | fromjson"#,
        r#""{\"a\":nan}" | fromjson | .a | type"#,
        "[1] | try path(map_values(.+1)) catch .",
        "[1,2] | try (map_values(.+1) = [9]) catch .",
        "[1,2] | try del(map_values(.+1)) catch .",
        r#"{"a":1} | try path(map_values(.+1)) catch ."#,
        "null | try path(map_values(.+1)) catch .",
        r#"try ([1,2] | path(map_values(error("z")))) catch ."#,
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
    let streamed = [
        (
            "any stops before input",
            "any(true, input; .), input",
            &b"1\n2\n"[..],
        ),
        (
            "all stops before input",
            "all(false, input; .), input",
            &b"1\n2\n"[..],
        ),
        (
            "isempty stops before input",
            "isempty(1, input), input",
            &b"1\n2\n"[..],
        ),
        (
            "map_values stops before input",
            "[1] | map_values(1, input), input",
            &b"10\n20\n"[..],
        ),
    ];
    for (name, program, input) in streamed {
        compare_exact(name, &["-n", "-c", "--", program], input);
    }
}

#[test]
fn type_filters_match_jq() {
    require_jq();
    let programs = [
        r#"[1,2,"foo",[],[3,[]],{},true,false,null] | [.[]|arrays]"#,
        r#"[1,2,"foo",[],[3,[]],{},true,false,null] | [.[]|objects]"#,
        r#"[1,2,"foo",[],[3,[]],{},true,false,null] | [.[]|iterables]"#,
        r#"[1,2,"foo",[],[3,[]],{},true,false,null] | [.[]|scalars]"#,
        r#"[1,2,"foo",[],[3,[]],{},true,false,null] | [.[]|values]"#,
        r#"[1,2,"foo",[],[3,[]],{},true,false,null] | [.[]|booleans]"#,
        r#"[1,2,"foo",[],[3,[]],{},true,false,null] | [.[]|nulls]"#,
        r#"[1,null,"a",{},[2],true,false,0] | [.[]|numbers]"#,
        r#"[1,null,"a",{},[2],true,false,0] | [.[]|strings]"#,
        "null | [values, nulls, booleans, numbers, strings, arrays, objects, iterables, scalars]",
        "false | [values, booleans, scalars]",
        "true | [values, booleans, scalars]",
        "0 | [values, numbers, scalars]",
        "1.0 | numbers",
        "nan | [numbers, scalars, values]",
        "infinite | [numbers, scalars]",
        r#""a" | [strings, scalars, arrays]"#,
        "[] | [arrays, iterables, scalars]",
        "{} | [objects, iterables, scalars]",
        "path(values)",
        "path(nulls)",
        "path(numbers)",
        "path(strings)",
        "path(booleans)",
        "path(arrays)",
        "path(objects)",
        "path(iterables)",
        "path(scalars)",
        "[1,null,2] | .[] |= values",
        "[1,null,2] | .[] |= numbers",
        r#"{"a":1,"b":null,"c":"x"} | .[] |= strings"#,
        r#"{"a":1,"b":[]} | .[] |= iterables"#,
        r#"[1,null,"a",{}] | .[] |= scalars"#,
        "1 | IN(1,2,3)",
        "2 | IN(1,2,3)",
        "0 | IN(1,2)",
        "1 | IN(empty)",
        "IN(empty)",
        "IN(1; empty)",
        "IN(empty; 1)",
        "null | IN(null, 1)",
        "false | IN(false, null)",
        "nan | IN(nan)",
        "nan | IN(nan, 1)",
        r#"{"b":1,"a":2} | IN({"a":2,"b":1})"#,
        r#"1 | IN("1", 1)"#,
        r#""1" | IN(1, "1")"#,
        "1,2 | IN(2)",
        "1 == 1.0 | IN(true)",
        "range(5;10) | IN(range(10))",
        "range(5;13) | IN(range(0;10;3))",
        "range(10;12) | IN(range(10))",
        "IN(range(10;20); range(10))",
        "IN(range(5;20); range(10))",
        r#"try (1 | IN(1, error("z"))) catch ."#,
        r#"try (0 | IN(1, error("z"))) catch ."#,
        r#"try (1 | IN(error("z"), 1)) catch ."#,
        r#"try IN(1, error("z"); 1) catch ."#,
        r#"try IN(error("z"), 1; 1) catch ."#,
        r#"try IN(9; 1, error("z")) catch ."#,
        r#"try IN(1; 1, error("z")) catch ."#,
        r#"try IN(2; 1, 2, error("z")) catch ."#,
        r#"try IN((1, error("z")); 0, 1) catch ."#,
        r#"try IN((1, error("z")); 1, 0) catch ."#,
        "path(IN(1))",
        "path(IN(1; 1))",
        "path(IN(1; 2))",
        "9 as $v | 1 | IN($v)",
        "1 as $v | 1 | IN($v, 2)",
        "5 as $x | IN(1, $x; 5)",
        "5 as $y | IN($y; 1, 5)",
        r#"{"a":1,"b":[]} | walk(select(IN({}, []) | not))"#,
        r#"INDEX(range(5)|[., "foo\(.)"]; .[0])"#,
        "[1,2,3] | INDEX(.)",
        r#"[{"a":1},{"a":2},{"a":1}] | INDEX(.a)"#,
        "INDEX(1,2,1; .)",
        r#"INDEX("b","a","b"; .)"#,
        r#"INDEX(1; 1, "a")"#,
        "INDEX(1; empty)",
        r#"INDEX(1; empty, "a")"#,
        "INDEX(empty; .)",
        "INDEX(nan, null; .)",
        "INDEX(nan; .) | .null | type",
        "INDEX(-0, 0, 1.0, 1; .)",
        r#"INDEX("1", 1; .)"#,
        r#"INDEX(1, "1"; .)"#,
        r#"[null,true,false,0,1,1.5,"a",[1],{"b":1,"a":2}] | INDEX(.)"#,
        "INDEX(infinite, -infinite; .)",
        "null | INDEX(.)",
        r#""ab" | INDEX(.)"#,
        "1 | INDEX(.)",
        r#"try (null | INDEX(.)) catch ."#,
        r#"try ("ab" | INDEX(.)) catch ."#,
        r#"try INDEX(1, error("z"); .) catch ."#,
        r#"try INDEX(10; 1, error("z")) catch ."#,
        r#"try INDEX(10; error("z"), 1) catch ."#,
        r#"try INDEX(error("z"); .) catch ."#,
        "[{a:1},{}] | INDEX(.a)",
        r#"INDEX([1,2], {"z":1,"a":2}; .)"#,
        "null | INDEX(1,2; .)",
        "[10,20] | INDEX(.+1)",
        r#"{"a":1,"b":2} | INDEX(.)"#,
        "path(INDEX(.))",
        "path(INDEX(1; .))",
        "[1,2,3] | path(INDEX(.))",
        "INDEX(9007199254740993; .)",
        "INDEX(1.5, 1.0, 0.0, -0.0; .)",
        r#"INDEX(1; "a", "a")"#,
        "9 as $row | INDEX(1,2; $row)",
        "9 as $row | INDEX(1,2; .)",
        "9 as $row | INDEX(1,2; $row + 1)",
        "1 as $row | [1,2] | INDEX(.[]; $row)",
        "def values: 7; 1, null | values",
        "def IN(s): 7; 1 | IN(1)",
        "def INDEX(k): 7; [1] | INDEX(k)",
        "values(1)",
        "nulls(1)",
        "IN",
        "IN(1;2;3)",
        "INDEX",
        "INDEX(1;2;3)",
        "booleans(1)",
        "iterables(1)",
        "scalars(1)",
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
    let streamed = [
        (
            "in stops before input",
            "try (1 | IN(1, input)) catch ., input",
            &b"9\n"[..],
        ),
        (
            "in reads input",
            "try (0 | IN(1, input)) catch .",
            &b"9\n"[..],
        ),
        ("in at end", "try (0 | IN(1, input)) catch .", b""),
        ("index one input", "INDEX(input; .)", &b"1\n2\n"[..]),
        ("index every input", "INDEX(inputs; .)", &b"1\n2\n"[..]),
    ];
    for (name, program, input) in streamed {
        compare_exact(name, &["-n", "-c", "--", program], input);
    }
}

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
    deep.extend(std::iter::repeat(b']').take(300));
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

/// `pick`, `transpose`, both `combinations`, `isnan`, `isinfinite`,
/// `isnormal`, `halt`, and both `halt_error` forms.
#[test]
fn eight_match_jq() {
    require_jq();
    let cases: &[(&str, &[&str], &[u8])] = &[
        ("isnan zero", &["-c", "isnan"], b"0\n"),
        ("isnan one", &["-c", "isnan"], b"1\n"),
        ("isnan nan", &["-c", "isnan"], b"nan\n"),
        ("isnan null", &["-c", "isnan"], b"null\n"),
        ("isnan text", &["-c", "isnan"], b"\"x\"\n"),
        ("isnan array", &["-c", "isnan"], b"[1]\n"),
        ("isnan object", &["-c", "isnan"], b"{}\n"),
        ("isnan bool", &["-c", "isnan"], b"true\n"),
        ("isnan neg zero", &["-nc", "(-0)|isnan"], b""),
        ("isnan inf", &["-nc", "infinite|isnan"], b""),
        ("isinfinite inf", &["-nc", "infinite|isinfinite"], b""),
        ("isinfinite ninf", &["-nc", "(-infinite)|isinfinite"], b""),
        ("isinfinite nan", &["-nc", "nan|isinfinite"], b""),
        ("isinfinite one", &["-nc", "1|isinfinite"], b""),
        ("isinfinite null", &["-nc", "null|isinfinite"], b""),
        ("isnormal one", &["-nc", "1|isnormal"], b""),
        ("isnormal zero", &["-nc", "0|isnormal"], b""),
        ("isnormal neg zero", &["-nc", "(-0)|isnormal"], b""),
        ("isnormal nan", &["-nc", "nan|isnormal"], b""),
        ("isnormal inf", &["-nc", "infinite|isnormal"], b""),
        ("isnormal sub", &["-nc", "1e-320|isnormal"], b""),
        (
            "isnormal min",
            &["-nc", "2.2250738585072014e-308|isnormal"],
            b"",
        ),
        ("isnormal plain", &["-nc", "1e-307|isnormal"], b""),
        ("isnan fromjson", &["-c", "fromjson|isnan"], b"\"nan\"\n"),
        (
            "isnan fromjson digits",
            &["-c", "fromjson|isnan"],
            b"\"nan1234\"\n",
        ),
        ("isnan mod", &["-nc", "[nan % 1, 1 % nan | isnan]"], b""),
        ("isnan empty", &["-nc", "empty|isnan"], b""),
        ("isnan arity", &["-nc", "isnan(1)"], b""),
        ("halt plain", &["-nc", "1, halt, 2"], b""),
        ("halt only", &["-nc", "halt"], b""),
        ("halt pipe", &["-nc", "1 | halt, 2"], b""),
        ("halt array", &["-nc", "[1, halt, 2]"], b""),
        ("halt object", &["-nc", "{a: halt}"], b""),
        ("halt try", &["-nc", "try (1, halt) catch \"c\""], b""),
        ("halt optional", &["-nc", "halt?"], b""),
        ("halt empty", &["-nc", "empty|halt"], b""),
        ("halt exit", &["-ne", "halt"], b""),
        ("halt false exit", &["-ne", "false, halt"], b""),
        ("halt null exit", &["-ne", "null, halt"], b""),
        ("halt shadow", &["-nc", "def halt: 9; halt"], b""),
        ("halt arity", &["-nc", "halt(1)"], b""),
        ("halt input", &["-c", "halt"], b"7\n"),
        ("herr string", &["-nc", "\"boom\"|halt_error"], b""),
        ("herr number", &["-nc", "1|halt_error"], b""),
        ("herr null", &["-nc", "null|halt_error"], b""),
        ("herr false", &["-nc", "false|halt_error"], b""),
        ("herr object", &["-nc", "{a:1}|halt_error"], b""),
        ("herr array", &["-nc", "[1,2]|halt_error"], b""),
        ("herr empty string", &["-nc", "\"\"|halt_error"], b""),
        ("herr newline", &["-nc", "\"hello\\n\"|halt_error"], b""),
        ("herr plain string", &["-nc", "\"hello\"|halt_error"], b""),
        ("herr code", &["-nc", "\"boom\"|halt_error(3)"], b""),
        ("herr code zero", &["-ne", "\"z\"|halt_error(0)"], b""),
        ("herr trunc", &["-nc", "\"z\"|halt_error(1.9)"], b""),
        ("herr neg", &["-nc", "\"z\"|halt_error(-1)"], b""),
        ("herr wide", &["-nc", "\"z\"|halt_error(300)"], b""),
        ("herr byte", &["-nc", "\"z\"|halt_error(256)"], b""),
        ("herr nan code", &["-nc", "\"z\"|halt_error(nan)"], b""),
        ("herr inf code", &["-nc", "\"z\"|halt_error(infinite)"], b""),
        (
            "herr ninf code",
            &["-nc", "\"z\"|halt_error(-infinite)"],
            b"",
        ),
        ("herr big", &["-nc", "\"z\"|halt_error(1e20)"], b""),
        (
            "herr int max",
            &["-nc", "\"z\"|halt_error(2147483648)"],
            b"",
        ),
        ("herr bad code", &["-nc", "\"z\"|halt_error(\"x\")"], b""),
        ("herr bad input", &["-nc", "1|halt_error(\"x\")"], b""),
        ("herr null code", &["-nc", "\"z\"|halt_error(null, 1)"], b""),
        (
            "herr caught",
            &["-nc", "try (\"z\"|halt_error(\"x\")) catch ."],
            b"",
        ),
        (
            "herr try",
            &["-nc", "try (1, \"e\"|halt_error) catch \"c\""],
            b"",
        ),
        ("herr optional", &["-nc", "(\"z\"|halt_error)?"], b""),
        ("herr length", &["-nc", "\"ab\"|halt_error(length)"], b""),
        ("herr before", &["-nc", "1, (\"e\"|halt_error), 2"], b""),
        ("herr empty in", &["-nc", "empty|halt_error"], b""),
        ("herr empty code", &["-nc", "\"z\"|halt_error(empty)"], b""),
        (
            "herr stream",
            &["-nc", "\"z\"|halt_error(1, error(\"no\"))"],
            b"",
        ),
        ("herr nan value", &["-nc", "nan|halt_error"], b""),
        ("herr inf value", &["-nc", "infinite|halt_error"], b""),
        ("herr neg zero", &["-nc", "(-0)|halt_error"], b""),
        ("herr pretty", &["{b:1,a:2}|halt_error"], b"null\n"),
        ("herr raw", &["-r", "halt_error"], b"\"hi\"\n"),
        ("herr join", &["-j", "halt_error"], b"{\"a\":1}\n"),
        ("herr sort", &["-S", "halt_error"], b"{\"b\":1,\"a\":2}\n"),
        ("herr nul", &["-nc", "\"a\\u0000b\"|halt_error"], b""),
        (
            "herr shadow",
            &["-nc", "def halt_error: 9; halt_error"],
            b"",
        ),
        (
            "herr shadow code",
            &["-nc", "def halt_error: 9; \"z\"|halt_error(3)"],
            b"",
        ),
        ("herr arity", &["-nc", "halt_error(1;2)"], b""),
        ("herr two lines", &["-c", "halt_error"], b"1\n2\n"),
        ("pick nest", &["-c", "pick(.a.b.c)"], b"null\n"),
        ("pick first", &["-c", "pick(first)"], b"[1,2]\n"),
        (
            "pick first twice",
            &["-c", "pick(first|first)"],
            b"[[10,20],30]\n",
        ),
        ("pick last", &["-c", "try pick(last) catch ."], b"[1,2]\n"),
        (
            "pick keys",
            &["-c", "pick(.a, .b)"],
            b"{\"a\":1,\"b\":2,\"c\":3}\n",
        ),
        ("pick missing", &["-c", "pick(.no)"], b"{\"a\":1}\n"),
        ("pick empty", &["-c", "pick(empty)"], b"{\"a\":1}\n"),
        ("pick ident", &["-c", "pick(.)"], b"{\"a\":1}\n"),
        ("pick index", &["-c", "pick(.[1])"], b"[0,1,2]\n"),
        ("pick indexes", &["-c", "pick(.[2], .[0])"], b"[0,1,2,3]\n"),
        (
            "pick deep",
            &["-c", "pick(.a.b, .a)"],
            b"{\"a\":{\"b\":1,\"c\":2}}\n",
        ),
        (
            "pick shallow",
            &["-c", "pick(.a, .a.b)"],
            b"{\"a\":{\"b\":1,\"c\":2}}\n",
        ),
        ("pick string", &["-c", "pick(.[1:3])"], b"\"abcd\"\n"),
        (
            "pick error",
            &["-c", "try pick(error(\"z\")) catch ."],
            b"1\n",
        ),
        ("pick number", &["-c", "pick(.a)"], b"1\n"),
        ("pick halt", &["-nc", "pick(halt)"], b""),
        ("pick arity", &["-nc", "pick"], b""),
        (
            "pick shadow",
            &["-nc", "def path(f): 1; {\"a\":1}|pick(.a)"],
            b"",
        ),
        ("transpose jagged", &["-c", "transpose"], b"[[1],[2,3]]\n"),
        ("transpose empty", &["-c", "transpose"], b"[]\n"),
        ("transpose row", &["-c", "transpose"], b"[[1,2,3]]\n"),
        ("transpose square", &["-c", "transpose"], b"[[1,2],[3,4]]\n"),
        ("transpose pad", &["-c", "transpose"], b"[[],[1,2]]\n"),
        ("transpose number", &["-c", "try transpose catch ."], b"1\n"),
        ("transpose object", &["-c", "transpose"], b"{}\n"),
        (
            "transpose three",
            &["-c", "transpose"],
            b"[[1,2,3],[4],[5,6]]\n",
        ),
        (
            "transpose shadow",
            &["-nc", "def max: \"no\"; [[1,2],[3,4]]|transpose"],
            b"",
        ),
        ("transpose arity", &["-nc", "transpose(1)"], b""),
        ("comb empty", &["-c", "combinations"], b"[]\n"),
        ("comb one", &["-c", "combinations"], b"[[1,2]]\n"),
        ("comb two", &["-c", "combinations"], b"[[1,2],[3,4]]\n"),
        (
            "comb three",
            &["-c", "[combinations]"],
            b"[[1],[2,3],[4]]\n",
        ),
        ("comb hole", &["-c", "combinations"], b"[[1],[],[2]]\n"),
        ("comb number", &["-c", "try combinations catch ."], b"1\n"),
        ("comb null", &["-c", "combinations"], b"null\n"),
        ("comb n zero", &["-c", "combinations(0)"], b"[1,2]\n"),
        ("comb n one", &["-c", "combinations(1)"], b"[1,2]\n"),
        ("comb n two", &["-c", "[combinations(2)]"], b"[1,2]\n"),
        ("comb n neg", &["-c", "combinations(-1)"], b"[1,2]\n"),
        (
            "comb n length",
            &["-c", "combinations(length)"],
            b"[1,2,3]\n",
        ),
        ("comb n stream", &["-c", "[combinations(1,2)]"], b"[1,2]\n"),
        ("comb n empty", &["-c", "combinations(empty)"], b"[1,2]\n"),
        (
            "comb n string",
            &["-c", "try combinations(\"a\") catch ."],
            b"[1]\n",
        ),
        ("comb rows", &["-c", "try combinations catch ."], b"[1,2]\n"),
        (
            "comb shadow",
            &["-nc", "def range(x): 7; [1,2]|combinations(2)"],
            b"",
        ),
        ("comb arity", &["-nc", "combinations(1;2)"], b""),
    ];
    for (name, args, input) in cases {
        compare_exact(name, args, input);
    }

    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("halt-span-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    let left = dir.join("left.json");
    let right = dir.join("right.json");
    fs::write(&left, b"1\n").unwrap();
    fs::write(&right, b"2\n").unwrap();
    compare_exact(
        "halt files",
        &[
            "-c",
            "halt",
            left.to_str().unwrap(),
            right.to_str().unwrap(),
        ],
        b"",
    );
    compare_exact(
        "halt_error files",
        &[
            "-c",
            "halt_error",
            left.to_str().unwrap(),
            right.to_str().unwrap(),
        ],
        b"",
    );
    let _ = fs::remove_dir_all(&dir);
}

/// `debug`, `stderr`, `input_line_number`, `builtins`, `finites`, `normals`,
/// `get_jq_origin`, and `get_prog_origin`.
#[test]
fn report_match_jq() {
    require_jq();
    let cases: &[(&str, &[&str], &[u8])] = &[
        ("debug null", &["-nc", "debug"], b""),
        ("debug number", &["-c", "debug"], b"1\n"),
        ("debug pretty", &["debug"], b"{\"b\":1,\"a\":2}\n"),
        ("debug compact", &["-c", "debug"], b"{\"b\":1,\"a\":2}\n"),
        ("debug string", &["-c", "debug"], b"\"hi\"\n"),
        ("debug sorted", &["-S", "debug"], b"{\"b\":1,\"a\":2}\n"),
        ("debug raw", &["-r", "debug"], b"\"hi\"\n"),
        ("debug join", &["-j", "debug"], b"\"hi\"\n"),
        ("debug tab", &["--tab", "debug"], b"{\"a\":1}\n"),
        ("debug both", &["-cS", "debug"], b"{\"b\":1,\"a\":2}\n"),
        ("debug ascii", &["-anc", "\"é\"|debug"], b""),
        ("stderr ascii", &["-anc", "{\"b\":\"é\"}|stderr"], b""),
        ("debug message", &["-nc", "1|debug(\"x\")"], b""),
        ("debug messages", &["-nc", "1|debug(1,2)"], b""),
        ("debug empty msg", &["-nc", "1|debug(empty)"], b""),
        ("debug pipe", &["-nc", "1|debug|.+1"], b""),
        ("debug array", &["-nc", "[1|debug]"], b""),
        ("debug two", &["-nc", "(1,2)|debug"], b""),
        ("debug try", &["-nc", "try debug catch \"c\""], b""),
        ("debug error", &["-nc", "debug(error(\"z\"))"], b""),
        ("debug arity", &["-nc", "debug(\"A\";\"B\")"], b""),
        ("debug false", &["-nc", "false|debug"], b""),
        ("debug blank", &["-nc", "\"\"|debug"], b""),
        ("debug newline", &["-nc", "\"a\\nb\"|debug"], b""),
        ("debug nul", &["-nc", "\"a\\u0000b\"|debug"], b""),
        ("debug nan", &["-nc", "nan|debug"], b""),
        ("stderr number", &["-nc", "1|stderr"], b""),
        ("stderr string", &["-c", "stderr"], b"\"hi\"\n"),
        ("stderr pretty", &["stderr"], b"{\"b\":1,\"a\":2}\n"),
        ("stderr compact", &["-c", "stderr"], b"{\"b\":1,\"a\":2}\n"),
        ("stderr sorted", &["-S", "stderr"], b"{\"b\":1,\"a\":2}\n"),
        ("stderr raw", &["-r", "stderr"], b"\"hi\"\n"),
        ("stderr newline", &["-nc", "\"a\\nb\"|stderr"], b""),
        ("stderr quote", &["-nc", "\"say \\\"hi\\\"\"|stderr"], b""),
        ("stderr blank", &["-nc", "\"\"|stderr"], b""),
        ("stderr true", &["-nc", "true|stderr"], b""),
        ("stderr pipe", &["-nc", "1|stderr|.+1"], b""),
        ("stderr two", &["-nc", "(1,\"a\")|stderr"], b""),
        ("stderr nan", &["-nc", "nan|stderr"], b""),
        ("stderr inf", &["-nc", "infinite|stderr"], b""),
        ("stderr nul", &["-nc", "\"a\\u0000b\"|stderr"], b""),
        ("stderr empty", &["-nc", "empty|stderr"], b""),
        ("line none", &["-n", "input_line_number"], b""),
        ("line one", &["-c", "input_line_number"], b"1\n"),
        ("line two", &["-c", "input_line_number"], b"1\n2\n"),
        ("line no nl", &["-c", "input_line_number"], b"1"),
        ("line blank", &["-c", "input_line_number"], b"1\n\n2\n"),
        ("line cut", &["-c", "input_line_number"], b"1\n2"),
        (
            "line object",
            &["-c", ".|input_line_number"],
            b"{\"a\":1,\n\"b\":2}\n3\n",
        ),
        ("line raw", &["-Rc", "input_line_number"], b"a\nb\n"),
        ("line raw cut", &["-Rc", "input_line_number"], b"a\nb"),
        ("line slurp", &["-sc", "input_line_number"], b"1\n2\n"),
        ("line slurp cut", &["-sc", "input_line_number"], b"1\n2"),
        ("line slurp empty", &["-sc", "input_line_number"], b""),
        (
            "line inputs",
            &["-nc", "inputs|input_line_number"],
            b"1\n2\n3\n",
        ),
        (
            "line input",
            &["-c", "input_line_number, input|input_line_number"],
            b"1\n2\n",
        ),
        (
            "line twice",
            &["-c", "input_line_number, input_line_number"],
            b"1\n2\n",
        ),
        (
            "line after inputs",
            &["-nc", "[inputs]|length, input_line_number"],
            b"1\n2\n",
        ),
        (
            "line stream",
            &["-c", "--stream", "input_line_number"],
            b"[1,2]\n",
        ),
        ("line raw slurp", &["-Rsc", "input_line_number"], b"a\nb\n"),
        ("builtins", &["-nc", "builtins"], b""),
        ("builtins length", &["-c", "builtins|length"], b"5\n"),
        ("builtins long", &["-nc", "builtins|length > 10"], b""),
        (
            "builtins arity",
            &["-nc", "\"-1\"|IN(builtins[]/\"/\"|.[1])"],
            b"",
        ),
        (
            "builtins nums",
            &["-nc", "all(builtins[]/\"/\"; .[1]|tonumber >= 0)"],
            b"",
        ),
        (
            "builtins under",
            &["-nc", "builtins|any(.[:1]==\"_\")"],
            b"",
        ),
        (
            "finites",
            &["-nc", "(1,nan,infinite,0,\"a\",null)|finites"],
            b"",
        ),
        (
            "normals",
            &["-nc", "(1,-1,0,nan,infinite,1.5,1e-320)|normals"],
            b"",
        ),
        (
            "isfinite",
            &["-nc", "[1,nan,infinite,0]|map(isfinite)"],
            b"",
        ),
        ("isfinite text", &["-nc", "\"a\"|isfinite"], b""),
        ("isfinite zero", &["-nc", "(-0)|isfinite"], b""),
        ("normals array", &["-nc", "[1,0,nan]|normals"], b""),
        ("prog origin", &["-n", "get_prog_origin"], b""),
    ];
    for (name, args, input) in cases {
        compare_exact(name, args, input);
    }

    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("report-span-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    let prog = dir.join("prog.jq");
    let nested = dir.join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(&prog, "get_prog_origin\n").unwrap();
    fs::write(nested.join("prog.jq"), "get_prog_origin\n").unwrap();
    let left = dir.join("left.json");
    let right = dir.join("right.json");
    let cut = dir.join("cut.json");
    fs::write(&left, b"1\n2\n").unwrap();
    fs::write(&right, b"3\n").unwrap();
    fs::write(&cut, b"1\n2").unwrap();
    compare_exact(
        "prog origin file",
        &["-n", "-f", prog.to_str().unwrap()],
        b"",
    );
    compare_exact(
        "line files",
        &[
            "-c",
            "input_line_number",
            left.to_str().unwrap(),
            right.to_str().unwrap(),
        ],
        b"",
    );
    compare_exact(
        "line slurp files",
        &[
            "-sc",
            "input_line_number",
            left.to_str().unwrap(),
            right.to_str().unwrap(),
        ],
        b"",
    );
    compare_exact(
        "line cut file",
        &["-c", "input_line_number", cut.to_str().unwrap()],
        b"",
    );

    let bin = env!("CARGO_BIN_EXE_tumulus");
    let origin = Path::new(bin)
        .parent()
        .and_then(|dir| dir.to_str())
        .expect("binary directory");
    let got = run(bin, &["-n".to_string(), "get_jq_origin".to_string()], b"");
    let quoted = format!("\"{origin}\"\n");
    assert_eq!(
        got.stdout,
        quoted.as_bytes(),
        "get_jq_origin should be the directory of the invoked binary"
    );
    assert!(got.stderr.is_empty());
    assert_eq!(got.status.code(), Some(0));
    let _ = fs::remove_dir_all(&dir);
}

/// `reverse`, `in`, `bsearch`, `JOIN`, `format`, `fromdateiso8601`, and `todateiso8601`.
#[test]
fn search_formats_match_jq() {
    require_jq();
    let index = r#"{"0":[0,"abc"],"1":[1,"bcd"],"2":[2,"def"],"3":[3,"efg"],"4":[4,"fgh"]}"#;
    let rows = r#"[[5,"foo"],[3,"bar"],[1,"foobar"]]"#;
    let join2 = format!("{rows}|JOIN({index}; .[0]|tostring)");
    let join3 = format!("{rows}|JOIN({index}; .[]; .[0]|tostring)");
    let join4 = format!("{rows}|JOIN({index}; .[]; .[0]|tostring; .[1])");
    let join_empty = format!("[]|JOIN({index}; .[0]|tostring)");
    let join_expr = format!("{rows}|JOIN({index}; .[]; .[0]|tostring; empty)");
    let join3_empty = format!("[]|JOIN({index}; .[]; .[0]|tostring)");
    let cases: Vec<(&str, Vec<&str>, &[u8])> = vec![
        ("rev arr", vec!["-c", "reverse"], b"[1,2,3]\n"),
        ("rev empty", vec!["-c", "reverse"], b"[]\n"),
        ("rev null", vec!["-c", "reverse"], b"null\n"),
        ("rev blank", vec!["-c", "reverse"], b"\"\"\n"),
        ("rev str", vec!["-c", "reverse"], b"\"abc\"\n"),
        ("rev num", vec!["-c", "reverse"], b"1\n"),
        ("rev obj empty", vec!["-c", "reverse"], b"{}\n"),
        ("rev obj", vec!["-c", "reverse"], b"{\"a\":1}\n"),
        ("rev bool", vec!["-c", "reverse"], b"true\n"),
        ("rev nest", vec!["-c", "reverse"], b"[[1,2],[3]]\n"),
        ("rev one", vec!["-c", "reverse"], b"[1]\n"),
        ("rev then ok", vec!["-c", "reverse"], b"1\n[1,2]\n"),
        ("rev arity", vec!["-nc", "reverse(1)"], b""),
        (
            "rev shadow len",
            vec!["-nc", "def length: 1; [1,2,3]|reverse"],
            b"",
        ),
        (
            "rev shadow",
            vec!["-nc", "def reverse: 9; [1,2]|reverse"],
            b"",
        ),
        (
            "rev path",
            vec!["-nc", "try ([0,1,2]|def x: reverse; x=10) catch ."],
            b"",
        ),
        ("in hit", vec!["-nc", "2|in([0,1,2,3])"], b""),
        ("in miss", vec!["-nc", "9|in([0,1,2])"], b""),
        ("in key", vec!["-nc", "\"a\"|in({a:1})"], b""),
        ("in missing key", vec!["-nc", "\"z\"|in({a:1})"], b""),
        ("in number key", vec!["-nc", "1|in({a:1})"], b""),
        ("in bool key", vec!["-nc", "true|in({true:1})"], b""),
        ("in string", vec!["-nc", "0|in(\"ab\")"], b""),
        ("in number", vec!["-nc", "0|in(1)"], b""),
        ("in null", vec!["-nc", "0|in(null)"], b""),
        ("in neg", vec!["-nc", "(-1)|in([1,2,3])"], b""),
        ("in empty", vec!["-nc", "1|in(empty)"], b""),
        ("in stream", vec!["-nc", "1|in([0,1],[2])"], b""),
        ("in arity", vec!["-nc", "in"], b""),
        ("in arity two", vec!["-nc", "in(1;2)"], b""),
        ("in string idx", vec!["-nc", "\"0\"|in([0,1])"], b""),
        ("in zero", vec!["-nc", "0|in([0,1])"], b""),
        ("in null key", vec!["-nc", "null|in([null])"], b""),
        ("in stops", vec!["-nc", "try (1|in(1, [1])) catch ."], b""),
        (
            "in shadow has",
            vec!["-nc", "def has(x): 9; 0|in([0])"],
            b"",
        ),
        ("in shadow", vec!["-nc", "def in(x): 9; 0|in([0])"], b""),
        ("bs stream", vec!["-nc", "[1,2,3]|bsearch(0,2,4)"], b""),
        ("bs empty", vec!["-nc", "[]|bsearch(1)"], b""),
        ("bs one", vec!["-nc", "[5]|bsearch(5)"], b""),
        ("bs low", vec!["-nc", "[5]|bsearch(1)"], b""),
        ("bs high", vec!["-nc", "[5]|bsearch(9)"], b""),
        ("bs ends", vec!["-nc", "[1,2,3,4]|bsearch(1,4)"], b""),
        ("bs mid", vec!["-nc", "[1,3,5,7,9]|bsearch(5)"], b""),
        ("bs dups", vec!["-nc", "[1,2,2,2,3]|bsearch(2)"], b""),
        ("bs unsorted", vec!["-nc", "[3,1,2]|bsearch(1)"], b""),
        (
            "bs range",
            vec!["-nc", "[range(20)]|bsearch(0,7,19,20,-1)"],
            b"",
        ),
        ("bs number", vec!["-nc", "1|bsearch(1)"], b""),
        ("bs obj empty", vec!["-nc", "{}|bsearch(1)"], b""),
        ("bs obj", vec!["-nc", "{a:1}|bsearch(1)"], b""),
        ("bs str", vec!["-nc", "\"abc\"|bsearch(\"b\")"], b""),
        ("bs blank", vec!["-nc", "\"\"|bsearch(1)"], b""),
        ("bs arity", vec!["-nc", "bsearch"], b""),
        ("bs arity two", vec!["-nc", "bsearch(1;2)"], b""),
        ("bs none", vec!["-nc", "[1,2,3]|bsearch(empty)"], b""),
        (
            "bs text",
            vec!["-nc", "[\"a\",\"b\",\"c\"]|bsearch(\"b\")"],
            b"",
        ),
        ("bs null", vec!["-nc", "[null,1]|bsearch(null)"], b""),
        ("bs nan", vec!["-nc", "[1,2,3]|bsearch(nan)"], b""),
        ("bs before", vec!["-nc", "[2,4,6]|bsearch(1)"], b""),
        ("bs between", vec!["-nc", "[2,4,6]|bsearch(3)"], b""),
        ("bs after", vec!["-nc", "[2,4,6]|bsearch(7)"], b""),
        (
            "bs shadow floor",
            vec!["-nc", "def floor: 9; [1,2,3]|bsearch(2)"],
            b"",
        ),
        (
            "bs shadow",
            vec!["-nc", "def bsearch(x): 9; [1,2,3]|bsearch(2)"],
            b"",
        ),
        ("join2", vec!["-nc", join2.as_str()], b""),
        ("join3", vec!["-nc", join3.as_str()], b""),
        ("join4", vec!["-nc", join4.as_str()], b""),
        ("join empty", vec!["-nc", join_empty.as_str()], b""),
        ("join3 empty", vec!["-nc", join3_empty.as_str()], b""),
        ("join null", vec!["-nc", "null|JOIN({a:1}; .[0])"], b""),
        ("join num", vec!["-nc", "1|JOIN(1; \"a\")"], b""),
        (
            "join bad index",
            vec!["-nc", "[[1]]|JOIN(1; .[0]|tostring)"],
            b"",
        ),
        ("join arity", vec!["-nc", "JOIN"], b""),
        ("join arity one", vec!["-nc", "JOIN(1)"], b""),
        ("join arity five", vec!["-nc", "JOIN(1;2;3;4;5)"], b""),
        (
            "join stream",
            vec!["-nc", "[1,2]|JOIN({a:1}; \"a\",\"b\")"],
            b"",
        ),
        ("join missing", vec!["-nc", "JOIN({a:1}; \"x\"; .)"], b""),
        (
            "join four id",
            vec!["-nc", "JOIN({a:[1]}; 1,2; \"a\"; .)"],
            b"",
        ),
        (
            "join shadow",
            vec!["-nc", "def JOIN(a;b): 9; [1]|JOIN(1; 0)"],
            b"",
        ),
        (
            "join fn",
            vec!["-nc", "def f: \"a\"; [\"x\"]|JOIN({a:7}; f)"],
            b"",
        ),
        ("join empty expr", vec!["-nc", join_expr.as_str()], b""),
        (
            "join err",
            vec!["-nc", "try JOIN({a:1}; error(\"z\"); .) catch ."],
            b"",
        ),
        (
            "join string key",
            vec!["-nc", "[\"x\",\"y\"]|JOIN([\"A\",\"B\"]; .)"],
            b"",
        ),
        ("iso zero", vec!["-nc", "0|todateiso8601"], b""),
        (
            "iso read",
            vec!["-nc", "\"1970-01-01T00:00:00Z\"|fromdateiso8601"],
            b"",
        ),
        ("iso frac", vec!["-nc", "1.5|todateiso8601"], b""),
        ("iso neg", vec!["-nc", "(-1)|todateiso8601"], b""),
        ("iso nan", vec!["-nc", "nan|todateiso8601"], b""),
        ("iso inf", vec!["-nc", "infinite|todateiso8601"], b""),
        ("iso text", vec!["-nc", "\"x\"|todateiso8601"], b""),
        ("iso null", vec!["-nc", "null|fromdateiso8601"], b""),
        ("iso bad", vec!["-nc", "\"nope\"|fromdateiso8601"], b""),
        (
            "iso bare",
            vec!["-nc", "\"1970-01-01T00:00:00\"|fromdateiso8601"],
            b"",
        ),
        (
            "iso fraction",
            vec!["-nc", "\"1970-01-01T00:00:01.5Z\"|fromdateiso8601"],
            b"",
        ),
        ("iso arity", vec!["-nc", "todateiso8601(1)"], b""),
        (
            "iso round",
            vec!["-nc", "1234567890|todateiso8601|fromdateiso8601"],
            b"",
        ),
        ("iso known", vec!["-nc", "1425599507|todateiso8601"], b""),
        (
            "iso shadow",
            vec!["-nc", "def strftime(f): \"no\"; 0|todateiso8601"],
            b"",
        ),
        (
            "iso shadow self",
            vec![
                "-nc",
                "def fromdateiso8601: 9; \"1970-01-01T00:00:00Z\"|fromdateiso8601",
            ],
            b"",
        ),
        ("fmt both", vec!["-nc", "1|format(\"json\",\"text\")"], b""),
        ("fmt empty", vec!["-nc", "1|format(empty)"], b""),
        ("fmt json str", vec!["-nc", "\"hi\"|format(\"json\")"], b""),
        ("fmt text", vec!["-nc", "\"hi\"|format(\"text\")"], b""),
        ("fmt html", vec!["-nc", "\"a<b>&c\"|format(\"html\")"], b""),
        ("fmt uri", vec!["-nc", "\"a b\"|format(\"uri\")"], b""),
        ("fmt b64", vec!["-nc", "\"hi\"|format(\"base64\")"], b""),
        ("fmt b64d", vec!["-nc", "\"aGk=\"|format(\"base64d\")"], b""),
        (
            "fmt b64 bad",
            vec!["-nc", "\"a<b>\"|format(\"base64d\")"],
            b"",
        ),
        ("fmt csv", vec!["-nc", "[\"a\",\"b\"]|format(\"csv\")"], b""),
        ("fmt tsv", vec!["-nc", "[\"a\",\"b\"]|format(\"tsv\")"], b""),
        ("fmt sh", vec!["-nc", "[\"a\",\"b c\"]|format(\"sh\")"], b""),
        ("fmt sh num", vec!["-nc", "1|format(\"sh\")"], b""),
        ("fmt sh obj", vec!["-nc", "{a:1}|format(\"sh\")"], b""),
        ("fmt csv str", vec!["-nc", "\"a\"|format(\"csv\")"], b""),
        ("fmt csv null", vec!["-nc", "[null]|format(\"csv\")"], b""),
        ("fmt csv nan", vec!["-nc", "[nan]|format(\"csv\")"], b""),
        ("fmt csv obj", vec!["-nc", "[{a:1}]|format(\"csv\")"], b""),
        ("fmt nope", vec!["-nc", "1|format(\"nope\")"], b""),
        ("fmt case", vec!["-nc", "1|format(\"JSON\")"], b""),
        ("fmt blank", vec!["-nc", "1|format(\"\")"], b""),
        (
            "fmt long",
            vec!["-nc", "1|format(\"abcdefghijklmnop\")"],
            b"",
        ),
        ("fmt number", vec!["-nc", "1|format(1)"], b""),
        ("fmt null", vec!["-nc", "1|format(null)"], b""),
        ("fmt true", vec!["-nc", "1|format(true)"], b""),
        ("fmt false", vec!["-nc", "1|format(false)"], b""),
        ("fmt nan", vec!["-nc", "1|format(nan)"], b""),
        ("fmt inf", vec!["-nc", "1|format(infinite)"], b""),
        ("fmt neg", vec!["-nc", "1|format(-1.5)"], b""),
        ("fmt obj", vec!["-nc", "1|format({a:1})"], b""),
        (
            "fmt arr",
            vec!["-nc", "1|format([\"abcdefghijklmnop\"])"],
            b"",
        ),
        (
            "fmt partial",
            vec!["-nc", "[1,2]|format(\"csv\",\"nope\")"],
            b"",
        ),
        (
            "fmt partial type",
            vec!["-nc", "[1,2]|format(\"csv\",1)"],
            b"",
        ),
        (
            "fmt later",
            vec!["-nc", "1|format(\"json\",\"text\",\"nope\")"],
            b"",
        ),
        ("fmt stop", vec!["-nc", "1|format(\"nope\",\"json\")"], b""),
        (
            "fmt arg err",
            vec!["-nc", "1|format(\"json\", error(\"z\"))"],
            b"",
        ),
        (
            "fmt skip debug",
            vec!["-nc", "1|format(\"nope\", debug(\"x\"))"],
            b"",
        ),
        (
            "fmt then debug",
            vec!["-nc", "1|format(\"json\", debug(\"x\"))"],
            b"",
        ),
        (
            "fmt try",
            vec!["-nc", "try ([1,2]|format(\"csv\",\"nope\")) catch ."],
            b"",
        ),
        ("fmt dot", vec!["-nc", "\"text\"|format(.)"], b""),
        ("fmt arity", vec!["-nc", "format"], b""),
        (
            "fmt arity two",
            vec!["-nc", "format(\"json\"; \"text\")"],
            b"",
        ),
        (
            "fmt shadow",
            vec!["-nc", "def format(a): 7; 1|format(\"json\")"],
            b"",
        ),
        ("fmt inputs", vec!["-nc", "(1,[1])|format(\"csv\")"], b""),
        ("fmt raw", vec!["-ncr", "1|format(\"text\")"], b""),
        ("fmt raw json", vec!["-ncr", "\"hi\"|format(\"json\")"], b""),
        ("fmt join out", vec!["-ncj", "1|format(\"json\")"], b""),
        ("fmt ascii", vec!["-nca", "\"é<\"|format(\"html\")"], b""),
        ("fmt sort", vec!["-ncS", "{b:1,a:2}|format(\"json\")"], b""),
        ("fmt two bad", vec!["-c", "format(\"nope\")"], b"1\n2\n"),
        ("fmt files", vec!["-c", "format(\"json\")"], b"1\n[2]\n"),
    ];
    for (name, args, input) in &cases {
        compare_exact(name, args, input);
    }
}

/// `import`, `include`, `-L`, `~/.jq`, `modulemeta`, and `get_search_list`.
#[test]
fn modules_match_jq() {
    require_jq();
    let plain = [
        ("search", &["-nc", "get_search_list"][..], b""),
        ("search one", &["-nc", "1|get_search_list"], b""),
        ("search none", &["-nc", "empty|get_search_list"], b""),
        ("search arity", &["-n", "get_search_list(1)"], b""),
        ("meta arity", &["-n", "modulemeta(1)"], b""),
        ("meta null", &["-nc", "null|modulemeta"], b""),
        ("L bare", &["-n", "-L"], b""),
        ("no program", &["-n", "def f: 1;"], b""),
        ("mod only", &["-n", "module {\"a\":1};"], b""),
        ("mod null", &["-n", "module null; 0"], b""),
        ("mod text", &["-n", "module \"x\"; 0"], b""),
        ("mod dot", &["-n", "module {\"a\":.}; 0"], b""),
        ("mod plus", &["-n", "module (.+1); 0"], b""),
        ("mod array", &["-n", "module []; 0"], b""),
        ("inc plus", &["-n", "include \"a\" (.+1); 0"], b""),
        ("inc array", &["-n", "include \"a\" []; 0"], b""),
        ("inc escape", &["-n", "include \"\\ \"; 0"], b""),
        ("inc interp", &["-n", "include \"\\(a)\"; 0"], b""),
        ("inc letter", &["-n", "include \"\\u0041\"; 0"], b""),
        ("parent", &["-n", "import \"a/../b\" as b; 1"], b""),
        ("not defined as", &["-nc", "1 as $n | $n::n"], b""),
    ];
    for (name, args, input) in plain {
        compare_exact(name, args, input);
    }

    let (dir, _guard) = scratch("modules");
    let lib = dir.join("lib");
    let home = dir.join("home");
    let work = dir.join("work");
    fs::create_dir_all(lib.join("pkg")).unwrap();
    fs::create_dir_all(home.join(".jq")).unwrap();
    fs::create_dir_all(&work).unwrap();
    fs::write(lib.join("foo.jq"), "def f: 1; def g($x): $x+1;\n").unwrap();
    fs::write(lib.join("bar.jq"), "def f: 2;\n").unwrap();
    fs::write(lib.join("foo.json"), "{\"from\":\"json\"}\n").unwrap();
    fs::write(lib.join("num.json"), "1\n2\n").unwrap();
    fs::write(lib.join("bad.json"), "{not json\n").unwrap();
    fs::write(lib.join("pkg").join("pkg.jq"), "def p: 4;\n").unwrap();
    fs::write(lib.join("sh.jq"), "def f: 1; def g: f;\n").unwrap();
    fs::write(
        lib.join("meta.jq"),
        "module {\"name\":\"meta\",\"version\":\"1.2\"}; def a: 1;\n",
    )
    .unwrap();
    fs::write(
        lib.join("importer.jq"),
        "import \"foo\" as foo {\"extra\":true}; import \"num\" as $n; def use: foo::f; def see: $n;\n",
    )
    .unwrap();
    fs::write(
        lib.join("dot.jq"),
        "import \"./foo\" as foo; def k: foo::f;\n",
    )
    .unwrap();
    fs::write(
        lib.join("datause.jq"),
        "import \"num\" as $n; def f: $n; def g: $n::n;\n",
    )
    .unwrap();
    fs::write(lib.join("miss.jq"), "import \"nope\" as n; def a: 1;\n").unwrap();
    fs::write(
        lib.join("over.jq"),
        "module {\"name\":\"o\",\"deps\":[9],\"extra\":1}; def a: 1;\n",
    )
    .unwrap();
    fs::write(home.join(".jq").join("homeonly.jq"), "def h: \"home\";\n").unwrap();
    fs::write(work.join("sib.jq"), "def v: 7;\n").unwrap();
    let lib_path = lib.to_str().unwrap();
    let cases = [
        ("import", format!("import \"foo\" as foo; foo::f")),
        ("call", format!("import \"foo\" as foo; foo::g(3)")),
        ("hidden", format!("import \"foo\" as foo; f")),
        ("include", format!("include \"foo\"; g(4)")),
        ("shadow", format!("include \"sh\"; def f: 9; [f,g]")),
        (
            "replace",
            format!("import \"foo\" as f; import \"bar\" as f; f::f"),
        ),
        (
            "redef",
            format!("import \"foo\" as foo; def foo::f: 9; foo::f"),
        ),
        ("data", format!("import \"num\" as $n; $n")),
        ("colon", format!("import \"num\" as $n; $n::n")),
        ("colon bad", format!("import \"num\" as $n; $n::x")),
        ("json", format!("import \"foo\" as $f; $f")),
        ("in def", format!("import \"num\" as $n; def f: $n; f")),
        ("mod data", format!("import \"datause\" as d; [d::f, d::g]")),
        ("pkg", format!("import \"pkg\" as p; p::p")),
        ("dot", format!("import \"dot\" as d; d::k")),
        ("bad", format!("import \"bad\" as $d; $d")),
        ("missing", format!("import \"no-such\" as m; 1")),
        ("meta", format!("\"meta\"|modulemeta")),
        ("deps", format!("\"importer\"|modulemeta")),
        ("miss meta", format!("\"miss\"|modulemeta")),
        ("over", format!("\"over\"|modulemeta")),
        ("nope", format!("\"nope\"|modulemeta")),
        ("try nope", format!("try (\"nope\"|modulemeta) catch .")),
        ("two", format!("(\"foo\",\"bar\")|modulemeta")),
    ];
    for (name, program) in cases {
        compare_module(name, &dir, &home, &["-nc", "-L", lib_path, &program], b"");
    }
    compare_module(
        "home",
        &dir,
        &home,
        &["-n", "import \"homeonly\" as h; h::h"],
        b"",
    );
    compare_module(
        "hidden home",
        &dir,
        &home,
        &["-n", "-L", lib_path, "import \"homeonly\" as h; h::h"],
        b"",
    );
    compare_module(
        "cwd",
        &work,
        &home,
        &["-n", "-L", lib_path, "import \"sib\" as s; s::v"],
        b"",
    );
    compare_module(
        "rel search",
        &dir,
        &home,
        &["-nc", "-L", "lib", "get_search_list"],
        b"",
    );
    compare_module(
        "missing dir",
        &dir,
        &home,
        &["-nc", "-L", "no/such", "get_search_list"],
        b"",
    );
    compare_module(
        "no home",
        &dir,
        &home,
        &["-n", "import \"homeonly\" as h; h::h"],
        b"",
    );
}

fn compare_module(name: &str, cwd: &Path, home: &Path, args: &[&str], input: &[u8]) {
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let owned: Vec<String> = args.iter().copied().map(str::to_string).collect();
    let go = |program: &str, drop_home: bool| {
        let mut cmd = Command::new(program);
        cmd.args(&owned)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if drop_home {
            cmd.env_remove("HOME");
        } else {
            cmd.env("HOME", home);
        }
        let mut child = cmd
            .spawn()
            .unwrap_or_else(|err| panic!("{name}: start {program}: {err}"));
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input);
        }
        child
            .wait_with_output()
            .unwrap_or_else(|err| panic!("{name}: wait {program}: {err}"))
    };
    let drop_home = name == "no home" || name == "no home search";
    let got = go(bin, drop_home);
    let expected = go("jq", drop_home);
    assert_eq!(
        got.stdout,
        expected.stdout,
        "{name}: stdout\nours {}\njq {}",
        String::from_utf8_lossy(&got.stdout),
        String::from_utf8_lossy(&expected.stdout)
    );
    assert_eq!(
        got.stderr,
        expected.stderr,
        "{name}: stderr\nours {}\njq {}",
        String::from_utf8_lossy(&got.stderr),
        String::from_utf8_lossy(&expected.stderr)
    );
    assert_eq!(
        got.status.code(),
        expected.status.code(),
        "{name}: status\n{}",
        String::from_utf8_lossy(&got.stderr)
    );
}

#[test]
fn search_key_matches_jq() {
    require_jq();
    let (dir, _guard) = scratch("searchkey");
    let home = dir.join("home");
    let root = dir.join("root");
    let sdir = root.join("sdir");
    let sother = root.join("sother");
    let prog = root.join("prog");
    let away = root.join("away");
    let decoy = root.join("decoy");
    let decoy_nest = decoy.join("nested");
    let real = root.join("real");
    let real_sub = real.join("sub");
    for path in [
        home.join(".jq"),
        sdir.join("pkg"),
        sdir.join("foo"),
        sother.clone(),
        prog.join("nest"),
        prog.join("elsewhere"),
        away.clone(),
        decoy_nest.clone(),
        decoy.join("nest"),
        decoy.join("sother"),
        decoy.join("elsewhere"),
        real_sub.clone(),
    ] {
        fs::create_dir_all(&path).unwrap();
    }
    fs::write(home.join(".jq").join("foo.jq"), "def h: \"fromhome\";\n").unwrap();
    fs::write(home.join(".jq").join("homeonly.jq"), "def h: \"home\";\n").unwrap();
    fs::write(root.join("foo.jq"), "def f: \"root\";\n").unwrap();
    fs::write(sdir.join("foo.jq"), "def f: \"sdir\";\n").unwrap();
    fs::write(sdir.join("foo").join("foo.jq"), "def f: \"dirform\";\n").unwrap();
    fs::write(sdir.join("num.json"), "1\n").unwrap();
    fs::write(sdir.join("pkg").join("pkg.jq"), "def p: \"pkg\";\n").unwrap();
    fs::write(sdir.join("onlycwd.jq"), "def z: 1;\n").unwrap();
    fs::write(
        sdir.join("uses.jq"),
        "import \"side\" as s {search: \"../sother\"}; def k: s::f;\n",
    )
    .unwrap();
    fs::write(
        sdir.join("lookdot.jq"),
        "import \"foo\" as foo {search: \".\"}; def k: foo::f;\n",
    )
    .unwrap();
    fs::write(
        sdir.join("looksl.jq"),
        "import \"foo\" as foo {search: \"./\"}; def k: foo::f;\n",
    )
    .unwrap();
    fs::write(
        sdir.join("mid.jq"),
        "import \"foo\" as foo {search: \".\", extra: 1}; def k: foo::f;\n",
    )
    .unwrap();
    fs::write(
        sdir.join("incm.jq"),
        "include \"foo\" {search: \"./\", extra: 1}; def k: f;\n",
    )
    .unwrap();
    fs::write(
        sdir.join("arr.jq"),
        "import \"foo\" as foo {search: [\"../sdir\", \"../sother\"]}; def k: foo::f;\n",
    )
    .unwrap();
    fs::write(sother.join("foo.jq"), "def f: \"sother\";\n").unwrap();
    fs::write(sother.join("side.jq"), "def f: \"side\";\n").unwrap();
    fs::write(sother.join("num.json"), "9\n").unwrap();
    fs::write(
        decoy.join("sother").join("side.jq"),
        "def f: \"decoyside\";\n",
    )
    .unwrap();
    fs::write(decoy.join("nest").join("foo.jq"), "def f: \"decoynest\";\n").unwrap();
    fs::write(
        decoy.join("elsewhere").join("foo.jq"),
        "def f: \"elsewhere\";\n",
    )
    .unwrap();
    fs::write(prog.join("foo.jq"), "def f: \"prog\";\n").unwrap();
    fs::write(prog.join("num.json"), "9\n").unwrap();
    fs::write(prog.join("nest").join("foo.jq"), "def f: \"nest\";\n").unwrap();
    fs::write(away.join("foo.jq"), "def f: \"away\";\n").unwrap();
    fs::write(
        prog.join("main.jq"),
        "import \"foo\" as foo {search: \"../sdir\"}; foo::f\n",
    )
    .unwrap();
    fs::write(
        prog.join("dot.jq"),
        "import \"foo\" as foo {search: \".\"}; foo::f\n",
    )
    .unwrap();
    fs::write(
        prog.join("sl.jq"),
        "import \"foo\" as foo {search: \"./\"}; foo::f\n",
    )
    .unwrap();
    fs::write(
        prog.join("em.jq"),
        "import \"foo\" as foo {search: \"\"}; foo::f\n",
    )
    .unwrap();
    fs::write(
        prog.join("nestp.jq"),
        "import \"foo\" as foo {search: \"nest\"}; foo::f\n",
    )
    .unwrap();
    fs::write(prog.join("inc.jq"), "include \"foo\" {search: \"./\"}; f\n").unwrap();
    fs::write(
        prog.join("data.jq"),
        "import \"num\" as $n {search: \"./\"}; $n\n",
    )
    .unwrap();
    fs::write(
        prog.join("dataup.jq"),
        "import \"num\" as $n {search: \"../sdir\"}; $n\n",
    )
    .unwrap();
    fs::write(
        prog.join("arr.jq"),
        "import \"foo\" as foo {search: [\".\", \"nest\"]}; foo::f\n",
    )
    .unwrap();
    fs::write(
        prog.join("arr2.jq"),
        "import \"foo\" as foo {search: [\"elsewhere\", \"nest\"]}; foo::f\n",
    )
    .unwrap();
    fs::write(
        prog.join("elsep.jq"),
        "import \"foo\" as foo {search: \"elsewhere\"}; foo::f\n",
    )
    .unwrap();
    fs::write(real.join("foo.jq"), "def f: \"real\";\n").unwrap();
    fs::write(real_sub.join("foo.jq"), "def f: \"sub\";\n").unwrap();
    fs::write(
        real_sub.join("up.jq"),
        "import \"foo\" as foo {search: \"..\"}; foo::f\n",
    )
    .unwrap();
    std::os::unix::fs::symlink("real/sub", root.join("linksub")).unwrap();
    std::os::unix::fs::symlink("real/sub/up.jq", root.join("up-link.jq")).unwrap();

    let cli = [
        ("sdir", r#"import "foo" as foo {search: "sdir"}; foo::f"#),
        (
            "sother",
            r#"import "foo" as foo {search: "sother"}; foo::f"#,
        ),
        ("dot", r#"import "foo" as foo {search: "."}; foo::f"#),
        ("slash", r#"import "foo" as foo {search: "./"}; foo::f"#),
        ("empty", r#"import "foo" as foo {search: ""}; foo::f"#),
        ("dotdot", r#"import "foo" as foo {search: "./."}; foo::f"#),
        ("include", r#"include "foo" {search: "sdir"}; f"#),
        ("data", r#"import "num" as $n {search: "sdir"}; $n"#),
        (
            "missing",
            r#"import "foo" as foo {search: "missing"}; foo::f"#,
        ),
        ("number", r#"import "foo" as foo {search: 1}; foo::f"#),
        ("null", r#"import "foo" as foo {search: null}; foo::f"#),
        ("empty arr", r#"import "foo" as foo {search: []}; foo::f"#),
        ("false", r#"import "foo" as foo {search: false}; foo::f"#),
        (
            "object",
            r#"import "foo" as foo {search: {"a": 1}}; foo::f"#,
        ),
        (
            "arr skip",
            r#"import "foo" as foo {search: [1, null, false, "sdir"]}; foo::f"#,
        ),
        (
            "arr first",
            r#"import "foo" as foo {search: ["sother", "sdir"]}; foo::f"#,
        ),
        (
            "arr second",
            r#"import "foo" as foo {search: ["missing", "sdir"]}; foo::f"#,
        ),
        (
            "up same",
            r#"import "foo" as foo {search: "sdir/../sdir"}; foo::f"#,
        ),
        ("tilde", r#"import "homeonly" as h {search: "~/.jq"}; h::h"#),
        (
            "dollar",
            r#"import "foo" as foo {search: "$ORIGIN"}; foo::f"#,
        ),
        ("pkg", r#"import "pkg" as p {search: "sdir"}; p::p"#),
        ("escape", r#"import "a/../b" as b {search: "sdir"}; 1"#),
        ("no key", r#"import "foo" as foo; foo::f"#),
    ];
    for (name, program) in cli {
        compare_module(name, &root, &home, &["-n", program], b"");
    }
    let sdir_s = sdir.to_str().unwrap().to_string();
    let sother_s = sother.to_str().unwrap().to_string();
    compare_module(
        "search beats -L",
        &root,
        &home,
        &[
            "-n",
            "-L",
            &sother_s,
            r#"import "foo" as foo {search: "sdir"}; foo::f"#,
        ],
        b"",
    );
    compare_module(
        "cwd beats -L",
        &root,
        &home,
        &["-n", "-L", &sdir_s, r#"import "foo" as foo; foo::f"#],
        b"",
    );
    let absolute = format!(
        "import \"foo\" as foo {{search: \"{}\"}}; foo::f",
        sdir.display()
    );
    compare_module("absolute", &away, &home, &["-n", &absolute], b"");
    compare_module(
        "cli up from prog",
        &prog,
        &home,
        &["-n", r#"import "foo" as foo {search: "../sdir"}; foo::f"#],
        b"",
    );
    compare_module(
        "cli up from root",
        &root,
        &home,
        &["-n", r#"import "foo" as foo {search: "../sdir"}; foo::f"#],
        b"",
    );
    compare_module(
        "nested side",
        &decoy_nest,
        &home,
        &["-n", r#"import "uses" as u {search: "../../sdir"}; u::k"#],
        b"",
    );
    compare_module(
        "nested dot",
        &away,
        &home,
        &["-n", r#"import "lookdot" as m {search: "../sdir"}; m::k"#],
        b"",
    );
    compare_module(
        "nested slash",
        &away,
        &home,
        &["-n", r#"import "looksl" as m {search: "../sdir"}; m::k"#],
        b"",
    );

    let files = [
        "main.jq",
        "dot.jq",
        "sl.jq",
        "em.jq",
        "nestp.jq",
        "inc.jq",
        "data.jq",
        "dataup.jq",
        "arr.jq",
        "arr2.jq",
        "elsep.jq",
    ];
    for cwd in [&root, &prog, &away, &decoy] {
        for file in files {
            let path = prog.join(file);
            let text = path.to_str().unwrap().to_string();
            let label = format!("f {file} {}", cwd.file_name().unwrap().to_string_lossy());
            compare_module(&label, cwd, &home, &["-n", "-f", &text], b"");
        }
    }
    compare_module(
        "f rel slash",
        &root,
        &home,
        &["-n", "-f", "prog/sl.jq"],
        b"",
    );
    compare_module(
        "f rel dot",
        &away,
        &home,
        &["-n", "-f", "../prog/dot.jq"],
        b"",
    );

    compare_module(
        "meta mid",
        &away,
        &home,
        &["-nc", "-L", &sdir_s, r#""mid"|modulemeta"#],
        b"",
    );
    compare_module(
        "meta inc",
        &away,
        &home,
        &["-nc", "-L", &sdir_s, r#""incm"|modulemeta"#],
        b"",
    );
    compare_module(
        "meta arr",
        &away,
        &home,
        &["-nc", "-L", &sdir_s, r#""arr"|modulemeta"#],
        b"",
    );
    compare_module(
        "meta cwd skipped",
        &sdir,
        &home,
        &["-nc", r#""onlycwd"|modulemeta"#],
        b"",
    );
    compare_module(
        "meta home not cwd",
        &sdir,
        &home,
        &["-nc", r#""foo"|modulemeta"#],
        b"",
    );
    compare_module(
        "import still sees cwd",
        &sdir,
        &home,
        &["-n", r#"import "onlycwd" as o; o::z"#],
        b"",
    );

    let link = root.join("linksub").join("up.jq");
    let link_s = link.to_str().unwrap().to_string();
    compare_module("sym dir", &root, &home, &["-n", "-f", &link_s], b"");
    let up = root.join("up-link.jq");
    let up_s = up.to_str().unwrap().to_string();
    compare_module("sym file", &root, &home, &["-n", "-f", &up_s], b"");
    compare_module(
        "no home search",
        &away,
        &home,
        &["-n", r#"import "foo" as foo {search: "~/.jq"}; foo::f"#],
        b"",
    );
}
