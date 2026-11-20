# Native schema resources

`http.json` is captured from clean ukiryu/ukiryu
`ab6df353c6dc2d7faf7817a86aafcafa82579896`, Ruby 4.0.1 and
json-schema 6.2.1. Run the generator in that oracle's bundle:

```sh
bundle exec ruby /path/to/capture_http.rb /path/to/oracle /tmp/http.json
```

The generator starts a loopback HTTP server, clears the Ruby schema cache for
every row and captures results and exception class/message pairs. The only
canonicalization replaces ephemeral server addresses and the refused port.
Data strings are passed as JSON text: a bare string could instead read an
ambient file with that name, so changing PWD would change the measured action.
The definition fallback URL is changed only inside the probe process; parsing,
transport, resolution and validation still run through the original methods.
The fallback's success/error results and the existing offline tests are separate
observations: old captures explicitly stubbed download failure and now choose
`RemoteSchemaSource::Unavailable` in Rust.

Observed traps reproduced by the native implementation:

- An HTTP 404 raises `JSON::Schema::ReadFailed` for schema resources and is rescued
  by the definition fallback. Connection refusal raises `Errno::ECONNREFUSED` in
  both paths, because Ruby's rescue lists do not include it.
- Relative references use the requested URI, including after redirects. External
  schemas append `#` in validation diagnostics. A top-level URI fragment does not
  select a subschema, while a `$ref` fragment does.
- Fallback YAML syntax errors, nil and false skip schema validation; permitted
  Symbols are returned by the loader and still fail at schema preparation.

Production native validation downloads `REMOTE_SCHEMA_URL` when no explicit or
environment schema loads. An explicit missing schema path skips validation and
never falls back. `RemoteSchemaSource::Url` allows a caller to choose a local or
controlled source without a process-global override. Native CLI/wire hosts may
select it with `UKIRYU_REMOTE_SCHEMA_URL`; when unset, the Ruby URL is used.
Binary tests point this override at a real loopback 404 response.

WASM has no HTTP/TLS client dependency. HTTP schema resources/default fallback
return an explicit `HTTP schema on WASM` refusal. Browser hosts can use
`validate_report_with_schema_text` and provide their own loaded schema. Fetching
external resources from browser JavaScript is not implemented here.

Remaining limits are explicit: non-file/non-HTTP transports, unmeasured malformed
URI diagnostics, transport failures beyond the observed HTTP/DNS/refusal paths,
and malformed JSON resource diagnostics still refuse. Schema dialect/keyword
coverage and YAML/JSON value spelling are separate work.
