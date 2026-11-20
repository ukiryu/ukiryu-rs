# CLI execution captures

`cases.json` is the frozen output of the pure-Ruby CLI at
`ukiryu/ukiryu` revision `ab6df353c6dc2d7faf7817a86aafcafa82579896`,
using Ruby 4.0.1. The adjacent `execute.rs` runner pins all 104 rows and
requires an outcome for every row. No anonymous-setter errors are refused.

The compiled binary also replays the 12 `pty` rows through a real Linux
pseudo-terminal, including ANSI colors, `NO_COLOR`, `TERM=dumb` and the
oracle's cache-info `NameError`. Both streams share the terminal as in the
capture. Only the terminal's LF-to-CRLF translation is disabled; output bytes,
process status and fixture file contents are checked without masks. The check
rejects changed output and changed status as negative controls:

```sh
python3 scripts/check-cli-terminal.py --binary target/debug/ukiryu
cargo test --locked -p ukiryu-cli --test cli_binary terminal_execute_captures
```

## Comparison canonicalization (#22, #23)

The four rows `exec_register_unknown_param`, `runfile_execute`,
`runfile_execute_json` and `runfile_execute_table` contain a rescued
`NoMethodError` whose receiver is an instance of an anonymous options class.
Its `#<Class:0x...>` address changes with each process. The native CLI emits
the command definition's native address in that position. Both runtimes keep
the full diagnostic in runtime output.

Only during comparison, replace the nonempty hexadecimal address in an exact
`#<Class:0x[0-9a-fA-F]+>` token with `<address>`. This applies symmetrically
to expected and actual stdout, stderr and output-file contents. Preserve the
class token, exception class, setter, surrounding context, response fields,
response exit code and CLI process status. Named class/object addresses,
ordinary `0x...` values and malformed inspect tokens remain exact. Negative
controls in `execute.rs` enforce these boundaries across YAML, JSON and table
responses. A response with `exit_code: 1` still exits the CLI with status 0,
as the Ruby CLI does when its command returns a rescued error response.

The existing timing masks, YAML value comparison and JSON-file timing masks
remain documented at the top of `execute.rs`. One malformed-YAML case uses
separate pinned Yeptris diagnostics; it does not claim Psych message parity.

To re-measure just these four errors without changing the frozen expectations:

```sh
python3 scripts/check-cli-anonymous-errors.py /path/to/pinned/ukiryu \
  --ruby /path/to/ruby-4.0.1-wrapper --output /tmp/anonymous-errors.json
```

The helper checks the clean oracle revision and Ruby version, runs each row
in two independent processes/fixture roots with different timezone and locale,
and records raw outputs plus oracle lockfile, cases and generator hashes.
A changed error shape or status fails. Runtime output is never canonicalized.
