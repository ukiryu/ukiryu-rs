# Ukiryu conformance corpus

This directory is the cross-runtime behavioural contract for Ukiryu. Every
engine (the Rust core, the Ruby, Python and TypeScript bindings) reads it
through `UKIRYU_CONFORMANCE_DIR` and must reproduce every applicable case.

The expectations in this first version come from the original Ruby
implementation, `ukiryu/ukiryu` at commit
`ab6df353c6dc2d7faf7817a86aafcafa82579896`, run by the Ruby oracle in
`../oracle-ruby`. They record what that code does, including behaviour that
is awaiting an owner decision or that is plainly a defect. A row is not a
statement that the behaviour is desired; rows that wait on a decision say so
in their `decision` field.

## The rules

Adopted from the parsanol corpus:

1. **A behaviour change and its corpus change land in the same commit**
   (the WPT rule).
2. **Every engine runs every case in CI.** Failures block.
3. **A red divergent case is added before the fix.** The red case is the
   issue tracker.
4. **Expectation equality is canonical per engine**, with the
   canonicalization documented (below).

## Layout

- `manifest.json`: corpus identity, oracle provenance, case count, the
  SHA-256 of the sorted case-ID list, the SHA-256 of every case file, the
  applicable case count per platform, and every input file with its digest
  and its source path in the pinned Ruby checkout.
- `cases/<kind>.jsonl`: one case per line, sorted by ID. The row format is
  defined by `case.schema.json`.
- `inputs/`: byte-for-byte copies of the pinned source's fixtures:
  `register/` (the spec register: 21 tool directories and 14 interfaces),
  `profiles/`, `fixture-schema/tool.schema.yaml` (the spec fixture schema),
  `schemas/v1/` (the four schemas bundled in the gem) and `repo-yaml/` (the
  checkout's other tracked YAML: workflows, docs and RuboCop config, used
  only for parse rows). They are never edited by hand. The register and
  schemas repositories are not copied or modified.
- `inputs/synthetic-register/`: an authored register (written by
  `capture.rb`, marked `authored` in the manifest) with one tool,
  `absenttool`, whose executable exists on no host. CLI rows that report
  executable discovery use it, so "not installed" is a declared input
  rather than a property of the machine that captured the corpus.

## Case kinds

| Kind | Ops | What is pinned |
|---|---|---|
| `escape` | `shell.escape`, `shell.quote`, `shell.join`, `shell.env_var`, `shell.format_path` | Per-shell output for bash, zsh, fish, sh, dash, tcsh, powershell and cmd over a fixed string matrix. Exact bytes. |
| `validate` | `type.validate` | Accept (converted value) or reject (error) for each parameter type and constraint. |
| `build` | `build.args` | The argv a register profile builds for a command and parameter set. Every command of every version file in the register; parameters are derived from the command's own argument, option and flag definitions. |
| `resolve` | `resolve.extract_version`, `resolve.version_spec`, `resolve.range`, `resolve.shell_detect` | Version text extraction from detection patterns, register version routing, Versionian range matching (semantic and calver boundaries), and shell detection from the environment. |
| `lint` | `lint.string` | The full lint result for every fixture YAML file and some synthetic inputs. |
| `schema` | `schema.validate_profile`, `schema.definition_validate` | Accept/reject and the error list for fixtures against the fixture schema and against the matching bundled schema, plus missing-schema behaviour. |
| `yaml` | `yaml.load`, `yaml.model_load`, `yaml.lutaml_roundtrip` | The parsed structure (or parse error) of all 81 YAML files tracked at the pinned commit (65 spec fixtures, 4 bundled schemas, 12 workflow/docs/RuboCop files), the loaded state of the register models, and the emitted YAML of a Lutaml model round trip. |
| `cli-output` | `cli.run` | stdout, stderr and exit status of the Ruby CLI for a set of verbs. |

Real process execution stays runtime-local: subprocess behaviour differs per
runtime. The corpus pins the command each engine builds (`build`) and the
shape of its results, not the output of external tools.

## Op contract

Each row's `input` is the op's argument object and `expect` is either
`{"ok": value}` or `{"error": {"class": ..., "message": ...}}`. Paths in
`input` (`file`, `schema`) are relative to `inputs/`. Engines implement each
op with their own public API; `api` names the Ruby entrypoint the
expectation came from (`(private)` marks a Ruby-internal function whose
behaviour is still part of the contract).

Error classes are the Ruby class names. The Ruby implementation has no
error codes; mapping these classes to stable codes is open work for the
core (see "Known gaps").

## Value encoding

Values use JSON, with tags for values JSON cannot carry losslessly:

| Value | Encoding |
|---|---|
| null, booleans, integers, strings, arrays | themselves |
| symbol | `{"$sym": "name"}` |
| float | `{"$float": "0.0"}` (Ruby `Float#to_s`; never an integer) |
| time | `{"$time": "2024-01-17T10:20:30.000000000Z"}` (UTC) |
| regexp | `{"$regexp": "source"}` |
| non-UTF-8 string | `{"$bytes": "hex"}` |
| map with only string keys | a JSON object |
| map with any non-string key | `{"$map": [[key, value], ...]}` (keys keep their type and order) |

