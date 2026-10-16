# Changelog

## Unreleased

A slice bound can be any expression. `.[$i:]`, `.[:length-1]`, and `.[index("b"):]` read, assign, and delete the way jq does. The bounds run on the value the brackets were applied to, and a comma on one side runs every combination. Assignment applies those slices from left to right. `del` removes them from the original positions. A bound that is not a number or null fails with `Array/string slice indices must be integers` on an array or a string. Of the 447 programs in jq 1.7.1's test file, 320 come out the same. The new one is `map([1,2][0:.])`.

Inside `reduce`, `.[expr] |= expr` sets a key or an index, and `setpath` writes a path, without copying the whole value on every step. `foreach` appends with `. + [expr]` and updates with `.[expr] |= expr` the same way, and yields each step before it builds the next one. A step that is saved, as in `[foreach]`, is copied. A `foreach` inside a function is copied too, because the call hands back each step. The result is unchanged.

`setpath([nan]; value)` on an array or null fails with `Cannot set array element at NaN index`. A slice bound may be `nan` or `infinite`. `nan` on the left of `:` is the start of the value, and on the right it is the end. `infinite` is the end and `-infinite` is the start. Setting `-infinite`, or another negative index that does not fit such as `-1e300`, fails with `Out of bounds negative array index`. Removing that index leaves the value as it is.

`ascii_downcase` and `ascii_upcase` change only ASCII letters. `explode` writes the code points of a string, and `implode` reads them back. A code point that does not fit, or a UTF-16 surrogate, becomes U+FFFD, and a fraction is truncated toward zero. `startswith` and `endswith` test a string prefix or suffix. `index`, `rindex`, and `indices` find a value in a string or an array. A string search counts UTF-8 bytes. An array search for an array looks for that sequence. On any other value, `indices(s)` is `.[s]`. `utf8bytelength` counts the UTF-8 bytes of a string. A value of the wrong kind fails the way jq fails, and the exit status is 5.

## 0.6.0

Released on 2026-10-16.

Inside `reduce`, `. + [expr]` appends to an array, and `.[expr] = expr` sets a key or an index, without copying the whole value on every step. The result is unchanged.

`sort` orders an array. The order is the comparison order: null, then false, then true, then numbers, strings, arrays, and objects. Equal values stay in the order they arrived. `sort_by(f)`, `group_by(f)`, `unique_by(f)`, `min_by(f)`, and `max_by(f)` take that one filter. `f` runs once on each element, from left to right, and every value it yields is part of the key. `sort_by(.a, .b)` compares `.a` and then `.b`. `group_by(f)` keeps each run of equal keys as an array, in the order the values arrived inside the run. `unique` and `unique_by(f)` keep the first value of each run. `min` and `min_by(f)` keep the earliest value when the keys compare equal. `max` and `max_by(f)` keep the latest. `min`, `max`, `min_by`, and `max_by` of an empty array are null. `nan` compares less than itself, so `sort` and `sort_by` leave those values in the order they arrived, `min` keeps the last `nan`, and `max` keeps the first. `unique` and `group_by` keep every `nan`. `sort`, `unique`, `min`, and `max` take no argument. A value of the wrong kind fails the way jq fails, and the exit status is 5.

`input` reads the next value from the input. `inputs` reads every value still waiting. With `-n`, nothing is read until one of those runs. At the end of the input, `input` fails with `break` and the exit status is 5. `inputs` stops there. A value that is not valid JSON fails that read with jq's parse message. If that failure is caught, a later `input` continues on the next line. A missing file is named, later files are still read, and the exit status is 2. `input_filename` is the name of the file the current value came from, or null before any value has been read. With `-s`, one array is the whole input. With `-R`, each line is one string.

`$__loc__` is `{"file":"<top-level>","line":N}`, and `N` is the line of that `$`. A program from `-f` still says `<top-level>`.

`$ENV` is the process environment. `env` reads that same object. An `as` binding of `$ENV` hides it and leaves `env` alone. `--arg ENV` is stored on `$ARGS.named` and does not replace `$ENV`.

`--args` fills `$ARGS.positional` with the later file arguments as strings. `--jsonargs` parses each one as JSON. A value that is not one JSON value exits 2 with `jq: invalid JSON text passed to --jsonargs`. The later of the two flags decides what follows it. `--slurpfile name path` binds `$name` to the JSON values in that file as one array. `--rawfile name path` binds the file's bytes as one string. Both names are fields of `$ARGS.named`. A missing file, a directory, or JSON that is not valid exits 2, and the message names the option, the name, and the path. The first binding of a name wins.

`path(expr)` is the path from the input to each value `expr` reaches, as an array. A string is a field, a number is an index, and an object is a slice. `paths` lists every path inside the input except the empty path to the input itself. `paths(expr)` keeps a path once for each truthy value `expr` produces. `expr` is tried on the input first, so a failure there yields nothing else.

`getpath(path)` reads the value at one path. `setpath(path; value)` writes it, and builds the missing objects and arrays along the way. The value runs outermost, and each path is tried again for every value. `delpaths(paths)` removes every path in one array. Indexes are the ones from before any path was removed. A path jq refuses, or a value of the wrong kind, fails the way jq fails, and the exit status is 5. `getpath` works on the left of an update.

`strftime`, `strflocaltime`, and `strptime` also accept `%A`, `%B`, `%I`, `%p`, `%u`, `%w`, `%s`, `%D`, `%F`, `%T`, `%R`, `%c`, and `%%`. `%A` and `%B` are the full weekday and month. `%I` is the hour from 01 to 12 and `%p` is the morning or afternoon mark. `%u` counts Monday as 1 and `%w` counts Sunday as 0. `%s` is seconds since the epoch. `%D`, `%F`, `%T`, and `%R` are the short date, the calendar date, the time, and the hour and minute. `%c` is the locale's date and time. `%%` is a percent sign. A letter still outside that set is rejected. The message names it and the exit status is 5.

