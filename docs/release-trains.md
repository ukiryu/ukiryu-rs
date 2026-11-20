# Release train checklist

A Ukiryu release includes the core crate, native Ruby gem, Python wheels,
npm package with Node and browser WASM, and all five CLI archives. A partial
upload is an incomplete train.

## Prepare

1. The release owner approves the version, runtime floors, package names and
   first publication. `release/train.json` records the pending decisions.
2. Record exact core, Ruby, Python and TypeScript commit SHAs. All bindings
   must use the qualified core and the same train version. Keep the frozen
   corpus expectations unchanged.
3. Require the affected repository CI, corpus gates and package checks at
   those commits to pass. `Release train (dry run)` packages core artifacts
   and replays built CLI/WASM candidates. It does not publish them.
4. Require `Docker test all (five distros)` to pass for Alpine,
   Ubuntu 22.04/24.04 and Debian Bookworm/Trixie. The Rust workflow exercises the CLI with a read-only register. The Ruby
   `native-gems` workflow also tests the installed native gem, its ffi tier
   and the frozen corpus with a CLI built from the gem's own core pin.
   Both matrices must pass at their qualified revisions.

## Publish and verify

Publish only after the owner's approval and registry setup. The Python
repository has an owner-gated `Publish to PyPI` workflow. Rust packages are
still `publish = false`; the npm package is still `private`. This checklist
does not enable those publication paths, choose a version or grant credentials.

After every runtime upload completes, dispatch `Published artifact audit` in
`ukiryu-rs`. Select the qualified workflow revision and enter the exact train
version and all four source SHAs. The workflow revision must carry that version
and the audit implementation. The source checkouts provide the sealed corpus
and test adapters; they never provide an engine fallback.

The workflow reads public registries and release assets. It checks:

- The exact crates.io version through a standalone Cargo consumer, with a
  registry source and checksum required in its generated lockfile.
- The published Ruby platform gem in a clean `GEM_HOME`, using its own native
  library and the qualified Ruby corpus adapter.
- The exact PyPI wheel in a clean virtual environment, with source imports and
  library overrides removed.
- The exact npm package in a clean project, including both shipped WASM builds,
  core revision, lock digest, payload hashes and the qualified TS corpus adapter.
- The five published CLI archives from `ukiryu-vX.Y.Z`, including file hashes,
  licenses, platform identity and core provenance. The Linux binary replays
  every frozen `cli-output` row. Other targets receive archive validation.

The corpus consumers retain their declared platform/capability coverage and
existing traced migration rules. A green audit does not extend a runtime's API
or remove its declared exclusions. Browser WASM is replayed in Node in this
audit; browser-environment qualification remains part of repository CI.

Keep the train open until `Published train is complete` succeeds. Record the
audit run URL and qualified SHAs in the release record. Missing artifacts,
wrong versions, incomplete targets and failed replays fail the completion job.
Once a real cross-runtime publisher exists, it must invoke this audit after
its last upload; dry-run success is not a substitute. The manual audit can also
be rerun to investigate a registry mismatch without republishing anything.
