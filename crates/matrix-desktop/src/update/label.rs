//! The update switch is the first localized desktop label.

pub fn update_check_label() -> &'static str {
    label_for(sys_locale::get_locale().as_deref().unwrap_or("en-US"))
}

pub fn label_for(locale: &str) -> &'static str {
    let language = locale.split(['-', '_', '.','@']).next().unwrap_or("");
    if language.eq_ignore_ascii_case("it") { "Controlla aggiornamenti" }
    else if language.eq_ignore_ascii_case("de") { "Nach Updates suchen" }
    else if language.eq_ignore_ascii_case("fr") { "Rechercher les mises à jour" }
    else if language.eq_ignore_ascii_case("es") { "Buscar actualizaciones" }
    else if language.eq_ignore_ascii_case("pt") { "Procurar atualizações" }
    else { "Check for updates" }
}

#[cfg(test)]
mod tests {
    use super::label_for;
    #[test]
    fn supported_locales_and_fallback() {
        for (locale, expected) in [
            ("it-IT", "Controlla aggiornamenti"), ("en-US", "Check for updates"),
            ("de-DE", "Nach Updates suchen"), ("fr-FR", "Rechercher les mises à jour"),
            ("es-ES", "Buscar actualizaciones"), ("pt-PT", "Procurar atualizações"),
            ("it_IT.UTF-8", "Controlla aggiornamenti"), ("ja-JP", "Check for updates"),
            ("", "Check for updates"),
        ] { assert_eq!(label_for(locale), expected, "{locale}"); }
    }
}
