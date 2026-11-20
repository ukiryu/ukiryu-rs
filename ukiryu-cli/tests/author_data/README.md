# Original Ruby author contract

These captures use ukiryu/ukiryu at
`ab6df353c6dc2d7faf7817a86aafcafa82579896`, Ruby 4.0.1 and the frozen
`oracle.Gemfile.lock` beside this file. The original CLI does not register
`lint` or `docs`; its unknown-command outputs are retained. The standalone
Thor classes are required explicitly, and exceptions are printed by the
original `FriendlyCLI` error handler. No command or model method is repaired.
In particular, absent FileUtils, absent JSON loading, absent `error_count`,
and the documentation generator's model indexing are observable outcomes.

`case_inputs.json` selects author/error cases. `tools_case_inputs.json` selects
51 describe/executable-validation cases, including the fake tools and smoke
fixtures under `tree/`. `fixture_cases.json` measures the linter and definition
validator for all 64 original register/profile fixtures; generation checks each
input hash against the copy in `corpus/conformance/inputs`. `provenance.json`
records the generator, input, output and dependency-lock hashes and loaded gems.

Regenerate from a clean checkout of the original Ruby source:

```sh
cp ukiryu-cli/tests/author_data/oracle.Gemfile.lock /path/to/ukiryu/Gemfile.lock
mise x -- python3 scripts/capture-author-reference.py /path/to/ukiryu
```

Use a dedicated oracle checkout: its lockfile is local test setup. Install its
locked dependencies before generation. Each command runs in a fresh Ruby
process with an isolated home and PATH; file mtimes are fixed. The sole stub
makes the default remote schema download fail, as in the earlier captures.
HTTP/default-schema retrieval remains separate work and is not qualified here.

The temporary absolute path is replaced by `{root}`. Its SHA256 in a file
source identifier is replaced by `{sha256:relative/path}`; replay computes that
exact digest from its own input path. No error class, message suffix, member,
issue code or exit status is discarded. Malformed-YAML author cases compare the complete error text, including the
parser diagnostic. Line-order and YAML-data comparisons for unrelated commands
retain their earlier explicitly recorded modes.

Run the focused checks with:

```sh
cargo test --locked -p ukiryu-cli --features ukiryu/c --test author_oracle --test author_tools_oracle --test author_fixture_contract --test cli_binary --test arguments
```
