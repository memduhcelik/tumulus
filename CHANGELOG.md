# Changelog

## 0.5.0

Released on 2026-10-14.

`floor`, `ceil`, `round`, `trunc`, `nearbyint`, and `rint` round a number. `fabs` is its absolute value. `sqrt` and `cbrt` are roots. `exp`, `exp2`, `exp10`, and `expm1` are exponentials. `log`, `log2`, `log10`, `log1p`, and `logb` are logarithms. `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, and `atanh` are the trigonometric functions. `gamma` and `tgamma` are the gamma function, and `lgamma` is its log. `erf` and `erfc` are the error function. `j0`, `j1`, `y0`, and `y1` are Bessel functions. `significand` is the significand. `frexp` returns `[mantissa, exponent]`. `modf` returns `[fraction, integer]`.

`pow(x; y)`, `atan2(y; x)`, `hypot`, `copysign`, `fdim`, `fmax`, `fmin`, `fmod`, `remainder`, `drem`, `nextafter`, `scalb`, `scalbln`, `ldexp`, `jn`, and `yn` take two filters. `fma(x; y; z)` takes three. The rightmost filter runs outermost, and each filter to its left runs again for every value. A value that is not a number fails with `number required`, and the exit status is 5. `nan` stays a number and prints as `null`. `-(nan)` flips its sign. When both arguments are zero, a Mac `fmin` is negative if either zero is, and a Mac `fmax` is negative only when both are. Linux returns the first zero.

The Mac and Linux binaries answer differently where the two C libraries do. On a Mac, `gamma` matches `tgamma`. `ldexp`, `jn`, and `yn` convert an exponent through a C `int`, and `scalbln` through a C `long`. On x86_64 an out-of-range value, including `nan` and infinity, becomes the minimum integer. Elsewhere it saturates, and `nan` becomes 0. On a Mac, `ldexp(1; nan)` is 1 and `ldexp(1; infinite)` is infinite.

`abs` is `if . < 0 then -. else . end`. A string, array, or object is left as it is. `null` and a boolean fail with `cannot be negated`, and the exit status is 5. `lgamma_r` returns `[log-gamma, sign]`. A value that is not a number fails with `number required`. `nexttoward(x; y)` is the next number after `x` in the direction of `y`. A value that is not a number fails with `number required`. `pow10` fails with `Error: pow10/0 not found at build time` and exit status 5. The input is not examined.

Of the 447 programs in jq 1.7.1's test file, 216 come out the same.

## 0.4.0

Released on 2026-10-14.

A runtime error names the line jq names. A value spread over several lines was reported where it started. jq reports how far its reader got when the value finished.

`todate` writes a number of seconds since the epoch as UTC `YYYY-MM-DDTHH:MM:SSZ`. `fromdate` reads that text back into seconds. `gmtime` breaks a number of seconds into `[year, month, day, hour, minute, second, weekday, yearday]`. The month and the weekday start at 0. `mktime` turns that array back into seconds. `strftime(fmt)` formats a number of seconds or that array. `strptime(fmt)` reads text in that format back into the array. `now` is the current number of seconds since the epoch. The format letters are `%Y`, `%m`, `%d`, `%H`, `%M`, `%S`, `%Z`, `%j`, `%a`, `%b`, `%e`, `%y`, and `%z`. A value of the wrong kind fails the way jq fails, and the exit status is 5. Any other letter is rejected. The message names the letter and the exit status is 5.

The Mac and Linux binaries answer differently where the two jq builds do. On a Mac, `strptime` replaces the weekday and yearday with its own calculation. `"" | strptime("")` and `"" | strptime("%Y")` are both `[1900,0,0,0,0,0,6,-1]`. On Linux, `strptime` keeps the fields the C library stored. `"" | strptime("")` is `[1900,0,0,0,0,0,8,367]`, and `"" | strptime("%Y")` fails with `date "" does not match format "%Y"`. A `%z` offset moves the clock into the local zone on a Mac and leaves the written time on Linux. `nan | gmtime` is `[1970,0,1,0,0,null,4,0]` on a Mac. On Linux it fails with `error converting number of seconds since epoch to datetime`.

Of the 447 programs in jq 1.7.1's test file, 167 come out the same.

## 0.3.0

Released on 2026-10-13.

A `?` on a bracket keeps the later indexes. `.["a",0]?` on an array prints the element. Without `?`, and with the `?` outside the parentheses, the first failure still stops the rest.

`-s` reads the input as one array. `-R` reads each line as a string, and with `-s` reads every opened file as one string. `-e` replaces a successful exit: no output is 4, a last `null` or `false` is 1, and any other value stays 0. Status 2, 3, and 5 are left as they are. `-f` reads the program from a file. `--arg` binds a string and `--argjson` binds one JSON value. The first definition of a name wins.

`=`, `|=`, `+=`, `-=`, `*=`, `/=`, `%=`, and `//=` write through a path. `del` removes what a path reaches. `|=` writes the first result back. No result removes that place, which is also what `del` does. A path is a field, an index, a slice, or iteration. It can go through `|`, a comma, `select`, `//`, `if`, and `as`. `and`, `or`, and a matching `null`, `true`, or `false` stay paths when they do in jq. An update on the wrong kind of value fails the way jq fails.

