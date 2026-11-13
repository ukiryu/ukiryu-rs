#ifndef UKIRYU_H
#define UKIRYU_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
uint32_t ukiryu_abi_version(void);
/* Input is borrowed for the call. Nonzero length requires a readable byte span.
 * Reply is owned UTF-8 JSON, NUL terminated. Free once with ukiryu_free.
 * Requests are JSON objects naming an "op": version, resolve_config,
 * parse_yaml, escape, resolve, build, validate, schema, results, execute.
 * Replies are {"ok":true,...} or {"ok":false,"error":{"code":...}}; see the
 * core crate's wire module for each operation and the value encoding. */
char *ukiryu_json(const uint8_t *input, size_t length);
void ukiryu_free(char *reply);
#ifdef __cplusplus
}
#endif
#endif
