//! Lightweight internationalization (i18n) for the Warp UI.
//!
//! Translation catalogs are embedded from the `locales/` directory at build
//! time (one flat JSON `key -> string` file per locale, e.g. `en.json`,
//! `zh-CN.json`) using `rust-embed`, mirroring the catalog approach already
//! used by the `languages` crate. A single active locale is held in a global
//! [`RwLock`] so any UI code can translate a key with the free function
//! [`t`] without threading a context through every call site.
//!
//! # Usage
//!
//! ```ignore
//! // At startup, after reading the saved language preference:
//! i18n::set_locale("zh-CN");
//!
//! // Anywhere in the UI — returns an owned `String`, which satisfies
//! // `impl Into<Cow<'static, str>>` accepted by `Text::new(..)` and friends:
//! let label = i18n::t("settings.appearance.language.label");
//! // or via the macro:
//! let label = i18n::t!("settings.appearance.language.label");
//! ```
//!
//! # Fallback
//!
//! Lookups degrade gracefully: the active locale's catalog is built by layering
//! the more specific file on top of less specific ones and the English base,
//! e.g. `zh-CN` resolves through `en -> zh -> zh-CN`. A key missing from every
//! catalog returns the key itself, so the UI never panics or shows a blank.

use std::collections::HashMap;
use std::sync::RwLock;

use once_cell::sync::Lazy;
use rust_embed::RustEmbed;

/// The catalog whose translations are considered the source of truth and the
/// final fallback for any key missing from the active locale.
pub const FALLBACK_LOCALE: &str = "en";

/// The locale selected on first run, before the user picks one. Chinese per
/// product requirement; keep in sync with `Language`'s `#[default]` in the app.
pub const DEFAULT_LOCALE: &str = "zh-CN";

#[derive(RustEmbed)]
#[folder = "locales"]
struct Locales;

/// A flat `key -> translated string` catalog for a single locale.
type Catalog = HashMap<String, String>;

struct State {
    /// BCP-47-ish tag of the active locale, e.g. `"zh-CN"`.
    active_tag: String,
    /// Active catalog: the English base with the active locale layered on top,
    /// so every key present in `en.json` resolves even if untranslated.
    active: Catalog,
}

static STATE: Lazy<RwLock<State>> = Lazy::new(|| {
    RwLock::new(State {
        active_tag: DEFAULT_LOCALE.to_string(),
        active: build_active(DEFAULT_LOCALE),
    })
});

/// Loads and parses the embedded catalog for `tag` (without the `.json`
/// suffix). Returns an empty catalog if the file is absent or malformed —
/// missing/invalid catalogs must never crash the UI.
fn load_catalog(tag: &str) -> Catalog {
    let path = format!("{tag}.json");
    match Locales::get(&path) {
        Some(file) => serde_json::from_slice(&file.data).unwrap_or_else(|err| {
            log::error!("i18n: failed to parse locale catalog `{path}`: {err}");
            Catalog::new()
        }),
        None => Catalog::new(),
    }
}

/// Returns the catalog file stems to layer for `tag`, least specific first, so
/// later entries override earlier ones.
///
/// `"zh-CN"` -> `["en", "zh", "zh-CN"]`; `"en"` -> `["en"]`.
fn merge_order(tag: &str) -> Vec<String> {
    let mut order = vec![FALLBACK_LOCALE.to_string()];
    if let Some((base, _region)) = tag.split_once('-') {
        if base != FALLBACK_LOCALE {
            order.push(base.to_string());
        }
    }
    if tag != FALLBACK_LOCALE && !order.iter().any(|stem| stem == tag) {
        order.push(tag.to_string());
    }
    order
}

/// Builds the merged active catalog for `tag` (English base + locale overrides).
fn build_active(tag: &str) -> Catalog {
    let mut merged = Catalog::new();
    for stem in merge_order(tag) {
        for (key, value) in load_catalog(&stem) {
            merged.insert(key, value);
        }
    }
    merged
}

/// Sets the active UI locale, rebuilding the active catalog. Idempotent for the
/// already-active tag. Call this at startup and whenever the language
/// preference changes; follow it with a full UI repaint so visible labels
/// update.
pub fn set_locale(tag: &str) {
    let mut state = STATE
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.active_tag == tag {
        return;
    }
    state.active = build_active(tag);
    state.active_tag = tag.to_string();
    log::info!("i18n: active locale set to `{tag}`");
}

/// Returns the tag of the currently active locale, e.g. `"zh-CN"`.
pub fn current_locale() -> String {
    STATE
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .active_tag
        .clone()
}

/// Translates `key` into the active locale. Returns the English fallback when
/// the active locale lacks the key, or the key itself when no catalog defines
/// it (so untranslated UI still renders something meaningful).
pub fn t(key: &str) -> String {
    let state = STATE
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state
        .active
        .get(key)
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

/// Ergonomic shorthand for [`t`]: `t!("some.key")`.
#[macro_export]
macro_rules! t {
    ($key:expr $(,)?) => {
        $crate::t($key)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_order_layers_region_over_base_over_english() {
        assert_eq!(merge_order("zh-CN"), vec!["en", "zh", "zh-CN"]);
        assert_eq!(merge_order("en"), vec!["en"]);
        assert_eq!(merge_order("fr"), vec!["en", "fr"]);
    }

    #[test]
    fn missing_key_returns_key() {
        set_locale("en");
        assert_eq!(t("this.key.does.not.exist"), "this.key.does.not.exist");
    }

    #[test]
    fn default_locale_is_chinese() {
        // The lazily-initialized state starts on the default locale.
        assert_eq!(DEFAULT_LOCALE, "zh-CN");
    }

    #[test]
    fn english_and_chinese_catalogs_have_the_same_keys() {
        let en = load_catalog(FALLBACK_LOCALE);
        let zh = load_catalog(DEFAULT_LOCALE);

        let mut missing_from_zh: Vec<_> = en.keys().filter(|key| !zh.contains_key(*key)).collect();
        let mut missing_from_en: Vec<_> = zh.keys().filter(|key| !en.contains_key(*key)).collect();
        missing_from_zh.sort();
        missing_from_en.sort();

        assert!(
            missing_from_zh.is_empty() && missing_from_en.is_empty(),
            "locale catalogs are out of sync; missing from zh-CN: {missing_from_zh:?}; missing from en: {missing_from_en:?}"
        );
    }
}
