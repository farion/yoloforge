//! UI language is a local preference. Never translate user content or API values.
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::cell::Cell;
#[cfg(not(test))]
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    Auto,
    English,
    German,
}

impl Language {
    pub const ALL: [Self; 3] = [Self::Auto, Self::English, Self::German];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::English => "English",
            Self::German => "Deutsch",
        }
    }

    pub fn resolve(self, locale: &str) -> Self {
        match self {
            Self::Auto => {
                let language = locale.split(['_', '-', '.', '@']).next().unwrap_or("");
                if language.eq_ignore_ascii_case("de") {
                    Self::German
                } else {
                    Self::English
                }
            }
            explicit => explicit,
        }
    }
}

// One desktop process has one local preference, shared with backend workers.
// Independent render tests use thread-local preferences instead.
#[cfg(not(test))]
static GERMAN: AtomicBool = AtomicBool::new(false);
#[cfg(test)]
thread_local! {
    // Legacy geometry tests explicitly use German; bilingual tests select their locale.
    static GERMAN: Cell<bool> = const { Cell::new(true) };
}

pub fn is_german() -> bool {
    #[cfg(test)]
    {
        GERMAN.with(Cell::get)
    }
    #[cfg(not(test))]
    {
        GERMAN.load(Ordering::Relaxed)
    }
}

pub fn set_language(language: Language) {
    let locale = system_locale();
    let german = language.resolve(&locale) == Language::German;
    #[cfg(test)]
    GERMAN.with(|current| current.set(german));
    #[cfg(not(test))]
    GERMAN.store(german, Ordering::Relaxed);
}

fn system_locale() -> String {
    // sys-locale uses native APIs on Windows/macOS, locale environment on Unix.
    // LC_ALL and LC_MESSAGES must take precedence over LANG for message language.
    #[cfg(unix)]
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            if !value.is_empty() {
                return value;
            }
        }
    }
    sys_locale::get_locale().unwrap_or_else(|| "en".into())
}

include!(concat!(env!("OUT_DIR"), "/messages.rs"));

/// Re-render retained UI metadata after switching languages. Only use for
/// application-owned titles/help text, never for user content or server output.
pub fn label(text: &str) -> &str {
    CATALOG
        .iter()
        .find_map(|(key, de, en)| {
            (*key == text || *de == text || *en == text).then_some(if is_german() {
                *de
            } else {
                *en
            })
        })
        .unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_resolves_german_and_falls_back_to_english() {
        for locale in ["de", "de_DE.UTF-8", "de-CH", "DE_at@euro"] {
            assert_eq!(Language::Auto.resolve(locale), Language::German);
        }
        for locale in ["", "C", "POSIX", "en_US.UTF-8", "fr-FR", "den"] {
            assert_eq!(Language::Auto.resolve(locale), Language::English);
        }
        assert_eq!(Language::English.resolve("de"), Language::English);
        assert_eq!(Language::German.resolve("en"), Language::German);
    }

    #[test]
    fn missing_preference_defaults_to_auto_and_round_trips() {
        let settings: crate::persistence::Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.language, Language::Auto);
        for language in Language::ALL {
            let json = serde_json::to_string(&language).unwrap();
            assert_eq!(serde_json::from_str::<Language>(&json).unwrap(), language);
        }
    }

    #[test]
    fn catalog_is_complete_and_uses_intent_terminology() {
        assert!(CATALOG.len() > 700);
        for (key, de, en) in CATALOG {
            assert!(
                !de.is_empty() && !en.is_empty(),
                "missing translation: {key}"
            );
            assert!(!de.contains("Aufgabe"), "old terminology: {key}");
            assert!(!en.contains("Aufgabe"), "old terminology: {key}");
        }
    }

    #[test]
    fn language_switch_changes_static_and_formatted_text() {
        set_language(Language::German);
        assert_eq!(crate::tr!("Speichern"), "Speichern");
        assert_eq!(crate::tr!("Aufgabe {id}", id = 42), "Intent 42");
        set_language(Language::English);
        assert_eq!(crate::tr!("Speichern"), "Save");
        assert_eq!(crate::tr!("Aufgabe {id}", id = 42), "Intent 42");
    }
}
