//! Modules and imports compared with jq.

use super::*;

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

/// A library that ends in a filter is a compile error for `import`,
/// `include`, and `modulemeta`.
#[test]
fn library_main_matches_jq() {
    require_jq();
    let (dir, _guard) = scratch("libmain");
    let lib = dir.join("lib");
    let home = dir.join("home");
    let work = dir.join("work");
    fs::create_dir_all(&lib).unwrap();
    fs::create_dir_all(home.join(".jq")).unwrap();
    fs::create_dir_all(&work).unwrap();
    fs::write(lib.join("ok.jq"), "def f: 1;\n").unwrap();
    fs::write(lib.join("main.jq"), "def f: 1; 2\n").unwrap();
    fs::write(lib.join("call.jq"), "def f: 1; f\n").unwrap();
    fs::write(lib.join("only.jq"), "1\n").unwrap();
    fs::write(lib.join("emptyf.jq"), "empty\n").unwrap();
    fs::write(lib.join("defempty.jq"), "def f: 1; empty\n").unwrap();
    fs::write(lib.join("modonly.jq"), "module {\"name\":\"m\"};\n").unwrap();
    fs::write(
        lib.join("modok.jq"),
        "module {\"name\":\"m\"}; def f: 1;\n",
    )
    .unwrap();
    fs::write(
        lib.join("usesmain.jq"),
        "import \"main\" as m; def f: 1;\n",
    )
    .unwrap();
    fs::write(
        lib.join("two.jq"),
        "import \"main\" as a; import \"only\" as b; def f: 1;\n",
    )
    .unwrap();
    fs::write(lib.join("missmain.jq"), "import \"nope\" as n; 2\n").unwrap();
    fs::write(
        lib.join("mix.jq"),
        "import \"main\" as m; import \"nope\" as n; def f: 1;\n",
    )
    .unwrap();
    fs::write(work.join("cwdmain.jq"), "def f: 9; 8\n").unwrap();
    let lib_path = lib.to_str().unwrap();
    let programs = [
        r#"import "main" as m; m::f"#,
        r#"include "main"; f"#,
        r#""main" | modulemeta"#,
        r#"try ("main" | modulemeta) catch ."#,
        r#""main" | modulemeta, 9"#,
        r#"("ok","main") | modulemeta"#,
        r#"import "only" as m; 1"#,
        r#"import "call" as m; 1"#,
        r#"import "emptyf" as m; 1"#,
        r#"import "defempty" as m; 1"#,
        r#"include "defempty"; 1"#,
        r#"import "ok" as m; m::f"#,
        r#"import "modonly" as m; 1"#,
        r#"import "modok" as m; m::f"#,
        r#""modok" | modulemeta"#,
        r#"import "usesmain" as u; 1"#,
        r#""usesmain" | modulemeta"#,
        r#"import "two" as t; 1"#,
        r#"import "missmain" as m; 1"#,
        r#"import "mix" as m; 1"#,
        r#"import "main" as a; import "only" as b; 1"#,
        r#"import "nope" as a; import "gone" as b; 1"#,
        r#"import "main" as a; import "nope" as b; 1"#,
        r#"import "nope" as a; import "main" as b; import "only" as c; 1"#,
    ];
    for program in programs {
        compare_module(
            program,
            &dir,
            &home,
            &["-n", "-c", "-L", lib_path, program],
            b"",
        );
    }
    compare_module(
        "run as program",
        &work,
        &home,
        &["-n", "-c", "-f", lib.join("main.jq").to_str().unwrap()],
        b"",
    );
    compare_module(
        "defs need a program",
        &work,
        &home,
        &["-n", "-f", lib.join("ok.jq").to_str().unwrap()],
        b"",
    );
    compare_module(
        "cwd import",
        &work,
        &home,
        &["-n", "-c", r#"import "cwdmain" as m; 1"#],
        b"",
    );
    compare_module(
        "exit null",
        &dir,
        &home,
        &["-n", "-e", "-c", "-L", lib_path, r#""main" | modulemeta"#],
        b"",
    );
}
