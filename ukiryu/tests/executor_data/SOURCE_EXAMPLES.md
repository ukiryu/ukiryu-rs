# Executor example map (ukiryu#20)

Oracle: `ukiryu/ukiryu@ab6df353c6dc2d7faf7817a86aafcafa82579896`, Ruby 4.0.1,
Linux. The census below comes from **running** RSpec `--dry-run` on the three
named specs, not from reading their source. `source_examples.json` pins the IDs,
descriptions, spec hashes, generator commit/hash and runtime. There are 53 rows:
32 executor, 6 executor-stdin, 15 CLI-stdin. An example can share an existing
Rust test; every source example has its own row and an explicit assertion target.

`E` = `ukiryu/tests/executor_oracle.rs`; `X` = `ukiryu/tests/executor_examples.rs`;
`H` = `ukiryu/tests/executor_host_env.rs`; `C` = `ukiryu-cli/tests/stdin_examples.rs`.
`e`, `s`, `c` in the source column abbreviate `spec/ukiryu/executor_spec.rb`,
`spec/ukiryu/executor_stdin_spec.rb`, `spec/ukiryu/cli_stdin_spec.rb` respectively.

| Source example ID | Runtime description | Rust assertion target |
|---|---|---|
| `e[1:1:1:1]` | Ukiryu::Executor.execute with valid command executes successfully and returns Result | `E::echo_rows` |
| `e[1:1:1:2]` | Ukiryu::Executor.execute with valid command captures stdout | `E::echo_rows` |
| `e[1:1:1:3]` | Ukiryu::Executor.execute with valid command captures exit status | `E::echo_rows` |
| `e[1:1:1:4]` | Ukiryu::Executor.execute with valid command captures metadata with timing | `E::echo_rows` |
| `e[1:1:2:1]` | Ukiryu::Executor.execute with timeout option uses custom timeout when specified | `E::echo_rows` |
| `e[1:1:2:2]` | Ukiryu::Executor.execute with timeout option raises TimeoutError when command exceeds timeout | `E::sleep_10_times_out_and_is_killed` |
| `e[1:1:3:1]` | Ukiryu::Executor.execute with environment variables passes environment variables to command | `E::environment_rows` |
| `e[1:1:3:2]` | Ukiryu::Executor.execute with environment variables merges with existing environment | `E::environment_rows` |
| `e[1:1:4:1]` | Ukiryu::Executor.execute with working directory option changes to specified directory before execution | `E::cwd_row` |
| `e[1:1:5:1]` | Ukiryu::Executor.execute with allow_failure option returns Result with non-zero exit status instead of raising | `E::exit_status_rows` |
| `e[1:1:5:2]` | Ukiryu::Executor.execute with allow_failure option raises ExecutionError when command fails and allow_failure is false | `E::exit_status_rows` |
| `e[1:1:5:3]` | Ukiryu::Executor.execute with allow_failure option includes stderr in error message | `E::exit_status_rows` |
| `e[1:1:6:1]` | Ukiryu::Executor.execute error formatting includes executable name in error message | `E::exit_status_rows` |
| `e[1:1:6:2]` | Ukiryu::Executor.execute error formatting includes exit status in error message | `E::exit_status_rows` |
| `e[1:1:7:1]` | Ukiryu::Executor.execute command building builds appropriate command for the shell | `X::command_building_matches_executor_examples` |
| `e[1:1:7:2]` | Ukiryu::Executor.execute command building includes executable path in CommandInfo | `X::command_building_matches_executor_examples` |
| `e[1:2:1:1]` | Ukiryu::Executor.find_executable with common executables finds executables in PATH | `X::lookup_path_and_additional_paths_match_executor_examples` |
| `e[1:2:1:2]` | Ukiryu::Executor.find_executable with common executables returns nil for non-existent command | `X::lookup_path_and_additional_paths_match_executor_examples` |
| `e[1:2:2:1]` | Ukiryu::Executor.find_executable with additional search paths searches in additional paths | `X::lookup_path_and_additional_paths_match_executor_examples` |
| `e[1:2:2:2]` | Ukiryu::Executor.find_executable with additional search paths prioritizes additional paths when specified | `X::lookup_path_and_additional_paths_match_executor_examples` |
| `e[1:3:1]` | Ukiryu::Executor.build_command joins executable and arguments properly | `X::command_building_matches_executor_examples` |
| `e[1:3:2]` | Ukiryu::Executor.build_command formats path for the shell | `X::command_building_matches_executor_examples` |
| `e[1:4:1]` | Ukiryu::Executor.prepare_environment includes user-specified variables | `E::environment_rows` |
| `e[1:4:2]` | Ukiryu::Executor.prepare_environment preserves existing environment | `E::environment_rows` |
| `e[1:4:3]` | Ukiryu::Executor.prepare_environment merges shell-specific headless environment | `H::host_environment_rows` |
| `e[1:5:1:1]` | Ukiryu::Executor.extract_status when process exited normally returns exit status | `E::extract_status_table` |
| `e[1:5:2:1]` | Ukiryu::Executor.extract_status when process terminated by signal returns 128 + signal number | `E::extract_status_table` |
| `e[1:5:3:1]` | Ukiryu::Executor.extract_status when process was stopped returns 128 + stop signal | `E::extract_status_table` |
| `e[1:5:4:1]` | Ukiryu::Executor.extract_status with unknown status returns 1 as failure code | `E::extract_status_table` |
| `e[1:6:1]` | Ukiryu::Executor integration tests handles multiple rapid executions | `E::repeated_executions` |
| `e[1:6:2]` | Ukiryu::Executor integration tests handles commands with special characters in arguments | `E::echo_rows` |
| `e[1:6:3]` | Ukiryu::Executor integration tests properly escapes special shell characters | `X::command_building_matches_executor_examples` |
| `s[1:1:1:1]` | Ukiryu::Executor.execute with stdin with string stdin data passes stdin data to the command | `E::stdin_rows` |
| `s[1:1:1:2]` | Ukiryu::Executor.execute with stdin with string stdin data passes multi-line stdin data | `E::stdin_rows` |
| `s[1:1:1:3]` | Ukiryu::Executor.execute with stdin with string stdin data passes binary stdin data | `E::stdin_rows` |
| `s[1:1:2:1]` | Ukiryu::Executor.execute with stdin with IO object stdin reads from file IO object | `E::stdin_from_reader` |
| `s[1:1:2:2]` | Ukiryu::Executor.execute with stdin with IO object stdin reads from pipe IO object | `E::stdin_from_reader` |
| `s[1:1:3:1]` | Ukiryu::Executor.execute with stdin with commands that close stdin early handles Errno::EPIPE gracefully | `E::stdin_rows` |
| `c[1:1:1]` | Ukiryu::CliCommands::RunCommand stdin handling --stdin flag reads from stdin pipe | `C::flag_pipe` |
| `c[1:1:2]` | Ukiryu::CliCommands::RunCommand stdin handling --stdin flag handles multi-line JSON from stdin | `C::multiline` |
| `c[1:2:1]` | Ukiryu::CliCommands::RunCommand stdin handling stdin=- parameter reads from stdin pipe when stdin=- is specified | `C::dash_pipe` |
| `c[1:2:2]` | Ukiryu::CliCommands::RunCommand stdin handling stdin=- parameter is equivalent to --stdin flag | `C::flag_equivalence` |
| `c[1:3:1]` | Ukiryu::CliCommands::RunCommand stdin handling stdin=@filename parameter reads from file when stdin=@filename is specified | `C::file` |
| `c[1:3:2]` | Ukiryu::CliCommands::RunCommand stdin handling stdin=@filename parameter shows error when file does not exist | `C::missing_file` |
| `c[1:4:1]` | Ukiryu::CliCommands::RunCommand stdin handling stdin=data parameter passes data directly to command stdin | `C::direct_data` |
| `c[1:5:1]` | Ukiryu::CliCommands::RunCommand stdin handling dry-run mode with stdin shows preview of stdin data | `C::preview` |
| `c[1:5:2]` | Ukiryu::CliCommands::RunCommand stdin handling dry-run mode with stdin truncates long stdin preview | `C::long_preview` |
| `c[1:6:1]` | Ukiryu::CliCommands::RunCommand stdin handling format options with stdin outputs JSON format with stdin | `C::json` |
| `c[1:6:2]` | Ukiryu::CliCommands::RunCommand stdin handling format options with stdin outputs YAML format with stdin | `C::yaml` |
| `c[1:7:1]` | Ukiryu::CliCommands::RunCommand stdin handling composition with other commands can pipe output to another command | `C::pipeline` |
| `c[1:7:2]` | Ukiryu::CliCommands::RunCommand stdin handling composition with other commands can process output from curl | `C::curl_output` |
| `c[1:8:1]` | Ukiryu::CliCommands::RunCommand stdin handling with --raw output format outputs only the command stdout without wrapping | `C::raw` |
| `c[1:8:2]` | Ukiryu::CliCommands::RunCommand stdin handling with --raw output format passes through stderr to actual stderr | `C::raw_error` |

