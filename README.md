# tumulus

tumulus reads JSON and prints it the way jq does. The filter language is still small. This is version 0.11.0.

macOS on Apple silicon:

```sh
curl -L -o tumulus https://github.com/memduhcelik/tumulus/releases/download/v0.11.0/tumulus-macos-arm64
chmod +x tumulus
```

Linux on x86_64:

```sh
curl -L -o tumulus https://github.com/memduhcelik/tumulus/releases/download/v0.11.0/tumulus-linux-x86_64
chmod +x tumulus
```

Build a release binary with `cargo build --release`. The binary is `target/release/tumulus`. The timed binary below was built with rustc 1.96.0 (ac68faa20 2026-05-25). `cargo test` runs the same inputs through tumulus and jq, and it fails if jq is missing. `scripts/ubuntu-jq.sh` runs that comparison in Ubuntu 24.04 against the jq package the Ubuntu CI job installs.

`cargo test --test check -- --exact jq_suite::compare_jq_test_file --nocapture` runs every program and input from `tests/jq.test` through both and prints how many come out the same. That file is the `tests/jq.test` shipped with jq 1.7.1. The expected lines in it only mark the end of a case. The run asks jq each time. A mismatch is printed and does not fail the test. Of the 447 programs, 444 come out the same on stdout, stderr, and the exit status.

## An example

`tests/fixtures/settings.json` is one settings object. These commands print the same text with tumulus and with jq.

```sh
tumulus -c '.ids' tests/fixtures/settings.json
```

```json
{"exact":9007199254740993,"neg_zero":-0,"long":123456789012345678901234567890}
```

`exact` is one past 2^53 and `long` is thirty digits. Both stay in decimal. `neg_zero` stays `-0`.

```sh
tumulus -c '[.limits.tiny, .limits.huge]' tests/fixtures/settings.json
```

```json
[5E-324,1.7976931348623157E+308]
```

```sh
tumulus -c '.labels | {tr, jp, emoji}' tests/fixtures/settings.json
```

```json
{"tr":"Şehir İçi Ölçüm","jp":"潮位計","emoji":"🌊 high tide"}
```

## What works

Input is standard input or files. `-` is standard input. With no filter, the filter is `.`. The files are one stream. An array or an object can open in one file and close in a later one. A value already written is kept when that joined text is not valid JSON. The message is jq's parse error and the exit status is 5.

Printing:

- The default layout is pretty, indented two spaces.
- `-c` and `--compact-output` print compact JSON.
- `--tab` indents with tabs. `--indent N` follows jq: `-1` is tabs, `0` is compact, and `1` through `7` are that many spaces.
- `-r` and `--raw-output` print a top-level string without quotes.
- `-j` and `--join-output` do the same and leave off the newline. `-j` implies `-r`.
- `-a` and `--ascii-output` escape non-ASCII as `\uXXXX`.
- `-S` and `--sort-keys` sort object keys when printing.
- `-C` and `--color-output` color JSON. `-M` and `--monochrome-output` do not. `-M` wins when both are set. With neither, color is used when stdout is a terminal, unless `NO_COLOR` is set to something other than empty. `JQ_COLORS` is eight colon-separated slots, for null, false, true, numbers, strings, arrays, objects, and keys. A bad value prints `Failed to set $JQ_COLORS` and the default colors are kept.
- `--seq` reads and writes application/json-seq. A JSON text is preceded by an ASCII record separator. A raw string is not. A bad record prints `jq: ignoring parse error` and reading continues.
- `--raw-output0` implies `-r` and ends each output with a NUL byte. A string that contains a NUL is rejected.
- `--unbuffered` flushes after each output.
- `-V` and `--version` print the version and exit 0. The filter is not run.
- `-h` and `--help` print help to stdout and exit 0. The filter is not run. A short cluster that contains `h`, such as `-hx` or `-Vh`, is help, and the other letters in that cluster are not read.
- `--build-configuration` prints `oniguruma=6.9.10` and exits 0. The filter is not run.
- The first of `-h`, `--help`, `-V`, `--version`, and `--build-configuration` wins. Later arguments are not read. After `--`, none of them is an option.
- An unknown option exits 2. The message is `jq: Unknown option ...`, then jq's usage text. A short cluster names the whole argument, as in `-Z` and `-eQ`.

Numbers are rewritten on the way out, including spellings such as `01`, `.1`, `+1`, and `1.`. `NaN` and `Infinity` are accepted. A leading UTF-8 BOM is ignored. Nesting deeper than 256 is rejected on input. A value built deeper than that still compares with `==`. `tojson`, `tostring`, and printing write `<skipped: too deep>` in place of anything past that depth.

Filters that work are field access (`.a`, `.a.b`, `.["a"]`, `."a b"`), array indexes (`.[0]` and negative indexes), slices (`.[2:5]`, `.[:-1]`, `.[$i:]`, `.[:length-1]`, `.[index("b"):]`), several keys or indexes in one bracket (`.["a","b"]`), iteration (`.[]`), recurse (`..`), pipe (`|`), comma (`,`), two filters written next to each other, parentheses, and the `?` suffix.

A program can also contain numbers, strings, `true`, `false`, and `null`. Strings take the usual escapes and `\( )`. Arrays are `[ ]`. Objects are `{ }`, including `{a}` and `{"a"}` as short field copies, `{"a\(1)"}`, and `{(expr): value}`. A `#` comment runs to the end of the line. `-n` and `--null-input` run the program once on `null` and do not read input.

An object key that is not a string fails the way jq fails. A bad string escape does too.

