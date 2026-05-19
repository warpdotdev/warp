use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::HashMap, fmt};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LocaleId {
    EnUs,
    ZhCn,
}

impl LocaleId {
    pub fn code(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::ZhCn => "zh-CN",
        }
    }

    pub fn from_system_locale(locale: &str) -> Option<Self> {
        let normalized = normalize_locale(locale)?;
        match normalized.as_str() {
            "c" | "posix" => None,
            "en" => Some(Self::EnUs),
            value if value.starts_with("en-") => Some(Self::EnUs),
            "zh" | "zh-cn" | "zh-sg" | "zh-hans" => Some(Self::ZhCn),
            value if value.starts_with("zh-hans-") => Some(Self::ZhCn),
            _ => None,
        }
    }
}

impl fmt::Display for LocaleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Deserialize,
    Eq,
    PartialEq,
    Serialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "The display language used by Warp.",
    rename_all = "snake_case"
)]
pub enum AppLanguage {
    #[default]
    System,
    English,
    SimplifiedChinese,
}

impl AppLanguage {
    pub const OPTIONS: [Self; 3] = [Self::System, Self::English, Self::SimplifiedChinese];

    pub fn effective_locale(self, system_locale: Option<&str>) -> LocaleId {
        self.effective_locale_from_candidates(system_locale)
    }

    pub fn effective_locale_from_candidates<I, S>(self, system_locales: I) -> LocaleId
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        match self {
            Self::System => system_locales
                .into_iter()
                .find_map(|locale| LocaleId::from_system_locale(locale.as_ref()))
                .unwrap_or(LocaleId::EnUs),
            Self::English => LocaleId::EnUs,
            Self::SimplifiedChinese => LocaleId::ZhCn,
        }
    }

    pub fn translation_key(self) -> &'static str {
        match self {
            Self::System => "settings.appearance.language.option.system",
            Self::English => "settings.appearance.language.option.english",
            Self::SimplifiedChinese => "settings.appearance.language.option.simplified_chinese",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("failed to parse {locale} localization catalog: {source}")]
    InvalidJson {
        locale: LocaleId,
        source: serde_json::Error,
    },
    #[error("localization catalog {locale} contains an empty key")]
    EmptyKey { locale: LocaleId },
    #[error("duplicate localization catalog for {0}")]
    DuplicateLocale(LocaleId),
    #[error("default localization catalog {0} is missing")]
    MissingDefault(LocaleId),
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum TemplateError {
    #[error("localization template argument {name} was provided more than once")]
    DuplicateArgument { name: String },
    #[error("localization template is missing argument {name}")]
    MissingArgument { name: String },
    #[error("localization template does not contain argument {name}")]
    UnusedArgument { name: String },
}

#[derive(Debug)]
pub struct Catalog {
    locale: LocaleId,
    entries: HashMap<String, String>,
}

impl Catalog {
    pub fn from_json(locale: LocaleId, source: &str) -> Result<Self, CatalogError> {
        let entries: HashMap<String, String> = serde_json::from_str(source)
            .map_err(|source| CatalogError::InvalidJson { locale, source })?;
        if entries.keys().any(|key| key.is_empty()) {
            return Err(CatalogError::EmptyKey { locale });
        }
        Ok(Self { locale, entries })
    }

    pub fn locale(&self) -> LocaleId {
        self.locale
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TranslationSource {
    RequestedLocale,
    DefaultLocale,
    Key,
}

#[derive(Debug, Eq, PartialEq)]
pub struct TranslationLookup<'a> {
    pub text: Cow<'a, str>,
    pub source: TranslationSource,
}

#[derive(Debug)]
pub struct CatalogBundle {
    default_locale: LocaleId,
    catalogs: HashMap<LocaleId, Catalog>,
}

impl CatalogBundle {
    pub fn new(
        default_locale: LocaleId,
        catalogs: impl IntoIterator<Item = Catalog>,
    ) -> Result<Self, CatalogError> {
        let mut catalog_map = HashMap::new();
        for catalog in catalogs {
            let locale = catalog.locale();
            let previous = catalog_map.insert(locale, catalog);
            if previous.is_some() {
                return Err(CatalogError::DuplicateLocale(locale));
            }
        }
        if !catalog_map.contains_key(&default_locale) {
            return Err(CatalogError::MissingDefault(default_locale));
        }
        Ok(Self {
            default_locale,
            catalogs: catalog_map,
        })
    }

    pub fn lookup<'a>(&'a self, locale: LocaleId, key: &'a str) -> TranslationLookup<'a> {
        if let Some(text) = self
            .catalogs
            .get(&locale)
            .and_then(|catalog| catalog.get(key))
        {
            return TranslationLookup {
                text: Cow::Borrowed(text),
                source: TranslationSource::RequestedLocale,
            };
        }
        if let Some(text) = self
            .catalogs
            .get(&self.default_locale)
            .and_then(|catalog| catalog.get(key))
        {
            return TranslationLookup {
                text: Cow::Borrowed(text),
                source: TranslationSource::DefaultLocale,
            };
        }
        TranslationLookup {
            text: Cow::Borrowed(key),
            source: TranslationSource::Key,
        }
    }

    pub fn text<'a>(&'a self, locale: LocaleId, key: &'a str) -> Cow<'a, str> {
        self.lookup(locale, key).text
    }
}

fn normalize_locale(locale: &str) -> Option<String> {
    let trimmed = locale.trim();
    if trimmed.is_empty() {
        return None;
    }
    let base = trimmed
        .split(['.', '@'])
        .next()
        .unwrap_or(trimmed)
        .replace('_', "-");
    Some(base.to_ascii_lowercase())
}

pub fn replace_placeholders(
    template: &str,
    args: &[(&str, &str)],
) -> Result<String, TemplateError> {
    for index in 0..args.len() {
        let name = args[index].0;
        if args.iter().skip(index + 1).any(|(other, _)| *other == name) {
            return Err(TemplateError::DuplicateArgument {
                name: name.to_owned(),
            });
        }
    }

    let mut output = String::with_capacity(template.len());
    let mut used_args = vec![false; args.len()];
    let mut rest = template;

    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        rest = &rest[start + 1..];

        if rest.starts_with('{') {
            output.push('{');
            rest = &rest[1..];
            continue;
        }

        let Some(end) = rest.find('}') else {
            output.push('{');
            output.push_str(rest);
            return ensure_all_args_used(args, &used_args, output);
        };

        let name = &rest[..end];
        if !is_placeholder_name(name) {
            output.push('{');
            output.push_str(name);
            output.push('}');
            rest = &rest[end + 1..];
            continue;
        }

        let Some(arg_index) = args.iter().position(|(arg_name, _)| *arg_name == name) else {
            return Err(TemplateError::MissingArgument {
                name: name.to_owned(),
            });
        };
        output.push_str(args[arg_index].1);
        used_args[arg_index] = true;
        rest = &rest[end + 1..];
    }

    output.push_str(rest);
    ensure_all_args_used(args, &used_args, output)
}

fn ensure_all_args_used(
    args: &[(&str, &str)],
    used_args: &[bool],
    output: String,
) -> Result<String, TemplateError> {
    for ((name, _), used) in args.iter().zip(used_args) {
        if !used {
            return Err(TemplateError::UnusedArgument {
                name: (*name).to_owned(),
            });
        }
    }
    Ok(output)
}

fn is_placeholder_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}
