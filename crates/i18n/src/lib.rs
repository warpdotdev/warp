use std::borrow::Cow;
use std::collections::HashMap;
#[cfg(not(target_family = "wasm"))]
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

const DEFAULT_LOCALE: &str = "en";
const ES_LOCALE: &str = "es";
#[cfg(not(target_family = "wasm"))]
const LOCALES_DIR: &str = "bundled/locales";

type Locale = String;
type Key = String;
type Translations = HashMap<Locale, HashMap<Key, String>>;

static CURRENT_LOCALE: RwLock<&'static str> = RwLock::new(DEFAULT_LOCALE);
static TRANSLATIONS: OnceLock<Translations> = OnceLock::new();

pub fn init_locale() {
    let locale = env_locale()
        .or_else(sys_locale::get_locale)
        .unwrap_or_default();

    set_locale(&locale);
}

pub fn set_locale(locale: &str) {
    let locale = supported_locale(locale);

    if let Ok(mut current_locale) = CURRENT_LOCALE.write() {
        *current_locale = locale;
    }
}

fn supported_locale(locale: &str) -> &'static str {
    let language = locale
        .trim()
        .split(['-', '_', '.', ':'])
        .next()
        .unwrap_or_default();
    if language.eq_ignore_ascii_case("es") {
        ES_LOCALE
    } else {
        DEFAULT_LOCALE
    }
}

#[macro_export]
macro_rules! t {
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::interpolate(
            $crate::text($key),
            &[$((stringify!($name), format!("{}", $value))),+],
        )
    };
    ($key:literal, $($name:ident),+ $(,)?) => {
        $crate::interpolate(
            $crate::text($key),
            &[$((stringify!($name), format!("{}", $name))),+],
        )
    };
    ($key:literal) => {
        $crate::text($key)
    };
}

pub fn text(key: &'static str) -> &'static str {
    lookup(translations(), current_locale(), key)
}

/// Translates predefined option labels without changing their stored values.
pub fn option_label(label: &str) -> &str {
    match label {
        "Normal" => text("settings_values.normal"),
        "Bold" => text("settings_values.bold"),
        "Bar" => text("settings_values.bar"),
        "Block" => text("settings_values.block"),
        "Underline" => text("settings_values.underline"),
        "Light" => text("settings_values.light"),
        "Dark" => text("settings_values.dark"),
        "English" => text("settings_values.english"),
        "Spanish" => text("settings_values.spanish"),
        "French" => text("settings_values.french"),
        "German" => text("settings_values.german"),
        "Portuguese" => text("settings_values.portuguese"),
        "Italian" => text("settings_values.italian"),
        "Chinese" => text("settings_values.chinese"),
        "Japanese" => text("settings_values.japanese"),
        "Korean" => text("settings_values.korean"),
        "Russian" => text("settings_values.russian"),
        "Arabic" => text("settings_values.arabic"),
        "Dutch" => text("settings_values.dutch"),
        "Catalan" => text("settings_values.catalan"),
        "Basque" => text("settings_values.basque"),
        "Galician" => text("settings_values.galician"),
        "Polish" => text("settings_values.polish"),
        "Swedish" => text("settings_values.swedish"),
        "Norwegian" => text("settings_values.norwegian"),
        "Danish" => text("settings_values.danish"),
        "Finnish" => text("settings_values.finnish"),
        "Greek" => text("settings_values.greek"),
        "Turkish" => text("settings_values.turkish"),
        "Ukrainian" => text("settings_values.ukrainian"),
        "Hindi" => text("settings_values.hindi"),
        "Hebrew" => text("settings_values.hebrew"),
        _ => label,
    }
}

/// Translates font weights separately from theme brightness labels.
pub fn font_weight_label(label: &str) -> &str {
    match label {
        "Thin" => text("settings_values.thin"),
        "ExtraLight" => text("settings_values.extra_light"),
        "Light" => text("settings_values.light_font"),
        "Medium" => text("settings_values.medium"),
        "Semibold" => text("settings_values.semibold"),
        "ExtraBold" => text("settings_values.extra_bold"),
        "Black" => text("settings_values.black"),
        _ => option_label(label),
    }
}

fn lookup<'a>(translations: &'a Translations, locale: &str, key: &'a str) -> &'a str {
    translations
        .get(locale)
        .and_then(|values| values.get(key))
        .or_else(|| {
            translations
                .get(DEFAULT_LOCALE)
                .and_then(|values| values.get(key))
        })
        .map(String::as_str)
        .unwrap_or(key)
}

pub fn interpolate(template: &str, args: &[(&str, String)]) -> Cow<'static, str> {
    let mut value = String::with_capacity(template.len());
    let mut remaining = template;
    while let Some(start) = remaining.find('{') {
        value.push_str(&remaining[..start]);
        remaining = &remaining[start..];
        let Some(end) = remaining.find('}') else {
            break;
        };
        let key = &remaining[1..end];
        if let Some((_, replacement)) = args.iter().find(|(name, _)| *name == key) {
            value.push_str(replacement);
        } else {
            value.push_str(&remaining[..=end]);
        }
        remaining = &remaining[end + 1..];
    }
    value.push_str(remaining);
    Cow::Owned(value)
}

fn env_locale() -> Option<String> {
    ["WARP_LANG", "LANG", "LANGUAGE", "LC_ALL", "LC_MESSAGES"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
}

fn current_locale() -> &'static str {
    CURRENT_LOCALE
        .read()
        .map(|locale| *locale)
        .unwrap_or(DEFAULT_LOCALE)
}

fn translations() -> &'static Translations {
    TRANSLATIONS.get_or_init(load_translations)
}

fn load_translations() -> Translations {
    let mut translations = embedded_translations();

    #[cfg(not(target_family = "wasm"))]
    if let Some(overrides) = locale_dirs().into_iter().find_map(load_dir) {
        for (locale, values) in overrides {
            translations.entry(locale).or_default().extend(values);
        }
    }

    translations
}

fn embedded_translations() -> Translations {
    let mut translations = Translations::new();
    merge_locale_file(
        include_str!("../../../resources/bundled/locales/en.yml"),
        &mut translations,
    );
    merge_locale_file(
        include_str!("../../../resources/bundled/locales/es.yml"),
        &mut translations,
    );

    translations
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

    dirs
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

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
