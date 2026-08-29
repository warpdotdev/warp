pub mod new_session_shell;
pub mod startup_shell;
pub mod working_directory_config;

pub use new_session_shell::*;
use settings::macros::define_settings_group;
use settings::{SupportedPlatforms, SyncToCloud};
pub use startup_shell::*;
pub use working_directory_config::*;

define_settings_group!(ShellSettings, settings: [
    working_directory_config: WorkingDirectoryConfig,
    startup_shell_override: StartupShellOverride {
        type: StartupShell,
        default: StartupShell::default(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "session.startup_shell_override",
        description: "The shell to use when Warp starts up.",
    },
    new_session_shell_override: NewSessionShellOverride {
        type: Option<NewSessionShell>,
        default: None,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "session.new_session_shell_override",
        description: "The shell to use when opening a new session.",
    }
]);

settings::macros::implement_setting_for_enum!(
    WorkingDirectoryConfig,
    ShellSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Never,
    surface: settings::SettingSurfaces::GUI,
    private: false,
    toml_path: "session.working_directory_config",
    max_table_depth: 1,
    description: "Controls the working directory used when opening new sessions.",
);
