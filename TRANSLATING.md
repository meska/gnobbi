# Translating Gnobbi

Gnobbi is translated with plain text files, one per language. You do not need to know Rust to fix or add a translation.

The first translations were written with AI assistance and have not been reviewed by native speakers. Corrections are very welcome, and so are reviews: a fix is one edited line.

## Where things are

- Catalogs: `crates/matrix-desktop/locales/<code>.tsv` (`de.tsv`, `pt-BR.tsv`, ...). English has no file: the English text in the code is the key.
- Language registry: the `LANGUAGES` table in `crates/matrix-desktop/src/i18n.rs`. Every supported language has a row there (code, native name, plural rule). A language appears in Settings > Language only once its catalog has at least one translation.

## Format

UTF-8, one row per string, three tab-separated columns:

```
context<TAB>English text<TAB>translation
```

- Empty context = a normal label; `menu` = a menu item. The same English text can have a different translation per context.
- In text, `\t` is a tab, `\n` a new line, `\\` a backslash. Lines starting with `#` and empty lines are ignored.
- Plurals: context `@plural`, the English text is the singular form, and the translation lists every plural form of your language, separated by `|`, in the order given in the file header (CLDR categories, https://unicode.org/cldr/charts/latest/supplemental/language_plural_rules.html):

```
@plural<TAB>{n} member<TAB>{n} membro|{n} membri
```

- Every catalog starts with a header: language, license (`MIT OR Apache-2.0`), date, and `Reviewed by:`. Put your name or handle there when you review a catalog.

## Rules

- Do not translate "Gnobbi", "Matrix" or other product names.
- Keep placeholders such as `{n}` and `{name}` exactly as they are.
- Keep the tone consistent and concise: the text must fit buttons and menus.
- Translate what exists. Do not add help text or new strings; to propose new wording, open an issue.
- Untranslated strings fall back to English, so partial catalogs are fine.

## Add a language

1. Create `crates/matrix-desktop/locales/<code>.tsv` (copy the header of another file). For a language that is already registered the empty file is there.
2. For a language not yet in `LANGUAGES`, add a row to that table in `crates/matrix-desktop/src/i18n.rs`: code, native name, the file with `include_str!`, the CLDR plural function and the number of plural forms. Scripts and right-to-left languages are handled in `crates/matrix-desktop/src/i18n/script.rs`.

## Try it

```sh
GNOBBI_LANG=de cargo run -p matrix-desktop --bin gnobbi
cargo i18n
```

`GNOBBI_LANG` forces the language for that run (any registered code, even with an empty catalog). `cargo i18n` lists, per language, the strings still missing and the malformed rows, for example a `@plural` row with the wrong number of forms. `cargo test -p matrix-desktop --lib i18n` checks that every catalog parses.

## Send it

Fork the repository, edit only your own `locales/<code>.tsv` (one language per pull request avoids conflicts), run `cargo i18n`, and open a pull request. Say whether a native speaker has reviewed the text.
