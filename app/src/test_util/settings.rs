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
    use warp_core::execution_mode::AppExecutionMode;
    use warp_core::semantic_selection::SemanticSelection;

    use crate::ai::cloud_agent_settings::CloudAgentSettings;
    use crate::drive::settings::WarpDriveSettings;
    use crate::search::command_search::settings::CommandSearchSettings;
    use crate::settings::app_icon::AppIconSettings;
    use crate::settings::manager::SettingsManager;
    use crate::settings::{
        init_and_register_user_preferences, AISettings, AccessibilitySettings,
        AliasExpansionSettings, AppEditorSettings, BlockVisibilitySettings, ChangelogSettings,
        CloudPreferencesSettings, CodeSettings, DebugSettings, EmacsBindingsSettings, FontSettings,
        GPUSettings, InputModeSettings, InputSettings, LocalControlSettings,
        NativePreferenceSettings, PaneSettings, SameLinePromptBlockSettings, ScrollSettings,
        SelectionSettings, SshSettings, ThemeSettings, VimBannerSettings,
    };
    use crate::terminal::general_settings::GeneralSettings;
    use crate::terminal::keys_settings::KeysSettings;
    use crate::terminal::ligature_settings::LigatureSettings;
    use crate::terminal::safe_mode_settings::SafeModeSettings;
    use crate::terminal::session_settings::SessionSettings;
    use crate::terminal::settings::TerminalSettings;
    use crate::terminal::shared_session::settings::SharedSessionSettings;
    use crate::terminal::warpify::settings::WarpifySettings;
    use crate::terminal::BlockListSettings;
    use crate::undo_close::UndoCloseSettings;
    use crate::user_config::WarpConfig;
    use crate::window_settings::WindowSettings;
    use crate::workspace::tab_settings::TabSettings;
    app.add_singleton_model(|ctx| AppExecutionMode::new(mode, is_sandboxed, ctx));

    app.update(init_and_register_user_preferences);
    app.add_singleton_model(|_ctx| SettingsManager::default());
    app.add_singleton_model(WarpConfig::mock);
    app.update(|ctx| {
        // Register a no-op secure storage provider for testing.
        warpui_extras::secure_storage::register_noop("test", ctx);
    });

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
    if warp_core::features::FeatureFlag::WarpControlCli.is_enabled() {
        LocalControlSettings::register(app);
    }

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
        // Add settings models that are backed by secure storage, not user preferences.
        ctx.add_singleton_model(ai::api_keys::ApiKeyManager::new);
    });
}
