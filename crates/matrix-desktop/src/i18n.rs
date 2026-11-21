//! Display-time localization. English strings are the keys: `t("Cancel")`.
//! Catalogs are `locales/<code>.tsv` (`context<TAB>source<TAB>translation`), embedded with `include_str!`.
//! The scheme follows CADCraft (MIT OR Apache-2.0, see THIRD_PARTY_NOTICES); the catalogs are ours.
//! `LANGUAGES` is the registry of every language; see TRANSLATING.md.
//! A missing language, string or malformed row falls back to the English key and never panics.
pub mod script;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

/// What the user chose: `Auto` follows the system locales, `Fixed(code)` a registered language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preference {
    #[default]
    Auto,
    Fixed(&'static str),
}

impl Preference {
    /// The value saved in the settings file: `auto` or a language code.
    pub fn code(self) -> &'static str {
        match self {
            Preference::Auto => "auto",
            Preference::Fixed(code) => code,
        }
    }

    /// `auto` or the code of a registered language (exact, e.g. `pt-BR`); anything else is `None`.
    pub fn parse(code: &str) -> Option<Self> {
        if code == "auto" {
            return Some(Preference::Auto);
        }
        LANGUAGES.iter().find(|l| l.code == code).map(|l| Preference::Fixed(l.code))
    }

    /// What the «Language» selector offers: `Auto` and every language with a non-empty catalog.
    pub fn choices() -> Vec<Preference> {
        std::iter::once(Preference::Auto).chain(LANGUAGES.iter().filter(|l| l.available()).map(|l| Preference::Fixed(l.code))).collect()
    }

    /// Each language in its own name, whatever the interface language; fixed on purpose.
    pub fn name(self) -> &'static str {
        match self {
            Preference::Auto => "Auto",
            Preference::Fixed(code) => language(code).map_or(code, |l| l.name),
        }
    }

    /// The language to show. `Auto` uses `GNOBBI_LANG` (tests, support) when it names a registered
    /// language, else the first system locale with a non-empty catalog, else English.
    pub fn resolve(self, locales: &[String]) -> &'static str {
        resolve_with(self, std::env::var("GNOBBI_LANG").ok().as_deref(), locales)
    }
}

fn resolve_with(pref: Preference, env: Option<&str>, locales: &[String]) -> &'static str {
    if let Preference::Fixed(code) = pref {
        return code;
    }
    let from_env = env.and_then(locale_language);
    let from_system = locales.iter().filter_map(|l| locale_language(l)).find(|c| language(c).is_some_and(Language::available));
    from_env.or(from_system).unwrap_or("en")
}

pub struct Language {
    pub code: &'static str,
    /// The language's own name, shown in the selector.
    pub name: &'static str,
    /// The catalog file, `locales/<code>.tsv` (header only until translated). `en` has none.
    pub source: &'static str,
    /// Index of the `@plural` form for a count.
    pub plural: fn(u64) -> usize,
    /// Number of `|`-separated forms every `@plural` row carries.
    pub plural_forms: usize,
    catalog: OnceLock<Catalog>,
}

impl Language {
    const fn new(code: &'static str, name: &'static str, source: &'static str, plural: fn(u64) -> usize, plural_forms: usize) -> Self {
        Language { code, name, source, plural, plural_forms, catalog: OnceLock::new() }
    }

    /// `en` always; any other language once its catalog has at least one translation.
    pub fn available(&self) -> bool {
        self.code == "en" || catalog(self.code).is_some_and(|c| !c.is_empty())
    }
}

