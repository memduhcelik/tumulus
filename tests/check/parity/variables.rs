//! Bindings, labels, and patterns compared with jq.

use super::*;

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
    each(programs.len(), |index| {
        let program = programs[index];
        compare_exact(program, &["-n", "-c", "--", program], b"");
    });
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
fn label_break_match_jq() {
    require_jq();
    let programs = [
        "[(label $here | .[] | if .>1 then break $here else . end), \"hi!\"]",
        "[label $out | foreach .[] as $item ([3, null]; if .[0] < 1 then break $out else [.[0] -1, $item] end; .[1])]",
        "[ label $if | range(10) | ., (select(. == 5) | break $if) ]",
        "label $out | 1",
        "label $out | break $out",
        "label $out | 1, break $out, 2",
        "[label $out | 1, break $out, 2]",
        "label $out | [1, break $out, 2]",
        "(label $out | 1, break $out, 2) | . + 10",
        "[1, label $out | break $out, 2]",
        "[label $a | 1, (label $b | break $a), 2]",
        "[label $a | 1, (label $a | break $a), 2]",
        "label $a | 1 as $a | break $a",
        "label $a | 1 as $a | $a",
        "1 as $out | label $out | $out",
        "label $out | def f: break $out; f, 1",
        "label $a | def f: break $a; label $a | (f, 2), 3",
        "label $a | def f: label $a | break $a; [f, 1]",
        "def f: break $out; 1",
        "label $out | try break $out catch .",
        "label $a | label $b | try break $b catch .",
        "label $a | label $b | label $c | try break $c catch .",
        "(label $a | try break $a catch .), (label $b | try break $b catch .)",
        "def f: label $a | try break $a catch .; f, f",
        "try (label $out | break $out) catch \"c\"",
        "label $out | try (break $out | 1) catch \"c\", 2",
        "label $a | try break $a catch error(.)",
        "label $a | try break $a catch error(\"nope\")",
        "label $a | error({\"__jq\":0})",
        "label $a | error({\"__jq\":1})",
        "label $a | error({\"__jq\":0.0})",
        "label $a | error({\"__jq\":0, \"b\":1})",
        "label $out | (break $out)?",
        "[label $out | (1, (break $out)?), 2]",
        "label $o | [1] | . as [$a] ?// $b | if $a != null then break $o else $b end",
        "first(label $out | 1, break $out, 2)",
        "first(label $o | 1, debug, 2)",
        "[limit(1; label $o | 1, debug, 2)]",
        "def f: label $o | 1, debug, 2; first(f)",
        "label $out | {a: (1, break $out)}",
        "{a: (label $o | 1, break $o)}",
        "label $o | {a: break $o}",
        "label $o | \"a\\(1, break $o)b\"",
        "def f: label $o | if .==0 then break $o else (.-1|f), . end; [3|f]",
        "def f: label $o | if .==0 then break $o else [(.-1|f)][0] end; 4|f",
        "label$out|break$out",
        "label $out | def f($out): break $out; f(1), 2",
        "def f(g): g; label $out | f(break $out), 1",
        "label $out | [1,2,3] | map(if .==2 then break $out else . end)",
        "label $out | 1,2,3,4 | if .==3 then break $out else . end",
        "label $out | (break $out) // 1",
        "label $out | first(1, break $out, 2)",
        "label $out | last(1, break $out, 2)",
        "1, label $out | break $out, 2",
        "break $out",
        ". as $foo | break $foo",
        "label $out | break $other",
        "def f: break $out; f",
        "def f: break $out; label $out | f",
        "label $a | break $b",
        "1 | break $nope",
        "label $very_long_name | break $no",
        "label $a |\nbreak $b",
    ];
    each(programs.len(), |index| {
        let program = programs[index];
        compare_exact(program, &["-n", "-c", "--", program], b"");
    });
    let with_input = [
        (
            "[(label $here | .[] | if .>1 then break $here else . end), \"hi!\"]",
            "[0,1,2]\n",
        ),
        (
            "[(label $here | .[] | if .>1 then break $here else . end), \"hi!\"]",
            "[0,2,1]\n",
        ),
        (
            "[label $out | foreach .[] as $item ([3, null]; if .[0] < 1 then break $out else [.[0] -1, $item] end; .[1])]",
            "[11,22,33,44,55,66,77,88,99]\n",
        ),
        (
            "label $out | reduce .[] as $i (0; if .>2 then break $out else .+$i end)",
            "[1,2,3,4]\n",
        ),
        (
            "reduce .[] as $i (0; label $o | if $i==2 then break $o else .+ $i end)",
            "[1,2,3]\n",
        ),
        ("path(label $o | .a)", "{\"a\":1}\n"),
        ("path(label $o | .a, break $o)", "{\"a\":1}\n"),
        (".a = (label $o | 1, break $o)", "{\"a\":0}\n"),
        ("label $out | .a = (break $out)", "{\"a\":1}\n"),
        ("label $a | try break $a catch .", "1\n2\n"),
    ];
    for (program, input) in with_input {
        compare_exact(program, &["-c", "--", program], input.as_bytes());
    }
    compare_exact(
        "break empty -e",
        &["-n", "-c", "-e", "--", "label $out | break $out"],
        b"",
    );
    compare_exact(
        "label empty -e",
        &["-n", "-c", "-e", "--", "label $out | empty"],
        b"",
    );
}

