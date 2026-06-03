use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, SupportedPlatforms, SyncToCloud};

define_settings_group!(CodeSettings, settings: [
    code_as_default_editor: CodeAsDefaultEditor {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "code.editor.use_warp_as_default_editor",
        description_key: "settings.schema.code.editor.use_warp_as_default_editor.description",
    }
    codebase_context_enabled: CodebaseContextEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        storage_key: "AgentModeCodebaseContext",
        toml_path: "code.indexing.agent_mode_codebase_context",
        description_key: "settings.schema.code.indexing.agent_mode_codebase_context.description",
    },
    auto_indexing_enabled: AutoIndexingEnabled {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        storage_key: "AgentModeCodebaseContextAutoIndexing",
        toml_path: "code.indexing.agent_mode_codebase_context_auto_indexing",
        description_key: "settings.schema.code.indexing.agent_mode_codebase_context_auto_indexing.description",
    },
    // Whether or not the user has manually dismissed the code toolbelt new feature popup.
    dismissed_code_toolbelt_new_feature_popup: DismissedCodeToolbeltNewFeaturePopup {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
    // Controls whether the project explorer / file tree appears in the tools panel.
    show_project_explorer: ShowProjectExplorer {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "code.editor.show_project_explorer",
        description_key: "settings.schema.code.editor.show_project_explorer.description",
    },
    // Controls whether global file search appears in the tools panel.
    show_global_search: ShowGlobalSearch {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "code.editor.show_global_search",
        description_key: "settings.schema.code.editor.show_global_search.description",
    },
]);
