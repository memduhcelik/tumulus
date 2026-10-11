# tumulus

tumulus reads JSON and prints it the way jq does. The filter language is still small. Version 0.1.0 is the first cut of it.

Build a release binary with `cargo build --release`. The binary is `target/release/tumulus`. The timed binary below was built with rustc 1.96.0 (ac68faa20 2026-05-25). `cargo test` runs the same inputs through tumulus and jq, and it fails if jq is missing.

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

A missing file is named and the other files are still read. The exit status is 2. A filter that hits the wrong kind of value uses jq's message. The exit status is 5 when that value is the last one, and 0 when a later value is fine. Output already produced is kept, and later values are still run. On one input, a cut-off final value still prints the complete values before it, then jq's parse error, and exits 5.

## What does not work yet

Filters past the list above are rejected. `.+1` is a syntax error here and prints `2` in jq. The message is `tumulus: syntax error` and the exit status is 3.

Options other than the ones above are rejected. `-s` exits 1 with `tumulus: unknown option: -s`. jq slurps with `-s`. An unknown long option exits 1 with `tumulus: unknown option: ...`. jq exits 2 and prints its usage text.

A directory passed as a file exits 1 with `tumulus: cannot read ...: Is a directory (os error 21)`. jq exits 2 and prints nothing.

A value that runs past the end of one file does not continue into the next file. jq keeps parsing across that boundary. After a parse error, later files are not read.

## Time against jq

These times are from one machine: an Apple M4 Pro, 12 CPUs, 25769803776 bytes of memory, macOS 26.6.2, jq-1.7.1-apple. Each file was read once before its timed runs. Each command was `/usr/bin/time -p` with stdout discarded, three times, jq and then tumulus. Every run exited 0. A byte compare of the outputs matched.

`requests.ndjson` is 72727816 bytes and 300000 lines. Compact output was 72727816 bytes. Pretty output was 85878107 bytes.

| run | `jq -c .` | `tumulus -c .` | `jq .` | `tumulus .` |
| --- | ---: | ---: | ---: | ---: |
| 1 | 1.85 | 4.49 | 2.00 | 0.75 |
| 2 | 1.87 | 0.73 | 2.00 | 0.75 |
| 3 | 1.86 | 0.74 | 2.01 | 0.76 |

The 4.49 second wall time on the first compact tumulus run had a user time of 0.71 seconds.

`citylots.json` is 189778220 bytes and 413125 lines. Compact output was 169215413 bytes. Pretty output was 395218854 bytes.

| run | `jq -c .` | `tumulus -c .` | `jq .` | `tumulus .` |
| --- | ---: | ---: | ---: | ---: |
| 1 | 3.61 | 2.26 | 7.11 | 2.45 |
| 2 | 3.54 | 2.27 | 7.09 | 2.41 |
| 3 | 3.57 | 2.29 | 7.17 | 2.46 |

Times are seconds.

## License

MIT. See `LICENSE`.
