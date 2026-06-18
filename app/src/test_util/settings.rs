#[cfg(test)]
use warpui::App;

#[cfg(test)]
pub struct LocalAIEnabledTestGuard;

#[cfg(test)]
impl Drop for LocalAIEnabledTestGuard {
    fn drop(&mut self) {
        warp_core::channel::ChannelState::set(warp_core::channel::ChannelState::init());
    }
}

#[cfg(test)]
pub fn initialize_local_ai_enabled_for_tests(app: &mut App) -> LocalAIEnabledTestGuard {
    use ai::api_keys::ApiKeyManager;
    use warp_core::{
        channel::{Channel, ChannelConfig, ChannelState},
        AppId,
    };
    use warpui::SingletonEntity as _;

    ChannelState::set(ChannelState::new(
        Channel::Oss,
        ChannelConfig {
            app_id: AppId::new("dev", "warper", "Warper"),
            logfile_name: "warper.log".into(),
            mcp_static_config: None,
        },
    ));

    app.update(|ctx| {
        if !ctx.has_singleton_model::<warpui_extras::secure_storage::Model>() {
            warpui_extras::secure_storage::register_noop("test", ctx);
        }
        if !ctx.has_singleton_model::<crate::settings::AISettings>() {
            crate::settings::AISettings::register_and_subscribe_to_events(ctx);
        }
        if !ctx.has_singleton_model::<ApiKeyManager>() {
            ctx.add_singleton_model(ApiKeyManager::new);
        }
        ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.set_open_router_key(Some("sk-or-v1-test".to_owned()), ctx);
        });
        assert!(crate::settings::AISettings::as_ref(ctx).is_any_ai_enabled(ctx));
    });

    LocalAIEnabledTestGuard
}

#[cfg(test)]
pub fn initialize_settings_for_tests(app: &mut App) {
    use warp_core::execution_mode::ExecutionMode;
    initialize_settings_for_tests_with_mode(app, ExecutionMode::App, false);
}

#[cfg(test)]
pub fn initialize_settings_for_tests_with_mode(
    app: &mut App,
    mode: warp_core::execution_mode::ExecutionMode,
    is_sandboxed: bool,
) {
    use crate::{
        search::command_search::settings::CommandSearchSettings,
        settings::{
            app_icon::AppIconSettings, init_and_register_user_preferences,
            manager::SettingsManager, AISettings, AccessibilitySettings, AliasExpansionSettings,
            AppEditorSettings, BlockVisibilitySettings, ChangelogSettings, CodeSettings,
            DebugSettings, EmacsBindingsSettings, FontSettings, GPUSettings, InputModeSettings,
            InputSettings, LocalPreferencesSettings, NativePreferenceSettings, PaneSettings,
            SameLinePromptBlockSettings, ScrollSettings, SelectionSettings, SshSettings,
            ThemeSettings, VimBannerSettings,
        },
        terminal::{
            general_settings::GeneralSettings, keys_settings::KeysSettings,
            ligature_settings::LigatureSettings, safe_mode_settings::SafeModeSettings,
            session_settings::SessionSettings, settings::TerminalSettings,
            warpify::settings::WarpifySettings, BlockListSettings,
        },
        undo_close::UndoCloseSettings,
        user_config::WarpConfig,
        window_settings::WindowSettings,
        workspace::tab_settings::TabSettings,
    };
    use warp_core::{execution_mode::AppExecutionMode, semantic_selection::SemanticSelection};
    app.add_singleton_model(|ctx| AppExecutionMode::new(mode, is_sandboxed, ctx));

    app.update(init_and_register_user_preferences);
    app.add_singleton_model(|_ctx| SettingsManager::default());
    app.add_singleton_model(WarpConfig::mock);

    AccessibilitySettings::register(app);
    app.update(AISettings::register_and_subscribe_to_events);
    AliasExpansionSettings::register(app);
    AppEditorSettings::register(app);
    BlockVisibilitySettings::register(app);
    BlockListSettings::register(app);
    ChangelogSettings::register(app);
    LocalPreferencesSettings::register(app);
    CommandSearchSettings::register(app);
    DebugSettings::register(app);
    AppIconSettings::register(app);
    EmacsBindingsSettings::register(app);

    #[cfg(feature = "local_fs")]
    {
        crate::util::file::external_editor::EditorSettings::register(app);
    }

    FontSettings::register(app);
    GeneralSettings::register(app);
    GPUSettings::register(app);
    InputModeSettings::register(app);
    InputSettings::register(app);
    KeysSettings::register(app);
    LigatureSettings::register(app);

    #[cfg(target_os = "linux")]
    {
        use crate::settings::LinuxAppConfiguration;
        LinuxAppConfiguration::register(app);
    }

    NativePreferenceSettings::register(app);
    SafeModeSettings::register(app);
    SameLinePromptBlockSettings::register(app);
    ScrollSettings::register(app);
    SelectionSettings::register(app);
    app.update(|ctx| {
        WarpifySettings::register(ctx);
    });
    SessionSettings::register(app);
    SshSettings::register(app);
    TabSettings::register(app);
    TerminalSettings::register(app);
    PaneSettings::register(app);
    ThemeSettings::register(app);
    UndoCloseSettings::register(app);
    VimBannerSettings::register(app);
    WindowSettings::register(app);
    CodeSettings::register(app);
    SemanticSelection::register(app);

    app.update(|ctx| {
        // Register a no-op secure storage provider for testing.
        warpui_extras::secure_storage::register_noop("test", ctx);

        // Add settings models that are backed by secure storage, not user preferences.
        ctx.add_singleton_model(ai::api_keys::ApiKeyManager::new);
    });
}
