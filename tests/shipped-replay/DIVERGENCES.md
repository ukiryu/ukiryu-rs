# Shipped replay: recorded divergences

The replay compares the built artifacts against the conformance corpus in
`corpus/conformance`. The corpus rows are captured from the Ruby gem and stay
unchanged. This file lists the rows where the replay expects something other
than the captured value, and why.

## CLI (`cli.py`)

- **`cli-output.version`.** `ukiryu version` prints the version of the
  implementation that is running. The captured row holds the Ruby gem's
  version (`Ukiryu version 0.3.1`). The Rust CLI prints its own crate version,
  which comes from `[workspace.package] version` in `Cargo.toml`. The replay
  checks that the captured stdout has the shape
  `<cyan>Ukiryu version <version><reset>\n`, then expects the same bytes with
  the Rust workspace version in place of the captured one. Stderr and the
  exit status are compared as captured. This is a difference in the version
  number only, not in behaviour.
