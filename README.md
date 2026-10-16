# tumulus

tumulus reads JSON and prints it the way jq does. The filter language is still small. This is version 0.7.0.

macOS on Apple silicon:

```sh
curl -L -o tumulus https://github.com/memduhcelik/tumulus/releases/download/v0.7.0/tumulus-macos-arm64
chmod +x tumulus
```

Linux on x86_64:

```sh
curl -L -o tumulus https://github.com/memduhcelik/tumulus/releases/download/v0.7.0/tumulus-linux-x86_64
chmod +x tumulus
```

Build a release binary with `cargo build --release`. The binary is `target/release/tumulus`. The timed binary below was built with rustc 1.96.0 (ac68faa20 2026-05-25). `cargo test` runs the same inputs through tumulus and jq, and it fails if jq is missing.

`cargo test --test jq_suite -- --nocapture` runs every program and input from `tests/jq.test` through both and prints how many come out the same. That file is the `tests/jq.test` shipped with jq 1.7.1. The expected lines in it only mark the end of a case. The run asks jq each time. A mismatch is printed and does not fail the test. Of the 447 programs, 382 come out the same on stdout, stderr, and the exit status.

## What works

Input is standard input or files. `-` is standard input. With no filter, the filter is `.`.

Printing:

- The default layout is pretty, indented two spaces.
- `-c` and `--compact-output` print compact JSON.
- `--tab` indents with tabs. `--indent N` follows jq: `-1` is tabs, `0` is compact, and `1` through `7` are that many spaces.
- `-r` and `--raw-output` print a top-level string without quotes.
- `-j` and `--join-output` do the same and leave off the newline. `-j` implies `-r`.
- `-a` and `--ascii-output` escape non-ASCII as `\uXXXX`.
- `-S` and `--sort-keys` sort object keys when printing.

Numbers are rewritten on the way out, including spellings such as `01`, `.1`, `+1`, and `1.`. `NaN` and `Infinity` are accepted. A leading UTF-8 BOM is ignored. Nesting deeper than 256 is rejected on input. A value built deeper than that still compares with `==`. `tojson`, `tostring`, and printing write `<skipped: too deep>` in place of anything past that depth.

Filters that work are field access (`.a`, `.a.b`, `.["a"]`, `."a b"`), array indexes (`.[0]` and negative indexes), slices (`.[2:5]`, `.[:-1]`, `.[$i:]`, `.[:length-1]`, `.[index("b"):]`), several keys or indexes in one bracket (`.["a","b"]`), iteration (`.[]`), recurse (`..`), pipe (`|`), comma (`,`), two filters written next to each other, parentheses, and the `?` suffix.

A program can also contain numbers, strings, `true`, `false`, and `null`. Strings take the usual escapes and `\( )`. Arrays are `[ ]`. Objects are `{ }`, including `{a}` and `{"a"}` as short field copies, `{"a\(1)"}`, and `{(expr): value}`. A `#` comment runs to the end of the line. `-n` and `--null-input` run the program once on `null` and do not read input.

An object key that is not a string fails the way jq fails. A bad string escape does too.

`+`, `-`, `*`, `/`, and `%` work on numbers. `+` also joins strings and arrays and merges objects. `-` removes elements from an array. `*` repeats a string, and it merges objects when both values of a key are objects. `/` splits a string. `nan` and `infinite` are numbers. Comparisons are `==`, `!=`, `<`, `<=`, `>`, and `>=`. The order is null, then false, then true, then numbers, strings, arrays, and objects. `and`, `or`, and `not` yield booleans. `//` is the alternative. `if` uses `then`, any number of `elif`, an optional `else`, and `end`.

An operator that is given values it cannot use fails the way jq fails, including division and remainder by zero. The exit status is 5.

