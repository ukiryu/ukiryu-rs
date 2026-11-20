These focused keyword and dialect goldens were captured from a clean checkout of
ukiryu/ukiryu at `ab6df353c6dc2d7faf7817a86aafcafa82579896`, using Ruby 4.0.1,
json-schema 6.2.1 and Addressable 2.9.2. They call `fully_validate` after the
profile key conversion used by `Ukiryu::SchemaValidator`. Accepted documents,
validation messages and Ruby exception classes/messages are all compared exactly.

From that reference checkout, with its dependencies available:

```sh
bundle exec ruby /path/to/ukiryu-rs/ukiryu/tests/schema_golden/keywords/capture.rb /tmp/keywords.json
```

Compare the output byte for byte with `cases.json`. The generator checks the
reference revision and clean state, records its own SHA-256 and the resource
input SHA-256, and normalizes only working-directory and fixture paths. It
regenerates identically with changed timezone, locale and working directory.

Run this fixture alone with:

```sh
cargo test -p ukiryu --locked --features c --test schema_keywords
```

The cases preserve the gem's draft 1/2 `optional` and inclusive-limit rules,
draft-specific keyword selection, malformed argument exceptions, overlapping
pattern properties, strict error ordering and the eagerly opened but invalid
String `extends` behavior. The implementation still explicitly refuses Onigmo
runtime match failures, decimal exponents outside signed 64-bit, and existing
local files whose names use a non-file URI scheme; those behaviors have not been measured here. Resource transport and
YAML value-spelling boundaries remain separate work.
