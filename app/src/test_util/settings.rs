#[cfg(test)]
use warpui::App;

#[cfg(test)]
pub fn initialize_settings_for_tests(app: &mut App) {
    use warp_core::execution_mode::ExecutionMode;
    initialize_settings_for_tests_with_mode(app, ExecutionMode::App, false);
}

#[cfg(test)]
pub fn initialize_history_persistence_for_tests(app: &mut App) {
    use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

    initialize_settings_for_tests(app);

    let global_resource_handles = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));
}

#[cfg(test)]
pub fn initialize_settings_for_tests_with_mode(
    app: &mut App,
    mode: warp_core::execution_mode::ExecutionMode,
    is_sandboxed: bool,
) {
    use crate::{
        ai::cloud_agent_settings::CloudAgentSettings,
        drive::settings::WarpDriveSettings,
        search::command_search::settings::CommandSearchSettings,
        settings::{
            app_icon::AppIconSettings, init_and_register_user_preferences,
            manager::SettingsManager, AISettings, AccessibilitySettings, AliasExpansionSettings,
            AppEditorSettings, BlockVisibilitySettings, ChangelogSettings,
            CloudPreferencesSettings, CodeSettings, DebugSettings, EmacsBindingsSettings,
            FontSettings, GPUSettings, InputModeSettings, InputSettings, NativePreferenceSettings,
            PaneSettings, SameLinePromptBlockSettings, ScrollSettings, SelectionSettings,
            SshSettings, ThemeSettings, VimBannerSettings,
        },
        terminal::{
            general_settings::GeneralSettings, keys_settings::KeysSettings,
            ligature_settings::LigatureSettings, safe_mode_settings::SafeModeSettings,
            session_settings::SessionSettings, settings::TerminalSettings,
            shared_session::settings::SharedSessionSettings, warpify::settings::WarpifySettings,
            BlockListSettings,
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
    CloudAgentSettings::register(app);
    AppEditorSettings::register(app);
    BlockVisibilitySettings::register(app);
    BlockListSettings::register(app);
    ChangelogSettings::register(app);
    CloudPreferencesSettings::register(app);
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

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
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
    WarpDriveSettings::register(app);
    WindowSettings::register(app);
    SharedSessionSettings::register(app);
    CodeSettings::register(app);
    SemanticSelection::register(app);

    app.update(|ctx| {
        // Register a no-op secure storage provider for testing.
        warpui_extras::secure_storage::register_noop("test", ctx);

        // Add settings models that are backed by secure storage, not user preferences.
        ctx.add_singleton_model(ai::api_keys::ApiKeyManager::new);
    });
}