A float is never equal to an integer: `[0.0, 1.0]` and `[0, 1]` are
different values. Numeric YAML map keys stay numeric through `$map`.

## Comparison

`compare` is `structural` for every row: JSON objects compare
independent of key order; arrays and `$map` entries compare in order;
strings compare byte for byte; every other scalar compares by type and
value. Shell strings, argv elements, error messages and CLI output are
therefore exact bytes. YAML documents are compared as parsed structures,
not as text, except `yaml.lutaml_roundtrip`, whose `emitted` field is the
exact text the model writes.

`error_match` is `class_and_message` (the default) or `class`. Messages that
embed Ruby `inspect` output are still pinned exactly in this version.

Two substitutions apply before comparison: the absolute path of `inputs/`
becomes `${INPUTS}` and the sandbox home directory becomes `${HOME}`. Each
case runs with `TZ=UTC`, `LANG=LC_ALL=C.UTF-8`, `SHELL=/bin/bash`,
`UKIRYU_REGISTER=inputs/register`, an empty temporary `HOME`, and with
`UKIRYU_SCHEMA_PATH` and `UKIRYU_TEST_SHELL` unset unless the row sets them.

## Applicability

Every row declares `applicability.platforms` (a non-empty subset of
`linux`, `macos`, `windows`) and `applicability.requires` (capabilities from
`fs`, `process`, `cli`). An engine runs every row whose platform list
includes its platform and whose requirements it provides. The manifest pins
how many rows apply on each platform for an engine with every capability;
an engine that executes a different number fails. An engine without some
capability (for example a browser build without `fs` or `process`) must
declare that and pin its own count. It may not drop rows silently.

`cli-output` rows apply only on Linux, where they were captured. Shell
detection rows apply on Linux and macOS.

## Decisions pending

Rows with `"decision": "D2"` record Ruby's shell detection: suffix
matching (`/bin/dash`, `/bin/tcsh`, `/bin/ksh` all detect as `sh`), and the
`UKIRYU_TEST_SHELL` override that ignores invalid values. Whether the new
core keeps this behaviour is an owner decision. Until it is made, these rows
describe the oracle; an engine that deliberately differs must change the
rows in the same commit, as the WPT rule requires.

The executor's handling of a missing `env` (decision D1) is process
behaviour and has no row here.

## Known defects recorded as they are

- `inputs/schemas/v1/tool.schema.yaml` is not valid YAML: line 287 has an
  unquoted `description: DEPRECATED: Use invocation.type instead`. Psych
  rejects it (`yaml.load.schemas-v1-tool-schema`), `SchemaValidator`
  raises, and `DefinitionValidator` silently falls back to structural checks
  only, so its rows against that schema report success. The file is kept
  byte-identical to the pinned source.
- The four bundled schemas declare JSON Schema draft-07. The json-schema gem
  in the oracle lock (6.2.1) has no draft-07 validator, so
  `SchemaValidator` raises `JSON::Schema::SchemaError` ("Schema not found:
  http://json-schema.org/draft-07/schema#") for every row against a
  bundled schema. No network access is involved.
- Versionian 0.1.0 cannot order calver versions: `before`, `after` and
  `between` ranges on the `calver` scheme raise `NoMethodError`
  ("undefined method '>=' for an instance of Array"), including register
  routing for `ping`'s gnu implementation. Only `equals` works.
- `DefinitionValidator` with no schema path downloads a remote schema. That
  path depends on the network and is not in the corpus.
- `register/tools/ping/index.yaml` routes some versions to `latest.yaml`
  files that do not exist. The `build` op follows
  `Tool.get`: a missing index, interface, version file or compatible
  profile ends in `ToolNotFoundError` ("Tool not found: <name>").

## Running

Ruby oracle (from `../oracle-ruby`):

```sh
bundle install
UKIRYU_CONFORMANCE_DIR=$PWD/../conformance bundle exec ruby verify.rb
bundle exec ruby selftest.rb        # negative controls
bundle exec ruby capture.rb --check # regeneration is byte-identical
```

`verify.rb` has no skip switch. It reports total, executed, failed and
not-applicable counts and exits non-zero on any failure, any integrity
problem (unknown fields, duplicate or missing IDs, a digest mismatch), or an
executed count that differs from the manifest.

`selftest.rb` copies the corpus, applies one deliberate defect at a time
(wrong value, wrong error class, wrong message, missing case, duplicate
case, unexpected case, unsealed edit, tampered input, a case moved off the
current platform, an unknown op, empty applicability) and requires the runner to reject
each one.

## Regenerating

`capture.rb` rebuilds `inputs/`, `cases/` and `manifest.json` from the
pinned Ruby gem and the committed `Gemfile.lock`. Changing the oracle
commit, the lockfile or the generators changes the corpus in the same
commit, and the diff is the review surface.

## Known gaps

This is the first increment of issue #23. It does not yet meet the issue's
acceptance criteria:

- No Rust consumer exists yet. The corpus is run only by the Ruby oracle.
- The Ruby binding (#24) does not exist yet; this oracle is the original
  gem, not the binding.
- Error codes are not defined; rows pin Ruby class names.
- `cli-output` rows are Linux-only, and the corpus has not been captured on
  macOS or Windows.
- The executor (`env`, timeouts, stdin) is runtime-local and has no
  result-shape rows yet.