#[test]
fn destructure_alt_match_jq() {
    require_jq();
    let programs = [
        "1 | . as [$a] ?// $a | $a",
        "1 | . as [$a] ?// {a:$b} | [$a,$b]",
        "1 | . as {a:$a} ?// {b:$b} | $a",
        "true | . as [$a] ?// {b:$b} ?// {c:$c} | $c",
        "[1,2] | . as [$a,$b] ?// $a | $a, $b, error(\"z\")",
        "1 | . as $a ?// $b | 1, error(\"z\")",
        "[1] | . as [$a] ?// $b | $a, error(\"nope\")",
        "[1] | . as [$a] ?// $b | 7, error(\"e\")",
        "{\"a\":1} | . as {(empty):$x} ?// $y | [$x,$y]",
        "{\"a\":1} | . as {(error(\"boom\")):$x} ?// $y | [$x,$y]",
        "{\"a\":1,\"b\":2} | . as {(\"a\",\"b\"):$x} ?// $y | if $x==2 then error(\"z\") else $x end",
        "{\"a\":[1],\"b\":2} | . as {(\"a\",\"b\"):[$x]} ?// $y | [$x,$y]",
        "{\"a\":[1],\"b\":2} | . as {(\"a\",\"b\"):[$x]} | $x",
        "5 as $a | 1 | . as [$a] ?// $c | [$a,$c]",
        "5 as $a | 1 | . as [$b] ?// $c | [$a,$b,$c]",
        "1 + 2 as [$a] ?// $a | $a",
        "4 * 2 as [$a] ?// $a | $a",
        "[foreach (1,[2]) as [$x] ?// $x (0; .+1; .)]",
        "[[1]] | reduce .[] as [$a] ?// $a (0; if $a|type==\"array\" then error(\"z\") else ($a|.[0]) end)",
        "[1] | . as [$a] ?// $b | if $a != null then halt else 9 end",
        "[1] | . as [$a] ?// $b | if $a != null then halt_error(7) else 9 end",
        "def f: . as [$a] ?// $a | $a; 1|f, [2]|f",
        "def f: . as [$a] ?// $a | if $a==0 then 0 else ($a-1|f) end; 3|f",
        ". as {a:$a}?//$a | $a",
        "{} | . as {a:$a} ?// $b | [$a,$b]",
        "null | . as {a:$a} ?// $b | [$a,$b]",
        "\"ab\" | . as [$a] ?// $a | [$a]",
        "nan | . as [$a] ?// $a | $a",
        "1 | . as [$a] ?// $a | 1/0",
        "(1 | . as {a:$a} ?// $b | $a)?",
        "1 | . as [$a] ?// $a | empty",
        "1 | . as [$a] ?// $b | select(false)",
        "1 | . as {((empty)):$a} ?// $b | $b",
        "{\"a\":1,\"b\":2} | . as {(\"a\",\"b\"):$x} ?// $y | [$x,$y]",
        "1 as $a | 2 | . as [$a] ?// $b | [$a,$b]",
        "1 as $k | {\"a\":1} | . as {($k):$v} ?// $v | $v",
        "1 as $k | {\"a\":1} | . as {($k | .a):$v} ?// $v | $v",
        "(1,2,3) as [$a] ?// $a | $a",
        "([1], 2) as [$a] ?// $a | $a",
        "1 | . as [$a] ?// $b | error(\"final\")",
        "1 | . as [$a] ?// {a:$b} | $b",
        "\"ab\" | . as {a:$a} ?// [$b] | $b",
        "[[3],[4],[5],6][] | . as {a:$a} ?// {a:$a} ?// {a:$a} | $a",
        "[[3],[4],[5],6] | .[] as {a:$a} ?// {a:$a} ?// $a | $a",
        ". as [$a] #c\n?// $b | $b",
        ". as [$a]\n?// $b | [$a,$b]",
        ". as [$a]  ?//  [$b] | [$a,$b]",
        "label $a | . as [$a] ?// $a | break $a",
        ". as $break ?// [$break] | $break",
        "try (1 | . as [$a] ?// $a | error(\"z\")) catch .",
        "try (1 | . as [$a] ?// $b | error(\"e\")) catch .",
        "{\"a\":1} | . as {$a:$b} ?// $c | [$a,$b,$c]",
        "1 | . as {$a:$b} ?// $c | [$a,$b,$c]",
        "{\"a\":[1]} | . as {a:[$b,$c]} ?// {a:$a} | [$a,$b,$c]",
        "{\"a\":[1,2]} | . as {a:[$b,$c]} ?// {a:$a} | [$a,$b,$c]",
        "{\"ab\":1} | . as {(.[:1]):$x} ?// $y | [$x,$y]",
        "{\"a\":1,\"b\":true} | . as {a:$a, b:[$b]} ?// $c | [$a,$b,$c]",
        "{\"a\":1} | . as {(\"a\", error(\"e\")):$x} ?// $y | $x",
        "{\"a\":1,\"b\":2,\"c\":3} | . as {(\"a\",\"b\",\"c\"):$x} ?// $y | if $x==2 then error(\"z\") else $x end",
        "def f: debug; {\"a\":1} | . as {a:$a} ?// $b | f | $a",
        "1 | . as {a:$a} ?// $b | debug | $b",
        "1 | . as [$a] ?// $a | 1,2,3",
        "[1,2] | . as [$a,$b] ?// $x | $a | .+10",
    ];
    each(programs.len(), |index| {
        let program = programs[index];
        compare_exact(program, &["-n", "-c", "--", program], b"");
    });
    let with_input = [
        (
            ".[] | . as {$a, b: [$c, {$d}]} ?// [$a, {$b}, $e] ?// $f | [$a, $b, $c, $d, $e, $f]",
            "[{\"a\":1, \"b\":[2,{\"d\":3}]}, [4, {\"b\":5, \"c\":6}, 7, 8, 9], \"foo\"]\n",
        ),
        (
            ".[] | . as {a:$a} ?// {a:$a} ?// {a:$a} | $a",
            "[[3],[4],[5],6]\n",
        ),
        (
            ".[] | . as {a:$a} ?// {a:$a} ?// $a | $a",
            "[[3],[4],[5],6]\n",
        ),
        (
            ".[] | . as {a:$a} ?// $a ?// {a:$a} | $a",
            "[[3],[4],[5],6]\n",
        ),
        (
            ".[] | . as $a ?// {a:$a} ?// {a:$a} | $a",
            "[[3],[4],[5],6]\n",
        ),
        (
            ".[] as [$a] ?// $a | if $a==[1] then error(\"z\") else $a end",
            "[[1],2]\n",
        ),
        (".[] as [$a] ?// $a | error(\"e\\($a)\")", "[[3],[4]]\n"),
        ("reduce .[] as [$x] ?// $x (0; . + $x)", "[1,2,3]\n"),
        (
            "[foreach .[] as $i (0; . as [$a] ?// $a | . + 1)]",
            "[[1]]\n",
        ),
        (". as [$a] ?// $a | input, $a", "7\n"),
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
        "1 as $x",
        ". as [$a,]",
        ". as {a:$a,}",
        "reduce 1+2 as $x (0; .)",
        "reduce if true then 1 else 2 end as $x (0; $x)",
        r#"{"a":1} | . as {a:$k, ($k):$v}"#,
        "1 as $x + 1",
        "reduce reduce .[] as $x (0; $x) as $y (0; $y)",
        ". as [$a]? | $a",
        ".[] as $a ?// 1 | $a",
        "{a: 1 as $x | $x}",
        "foreach .[] as $x (0)",
        "reduce .[] as $x (0; .; .)",
        "foreach .[] as $x (0; .; .; .)",
        "{$__loc__: 1}",
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
