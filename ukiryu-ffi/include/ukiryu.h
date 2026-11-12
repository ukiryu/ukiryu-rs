#ifndef UKIRYU_H
#define UKIRYU_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
uint32_t ukiryu_abi_version(void);
/* Input is borrowed for the call. Nonzero length requires a readable byte span.
 * Reply is owned UTF-8 JSON, NUL terminated. Free once with ukiryu_free. */
char *ukiryu_json(const uint8_t *input, size_t length);
void ukiryu_free(char *reply);
#ifdef __cplusplus
}
#endif
#endif
