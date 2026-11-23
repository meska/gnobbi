# Local changes to matrix-sdk-search 0.19.1

Upstream: https://github.com/matrix-org/matrix-rust-sdk (crate `matrix-sdk-search`, Apache-2.0, see LICENSE).
Only `src/index/mod.rs` changes; the full diff is `vendor/matrix-sdk-search-0.19.1.patch`.

- The "default" text analyzer (simple tokenizer, lower case, ASCII folding) is registered on every index, so search ignores case and accents.
- `search_query`: plain-text search where every word must match and the last word also matches as a prefix, newest message first. User text is never parsed as tantivy query syntax (`(`, `a:b`, `"…"` raise errors upstream).
- `contains()` uses `search_query`.

Remove this directory and the `[patch]` entry once upstream offers the same.
