# Type and constraint oracle rows

These JSONL files hold the expected behaviour of `ukiryu::types`. Each row
was produced by running the original Ruby implementation, ukiryu/ukiryu at
`ab6df353c6dc2d7faf7817a86aafcafa82579896`, under Ruby 4.0.1 with `TZ=UTC`,
`LANG=LC_ALL=C.UTF-8`, through the oracle bundle of the conformance corpus
(uri gem 1.1.2). Nothing here is written by hand.

| File | Ruby entry point |
|---|---|
| `type_validate.jsonl` | `Ukiryu::Type.validate(value, type, options)` with Hash options |
| `type_valid_type.jsonl` | `Ukiryu::Type.valid_type?(type)` |
| `type_file_fs.jsonl` | `Type.validate(path, :file, require_existing: ...)` against a real directory |
| `type_model_options.jsonl` | `Type.validate` with an `ArgumentDefinition`/`OptionDefinition` as options |
| `constraints.jsonl` | each `Ukiryu::Validation::*Constraint#validate` |
| `validator.jsonl` | `Ukiryu::Validation::Validator` (`constraints`, `validate!`, `valid?`, `errors`) |
| `ruby_format.jsonl` | Ruby core formatting that appears in messages (`Float#to_s`, `inspect`, `dump`, `downcase`) |

Each row has an `id`, a `source` (oracle commit and Ruby API), the `input`,
and the `expect`ed outcome: `{"ok": value}` or `{"error": {"class", "message"}}`.
Constraint errors also carry `attribute` and `validation_type`.

Values use the corpus tags: `{"$sym": name}`, `{"$float": "<Float#to_s>"}`,
`{"$time": "<UTC ISO 8601, 9 fraction digits>"}`, and `{"$map": [[key, value], ...]}`
for every Hash (key types and order preserved). Two tags are local to these
files: `{"$re": [source, options]}` keeps the Regexp flags that `$regexp`
drops, and a Time result also records `time` (`utc_offset`, `inspect`,
`to_s`, `utc`), because the UTC tag alone loses the zone Ruby prints.

`type_file_fs.jsonl` paths use `${TMP}` for the fixture directory the test
creates (`exists.txt`, `sp ace.txt`, `subdir/`, and on Unix the symlinks
`broken` and `link`). Rows list the platforms they apply to.

The Ruby source does not load `validation/constraints.rb` or
`validation/validator.rb` (`lib/ukiryu/validation.rb` is an empty module);
row `validator.reachability` records that, and the capture loads both files
explicitly. Row `validator.037` records that the validator cannot read a
`Models::CommandDefinition` (`NoMethodError` for `#[]`).

`gaps.txt` lists rows whose Ruby behaviour the port does not reproduce
(the port answers `Unsupported` for them instead of a guess).