`$name` reads a variable. `as` binds a value for the rest of that expression. The pattern can take an array or an object apart, as in `. as [$a, $b]`, `. as {a: $x}`, and `{$a}` for `{a: $a}`. `reduce` and `foreach` walk a stream with that kind of pattern. A pattern that meets the wrong kind of value fails with jq's index message. The exit status is 5. Inside `reduce`, `. + [expr]` appends to an array, `.[expr] = expr` and `.[expr] |= expr` set a key or an index, and `setpath` writes a path, without copying the whole value on every step. A nest of arrays and objects that contains one `.`, with literal strings as object keys and literal values everywhere else, moves the accumulator into that `.`. `[.]`, `{a:.}`, `[., 1]`, and `[[.]]` are that shape. Copying an array or an object shares it until a later write. `reduce range(25) as $i (0; [., .])` and `[.] | [.]` stay one step each, and `..` walks a deep value without copying every level. `foreach` reuses that accumulator for `. + [expr]` and `.[expr] |= expr`, and yields each step before it builds the next one. A step that is saved, as in `[foreach]`, is copied. A `foreach` inside a function is copied too, because the call hands back each step.

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

`todate` writes a number of seconds since the epoch as UTC `YYYY-MM-DDTHH:MM:SSZ`. `fromdate` reads that text back into seconds. `gmtime` breaks a number of seconds into `[year, month, day, hour, minute, second, weekday, yearday]` in UTC. `localtime` breaks a number of seconds into that array in local time. The month and the weekday start at 0. `mktime` turns that array back into seconds. `strftime(fmt)` formats a number of seconds in UTC, or that array as written. `strflocaltime(fmt)` formats a number of seconds in local time, or that array as written. `strptime(fmt)` reads text in that format back into the array. `now` is the current number of seconds since the epoch. The format letters are `%Y`, `%m`, `%d`, `%H`, `%M`, `%S`, `%Z`, `%j`, `%a`, `%A`, `%b`, `%B`, `%e`, `%y`, `%z`, `%I`, `%p`, `%u`, `%w`, `%s`, `%D`, `%F`, `%T`, `%R`, `%c`, and `%%`. `%A` and `%B` are the full weekday and month. `%I` is the hour from 01 to 12 and `%p` is the morning or afternoon mark. `%u` counts Monday as 1 and `%w` counts Sunday as 0. `%s` is seconds since the epoch. `%D`, `%F`, `%T`, and `%R` are the short date, the calendar date, the time, and the hour and minute. `%c` is the locale's date and time. `%%` is a percent sign. A value of the wrong kind fails the way jq fails, and the exit status is 5.

`=` writes a value through a path. `|=` runs a filter on the value there and writes the first result back. No result removes that place, which is also what `del(path)` does. `+=`, `-=`, `*=`, `/=`, `%=`, and `//=` combine the value already there with the right-hand side. A path is a field, an index, a slice, or iteration. It can go through `|`, `,`, `select`, `//`, `if`, and `as`. `and`, `or`, and a matching `null`, `true`, or `false` stay paths when they do in jq. An update that lands on the wrong kind of value fails the way jq fails, and the exit status is 5.

Each side of a slice is an expression, so a variable, arithmetic, or a call works there. `.[$i:]`, `.[:length-1]`, and `.[index("b"):]` read, assign, and delete the way a written number does. The bounds are run on the value the brackets were applied to, and the value before the brackets is sliced once for every combination. A comma on one side runs every combination, the start varying slowest. Reading yields one slice of the original value for each combination. Assignment applies those slices from left to right on the array as it changes. `del` removes them from the original positions. A bound that is not a number or null fails with `Array/string slice indices must be integers` on an array or a string. On null, that check waits until the value is written.

