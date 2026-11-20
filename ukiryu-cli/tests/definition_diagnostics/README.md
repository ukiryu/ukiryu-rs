These 24 CLI captures cover three invalid YAML shapes (unfinished flow sequence,
bad indentation and an extra colon), LF/CRLF input, validate/lint, and text/JSON.
The test compares complete stdout, stderr and exit status on every platform:
12 validate cases through the binary, and 12 through the shared standalone lint
handler. Ruby does not register LintCommand in its main executable.

Regenerate with Ruby 4.0.1 selected and a clean checkout of ukiryu/ukiryu at
`ab6df353c6dc2d7faf7817a86aafcafa82579896`:

```sh
mise x -- python3 scripts/capture-definition-diagnostics.py /path/to/oracle
```

The generator uses the original author-capture boot and its unavailable remote
schema stub. It verifies the resolved dependency versions and records the actual
frozen lockfile and source/payload hashes. Standalone Ruby lint JSON raises
`NoMethodError` for Hash#to_json; those error bytes are deliberately preserved.
