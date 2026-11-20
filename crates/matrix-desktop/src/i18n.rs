//! Display-time localization. English strings are the keys: `t("Cancel")`.
//! Catalogs are `i18n/<code>.tsv` (`context<TAB>source<TAB>translation`), embedded with `include_str!`.
//! The scheme follows CADCraft (MIT OR Apache-2.0, see THIRD_PARTY_NOTICES); the catalogs are ours.
//! A missing language, string or malformed row falls back to the English key and never panics.
pub mod script;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

/// What the user chose; `Auto` follows the system locales.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preference {
    #[default]
    Auto,
    English,
    Italian,
}

impl Preference {
    pub const ALL: [Preference; 3] = [Preference::Auto, Preference::English, Preference::Italian];

    pub fn code(self) -> &'static str {
        match self {
            Preference::Auto => "auto",
            Preference::English => "en",
            Preference::Italian => "it",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.code() == code)
    }

    /// The language to show. `Auto` uses `GNOBBI_LANG` (tests, support) when it names a supported
    /// language, else the first supported one in `locales`, else English.
    pub fn resolve(self, locales: &[String]) -> &'static str {
        resolve_with(self, std::env::var("GNOBBI_LANG").ok().as_deref(), locales)
    }
}

fn resolve_with(pref: Preference, env: Option<&str>, locales: &[String]) -> &'static str {
    if pref != Preference::Auto {
        return locale_language(pref.code()).unwrap_or("en");
    }
    env.into_iter().chain(locales.iter().map(String::as_str)).find_map(locale_language).unwrap_or("en")
}

pub struct Language {
    pub code: &'static str,
    pub source: &'static str,
    /// Index of the `@plural` form for a count.
    pub plural: fn(u64) -> usize,
    /// Number of `|`-separated forms every `@plural` row carries.
    pub plural_forms: usize,
    catalog: OnceLock<Catalog>,
}

/// Add a language: a `.tsv`, a row here (with its CLDR plural rule) and a `Preference` variant.
pub static LANGUAGES: [Language; 2] = [
    Language { code: "en", source: "", plural: plural_one_other, plural_forms: 2, catalog: OnceLock::new() },
    // CLDR `it` integer counts: one (1), other.
    Language { code: "it", source: include_str!("i18n/it.tsv"), plural: plural_one_other, plural_forms: 2, catalog: OnceLock::new() },
];

/// CLDR `en`/`it`/`de`/`es`…: one (1), other.
pub fn plural_one_other(n: u64) -> usize {
    usize::from(n != 1)
}

/// BCP 47 and POSIX tags (`it-IT`, `it_CH.UTF-8`) select a catalog by primary language.
pub fn locale_language(tag: &str) -> Option<&'static str> {
    let primary = tag.trim().split(['-', '_', '.', '@']).next()?;
    LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(primary)).map(|l| l.code)
}

/// context -> source -> translation, unescaped.
pub type Catalog = HashMap<String, HashMap<String, String>>;

fn unescape(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Malformed rows are skipped (and reported); the coverage test turns them into failures.
pub fn parse(source: &str) -> (Catalog, Vec<String>) {
    let mut catalog = Catalog::new();
    let mut errors = Vec::new();
    for (i, line) in source.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns: Vec<_> = line.split('\t').collect();
        let [context, key, value] = columns.as_slice() else {
            errors.push(format!("line {}: expected context, source, translation", i + 1));
            continue;
        };
        if key.is_empty() || value.is_empty() {
            errors.push(format!("line {}: empty source or translation", i + 1));
        } else {
            let entries = catalog.entry(unescape(context)).or_default();
            match entries.entry(unescape(key)) {
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert(unescape(value));
                }
                // The first row wins.
                std::collections::hash_map::Entry::Occupied(_) => errors.push(format!("line {}: duplicate key {key}", i + 1)),
            }
        }
    }
    (catalog, errors)
}

fn language(code: &str) -> Option<&'static Language> {
    LANGUAGES.iter().find(|l| l.code == code)
}

fn catalog(code: &str) -> Option<&'static Catalog> {
    language(code).filter(|l| !l.source.is_empty()).map(|l| l.catalog.get_or_init(|| parse(l.source).0))
}

/// Translation of `source` in `lang`, or `source` itself.
pub fn tr<'a>(lang: &str, context: &str, source: &'a str) -> &'a str {
    catalog(lang).and_then(|c| c.get(context)?.get(source)).map_or(source, String::as_str)
}

#[cfg(not(test))]
static CURRENT: AtomicUsize = AtomicUsize::new(0);
// Tests run on parallel threads: a language per thread keeps text assertions apart from the hot-switch test.
#[cfg(test)]
thread_local!(static CURRENT: AtomicUsize = const { AtomicUsize::new(0) });

fn swap_current(index: usize) -> usize {
    #[cfg(not(test))]
    return CURRENT.swap(index, Ordering::Relaxed);
    #[cfg(test)]
    CURRENT.with(|c| c.swap(index, Ordering::Relaxed))
}

/// The active language code (`en` until a preference is applied).
pub fn current() -> &'static str {
    #[cfg(not(test))]
    let index = CURRENT.load(Ordering::Relaxed);
    #[cfg(test)]
    let index = CURRENT.with(|c| c.load(Ordering::Relaxed));
    LANGUAGES[index.min(LANGUAGES.len() - 1)].code
}

/// Is the active language written right to left? egui has no bidi or mirrored layout: callers pick
/// `Layout::right_to_left` / `Align::RIGHT` with this.
pub fn rtl() -> bool {
    script::is_rtl(current())
}

pub fn t(source: &str) -> &str {
    tr(current(), "", source)
}