`+`, `-`, `*`, `/`, and `%` work on numbers. `+` also joins strings and arrays and merges objects. `-` removes elements from an array. `*` repeats a string, and it merges objects when both values of a key are objects. `/` splits a string. `nan` and `infinite` are numbers. Comparisons are `==`, `!=`, `<`, `<=`, `>`, and `>=`. The order is null, then false, then true, then numbers, strings, arrays, and objects. `and`, `or`, and `not` yield booleans. `//` is the alternative. `if` uses `then`, any number of `elif`, an optional `else`, and `end`.

An operator that is given values it cannot use fails the way jq fails, including division and remainder by zero. The exit status is 5.

`$name` reads a variable. `as` binds a value for the rest of that expression. The pattern can take an array or an object apart, as in `. as [$a, $b]`, `. as {a: $x}`, and `{$a}` for `{a: $a}`. `?//` supplies another pattern. It is tried when the pattern cannot match, or when the rest of the expression fails. `1 | . as [$a] ?// $a | $a` is `1`. A name that the successful pattern does not bind is null, and that null hides an outer binding of the same name. Every name from every pattern is in scope. A pattern that matches nothing, such as a key filter with no values, does not try the next pattern. `halt` and `halt_error` are not tried again. The last failure is kept, and the exit status is 5. `reduce` and `foreach` take the same alternatives and walk a stream with that kind of pattern. A pattern that meets the wrong kind of value fails with jq's index message. The exit status is 5. Inside `reduce`, `. + [expr]` appends to an array, `.[expr] = expr` and `.[expr] |= expr` set a key or an index, and `setpath` writes a path, without copying the whole value on every step. A nest of arrays and objects that contains one `.`, with literal strings as object keys and literal values everywhere else, moves the accumulator into that `.`. `[.]`, `{a:.}`, `[., 1]`, and `[[.]]` are that shape. Copying an array or an object shares it until a later write. `reduce range(25) as $i (0; [., .])` and `[.] | [.]` stay one step each, and `..` walks a deep value without copying every level. `foreach` reuses that accumulator for `. + [expr]` and `.[expr] |= expr`, and yields each step before it builds the next one. A step that is saved, as in `[foreach]`, is copied. A `foreach` inside a function is copied too, because the call hands back each step.

`empty` yields no values. `length` is the number of Unicode code points in a string, the number of elements in an array, and the number of keys in an object. Null has length 0. The length of a number is its absolute value. `keys` is the indexes of an array, or the keys of an object in order. `keys_unsorted` is the same for an array. An object's keys stay in the order they are stored, and a repeated name keeps the first place. `has(expr)`, `select(expr)`, and `map(expr)` take that one filter. `select` repeats the input for each truthy result. `map` runs the filter on each array element or object value and collects the results. `map_values(expr)` runs the filter on each array element or object value. The first value replaces that element, no value removes it, and a later value is not run. The wrong kind of value fails the way jq fails, and the exit status is 5.

`split(sep)` splits a string on a string. An empty separator splits it into Unicode code points. An empty string becomes `[]`. Each value `sep` yields is one split. The input and the separator have to be strings, or the error is `split input and separator must be strings`. `fromjson` reads one JSON value from a string. `nan` and `NaN` are the number `nan`, in any letter case, and a leading `+` or `-` is allowed. On macOS, digits may follow, so `nan1234` is `nan`. On Linux, only zeros may follow, so `nan0` is `nan` and `nan1234` fails with `Invalid numeric literal`. The same spellings are accepted as input values. `Infinity`, `Inf`, and `infinity` are infinite, and a leading `+` or `-` is allowed on those words. A value that is not a string fails with `only strings can be parsed`. A string that is not one JSON value fails with jq's parse message followed by `(while parsing '...')`.

`contains(value)` is true when a string contains that string, and the empty string is contained. An array contains another array when every element of the second is contained in some element of the first. An object contains another object when every key of the second is present and that value contains the second's value. Numbers use `==`. `null` contains `null`. A boolean contains the same boolean. A different type, or the other boolean, fails with `cannot have their containment checked`. Inside an array or an object, that mismatch is false. `inside(value)` asks whether `value` contains the input. Each value the argument yields is one result, and a failure keeps the results already produced.

`flatten` pulls every nested array out. An object contributes its values. `flatten(n)` pulls `n` levels when `n` is an integer and at least 0. `infinite` pulls every level. A positive number that is not an integer pulls every level. `nan`, `null`, a boolean, or a negative number fails with `flatten depth must not be negative`, before the value is iterated. A string, array, or object depth is subtracted from when a nested array is entered, and that fails the way subtraction fails. A value that cannot be iterated fails with `Cannot iterate over`.

`any` and `all` take no filter, one filter, or a generator and a condition. With fewer than two, the generator is `.[]`. `any` is true when a condition value is truthy. `all` is true when every condition value is truthy, and it is true when there is nothing to test. A condition that yields nothing does not decide the result. The first answer stops the rest, so a later `input` or `error` is not run. `isempty(expr)` is true when `expr` yields nothing, and a value stops it before a later one is run. `any` and `all` are paths when the value they stop on was reached through the input. `path(map_values(expr))`, assignment through `map_values`, and `del(map_values(expr))` fail the way jq fails. The wrong kind of value fails the way jq fails, and the exit status is 5.

`values` keeps a value other than null. `nulls` keeps null. `booleans`, `numbers`, `strings`, `arrays`, and `objects` keep a value of that type. `iterables` keeps an array or an object. `scalars` keeps every other value, including null. `IN(expr)` is true when the input equals a value `expr` yields. `IN(src; expr)` is true when a value of `src` equals a value of `expr`. The second filter is the outer loop, and the first match stops the rest, so a later `input` or `error` is not run. `INDEX(idx)` builds an object from the values inside the input. `INDEX(stream; idx)` builds it from that stream. `idx` runs on each value. A string is the key as written. Any other value's key is its compact JSON, so `1` and `1.0` stay different keys, and `nan` uses the key `null`. A repeated key stays where it first appeared and takes the later value. A value that yields no key is left out. An `idx` that uses a variable sees the caller's binding. A value that cannot be iterated fails the way jq fails, and the exit status is 5.