`path(expr)` is the path from the input to each value `expr` reaches, as an array. A string is a field, a number is an index, and an object is a slice. `paths` lists every path inside the input except the empty path to the input itself. `paths(expr)` keeps a path once for each truthy value `expr` produces. `getpath(path)` reads the value at one path. `setpath(path; value)` writes it, and builds the missing objects and arrays along the way. `delpaths(paths)` removes every path in one array. A path jq refuses, or a value of the wrong kind, fails the way jq fails, and the exit status is 5. `getpath` works on the left of an update. `setpath([nan]; value)` on an array or null fails with `Cannot set array element at NaN index`. A slice bound may be `nan` or `infinite`. `nan` on the left of `:` is the start of the value, and on the right it is the end. `infinite` is the end and `-infinite` is the start. Setting `-infinite`, or another negative index that does not fit such as `-1e300`, fails with `Out of bounds negative array index`. Removing that index leaves the value as it is.

`limit(n; expr)` keeps values from `expr`. A count of 0 keeps nothing and does not run `expr`. A negative count, `nan`, `null`, or a boolean keeps the whole stream. A positive finite count keeps the ceiling of that many and stops before a later value is run. A count that is not a number fails when it is subtracted from, unless the stream fails first. `first` is `.[0]` and `last` is `.[-1]`. `nth(n)` is `.[n]`. `first(expr)` is the first value of `expr` and does not run the rest. `last(expr)` is the last value, or `null` when there is none. `nth(n; expr)` picks one value. A negative `n`, `nan`, `null`, or a boolean fails with `nth doesn't support negative indices`. Past the end of the stream there is no value. `range(to)` counts from 0 up to but not including `to`. `range(from; to)` steps by 1. `range(from; to; step)` uses that step. A step of 0 yields nothing. A bound that is not a number fails with `Range bounds must be numeric`. With one or two arguments, an end of `nan` does not stop the count. `until(cond; next)` yields the value once `cond` is truthy. `while(cond; update)` yields the value while `cond` is truthy, then applies `update`. Each result of a condition or an update is its own branch. `repeat(expr)` runs `expr` on the original input, then repeats. `first`, `last`, `nth(n)`, `first(expr)`, `nth(n; expr)`, and `limit(n; expr)` are paths when the value they select was reached through the input. Setting a `nan` index fails with `Cannot set array element at NaN index`. The exit status is 5.

`def name: body; rest` defines a function. `def name(a; b): body; rest` takes filters. Each use runs that filter again, on the value at the use, and an unused filter is not run. `def name($a; $b): body; rest` binds values before the body runs. The arguments run in the caller's scope, the leftmost varies slowest, and the body runs once for each combination. The same name without `$` runs that argument again at each use, and `.` there is the value at the use. `$name` is still the value bound for that combination. A `$` binding from `as` is not a filter. A function defined inside another one sees the outer function's arguments. An inner function of the same name hides a parameter, and a parameter hides an outer function of the same name. In one parameter list the later parameter of a name hides the earlier one. The function can call itself a million levels deep, including when the call sits under a pipe or `+`. A later function of the same name and the same number of arguments hides the earlier one from there on. `true`, `false`, and `null` stay literals. `nan` and `infinite` do not, when a parameter or a function of that name is in scope. A call of a name or a number of arguments that is not defined fails with jq's `name/n is not defined` message, and the exit status is 3.

`recurse` and `..` yield a value and then each array element or object value inside it, in the order the values are stored. Null is included. A string, number, or boolean has nothing inside. `recurse(f)` yields the value, then each value `f` yields, and walks those depth first before the next one. `recurse(f; cond)` enters such a value once for each truthy result of `cond`. The value itself is yielded before `cond` runs. `limit` and `first` stop the walk before a later value is produced. As a path, `recurse(f)` includes the current value and then each value `f` reaches by indexing. `cond` decides whether that value is entered. A computed value is an invalid path. `walk(f)` rebuilds arrays and objects from the bottom, then runs `f`. An array keeps every value a child yields. An object keeps the first value of a field and does not run the rest of that field, and a field that yields nothing is left out. `to_entries` writes an array of `{key, value}` in stored order. An array's keys are numbers. `from_entries` reads those objects back. The key is `key`, then `Key`, then `name`, then `Name`. Null and false are skipped, so a null `key` can fall through to `name`. `k` is not a key. A missing value is null. A `value` that is present and null stays null and does not fall through to `Value`. `with_entries(f)` runs `f` on each of those objects and reads the result back. `tojson` writes compact JSON. A string is quoted, unlike `tostring`. `add` is `reduce .[] as $x (null; . + $x)`. An empty array or object is null. Adding strings joins them. Adding objects merges them, and the later value of a key wins. Null, a string, a number, or a boolean has nothing to walk, and that fails the way jq fails. A value of the wrong kind fails the way jq fails, and the exit status is 5.

