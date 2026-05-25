use std::sync::LazyLock;

use anyhow::Context as _;
use parking_lot::RwLock;
use warp_localization::{
    replace_placeholders, AppLanguage, Catalog, CatalogBundle, LocaleId, TranslationSource,
};
use warpui::{AppContext, AssetProvider as _, Entity, ModelContext, SingletonEntity as _};

use crate::{
    settings::{LanguageSettings, LanguageSettingsChangedEvent},
    ASSETS,
};

pub(crate) enum LocalizationEvent {
    LocaleChanged,
}

static CATALOGS: LazyLock<CatalogBundle> = LazyLock::new(|| {
    let catalogs = [LocaleId::EnUs, LocaleId::ZhCn]
        .into_iter()
        .map(load_catalog)
        .collect::<anyhow::Result<Vec<_>>>()
        .expect("bundled localization catalogs must be valid");

    CatalogBundle::new(LocaleId::EnUs, catalogs)
        .expect("default localization catalog must be bundled")
});

static SYSTEM_LOCALE_CANDIDATES: LazyLock<RwLock<Vec<String>>> =
    LazyLock::new(|| RwLock::new(system_locale_candidates()));

pub(crate) fn current_locale(app: &AppContext) -> LocaleId {
    match *LanguageSettings::as_ref(app).app_language {
        AppLanguage::System => {
            let candidates = SYSTEM_LOCALE_CANDIDATES.read();
            AppLanguage::System
                .effective_locale_from_candidates(candidates.iter().map(String::as_str))
        }
        AppLanguage::English => LocaleId::EnUs,
        AppLanguage::SimplifiedChinese => LocaleId::ZhCn,
    }
}

pub(crate) fn register_localization_updater(ctx: &mut AppContext) {
    ctx.add_singleton_model(LocalizationUpdater::new);
}

pub(crate) fn refresh_system_locale_candidates_if_needed(app: &AppContext) -> bool {
    if *LanguageSettings::as_ref(app).app_language == AppLanguage::System {
        refresh_system_locale_candidates()
    } else {
        false
    }
}

pub(crate) fn notify_locale_changed(ctx: &mut AppContext) {
    LocalizationUpdater::handle(ctx).update(ctx, |_, ctx| notify_locale_changed_from_model(ctx));
}

pub(crate) fn text_for_app(app: &AppContext, key: &str) -> String {
    text(current_locale(app), key)
}

pub(crate) fn text_for_app_with_args(app: &AppContext, key: &str, args: &[(&str, &str)]) -> String {
    replace_placeholders(&text_for_app(app, key), args)
        .expect("localized text template arguments must match the catalog")
}

pub(crate) fn text_for_app_or(app: &AppContext, key: &str, fallback: &str) -> String {
    let lookup = CATALOGS.lookup(current_locale(app), key);
    if lookup.source == TranslationSource::Key {
        fallback.to_owned()
    } else {
        lookup.text.into_owned()
    }
}

pub(crate) fn language_option_label(locale: LocaleId, language: AppLanguage) -> String {
    text(locale, language.translation_key())
}

pub(crate) fn text_for_locale(locale: LocaleId, key: &str) -> String {
    text(locale, key)
}

#[cfg(test)]
pub(crate) fn text_for_locale_with_args(
    locale: LocaleId,
    key: &str,
    args: &[(&str, &str)],
) -> String {
    replace_placeholders(&text_for_locale(locale, key), args)
        .expect("localized text template arguments must match the catalog")
}

fn text(locale: LocaleId, key: &str) -> String {
    CATALOGS.text(locale, key).into_owned()
}

fn load_catalog(locale: LocaleId) -> anyhow::Result<Catalog> {
    let path = format!("bundled/locales/{}.json", locale.code());
    let bytes = ASSETS
        .get(&path)
        .with_context(|| format!("failed to load {path}"))?;
    let source = std::str::from_utf8(&bytes)
        .with_context(|| format!("localization catalog {path} is not UTF-8"))?;

    Catalog::from_json(locale, source).with_context(|| format!("invalid {path}"))
}