/// With an explicit context, e.g. `t_ctx("menu", "View")`; falls back to the plain translation.
pub fn t_ctx<'a>(context: &str, source: &'a str) -> &'a str {
    let lang = current();
    match catalog(lang).and_then(|c| c.get(context)?.get(source)) {
        Some(v) => v,
        None => tr(lang, "", source),
    }
}

/// `{n}` counts: `plural(3, "{n} message", "{n} messages")`. The catalog row is
/// `@plural<TAB>{n} message<TAB>{n} messaggio|{n} messaggi`; without one the English forms are used.
pub fn plural(n: u64, one: &str, other: &str) -> String {
    plural_in(current(), n, one, other)
}

pub fn plural_in(lang: &str, n: u64, one: &str, other: &str) -> String {
    let forms = catalog(lang).and_then(|c| c.get("@plural")?.get(one));
    let index = language(lang).map_or(usize::from(n != 1), |l| (l.plural)(n));
    let form = forms.and_then(|f| f.split('|').nth(index)).unwrap_or(if n == 1 { one } else { other });
    form.replace("{n}", &n.to_string())
}

type Hook = Box<dyn Fn(&'static str) + Send + Sync>;
static HOOKS: Mutex<Vec<Hook>> = Mutex::new(Vec::new());

/// Called after every language change with the new code: rebuild native menus and the tray here.
pub fn on_change(hook: impl Fn(&'static str) + Send + Sync + 'static) {
    HOOKS.lock().unwrap_or_else(|e| e.into_inner()).push(Box::new(hook));
}

/// Applies a preference now, without restart. Hooks run only when the language actually changes;
/// the repaint is requested either way. Returns the active code.
pub fn apply(pref: Preference, ctx: Option<&eframe::egui::Context>) -> &'static str {
    let locales = sys_locale::get_locales().collect::<Vec<_>>();
    let code = script::usable(pref.resolve(&locales));
    let index = LANGUAGES.iter().position(|l| l.code == code).unwrap_or(0);
    if swap_current(index) != index {
        for hook in HOOKS.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            hook(code);
        }
    }
    if let Some(ctx) = ctx {
        ctx.request_repaint();
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# comment\n\nmenu\tView\tVisualizza\n\tCancel\tAnnulla\n\tA\\tB\\nC\\\\D\tX\\tY\\nZ\n@plural\t{n} message\t{n} messaggio|{n} messaggi\nbad row\n\tCancel\tDup\n";

    #[test]
    fn parses_escapes_and_reports_bad_rows() {
        let (c, errors) = parse(SAMPLE);
        assert_eq!(c["menu"]["View"], "Visualizza");
        assert_eq!(c[""]["A\tB\nC\\D"], "X\tY\nZ");
        assert_eq!(c[""]["Cancel"], "Annulla");
        assert_eq!(errors.len(), 2, "{errors:?}");
    }

    #[test]
    fn falls_back_to_the_key() {
        assert_eq!(tr("it", "", "Not in the catalog"), "Not in the catalog");
        assert_eq!(tr("xx", "", "Cancel"), "Cancel");
        assert_eq!(tr("en", "menu", "View"), "View");
        assert_eq!(plural_in("xx", 2, "{n} cat", "{n} cats"), "2 cats");
    }

    #[test]
    fn plural_rules_it_en() {
        assert_eq!([0, 1, 2, 11].map(plural_one_other), [1, 0, 1, 1]);
        assert_eq!(plural_in("en", 1, "{n} file", "{n} files"), "1 file");
        assert_eq!(plural_in("it", 0, "{n} file", "{n} files"), "0 files"); // no row: English forms
    }

    #[test]
    fn auto_resolution_and_env_override() {
        let loc = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(resolve_with(Preference::Auto, None, &loc(&["de-DE", "it_IT.UTF-8", "en"])), "it");
        assert_eq!(resolve_with(Preference::Auto, None, &loc(&["fr-FR"])), "en");
        assert_eq!(resolve_with(Preference::Auto, None, &[]), "en");
        assert_eq!(resolve_with(Preference::Auto, Some("it-IT"), &loc(&["en-US"])), "it");
        assert_eq!(resolve_with(Preference::Auto, Some("zz"), &loc(&["it"])), "it");
        assert_eq!(resolve_with(Preference::English, Some("it"), &loc(&["it"])), "en");
        assert_eq!(resolve_with(Preference::Italian, None, &[]), "it");
    }

    #[test]
    fn catalogs_are_well_formed() {
        for l in &LANGUAGES {
            let (c, errors) = parse(l.source);
            assert!(errors.is_empty(), "{}: {errors:?}", l.code);
            for (k, v) in c.get("@plural").into_iter().flatten() {
                assert_eq!(v.split('|').count(), l.plural_forms, "{}: {k}", l.code);
            }
        }
    }

    /// `t(name)` on the emoji names is not a literal call: the coverage scan cannot see these keys.
    #[test]
    fn emoji_names_are_translated() {
        for (name, _) in crate::EMOJI_SET {
            assert_ne!(tr("it", "", name), name, "{name}");
        }
    }

    #[test]
    fn hot_switch_runs_hooks_once_per_change() {
        use std::sync::atomic::AtomicU32;
        static CALLS: AtomicU32 = AtomicU32::new(0);
        on_change(|_| {
            CALLS.fetch_add(1, Ordering::SeqCst);
        });
        let ctx = eframe::egui::Context::default();
        assert_eq!(apply(Preference::Italian, Some(&ctx)), "it");
        assert_eq!(current(), "it");
        apply(Preference::Italian, Some(&ctx));
        assert_eq!(apply(Preference::English, None), "en");
        assert_eq!(CALLS.load(Ordering::SeqCst), 2);
    }
}