`sort` orders an array. The order is the comparison order: null, then false, then true, then numbers, strings, arrays, and objects. Equal values stay in the order they arrived. `sort_by(f)`, `group_by(f)`, `unique_by(f)`, `min_by(f)`, and `max_by(f)` take that one filter. `f` runs once on each element, from left to right, and every value it yields is part of the key. `sort_by(.a, .b)` compares `.a` and then `.b`. `group_by(f)` keeps each run of equal keys as an array, in the order the values arrived inside the run. `unique` and `unique_by(f)` keep the first value of each run. `min` and `min_by(f)` keep the earliest value when the keys compare equal. `max` and `max_by(f)` keep the latest. `min`, `max`, `min_by`, and `max_by` of an empty array are null. `nan` compares less than itself, so `sort` and `sort_by` leave those values in the order they arrived, `min` keeps the last `nan`, and `max` keeps the first. `unique` and `group_by` keep every `nan`. `sort`, `unique`, `min`, and `max` take no argument. A value of the wrong kind fails the way jq fails, and the exit status is 5.

`try expr` keeps every value `expr` produced before it failed, then stops. That is the same as `try expr catch empty`. `try expr catch handler` runs `handler` on the error value instead. A builtin failure is caught as its message. `error` raises the current input, and `error(expr)` raises the first value `expr` produces. When nothing catches the error, a string is printed as `jq: error (at ...): ` and that text. Any other value is printed as `jq: error (at ...) (not a string): ` and the compact JSON. The exit status is 5 when that input is the last one.

`input` reads the next value. `inputs` reads every value still waiting. With `-n`, nothing is read until one of those runs. At the end of the input, `input` fails with `break` and `inputs` stops. A value that is not valid JSON fails that read with jq's parse message. If the failure is caught, a later `input` continues on the next line. A missing file is named, later files are still read, and the exit status is 2. `input_filename` is the name of the file the current value came from, or null before any value has been read. With `-s`, the stream is one array of every value. With `-R`, each line is one string.

`$__loc__` is an object `{"file":"<top-level>","line":N}`. `N` is the line of that `$`. A program read with `-f` still says `<top-level>`.

`$ENV` is the process environment, with each value a string. `env` reads that same object. Binding `$ENV` again with `as` hides it for `$ENV` and leaves `env` alone. `--arg ENV` stores its text on `$ARGS.named` and does not replace `$ENV`.

`--args` puts each later file argument into `$ARGS.positional` as a string. `--jsonargs` parses each one as one JSON value. A value that is not one JSON value exits 2 with `jq: invalid JSON text passed to --jsonargs`. When both are given, the later one decides what follows it. Arguments written before it are still files. The program itself is not a positional value.

`--slurpfile name path` binds `$name` to every JSON value in that file, as one array. `--rawfile name path` binds `$name` to the bytes of that file as one string. The name is also a field of `$ARGS.named`. A missing file, a directory, or a value that is not valid JSON exits 2. The message names the option, the name, and the path. The first binding of a name wins, and a repeated name does not read its file. `-` is a file name here, not standard input.

A missing file is named and the other files are still read. The exit status is 2. A filter that hits the wrong kind of value uses jq's message. The exit status is 5 when that value is the last one, and 0 when a later value is fine. Output already produced is kept, and later values are still run. On one input, a cut-off final value still prints the complete values before it, then jq's parse error, and exits 5.