fn system_locale_candidates() -> Vec<String> {
    platform_locale_candidates()
        .into_iter()
        .chain(environment_locale_candidates())
        .collect()
}

fn refresh_system_locale_candidates() -> bool {
    replace_system_locale_candidates(&SYSTEM_LOCALE_CANDIDATES, system_locale_candidates())
}

fn replace_system_locale_candidates(
    cache: &RwLock<Vec<String>>,
    new_candidates: Vec<String>,
) -> bool {
    let mut cached_candidates = cache.write();
    if *cached_candidates == new_candidates {
        false
    } else {
        *cached_candidates = new_candidates;
        true
    }
}

fn environment_locale_candidates() -> impl Iterator<Item = String> {
    environment_locale_candidates_from(|key| std::env::var(key).ok()).into_iter()
}

fn environment_locale_candidates_from(mut get: impl FnMut(&str) -> Option<String>) -> Vec<String> {
    ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .flat_map(|key| {
            get(key).into_iter().flat_map(move |value| {
                let values = if key == "LANGUAGE" {
                    value.split(':').map(str::to_owned).collect::<Vec<_>>()
                } else {
                    vec![value]
                };

                values
                    .into_iter()
                    .map(|candidate| candidate.trim().to_owned())
                    .filter(|candidate| !candidate.is_empty())
            })
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn platform_locale_candidates() -> Vec<String> {
    use objc::{class, msg_send, rc::autoreleasepool, runtime::Object, sel, sel_impl};
    use warpui::platform::mac::utils::nsstring_as_str;

    autoreleasepool(|| unsafe {
        let locale_class = class!(NSLocale);
        let languages: *const Object = msg_send![locale_class, preferredLanguages];
        if languages.is_null() {
            return Vec::new();
        }

        let count: usize = msg_send![languages, count];
        (0..count)
            .filter_map(|index| {
                let language: *const Object = msg_send![languages, objectAtIndex: index];
                if language.is_null() {
                    return None;
                }
                nsstring_as_str(language).map(str::to_owned).ok()
            })
            .collect()
    })
}

#[cfg(target_os = "windows")]
fn platform_locale_candidates() -> Vec<String> {
    use windows::core::PWSTR;
    use windows::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};

    let mut language_count = 0;
    let mut buffer_len = 0;
    if unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut language_count,
            None,
            &mut buffer_len,
        )
    }
    .is_err()
        || buffer_len == 0
    {
        return Vec::new();
    }

    let mut buffer = vec![0u16; buffer_len as usize];
    if unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut language_count,
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut buffer_len,
        )
    }
    .is_err()
    {
        return Vec::new();
    }

    buffer
        .split(|value| *value == 0)
        .filter(|language| !language.is_empty())
        .filter_map(|language| String::from_utf16(language).ok())
        .collect()
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn platform_locale_candidates() -> Vec<String> {
    Vec::new()
}

pub(crate) struct LocalizationUpdater;

impl LocalizationUpdater {
    fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&LanguageSettings::handle(ctx), |_, event, ctx| {
            let LanguageSettingsChangedEvent::AppLanguageSetting { .. } = event;
            let _ = refresh_system_locale_candidates_if_needed(ctx);
            notify_locale_changed_from_model(ctx);
        });

        Self
    }
}

impl Entity for LocalizationUpdater {
    type Event = LocalizationEvent;
}

impl warpui::SingletonEntity for LocalizationUpdater {}

fn notify_locale_changed_from_model(ctx: &mut ModelContext<LocalizationUpdater>) {
    ctx.emit(LocalizationEvent::LocaleChanged);
    ctx.invalidate_all_views();

    #[cfg(all(target_os = "macos", not(test)))]
    warpui::platform::mac::rebuild_native_menus(ctx);
}

#[cfg(test)]
#[path = "localization_tests.rs"]
mod tests;