`type` names the value: `null`, `boolean`, `number`, `string`, `array`, or `object`. `tonumber` leaves a number as it is and reads a string as one JSON number. `tostring` leaves a string as it is and writes any other value as compact JSON. `join(expr)` walks an array, or the values of an object, and joins them with each value of `expr`. A null in that walk is empty, and a number or boolean is written as text. `ltrimstr(expr)` and `rtrimstr(expr)` drop one leading or trailing copy of a string. `tonumber` and `join` fail on a value they cannot use, the way jq fails, and the exit status is 5. `type`, `tostring`, `ltrimstr`, and `rtrimstr` do not.

`ascii_downcase` and `ascii_upcase` change only ASCII letters. `explode` writes the code points of a string. `implode` reads those code points back. A code point that does not fit, or a UTF-16 surrogate, becomes U+FFFD. A fraction is truncated toward zero. `startswith(s)` and `endswith(s)` report whether a string has that prefix or suffix. `index(s)`, `rindex(s)`, and `indices(s)` find `s` in a string or an array. A string search counts UTF-8 bytes and can overlap. An array search for an array looks for that sequence of elements. On any other value, `indices(s)` is `.[s]`, `index(s)` keeps the element at 0, and `rindex(s)` keeps `.[-1:][0]`. `utf8bytelength` is the number of UTF-8 bytes in a string. A value of the wrong kind fails the way jq fails, and the exit status is 5.

`@text` writes a value the way `tostring` does. `@json` writes it the way `tojson` does. `@html` escapes `&`, `<`, `>`, `'`, and `"` in that text, and a NUL becomes `\0`. `@uri` percent-encodes every UTF-8 byte except letters, digits, and `-._~`. `@base64` is that text in standard base64, with `=` padding. `@base64d` reads it back. A character outside the alphabet fails with `is not valid base64 data`. One leftover character fails with `trailing base64 byte found`. Bytes that are not UTF-8 become U+FFFD. `@csv` and `@tsv` require an array. Null and `nan` are empty fields. A string in `@csv` is quoted, and a quote inside it is doubled. `@tsv` escapes `\`, newline, carriage return, tab, and NUL, and does not quote the field. An array or object in a row fails with `is not valid in a csv row`, whether the format is `@csv` or `@tsv`. `@sh` quotes a string for the shell. Numbers, booleans, and null are written as text with no extra quotes. An array of those values is joined with spaces. An object, or an array inside that array, fails with `can not be escaped for shell`. `@name "text \(expr)"` formats each interpolation and leaves the surrounding text as it is. The rightmost interpolation is the outer loop. A name other than these nine fails with `name is not a valid format` when a value is formatted. The exit status is 5.

`floor`, `ceil`, `round`, `trunc`, `nearbyint`, and `rint` round a number. `fabs` is its absolute value. `sqrt` and `cbrt` are roots. `exp`, `exp2`, `exp10`, and `expm1` are exponentials. `log`, `log2`, `log10`, `log1p`, and `logb` are logarithms. `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `sinh`, `cosh`, `tanh`, `asinh`, `acosh`, and `atanh` are the trigonometric functions. `gamma` and `tgamma` are the gamma function, and `lgamma` is its log. `erf` and `erfc` are the error function. `j0`, `j1`, `y0`, and `y1` are Bessel functions. `significand` is the significand. `frexp` returns `[mantissa, exponent]`. `modf` returns `[fraction, integer]`. `pow(x; y)`, `atan2(y; x)`, `hypot`, `copysign`, `fdim`, `fmax`, `fmin`, `fmod`, `remainder`, `drem`, `nextafter`, `scalb`, `scalbln`, `ldexp`, `jn`, and `yn` take two filters. `fma(x; y; z)` takes three. The rightmost filter runs outermost, and each filter to its left runs again for every value. A value that is not a number fails with `number required`, and the exit status is 5. `nan` stays a number and prints as `null`. A call with the wrong number of filters is rejected with jq's `name/n is not defined` message.

`abs` is `if . < 0 then -. else . end`. A string, array, or object is left as it is. `null` and a boolean fail with `cannot be negated`. `lgamma_r` returns `[log-gamma, sign]`. `nexttoward(x; y)` is the next number after `x` in the direction of `y`. A value that is not a number fails with `number required`, and the exit status is 5. `pow10` fails with `Error: pow10/0 not found at build time` and exit status 5. The input is not examined.