## What does not work yet

Filters past the list above are rejected. A call to a name or a number of arguments this version does not define fails with jq's `name/n is not defined` message and the exit status is 3. Anything else that this version does not parse is `tumulus: syntax error`, also exit status 3. A comparison chain such as `1 < 2 < 3` is rejected that way, and so is an object value written as `{a:1+2}`. An unbound `$name` is rejected that way too.

`split` with two filters is rejected that way. jq splits a string on a regular expression there. `fromjson | isnan` is rejected because `isnan` is not defined. jq runs that program.

`strftime`, `strflocaltime`, and `strptime` reject a format letter other than the ones above. The message names the letter and the exit status is 5.

`{$name}` and `{$name: value}` are rejected. jq uses the variable's name or the variable's value as an object key. `$__loc__` as a field is rejected too.

An object written in the brackets, `.[ {start:1, end:3} ]`, does not read the slice jq reads. It fails with `Array/string slice indices must be integers`. Assignment, `del`, `getpath`, and `setpath` of that object do slice when both `start` and `end` are present. `path` of it prints `start` before `end`. A `?` written immediately after a slice, as in `.[1,error("z"):]?`, also swallows an `error` raised while computing a bound. jq reports that error. `(.[1,error("z"):])?` agrees with jq. When the first slice of an update cannot be written, a later bound is still run. jq does not run it. `try (.[1,(input):] = 9) catch input` therefore reads the next input here and does not in jq.

Options other than the ones above are rejected. An unknown long option exits 1 with `tumulus: unknown option: ...`. jq exits 2 and prints its usage text.

A directory passed as a file exits 1 with `tumulus: cannot read ...: Is a directory (os error 21)`, unless the program calls `input`, `inputs`, or `input_filename`. jq exits 2. Without one of those calls, jq prints nothing for that directory.

A value that runs past the end of one file does not continue into the next file, unless the program calls `input`, `inputs`, or `input_filename`. jq keeps parsing across that boundary. After a parse error, later files are not read, unless `input` or `inputs` caught the bad value and asked for another one.

## Time against jq

These times are from one machine: an Apple M4 Pro, 12 CPUs, 25769803776 bytes of memory, macOS 26.6.2, jq-1.7.1-apple. Each file was read once before its timed runs. Each command was `/usr/bin/time -p` with stdout discarded, three times, jq and then tumulus. Every run exited 0. A byte compare of the outputs matched.

`requests.ndjson` is 72727816 bytes and 300000 lines. Compact output was 72727816 bytes. Pretty output was 85878107 bytes.

| run | `jq -c .` | `tumulus -c .` | `jq .` | `tumulus .` |
| --- | ---: | ---: | ---: | ---: |
| 1 | 1.87 | 1.09 | 2.07 | 0.83 |
| 2 | 1.88 | 0.79 | 2.06 | 0.84 |
| 3 | 1.91 | 0.79 | 2.09 | 0.82 |

`citylots.json` is 189778220 bytes and 413125 lines. Compact output was 169215413 bytes. Pretty output was 395218854 bytes.

| run | `jq -c .` | `tumulus -c .` | `jq .` | `tumulus .` |
| --- | ---: | ---: | ---: | ---: |
| 1 | 3.67 | 2.36 | 7.29 | 2.45 |
| 2 | 3.66 | 2.33 | 7.35 | 2.44 |
| 3 | 3.67 | 2.33 | 7.32 | 2.46 |

`[.features[] | select(.properties.STREET == "UNKNOWN")] | length` reads that same file and prints `2843`. Compact and pretty printing of that number are the same text, so these runs used `-c`.

| run | `jq -c` | `tumulus -c` |
| --- | ---: | ---: |
| 1 | 1.70 | 2.60 |
| 2 | 1.70 | 2.62 |
| 3 | 1.72 | 2.64 |

Times are seconds.

## License

MIT. See `LICENSE`.
