# ukiryu-rs

Rust core migration tracked by [ukiryu/ukiryu#29](https://github.com/ukiryu/ukiryu/issues/29).

## Vendored YAML dependency probe

Before relying on Yeptris in the core, the dependency probe exercises the published
`yeptris = 0.6.63` crate with its `vendored` feature on Linux, macOS, Windows and
the planned browser WASM target. Native jobs parse and re-emit the 69 unique YAML
files in the pinned Ruby source: 65 spec YAML files and four bundled schemas.
Each value must survive serialization and repeated emission must be deterministic.

```sh
cargo test --locked --manifest-path tools/dependency-probe/Cargo.toml
cargo run --locked --manifest-path tools/dependency-probe/Cargo.toml -- /path/to/ukiryu-ruby-reference
cargo check --locked --manifest-path tools/dependency-probe/Cargo.toml --target wasm32-unknown-unknown
```

Use Ruby reference commit `ab6df353c6dc2d7faf7817a86aafcafa82579896`.
The checks establish dependency behavior only: they do not prove Psych parity,
the explicit emitter-settings contract, domain models or a complete Ukiryu port.
The probe is private and its version is not a product release version or MSRV promise.
Ukiryu runtime migration: one Rust core, coordinated language runtimes (ukiryu/ukiryu#29).