`todate` writes a number of seconds since the epoch as UTC `YYYY-MM-DDTHH:MM:SSZ`. `fromdate` reads that text back into seconds. `gmtime` breaks a number of seconds into `[year, month, day, hour, minute, second, weekday, yearday]` in UTC. `localtime` breaks a number of seconds into that array in local time. The month and the weekday start at 0. `mktime` turns that array back into seconds. `strftime(fmt)` formats a number of seconds in UTC, or that array as written. `strflocaltime(fmt)` formats a number of seconds in local time, or that array as written. `strptime(fmt)` reads text in that format back into the array. `now` is the current number of seconds since the epoch. The format letters are `%Y`, `%m`, `%d`, `%H`, `%M`, `%S`, `%Z`, `%j`, `%a`, `%A`, `%b`, `%B`, `%e`, `%y`, `%z`, `%I`, `%p`, `%u`, `%w`, `%s`, `%D`, `%F`, `%T`, `%R`, `%c`, `%C`, `%g`, `%G`, `%h`, `%k`, `%l`, `%n`, `%r`, `%t`, `%U`, `%V`, `%W`, `%x`, `%X`, `%P`, and `%%`. `%A` and `%B` are the full weekday and month. `%I` is the hour from 01 to 12 and `%p` is the morning or afternoon mark. `%u` counts Monday as 1 and `%w` counts Sunday as 0. `%s` is seconds since the epoch. `%D`, `%F`, `%T`, and `%R` are the short date, the calendar date, the time, and the hour and minute. `%c` is the locale's date and time. `%%` is a percent sign. `%C` is the century. `%g` is the week-based year without the century and `%G` is the week-based year. `%h` is the abbreviated month. `%k` is the hour from 0 to 23 and `%l` is the hour from 1 to 12, each padded with a space. `%n` is a newline and `%t` is a tab. `%r` is the 12-hour clock. `%U` is the week number with Sunday as the first day, `%W` uses Monday, and `%V` is the ISO week. `%x` is the locale's date and `%X` is the locale's time. `%P` follows the host C library. On macOS it is the letter `P`. `%+`, `%v`, a `-` or `_` flag, an `E` or `O` modifier, and any other character after `%` are given to that library too. On macOS, `%+` is the date and time, `%v` is the day, abbreviated month, and year, `%-d` has no padding, `%_H` is padded with a space, `%Ey` and `%Od` are the year in the century and the day, and `%Q` is printed without the percent sign. A trailing `%` is a percent sign. `strptime` reads those same formats. Text that does not match is an error and the exit status is 5. `strftime`, `strflocaltime`, and `strptime` take the locale from `LC_ALL` when that names a locale, otherwise from `LC_TIME`, otherwise from `LANG`. A name the system does not have leaves the C locale. A value of the wrong kind fails the way jq fails, and the exit status is 5.

`=` writes a value through a path. `|=` runs a filter on the value there and writes the first result back. No result removes that place, which is also what `del(path)` does. `+=`, `-=`, `*=`, `/=`, `%=`, and `//=` combine the value already there with the right-hand side. A path is a field, an index, a slice, or iteration. It can go through `|`, `,`, `select`, `//`, `if`, and `as`. `and`, `or`, and a matching `null`, `true`, or `false` stay paths when they do in jq. An update that lands on the wrong kind of value fails the way jq fails, and the exit status is 5.

Each side of a slice is an expression, so a variable, arithmetic, or a call works there. `.[$i:]`, `.[:length-1]`, and `.[index("b"):]` read, assign, and delete the way a written number does. An object in the brackets, `.[ {start:1, end:3} ]`, is that same slice when both `start` and `end` are present. Null on either side is the omitted bound. A later repeated name wins. Any other object fails with `Array/string slice indices must be integers` on an array or a string. On null it is null. On any other value it fails with `Cannot index ... with object`. The bounds are run on the value the brackets were applied to, and the value before the brackets is sliced once for every combination. A comma on one side runs every combination, the start varying slowest. Reading yields one slice of the original value for each combination. Assignment applies those slices from left to right on the array as it changes. `del` removes them from the original positions. A bound that is not a number or null fails with `Array/string slice indices must be integers` on an array or a string. On null, that check waits until the value is written.

`path(expr)` is the path from the input to each value `expr` reaches, as an array. A string is a field, a number is an index, and an object is a slice. `paths` lists every path inside the input except the empty path to the input itself. `paths(expr)` keeps a path once for each truthy value `expr` produces. `getpath(path)` reads the value at one path. `setpath(path; value)` writes it, and builds the missing objects and arrays along the way. `delpaths(paths)` removes every path in one array. A path jq refuses, or a value of the wrong kind, fails the way jq fails, and the exit status is 5. `getpath` works on the left of an update. `setpath([nan]; value)` on an array or null fails with `Cannot set array element at NaN index`. A slice bound may be `nan` or `infinite`. `nan` on the left of `:` is the start of the value, and on the right it is the end. `infinite` is the end and `-infinite` is the start. Setting `-infinite`, or another negative index that does not fit such as `-1e300`, fails with `Out of bounds negative array index`. Removing that index leaves the value as it is.

`tostream` writes the input as a stream of paths. A scalar, or an empty array or object, is one pair of the path and the value. Any other array or object is the stream of what is inside it, then one closer, the path of its last child. `nan` is still a number and prints as `null`. `fromstream(stream)` builds values back from that stream. The argument is the stream, and the filter's own input is ignored. A pair whose path is empty is emitted at once and replaces whatever was being built. A closer whose path has one component emits the value so far and the next item starts over. A closer with any other path does not emit. An item that is not a two-element array is a closer, and its first element is the path. A value with no closer and no empty path is not emitted. `truncate_stream(stream)` keeps each item whose path is longer than the input, and drops that many components from the front of the path. The argument runs on `null`, not on the input. The wrong kind of value fails the way jq fails, and the exit status is 5.

`--stream` reads input as those path items, including items that were finished before a value was cut off. A finished item is printed while the rest of the input is still being read. A repeated object key is kept each time it appears. `-s` gathers the items into one array. It still runs the program on a cut-off input when that input ends on a comma and every item is a scalar pair. When the input does not end in whitespace and the last closer names a scalar, a runtime error for that closer is reported at `<unknown>`. With `-s`, that closer is its own value and the other items are the array. `-R` still reads lines. `-n` still starts from `null`, and `input` or `inputs` then reads the items. The first item is the main input unless `-n` is set. `--stream` does not stop at a nesting depth of 256. `--stream-errors` parses the same way and, where `--stream` would stop with a parse error, hands the program one more value, `[message, path]`, and keeps reading. The exit status is 0 unless the program itself fails. On macOS, a NUL with no newline after it ends the JSON input, and the bytes after it are ignored. On Linux, and on macOS when a newline does follow, a NUL ends the token before it and the bytes up to the next delimiter are not part of that value. A number keeps the text before the NUL. `true`, `false`, `null`, and a token that starts on a NUL do not. `-R` still keeps the byte.

