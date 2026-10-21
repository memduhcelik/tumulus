//! Command-line options compared with jq.

use super::*;

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
    for option in [
        "--arg=a",
        "--argjson=a",
        "--from-file=prog.jq",
        "--indent=4",
        "--slurpfile=a",
        "--args=foo",
        "--bogus",
        "--Help",
        "-Z",
        "-eQ",
        "-fprog.jq",
        "-cZ",
    ] {
        compare_exact(option, &[option], b"");
    }

    let (dir, _guard) = scratch("reject");
    let empty = write_bytes(&dir, "empty.jq", b"");
    let bad = write_bytes(&dir, "bad.jq", b"{\n");
    let badb = write_bytes(&dir, "badb.jq", b"\xff");
    let syntax = b"tumulus: syntax error\n";
    assert_rejects("empty filter", &["-n", "-e", "-f", &empty], b"", syntax, 3);
    assert_rejects("bad filter", &["-n", "-f", &bad], b"", syntax, 3);
    assert_rejects("bad byte", &["-n", "-f", &badb], b"", syntax, 3);
    compare_exact("unbound", &["--arg", "a", "1", "-n", "-e", "$b"], b"");
    compare_exact("dash f", &["-n", "--", "-f"], b"");
    compare_exact("dash arg", &["-n", "--", "--arg"], b"");
}

