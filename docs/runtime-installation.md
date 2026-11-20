# Ukiryu runtimes and installation

Ukiryu's migration uses one Rust core for the Rust CLI, Ruby, Python and
TypeScript/WASM runtimes. The register and schema repositories keep the same
YAML profile contract. Parsing uses Yeptris in the core; bindings carry values
over the core boundary rather than parsing YAML themselves.

The new packages are development candidates. The existing `ukiryu` RubyGem
is the classic Ruby implementation until the release owner approves cutover.
Installing that gem today does not establish that the Rust migration shipped.
Use a qualified source revision or a CI artifact to evaluate the new runtimes.

| Runtime | Source | Development installation |
| --- | --- | --- |
| Rust CLI | [ukiryu-rs](https://github.com/ukiryu/ukiryu-rs) | `cargo build --locked --release -p ukiryu-cli`; binary: `target/release/ukiryu` |
| Ruby | [ukiryu-ruby](https://github.com/ukiryu/ukiryu-ruby) | Download a matching native gem from `native-gems` or `platform-gems` CI and run `gem install --local PATH.gem` |
| Python | [ukiryu-python](https://github.com/ukiryu/ukiryu-python) | Download the matching OS/architecture wheel from `Wheels` CI and run `python -m pip install PATH.whl` |
| TypeScript / Node / browser | [ukiryu-ts](https://github.com/ukiryu/ukiryu-ts) | Download the tarball from `npm pack` CI and run `npm install PATH.tgz` |

Obtain artifacts from the exact reviewed commit. A tarball or gem with the same
development version can carry a different engine revision. Node uses the
package's Node WASM; browsers use its separately built browser WASM. Only Node
has the host subprocess adapter. Browser code supplies profile and schema text
and cannot access the host filesystem or start a process.

After an approved release, the CLI archives will be attached to
`ukiryu/ukiryu-rs` releases tagged `ukiryu-vX.Y.Z`. Choose the OS/architecture
archive, verify its `BUILT_FROM` file and bundled file digests, and put the
extracted `ukiryu` binary on `PATH`. Registry install commands for the new
runtimes must name the approved train version. Publication is incomplete until
the [published artifact audit](release-trains.md) passes.

For sibling development checkouts, keep `ukiryu-rs`, `ukiryu-ruby`,
`ukiryu-python`, `ukiryu-ts`, `ukiryu`, `register` and `ukiryu.github.io`
under one parent, with a separate `worktrees/` directory for task branches.
The scripts do not move existing checkouts or change a developer's configuration.
Run `python3 scripts/checkout-siblings.py` to inspect `~/src/ukiryu`, or pass
`--clone` to create missing checkouts. Existing checkouts are left unchanged;
`--root PATH` selects another parent.
