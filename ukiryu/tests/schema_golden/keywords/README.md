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
String `extends` behavior.

`capture_extends.rb` separately captures 14 local-file cases for `urn:` and
custom URI schemes, including missing files, valid schemas and malformed
resources. Run it from the same clean reference checkout with an output path,
then compare with `extends.json`. Run its focused test with:

```sh
cargo test -p ukiryu --locked --features c --test schema_extends
```

Existing colon-named local files are tested on Unix; Windows cannot create
them. Missing-file cases remain covered by `schema_keywords`.

`capture_runtime.rb` adds 478 focused observations of BigDecimal exponent
boundaries, zero divisors and Onigmo runtime failures. It uses the same clean
reference, Ruby 4.0.1, BigDecimal 4.1.3 and Fiddle 1.1.8. The fixture records the
lockfile and generator hashes. Its generator source was committed first as
`58ce4218b2855b518a676f3b45bbe3cf6be2ea5f`.

```sh
bundle exec ruby -EUTF-8 /path/to/capture_runtime.rb /tmp/runtime.json
cargo test -p ukiryu --locked --features c --test schema_runtime
```

UTF-8 is explicit: Ruby otherwise escapes non-ASCII regexp sources differently
under an ASCII locale. With that input encoding fixed, captures are identical
across changed timezone, locale, working directory and wall-clock second.
Only the test process temporarily sets Onigmo's stack limit to 1000, restoring
its original limit afterwards. Production keeps the default unlimited limit.
The runtime errors retain Ruby's class and message, including regexp source,
literal newlines and escaped slash delimiters.

BigDecimal's exponent overflow/underflow, numeric-zero short circuit and String
zero-divisor exception are reproduced. Remainders that require multi-exabyte
quotient or sign-adjustment storage raise the observed `NoMemoryError` without
initializing that storage: checked capacity reservation fails first. Ordinary
arithmetic stays symbolic. This fixture does not specify ambient heap exhaustion
for otherwise representable allocations, user-configured BigDecimal modes,
Regexp timeouts or non-UTF-8 diagnostic output. Resource transport and YAML
value-spelling boundaries remain separate work.

The replay keeps the captured POSIX absolute-path spelling for scalar String
`extends` under the URN base. A Windows drive prefix would change that input
from `urn:/path` to `C:/path`, bypassing the captured missing-file behavior.
File-based cases continue to use the actual host path and file URI; no rows
are skipped or expected results changed. The fixture expansion regression
checks both `C:/x.json` and `C:\\x.json` on every platform.
