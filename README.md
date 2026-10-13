# tumulus

tumulus reads JSON and prints it the way jq does. The filter language is still small. This is version 0.4.0.

macOS on Apple silicon:

```sh
curl -L -o tumulus https://github.com/memduhcelik/tumulus/releases/download/v0.4.0/tumulus-macos-arm64
chmod +x tumulus
```

Linux on x86_64:

```sh
curl -L -o tumulus https://github.com/memduhcelik/tumulus/releases/download/v0.4.0/tumulus-linux-x86_64
chmod +x tumulus
```

Build a release binary with `cargo build --release`. The binary is `target/release/tumulus`. The timed binary below was built with rustc 1.96.0 (ac68faa20 2026-05-25). `cargo test` runs the same inputs through tumulus and jq, and it fails if jq is missing.

`cargo test --test jq_suite -- --nocapture` runs every program and input from `tests/jq.test` through both and prints how many come out the same. That file is the `tests/jq.test` shipped with jq 1.7.1. The expected lines in it only mark the end of a case. The run asks jq each time. A mismatch is printed and does not fail the test. Of the 447 programs, 194 come out the same on stdout, stderr, and the exit status.

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

Numbers are rewritten on the way out, including spellings such as `01`, `.1`, `+1`, and `1.`. `NaN` and `Infinity` are accepted. A leading UTF-8 BOM is ignored. Nesting deeper than 256 is rejected.

Filters that work are field access (`.a`, `.a.b`, `.["a"]`, `."a b"`), array indexes (`.[0]` and negative indexes), slices (`.[2:5]`, `.[:-1]`), several keys or indexes in one bracket (`.["a","b"]`), iteration (`.[]`), recurse (`..`), pipe (`|`), comma (`,`), two filters written next to each other, parentheses, and the `?` suffix.

A program can also contain numbers, strings, `true`, `false`, and `null`. Strings take the usual escapes and `\( )`. Arrays are `[ ]`. Objects are `{ }`, including `{a}` and `{"a"}` as short field copies, `{"a\(1)"}`, and `{(expr): value}`. A `#` comment runs to the end of the line. `-n` and `--null-input` run the program once on `null` and do not read input.

An object key that is not a string fails the way jq fails. A bad string escape does too.

`+`, `-`, `*`, `/`, and `%` work on numbers. `+` also joins strings and arrays and merges objects. `-` removes elements from an array. `*` repeats a string, and it merges objects when both values of a key are objects. `/` splits a string. `nan` and `infinite` are numbers. Comparisons are `==`, `!=`, `<`, `<=`, `>`, and `>=`. The order is null, then false, then true, then numbers, strings, arrays, and objects. `and`, `or`, and `not` yield booleans. `//` is the alternative. `if` uses `then`, any number of `elif`, an optional `else`, and `end`.

An operator that is given values it cannot use fails the way jq fails, including division and remainder by zero. The exit status is 5.

`$name` reads a variable. `as` binds a value for the rest of that expression. The pattern can take an array or an object apart, as in `. as [$a, $b]`, `. as {a: $x}`, and `{$a}` for `{a: $a}`. `reduce` and `foreach` walk a stream with that kind of pattern. A pattern that meets the wrong kind of value fails with jq's index message. The exit status is 5.

`empty` yields no values. `length` is the number of Unicode code points in a string, the number of elements in an array, and the number of keys in an object. Null has length 0. The length of a number is its absolute value. `keys` is the indexes of an array, or the keys of an object in order. `has(expr)`, `select(expr)`, and `map(expr)` take that one filter. `select` repeats the input for each truthy result. `map` runs the filter on each array element or object value and collects the results. The wrong kind of value fails the way jq fails, and the exit status is 5.

`todate` writes a number of seconds since the epoch as UTC `YYYY-MM-DDTHH:MM:SSZ`. `fromdate` reads that text back into seconds. `gmtime` breaks a number of seconds into `[year, month, day, hour, minute, second, weekday, yearday]` in UTC. `localtime` breaks a number of seconds into that array in local time. The month and the weekday start at 0. `mktime` turns that array back into seconds. `strftime(fmt)` formats a number of seconds in UTC, or that array as written. `strflocaltime(fmt)` formats a number of seconds in local time, or that array as written. `strptime(fmt)` reads text in that format back into the array. `now` is the current number of seconds since the epoch. The format letters are `%Y`, `%m`, `%d`, `%H`, `%M`, `%S`, `%Z`, `%j`, `%a`, `%b`, `%e`, `%y`, and `%z`. A value of the wrong kind fails the way jq fails, and the exit status is 5.

`=` writes a value through a path. `|=` runs a filter on the value there and writes the first result back. No result removes that place, which is also what `del(path)` does. `+=`, `-=`, `*=`, `/=`, `%=`, and `//=` combine the value already there with the right-hand side. A path is a field, an index, a slice, or iteration. It can go through `|`, `,`, `select`, `//`, `if`, and `as`. `and`, `or`, and a matching `null`, `true`, or `false` stay paths when they do in jq. An update that lands on the wrong kind of value fails the way jq fails, and the exit status is 5.

`try expr` keeps every value `expr` produced before it failed, then stops. That is the same as `try expr catch empty`. `try expr catch handler` runs `handler` on the error value instead. A builtin failure is caught as its message. `error` raises the current input, and `error(expr)` raises the first value `expr` produces. When nothing catches the error, a string is printed as `jq: error (at ...): ` and that text. Any other value is printed as `jq: error (at ...) (not a string): ` and the compact JSON. The exit status is 5 when that input is the last one.

A missing file is named and the other files are still read. The exit status is 2. A filter that hits the wrong kind of value uses jq's message. The exit status is 5 when that value is the last one, and 0 when a later value is fine. Output already produced is kept, and later values are still run. On one input, a cut-off final value still prints the complete values before it, then jq's parse error, and exits 5.

## What does not work yet

Filters past the list above are rejected. The message is `tumulus: syntax error` and the exit status is 3. A comparison chain such as `1 < 2 < 3` is rejected that way, and so is an object value written as `{a:1+2}`. An unbound `$name` is rejected that way too.

`strftime`, `strflocaltime`, and `strptime` reject a format letter other than the ones above. The message names the letter and the exit status is 5.

`{$name}` and `{$name: value}` are rejected. jq uses the variable's name or the variable's value as an object key. `$__loc__` as a field is rejected too.

Options other than the ones above are rejected. `-s` exits 1 with `tumulus: unknown option: -s`. jq slurps with `-s`. An unknown long option exits 1 with `tumulus: unknown option: ...`. jq exits 2 and prints its usage text.

A directory passed as a file exits 1 with `tumulus: cannot read ...: Is a directory (os error 21)`. jq exits 2 and prints nothing.

A value that runs past the end of one file does not continue into the next file. jq keeps parsing across that boundary. After a parse error, later files are not read.

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

Times are seconds.

## License

MIT. See `LICENSE`.
