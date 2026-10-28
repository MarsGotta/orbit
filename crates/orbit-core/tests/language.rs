use orbit_core::language::{LanguageChoice, ResolvedLanguage};

#[test]
fn resolves_preferred_languages_in_order_and_falls_back_to_english() {
    for (preferred, expected) in [
        (vec!["es-ES"], ResolvedLanguage::Spanish),
        (vec!["en-GB"], ResolvedLanguage::English),
        (vec!["fr-FR"], ResolvedLanguage::English),
        (vec!["fr-FR", "es-MX", "en-US"], ResolvedLanguage::Spanish),
        (vec!["en-US", "es-ES"], ResolvedLanguage::English),
        (vec![], ResolvedLanguage::English),
    ] {
        assert_eq!(LanguageChoice::System.resolve(&preferred), expected);
    }
    assert_eq!(
        LanguageChoice::English.resolve(&["es-ES"]),
        ResolvedLanguage::English
    );
    assert_eq!(
        LanguageChoice::Spanish.resolve(&["en-GB"]),
        ResolvedLanguage::Spanish
    );
}
