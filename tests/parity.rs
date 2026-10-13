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
        "1<2<3", "1==1==1", "{a:1+2}", "not true", "not(1)", "1++2", ".a=.b=1", ".a|=.|=.",
        ".a+=1+=2",
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
        "empty(1)",
        "empty (1)",
        "empty(1;2)",
        "empty .",
        "length()",
        "length(1)",
        "length (1)",
        "length(1;2)",
        "length .",
        "length?(1)",
        "keys()",
        "keys(1)",
        "keys (1)",
        "(length)(1)",
        "(empty)(1)",
        "has",
        "has()",
        "has(1;2)",
        "select",
        "select()",
        "select(1;2)",
        "map",
        "map()",
        "map(1;2)",
        ". | map",
        "map(.+1)(2)",
        "(map(.+1))(2)",
        "lengthx",
        "hasx(1)",
        "todate()",
        "todate(1)",
        "todate ()",
        "fromdate()",
        "fromdate(1)",
        "gmtime()",
        "gmtime(1)",
        "mktime()",
        "mktime(1)",
        "(todate)(0)",
        "(fromdate)(\"\")",
        "(gmtime)(0)",
        "(mktime)(0)",
        "strftime",
        "strftime()",
        "strftime ()",
        "strftime(1;2)",
        "(strftime(\"%Y\"))(0)",
        "strptime",
        "strptime()",
        "strptime ()",
        "strptime(1;2)",
        "(strptime(\"%Y\"))(0)",
        "now()",
        "now(1)",
        "now ()",
        "(now)(0)",
        "localtime()",
        "localtime(1)",
        "localtime ()",
        "(localtime)(0)",
        "strflocaltime",
        "strflocaltime()",
        "strflocaltime ()",
        "strflocaltime(1;2)",
        "(strflocaltime(\"%Y\"))(0)",
        "todateiso8601",
        "fromdateiso8601",
        "error()",
        "error( )",
        "error(1;2)",
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
        "tryerror",
        "try errorcatch",
        "reduce try 1 catch 2 as $x (0; .+$x)",
        "try (1)(2)",
        "try (1)(2) catch 3",
        "try length(1)",
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
    }
}

/// jq aborts when `strftime` is given a number and a format that is not a
/// string. The message here is the one jq prints for an array. Letters
/// other than the ones these filters accept are not compared with jq.
#[test]
fn strftime_letters_are_limited() {
    let cases = [
        (r#"0 | strftime("%A")"#, "strftime does not support %A"),
        (r#"0 | strftime("%B")"#, "strftime does not support %B"),
        (r#"0 | strftime("%%")"#, "strftime does not support %%"),
        (r#"0 | strftime("%")"#, "strftime does not support %"),
        (r#"0 | strftime("%Y%")"#, "strftime does not support %"),
        (r#"0 | strftime("%n")"#, "strftime does not support %n"),
        (r#"0 | strftime("%t")"#, "strftime does not support %t"),
        (r#"0 | strftime("%EY")"#, "strftime does not support %E"),
        (r#"0 | strftime("%-Y")"#, "strftime does not support %-"),
        (r#"0 | strftime("%10Y")"#, "strftime does not support %1"),
        (r#"0 | strftime("%Q%Y")"#, "strftime does not support %Q"),
        (
            r#"[1970,0,1,0,0,0,4,0] | strftime("%A")"#,
            "strftime does not support %A",
        ),
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
    let program = r#"0 | strftime("%Y", "%A")"#;
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let args = ["-n", "-c", "--", program];
    let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let got = run(bin, &owned, b"");
    assert_eq!(got.stdout, b"\"1970\"\n", "{program}");
    assert_eq!(
        got.stderr, b"jq: error (at <unknown>): strftime does not support %A\n",
        "{program}"
    );
    assert_eq!(got.status.code(), Some(5), "{program}");
}

/// jq aborts when `strflocaltime` converts a number and the format is not a
/// string, and when that conversion fails. The message is the one jq prints
/// for an array, or the converting error. Letters other than the ones these
/// filters accept are not compared with jq.
#[test]
fn strflocaltime_letters_are_limited() {
    let cases = [
        (
            r#"0 | strflocaltime("%A")"#,
            "strflocaltime does not support %A",
        ),
        (
            r#"0 | strflocaltime("%B")"#,
            "strflocaltime does not support %B",
        ),
        (
            r#"0 | strflocaltime("%%")"#,
            "strflocaltime does not support %%",
        ),
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
            r#"[1970,0,1,0,0,0,4,0] | strflocaltime("%A")"#,
            "strflocaltime does not support %A",
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
    // The year comes from the array, so the zone does not change it.
    let program = r#"[1970,0,1,0,0,0,4,0] | strflocaltime("%Y", "%A")"#;
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let args = ["-n", "-c", "--", program];
    let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let got = run(bin, &owned, b"");
    assert_eq!(got.stdout, b"\"1970\"\n", "{program}");
    assert_eq!(
        got.stderr, b"jq: error (at <unknown>): strflocaltime does not support %A\n",
        "{program}"
    );
    assert_eq!(got.status.code(), Some(5), "{program}");
}

/// `strptime` rejects the same letters `strftime` rejects. jq would hand
/// those letters to the C library, so the message is not compared with jq.
#[test]
fn strptime_letters_are_limited() {
    let cases = [
        (r#""1970" | strptime("%A")"#, "strptime does not support %A"),
        (r#""1970" | strptime("%B")"#, "strptime does not support %B"),
        (r#""1970" | strptime("%%")"#, "strptime does not support %%"),
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
    let program = r#""1970" | strptime("%Y", "%A")"#;
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let args = ["-n", "-c", "--", program];
    let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let got = run(bin, &owned, b"");
    assert_eq!(got.stdout, b"[1970,0,0,0,0,0,3,-1]\n", "{program}");
    assert_eq!(
        got.stderr, b"jq: error (at <unknown>): strptime does not support %A\n",
        "{program}"
    );
    assert_eq!(got.status.code(), Some(5), "{program}");
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
fn rejected_options_stay_ours() {
    require_jq();
    let unknown = [
        "--arg=a",
        "--argjson=a",
        "--from-file=prog.jq",
        "--indent=4",
        "-fprog.jq",
        "--slurpfile",
        "--args",
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
    assert_rejects("dash f", &["-n", "--", "-f"], b"", syntax, 3);
    assert_rejects("dash arg", &["-n", "--", "--arg"], b"", syntax, 3);
}