`limit(n; expr)` keeps values from `expr`. A count of 0 keeps nothing and does not run `expr`. A negative count, `nan`, `null`, or a boolean keeps the whole stream. A positive finite count keeps the ceiling of that many and stops before a later value is run. A count that is not a number fails when it is subtracted from, unless the stream fails first. `first` is `.[0]` and `last` is `.[-1]`. `nth(n)` is `.[n]`. `first(expr)` is the first value of `expr` and does not run the rest. `last(expr)` is the last value, or `null` when there is none. `nth(n; expr)` picks one value. A negative `n`, `nan`, `null`, or a boolean fails with `nth doesn't support negative indices`. Past the end of the stream there is no value. `range(to)` counts from 0 up to but not including `to`. `range(from; to)` steps by 1. `range(from; to; step)` uses that step. A step of 0 yields nothing. A bound that is not a number fails with `Range bounds must be numeric`. With one or two arguments, an end of `nan` does not stop the count. `until(cond; next)` yields the value once `cond` is truthy. `while(cond; update)` yields the value while `cond` is truthy, then applies `update`. Each result of a condition or an update is its own branch. `repeat(expr)` runs `expr` on the original input, then repeats. `first`, `last`, `nth(n)`, `first(expr)`, `nth(n; expr)`, and `limit(n; expr)` are paths when the value they select was reached through the input. Setting a `nan` index fails with `Cannot set array element at NaN index`. The exit status is 5.

`def name: body; rest` defines a function. `def name(a; b): body; rest` takes filters. Each use runs that filter again, on the value at the use, and an unused filter is not run. `def name($a; $b): body; rest` binds values before the body runs. The arguments run in the caller's scope, the leftmost varies slowest, and the body runs once for each combination. The same name without `$` runs that argument again at each use, and `.` there is the value at the use. `$name` is still the value bound for that combination. A `$` binding from `as` is not a filter. A function defined inside another one sees the outer function's arguments. An inner function of the same name hides a parameter, and a parameter hides an outer function of the same name. In one parameter list the later parameter of a name hides the earlier one. The function can call itself a million levels deep, including when the call sits under a pipe or `+`. A later function of the same name and the same number of arguments hides the earlier one from there on. `true`, `false`, and `null` stay literals. `nan` and `infinite` do not, when a parameter or a function of that name is in scope. A call of a name or a number of arguments that is not defined fails with jq's `name/n is not defined` message, and the exit status is 3.

`recurse` and `..` yield a value and then each array element or object value inside it, in the order the values are stored. Null is included. A string, number, or boolean has nothing inside. `recurse(f)` yields the value, then each value `f` yields, and walks those depth first before the next one. `recurse(f; cond)` enters such a value once for each truthy result of `cond`. The value itself is yielded before `cond` runs. `limit` and `first` stop the walk before a later value is produced. As a path, `recurse(f)` includes the current value and then each value `f` reaches by indexing. `cond` decides whether that value is entered. A computed value is an invalid path. `walk(f)` rebuilds arrays and objects from the bottom, then runs `f`. An array keeps every value a child yields. An object keeps the first value of a field and does not run the rest of that field, and a field that yields nothing is left out. `to_entries` writes an array of `{key, value}` in stored order. An array's keys are numbers. `from_entries` reads those objects back. The key is `key`, then `Key`, then `name`, then `Name`. Null and false are skipped, so a null `key` can fall through to `name`. `k` is not a key. A missing value is null. A `value` that is present and null stays null and does not fall through to `Value`. `with_entries(f)` runs `f` on each of those objects and reads the result back. `tojson` writes compact JSON. A string is quoted, unlike `tostring`. `add` is `reduce .[] as $x (null; . + $x)`. An empty array or object is null. Adding strings joins them. Adding objects merges them, and the later value of a key wins. Null, a string, a number, or a boolean has nothing to walk, and that fails the way jq fails. A value of the wrong kind fails the way jq fails, and the exit status is 5.

`sort` orders an array. The order is the comparison order: null, then false, then true, then numbers, strings, arrays, and objects. Equal values stay in the order they arrived. `sort_by(f)`, `group_by(f)`, `unique_by(f)`, `min_by(f)`, and `max_by(f)` take that one filter. `f` runs once on each element, from left to right, and every value it yields is part of the key. `sort_by(.a, .b)` compares `.a` and then `.b`. `group_by(f)` keeps each run of equal keys as an array, in the order the values arrived inside the run. `unique` and `unique_by(f)` keep the first value of each run. `min` and `min_by(f)` keep the earliest value when the keys compare equal. `max` and `max_by(f)` keep the latest. `min`, `max`, `min_by`, and `max_by` of an empty array are null. `nan` compares less than itself, so `sort` and `sort_by` leave those values in the order they arrived, `min` keeps the last `nan`, and `max` keeps the first. `unique` and `group_by` keep every `nan`. `sort`, `unique`, `min`, and `max` take no argument. A value of the wrong kind fails the way jq fails, and the exit status is 5.

`try expr` keeps every value `expr` produced before it failed, then stops. That is the same as `try expr catch empty`. `try expr catch handler` runs `handler` on the error value instead. A builtin failure is caught as its message, including one raised inside a function. `error` raises the current input, and `error(expr)` raises the first value `expr` produces. When nothing catches the error, a string is printed as `jq: error (at ...): ` and that text. Any other value is printed as `jq: error (at ...) (not a string): ` and the compact JSON. The exit status is 5 when that input is the last one. `?` is that empty catch, except when it is written immediately after `]`. Then a failed index or slice is skipped, and an error raised while computing a bound is still reported. `[1,2,3] | .[1,error("z"):]?` prints `[2,3]` and then `z`, and the exit status is 5. `(.[1,error("z"):])?` catches that error. A second `?` catches it too.