#[test]
fn output_options_match_jq() {
    require_jq();
    let value = r#"[null,false,true,1,"hi",[],{"b":1,"a":"x"}]"#;
    let programs = [
        ("color null", &["-n", "-C", "null"][..]),
        ("color value", &["-n", "-C", "-c", value]),
        ("color pretty", &["-n", "-C", "[1,2]"]),
        ("color object", &["-n", "-C", "{b:1,a:2}"]),
        ("color sorted", &["-n", "-C", "-S", "{b:1,a:2}"]),
        ("color tab", &["-n", "-C", "--tab", "[1]"]),
        ("color raw", &["-n", "-C", "-r", r#""hi""#]),
        ("color ascii", &["-n", "-C", "-a", r#""é""#]),
        ("mono", &["-n", "-M", "null"]),
        ("color then mono", &["-n", "-C", "-M", "null"]),
        ("mono then color", &["-n", "-M", "-C", "null"]),
        ("cluster CM", &["-n", "-CM", "null"]),
        ("cluster MC", &["-n", "-MC", "-c", "1"]),
        ("debug color", &["-n", "-C", "1|debug"]),
        ("debug plain", &["-n", "1|debug"]),
        ("stderr color", &["-n", "-C", "1|stderr"]),
        ("seq values", &["-n", "--seq", "1,2,null"]),
        ("seq compact", &["-n", "-c", "--seq", "[1,2]"]),
        ("seq pretty", &["-n", "--seq", "{a:1}"]),
        ("seq raw", &["-n", "--seq", "-r", r#""ab""#]),
        ("seq join", &["-n", "-j", "--seq", r#"1,"a""#]),
        ("seq ascii", &["-n", "-a", "--seq", r#""é""#]),
        ("seq color", &["-n", "-C", "--seq", "-c", "1"]),
        ("seq debug", &["-n", "--seq", "1|debug"]),
        ("seq empty", &["-n", "--seq", "empty"]),
        ("raw0 string", &["-n", "--raw-output0", r#""ab""#]),
        ("raw0 two", &["-n", "--raw-output0", r#""a","b",1"#]),
        ("raw0 empty", &["-n", "--raw-output0", r#""""#]),
        ("raw0 newline", &["-n", "--raw-output0", r#""a\nb""#]),
        ("raw0 object", &["-n", "--raw-output0", "{a:1}"]),
        ("raw0 compact", &["-n", "--raw-output0", "-c", "{a:1}"]),
        ("raw0 nul", &["-n", "--raw-output0", r#""a\u0000b""#]),
        ("raw0 file nul", &["-c", "--raw-output0", "."]),
        ("raw0 join", &["-n", "--raw-output0", "-j", "1,2"]),
        ("raw0 then join", &["-n", "-j", "--raw-output0", r#""ab""#]),
        ("raw0 ascii", &["-n", "-a", "--raw-output0", r#""é""#]),
        ("raw0 color", &["-n", "-C", "--raw-output0", "1"]),
        (
            "raw0 color string",
            &["-n", "-C", "--raw-output0", r#""hi""#],
        ),
        (
            "raw0 seq",
            &["-n", "--seq", "--raw-output0", r#"1,{a:1},"z""#],
        ),
        ("raw0 status", &["-n", "-e", "--raw-output0", "false"]),
        ("raw0 empty out", &["-n", "--raw-output0", "empty"]),
    ];
    for (name, args) in programs {
        compare_exact(name, args, b"");
    }
    compare_exact(
        "raw0 stdin nul",
        &["-c", "--raw-output0", "."],
        b"\"a\\u0000b\"\n",
    );

    let seq_in = [
        ("seq in", &b"\x1e1\n\x1e2\n"[..]),
        ("seq ws", b"\x1e  1\n"),
        ("seq two in one", b"\x1e1 2\n"),
        ("seq no rs", b"1\n"),
        ("seq bare", b"1"),
        ("seq empty", b""),
        ("seq only rs", b"\x1e"),
        ("seq newline", b"\n"),
        ("seq lead", b"x\x1e1\n"),
        ("seq cut number", b"\x1e1\x1e2\n"),
        ("seq cut true", b"\x1etrue\x1e1\n"),
        ("seq number eof", b"\x1e1"),
        ("seq number space", b"\x1e1 "),
        ("seq true eof", b"\x1etrue"),
        ("seq array eof", b"\x1e[]"),
        ("seq drop tail", b"\x1e1\n\x1e[]"),
        ("seq keep nl", b"\x1e1\n\x1e[]\n"),
        ("seq second number", b"\x1e1\n2"),
        ("seq bad middle", b"\x1e1\n\x1ebad\n\x1e3\n"),
        ("seq string nl", b"\x1e\"a\nb\"\n"),
        ("seq slash", b"\x1e[\x1e]\n"),
        ("seq obj cut", b"\x1e{\"a\"\x1e1\n"),
        ("seq comment", b"\x1e // c\n1\n"),
        ("seq slurp", b"\x1e1\n\x1e2\n"),
        ("seq slurp empty", b""),
        ("seq slurp bad", b"\x1e1\n\x1ebad\n\x1e3\n"),
        ("seq stream obj", b"\x1e{\"a\":1}\n"),
        ("seq stream arr", b"\x1e[1,2]\n"),
        ("seq raw lines", b"\x1e1\n\x1e2\n"),
    ];
    for (name, input) in seq_in {
        let args: &[&str] = if name.starts_with("seq slurp bad") || name == "seq slurp" {
            &["-s", "--seq", "-c", "."]
        } else if name == "seq slurp empty" {
            &["-s", "--seq", "-c", "."]
        } else if name.starts_with("seq stream") {
            &["--stream", "--seq", "-c", "."]
        } else if name == "seq raw lines" {
            &["-R", "--seq", "."]
        } else {
            &["--seq", "-c", "."]
        };
        compare_exact(name, args, input);
    }

    compare_colored(
        "palette",
        &["-n", "-C", "-c", value],
        "1;31:1;32:1;33:1;34:1;35:1;36:1;37:1;4",
    );
    compare_colored("palette one", &["-n", "-C", "null"], "1;31");
    compare_colored("palette empty slots", &["-n", "-C", "-c", value], ":::::");
    compare_colored("palette colon", &["-n", "-C", "null"], ":");
    compare_colored("palette bad", &["-n", "-C", "null"], "bad");
    compare_colored("palette bad mono", &["-n", "-M", "null"], "nope");
    compare_colored("palette empty", &["-n", "-C", "null"], "");

    let bin = env!("CARGO_BIN_EXE_tumulus");
    for args in [
        &["-V"][..],
        &["--version"],
        &["-Vn", "1"],
        &["!", "-V"],
        &["-V", "--bogus"],
    ] {
        let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
        let got = run(bin, &owned, b"");
        assert_eq!(got.stdout, b"tumulus-0.12.1\n", "{args:?}");
        assert!(
            got.stderr.is_empty(),
            "{args:?} {}",
            String::from_utf8_lossy(&got.stderr)
        );
        assert_eq!(got.status.code(), Some(0), "{args:?}");
        let mut argv = vec![bin.to_string()];
        argv.extend(owned);
        let inplace = tumulus::execute(&argv, b"");
        assert_eq!(inplace.stdout, got.stdout, "{args:?}");
        assert_eq!(inplace.stderr, got.stderr, "{args:?}");
        assert_eq!(Some(inplace.code), got.status.code(), "{args:?}");
    }
    let jq = run("jq", &["-V".to_string()], b"");
    assert!(jq.stdout.ends_with(b"\n") && jq.stdout.starts_with(b"jq-"));
    assert!(jq.stderr.is_empty());
    assert_eq!(jq.status.code(), Some(0));

    assert!(early_byte(&["-c", "--unbuffered", "."], b"1\n").starts_with(b"1\n"));
    assert!(early_byte(&["-c", "."], b"1\n").is_empty());
}

#[test]
fn help_and_build_match_jq() {
    require_jq();
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let version = env!("CARGO_PKG_VERSION");
    // An object, so a consumed `--indent` is compact and a consumed `-L` is pretty.
    let input = b"{\"a\":1}\n";
    let cases: &[&[&str]] = &[
        &["-h"],
        &["--help"],
        &["--build-configuration"],
        &["-h", "."],
        &["--help", "."],
        &["-nh"],
        &["-hn"],
        &["-hV"],
        &["-Vh"],
        &["-V", "-h"],
        &["-h", "-V"],
        &["--help", "--version"],
        &["--version", "--help"],
        &["--build-configuration", "."],
        &["-n", "--build-configuration"],
        &["--build-configuration", "--help"],
        &["--help", "--build-configuration"],
        &["-h", "--bogus"],
        &["--help", "--bogus"],
        &["-eh"],
        &["-he"],
        &["--", "-h"],
        &["-f", "-h"],
        &["--from-file", "--help"],
        &["-L", "--help"],
        &["--indent", "--help"],
        &["-L", "--build-configuration"],
        &["--indent", "--build-configuration"],
        &["--arg", "a", "1", "-h"],
        &["-h", "-c"],
        &["-ch"],
        &["--compact-output", "--help"],
        &["-hx"],
        &["-xh"],
        &["-Vhx"],
        &["-xVh"],
        &["-h=foo"],
        &["-hbogus"],
        &["--build-configuration", "--bogus"],
        &["--build-configuration", "--version"],
        &["--version", "--build-configuration"],
        &[".", "-h"],
        &[".", "--build-configuration", "-h"],
        &["-h", ".", "--build-configuration"],
        &["--args", "-h"],
        &["--jsonargs", "-h"],
        &[".", "--", "-h"],
        &["--", "--help"],
        &["--", "--build-configuration"],
        &["!", "-h"],
        &["!", "--build-configuration"],
        &["-fh"],
        &["-hf"],
        &["-hello"],
        &["-nh=foo"],
        &["-ah"],
        &["-ha"],
        &["-h."],
        &["-h", "--arg"],
        &["--help", "--indent"],
        &["--stream", "--build-configuration"],
        &["--tab", "-h"],
        &["-c", "-h", "."],
        &["-n", ".", "-h"],
        &[".", "--args", "-h"],
        &[".", "/no/such", "-h"],
        &["-xV"],
        &["-Vx"],
        &["-nV"],
        &["-Vn"],
    ];
    let mut help: Option<Vec<u8>> = None;
    for args in cases {
        let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
        let jq = run("jq", &owned, input);
        let got = run(bin, &owned, input);
        assert_eq!(
            got.status.code(),
            jq.status.code(),
            "{args:?} exit\nours:\n{}\njq:\n{}",
            String::from_utf8_lossy(&got.stderr),
            String::from_utf8_lossy(&jq.stderr)
        );
        if jq.stdout.starts_with(b"jq - commandline") {
            assert!(jq.stderr.is_empty(), "{args:?}");
            assert!(
                got.stderr.is_empty(),
                "{args:?} {}",
                String::from_utf8_lossy(&got.stderr)
            );
            assert_eq!(got.status.code(), Some(0), "{args:?}");
            match &help {
                Some(earlier) => assert_eq!(&got.stdout, earlier, "{args:?} help differs"),
                None => {
                    let text = String::from_utf8(got.stdout.clone()).expect("help is utf-8");
                    let own = text.replace(bin, "PROG");
                    assert!(own.starts_with(&format!("tumulus {version}\n")), "{own}");
                    assert!(own.contains("\nUsage:  PROG [options] <filter> [file...]\n"));
                    assert!(own.contains("--build-configuration"));
                    assert!(own.contains("-h, --help"));
                    assert!(own.contains("Print this text and exit."));
                    assert!(own.contains("oniguruma=6.9.10"));
                    assert!(!own.contains("jq"), "{own}");
                    assert!(!own.contains("commandline JSON processor"));
                    assert!(!own.contains("jqlang"));
                    assert!(!own.contains("--with-oniguruma"));
                    assert!(text.ends_with('\n'));
                    help = Some(got.stdout.clone());
                }
            }
        } else if jq_build_configuration(&jq.stdout) {
            // tumulus names the Oniguruma it ships. jq names its own build.
            assert_eq!(
                got.stdout,
                b"oniguruma=6.9.10\n",
                "{args:?}\njq:\n{}",
                String::from_utf8_lossy(&jq.stdout)
            );
            assert!(jq.stderr.is_empty() && got.stderr.is_empty(), "{args:?}");
            assert_eq!(got.status.code(), Some(0), "{args:?}");
        } else if jq.stdout.starts_with(b"jq-") {
            let line = format!("tumulus-{version}\n");
            assert_eq!(got.stdout, line.as_bytes(), "{args:?}");
            assert!(jq.stderr.is_empty() && got.stderr.is_empty(), "{args:?}");
            assert_eq!(got.status.code(), Some(0), "{args:?}");
        } else {
            assert_eq!(got.stdout, jq.stdout, "{args:?} stdout");
            assert_eq!(
                got.stderr, jq.stderr,
                "{args:?} stderr\nours {:?}\njq {:?}",
                got.stderr, jq.stderr
            );
        }
        let mut argv = vec![bin.to_string()];
        argv.extend(owned);
        let inplace = tumulus::execute(&argv, input);
        assert_eq!(inplace.stdout, got.stdout, "{args:?}");
        assert_eq!(inplace.stderr, got.stderr, "{args:?}");
        assert_eq!(Some(inplace.code), got.status.code(), "{args:?}");
    }
    assert!(help.is_some());
    for program in ["jq", bin] {
        for flag in ["-h", "--help", "--build-configuration"] {
            exits_while_stdin_stays_open(program, flag);
        }
    }
}

/// jq's `--build-configuration` text, checked live.
///
/// On macOS that text is `--with-oniguruma=builtin`. On Ubuntu it is one
/// configure line from apt jq 1.7.1-3ubuntu0.24.04.2. The line starts with
/// `--build=x86_64-linux-gnu`, includes `--prefix=/usr`, and does not name
/// Oniguruma.
fn jq_build_configuration(stdout: &[u8]) -> bool {
    #[cfg(target_os = "macos")]
    {
        stdout == b"--with-oniguruma=builtin\n"
    }
    #[cfg(target_os = "linux")]
    {
        let Ok(text) = std::str::from_utf8(stdout) else {
            return false;
        };
        let Some(line) = text.strip_suffix('\n') else {
            return false;
        };
        !line.contains('\n')
            && line.starts_with("--build=x86_64-linux-gnu ")
            && line.contains(" --prefix=/usr ")
            && line.contains("1.7.1-3ubuntu0.24.04.2")
            && !line.contains("oniguruma")
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = stdout;
        false
    }
}

/// The pipe stays open. `-h`, `--help`, and `--build-configuration` still exit.
fn exits_while_stdin_stays_open(program: &str, flag: &str) {
    let mut child = Command::new(program)
        .arg(flag)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("could not start {program}: {err}"));
    let _held = child.stdin.take();
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < std::time::Duration::from_secs(2) => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("{program} {flag} waited for stdin");
            }
            Err(err) => panic!("{program} {flag}: {err}"),
        }
    };
    assert!(status.success(), "{program} {flag} exited {status}");
    let mut stdout = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut stdout)
        .unwrap();
    assert!(!stdout.is_empty(), "{program} {flag} wrote nothing");
    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .unwrap();
    assert!(
        stderr.is_empty(),
        "{program} {flag} stderr {}",
        String::from_utf8_lossy(&stderr)
    );
}

fn compare_colored(name: &str, args: &[&str], colors: &str) {
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let go = |program: &str| {
        let mut cmd = Command::new(program);
        cmd.args(&owned)
            .env("JQ_COLORS", colors)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.output().unwrap_or_else(|err| panic!("{name}: {err}"))
    };
    let got = go(bin);
    let expected = go("jq");
    assert_eq!(got.stdout, expected.stdout, "{name}: stdout");
    assert_eq!(
        got.stderr,
        expected.stderr,
        "{name}: stderr\n{}",
        String::from_utf8_lossy(&got.stderr)
    );
    assert_eq!(got.status.code(), expected.status.code(), "{name}: status");
}

fn early_byte(args: &[&str], input: &[u8]) -> Vec<u8> {
    let bin = env!("CARGO_BIN_EXE_tumulus");
    let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let mut child = Command::new(bin)
        .args(&owned)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let _ = stdin.write_all(input);
    let _ = stdin.flush();
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 64];
        let n = stdout.read(&mut buf).unwrap_or(0);
        let _ = tx.send(buf[..n].to_vec());
    });
    // Buffered output stays empty until the process ends. `--unbuffered` does not.
    let got = rx
        .recv_timeout(std::time::Duration::from_millis(400))
        .unwrap_or_default();
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    got
}
