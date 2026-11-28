#include <stdint.h>
#include <stddef.h>
// C ABI of crates/gnobbi-nse. fetch: 0 = *out is JSON {kind, title?, subtitle?, body?, text?, sender_id?, group?}, and *avatar/*avatar_len
// the sender's PNG/JPEG thumbnail when there is one (else NULL/0; free with gnobbi_nse_free_bytes); 1 = failure, all outputs NULL.
int32_t gnobbi_nse_fetch(const char *data_dir, const char *tag, const char *pass, const char *room, const char *event, char **out, uint8_t **avatar, size_t *avatar_len);
void gnobbi_nse_free_bytes(uint8_t *p, size_t len);
void gnobbi_nse_free(char *p);
uint64_t gnobbi_nse_peak_footprint(void);