`label $name | expr` runs `expr` where `break $name` can leave it. Values already produced stay, and nothing after the break in that label is run. `break` raises `{"__jq": n}`. `n` counts labels from 0 in the order they are entered, and the count continues across inputs of one run. `try` and `?` can catch that object. A label catches it when nothing else does, including `error({"__jq": n})` for its own number. An object with any other field is not that break. `break` uses the label visible where it is written, so a function keeps the outer label when an inner one reuses the name. `$name` after `label` is not a value. A `break` that can run with no visible label fails to compile. The message is `$*label-name is not defined`, and the exit status is 3. A function that is never called is not checked. With `-e`, a label that produces nothing exits 4.

`input` reads the next value. `inputs` reads every value still waiting. With `-n`, nothing is read until one of those runs. At the end of the input, `input` fails with `break` and `inputs` stops. A value continues from one file into the next: `1`, a newline, and `2` in the first file and `3` in the second are `1` and `23`. A value that is not valid JSON fails that read with jq's parse message. If the failure is caught, a later `input` continues on the next line. A missing file is named, later files are still read, and the exit status is 2. `input_filename` is the name of the file the current value came from, or null before any value has been read. With `-s`, the stream is one array of every value. With `-R`, each line is one string.

`$__loc__` is an object `{"file":"<top-level>","line":N}`. `N` is the line of that `$`. A program read with `-f` still says `<top-level>`. `{$name}` is `{name: $name}`. `{$name: value}` uses each value of `$name` as one object key. A key that is not a string fails with `Cannot use ... as object key` when a value is produced, and the exit status is 5. A value that yields nothing produces no object, and a bad key is not reported. `{$__loc__}` is `{__loc__: $__loc__}`. A colon after `$__loc__` is a syntax error. `if`, `and`, `reduce`, `foreach`, and the other words the language uses are ordinary names as an object key and after a dot, as in `{if: 1}`, `{reduce}`, and `.foreach`. An unbound `$name` fails to compile with `$name is not defined`, and the exit status is 3.

`$ENV` is the process environment, with each value a string. `env` reads that same object. Binding `$ENV` again with `as` hides it for `$ENV` and leaves `env` alone. `--arg ENV` stores its text on `$ARGS.named` and does not replace `$ENV`.

`--args` puts each later file argument into `$ARGS.positional` as a string. `--jsonargs` parses each one as one JSON value. A value that is not one JSON value exits 2 with `jq: invalid JSON text passed to --jsonargs`. When both are given, the later one decides what follows it. Arguments written before it are still files. The program itself is not a positional value.

`--slurpfile name path` binds `$name` to every JSON value in that file, as one array. `--rawfile name path` binds `$name` to the bytes of that file as one string. The name is also a field of `$ARGS.named`. A missing file, a directory, or a value that is not valid JSON exits 2. The message names the option, the name, and the path. The first binding of a name wins, and a repeated name does not read its file. `-` is a file name here, not standard input.

A missing file is named and the other files are still read. A directory is not a value. The exit status is 2, including when a later value fails. `jq: error: Is a directory` is printed when another file comes after it, and not when the directory is the last file. `-n` does not open the files. A filter that hits the wrong kind of value uses jq's message. The exit status is 5 when that value is the last one, and 0 when a later value is fine. Output already produced is kept, and later values are still run. On one input, a cut-off final value still prints the complete values before it, then jq's parse error, and exits 5.

`isnan` is true for `nan` and false for every other value, including one that is not a number. `isinfinite` is true for an infinite number. `isnormal` is true for a finite number other than zero or a subnormal. `isfinite` is true for every other number, including `nan`. `finites` keeps those numbers. `normals` keeps the normal ones. `halt` stops the program with status 0 and writes nothing more. `halt_error` stops with status 5. `halt_error(code)` uses the first number `code` yields. The fraction is dropped. A negative number or `nan` is status 0. A number that fits in 32 bits keeps its low eight bits. A number that does not fit follows the host C cast, so a huge positive code is 255 on macOS and 0 on Linux. The input is written to stderr. A string is written as its bytes, `null` writes nothing, and any other value is compact JSON plus a newline. `-c`, `-r`, `-j`, `-S`, and `-a` do not change that text. `try` does not catch `halt` or `halt_error`. `pick(paths)` copies each of those paths onto a value that starts as null. `transpose` turns the rows of an array into columns and fills a short row with null. `combinations` takes one element from each array in the input. `combinations(n)` repeats the input once for every value `range(n)` yields.

`debug` writes `["DEBUG:", value]` to stderr and returns the value. `debug(msgs)` does that for each message, then returns the input. The debug text is compact. `-S` sorts its keys and `-a` escapes non-ASCII. Other output flags change only stdout. `stderr` writes the value there with no added newline and returns it. A string is its bytes. Any other value is compact JSON, and `-S` and `-a` do not change that text. `input_line_number` is how many newlines the reader has consumed. It is 0 before a newline, it stays put when a value ends without one, and it starts over in the next file. After `input` or `inputs` reads past the last value, the count stays if the file ended on a newline, and it is 0 when that value was completed at the end of the file. A newline with no value still counts. With `-s`, the count is 0 once `inputs` has taken the array. `builtins` lists each builtin as `name/arity`, in jq's order. `get_jq_origin` is the directory of the program as it was invoked. A name with no slash is `.`. `get_prog_origin` is the directory of the `-f` file, or the current directory when the program is an argument.

`reverse` reverses an array. An empty array, an empty object, `null`, and `""` become `[]`. A non-empty string, a number, a boolean, or an object that has keys fails the way `length` or indexing fails. `in(xs)` is `has` for the input, once for each value `xs` yields. `bsearch(target)` searches an array. An empty array is `-1`. One element is `0` when it matches, `-1` when the target is smaller, and `-2` when it is larger. A longer array returns an index, which need not be the first copy of a repeated value, or a negative number for a value that is absent. `JOIN` takes an index and an index expression. Two arguments walk the input and return one array of pairs. Three arguments walk a stream and return each pair. Four arguments run one more filter on each pair. A missing object key is null. `format(name)` formats the input once for every name `name` yields, using the same names as `@text` and the other `@` filters. A bad name fails, names after it are not used, and names that already worked are kept. `fromdateiso8601` reads `YYYY-MM-DDTHH:MM:SSZ` as seconds since the epoch. `todateiso8601` writes a number of seconds in that form. `nan` is the epoch. An infinite number fails.

