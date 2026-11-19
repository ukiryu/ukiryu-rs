# ukiryu-rs

Shared Rust core for the migration in [ukiryu/ukiryu#29](https://github.com/ukiryu/ukiryu/issues/29).
The workspace contains the core, a thin non-published C-ABI cdylib entry and the
future Rust CLI. Initial development metadata follows Parsanol: version0.1.0,
Rust1.86, shared dependencies and an LTO release profile. Publication is not enabled.

## Foundation currently implemented

Configuration conversion, environment overrides and priority resolution run in
the core with explicit environment inputs. CLI values take precedence over ENV,
programmatic configuration and defaults. False, null and absent values retain
their distinct roles; Ruby integer prefixes, underscores and arbitrary-size
integers are checked against a regenerated pinned-source reference.

All YAML reading uses vendored Yeptris and serde through `from_yaml` or
`from_yaml_slice`. The initial JSON transport offers version/configuration and
JSON-compatible YAML diagnostics; it is not the complete typed-model interface.
Diagnostics reject non-finite numbers, non-string mapping keys and YAML tags rather
than silently changing their values to fit JSON. The typed YAML API remains
available to the core without that JSON profile restriction.
Development currently pins the exact qualified Yeptris source revision carrying
float fidelity, explicit emitter options, Rust1.86 compatibility and browser
build support. This is a source qualification pin; no fixed registry release or
Ukiryu publication is claimed.
Only `--version` exists in the CLI foundation. Full models, loaders, schema/type
validation, shells, process execution, authoring and runtime bindings remain
tracked migration work; this increment does not complete issue17 or the product.

## Checks

```sh
cargo test --workspace --locked --features ukiryu/c
cargo clippy --workspace --all-targets --locked --features ukiryu/c -- -D warnings
cargo build --workspace --locked
python3 scripts/check-c-api.py
cargo run --locked -p ukiryu-cli -- --version
```

CI runs Linux/macOS/Windows plus the Rust1.86 floor and loads the actual shared
library with a fresh ctypes consumer. Regenerate the configuration reference with
Ruby4.0.1 and the pinned source commit `ab6df353c6dc2d7faf7817a86aafcafa82579896`:

```sh
ruby scripts/capture-config-reference.rb /path/to/pinned-ruby-source > fixtures/reference/config.json
```

The reference's JSON transport profile gives Ruby symbols textual values. It is
separate from the shared corpus's richer typed value profile; neither establishes
full Ruby API or cross-runtime parity by itself.

## CLI help wording

The Rust CLI replaces the Ruby CLI's Thor parser with clap, but it keeps
Thor's help text wherever the Ruby CLI has one. `ukiryu help`, `ukiryu help
VERB`, `ukiryu GROUP help SUB` and `-h` or `--help` on a Ruby verb print the
same bytes as the pinned Ruby CLI (ukiryu `ab6df35`, Thor 1.5.0, Ruby 4.0.1).
The captured screens are in `ukiryu-cli/src/thor_help.rs`, and
`ukiryu-cli/tests/thor_data/cases.json` checks them, along with the unknown
command and wrong argument count messages.

clap's own help wording appears in only two places. The first is the `lint`
verb, which the Ruby CLI does not have. The second is command lines that the
Thor model passes on without an answer: arguments after `--`, numeric
switches, non-ASCII text, unusual switch spellings and switches given to a
command group. In those cases the layout and phrasing come from clap. Verbs,
option names, short aliases, defaults, argument order and exit codes are the
same as in the Ruby CLI. `--version` is also new: the Ruby CLI rejects it,
while the binary keeps it for release checks.

## C boundary

`ukiryu-ffi/include/ukiryu.h` specifies ABI1. Input is borrowed UTF-8 JSON with an
explicit length. Every reply is an owned NUL-terminated JSON string; free it once
with `ukiryu_free`, never with a foreign allocator. Binding entrypoints remain in
the core behind feature `c`; the cdylib crate is only a link entry.
Ukiryu runtime migration: one Rust core, coordinated language runtimes (ukiryu/ukiryu#29).
