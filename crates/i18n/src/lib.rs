//! Warp i18n — YAML-based internationalization.
//!
//! Locale resolution order:
//! 1. `WARP_LANG` env var
//! 2. System locale (via `sys-locale`)
//! 3. Fallback: `en`
//!
//! ## Usage
//!
//! ```ignore
//! i18n::init_locale();
//! let text = i18n::t!("menu.file"); // "File" or "Файл"
//! ```

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

const DEFAULT_LOCALE: &str = "en";
const RU_LOCALE: &str = "ru";
const KO_LOCALE: &str = "ko";
const ZH_CN_LOCALE: &str = "zh-CN";
const LOCALES_DIR: &str = "bundled/locales";

type Locale = String;
type Key = String;
type Translations = HashMap<Locale, HashMap<Key, String>>;

static CURRENT_LOCALE: RwLock<&'static str> = RwLock::new(DEFAULT_LOCALE);
static TRANSLATIONS: OnceLock<Translations> = OnceLock::new();

// ── Public API ──────────────────────────────────────────────────

/// Detect locale from `WARP_LANG`, system locale, or default to `en`.
/// Loads translations and sets the active locale.
pub fn init_locale() {
    let locale = env_locale()
        .or_else(sys_locale::get_locale)
        .unwrap_or_default();

    set_locale(&locale);
}

/// Set the active locale.
///
/// Maps locale prefixes to supported locales:
/// - `ru*` → Russian (`ru`)
/// - `ko*` → Korean (`ko`)
/// - `zh*` → Chinese (`zh-CN`) — NOTE: zh-CN locale file is not yet shipped;
///   the code path is forward-compatible and harmless.
/// - anything else → English (`en`)
pub fn set_locale(locale: &str) {
    let locale = if locale.starts_with("ru") {
        RU_LOCALE
    } else if locale.starts_with("ko") {
        KO_LOCALE
    } else if locale.starts_with("zh") {
        ZH_CN_LOCALE
    } else {
        DEFAULT_LOCALE
    };

    if let Ok(mut current_locale) = CURRENT_LOCALE.write() {
        *current_locale = locale;
    } else {
        log::error!("i18n: CURRENT_LOCALE lock poisoned, locale not changed");
    }
}

/// Look up a translation key for the active locale.
///
/// Falls back to `en`, then returns the key itself if no translation exists.
pub fn t(key: &str) -> Cow<'static, str> {
    translate(current_locale(), key)
        .or_else(|| translate(DEFAULT_LOCALE, key))
        .unwrap_or(Cow::Owned(key.to_string()))
}

/// Interpolate `{key}` placeholders in a template string.
///
/// ```ignore
/// let msg = i18n::interpolate("Hello, {name}!", &[("name", "World".into())]);
/// assert_eq!(msg, "Hello, World!");
/// ```
pub fn interpolate(template: &str, args: &[(&str, String)]) -> Cow<'static, str> {
    let mut value = template.to_owned();
    for (key, replacement) in args {
        value = value.replace(&format!("{{{key}}}"), replacement);
    }
    Cow::Owned(value)
}

/// Returns the currently active locale code (e.g. `"en"`, `"ru"`).
pub fn current_locale() -> &'static str {
    CURRENT_LOCALE
        .read()
        .map(|locale| *locale)
        .unwrap_or(DEFAULT_LOCALE)
}

// ── TranslationLookup ───────────────────────────────────────────

/// Result of a translation lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranslationLookup {
    /// Translation found.
    Found(Cow<'static, str>),
    /// No translation for the given locale+key.
    Missing,
}

/// Look up a key for a specific locale, returning a `TranslationLookup`.
pub fn lookup(key: &str, locale: &str) -> TranslationLookup {
    match translations().get(locale).and_then(|t| t.get(key)) {
        Some(value) => TranslationLookup::Found(Cow::Owned(value.clone())),
        None => TranslationLookup::Missing,
    }
}

// ── Macros ──────────────────────────────────────────────────────

/// Look up a translation key for the active locale (1-arg form).
///
/// Falls back to English, then returns the key as-is.
///
/// ```ignore
/// let label = i18n::t!("menu.file"); // "File" or "Файл"
/// ```
#[macro_export]
macro_rules! t {
    ($key:expr) => {
        $crate::t($key)
    };
}

/// Look up a translation key, panicking if not found in any locale.
///
/// Use this for keys that MUST exist in both the target locale AND the
/// English fallback.
///
/// Reserved for security-sensitive UI per i18n spec.
///
/// ```ignore
/// let label = i18n::t_required!("menu.file"); // panics if missing
/// ```
#[macro_export]
macro_rules! t_required {
    ($key:expr) => {{
        let key: &'static str = $key;
        match $crate::lookup(key, $crate::current_locale()) {
            $crate::TranslationLookup::Found(v) => v,
            $crate::TranslationLookup::Missing => match $crate::lookup(key, "en") {
                $crate::TranslationLookup::Found(v) => v,
                $crate::TranslationLookup::Missing => {
                    panic!("i18n: required key '{}' not found in any locale", key);
                }
            },
        }
    }};
}

