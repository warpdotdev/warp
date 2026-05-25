use pathfinder_color::ColorU;
use settings::Setting as _;
use settings::SettingsManager;
use warpui::fonts::Properties;
use warpui::{App, AppContext};

use crate::context_chips::{ChipAvailability, ChipDisabledReason, ContextChipKind};
use crate::settings::{init_and_register_user_preferences, AppLanguage, LanguageSettings};

use super::{Renderer, RendererStyles};

#[test]
fn test_constructor_availability_updates_disabled_state_without_tooltip_override() {
    let kind = ContextChipKind::ShellGitBranch;
    let chip = kind.to_chip().expect("chip definition should exist");
    let renderer = Renderer::new(
        kind,
        chip,
        crate::context_chips::ChipValue::Text("main".to_string()),
        renderer_styles(),
        ChipAvailability::Disabled(ChipDisabledReason::RequiresExecutable {
            command: "gh".to_string(),
        }),
    );

    assert!(renderer.is_disabled);
    assert_eq!(renderer.tooltip_override_text, None);
}

#[test]
fn test_constructor_availability_enabled_has_no_disabled_state_or_tooltip_override() {
    let kind = ContextChipKind::ShellGitBranch;
    let chip = kind.to_chip().expect("chip definition should exist");
    let renderer = Renderer::new(
        kind,
        chip,
        crate::context_chips::ChipValue::Text("main".to_string()),
        renderer_styles(),
        ChipAvailability::Enabled,
    );

    assert!(!renderer.is_disabled);
    assert_eq!(renderer.tooltip_override_text, None);
}

#[test]
fn test_constructor_for_app_localizes_disabled_tooltip_override() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            register_language_settings(ctx, AppLanguage::SimplifiedChinese);
        });

        app.read(|ctx| {
            let kind = ContextChipKind::ShellGitBranch;
            let chip = kind.to_chip().expect("chip definition should exist");
            let renderer = Renderer::new_for_app(
                kind,
                chip,
                crate::context_chips::ChipValue::Text("main".to_string()),
                renderer_styles(),
                ChipAvailability::Disabled(ChipDisabledReason::RequiresExecutable {
                    command: "gh".to_string(),
                }),
                ctx,
            );

            assert!(renderer.is_disabled);
            assert_eq!(
                renderer.tooltip_override_text.as_deref(),
                Some("需要 GitHub CLI")
            );
        });
    });
}

fn renderer_styles() -> RendererStyles {
    RendererStyles::new(
        ColorU {
            r: 255,
            g: 255,
            b: 255,
            a: 255,
        },
        Properties::default(),
    )
}

fn register_language_settings(ctx: &mut AppContext, language: AppLanguage) {
    init_and_register_user_preferences(ctx);
    ctx.add_singleton_model(|_| SettingsManager::default());
    let language_settings = LanguageSettings::register(ctx);
    language_settings.update(ctx, |settings, ctx| {
        settings
            .app_language
            .set_value(language, ctx)
            .expect("test app language should be configurable");
    });
}
