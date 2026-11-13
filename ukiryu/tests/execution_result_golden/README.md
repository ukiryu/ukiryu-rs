# Execution result and error oracle rows

These files hold the expected behaviour of `ukiryu::execution` and
`ukiryu::errors`. `capture.rb` produced every row by running the original Ruby
implementation, ukiryu/ukiryu at `ab6df353c6dc2d7faf7817a86aafcafa82579896`,
under Ruby 4.0.1 with the json 3.1.0 gem from the oracle bundle, on
x86_64-linux (`PROVENANCE.json`). Nothing here is written by hand.

| File | Ruby entry point |
|---|---|
| `output.jsonl` | `Ukiryu::Execution::Output` readers, `to_h`, `to_s`, `inspect`, `*_contains?` (first row: the patterns) |
| `metadata.jsonl` | `Ukiryu::Execution::ExecutionMetadata` over `Time` pairs and Integer/Float/nil timeouts |
| `duration_format.jsonl` | `#formatted_duration`, `Float#to_s` and `JSON.generate(Float)` over Float durations (seeded) |
| `command_info.jsonl` | `Ukiryu::Execution::CommandInfo` (`File.basename` as on Linux) |
| `result.jsonl` | `Ukiryu::Execution::Result` readers, `to_json`, `to_s`, `inspect` |
| `json_errors.jsonl` | `Result#to_json` with a NaN/Infinity timeout |
| `errors.jsonl` | every `Ukiryu::Errors` class: superclass, ancestors, messages, suggestions, `timeout:` |
| `format_error.jsonl` | `Ukiryu::Executor.format_error` (the `ExecutionError` message) |
| `executor_real.jsonl` | `Ukiryu::Executor.execute` with real `sleep`/`sh` processes under bash |
| `exit_code_mapping.jsonl` | `Ukiryu::Response::Base#exit_code_meaning`/`#format_meaning` with `Tool.find_by` stubbed to fixed profiles |

The Ruby regex patterns in `output.jsonl` carry a Rust `regex` equivalent
(Ruby's `^`/`$` are line anchors, so `^a$` becomes `(?m)^a$`); that
translation is the only hand-written part, and it is an input, not an
expectation.

Observed source behaviour recorded here and kept by the port:

- `Ukiryu::Executor` declares `autoload :Execution`, which names
  `Ukiryu::Executor::Execution`. If `Ukiryu::Execution` is loaded before the
  first `Executor.execute`, that call raises `NameError`; the capture runs the
  executor first for that reason.
- `TimeoutError` raised by the executor carries no `timeout` (nil), although
  the class accepts one and the docs read `e.timeout`.
- `ExecutionError` has no `result` reader, although its suggestions and the
  docs read `e.result`.
- `Response::Base#timed_out?` calls `Result#timeout_exceeded?`, which does not
  exist (`NoMethodError`, last row of `exit_code_mapping.jsonl`).
- `Output#stdout_length`/`#stderr_length` count characters; the doc comments
  say bytes.
- A profile exit-code meaning of `success` does not make a non-zero status a
  success: `Response::Base#success?` and `Result#success?` test `zero?` only.