/// The language registry, the only list. A new language: add a row here with its CLDR plural rule and
/// form count (https://unicode.org/cldr/charts/latest/supplemental/language_plural_rules.html), and
/// create `locales/<code>.tsv`. Rule functions take integers only.
pub static LANGUAGES: [Language; 23] = [
    Language::new("en", "English", "", plural_one_other, 2),
    Language::new("it", "Italiano", include_str!("../locales/it.tsv"), plural_one_other, 2),
    Language::new("ar", "العربية", include_str!("../locales/ar.tsv"), plural_arabic, 6),
    Language::new("bs", "Bosanski", include_str!("../locales/bs.tsv"), plural_slavic, 3),
    Language::new("cs", "Čeština", include_str!("../locales/cs.tsv"), plural_czech, 3),
    Language::new("de", "Deutsch", include_str!("../locales/de.tsv"), plural_one_other, 2),
    Language::new("el", "Ελληνικά", include_str!("../locales/el.tsv"), plural_one_other, 2),
    Language::new("es", "Español", include_str!("../locales/es.tsv"), plural_one_other, 2),
    Language::new("fr", "Français", include_str!("../locales/fr.tsv"), plural_zero_or_one, 2),
    Language::new("hu", "Magyar", include_str!("../locales/hu.tsv"), plural_one_other, 2),
    Language::new("ka", "ქართული", include_str!("../locales/ka.tsv"), plural_one_other, 2),
    Language::new("ko", "한국어", include_str!("../locales/ko.tsv"), plural_other, 1),
    Language::new("pl", "Polski", include_str!("../locales/pl.tsv"), plural_polish, 3),
    Language::new("pt-BR", "Português (Brasil)", include_str!("../locales/pt-BR.tsv"), plural_zero_or_one, 2),
    Language::new("ru", "Русский", include_str!("../locales/ru.tsv"), plural_slavic, 3),
    Language::new("sv", "Svenska", include_str!("../locales/sv.tsv"), plural_one_other, 2),
    Language::new("tr", "Türkçe", include_str!("../locales/tr.tsv"), plural_one_other, 2),
    Language::new("uk", "Українська", include_str!("../locales/uk.tsv"), plural_slavic, 3),
    // CLDR 49 gives `vi` one (0 and 1) and other.
    Language::new("vi", "Tiếng Việt", include_str!("../locales/vi.tsv"), plural_zero_or_one, 2),
    Language::new("zh", "简体中文", include_str!("../locales/zh.tsv"), plural_other, 1),
    Language::new("ja", "日本語", include_str!("../locales/ja.tsv"), plural_other, 1),
    Language::new("bg", "Български", include_str!("../locales/bg.tsv"), plural_one_other, 2),
    // CLDR 49 `he`: one, two, other (`many` was dropped in CLDR 42).
    Language::new("he", "עברית", include_str!("../locales/he.tsv"), plural_hebrew, 3),
];

// CLDR 49 cardinal rules on integers only (shortcut: no decimals, counts here are whole; upgrade if a
// fractional count is ever shown). `it`/`es`/`fr`/`pt` also have `many` for exact multiples of
// 1,000,000 ("1 milione di"): folded into `other`.

/// `en it de es sv hu tr el bg ka`: one (1), other.
pub fn plural_one_other(n: u64) -> usize {
    usize::from(n != 1)
}

/// `fr pt-BR vi`: one (0 and 1), other.
pub fn plural_zero_or_one(n: u64) -> usize {
    usize::from(n > 1)
}

/// `zh ja ko`: other only.
pub fn plural_other(_: u64) -> usize {
    0
}

/// `ru uk` one|few|many and `bs` one|few|other (same cut-offs): 1, 21, 101… / 2-4, 22-24… / the rest.
pub fn plural_slavic(n: u64) -> usize {
    match (n % 10, n % 100) {
        (1, r) if r != 11 => 0,
        (2..=4, r) if !(12..=14).contains(&r) => 1,
        _ => 2,
    }
}

/// `pl`: one (1), few (2-4, 22-24…), many.
pub fn plural_polish(n: u64) -> usize {
    match (n, n % 10, n % 100) {
        (1, ..) => 0,
        (_, 2..=4, r) if !(12..=14).contains(&r) => 1,
        _ => 2,
    }
}

/// `cs`: one (1), few (2-4), other.
pub fn plural_czech(n: u64) -> usize {
    match n {
        1 => 0,
        2..=4 => 1,
        _ => 2,
    }
}

