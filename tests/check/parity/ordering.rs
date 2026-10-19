//! Ordering filters compared with jq.

use super::*;

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
    let pairs: Vec<String> = filters
        .iter()
        .flat_map(|filter| {
            values
                .iter()
                .map(move |value| format!("{value} | {filter}"))
        })
        .collect();
    each(pairs.len(), |index| {
        let program = &pairs[index];
        compare_exact(program, &["-n", "-c", "--", program], b"");
    });

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
    each(programs.len(), |index| {
        let program = programs[index];
        compare_exact(program, &["-n", "-c", "--", program], b"");
    });

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
