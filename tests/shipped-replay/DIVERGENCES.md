# Shipped replay: recorded divergences

The replay compares the built artifacts against the conformance corpus in
`corpus/conformance`. The corpus rows are captured from the Ruby gem and stay
unchanged. This file lists the rows where the replay expects something other
than the captured value, and why.

## CLI (`cli.py`)

None. Every CLI row, including `cli-output.version`, is compared byte for
byte after host path substitutions, just like
`every_cli_corpus_row_is_captured_byte_exact` in `ukiryu-cli/tests/capture.rs`.
The `CLI_VERSION` constant in `ukiryu/src/cli/commands/inspect.rs` pins the
CLI's compatibility version separately from the Cargo workspace version.
The replay therefore keeps the captured version stdout, stderr and exit
status unchanged; it neither normalizes nor skips the version row.
