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
/// - `zh*` → Chinese (`zh-CN`) — NOTE: zh-CN locale file is not yet shipped;
///   the code path is forward-compatible and harmless.
/// - anything else → English (`en`)
pub fn set_locale(locale: &str) {
    let locale = if locale.starts_with("ru") {
        RU_LOCALE
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
mod tests {
    use super::*;

    #[test]
    fn test_default_locale_is_en() {
        assert_eq!(current_locale(), "en");
    }

    #[test]
    fn test_set_locale_ru() {
        set_locale("ru_RU.UTF-8");
        assert_eq!(current_locale(), "ru");
    }

    #[test]
    fn test_set_locale_zh() {
        set_locale("zh_CN.UTF-8");
        assert_eq!(current_locale(), "zh-CN");
    }

    #[test]
    fn test_set_locale_unknown_falls_back_to_en() {
        set_locale("de");
        assert_eq!(current_locale(), "en");
    }

    #[test]
    fn test_interpolate() {
        let result = interpolate("Hello, {name}!", &[("name", "World".into())]);
        assert_eq!(result, "Hello, World!");
    }

    #[test]
    fn test_interpolate_multiple() {
        let result = interpolate(
            "{greeting}, {name}!",
            &[("greeting", "Hi".into()), ("name", "Alice".into())],
        );
        assert_eq!(result, "Hi, Alice!");
    }

    #[test]
    fn test_lookup_missing() {
        assert_eq!(lookup("nonexistent.key", "en"), TranslationLookup::Missing);
    }

    #[test]
    fn test_flatten_value_nested() {
        let yaml = r#"
en:
  menu:
    file: "File"
    edit: "Edit"
  appearance:
    opacity: "Opacity"
"#;
        let mut translations = Translations::new();
        merge_locale_file(yaml, &mut translations);
        let en = translations.get("en").expect("en locale should exist");
        assert_eq!(en.get("menu.file"), Some(&"File".to_string()));
        assert_eq!(en.get("menu.edit"), Some(&"Edit".to_string()));
        assert_eq!(en.get("appearance.opacity"), Some(&"Opacity".to_string()));
    }

    #[test]
    fn test_yaml_key_balance_en_equals_ru() {
        // Both locale files must have the same set of keys.
        // This prevents drift where a key is added to en.yml but forgotten in ru.yml.
        let mut en_translations = Translations::new();
        let mut ru_translations = Translations::new();

        merge_locale_file(
            include_str!("../../../resources/bundled/locales/en.yml"),
            &mut en_translations,
        );
        merge_locale_file(
            include_str!("../../../resources/bundled/locales/ru.yml"),
            &mut ru_translations,
        );

        let en_keys: std::collections::BTreeSet<&String> = en_translations
            .get("en")
            .expect("en locale missing")
            .keys()
            .collect();
        let ru_keys: std::collections::BTreeSet<&String> = ru_translations
            .get("ru")
            .expect("ru locale missing")
            .keys()
            .collect();

        let en_only: Vec<_> = en_keys.difference(&ru_keys).collect();
        let ru_only: Vec<_> = ru_keys.difference(&en_keys).collect();

        assert!(
            en_only.is_empty(),
            "Keys in en.yml missing from ru.yml: {en_only:?}"
        );
        assert!(
            ru_only.is_empty(),
            "Keys in ru.yml missing from en.yml: {ru_only:?}"
        );
    }

    #[test]
    fn test_yaml_no_key_collisions() {
        let en = include_str!("../../../resources/bundled/locales/en.yml");
        let ru = include_str!("../../../resources/bundled/locales/ru.yml");
        for (raw, name) in [(en, "en.yml"), (ru, "ru.yml")] {
            let value: serde_yaml::Value = serde_yaml::from_str(raw).expect("invalid YAML");
            let mut collisions = Vec::new();
            walk_collisions(&[], &value, &mut collisions);
            assert!(
                collisions.is_empty(),
                "YAML key collision in {name}: {collisions:?}"
            );
        }
    }

    fn walk_collisions(path: &[&str], value: &serde_yaml::Value, out: &mut Vec<String>) {
        let serde_yaml::Value::Mapping(map) = value else {
            return;
        };
        let mut scalars: Vec<Vec<&str>> = Vec::new();
        let mut map_paths: Vec<Vec<&str>> = Vec::new();
        for (k, v) in map {
            let Some(key) = k.as_str() else { continue };
            if key == "_comment" {
                continue;
            }
            let mut child = path.to_vec();
            child.push(key);
            if matches!(v, serde_yaml::Value::Mapping(_)) {
                map_paths.push(child.clone());
                walk_collisions(&child, v, out);
            } else {
                scalars.push(child);
            }
        }
        for s in &scalars {
            if map_paths.contains(s) {
                out.push(s.join("."));
            }
        }
    }

    #[test]
    fn test_flatten_value_with_comment() {
        let yaml = r#"
en:
  _comment: "this should be skipped"
  menu:
    file: "File"
"#;
        let mut translations = Translations::new();
        merge_locale_file(yaml, &mut translations);
        let en = translations.get("en").expect("en locale should exist");
        // _comment at root level is filtered out
        assert_eq!(en.get("_comment"), None);
        assert_eq!(en.get("menu.file"), Some(&"File".to_string()));
    }

    // ── Orphan-key gate ────────────────────────────────────────
    //
    // `menu_label(key, fallback)` silently returns the English fallback string when
    // `key` is absent from any locale. That means a typo / missing translation in
    // `en.yml` would ship English text to Russian users without any compile-time
    // signal. This test catches the failure mode: every `menu_label("<dotted.key>", …)`
    // call site must reference a key defined in `en.yml`.
    //
    // Excluded scope:
    //   - `app/src/settings_view/**` — owned by needsbuilder (PR warpdotdev/warp#13374)
    //   - `*_tests.rs` files           — only run under `cargo test`, never shipped
    //   - `#[cfg(test)] mod … { … }` blocks — same reason; stripped before scanning
    //
    // Limitation:
    //   - Only string LITERALS are scanned. Keys built at runtime via `format!`,
    //     concatenation (`+`), `const &str` references, or variables are NOT checked.
    //     Audit those manually when adding new keys.
    //
    // If this test fails, add the missing keys to `resources/bundled/locales/en.yml`
    // (and `ru.yml`) BEFORE merging — never weaken this test by adding an allowlist.

    /// Walk ancestors of `CARGO_MANIFEST_DIR` looking for the workspace `app/src/lib.rs`.
    /// Returns the FIRST (deepest) match, which is robust to the i18n crate being
    /// relocated, the workspace being nested, or unrelated `app/` directories appearing
    /// higher up the path. (Previously used `take(5)` + `is_dir()` which hard-coded
    /// the workspace shape and could pick up an unrelated `app/src/` with no `lib.rs`.)
    fn find_app_src() -> Option<PathBuf> {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        // skip(1) drops the i18n dir itself (`.../crates/i18n`); the workspace root
        // is the next ancestor up.
        for ancestor in manifest_dir.ancestors().skip(1) {
            if ancestor.join("app").join("src").join("lib.rs").is_file() {
                return Some(ancestor.join("app").join("src"));
            }
        }
        None
    }

    /// Recursively collect `.rs` files under `root`, excluding `settings_view/**` and
    /// `*_tests.rs` (separate-file test modules — body lives in the file, but they are
    /// never compiled into the production binary).
    fn walk_rs_files(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                // Exclude settings_view/ entirely (needsbuilder scope, PR #13374).
                if path.is_dir() {
                    if name == "settings_view" {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }
                if !path.is_file() || !name.ends_with(".rs") || name.ends_with("_tests.rs") {
                    continue;
                }
                out.push(path);
            }
        }
        out.sort();
        out
    }

    /// Strip `#[cfg(test)] mod <name> { … }` bodies (with brace-balanced inner scopes,
    /// and string/comment awareness) so menu_label() calls inside test modules are not
    /// counted as production usage. Multi-attribute forms
    /// (`#[cfg(test)] #[path = "…"] mod tests;`) leave no body in this file, so they
    /// are no-ops here.
    fn strip_cfg_test_mods(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut i = 0;

        while i < bytes.len() {
            // Find next `#[cfg(test)]` (allow internal whitespace).
            let Some(rel_start) = find_cfg_test_attr(bytes, i) else {
                out.push_str(&text[i..]);
                break;
            };
            // Append everything before the cfg(test) attribute.
            out.push_str(&text[i..rel_start]);
            i = rel_start;
            // Advance past the cfg(test) attribute itself.
            let cfg_end = consume_attr(bytes, i).expect("found #[cfg(test)] without closing ]");
            i = cfg_end;

            // Skip any further `#[…]` attributes between cfg(test) and the mod decl.
            loop {
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if bytes.get(i) == Some(&b'#') {
                    match consume_attr(bytes, i) {
                        Some(end) => i = end,
                        None => break,
                    }
                } else {
                    break;
                }
            }

            // Expect `mod <name>`. Skip whitespace, then check keyword.
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if !text[i..].starts_with("mod ") {
                // Not a mod block (e.g., `#[cfg(test)] use foo;`). Stripping the attr
                // alone is harmless — the body it gates is elsewhere or nonexistent.
                continue;
            }
            i += 4;
            // Skip mod name.
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
            {
                i += 1;
            }
            // Skip whitespace; expect `;` (external mod, no body here) or `{` (body).
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b';' => {
                    // External test module — body lives in another file (already
                    // excluded by the `_tests.rs` skip in walk_rs_files). Nothing
                    // more to strip in this file.
                    i += 1;
                }
                b'{' => {
                    // Inline test module — brace-balanced skip of the body, with
                    // string-literal and comment awareness so `{`/`}` inside strings
                    // or `/* */` comments don't fool the counter.
                    let mut k = i + 1;
                    let mut depth: u32 = 1;
                    let mut in_string = false;
                    let mut in_line_comment = false;
                    let mut in_block_comment = false;
                    while k < bytes.len() && depth > 0 {
                        let c = bytes[k];
                        if in_string {
                            if c == b'\\' && k + 1 < bytes.len() {
                                k += 2;
                                continue;
                            }
                            if c == b'"' {
                                in_string = false;
                            }
                        } else if in_line_comment {
                            if c == b'\n' {
                                in_line_comment = false;
                            }
                        } else if in_block_comment {
                            if c == b'*' && bytes.get(k + 1) == Some(&b'/') {
                                in_block_comment = false;
                                k += 1;
                            }
                        } else {
                            match c {
                                b'/' if bytes.get(k + 1) == Some(&b'/') => {
                                    in_line_comment = true;
                                    k += 1;
                                }
                                b'/' if bytes.get(k + 1) == Some(&b'*') => {
                                    in_block_comment = true;
                                    k += 1;
                                }
                                b'"' => {
                                    in_string = true;
                                }
                                b'{' => depth += 1,
                                b'}' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        k += 1;
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                        k += 1;
                    }
                    i = k;
                }
                _ => {
                    // Unexpected token; bail out to avoid eating real code.
                    break;
                }
            }
        }
        out
    }

    /// Return the byte index of the start of the next `#[cfg(test)]` (with internal
    /// whitespace permitted) at or after `from`, or `None` if no such attribute exists.
    fn find_cfg_test_attr(bytes: &[u8], from: usize) -> Option<usize> {
        let mut i = from;
        while i + 1 < bytes.len() {
            if bytes[i] == b'#' && bytes[i + 1] == b'[' {
                // Try to consume the full attribute; check it's `cfg(test)`.
                if let Some(end) = consume_attr(bytes, i) {
                    let attr = &bytes[i..end];
                    if is_cfg_test(attr) {
                        return Some(i);
                    }
                    i = end;
                } else {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
        None
    }

    /// Consume a `#[…]` attribute (bracket-balanced) starting at `start`. Returns the
    /// index just past the closing `]`.
    fn consume_attr(bytes: &[u8], start: usize) -> Option<usize> {
        if bytes.get(start) != Some(&b'#') || bytes.get(start + 1) != Some(&b'[') {
            return None;
        }
        let mut i = start + 2;
        let mut depth: u32 = 1;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'[' => depth += 1,
                b']' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        if depth == 0 {
            Some(i)
        } else {
            None
        }
    }

    /// True if the byte slice (already including `#[` … `]`) is a `cfg(test)` attribute.
    fn is_cfg_test(attr: &[u8]) -> bool {
        // attr looks like `# [ cfg ( test ) ]` with arbitrary internal whitespace.
        // Strip whitespace and compare to `#[cfg(test)]`.
        let compact: Vec<u8> = attr
            .iter()
            .copied()
            .filter(|b| !b.is_ascii_whitespace())
            .collect();
        compact == b"#[cfg(test)]"
    }

    /// Strip Rust line (`//`) and block (`/* ... */`) comments from `text` while being
    /// string-aware: inside `"..."` string literals, `//` and `/*` are NOT treated as
    /// comments, so a URL literal like `"http://example.com/foo"` is preserved
    /// verbatim instead of being truncated at the `//`.
    ///
    /// Naive by design: we do NOT understand lifetimes, attributes (`#[...]`), doc
    /// comments (`///`, `//!`), or raw strings (`r"..."`). Removing those is fine for
    /// the orphan-key scan because `menu_label("…")` only appears in expression
    /// position. If a future call site hides inside a doc comment or raw string,
    /// that call will be silently missed — the same trade-off `strip_cfg_test_mods`
    /// already makes for `#[cfg(test)]` mod bodies.
    fn strip_comments(text: &str) -> String {
        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut out = String::with_capacity(text.len());
        let mut i = 0;

        while i < len {
            let c = bytes[i];

            // /* ... */ block comment (Rust doesn't nest these).
            if c == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
                i += 2;
                while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(len);
                continue;
            }

            // // ... line comment, up to (but not including) the newline.
            if c == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
                i += 2;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }

            // "..." string literal — copy verbatim (with backslash escapes) so URL
            // content like `"http://"` inside a string is preserved.
            if c == b'"' {
                out.push(c as char);
                i += 1;
                while i < len {
                    let cc = bytes[i];
                    if cc == b'\\' && i + 1 < len {
                        // Escape sequence: copy both bytes verbatim. Naive — does not
                        // validate the escape, but for our purposes (preserve content)
                        // that's fine.
                        out.push(cc as char);
                        out.push(bytes[i + 1] as char);
                        i += 2;
                        continue;
                    }
                    out.push(cc as char);
                    i += 1;
                    if cc == b'"' {
                        break;
                    }
                }
                continue;
            }

            out.push(c as char);
            i += 1;
        }

        out
    }

    /// Extract every `menu_label("<dotted.key>", …)` key literal from `text`.
    ///
    /// **Limitation — only string LITERALS are scanned.** Keys assembled at runtime
    /// via `format!`, concatenation (`+`), `const &str` references, variables, or
    /// any other non-literal mechanism are NOT checked by this gate. Audit those
    /// manually when adding new keys.
    ///
    /// Pattern: at least 2 dot-separated segments, lowercase + underscore + digit
    /// (digits are required because ~13 production keys contain digits, e.g.
    /// `agent.filter.last_24_hours`, `terminal.osc52.allow_button`,
    /// `settings.custom_router.editor.rules_help_1`).
    ///
    /// Callers should run `strip_cfg_test_mods` and `strip_comments` on the source
    /// before invoking this function so cfg(test) bodies and comments don't pollute
    /// the result.
    fn extract_menu_label_keys(text: &str) -> std::collections::BTreeSet<String> {
        // Anchored to the `menu_label(` opener so we don't pick up unrelated
        // string literals that happen to look like dotted keys.
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let re = RE.get_or_init(|| {
            // Allow digits in key segments so keys like
            // `agent.filter.last_24_hours`, `terminal.osc52.allow_button`,
            // and `settings.custom_router.editor.rules_help_1` are matched.
            regex::Regex::new(r#"menu_label\(\s*"([a-z_][a-z_0-9]*(?:\.[a-z_][a-z_0-9]*)+)""#)
                .expect("menu_label regex is valid")
        });
        let mut out = std::collections::BTreeSet::new();
        for capture in re.captures_iter(text) {
            if let Some(key_match) = capture.get(1) {
                out.insert(key_match.as_str().to_owned());
            }
        }
        out
    }

    #[test]
    fn test_no_orphan_menu_label_keys() {
        // 1. Locate `app/src/` from CARGO_MANIFEST_DIR (crates/i18n).
        let app_src = find_app_src().expect(
            "could not locate workspace `app/src` from CARGO_MANIFEST_DIR; \
             is this crate still nested under the warp workspace?",
        );

        // 2. Walk .rs files (excluding settings_view/ and *_tests.rs).
        let rs_files = walk_rs_files(&app_src);
        assert!(
            !rs_files.is_empty(),
            "no .rs files found under {} — did the workspace layout change?",
            app_src.display()
        );

        // 3. Collect all menu_label("…") keys used in non-test, non-settings_view code.
        let mut used: std::collections::BTreeSet<String> =
            std::collections::BTreeSet::new();
        let mut per_key_files: std::collections::BTreeMap<String, Vec<PathBuf>> =
            std::collections::BTreeMap::new();
        for path in &rs_files {
            let raw = match std::fs::read_to_string(path) {
                Ok(text) => text,
                Err(err) => {
                    eprintln!("warn: skipping {}: {err}", path.display());
                    continue;
                }
            };
            let stripped = strip_cfg_test_mods(&raw);
            // Strip Rust comments (line + block, string-aware) from the OUTER
            // (non-cfg-test) text so `// menu_label("foo.bar", …)` in a comment
            // is not captured as real usage. Inner cfg(test) bodies already had
            // their comments handled correctly by `strip_cfg_test_mods`.
            let cleaned = strip_comments(&stripped);
            for key in extract_menu_label_keys(&cleaned) {
                used.insert(key.clone());
                per_key_files.entry(key).or_default().push(path.clone());
            }
        }

        // 4. Load en.yml and flatten to the defined-key set.
        let mut en_translations = Translations::new();
        merge_locale_file(
            include_str!("../../../resources/bundled/locales/en.yml"),
            &mut en_translations,
        );
        let defined: std::collections::BTreeSet<&String> = en_translations
            .get("en")
            .expect("en locale missing from en.yml")
            .keys()
            .collect();

        // 5. Compute orphans (used \ defined), sorted for deterministic output.
        let orphans: Vec<&String> = used.iter().filter(|k| !defined.contains(k)).collect();
        let orphans_sorted: Vec<&&String> = {
            let mut v: Vec<&&String> = orphans.iter().collect();
            v.sort();
            v
        };

        if !orphans_sorted.is_empty() {
            let mut msg = String::from(
                "menu_label keys used in code but missing from en.yml \
                 (these would silently fall back to English on Russian builds):\n",
            );
            for key in &orphans_sorted {
                let files = per_key_files
                    .get(key.as_str())
                    .map(|paths| {
                        paths
                            .iter()
                            .map(|p| {
                                p.strip_prefix(&app_src)
                                    .map(|rel| rel.display().to_string())
                                    .unwrap_or_else(|_| p.display().to_string())
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                msg.push_str(&format!("  {key}  (in: {files})\n"));
            }
            msg.push_str(&format!(
                "\nTotal: {} orphan key(s). Add them to \
                 resources/bundled/locales/en.yml (and ru.yml) — DO NOT add an \
                 allowlist. settings_view/ is already excluded (needsbuilder's scope, \
                 PR warpdotdev/warp#13374).\n",
                orphans_sorted.len()
            ));
            panic!("{msg}");
        }

        // Sanity: the scan must actually exercise something substantial. With the
        // current wide regex + comment-stripping pipeline we expect ~440+ unique
        // keys from non-settings_view production code. Require >= 100 so a partial
        // regex regression (e.g. someone narrowing the character class back to
        // `[a-z_]+`) is caught loudly instead of silently passing.
        assert!(
            used.len() >= 100,
            "scan extracted only {} menu_label keys from {} files under {} — \
             expected >= 100 (~440+ in practice); the regex or walker is likely \
             broken (e.g. character class too narrow, or strip pipeline regressed)",
            used.len(),
            rs_files.len(),
            app_src.display()
        );
    }
}
