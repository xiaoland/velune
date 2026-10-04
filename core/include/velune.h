#ifndef VELUNE_CORE_H
#define VELUNE_CORE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct VeluneCore VeluneCore;

uint32_t velune_core_abi_version(void);

/* UTF-8 input strings are borrowed only for the duration of the call.
 * Result strings are owned by Rust and must be freed with string_free.
 * Open clears out_core on failure. Close preserves it when busy and
 * clears it after successful shutdown. Serialize calls to each handle. */
char *velune_core_open(const char *options_json, VeluneCore **out_core);
char *velune_core_request(VeluneCore *core, const char *request_json);
char *velune_core_close(VeluneCore **core);
void velune_core_string_free(char *result_json);

#ifdef __cplusplus
}
#endif

#endif
