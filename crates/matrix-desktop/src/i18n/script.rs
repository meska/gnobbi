//! Scripts, fonts and text direction per language. egui 0.37 (eframe `system_fonts`) already loads a system
//! font on demand for any glyph the bundled ones lack, so nothing is bundled: a language whose script has no
//! system font is not usable and `apply` falls back to English (no tofu).
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Script {
    Latin,
    Han,
    Kana,
    Hangul,
    Arabic,
    Hebrew,
    Georgian,
    Devanagari,
    Thai,
}

/// Script of a language code (primary subtag). Unknown codes count as Latin.
pub fn script_of(code: &str) -> Script {
    match code.split(['-', '_']).next().unwrap_or("") {
        "zh" => Script::Han,
        "ja" => Script::Kana,
        "ko" => Script::Hangul,
        "ar" | "fa" | "ur" => Script::Arabic,
        "he" | "iw" | "yi" => Script::Hebrew,
        "ka" => Script::Georgian,
        "hi" | "mr" | "ne" => Script::Devanagari,
        "th" => Script::Thai,
        _ => Script::Latin,
    }
}

pub fn is_rtl(code: &str) -> bool {
    matches!(script_of(code), Script::Arabic | Script::Hebrew)
}

fn sample(script: Script) -> &'static str {
    match script {
        Script::Latin => "a",
        Script::Han => "中",
        Script::Kana => "あ",
        Script::Hangul => "한",
        Script::Arabic => "ع",
        Script::Hebrew => "א",
        Script::Georgian => "ა",
        Script::Devanagari => "अ",
        Script::Thai => "ก",
    }
}

/// Does the system have a font with a glyph for the script? Asked once per script (the lookup enumerates the fonts).
fn system_has_font(script: Script) -> bool {
    static CACHE: Mutex<Option<HashMap<Script, bool>>> = Mutex::new(None);
    if script == Script::Latin {
        return true;
    }
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    *cache.get_or_insert_default().entry(script).or_insert_with(|| {
        use eframe::egui::{FallbackRequest, FontFamily, FontProvider};
        egui_system_font_provider::SystemFontProvider::new()
            .font_for(&FallbackRequest { cluster: sample(script), family: &FontFamily::Proportional })
            .is_some()
    })
}

/// `code` when its script can be drawn, else `en`.
pub fn usable(code: &'static str) -> &'static str {
    usable_with(code, system_has_font)
}

fn usable_with(code: &'static str, has_font: impl Fn(Script) -> bool) -> &'static str {
    if has_font(script_of(code)) { code } else { "en" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direction_per_language() {
        for c in ["ar", "ar-EG", "he", "fa", "ur"] {
            assert!(is_rtl(c), "{c}");
        }
        for c in ["en", "it", "ja", "zh-Hans", "ka", "hi", "th", ""] {
            assert!(!is_rtl(c), "{c}");
        }
    }

    #[test]
    fn language_without_font_falls_back_to_english() {
        assert_eq!(usable_with("ar", |_| false), "en");
        assert_eq!(usable_with("ar", |s| s == Script::Arabic), "ar");
        assert_eq!(usable_with("it", |s| s == Script::Latin), "it");
        assert_eq!(usable_with("ja", |s| s == Script::Kana), "ja");
    }

    #[test]
    fn latin_never_needs_a_probe() {
        assert!(system_has_font(Script::Latin));
        assert_eq!(usable("it"), "it");
    }
}