// ── Internal helpers ────────────────────────────────────────────

fn env_locale() -> Option<String> {
    ["WARP_LANG", "LANG", "LANGUAGE", "LC_ALL", "LC_MESSAGES"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
}

fn translate(locale: &str, key: &str) -> Option<Cow<'static, str>> {
    translations()
        .get(locale)
        .and_then(|t| t.get(key))
        .map(|value| Cow::Borrowed(value.as_str()))
}

fn translations() -> &'static Translations {
    TRANSLATIONS.get_or_init(load_translations)
}

// ── Platform-specific loading ───────────────────────────────────

#[cfg(not(target_family = "wasm"))]
fn load_translations() -> Translations {
    locale_dirs()
        .into_iter()
        .find_map(load_dir)
        .unwrap_or_default()
}

#[cfg(not(target_family = "wasm"))]
fn locale_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Ok(path) = std::env::var("WARP_LOCALES_DIR") {
        dirs.push(path.into());
    }

    if let Some(resources_dir) = bundled_resources_dir() {
        dirs.push(resources_dir.join(LOCALES_DIR));
    }

    if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR") {
        dirs.extend(resource_dirs_from_manifest(Path::new(manifest_dir)));
    }

    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("resources").join(LOCALES_DIR));
    }

    dirs
}

#[cfg(not(target_family = "wasm"))]
fn resource_dirs_from_manifest(manifest_dir: &Path) -> Vec<PathBuf> {
    manifest_dir
        .ancestors()
        .take(4)
        .map(|dir| dir.join("resources").join(LOCALES_DIR))
        .collect()
}

#[cfg(not(target_family = "wasm"))]
fn bundled_resources_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let executable = std::env::current_exe().ok()?;
        let mut path = std::fs::canonicalize(executable).ok()?;
        while path.pop() {
            if path.extension().and_then(|extension| extension.to_str()) == Some("app") {
                return Some(path.join("Contents").join("Resources"));
            }
        }
        None
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "windows"))]
    {
        std::env::current_exe()
            .ok()
            .and_then(|executable| std::fs::canonicalize(executable).ok())
            .and_then(|executable| executable.parent().map(|parent| parent.join("resources")))
    }

    #[cfg(not(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "windows"
    )))]
    {
        None
    }
}

#[cfg(not(target_family = "wasm"))]
fn load_dir(path: PathBuf) -> Option<Translations> {
    let entries = std::fs::read_dir(path).ok()?;
    let mut translations = Translations::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
            continue;
        };

        if !matches!(extension, "yml" | "yaml") {
            continue;
        }

        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        merge_locale_file(&contents, &mut translations);
    }

    (!translations.is_empty()).then_some(translations)
}

#[cfg(target_family = "wasm")]
fn load_translations() -> Translations {
    let mut translations = Translations::new();
    merge_locale_file(
        include_str!("../../../resources/bundled/locales/en.yml"),
        &mut translations,
    );
    merge_locale_file(
        include_str!("../../../resources/bundled/locales/ru.yml"),
        &mut translations,
    );
    merge_locale_file(
        include_str!("../../../resources/bundled/locales/ko.yml"),
        &mut translations,
    );
    merge_locale_file(
        include_str!("../../../resources/bundled/locales/extra.yml"),
        &mut translations,
    );
    translations
}

fn merge_locale_file(contents: &str, translations: &mut Translations) {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(contents) else {
        return;
    };

    let serde_yaml::Value::Mapping(locales) = value else {
        return;
    };

    for (locale, values) in locales {
        let Some(locale) = locale.as_str() else {
            continue;
        };

        flatten_value(
            "",
            &values,
            translations.entry(locale.to_owned()).or_default(),
        );
    }
}

fn flatten_value(prefix: &str, value: &serde_yaml::Value, translations: &mut HashMap<Key, String>) {
    match value {
        serde_yaml::Value::Mapping(values) => {
            for (key, value) in values {
                let Some(key) = key.as_str() else {
                    continue;
                };

                // Filter out _comment keys so they never appear in lookups.
                if key == "_comment" {
                    continue;
                }

                let key = if prefix.is_empty() {
                    key.to_owned()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_value(&key, value, translations);
            }
        }
        serde_yaml::Value::String(value) => {
            translations.insert(prefix.to_owned(), value.to_owned());
        }
        _ => {}
    }
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
