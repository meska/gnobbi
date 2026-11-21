#include <stdint.h>
// C ABI of crates/gnobbi-nse. fetch: 0 = *out is JSON {kind, sender?, body?, room?}; 1 = failure, *out is NULL.
int32_t gnobbi_nse_fetch(const char *data_dir, const char *tag, const char *pass, const char *room, const char *event, char **out);
void gnobbi_nse_free(char *p);
uint64_t gnobbi_nse_peak_footprint(void);