`test`, `match`, and `capture` search with Oniguruma's Perl NG patterns. One argument is a string, or an array `[regex]` or `[regex, flags]`. Anything past the flags is ignored. An empty array, or a value that is neither a string nor a non-empty array, fails with `not a string or array` before the input is checked. Two arguments are the pattern and the flags, and the pattern must be a string. The input is checked next. A value that is not a string fails with `cannot be matched, as it is not a string`. A pattern or flags value that is not a string fails with `is not a string`. Null flags mean there are none. A flag character other than `g`, `i`, `m`, `n`, `p`, `s`, `l`, or `x` fails with `is not a valid modifier string`, and the message is the whole flags string. A pattern that does not compile fails with `Regex failure:` and Oniguruma's message. The exit status is 5. Values already produced are kept.

`g` finds every match that does not overlap. Without it, only the first is kept. An empty match advances one byte, so a character that occupies more than one byte can share an offset. `i` ignores case. `m` lets `.` match a newline. `s` does not. `p` turns on both, and `.` follows `m`. `n` drops empty matches. `l` keeps the longest match in what remains, then `g` continues after it. `x` ignores whitespace and `#` comments in the pattern. Inline `(?i)`, `(?-i)`, `(?m)`, `(?s)`, and `(?x)` are the pattern's own options. `test` yields one boolean. `match` yields one object per kept match, with `offset`, `length`, `string`, and `captures`, counted in code points. A capture that took characters has `length` before `string`. One that took none, or that did not match, has `string` before `length`. An unset capture has offset -1 and string null. Inside an empty match, an unset capture is an empty string at that match's offset. `capture` yields one object per match, with each named group in the order the name is first seen. A later group with the same name replaces the value. A name that did not match is null. A match with no named group yields `{}`. No match yields nothing. The flags are the outer loop. `first` and `limit` do not read a later pattern or flag. `path(match(...))` and `path(capture(...))` fail while iterating the match array. `path(test(...))` fails with the boolean.

`scan` yields each match. `g` is added in front of the flags, and null adds nothing, so `"g" + null` stays `"g"`. A pattern with a capture yields an array of the capture strings. Null is a capture that did not match. A pattern with no capture yields the matched string. No match yields nothing. `splits` yields the pieces between matches and adds `g` the same way. No match yields the whole string. `split` with two arguments collects those pieces into one array. `split` with one argument still splits on a string. `sub` replaces the first match. `gsub` adds `g` at the end of the flags and replaces every non-overlapping match. The replacement is a filter. It sees one object of the named captures, and each value it yields is one result. A match that is not found leaves the string unchanged. `sub` passes its flags through. A bad letter is that string for `sub`, the string with `g` in front for `scan`, `splits`, and `split` with two arguments, and the string with `g` at the end for `gsub`. Joining a flags value that is not a string fails as addition, except null, which adds nothing. When the flags are already a string, the input is checked first. A value that is not a string fails the way `match` fails, and the exit status is 5. Values already produced are kept. `first` and `limit` do not read a later value argument. `path` fails while iterating the match array, including through the array or `reduce` that holds it.

`import "path" as name` defines `name::` functions from that module. `include "path"` defines those functions under their own names. A later definition of the same name is the one that runs. A function inside the module still calls the module's own definition. `import "path" as $name` reads every JSON value in the file into one array, and `$name::name` is that array. Code is a `.jq` file. Data is a `.json` file. `path/name.jq` is used when `path.jq` is absent, and the same rule applies to `.json`. The working directory is searched first. It is not part of the list `get_search_list` prints. With no `-L`, that list is `~/.jq`, `$ORIGIN/../lib/jq`, and `$ORIGIN/../lib`. `$ORIGIN` is the directory of the program as it was invoked. `-L directory` replaces the list. A directory that exists is stored as its canonical path, and `-L` without a directory exits 2. A path containing `..` is rejected. A `search` value on `import` or `include` replaces the working directory and `-L` for that one directive. A string is one directory. `~/` and `$ORIGIN/` expand. A bare `~`, and `$ORIGIN` not followed by `/`, are ordinary relative names. An array is directories in order, and a value in it that is not a string is skipped. `null`, a number, a boolean, an object, or `[]` finds nothing. The string `"."` is the working directory. Any other string is relative to the file that contains the directive. A program typed on the command line has no file, so that string is relative to the working directory. `"./"` and an empty string name the file's directory. `..` in the search directory is followed. `module` at the top of a file takes one constant object. `modulemeta` reads the module named by its input from the library path, not from the working directory, and reports that object's `deps` and `defs`. `search` is kept as it was written. A missing module is an error. A module that imports itself is rejected. The main program has to end with a filter. A library file may contain only definitions.

A program that ends after an operator, such as `1 +` or `1 *`, is a syntax error. So is one that stops early, such as `(1`, `.[`, or `[1,`. `[1,2` is a syntax error when the `]` is missing at the end. `1 2`, `{a:1+2}`, `if 1 then 2`, `.a }`, `"\(1 +)"`, and `def f: 1 +; f` are syntax errors. A `}` on its own is a syntax error. Each prints jq's message, and the exit status is 3. Of the 447 programs in jq 1.7.1's test file, 444 come out the same. The new one is line 1726, `}`. No previously matching program was lost.

## What does not work yet