/// `ar`: zero, one, two, few (3-10 mod 100), many (11-99 mod 100), other.
pub fn plural_arabic(n: u64) -> usize {
    match (n, n % 100) {
        (0, _) => 0,
        (1, _) => 1,
        (2, _) => 2,
        (_, 3..=10) => 3,
        (_, 11..=99) => 4,
        _ => 5,
    }
}

/// `he`: one (1), two (2), other.
pub fn plural_hebrew(n: u64) -> usize {
    match n {
        1 => 0,
        2 => 1,
        _ => 2,
    }
}

/// BCP 47 and POSIX tags (`it-IT`, `it_CH.UTF-8`) select a registered language by primary subtag;
/// `pt*` is `pt-BR`, `zh`/`zh-CN`/`zh-Hans*` is `zh` (Traditional `zh-Hant`/`TW`/`HK`/`MO` has no catalog), `iw` is `he`.
pub fn locale_language(tag: &str) -> Option<&'static str> {
    let tag = tag.trim().split(['.', '@']).next()?.to_ascii_lowercase();
    let mut parts = tag.split(['-', '_']);
    let primary = parts.next()?;
    let code = match primary {
        "iw" => "he",
        "pt" => "pt-br",
        "zh" if parts.any(|p| matches!(p, "hant" | "tw" | "hk" | "mo")) => return None,
        other => other,
    };
    LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code)).map(|l| l.code)
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

