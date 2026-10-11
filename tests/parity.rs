//! Run the same inputs through tumulus and jq.
//! jq has to be installed. A missing jq fails this check.
//! Nothing here stands in for jq's output.

use std::fs;
use std::io::Write;
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
        stdin
            .write_all(input)
            .unwrap_or_else(|err| panic!("write stdin to {program}: {err}"));
    }
    child
        .wait_with_output()
        .unwrap_or_else(|err| panic!("wait for {program}: {err}"))
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
        (".[] | .a", r#"[{"a":1},2]"#),
        (".a, .b", "1"),
    ];
    for (expr, input) in cases {
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

struct DirGuard(std::path::PathBuf);

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