Filters past the list above are rejected. A call to a name or a number of arguments this version does not define fails with jq's `name/n is not defined` message and the exit status is 3. Anything else that this version does not parse is `tumulus: syntax error`, also exit status 3. A comparison chain such as `1 < 2 < 3` is rejected that way. An unclosed `{` is rejected that way too.

A syntax error in a module file is `tumulus: syntax error`. jq prints its own parser message.

`path` of an object in the brackets, `.[ {start:1, end:3} ]`, prints that object, with `start` before `end`. When the first slice of an update cannot be written, a later bound is still run. jq does not run it. `try (.[1,(input):] = 9) catch input` therefore reads the next input here and does not in jq.

After a parse error, later files are not read, unless `input` or `inputs` caught the bad value and asked for another one, or `--stream-errors` is set.

## Time against jq

These times are from one machine: an Apple M4 Pro, 12 CPUs, 25769803776 bytes of memory, macOS 26.6.2, jq-1.7.1-apple. Each file was read once before its timed runs. Each command was `/usr/bin/time -p` with stdout discarded, three times, jq and then tumulus. Every run exited 0. A byte compare of the outputs matched.

`requests.ndjson` is 72727816 bytes and 300000 lines. Compact output was 72727816 bytes. Pretty output was 85878107 bytes.

| run | `jq -c .` | `tumulus -c .` | `jq .` | `tumulus .` |
| --- | ---: | ---: | ---: | ---: |
| 1 | 1.90 | 0.86 | 2.05 | 0.89 |
| 2 | 1.90 | 0.86 | 2.05 | 0.89 |
| 3 | 1.91 | 0.87 | 2.08 | 0.91 |

`--stream -c .`, `-sc .`, `-Rc .`, `-nc inputs`, and `--seq -c .` were timed the same way on that file. Stdout and stderr matched. `--stream -c .` wrote 90029174 bytes. `-sc .` wrote 72727818 bytes. `-Rc .` wrote 83184148 bytes. `-nc inputs` wrote 72727816 bytes. `--seq -c .` wrote no values. Both programs print `jq: ignoring parse error: Unfinished abandoned text at EOF at line 300001, column 0`.

| run | `jq --stream -c .` | `tumulus --stream -c .` |
| --- | ---: | ---: |
| 1 | 2.76 | 3.74 |
| 2 | 2.72 | 3.76 |
| 3 | 2.78 | 3.76 |

| run | `jq -sc .` | `tumulus -sc .` |
| --- | ---: | ---: |
| 1 | 2.00 | 1.36 |
| 2 | 1.99 | 1.36 |
| 3 | 1.99 | 1.34 |

| run | `jq -Rc .` | `tumulus -Rc .` |
| --- | ---: | ---: |
| 1 | 1.70 | 0.29 |
| 2 | 1.70 | 0.28 |
| 3 | 1.71 | 0.28 |

| run | `jq -nc inputs` | `tumulus -nc inputs` |
| --- | ---: | ---: |
| 1 | 2.00 | 0.87 |
| 2 | 2.01 | 0.87 |
| 3 | 1.96 | 0.88 |

| run | `jq --seq -c .` | `tumulus --seq -c .` |
| --- | ---: | ---: |
| 1 | 0.16 | 0.03 |
| 2 | 0.16 | 0.03 |
| 3 | 0.17 | 0.03 |

`citylots.json` is 189778220 bytes and 413125 lines. Compact output was 169215413 bytes. Pretty output was 395218854 bytes.

| run | `jq -c .` | `tumulus -c .` | `jq .` | `tumulus .` |
| --- | ---: | ---: | ---: | ---: |
| 1 | 3.59 | 2.33 | 7.18 | 2.47 |
| 2 | 3.53 | 2.36 | 7.23 | 2.48 |
| 3 | 3.59 | 2.35 | 7.24 | 2.49 |

The same five commands were timed on that file. Stdout and stderr matched. `--stream -c .` wrote 837003618 bytes. `-sc .` wrote 169215415 bytes. `-Rc .` wrote 200823780 bytes. `-nc inputs` wrote 169215413 bytes. `--seq -c .` wrote no values. Both programs print `jq: ignoring parse error: Unfinished abandoned text at EOF at line 413126, column 0`.

| run | `jq --stream -c .` | `tumulus --stream -c .` |
| --- | ---: | ---: |
| 1 | 18.94 | 37.57 |
| 2 | 18.65 | 38.16 |
| 3 | 18.95 | 38.40 |

| run | `jq -sc .` | `tumulus -sc .` |
| --- | ---: | ---: |
| 1 | 3.73 | 4.48 |
| 2 | 3.79 | 4.37 |
| 3 | 3.85 | 4.37 |

| run | `jq -Rc .` | `tumulus -Rc .` |
| --- | ---: | ---: |
| 1 | 4.16 | 0.60 |
| 2 | 4.04 | 0.60 |
| 3 | 3.99 | 0.62 |

| run | `jq -nc inputs` | `tumulus -nc inputs` |
| --- | ---: | ---: |
| 1 | 3.82 | 2.41 |
| 2 | 3.79 | 2.43 |
| 3 | 3.75 | 2.42 |

| run | `jq --seq -c .` | `tumulus --seq -c .` |
| --- | ---: | ---: |
| 1 | 0.43 | 0.06 |
| 2 | 0.43 | 0.06 |
| 3 | 0.43 | 0.09 |

`[.features[] | select(.properties.STREET == "UNKNOWN")] | length` reads that same file and prints `2843`. Compact and pretty printing of that number are the same text, so these runs used `-c`.

| run | `jq -c` | `tumulus -c` |
| --- | ---: | ---: |
| 1 | 1.68 | 1.33 |
| 2 | 1.69 | 1.32 |
| 3 | 1.71 | 1.32 |

Times are seconds.

## License

MIT. See `LICENSE`.