A version tag builds macOS arm64 and Linux x86_64 binaries and attaches them to the release. Publishing that release does not need a git checkout.

## 0.2.0

Released on 2026-10-12.

`.[2:5]` and `.[:-1]` work on arrays and strings. `."a b"`, `.["a","b"]`, and `..` follow jq. A slice of the wrong type uses jq's message.

A program can contain numbers, strings, `true`, `false`, `null`, arrays, and objects. Strings take the usual escapes and `\( )`. `{a}` and `{"a"}` copy a field. A `#` comment runs to the end of the line. `-n` runs the program once on `null` and does not read input. A bad object key or a bad string escape fails the way jq fails.

`+`, `-`, `*`, `/`, and `%` work on numbers and on the strings, arrays, and objects jq allows. Comparisons, `and`, `or`, `not`, `//`, and `if` follow jq. An operator given values it cannot use fails the way jq fails. The remainder of 2^63 by 10 matches jq. That value does not fit in a signed 64-bit integer. jq's conversion of it is the minimum on x86_64 and the maximum elsewhere.

`-1` and `-.a` are filters. An argument is an option cluster only when the character after the dash is an ASCII letter.

A bad byte inside a string becomes U+FFFD and parsing continues. Columns still count the original bytes.

`$name` reads a binding. `as` binds a value for the rest of an expression, and the pattern can take an array or an object apart. `reduce` and `foreach` walk a stream with that pattern. A pattern that meets the wrong kind of value fails with jq's index message.

`empty`, `length`, `keys`, `has`, `select`, and `map` follow jq. `has`, `select`, and `map` take one filter. The wrong kind of value fails the way jq fails, and the exit status is 5.

The exit status is 5 only when the last value is the one that failed. A later value that survives still leaves the status at 0, and the error is still printed. When the output pipe closes, as `head` does, the process dies on that signal instead of printing a broken pipe error.

Of the 447 programs in jq 1.7.1's test file, 141 come out the same.

## 0.1.0

Released on 2026-10-11.

tumulus reads JSON from standard input or files and prints it in jq's default layout or compact form. The license is MIT.

Exponents and odd number spellings are rewritten the way jq does. `NaN` and `Infinity` are read the way jq does. `-r`, `-j`, `-a`, `-S`, `--tab`, and `--indent` are accepted, including combinations. When a named file is missing, that file is reported and the others are still printed.

Filters are `.a`, `.a.b`, `.["a"]`, `.[0]`, `.[]`, pipe, comma, and the `?` suffix. A type error uses jq's message and exit status.

A truncated final value used to discard every complete value parsed before it. Those values are printed, and then jq's parse error is reported.
