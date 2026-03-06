//! Internationalization (i18n) — translation loading and lookup.
//!
//! Uses YAML files in `i18n/` directory. Each file is `{locale}.yaml`
//! with flat key-value pairs: `"section.key": "Translated text"`.
//!
//! Fallback chain: requested locale -> English -> key itself.
//!
//! Usage:
//!   let i18n = I18n::load("es");
//!   let text = i18n.tr("installer.welcome.title"); // "Bienvenido a Yantrik OS"
//!   let rtl = i18n.is_rtl(); // false for Spanish

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

/// Supported locales with metadata.
pub static SUPPORTED_LOCALES: &[LocaleInfo] = &[
    LocaleInfo { code: "en", name: "English", native: "English", flag: "\u{1F1FA}\u{1F1F8}", rtl: false },
    LocaleInfo { code: "es", name: "Spanish", native: "Espa\u{00F1}ol", flag: "\u{1F1EA}\u{1F1F8}", rtl: false },
    LocaleInfo { code: "fr", name: "French", native: "Fran\u{00E7}ais", flag: "\u{1F1EB}\u{1F1F7}", rtl: false },
    LocaleInfo { code: "de", name: "German", native: "Deutsch", flag: "\u{1F1E9}\u{1F1EA}", rtl: false },
    LocaleInfo { code: "pt", name: "Portuguese", native: "Portugu\u{00EA}s", flag: "\u{1F1E7}\u{1F1F7}", rtl: false },
    LocaleInfo { code: "zh", name: "Chinese", native: "\u{4E2D}\u{6587}", flag: "\u{1F1E8}\u{1F1F3}", rtl: false },
    LocaleInfo { code: "ja", name: "Japanese", native: "\u{65E5}\u{672C}\u{8A9E}", flag: "\u{1F1EF}\u{1F1F5}", rtl: false },
    LocaleInfo { code: "ko", name: "Korean", native: "\u{D55C}\u{AD6D}\u{C5B4}", flag: "\u{1F1F0}\u{1F1F7}", rtl: false },
    LocaleInfo { code: "ar", name: "Arabic", native: "\u{0627}\u{0644}\u{0639}\u{0631}\u{0628}\u{064A}\u{0629}", flag: "\u{1F1F8}\u{1F1E6}", rtl: true },
    LocaleInfo { code: "hi", name: "Hindi", native: "\u{0939}\u{093F}\u{0928}\u{094D}\u{0926}\u{0940}", flag: "\u{1F1EE}\u{1F1F3}", rtl: false },
];

#[derive(Debug, Clone)]
pub struct LocaleInfo {
    pub code: &'static str,
    pub name: &'static str,
    pub native: &'static str,
    pub flag: &'static str,
    pub rtl: bool,
}

/// Thread-safe translation store.
#[derive(Clone)]
pub struct I18n {
    inner: Arc<RwLock<I18nInner>>,
}

struct I18nInner {
    locale: String,
    strings: HashMap<String, String>,
    fallback: HashMap<String, String>, // English fallback
    rtl: bool,
}

impl I18n {
    /// Load translations for a locale. Falls back to English for missing keys.
    pub fn load(locale: &str) -> Self {
        let base_dir = Self::i18n_dir();
        let fallback = Self::load_yaml(&base_dir, "en");
        let strings = if locale == "en" {
            fallback.clone()
        } else {
            Self::load_yaml(&base_dir, locale)
        };

        let rtl = SUPPORTED_LOCALES
            .iter()
            .find(|l| l.code == locale)
            .map(|l| l.rtl)
            .unwrap_or(false);

        tracing::info!(
            locale,
            keys = strings.len(),
            fallback_keys = fallback.len(),
            rtl,
            "Loaded translations"
        );

        I18n {
            inner: Arc::new(RwLock::new(I18nInner {
                locale: locale.to_string(),
                strings,
                fallback,
                rtl,
            })),
        }
    }

    /// Switch to a different locale at runtime.
    pub fn switch_locale(&self, locale: &str) {
        let base_dir = Self::i18n_dir();
        let strings = Self::load_yaml(&base_dir, locale);
        let rtl = SUPPORTED_LOCALES
            .iter()
            .find(|l| l.code == locale)
            .map(|l| l.rtl)
            .unwrap_or(false);

        if let Ok(mut inner) = self.inner.write() {
            inner.locale = locale.to_string();
            inner.strings = strings;
            inner.rtl = rtl;
        }

        tracing::info!(locale, "Switched locale");
    }

    /// Translate a key. Returns: translated string > English fallback > key itself.
    pub fn tr(&self, key: &str) -> String {
        if let Ok(inner) = self.inner.read() {
            if let Some(s) = inner.strings.get(key) {
                return s.clone();
            }
            if let Some(s) = inner.fallback.get(key) {
                return s.clone();
            }
        }
        // Last resort: return the key
        key.to_string()
    }

