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

## C boundary

`ukiryu-ffi/include/ukiryu.h` specifies ABI1. Input is borrowed UTF-8 JSON with an
explicit length. Every reply is an owned NUL-terminated JSON string; free it once
with `ukiryu_free`, never with a foreign allocator. Binding entrypoints remain in
the core behind feature `c`; the cdylib crate is only a link entry.
Ukiryu runtime migration: one Rust core, coordinated language runtimes (ukiryu/ukiryu#29).
