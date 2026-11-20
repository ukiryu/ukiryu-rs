# Schema value spelling and malformed inputs

These frozen outputs come from the clean `ukiryu/ukiryu` reference at
`ab6df353c6dc2d7faf7817a86aafcafa82579896`, with Ruby 4.0.1,
Psych 5.3.1, date 3.5.1, json 3.1.0 and json-schema 6.2.1. The sidecars
record the bundle lock, input, generator and payload hashes and the port's
source commit. The generators fix `TZ=UTC` and the external encoding to UTF-8; a timezone-less YAML timestamp
is constructed in UTC and then displayed in the process's local zone.

From the port root, with the pinned reference bundle installed:

```sh
BUNDLE_GEMFILE=/clean/ukiryu/Gemfile bundle exec ruby \
  ukiryu/tests/schema_golden/value-spelling/capture.rb \
  ukiryu/tests/schema_golden/value-spelling/cases.json
BUNDLE_GEMFILE=/clean/ukiryu/Gemfile bundle exec ruby \
  ukiryu/tests/schema_golden/value-spelling/capture_diagnostics.rb \
  ukiryu/tests/schema_golden/value-spelling/diagnostics.json
cargo test -p ukiryu --locked --lib schema::
cargo test -p ukiryu --locked --test schema_parity
```

`cases.json` contains 320 success/error probes across permitted-class
policies. The core matches 300 exactly and pins 20 existing loud refusals:

- 9 binary-string cases: `Data::String` is UTF-8 and cannot preserve Ruby's
  ASCII-8BIT encoding or arbitrary bytes.
- 6 Encoding-object cases: the schema data model has no Encoding object.
- 2 permitted Class-object cases: `!ruby/class Time` and `!ruby/module Date`
  load Class objects, which the schema data model does not represent.
- 3 nonempty `!ruby/hash-with-ivars` cases: instance-variable assignment is
  not represented. The empty form is supported.

Some byte-level contracts pinned here:

| Operation | Ruby behavior |
| --- | --- |
| Time JSON dump | Uses `to_s`; omits the fractional second |
| Time inspect | Keeps the exact fraction, including sub-nanosecond rationals |
| YAML Date inspect | Uses the proleptic Gregorian start (`-Infj`) |
| Allocated Date | Julian day zero with the Italian cutover |
| Hash Symbol keys | Label syntax, including quoted names and ASCII byte escapes |
| YAML merge list | First map wins; a merge overrides earlier explicit keys |
| Tagged float | Converts the resolved Ruby object; reproduces conversion errors |

The complete Ruby result remains in the payload for every refusal. No
accepted reference input is replaced with a fabricated success.

`diagnostics.json` contains 94 parser/reader probes. JSON parse results are
recorded separately from schema-validation results: a valid JSON string can
parse and then raise from the schema engine. Tests assert the parser result
at the parser boundary and wrap its error as `JSON::Schema::JsonParseError`.

Other remaining boundaries are explicit: tagged `yeptris::Value` conversion,
unmeasured OS read errors, and local-timezone display when the host does not
supply a UTC offset. This work does not close issue 17 or change resource
retrieval, dialects or schema keywords.
