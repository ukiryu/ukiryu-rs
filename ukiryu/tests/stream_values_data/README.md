# Stream values golden data

Observations of the pinned Ruby implementation (ukiryu/ukiryu at
ab6df353c6dc2d7faf7817a86aafcafa82579896, Ruby 4.0.1, the locked
`corpus/oracle-ruby` bundle) read by `ukiryu/tests/stream_values.rs`.

- `observations.json` comes from `capture_stream_values.rb`. A real bash
  child writes each payload on stdout or stderr through
  `Ukiryu::Executor.execute`. The file records the value or the raised error
  at each stage: the result's raw streams, the `Execution::Output` and
  `Execution::Result` readers, `to_h` and `to_json`,
  `SuccessResponse.from_result`, and the raw, YAML and JSON response formats,
  with and without an output file.
  The file is the capture from GitHub Actions (ubuntu-24.04, libyaml
  0.2.5). The YAML bytes depend on the host's libyaml: libyaml 0.2.5 writes a
  NUL unquoted, so Psych cannot load the document back, while libyaml 0.2.2
  writes `"a\0b"`. The test therefore compares the YAML format by loaded
  values, against the JSON format of the same response.
- `cli.json` comes from `cli/capture_cli.py`, which runs the real
  `ukiryu exec` command against the `emit` tool in `cli/register`
  (`cli/bin/emit` must be on `PATH`). Paths under `cli/` are recorded as
  `<CLI>`.

The `Stream values reference` workflow runs both captures again and compares
the results with these files using `compare.py`.
