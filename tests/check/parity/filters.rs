//! Filters, operators, and syntax compared with jq.

use super::*;

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
fn early_syntax_matches_jq() {
    require_jq();
    for program in [
        "1 +", "1 -", "1 +\n", "1 + 2 +", "[1,2", "[1,2\n", "[1,2 ", "}", " }", "}\n", "\n}",
    ] {
        compare_exact(program, &["-n", "--", program], b"");
    }
}

#[test]
fn more_syntax_matches_jq() {
    require_jq();
    let programs = [
        "(1",
        ".[",
        "1 2",
        "{a:1+2}",
        "if 1 then 2",
        "1 *",
        "1 /",
        "1 %",
        "1 |",
        "1 and",
        "1 or",
        "1 //",
        "1 ==",
        "1 <",
        "1 >",
        "1 !=",
        "1 <=",
        "1 >=",
        "[1,",
        ".a }",
        r#""\(1 +)""#,
        "def f: 1 +; f",
    ];
    for program in programs {
        compare_exact(program, &["-n", "--", program], b"");
    }
}

#[test]
fn operator_syntax_stays_generic() {
    require_jq();
    let bin = env!("CARGO_BIN_EXE_tumulus");
    for program in [
        "1<2<3", "1==1==1", "not true", "1++2", ".a=.b=1", ".a|=.|=.", ".a+=1+=2",
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
fn interpolated_strings_match_jq() {
    require_jq();
    let programs = [
        r#""\(1,2) \(3,4)""#,
        r#""\(1,2)-\(3,4)-\(5,6)""#,
        r#""a\(1,2)b\(3,4)c""#,
        r#""\(1,2) \(empty)""#,
        r#""\(empty) \(1,2)""#,
        r#""\(1,2) \(empty) \(3,4)""#,
        r#""\([1,2]) \(true)""#,
        r#""\({a:1}) \(null)""#,
        r#"{"\(1,2)\(3,4)": 9}"#,
        r#"def f: "\(1,2) \(3,4)"; f"#,
        r#""\(1,2) \(3,error("z"))""#,
        r#""\(1,error("z")) \(3,4)""#,
        r#""\(1,2) \(error("z")) \(3,4)""#,
        r#""\(1,error("L")) \(2,error("R"))""#,
        r#"try "\(1,error("z")) \(3,4)" catch ."#,
    ];
    for program in programs {
        compare_exact(program, &["-n", "-c", "--", program], b"");
    }
    let with_input = [
        (r#""\(input) \(input)""#, "1\n2\n3\n"),
        (r#""\(input,input) \(input,input)""#, "1\n2\n3\n4\n5\n6\n7\n8\n"),
        (r#""\(input)-\(input)-\(input)""#, "1\n2\n3\n4\n"),
        (r#""\(input,input) \(input,error("z"))""#, "1\n2\n3\n4\n"),
        (r#"{"\(input,input)\(input,input)": 1}"#, "1\n2\n3\n4\n5\n6\n7\n8\n"),
    ];
    for (program, input) in with_input {
        compare_exact(program, &["-n", "-c", "--", program], input.as_bytes());
    }
}