The `stdin_from_reader` assertion uses an actual child-process stdout pipe. Its
producer waits for the executor to read the first chunk before producing the
second chunk, and is reaped. The expected bytes/status come from Ruby `IO.pipe`.
Lookup checks run the discovery module's additional-path API, which owns that
Rust surface; measured Ruby lookup searches PATH before extra directories.
The direct-data CLI example uses an `@file` input, as observed by running that
source example with an Open3 call recorder. The curl example uses a local
producer, matching the original test's simulated curl data; no network is used.
CLI tests require real jq on Linux and execute the compiled binary; the pipeline
case also starts a real producer and `head` consumer. They compare the original
examples' output assertions, excluding volatile framework timings. JSON/raw child
stdout is byte-exact; preview/YAML/error projections retain the source predicates.
Linux is the qualified host for these captures. Cross-platform process coverage
remains in `executor_process.rs` and the Core/executor-reference CI matrices.

Existing traced target behavior remains explicit in `observations.json`: D1
requires environment/shell/timeout in the Rust API; the timeout tests kill the
process tree at the deadline; the large-stdin test corrects the Ruby deadlock.
Those rows keep their source observations and named `migrated` traces. This map
does not reclassify those corrections as byte parity. Corpus timeout, exit-code
mapping and per-shell environment checks remain in `execution_result_oracle.rs`,
`environment_oracle.rs`, `executor_process.rs` and `wire_executor.rs`.

Regenerate from the clean pinned oracle with its dependencies installed:

```sh
cd /path/to/ukiryu-oracle
bundle exec rspec spec/ukiryu/executor_spec.rb spec/ukiryu/executor_stdin_spec.rb spec/ukiryu/cli_stdin_spec.rb --dry-run --format json --out /tmp/executor-census.json
bundle exec ruby -Ilib /path/to/ukiryu-rs/ukiryu/tests/executor_data/capture/capture_source_examples.rb /tmp/executor-examples.json
bundle exec ruby -Ilib /path/to/ukiryu-rs/ukiryu-cli/tests/stdin_data/capture_source_examples.rb /tmp/cli-stdin-examples.json
```

Generators refuse a different/dirty oracle. `generator_commit` identifies the
source commit used to capture each file; keep that commit available, or review
the metadata change when recapturing from a newer generator commit. Compare the
payload as well as its provenance. Local verification commands (GHA runs all
platforms and the full suite):

```sh
cargo fmt --all -- --check
cargo test --locked -p ukiryu --test executor_oracle --test executor_examples
cargo test --locked -p ukiryu-cli --test stdin_examples
cargo clippy --locked -p ukiryu -p ukiryu-cli --all-targets --features ukiryu/c -- -D warnings
```
