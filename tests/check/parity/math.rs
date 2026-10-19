//! Math filters compared with jq.

use super::*;

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
