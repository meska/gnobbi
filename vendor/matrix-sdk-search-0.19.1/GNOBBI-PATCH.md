# Gnobbi vendored matrix-sdk-search

- Version: 0.19.1 (Apache-2.0), used only through the `experimental-search` feature of the vendored matrix-sdk.
- Official archive: https://static.crates.io/crates/matrix-sdk-search/0.19.1/download
- Upstream source file modified: `src/index/mod.rs` only.
- `RoomIndex::new_with` registers the `default` text analyzer with an ASCII-folding filter (tantivy's own `AsciiFoldingFilter`) after lower-casing: the same analyzer indexes and searches the body, so «citta» finds «Città».
- `RoomIndex::search` now takes plain text: every word must be in the message and the last word may be unfinished (prefix), as a message search field does while typing; results are newest first (by the indexed date), there is no query syntax, so a stray `(` or `a:b` cannot fail the search. The old tantivy query-language search is kept as `RoomIndex::search_query`, which the index uses internally (`contains`, `events_to_be_removed`).
- No dependency changes. A unit test (`plain_text_search_matches_prefixes_and_accents`) covers the new behaviour.
- `../matrix-sdk-search-0.19.1.patch` is generated with `diff -ruN` from the extracted official archive to this directory, excluding `GNOBBI-PATCH.md`, `LICENSE`, `Cargo.lock`, `Cargo.toml.orig`.