    /// Translate with placeholder substitution. Replaces {0}, {1}, ... with args.
    pub fn tr_args(&self, key: &str, args: &[&str]) -> String {
        let mut result = self.tr(key);
        for (i, arg) in args.iter().enumerate() {
            result = result.replace(&format!("{{{}}}", i), arg);
        }
        result
    }

    /// Current locale code.
    pub fn locale(&self) -> String {
        self.inner.read().map(|i| i.locale.clone()).unwrap_or_else(|_| "en".to_string())
    }

    /// Whether current locale is RTL.
    pub fn is_rtl(&self) -> bool {
        self.inner.read().map(|i| i.rtl).unwrap_or(false)
    }

    /// Get all supported locales.
    pub fn supported_locales() -> &'static [LocaleInfo] {
        SUPPORTED_LOCALES
    }

    /// Detect system locale from environment.
    pub fn detect_locale() -> String {
        // Check LANG, LC_ALL, LC_MESSAGES
        for var in &["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(val) = std::env::var(var) {
                let code = val.split('.').next().unwrap_or("en");
                let short = code.split('_').next().unwrap_or("en");
                // Check if we support this locale
                if SUPPORTED_LOCALES.iter().any(|l| l.code == short) {
                    return short.to_string();
                }
            }
        }
        "en".to_string()
    }

    /// Find the i18n directory. Checks multiple locations.
    fn i18n_dir() -> PathBuf {
        // 1. Next to the binary (deployed: /opt/yantrik/i18n/)
        if let Ok(exe) = std::env::current_exe() {
            let dir = exe.parent().unwrap_or(Path::new("/")).join("i18n");
            if dir.is_dir() {
                return dir;
            }
            // Also check one level up (bin/yantrik-ui -> i18n/)
            let dir = exe
                .parent()
                .and_then(|p| p.parent())
                .unwrap_or(Path::new("/"))
                .join("i18n");
            if dir.is_dir() {
                return dir;
            }
        }

        // 2. /opt/yantrik/i18n/ (standard deploy path)
        let sys = PathBuf::from("/opt/yantrik/i18n");
        if sys.is_dir() {
            return sys;
        }

        // 3. Relative to CWD (development)
        let cwd = PathBuf::from("i18n");
        if cwd.is_dir() {
            return cwd;
        }

        // 4. Source tree path (development on Windows)
        let src = PathBuf::from("crates/yantrik-ui/i18n");
        if src.is_dir() {
            return src;
        }

        // Fallback
        PathBuf::from("/opt/yantrik/i18n")
    }

    /// Load a single YAML translation file.
    fn load_yaml(base_dir: &Path, locale: &str) -> HashMap<String, String> {
        let path = base_dir.join(format!("{}.yaml", locale));
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                match serde_yaml::from_str::<HashMap<String, String>>(&content) {
                    Ok(map) => map,
                    Err(e) => {
                        tracing::warn!(locale, error = %e, "Failed to parse translation file");
                        HashMap::new()
                    }
                }
            }
            Err(e) => {
                if locale != "en" {
                    tracing::debug!(locale, error = %e, "Translation file not found, using English fallback");
                } else {
                    tracing::warn!("English translation file not found at {:?}", path);
                }
                HashMap::new()
            }
        }
    }
}

impl Default for I18n {
    fn default() -> Self {
        Self::load("en")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supported_locales() {
        assert_eq!(SUPPORTED_LOCALES.len(), 10);
        assert_eq!(SUPPORTED_LOCALES[0].code, "en");
        assert!(SUPPORTED_LOCALES.iter().any(|l| l.code == "ar" && l.rtl));
    }

    #[test]
    fn test_tr_fallback() {
        let i18n = I18n {
            inner: Arc::new(RwLock::new(I18nInner {
                locale: "xx".to_string(),
                strings: HashMap::new(),
                fallback: HashMap::new(),
                rtl: false,
            })),
        };
        // Returns key itself when no translation exists
        assert_eq!(i18n.tr("missing.key"), "missing.key");
    }

    #[test]
    fn test_tr_args() {
        let mut strings = HashMap::new();
        strings.insert("greeting".to_string(), "Hello, {0}! You have {1} messages.".to_string());
        let i18n = I18n {
            inner: Arc::new(RwLock::new(I18nInner {
                locale: "en".to_string(),
                strings,
                fallback: HashMap::new(),
                rtl: false,
            })),
        };
        assert_eq!(
            i18n.tr_args("greeting", &["Pranab", "5"]),
            "Hello, Pranab! You have 5 messages."
        );
    }
}