/// Malformed rows are skipped (and reported); the coverage test turns them into failures. An `@plural`
/// row must carry exactly `plural_forms` forms.
pub fn parse(source: &str, plural_forms: usize) -> (Catalog, Vec<String>) {
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
        } else if *context == "@plural" && value.split('|').count() != plural_forms {
            errors.push(format!("line {}: @plural needs {plural_forms} forms separated by |: {key}", i + 1));
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
    language(code).filter(|l| !l.source.is_empty()).map(|l| l.catalog.get_or_init(|| parse(l.source, l.plural_forms).0))
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

/// Is the active language written right to left? The egui app mirrors its whole layout on this,
/// through its `dir` module.
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
    // The notification bodies are built in matrix-domain; give it our translator (set once).
    matrix_domain::notifications::set_translator(|k| t(k).to_owned());
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
        let (c, errors) = parse(SAMPLE, 2);
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

    /// CLDR 49 integer rules: index per count, one row per distinct rule.
    #[test]
    fn plural_rules_by_cldr_table() {
        const N: [u64; 11] = [0, 1, 2, 3, 5, 11, 21, 22, 25, 101, 111];
        let table: [(&str, fn(u64) -> usize, [usize; 11]); 8] = [
            ("one_other", plural_one_other, [1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1]),
            ("zero_or_one", plural_zero_or_one, [0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1]),
            ("other", plural_other, [0; 11]),
            ("slavic", plural_slavic, [2, 0, 1, 1, 2, 2, 0, 1, 2, 0, 2]),
            ("polish", plural_polish, [2, 0, 1, 1, 2, 2, 2, 1, 2, 2, 2]),
            ("czech", plural_czech, [2, 0, 1, 1, 2, 2, 2, 2, 2, 2, 2]),
            ("arabic", plural_arabic, [0, 1, 2, 3, 3, 4, 4, 4, 4, 5, 4]),
            ("hebrew", plural_hebrew, [2, 0, 1, 2, 2, 2, 2, 2, 2, 2, 2]),
        ];
        for (name, rule, want) in table {
            assert_eq!(N.map(rule), want, "{name}");
        }
        for l in &LANGUAGES {
            assert!(N.iter().all(|&n| (l.plural)(n) < l.plural_forms), "{}", l.code);
        }
    }

    #[test]
    fn registry_and_locale_tags() {
        assert_eq!(LANGUAGES.len(), 23); // en + the 22 target languages
        for (tag, want) in [("pt_BR.UTF-8", "pt-BR"), ("pt", "pt-BR"), ("pt-PT", "pt-BR"), ("zh-Hans-CN", "zh"), ("zh_CN", "zh"), ("zh", "zh"), ("iw", "he"), ("IT-it", "it"), ("nb", ""), ("zh-TW", ""), ("zh-Hant", "")] {
            assert_eq!(locale_language(tag).unwrap_or(""), want, "{tag}");
        }
        assert_eq!(Preference::parse("pt-BR"), Some(Preference::Fixed("pt-BR")));
        assert_eq!(Preference::parse("auto"), Some(Preference::Auto));
        assert_eq!(Preference::parse("xx"), None);
        assert_eq!(Preference::Fixed("ar").name(), "العربية");
        // The selector shows exactly Auto plus the languages with translations (en always).
        let shown = Preference::choices();
        assert_eq!(shown[..2], [Preference::Auto, Preference::Fixed("en")]);
        for l in &LANGUAGES {
            assert_eq!(shown.contains(&Preference::Fixed(l.code)), l.available(), "{}", l.code);
        }
    }

    #[test]
    fn plural_row_with_wrong_form_count_is_reported() {
        let (c, errors) = parse("@plural\t{n} cat\t{n} gatto\n@plural\t{n} dog\t{n} cane|{n} cani\n", 2);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("line 1"), "{errors:?}");
        assert_eq!(c["@plural"].len(), 1);
    }

    #[test]
    fn plural_rules_it_en() {
        assert_eq!([0, 1, 2, 11].map(plural_one_other), [1, 0, 1, 1]);
        assert_eq!(plural_in("en", 1, "{n} file", "{n} files"), "1 file");
        assert_eq!(plural_in("it", 0, "{n} file", "{n} files"), "0 files"); // no row: English forms
    }

    #[test]
    fn plural_with_real_catalog_rows_zero_one_many() {
        let f = |l, n| plural_in(l, n, "{n} member", "{n} members");
        assert_eq!([0, 1, 5].map(|n| f("it", n)), ["0 membri", "1 membro", "5 membri"]);
        assert_eq!([0, 1, 5].map(|n| f("en", n)), ["0 members", "1 member", "5 members"]);
    }

    #[test]
    fn auto_resolution_and_env_override() {
        let loc = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(resolve_with(Preference::Auto, None, &loc(&["de-DE", "it_IT.UTF-8", "en"])), "it");
        assert_eq!(resolve_with(Preference::Auto, None, &loc(&["fr-FR"])), "en");
        assert_eq!(resolve_with(Preference::Auto, None, &[]), "en");
        assert_eq!(resolve_with(Preference::Auto, Some("it-IT"), &loc(&["en-US"])), "it");
        assert_eq!(resolve_with(Preference::Auto, Some("zz"), &loc(&["it"])), "it");
        assert_eq!(resolve_with(Preference::Fixed("en"), Some("it"), &loc(&["it"])), "en");
        assert_eq!(resolve_with(Preference::Fixed("it"), None, &[]), "it");
        // Auto only lands on a language with a catalog, but GNOBBI_LANG takes any registered one (RTL tryout).
        assert!(!LANGUAGES.iter().find(|l| l.code == "he").unwrap().available());
        assert_eq!(resolve_with(Preference::Auto, None, &loc(&["he-IL", "it"])), "it");
        assert_eq!(resolve_with(Preference::Auto, Some("he"), &[]), "he");
    }

    #[test]
    fn catalogs_are_well_formed() {
        for l in &LANGUAGES {
            let (_, errors) = parse(l.source, l.plural_forms);
            assert!(errors.is_empty(), "{}: {errors:?}", l.code);
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
        assert_eq!(apply(Preference::Fixed("it"), Some(&ctx)), "it");
        assert_eq!(current(), "it");
        apply(Preference::Fixed("it"), Some(&ctx));
        assert_eq!(apply(Preference::Fixed("en"), None), "en");
        assert_eq!(CALLS.load(Ordering::SeqCst), 2);
    }
}