`limit(n; expr)` keeps values from `expr` and does not run the rest. A count of 0 keeps nothing and does not run `expr`. A negative count, `nan`, `null`, or a boolean keeps every value. A positive finite count keeps the ceiling of that many. A count that is not a number fails when subtracted from, after the stream has produced a value, unless the stream fails first.

`first` is `.[0]` and `last` is `.[-1]`. `nth(n)` is `.[n]`. `first(expr)` is the first value `expr` produces and does not run a later one. `last(expr)` is the last value, or `null` when `expr` yields nothing. `nth(n; expr)` counts from 0. A negative index, `nan`, `null`, or a boolean fails with `nth doesn't support negative indices`, and the exit status is 5. An index past the end produces nothing.

`range(to)` counts from 0 up to but not including `to`. `range(from; to)` steps by 1. `range(from; to; step)` uses `step`. A step of 0 produces nothing. A bound that is not a number fails with `Range bounds must be numeric`, and the exit status is 5. With one or two arguments, an end of `nan` does not stop the count. With three, it does.

`until(cond; next)` yields the first value for which `cond` is truthy. `while(cond; update)` yields a value while `cond` is truthy and then applies `update`. Each result of `cond`, `next`, and `update` is its own branch. `repeat(expr)` runs `expr` on the original input and then repeats. `first`, `last`, and `nth(n)` can be updated, and so can `first(expr)`, `nth(n; expr)`, and `limit(n; expr)` when the value was reached through the input. Setting a `nan` index fails with `Cannot set array element at NaN index`.

`def name: body; rest` defines a function. `def name(a; b): body; rest` takes filters. Each use runs that filter again, on the value at the use, and an unused filter is not run. `def name($a; $b): body; rest` binds values before the body runs. The arguments run in the caller's scope, the leftmost varies slowest, and the body runs once for each combination. The same name without `$` runs that argument again at each use, and `.` there is the value at the use. `$name` is still the value bound for that combination. A `$` binding from `as` is not a filter. A function defined inside another one sees the outer function's arguments. An inner function of the same name hides a parameter, and a parameter hides an outer function of the same name. In one parameter list the later parameter of a name hides the earlier one. The function can call itself a million levels deep, including when the call sits under a pipe or `+`. A later function of the same name and the same number of arguments hides the earlier one from there on. `true`, `false`, and `null` stay literals. `nan` and `infinite` do not, when a parameter or a function of that name is in scope. A call of a name or a number of arguments that is not defined fails with jq's `name/n is not defined` message, and the exit status is 3.

A filter that does not call a user function runs at the speed it did before a function could call itself a million levels deep. A function can still call itself that deep. A call inside `[]`, `{}`, `//`, `try`, `reduce`, `as`, `first`, `limit`, or `map` can go 100000 levels deep.

`recurse` and `..` yield a value and then each value inside it. `recurse(f)` walks each value `f` yields, depth first. `recurse(f; cond)` walks such a value once for each truthy result of `cond`. `limit` and `first` stop that walk before a later value is produced. Used as a path, `recurse(f)` includes the current value and then each value `f` reaches by indexing, and `cond` decides whether that value is entered. A computed value is an invalid path. `walk(f)` rebuilds an array or object from the bottom and then runs `f`. An array keeps every value a child yields. An object keeps the first value of a field and does not run the rest of that field, and a field that yields nothing is left out. `to_entries` writes `{key, value}` pairs in stored order. An array's keys are numbers. `from_entries` reads `key`, then `Key`, then `name`, then `Name`, and skips null and false. `k` is not a key. The value is the `value` field when it is present, even if that value is null, otherwise `Value`, otherwise null. `with_entries(f)` runs `f` over those pairs and reads them back. `tojson` writes compact JSON, and a string is quoted. `add` folds with `+`, starting from null. An empty array or object is null. A value of the wrong kind fails the way jq fails, and the exit status is 5.

Of the 447 programs in jq 1.7.1's test file, 302 come out the same.

## 0.5.0

Released on 2026-10-14.

`localtime` breaks a number of seconds into `[year, month, day, hour, minute, second, weekday, yearday]` in the local zone. The month and the weekday start at 0. `strflocaltime(fmt)` formats a number of seconds in local time, or that array as written. The format letters are the same ones `strftime` accepts. A value of the wrong kind fails the way jq fails, and the exit status is 5.

`try expr` keeps every value `expr` produced before it failed, then stops. That is the same as `try expr catch empty`. `try expr catch handler` runs `handler` on the error value instead. A builtin failure is caught as its message. `error` raises the current input, and `error(expr)` raises the first value `expr` produces. When nothing catches the error, a string is printed as `jq: error (at ...): ` and that text. Any other value is printed as `jq: error (at ...) (not a string): ` and the compact JSON. The exit status is 5 when that input is the last one.

`type` names the value: `null`, `boolean`, `number`, `string`, `array`, or `object`. `tonumber` leaves a number as it is and reads a string as one JSON number. `tostring` leaves a string as it is and writes any other value as compact JSON. `join(expr)` walks an array, or the values of an object, and joins them with each value of `expr`. A null in that walk is empty, and a number or boolean is written as text. `ltrimstr(expr)` and `rtrimstr(expr)` drop one leading or trailing copy of a string. `tonumber` and `join` fail on a value they cannot use, the way jq fails, and the exit status is 5. `type`, `tostring`, `ltrimstr`, and `rtrimstr` do not.

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
