#[cfg(not(target_family = "wasm"))]
use crate::ai::aws_credentials::refresh_aws_credentials;
use crate::ai::blocklist::agent_view::agent_input_footer::editor::{
    AgentToolbarEditorMode, AgentToolbarInlineEditor,
};
use crate::ai::blocklist::BlocklistAIPermissions;
use crate::ai::execution_profiles::model_menu_items::available_model_menu_items;
use crate::ai::execution_profiles::profiles::{
    AIExecutionProfilesModel, AIExecutionProfilesModelEvent, ClientProfileId,
};
use crate::ai::execution_profiles::{AIExecutionProfile, ActionPermission, WriteToPtyPermission};
use crate::ai::llms::{LLMContextWindow, LLMId, LLMPreferences, LLMPreferencesEvent};
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::ai::paths::host_native_absolute_path;
use crate::auth::auth_manager::{AuthManager, LoginGatedFeature};
use crate::auth::auth_view_modal::AuthViewVariant;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::{CloudModel, CloudModelEvent};
use crate::cloud_object::GenericStringObjectFormat::Json;
use crate::cloud_object::JsonObjectType;
use crate::cloud_object::ObjectType;

use crate::editor::{EditorOptions, InteractionState, SingleLineEditorOptions, TextColors};
use crate::i18n::{self, I18nKey};
use crate::modal::{Modal, ModalEvent, ModalViewState};
use crate::settings::InputSettings;
use crate::settings::{
    AIAutoDetectionEnabled, AICommandDenylist, AISettingsChangedEvent,
    AgentModeCodingPermissionsType, AgentModeCommandExecutionDenylist,
    AgentModeCommandExecutionPredicate, AgentModeQuerySuggestionsEnabled, AwsBedrockAutoLogin,
    AwsBedrockCredentialsEnabled, CanUseWarpCreditsForFallback, CodeSettings,
    CodebaseContextEnabled, FeedbackBundledSkillEnabled, FileBasedMcpEnabled,
    GitOperationsAutogenEnabled, IncludeAgentCommandsInHistory, IntelligentAutosuggestionsEnabled,
    MemoryEnabled, NLDInTerminalEnabled, NaturalLanguageAutosuggestionsEnabled,
    RuleSuggestionsEnabled, SharedBlockTitleGenerationEnabled, ShouldRenderCLIAgentToolbar,
    ShouldRenderUseAgentToolbarForUserCommands, ShouldShowOzUpdatesInZeroState, ShowAgentTips,
    ShowConversationHistory, ShowHintText, ThinkingDisplayMode, VoiceInputEnabled,
    WarpDriveContextEnabled,
};
use crate::terminal::session_settings::{SessionSettings, SessionSettingsChangedEvent};
use crate::terminal::CLIAgent;
use crate::view_components::{
    action_button::{ActionButton, ButtonSize, SecondaryTheme},
    FilterableDropdown, SubmittableTextInput, SubmittableTextInputEvent,
};
use crate::workspaces::user_workspaces::UserWorkspacesEvent;
use ::ai::api_keys::{ApiKeyManager, ApiKeys};
use enum_iterator::all;
use itertools::Itertools;
use pathfinder_geometry::vector::vec2f;
use regex::Regex;
use settings::{Setting, ToggleableSetting};
use strum::IntoEnumIterator;
use warp_core::channel::ChannelState;
use warp_core::context_flag::ContextFlag;
use warp_core::features::FeatureFlag;
use warp_core::ui::color::contrast::MinimumAllowedContrast;
use warp_core::ui::color::ContrastingColor;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill as ThemeFill;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ConstrainedBox, CornerRadius, CrossAxisAlignment, Dismiss,
    Empty, Expanded, Fill, Hoverable, HyperlinkLens, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, OffsetPositioning, ParentAnchor, ParentOffsetBounds, Radius, Shrinkable,
    Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::id;
use warpui::keymap::{ContextPredicate, Keystroke};
use warpui::platform::Cursor;
use warpui::ui_components::slider::SliderStateHandle;
use warpui::{
    elements::{
        Container, Flex, FormattedTextElement, HighlightedHyperlink, HyperlinkUrl, ParentElement,
    },
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
        switch::{SwitchStateHandle, TooltipConfig},
    },
    Action, AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::custom_inference_modal::{
    CustomEndpointModal, CustomEndpointModalEvent, CustomEndpointModalViewState,
};
use super::execution_profile_view::{ExecutionProfileView, ExecutionProfileViewEvent};
use super::remove_custom_endpoint_confirmation_dialog::{
    RemoveCustomEndpointConfirmationDialog, RemoveCustomEndpointConfirmationDialogEvent,
};
use super::settings_page::{render_custom_size_header, render_settings_info_banner};
use super::{
    flags,
    settings_page::{
        build_sub_header, build_toggle_element, render_body_item_label,
        render_body_item_label_with_icon, render_dropdown_item, render_dropdown_item_label,
        render_full_pane_width_ai_button, render_input_list, render_separator, InputListItem,
        LocalOnlyIconState, MatchData, PageType, SettingsPageMeta, SettingsPageViewHandle,
        SettingsWidget, ToggleState, HEADER_PADDING, TOGGLE_BUTTON_RIGHT_PADDING,
    },
    SettingActionPairContexts, SettingActionPairDescriptions, SettingsAction, SettingsSection,
    ToggleSettingActionPair,
};

/// Identifies which subpage of the AI settings the user is viewing.
/// When `None`, the page shows all widgets (legacy/full view).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AISubpage {
    /// The main "WarpAgent" page: global AI toggle + Active AI + Input + Other sections.
    WarpAgent,
    /// Agent profiles and permissions.
    Profiles,
    /// Knowledge / Rules settings.
    Knowledge,
    /// Third-party CLI agent settings.
    ThirdPartyCLIAgents,
}

impl AISubpage {
    pub fn from_section(section: SettingsSection) -> Option<Self> {
        match section {
            SettingsSection::WarpAgent => Some(Self::WarpAgent),
            SettingsSection::AgentProfiles => Some(Self::Profiles),
            SettingsSection::Knowledge => Some(Self::Knowledge),
            SettingsSection::ThirdPartyCLIAgents => Some(Self::ThirdPartyCLIAgents),
            // AgentMCPServers renders the standalone MCPServers page, not an AI subpage.
            _ => None,
        }
    }
}
use crate::ai::{AIRequestUsageModel, AIRequestUsageModelEvent};
use crate::menu::{MenuItem, MenuItemFields};
use crate::server::telemetry::{
    AgentModeAutoDetectionSettingOrigin, AutonomySettingToggleSource,
    ToggleCodeSuggestionsSettingSource,
};
use crate::ui_components::icons::Icon;
use crate::view_components::dropdown::DropdownAction;
use crate::workspaces::workspace::{AdminEnablementSetting, CustomerType};
use crate::{
    appearance::Appearance,
    editor::Event as EditorEvent,
    editor::{EditorView, TextOptions},
    settings::{AISettings, LanguageSettings, VoiceInputToggleKey},
    ui_components::blended_colors,
    util::bindings,
    view_components::{Dropdown, DropdownItem},
};
use crate::{report_error, report_if_error, send_telemetry_from_ctx};
use crate::{TelemetryEvent, UserWorkspaces};
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Not;
use std::path::{Path, PathBuf};

const CONTENT_FONT_SIZE: f32 = 12.;
const PRIMARY_HEADER_FONT_SIZE: f32 = 24.;

const AI_SETTINGS_DROPDOWN_WIDTH: f32 = 250.;
const AI_SETTINGS_DROPDOWN_MAX_HEIGHT: f32 = 250.;
const CONTEXT_WINDOW_SLIDER_WIDTH: f32 = 220.;
const CONTEXT_WINDOW_INPUT_BOX_WIDTH: f32 = 120.;

const WISPR_FLOW_URL: &str = "https://wisprflow.ai/";
const CUSTOM_INFERENCE_LEARN_MORE_URL: &str =
    "https://docs.warp.dev/support-and-community/plans-and-billing/bring-your-own-api-key/";
const CUSTOM_INFERENCE_TERMS_URL: &str = "https://www.warp.dev/legal/terms-of-service";
const CUSTOM_INFERENCE_INFO_TOOLTIP_MAX_WIDTH: f32 = 320.;

fn action_permission_label(app: &AppContext, permission: ActionPermission) -> &'static str {
    match permission {
        ActionPermission::AgentDecides => i18n::tr(app, I18nKey::AiAgentDecides),
        ActionPermission::AlwaysAllow => i18n::tr(app, I18nKey::AiAlwaysAllow),
        ActionPermission::AlwaysAsk => i18n::tr(app, I18nKey::AiAlwaysAsk),
        ActionPermission::Unknown => i18n::tr(app, I18nKey::AiUnknown),
    }
}

fn action_permission_dropdown_items(
    app: &AppContext,
    action: fn(ActionPermission) -> AISettingsPageAction,
) -> Vec<DropdownItem<AISettingsPageAction>> {
    [
        ActionPermission::AgentDecides,
        ActionPermission::AlwaysAllow,
        ActionPermission::AlwaysAsk,
    ]
    .into_iter()
    .map(|permission| {
        DropdownItem::new(action_permission_label(app, permission), action(permission))
    })
    .collect()
}

fn write_to_pty_permission_label(
    app: &AppContext,
    permission: WriteToPtyPermission,
) -> &'static str {
    match permission {
        WriteToPtyPermission::AlwaysAllow => i18n::tr(app, I18nKey::AiAlwaysAllow),
        WriteToPtyPermission::AlwaysAsk => i18n::tr(app, I18nKey::AiAlwaysAsk),
        WriteToPtyPermission::AskOnFirstWrite => i18n::tr(app, I18nKey::AiAskOnFirstWrite),
        WriteToPtyPermission::Unknown => i18n::tr(app, I18nKey::AiUnknown),
    }
}

fn write_to_pty_dropdown_items(app: &AppContext) -> Vec<DropdownItem<AISettingsPageAction>> {
    [
        WriteToPtyPermission::AlwaysAllow,
        WriteToPtyPermission::AlwaysAsk,
        WriteToPtyPermission::AskOnFirstWrite,
    ]
    .into_iter()
    .map(|permission| {
        DropdownItem::new(
            write_to_pty_permission_label(app, permission),
            AISettingsPageAction::SetWriteToPty(permission),
        )
    })
    .collect()
}

fn action_permission_description(app: &AppContext, permission: ActionPermission) -> &'static str {
    match permission {
        ActionPermission::AgentDecides | ActionPermission::Unknown => {
            i18n::tr(app, I18nKey::AiPermissionAgentDecidesDescription)
        }
        ActionPermission::AlwaysAllow => i18n::tr(app, I18nKey::AiPermissionAlwaysAllowDescription),
        ActionPermission::AlwaysAsk => i18n::tr(app, I18nKey::AiPermissionAlwaysAskDescription),
    }
}

fn write_to_pty_permission_description(
    app: &AppContext,
    permission: WriteToPtyPermission,
) -> &'static str {
    match permission {
        WriteToPtyPermission::AlwaysAllow => {
            i18n::tr(app, I18nKey::AiPermissionAlwaysAllowDescription)
        }
        WriteToPtyPermission::AskOnFirstWrite => {
            i18n::tr(app, I18nKey::AiWriteToPtyAskOnFirstWriteDescription)
        }
        WriteToPtyPermission::AlwaysAsk => i18n::tr(app, I18nKey::AiWriteToPtyAlwaysAskDescription),
        WriteToPtyPermission::Unknown => {
            i18n::tr(app, I18nKey::AiPermissionAgentDecidesDescription)
        }
    }
}

fn thinking_display_mode_label(app: &AppContext, mode: ThinkingDisplayMode) -> &'static str {
    match mode {
        ThinkingDisplayMode::ShowAndCollapse => i18n::tr(app, I18nKey::AiShowAndCollapse),
        ThinkingDisplayMode::AlwaysShow => i18n::tr(app, I18nKey::AiAlwaysShow),
        ThinkingDisplayMode::NeverShow => i18n::tr(app, I18nKey::AiNeverShow),
    }
}

fn thinking_display_mode_dropdown_items(
    app: &AppContext,
) -> Vec<DropdownItem<AISettingsPageAction>> {
    ThinkingDisplayMode::iter()
        .map(|mode| {
            DropdownItem::new(
                thinking_display_mode_label(app, mode),
                AISettingsPageAction::SetThinkingDisplayMode(mode),
            )
        })
        .collect()
}

fn voice_input_toggle_key_label(app: &AppContext, key: VoiceInputToggleKey) -> String {
    let (super_key_name, alt_key_name): (&'static str, &'static str) =
        match warpui::platform::OperatingSystem::get() {
            warpui::platform::OperatingSystem::Mac => ("Command", "Option"),
            warpui::platform::OperatingSystem::Windows => ("Windows", "Alt"),
            warpui::platform::OperatingSystem::Linux
            | warpui::platform::OperatingSystem::Other(_) => ("Super", "Alt"),
        };
    let left = i18n::tr(app, I18nKey::AiLeft);
    let right = i18n::tr(app, I18nKey::AiRight);

    match key {
        VoiceInputToggleKey::None => i18n::tr(app, I18nKey::AiNone).to_string(),
        VoiceInputToggleKey::Fn => "Fn".to_string(),
        VoiceInputToggleKey::AltLeft => format!("{alt_key_name} ({left})"),
        VoiceInputToggleKey::AltRight => format!("{alt_key_name} ({right})"),
        VoiceInputToggleKey::ControlLeft => format!("Control ({left})"),
        VoiceInputToggleKey::ControlRight => format!("Control ({right})"),
        VoiceInputToggleKey::SuperLeft => format!("{super_key_name} ({left})"),
        VoiceInputToggleKey::SuperRight => format!("{super_key_name} ({right})"),
        VoiceInputToggleKey::ShiftLeft => format!("Shift ({left})"),
        VoiceInputToggleKey::ShiftRight => format!("Shift ({right})"),
    }
}

fn voice_input_toggle_key_dropdown_items(
    app: &AppContext,
) -> Vec<DropdownItem<AISettingsPageAction>> {
    VoiceInputToggleKey::all_possible_values()
        .into_iter()
        .map(|key| {
            DropdownItem::new(
                voice_input_toggle_key_label(app, key),
                AISettingsPageAction::SetVoiceInputToggleKey(key),
            )
        })
        .collect()
}

pub fn init_actions_from_parent_view<T: Action + Clone>(
    app: &mut AppContext,
    context: &ContextPredicate,
    builder: fn(SettingsAction) -> T,
) {
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![ToggleSettingActionPair::new(
            "AI",
            builder(SettingsAction::AI(AISettingsPageAction::ToggleGlobalAI)),
            context,
            flags::IS_ANY_AI_ENABLED,
        )
        .with_group(bindings::BindingGroup::WarpAi)],
        app,
    );

    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![ToggleSettingActionPair::new(
            "Active AI",
            builder(SettingsAction::AI(AISettingsPageAction::ToggleActiveAI)),
            &(context.clone() & id!(flags::IS_ANY_AI_ENABLED)),
            flags::IS_ACTIVE_AI_ENABLED,
        )
        .with_group(bindings::BindingGroup::WarpAi)],
        app,
    );

    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![ToggleSettingActionPair::new(
            if FeatureFlag::AgentView.is_enabled() {
                "terminal command autodetection in agent input"
            } else {
                "natural language detection"
            },
            builder(SettingsAction::AI(
                AISettingsPageAction::ToggleAIInputAutoDetection,
            )),
            &(context.clone() & id!(flags::IS_ANY_AI_ENABLED)),
            flags::AI_INPUT_AUTODETECTION_FLAG,
        )
        .with_group(bindings::BindingGroup::WarpAi)
        .with_enabled(|| FeatureFlag::AgentMode.is_enabled())],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![ToggleSettingActionPair::new(
            "agent prompt autodetection in terminal input",
            builder(SettingsAction::AI(
                AISettingsPageAction::ToggleNLDInTerminal,
            )),
            &(context.clone() & id!(flags::IS_ANY_AI_ENABLED)),
            flags::NLD_IN_TERMINAL_FLAG,
        )
        .with_group(bindings::BindingGroup::WarpAi)
        .with_enabled(|| FeatureFlag::AgentView.is_enabled())],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![ToggleSettingActionPair::new(
            "Next Command",
            builder(SettingsAction::AI(
                AISettingsPageAction::ToggleIntelligentAutosuggestions,
            )),
            &(context.clone() & id!(flags::IS_ACTIVE_AI_ENABLED)),
            flags::INTELLIGENT_AUTOSUGGESTIONS_FLAG,
        )
        .with_group(bindings::BindingGroup::WarpAi)],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![ToggleSettingActionPair::new(
            "prompt suggestions",
            builder(SettingsAction::AI(
                AISettingsPageAction::TogglePromptSuggestions,
            )),
            &(context.clone() & id!(flags::IS_ACTIVE_AI_ENABLED)),
            flags::PROMPT_SUGGESTIONS_FLAG,
        )
        .with_group(bindings::BindingGroup::WarpAi)],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![ToggleSettingActionPair::new(
            "code suggestions",
            builder(SettingsAction::AI(
                AISettingsPageAction::ToggleCodeSuggestions,
            )),
            &(context.clone()
                & id!(flags::IS_ACTIVE_AI_ENABLED)
                & id!(flags::PROMPT_SUGGESTIONS_FLAG)),
            flags::CODE_SUGGESTIONS_FLAG,
        )
        .with_group(bindings::BindingGroup::WarpAi)],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![ToggleSettingActionPair::custom(
            SettingActionPairDescriptions::new("Show agent tips", "Hide agent tips"),
            builder(SettingsAction::AI(
                AISettingsPageAction::ToggleShowAgentTips,
            )),
            SettingActionPairContexts::new(
                context.clone() & id!(flags::IS_ANY_AI_ENABLED) & !id!(flags::SHOW_AGENT_TIPS_FLAG),
                context.clone() & id!(flags::IS_ANY_AI_ENABLED) & id!(flags::SHOW_AGENT_TIPS_FLAG),
            ),
            None,
        )
        .with_group(bindings::BindingGroup::WarpAi)
        .with_enabled(|| FeatureFlag::AgentTips.is_enabled())],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![ToggleSettingActionPair::custom(
            SettingActionPairDescriptions::new(
                "Show Oz changelog in new agent conversation view",
                "Hide Oz changelog in new agent conversation view",
            ),
            builder(SettingsAction::AI(
                AISettingsPageAction::ToggleShowOzUpdatesInZeroState,
            )),
            SettingActionPairContexts::new(
                context.clone()
                    & id!(flags::IS_ANY_AI_ENABLED)
                    & !id!(flags::SHOW_OZ_UPDATES_IN_ZERO_STATE_FLAG),
                context.clone()
                    & id!(flags::IS_ANY_AI_ENABLED)
                    & id!(flags::SHOW_OZ_UPDATES_IN_ZERO_STATE_FLAG),
            ),
            None,
        )
        .with_group(bindings::BindingGroup::WarpAi)
        .with_enabled(|| FeatureFlag::AgentView.is_enabled())],
        app,
    );
    {
        use crate::settings::ThinkingDisplayMode;
        use warpui::keymap::FixedBinding;

        let ai_context = context.clone() & id!(flags::IS_ANY_AI_ENABLED);
        let mode_bindings: Vec<FixedBinding> = ThinkingDisplayMode::iter()
            .map(|mode| {
                let context_flag = match mode {
                    ThinkingDisplayMode::ShowAndCollapse => {
                        flags::THINKING_DISPLAY_SHOW_AND_COLLAPSE
                    }
                    ThinkingDisplayMode::AlwaysShow => flags::THINKING_DISPLAY_ALWAYS_SHOW,
                    ThinkingDisplayMode::NeverShow => flags::THINKING_DISPLAY_NEVER_SHOW,
                };
                FixedBinding::empty(
                    mode.command_palette_description(),
                    builder(SettingsAction::AI(
                        AISettingsPageAction::SetThinkingDisplayMode(mode),
                    )),
                    ai_context.clone() & !id!(context_flag),
                )
                .with_group(bindings::BindingGroup::WarpAi.as_str())
            })
            .collect();
        app.register_fixed_bindings(mode_bindings);
    }
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![ToggleSettingActionPair::new(
            "natural language autosuggestions",
            builder(SettingsAction::AI(
                AISettingsPageAction::ToggleNaturalLanguageAutosuggestions,
            )),
            &(context.clone() & id!(flags::IS_ACTIVE_AI_ENABLED)),
            flags::NATURAL_LANGUAGE_AUTOSUGGESTIONS_FLAG,
        )
        .with_group(bindings::BindingGroup::WarpAi)
        .with_enabled(|| FeatureFlag::PredictAMQueries.is_enabled())],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![ToggleSettingActionPair::new(
            "shared block title generation",
            builder(SettingsAction::AI(
                AISettingsPageAction::ToggleSharedTitleGeneration,
            )),
            &(context.clone() & id!(flags::IS_ACTIVE_AI_ENABLED)),
            flags::SHARED_BLOCK_TITLE_GENERATION_FLAG,
        )
        .with_group(bindings::BindingGroup::WarpAi)
        .with_enabled(|| FeatureFlag::SharedBlockTitleGeneration.is_enabled())],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![ToggleSettingActionPair::new(
            "voice input",
            builder(SettingsAction::AI(AISettingsPageAction::ToggleVoiceInput)),
            &(context.clone() & id!(flags::IS_ANY_AI_ENABLED)),
            flags::IS_VOICE_INPUT_ENABLED,
        )
        .with_group(bindings::BindingGroup::WarpAi)
        .with_enabled(|| cfg!(feature = "voice_input"))],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![ToggleSettingActionPair::custom(
            SettingActionPairDescriptions::new(
                "Show \"Use Agent\" footer",
                "Hide \"Use Agent\" footer",
            ),
            builder(SettingsAction::AI(
                AISettingsPageAction::ToggleUseAgentToolbar,
            )),
            SettingActionPairContexts::new(
                context.clone()
                    & id!(flags::IS_ANY_AI_ENABLED)
                    & !id!(flags::USE_AGENT_FOOTER_FLAG),
                context.clone() & id!(flags::IS_ANY_AI_ENABLED) & id!(flags::USE_AGENT_FOOTER_FLAG),
            ),
            None,
        )
        .with_group(bindings::BindingGroup::WarpAi)],
        app,
    );
    if !FeatureFlag::FullSourceCodeEmbedding.is_enabled() {
        ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
            vec![ToggleSettingActionPair::new(
                "codebase index",
                builder(SettingsAction::AI(
                    AISettingsPageAction::ToggleCodebaseContext,
                )),
                &(context.clone() & id!(flags::IS_ANY_AI_ENABLED)),
                flags::IS_CODEBASE_INDEXING_ENABLED,
            )],
            app,
        );
    }
}

pub struct AISettingsPageView {
    page: PageType<Self>,
    active_subpage: Option<AISubpage>,
    voice_input_toggle_key_dropdown: ViewHandle<Dropdown<AISettingsPageAction>>,
    local_only_icon_tooltip_states: RefCell<HashMap<String, MouseStateHandle>>,
    autodetection_denylist_editor: ViewHandle<EditorView>,
    autonomy_dropdown_menu: ViewHandle<Dropdown<AISettingsPageAction>>,

    code_read_autonomy_dropdown_menu: ViewHandle<Dropdown<AISettingsPageAction>>,

    code_read_allowlist_editor: ViewHandle<SubmittableTextInput>,
    code_read_allowlist_mouse_state_handles: Vec<MouseStateHandle>,

    command_execution_allowlist_editor: ViewHandle<SubmittableTextInput>,
    command_execution_allowlist_mouse_state_handles: Vec<MouseStateHandle>,
    command_execution_denylist_editor: ViewHandle<SubmittableTextInput>,
    command_execution_denylist_mouse_state_handles: Vec<MouseStateHandle>,
    command_execution_denylist_tooltip_mouse_state_handles: Vec<MouseStateHandle>,
    cli_agent_footer_command_editor: ViewHandle<SubmittableTextInput>,
    cli_agent_footer_command_mouse_state_handles: Vec<MouseStateHandle>,
    cli_agent_footer_command_agent_dropdowns: Vec<ViewHandle<Dropdown<AISettingsPageAction>>>,
    agent_toolbar_inline_editor: ViewHandle<AgentToolbarInlineEditor>,
    cli_agent_toolbar_inline_editor: ViewHandle<AgentToolbarInlineEditor>,

    apply_code_diffs_dropdown_menu: ViewHandle<Dropdown<AISettingsPageAction>>,
    read_files_dropdown_menu: ViewHandle<Dropdown<AISettingsPageAction>>,
    execute_commands_dropdown_menu: ViewHandle<Dropdown<AISettingsPageAction>>,
    write_to_pty_autonomy_dropdown_menu: ViewHandle<Dropdown<AISettingsPageAction>>,
    mcp_permissions_dropdown_menu: ViewHandle<Dropdown<AISettingsPageAction>>,

    // Allowlisting directories (default profile)
    directory_allowlist_mouse_state_handles: Vec<MouseStateHandle>,
    directory_allowlist_editor: ViewHandle<SubmittableTextInput>,

    // Allowlisting commands (default profile)
    command_allowlist_mouse_state_handles: Vec<MouseStateHandle>,
    command_allowlist_editor: ViewHandle<SubmittableTextInput>,

    // Denylisting commands (default profile)
    command_denylist_mouse_state_handles: Vec<MouseStateHandle>,
    command_denylist_tooltip_mouse_state_handles: Vec<MouseStateHandle>,
    command_denylist_editor: ViewHandle<SubmittableTextInput>,

    mcp_allowlist_mouse_state_handles: Vec<MouseStateHandle>,
    mcp_allowlist_dropdown: ViewHandle<FilterableDropdown<AISettingsPageAction>>,

    mcp_denylist_mouse_state_handles: Vec<MouseStateHandle>,
    mcp_denylist_dropdown: ViewHandle<FilterableDropdown<AISettingsPageAction>>,

    base_model_dropdown: ViewHandle<Dropdown<AISettingsPageAction>>,
    coding_model_dropdown: ViewHandle<Dropdown<AISettingsPageAction>>,

    context_window_slider_state: SliderStateHandle,
    context_window_editor: ViewHandle<EditorView>,
    last_synced_context_window_editor_value: Option<u32>,

    thinking_display_mode_dropdown: ViewHandle<Dropdown<AISettingsPageAction>>,
    #[cfg(feature = "local_fs")]
    conversation_layout_dropdown: ViewHandle<Dropdown<AISettingsPageAction>>,

    // Profile views
    profile_views: Vec<ViewHandle<ExecutionProfileView>>,
    add_profile_button: ViewHandle<ActionButton>,

    // Custom inference (custom endpoints)
    custom_endpoint_modal_state: CustomEndpointModalViewState,
    remove_custom_endpoint_confirmation_dialog: ViewHandle<RemoveCustomEndpointConfirmationDialog>,
    pending_remove_custom_endpoint_index: Option<usize>,
    custom_inference_add_button: ViewHandle<ActionButton>,
    custom_endpoint_edit_buttons: Vec<ViewHandle<ActionButton>>,
}

impl AISettingsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let is_any_ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);

        let workspace = UserWorkspaces::handle(ctx);
        let ai_autonomy_settings = workspace.as_ref(ctx).ai_autonomy_settings();
        ctx.subscribe_to_model(&LanguageSettings::handle(ctx), |me, _, _, ctx| {
            me.refresh_localized_controls(ctx);
            ctx.notify();
        });
        ctx.subscribe_to_model(&workspace, |me, workspace, event, ctx| {
            if let UserWorkspacesEvent::TeamsChanged = event {
                me.refresh_all_execution_profile_ui(ctx);
                me.reset_execution_profile_mouse_state_handles(ctx);

                let is_any_ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
                let ai_autonomy_settings = workspace.as_ref(ctx).ai_autonomy_settings();

                Self::update_editor_interaction_state(
                    me.command_denylist_editor.as_ref(ctx).editor().clone(),
                    is_any_ai_enabled,
                    ctx,
                );

                Self::update_editor_interaction_state(
                    me.command_allowlist_editor.as_ref(ctx).editor().clone(),
                    is_any_ai_enabled
                        && !ai_autonomy_settings.has_override_for_execute_commands_allowlist(),
                    ctx,
                );

                Self::update_editor_interaction_state(
                    me.directory_allowlist_editor.as_ref(ctx).editor().clone(),
                    is_any_ai_enabled
                        && !ai_autonomy_settings.has_override_for_read_files_allowlist(),
                    ctx,
                );

                me.sync_custom_endpoint_buttons(ctx);
                ctx.notify();
            }
        });

        let voice_input_toggle_key_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            if !AISettings::as_ref(ctx).is_voice_input_enabled(ctx) {
                dropdown.set_disabled(ctx);
            }

            let values = VoiceInputToggleKey::all_possible_values();
            let current_value = AISettings::as_ref(ctx).voice_input_toggle_key.value();
            let selected_index = values
                .iter()
                .position(|val| val == current_value)
                .unwrap_or_else(|| {
                    log::warn!(
                        "Could not find current VoiceInputToggleKey value in dropdown option list"
                    );
                    0
                });

            dropdown.add_items(voice_input_toggle_key_dropdown_items(ctx), ctx);
            dropdown.set_selected_by_index(selected_index, ctx);

            dropdown
        });

        let coding_model_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);
            dropdown.set_menu_max_height(AI_SETTINGS_DROPDOWN_MAX_HEIGHT, ctx);
            dropdown
        });
        Self::refresh_coding_model_menu(&coding_model_dropdown, ctx);

        let base_model_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);
            dropdown.set_menu_max_height(AI_SETTINGS_DROPDOWN_MAX_HEIGHT, ctx);

            dropdown
        });
        Self::refresh_base_model_menu(&base_model_dropdown, ctx);

        let initial_context_window_value = Self::initial_context_window_value(ctx);
        let clamped_initial = Self::configurable_context_window(ctx)
            .map(|cw| initial_context_window_value.clamp(cw.min, cw.max))
            .unwrap_or(initial_context_window_value);
        let context_window_slider_state = SliderStateHandle::default();

        let context_window_editor = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_size_override: Some(Appearance::as_ref(ctx).ui_font_size()),
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_buffer_text(&clamped_initial.to_string(), ctx);
            editor
        });
        ctx.subscribe_to_view(&context_window_editor, |me, _, event, ctx| {
            me.handle_context_window_editor_event(event, ctx);
        });
        let last_synced_context_window_editor_value = Some(clamped_initial);

        let thinking_display_mode_dropdown =
            OtherAIWidget::create_thinking_display_mode_dropdown(ctx);
        // Set initial selection based on current setting value.
        {
            let current_mode = AISettings::as_ref(ctx).thinking_display_mode;
            thinking_display_mode_dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_selected_by_action(
                    AISettingsPageAction::SetThinkingDisplayMode(current_mode),
                    ctx,
                );
            });
        }

        let autonomy_dropdown_menu = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);
            dropdown
        });
        Self::refresh_autonomy_dropdown_menu(&autonomy_dropdown_menu, ctx);

        let code_read_autonomy_dropdown_menu = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);
            dropdown
        });
        Self::refresh_code_read_autonomy_dropdown_menu(&code_read_autonomy_dropdown_menu, ctx);

        // While the data model supports arbitrary files in the allowlist,
        // it's most intuitive to allowlist whole directories.
        let code_read_allowlist_editor = ctx.add_typed_action_view(|ctx| {
            let mut input = SubmittableTextInput::new(ctx).validate_on_submit(|s| {
                let expanded = host_native_absolute_path(s, &None, &None);
                Path::new(&expanded).is_dir()
            });
            input.set_placeholder_text(i18n::tr(ctx, I18nKey::AiDirectoryPathPlaceholder), ctx);
            input
        });
        Self::update_editor_interaction_state(
            code_read_allowlist_editor.as_ref(ctx).editor().clone(),
            is_any_ai_enabled,
            ctx,
        );

        ctx.subscribe_to_view(&code_read_allowlist_editor, |_, _, event, ctx| {
            if let SubmittableTextInputEvent::Submit(s) = event {
                let expanded = host_native_absolute_path(s, &None, &None);
                BlocklistAIPermissions::handle(ctx).update(ctx, |model, ctx| {
                    report_if_error!(
                        model.add_filepath_to_code_read_allowlist(PathBuf::from(expanded), ctx)
                    );
                });
            }
        });

        let autodetection_denylist_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = EditorOptions {
                autogrow: true,
                soft_wrap: true,
                text: TextOptions {
                    font_size_override: Some(appearance.ui_font_size()),
                    font_family_override: Some(appearance.monospace_font_family()),
                    text_colors_override: Some(TextColors {
                        default_color: appearance.theme().active_ui_text_color(),
                        disabled_color: appearance.theme().disabled_ui_text_color(),
                        hint_color: appearance.theme().disabled_ui_text_color(),
                    }),
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut editor = EditorView::new(options, ctx);

            editor.set_placeholder_text(
                i18n::tr(ctx, I18nKey::AiCommandsCommaSeparatedPlaceholder),
                ctx,
            );

            let current_value = AISettings::as_ref(ctx)
                .autodetection_command_denylist
                .value()
                .clone();
            editor.set_buffer_text(current_value.as_str(), ctx);
            editor
        });
        Self::update_editor_interaction_state(
            autodetection_denylist_editor.clone(),
            is_any_ai_enabled,
            ctx,
        );

        ctx.subscribe_to_view(&autodetection_denylist_editor, move |me, _, event, ctx| {
            me.handle_detection_denylist_editor_event(event, ctx);
        });

        let command_execution_allowlist_editor = ctx.add_typed_action_view(|ctx| {
            let mut input =
                SubmittableTextInput::new(ctx).validate_on_edit(|s| Regex::new(s).is_ok());
            input.set_placeholder_text(i18n::tr(ctx, I18nKey::AiCommandAllowPlaceholder), ctx);
            input
        });
        Self::update_editor_interaction_state(
            command_execution_allowlist_editor
                .as_ref(ctx)
                .editor()
                .clone(),
            is_any_ai_enabled,
            ctx,
        );

        ctx.subscribe_to_view(&command_execution_allowlist_editor, |_, _, event, ctx| {
            if let SubmittableTextInputEvent::Submit(s) = event {
                let predicate = match AgentModeCommandExecutionPredicate::new_regex(s) {
                    Ok(regex) => regex,
                    Err(e) => {
                        log::warn!(
                            "Failed to convert string to regex for cmd execution allowlist: {e}"
                        );
                        return;
                    }
                };
                BlocklistAIPermissions::handle(ctx).update(ctx, |model, ctx| {
                    report_if_error!(model.add_command_to_autoexecution_allowlist(predicate, ctx));
                })
            }
        });

        let command_execution_denylist_editor = ctx.add_typed_action_view(|ctx| {
            let mut input =
                SubmittableTextInput::new(ctx).validate_on_edit(|s| Regex::new(s).is_ok());
            input.set_placeholder_text(i18n::tr(ctx, I18nKey::AiCommandDenyPlaceholder), ctx);
            input
        });
        Self::update_editor_interaction_state(
            command_execution_denylist_editor
                .as_ref(ctx)
                .editor()
                .clone(),
            is_any_ai_enabled,
            ctx,
        );

        ctx.subscribe_to_view(&command_execution_denylist_editor, |_, _, event, ctx| {
            if let SubmittableTextInputEvent::Submit(s) = event {
                let predicate = match AgentModeCommandExecutionPredicate::new_regex(s) {
                    Ok(regex) => regex,
                    Err(e) => {
                        log::warn!(
                            "Failed to convert string to regex for cmd execution denylist: {e}"
                        );
                        return;
                    }
                };
                BlocklistAIPermissions::handle(ctx).update(ctx, |model, ctx| {
                    report_if_error!(model.add_command_to_autoexecution_denylist(predicate, ctx));
                })
            }
        });

        let cli_agent_footer_command_editor = ctx.add_typed_action_view(|ctx| {
            let mut input =
                SubmittableTextInput::new(ctx).validate_on_edit(|s| Regex::new(s).is_ok());
            input.set_placeholder_text(i18n::tr(ctx, I18nKey::AiCommandRegexPlaceholder), ctx);
            input
        });
        // The coding agent footer command editor is always enabled,
        // independent of the global AI toggle, because it controls
        // third-party coding agents rather than Warp's own AI.
        Self::update_editor_interaction_state(
            cli_agent_footer_command_editor.as_ref(ctx).editor().clone(),
            true,
            ctx,
        );
        ctx.subscribe_to_view(
            &cli_agent_footer_command_editor,
            |_, _, event, ctx| match event {
                SubmittableTextInputEvent::Submit(command) => {
                    AISettings::handle(ctx).update(ctx, |settings, ctx| {
                        settings.add_cli_agent_footer_enabled_command(command, ctx);
                    });
                }
                SubmittableTextInputEvent::Escape => ctx.emit(AISettingsPageEvent::FocusModal),
            },
        );

        let request_usage_model = AIRequestUsageModel::handle(ctx);
        ctx.subscribe_to_model(&request_usage_model, |_, _, _, ctx| {
            // The only event is RequestUsageUpdated
            ctx.notify();
        });

        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _handle, _event, ctx| {
            // Re-render if teams-related data changed that may affect whether features such as voice input are enabled.
            Self::refresh_base_model_menu(&me.base_model_dropdown, ctx);
            Self::refresh_coding_model_menu(&me.coding_model_dropdown, ctx);
            me.sync_custom_endpoint_buttons(ctx);
            ctx.notify();
        });

        ctx.subscribe_to_model(
            &AIExecutionProfilesModel::handle(ctx),
            |me, _, event, ctx| {
                match event {
                    AIExecutionProfilesModelEvent::ProfileCreated
                    | AIExecutionProfilesModelEvent::ProfileDeleted => {
                        me.refresh_profile_views(ctx);
                    }
                    AIExecutionProfilesModelEvent::ProfileUpdated(_) => {
                        me.refresh_all_execution_profile_ui(ctx);
                        me.reset_execution_profile_mouse_state_handles(ctx);
                        me.sync_context_window_editor(ctx, false);
                    }
                    AIExecutionProfilesModelEvent::UpdatedActiveProfile { .. } => (),
                }
                ctx.notify();
            },
        );

        let cloud_model = CloudModel::handle(ctx);
        ctx.subscribe_to_model(&cloud_model, |me, _, event, ctx| {
            let added_or_deleted_mcp_servers = matches!(
                event,
                CloudModelEvent::ObjectCreated { type_and_id } | CloudModelEvent::ObjectDeleted { type_and_id, .. }
                if matches!(
                    type_and_id.object_type(),
                    ObjectType::GenericStringObject(Json(JsonObjectType::MCPServer))
                )
            );

            if added_or_deleted_mcp_servers {
                Self::refresh_mcp_allowlist_dropdown(&me.mcp_allowlist_dropdown, ctx);
                Self::refresh_mcp_denylist_dropdown(&me.mcp_denylist_dropdown, ctx);
                ctx.notify();
            }
        });

        let templatable_manager = TemplatableMCPServerManager::handle(ctx);
        ctx.subscribe_to_model(&templatable_manager, |me, _, _event, ctx| {
            Self::refresh_mcp_allowlist_dropdown(&me.mcp_allowlist_dropdown, ctx);
            Self::refresh_mcp_denylist_dropdown(&me.mcp_denylist_dropdown, ctx);
            ctx.notify();
        });

        ctx.subscribe_to_model(
            &LLMPreferences::handle(ctx),
            |me, _, event, ctx| match event {
                LLMPreferencesEvent::UpdatedAvailableLLMs => {
                    Self::refresh_base_model_menu(&me.base_model_dropdown, ctx);
                    Self::refresh_coding_model_menu(&me.coding_model_dropdown, ctx);
                    me.sync_context_window_editor(ctx, false);
                }
                LLMPreferencesEvent::UpdatedActiveAgentModeLLM => {
                    Self::refresh_base_model_menu(&me.base_model_dropdown, ctx);
                    me.sync_context_window_editor(ctx, false);
                }
                LLMPreferencesEvent::UpdatedActiveCodingLLM => {
                    Self::refresh_coding_model_menu(&me.coding_model_dropdown, ctx);
                }
            },
        );

        // Refresh model dropdowns when BYO API keys update so key icons reflect latest state.
        ctx.subscribe_to_model(&ApiKeyManager::handle(ctx), |me, _model, _event, ctx| {
            Self::refresh_base_model_menu(&me.base_model_dropdown, ctx);
            Self::refresh_coding_model_menu(&me.coding_model_dropdown, ctx);
            me.sync_context_window_editor(ctx, false);
            me.sync_custom_endpoint_buttons(ctx);
            ctx.notify();
        });

        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, event, ctx| {
            match event {
                AISettingsChangedEvent::AICommandDenylist { .. } => {
                    me.autodetection_denylist_editor.update(ctx, |editor, ctx| {
                        let denylist_value = &AISettings::as_ref(ctx)
                            .autodetection_command_denylist
                            .value()
                            .clone();
                        editor.set_buffer_text(denylist_value, ctx);
                    });
                }
                AISettingsChangedEvent::IsAnyAIEnabled { .. } => {
                    let is_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
                    let ai_autonomy_settings = UserWorkspaces::as_ref(ctx).ai_autonomy_settings();

                    Self::update_editor_interaction_state(
                        me.autodetection_denylist_editor.clone(),
                        is_enabled,
                        ctx,
                    );
                    Self::update_editor_interaction_state(
                        me.command_execution_allowlist_editor
                            .as_ref(ctx)
                            .editor()
                            .clone(),
                        is_enabled,
                        ctx,
                    );
                    Self::update_editor_interaction_state(
                        me.command_execution_denylist_editor
                            .as_ref(ctx)
                            .editor()
                            .clone(),
                        is_enabled,
                        ctx,
                    );
                    Self::update_editor_interaction_state(
                        me.code_read_allowlist_editor.as_ref(ctx).editor().clone(),
                        is_enabled,
                        ctx,
                    );

                    Self::update_editor_interaction_state(
                        me.directory_allowlist_editor.as_ref(ctx).editor().clone(),
                        is_enabled && !ai_autonomy_settings.has_override_for_read_files_allowlist(),
                        ctx,
                    );

                    Self::update_editor_interaction_state(
                        me.command_denylist_editor.as_ref(ctx).editor().clone(),
                        is_enabled,
                        ctx,
                    );

                    Self::update_editor_interaction_state(
                        me.command_allowlist_editor.as_ref(ctx).editor().clone(),
                        is_enabled
                            && !ai_autonomy_settings.has_override_for_execute_commands_allowlist(),
                        ctx,
                    );

                    me.update_voice_input_dropdown_enablement(ctx);
                    Self::refresh_autonomy_dropdown_menu(&me.autonomy_dropdown_menu, ctx);

                    me.refresh_all_execution_profile_ui(ctx);

                    Self::refresh_code_read_autonomy_dropdown_menu(
                        &me.code_read_autonomy_dropdown_menu,
                        ctx,
                    );
                    Self::refresh_base_model_menu(&me.base_model_dropdown, ctx);
                    Self::refresh_coding_model_menu(&me.coding_model_dropdown, ctx);
                    Self::refresh_mcp_allowlist_dropdown(&me.mcp_allowlist_dropdown, ctx);
                    Self::refresh_mcp_denylist_dropdown(&me.mcp_denylist_dropdown, ctx);
                    me.sync_context_window_editor(ctx, true);
                    me.sync_custom_endpoint_buttons(ctx);
                }
                AISettingsChangedEvent::VoiceInputEnabled { .. } => {
                    me.update_voice_input_dropdown_enablement(ctx);
                }
                AISettingsChangedEvent::AgentModeExecuteReadonlyCommands { .. } => {
                    Self::refresh_autonomy_dropdown_menu(&me.autonomy_dropdown_menu, ctx);
                    Self::refresh_code_read_autonomy_dropdown_menu(
                        &me.code_read_autonomy_dropdown_menu,
                        ctx,
                    );
                }
                AISettingsChangedEvent::AgentModeCodingPermissions { .. } => {
                    Self::refresh_code_read_autonomy_dropdown_menu(
                        &me.code_read_autonomy_dropdown_menu,
                        ctx,
                    );
                }
                AISettingsChangedEvent::VoiceInputToggleKey { .. } => {
                    let current_value = *AISettings::as_ref(ctx).voice_input_toggle_key.value();
                    me.voice_input_toggle_key_dropdown
                        .update(ctx, |dropdown, ctx| {
                            dropdown.set_selected_by_action(
                                AISettingsPageAction::SetVoiceInputToggleKey(current_value),
                                ctx,
                            )
                        });
                }
                AISettingsChangedEvent::AgentModeCommandExecutionAllowlist { .. } => {
                    me.command_execution_allowlist_mouse_state_handles = AISettings::as_ref(ctx)
                        .agent_mode_command_execution_allowlist
                        .value()
                        .iter()
                        .map(|_| Default::default())
                        .collect();
                }
                AISettingsChangedEvent::AgentModeCommandExecutionDenylist { .. } => {
                    me.command_execution_denylist_mouse_state_handles = AISettings::as_ref(ctx)
                        .agent_mode_command_execution_denylist
                        .value()
                        .iter()
                        .map(|_| Default::default())
                        .collect();
                    me.command_execution_denylist_tooltip_mouse_state_handles =
                        AISettings::as_ref(ctx)
                            .agent_mode_command_execution_denylist
                            .value()
                            .iter()
                            .map(|_| Default::default())
                            .collect();
                }
                AISettingsChangedEvent::AgentModeCodingFileReadAllowlist { .. } => {
                    me.code_read_allowlist_mouse_state_handles = AISettings::as_ref(ctx)
                        .agent_mode_coding_file_read_allowlist
                        .value()
                        .iter()
                        .map(|_| Default::default())
                        .collect();
                }
                AISettingsChangedEvent::CLIAgentToolbarEnabledCommands { .. } => {
                    me.cli_agent_footer_command_mouse_state_handles = AISettings::as_ref(ctx)
                        .cli_agent_footer_enabled_commands
                        .value()
                        .keys()
                        .map(|_| Default::default())
                        .collect();
                    me.cli_agent_footer_command_agent_dropdowns =
                        Self::create_cli_agent_dropdowns(ctx);
                }
                AISettingsChangedEvent::ThinkingDisplayMode { .. } => {
                    let current_mode = *AISettings::as_ref(ctx).thinking_display_mode.value();
                    me.thinking_display_mode_dropdown
                        .update(ctx, |dropdown, ctx| {
                            dropdown.set_selected_by_action(
                                AISettingsPageAction::SetThinkingDisplayMode(current_mode),
                                ctx,
                            );
                        });
                }
                _ => (),
            }
            ctx.notify();
        });

        ctx.subscribe_to_model(&SessionSettings::handle(ctx), |_, _, event, ctx| {
            if let SessionSettingsChangedEvent::ShowModelSelectorsInPrompt { .. } = event {
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(&InputSettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });

        let current_permission =
            BlocklistAIPermissions::as_ref(ctx).active_permissions_profile(ctx, None);

        let apply_code_diffs_dropdown_menu = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);

            dropdown.set_items(
                action_permission_dropdown_items(ctx, AISettingsPageAction::SetApplyCodeDiffs),
                ctx,
            );
            dropdown
        });
        Self::refresh_execution_profile_dropdown_menu(
            &apply_code_diffs_dropdown_menu,
            current_permission.apply_code_diffs,
            !AISettings::as_ref(ctx).is_code_diffs_permissions_editable(ctx),
            ctx,
        );

        let read_files_dropdown_menu = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);
            dropdown.set_items(
                action_permission_dropdown_items(ctx, AISettingsPageAction::SetReadFiles),
                ctx,
            );
            dropdown
        });
        Self::refresh_execution_profile_dropdown_menu(
            &read_files_dropdown_menu,
            current_permission.read_files,
            !AISettings::as_ref(ctx).is_read_files_permissions_editable(ctx),
            ctx,
        );

        let execute_commands_dropdown_menu = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);
            dropdown.set_items(
                action_permission_dropdown_items(ctx, AISettingsPageAction::SetExecuteCommands),
                ctx,
            );
            dropdown
        });
        Self::refresh_execution_profile_dropdown_menu(
            &execute_commands_dropdown_menu,
            current_permission.execute_commands,
            !AISettings::as_ref(ctx).is_execute_commands_permissions_editable(ctx),
            ctx,
        );

        let write_to_pty_autonomy_dropdown_menu = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);
            dropdown.set_items(write_to_pty_dropdown_items(ctx), ctx);
            dropdown
        });
        Self::refresh_write_to_pty_dropdown_menu(
            &write_to_pty_autonomy_dropdown_menu,
            current_permission.write_to_pty,
            !AISettings::as_ref(ctx).is_write_to_pty_permissions_editable(ctx),
            ctx,
        );

        let mcp_permissions_dropdown_menu = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);
            dropdown.set_items(
                action_permission_dropdown_items(ctx, AISettingsPageAction::SetMCPPermissions),
                ctx,
            );
            dropdown
        });
        Self::refresh_execution_profile_dropdown_menu(
            &mcp_permissions_dropdown_menu,
            current_permission.mcp_permissions,
            !AISettings::as_ref(ctx).is_mcp_permission_editable(ctx),
            ctx,
        );

        let mcp_allowlist_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = FilterableDropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);
            dropdown.set_menu_header_to_static(i18n::tr(ctx, I18nKey::AiSelectMcpServers));
            dropdown
        });
        Self::refresh_mcp_allowlist_dropdown(&mcp_allowlist_dropdown, ctx);
        let mcp_allowlist_mouse_state_handles = BlocklistAIPermissions::as_ref(ctx)
            .get_mcp_allowlist(ctx, None)
            .iter()
            .map(|_| Default::default())
            .collect();

        let mcp_denylist_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = FilterableDropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);
            dropdown.set_menu_header_to_static(i18n::tr(ctx, I18nKey::AiSelectMcpServers));
            dropdown
        });
        Self::refresh_mcp_denylist_dropdown(&mcp_denylist_dropdown, ctx);
        let mcp_denylist_mouse_state_handles = BlocklistAIPermissions::as_ref(ctx)
            .get_mcp_denylist(ctx, None)
            .iter()
            .map(|_| Default::default())
            .collect();

        let command_execution_allowlist_mouse_state_handles = AISettings::as_ref(ctx)
            .agent_mode_command_execution_allowlist
            .value()
            .iter()
            .map(|_| Default::default())
            .collect();

        let command_execution_denylist_mouse_state_handles = AISettings::as_ref(ctx)
            .agent_mode_command_execution_denylist
            .value()
            .iter()
            .map(|_| Default::default())
            .collect();
        let command_execution_denylist_tooltip_mouse_state_handles = AISettings::as_ref(ctx)
            .agent_mode_command_execution_denylist
            .value()
            .iter()
            .map(|_| Default::default())
            .collect();
        let cli_agent_footer_command_mouse_state_handles = AISettings::as_ref(ctx)
            .cli_agent_footer_enabled_commands
            .value()
            .keys()
            .map(|_| Default::default())
            .collect();

        let code_read_allowlist_mouse_state_handles = AISettings::as_ref(ctx)
            .agent_mode_coding_file_read_allowlist
            .value()
            .iter()
            .map(|_| Default::default())
            .collect();

        let directory_allowlist_mouse_state_handles = current_permission
            .directory_allowlist
            .iter()
            .map(|_| Default::default())
            .collect();

        let directory_allowlist_editor = ctx.add_typed_action_view(|ctx| {
            let mut input = SubmittableTextInput::new(ctx).validate_on_submit(|s| {
                let expanded = host_native_absolute_path(s, &None, &None);
                Path::new(&expanded).is_dir()
            });
            input.set_placeholder_text(i18n::tr(ctx, I18nKey::AiDirectoryPathPlaceholder), ctx);
            input
        });

        Self::update_editor_interaction_state(
            directory_allowlist_editor.as_ref(ctx).editor().clone(),
            is_any_ai_enabled,
            ctx,
        );

        ctx.subscribe_to_view(&directory_allowlist_editor, |_, _, event, ctx| {
            if let SubmittableTextInputEvent::Submit(s) = event {
                let expanded = host_native_absolute_path(s, &None, &None);
                AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                    let profile = model.default_profile(ctx);
                    let profile_id = profile.id();

                    model.add_to_directory_allowlist(*profile_id, &PathBuf::from(expanded), ctx);
                });
                ctx.notify();
            }
        });

        let org_denylist = BlocklistAIPermissions::get_org_execute_commands_denylist(ctx);
        let command_denylist_mouse_state_handles = current_permission
            .command_denylist
            .iter()
            .map(|_| Default::default())
            .collect();
        let command_denylist_tooltip_mouse_state_handles: Vec<MouseStateHandle> =
            org_denylist.iter().map(|_| Default::default()).collect();

        let command_denylist_editor = ctx.add_typed_action_view(|ctx| {
            let mut input =
                SubmittableTextInput::new(ctx).validate_on_edit(|s| Regex::new(s).is_ok());
            input.set_placeholder_text(i18n::tr(ctx, I18nKey::AiCommandDenyPlaceholder), ctx);
            input
        });
        Self::update_editor_interaction_state(
            command_denylist_editor.as_ref(ctx).editor().clone(),
            is_any_ai_enabled && !ai_autonomy_settings.has_override_for_execute_commands_denylist(),
            ctx,
        );

        ctx.subscribe_to_view(&command_denylist_editor, |_, _, event, ctx| {
            if let SubmittableTextInputEvent::Submit(s) = event {
                let predicate = match AgentModeCommandExecutionPredicate::new_regex(s) {
                    Ok(regex) => regex,
                    Err(e) => {
                        log::warn!(
                            "Failed to convert string to regex for cmd execution denylist: {e}"
                        );
                        return;
                    }
                };
                AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                    let profile = model.default_profile(ctx);
                    let profile_id = profile.id();
                    model.add_to_command_denylist(*profile_id, &predicate, ctx);
                });
                ctx.notify();
            }
        });

        let command_allowlist_mouse_state_handles = current_permission
            .command_allowlist
            .iter()
            .map(|_| Default::default())
            .collect();

        let command_allowlist_editor = ctx.add_typed_action_view(|ctx| {
            let mut input =
                SubmittableTextInput::new(ctx).validate_on_edit(|s| Regex::new(s).is_ok());
            input.set_placeholder_text(i18n::tr(ctx, I18nKey::AiCommandAllowPlaceholder), ctx);
            input
        });
        Self::update_editor_interaction_state(
            command_allowlist_editor.as_ref(ctx).editor().clone(),
            is_any_ai_enabled
                && !ai_autonomy_settings.has_override_for_execute_commands_allowlist(),
            ctx,
        );

        ctx.subscribe_to_view(&command_allowlist_editor, |_, _, event, ctx| {
            if let SubmittableTextInputEvent::Submit(s) = event {
                let predicate = match AgentModeCommandExecutionPredicate::new_regex(s) {
                    Ok(regex) => regex,
                    Err(e) => {
                        log::warn!(
                            "Failed to convert string to regex for cmd execution allowlist: {e}"
                        );
                        return;
                    }
                };
                AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                    let profile = model.default_profile(ctx);
                    let profile_id = profile.id();
                    model.add_to_command_allowlist(*profile_id, &predicate, ctx);
                });
                ctx.notify();
            }
        });

        let ai_request_model = AIRequestUsageModel::handle(ctx);
        ctx.subscribe_to_model(&ai_request_model, |me, _, event, ctx| {
            match event {
                AIRequestUsageModelEvent::RequestUsageUpdated => ctx.notify(),
                AIRequestUsageModelEvent::RequestBonusRefunded { .. } => ctx.notify(),
            }
            Self::refresh_base_model_menu(&me.base_model_dropdown, ctx);
            Self::refresh_coding_model_menu(&me.coding_model_dropdown, ctx);
        });

        let profile_views = Self::create_profile_views(ctx);

        let add_profile_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(i18n::tr(ctx, I18nKey::AiAddProfile), SecondaryTheme)
                .with_icon(Icon::Plus)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AISettingsPageAction::CreateProfile);
                })
        });

        add_profile_button.update(ctx, |button, ctx| {
            button.set_disabled(!is_any_ai_enabled, ctx);
        });

        // Custom inference
        let custom_inference_controls_enabled =
            is_any_ai_enabled && UserWorkspaces::as_ref(ctx).is_custom_inference_enabled(ctx);
        let custom_inference_add_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("+ Add custom model", SecondaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AISettingsPageAction::OpenAddCustomEndpointModal);
                })
        });
        custom_inference_add_button.update(ctx, |button, ctx| {
            button.set_disabled(!custom_inference_controls_enabled, ctx);
        });

        let custom_endpoint_modal_body =
            ctx.add_typed_action_view(|ctx| CustomEndpointModal::new(None, None, ctx));
        ctx.subscribe_to_view(&custom_endpoint_modal_body, |me, _, event, ctx| {
            me.handle_custom_endpoint_modal_event(event, ctx);
        });

        let custom_endpoint_modal_view = ctx.add_typed_action_view(|ctx| {
            Modal::new(
                Some("Add custom endpoint".to_string()),
                custom_endpoint_modal_body.clone(),
                ctx,
            )
            .with_modal_style(UiComponentStyles {
                width: Some(560.),
                height: Some(600.),
                ..Default::default()
            })
            .with_header_style(UiComponentStyles {
                padding: Some(Coords {
                    top: 24.,
                    bottom: 0.,
                    left: 24.,
                    right: 24.,
                }),
                font_size: Some(16.),
                font_weight: Some(Weight::Bold),
                ..Default::default()
            })
            .with_body_style(UiComponentStyles {
                padding: Some(Coords {
                    top: 0.,
                    bottom: 24.,
                    left: 24.,
                    right: 24.,
                }),
                ..Default::default()
            })
            .with_background_opacity(100)
            .with_dismiss_on_click()
            .with_dismiss_keystroke(Keystroke::parse("escape").unwrap())
        });
        ctx.subscribe_to_view(&custom_endpoint_modal_view, |me, _, event, ctx| {
            me.handle_custom_endpoint_modal_close_event(event, ctx);
        });

        let custom_endpoint_modal_state =
            CustomEndpointModalViewState::new(ModalViewState::new(custom_endpoint_modal_view));

        let remove_custom_endpoint_confirmation_dialog =
            ctx.add_typed_action_view(RemoveCustomEndpointConfirmationDialog::new);
        ctx.subscribe_to_view(
            &remove_custom_endpoint_confirmation_dialog,
            |me, _, event, ctx| {
                me.handle_remove_custom_endpoint_confirmation_dialog_event(event, ctx);
            },
        );

        let custom_endpoint_edit_buttons = Self::create_custom_endpoint_edit_buttons(
            ApiKeyManager::as_ref(ctx).keys().custom_endpoints.len(),
            custom_inference_controls_enabled,
            ctx,
        );

        let agent_toolbar_inline_editor = ctx.add_typed_action_view(|ctx| {
            AgentToolbarInlineEditor::new(AgentToolbarEditorMode::AgentView, ctx)
        });
        let cli_agent_toolbar_inline_editor = ctx.add_typed_action_view(|ctx| {
            AgentToolbarInlineEditor::new(AgentToolbarEditorMode::CLIAgent, ctx)
        });

        #[cfg(feature = "local_fs")]
        let conversation_layout_dropdown = ctx.add_typed_action_view(|ctx| {
            use crate::util::file::external_editor::settings::OpenConversationPreference;

            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);

            let items = vec![
                DropdownItem::new(
                    i18n::tr(ctx, I18nKey::CodeExternalNewTab),
                    AISettingsPageAction::SetConversationLayout(OpenConversationPreference::NewTab),
                ),
                DropdownItem::new(
                    i18n::tr(ctx, I18nKey::CodeExternalSplitPane),
                    AISettingsPageAction::SetConversationLayout(
                        OpenConversationPreference::SplitPane,
                    ),
                ),
            ];
            dropdown.set_items(items, ctx);

            let current = *crate::util::file::external_editor::EditorSettings::as_ref(ctx)
                .open_conversation_layout_preference;
            match current {
                OpenConversationPreference::NewTab => {
                    dropdown.set_selected_by_name(i18n::tr(ctx, I18nKey::CodeExternalNewTab), ctx)
                }
                OpenConversationPreference::SplitPane => dropdown
                    .set_selected_by_name(i18n::tr(ctx, I18nKey::CodeExternalSplitPane), ctx),
            };
            dropdown
        });

        Self {
            page: Self::build_page(None, ctx),
            active_subpage: None,
            voice_input_toggle_key_dropdown,
            autodetection_denylist_editor,
            local_only_icon_tooltip_states: Default::default(),
            command_execution_allowlist_editor,
            command_execution_denylist_editor,
            command_execution_allowlist_mouse_state_handles,
            command_execution_denylist_mouse_state_handles,
            command_execution_denylist_tooltip_mouse_state_handles,
            cli_agent_footer_command_editor,
            cli_agent_footer_command_mouse_state_handles,
            cli_agent_footer_command_agent_dropdowns: Self::create_cli_agent_dropdowns(ctx),
            agent_toolbar_inline_editor,
            cli_agent_toolbar_inline_editor,
            base_model_dropdown,
            coding_model_dropdown,
            context_window_slider_state,
            context_window_editor,
            last_synced_context_window_editor_value,
            autonomy_dropdown_menu,
            code_read_allowlist_editor,
            code_read_autonomy_dropdown_menu,
            code_read_allowlist_mouse_state_handles,
            apply_code_diffs_dropdown_menu,
            read_files_dropdown_menu,
            execute_commands_dropdown_menu,
            write_to_pty_autonomy_dropdown_menu,
            mcp_permissions_dropdown_menu,
            directory_allowlist_mouse_state_handles,
            directory_allowlist_editor,
            command_denylist_mouse_state_handles,
            command_denylist_tooltip_mouse_state_handles,
            command_denylist_editor,
            command_allowlist_mouse_state_handles,
            command_allowlist_editor,
            mcp_allowlist_dropdown,
            mcp_allowlist_mouse_state_handles,
            mcp_denylist_dropdown,
            mcp_denylist_mouse_state_handles,
            thinking_display_mode_dropdown,
            #[cfg(feature = "local_fs")]
            conversation_layout_dropdown,
            profile_views,
            add_profile_button,
            custom_endpoint_modal_state,
            remove_custom_endpoint_confirmation_dialog,
            pending_remove_custom_endpoint_index: None,
            custom_inference_add_button,
            custom_endpoint_edit_buttons,
        }
    }

    fn update_voice_input_dropdown_enablement(&mut self, ctx: &mut ViewContext<Self>) {
        let is_voice_enabled = AISettings::as_ref(ctx).is_voice_input_enabled(ctx);
        self.voice_input_toggle_key_dropdown
            .update(ctx, |dropdown, ctx| {
                if is_voice_enabled {
                    dropdown.set_enabled(ctx);
                } else {
                    dropdown.set_disabled(ctx);
                }
            });
        ctx.notify();
    }

    pub fn get_modal_content(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if self.custom_endpoint_modal_state.is_open() {
            Some(self.custom_endpoint_modal_state.render())
        } else if self
            .remove_custom_endpoint_confirmation_dialog
            .as_ref(app)
            .is_visible()
        {
            Some(ChildView::new(&self.remove_custom_endpoint_confirmation_dialog).finish())
        } else {
            None
        }
    }

    fn sync_custom_endpoint_buttons(&mut self, ctx: &mut ViewContext<Self>) {
        let enabled = Self::can_use_custom_inference_controls(ctx);

        self.custom_inference_add_button.update(ctx, |button, ctx| {
            button.set_disabled(!enabled, ctx);
        });

        let endpoint_count = ApiKeyManager::as_ref(ctx).keys().custom_endpoints.len();
        if self.custom_endpoint_edit_buttons.len() != endpoint_count {
            self.custom_endpoint_edit_buttons =
                Self::create_custom_endpoint_edit_buttons(endpoint_count, enabled, ctx);
        } else {
            for button in &self.custom_endpoint_edit_buttons {
                button.update(ctx, |button, ctx| {
                    button.set_disabled(!enabled, ctx);
                });
            }
        }
    }

    fn create_custom_endpoint_edit_buttons(
        count: usize,
        enabled: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Vec<ViewHandle<ActionButton>> {
        (0..count)
            .map(|index| {
                let button = ctx.add_typed_action_view(move |_| {
                    ActionButton::new("Edit", SecondaryTheme)
                        .with_icon(Icon::Pencil)
                        .with_size(ButtonSize::Small)
                        .on_click(move |ctx| {
                            ctx.dispatch_typed_action(
                                AISettingsPageAction::OpenEditCustomEndpointModal(index),
                            );
                        })
                });
                button.update(ctx, |button, ctx| {
                    button.set_disabled(!enabled, ctx);
                });
                button
            })
            .collect()
    }
    fn can_use_custom_inference_controls(app: &AppContext) -> bool {
        FeatureFlag::CustomInferenceEndpoints.is_enabled()
            && AISettings::as_ref(app).is_any_ai_enabled(app)
            && UserWorkspaces::as_ref(app).is_custom_inference_enabled(app)
    }

    fn show_add_custom_endpoint_modal(&mut self, ctx: &mut ViewContext<Self>) {
        if !Self::can_use_custom_inference_controls(ctx) {
            return;
        }
        self.remove_custom_endpoint_confirmation_dialog
            .update(ctx, |dialog, ctx| {
                dialog.hide(ctx);
            });
        self.pending_remove_custom_endpoint_index = None;

        self.custom_endpoint_modal_state
            .set_title(Some("Add custom endpoint".to_string()), ctx);
        self.custom_endpoint_modal_state.prefill(None, None, ctx);
        self.custom_endpoint_modal_state.open(ctx);
        ctx.emit(AISettingsPageEvent::ShowModal);
        ctx.notify();
    }

    fn show_edit_custom_endpoint_modal(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if !Self::can_use_custom_inference_controls(ctx) {
            return;
        }
        let endpoint = ApiKeyManager::as_ref(ctx)
            .keys()
            .custom_endpoints
            .get(index)
            .cloned();
        if endpoint.is_none() {
            return;
        }

        self.remove_custom_endpoint_confirmation_dialog
            .update(ctx, |dialog, ctx| {
                dialog.hide(ctx);
            });
        self.pending_remove_custom_endpoint_index = None;

        self.custom_endpoint_modal_state
            .set_title(Some("Edit custom endpoint".to_string()), ctx);
        self.custom_endpoint_modal_state
            .prefill(endpoint.as_ref(), Some(index), ctx);
        self.custom_endpoint_modal_state.open(ctx);
        ctx.emit(AISettingsPageEvent::ShowModal);
        ctx.notify();
    }

    fn hide_custom_endpoint_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.custom_endpoint_modal_state.close(ctx);
        ctx.emit(AISettingsPageEvent::HideModal);
        ctx.notify();
    }

    fn handle_custom_endpoint_modal_close_event(
        &mut self,
        event: &ModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ModalEvent::Close => {
                self.hide_custom_endpoint_modal(ctx);
            }
        }
    }

    fn handle_custom_endpoint_modal_event(
        &mut self,
        event: &CustomEndpointModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CustomEndpointModalEvent::Close => {
                self.hide_custom_endpoint_modal(ctx);
            }
            CustomEndpointModalEvent::AddEndpoint {
                name,
                url,
                api_key,
                models,
            } => {
                if !Self::can_use_custom_inference_controls(ctx) {
                    self.hide_custom_endpoint_modal(ctx);
                    return;
                }
                ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.add_custom_endpoint(
                        name.clone(),
                        url.clone(),
                        api_key.clone(),
                        models.clone(),
                        ctx,
                    );
                });
                self.hide_custom_endpoint_modal(ctx);

                let window_id = ctx.window_id();
                crate::ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = crate::view_components::DismissibleToast::success(
                        "Endpoint added".to_string(),
                    );
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                ctx.notify();
            }
            CustomEndpointModalEvent::SaveEndpoint {
                index,
                name,
                url,
                api_key,
                models,
            } => {
                if !Self::can_use_custom_inference_controls(ctx) {
                    self.hide_custom_endpoint_modal(ctx);
                    return;
                }
                ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.save_custom_endpoint(
                        *index,
                        name.clone(),
                        url.clone(),
                        api_key.clone(),
                        models.clone(),
                        ctx,
                    );
                });
                self.hide_custom_endpoint_modal(ctx);

                let window_id = ctx.window_id();
                crate::ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = crate::view_components::DismissibleToast::success(
                        "Endpoint saved".to_string(),
                    );
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                ctx.notify();
            }
            CustomEndpointModalEvent::RemoveEndpoint { index } => {
                if !Self::can_use_custom_inference_controls(ctx) {
                    self.hide_custom_endpoint_modal(ctx);
                    return;
                }
                self.hide_custom_endpoint_modal(ctx);
                self.show_remove_custom_endpoint_confirmation_dialog(*index, ctx);
            }
        }
    }

    fn show_remove_custom_endpoint_confirmation_dialog(
        &mut self,
        index: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        if !Self::can_use_custom_inference_controls(ctx) {
            return;
        }
        let endpoint = ApiKeyManager::as_ref(ctx)
            .keys()
            .custom_endpoints
            .get(index)
            .cloned();
        let Some(endpoint) = endpoint else {
            return;
        };

        let model_labels = endpoint
            .models
            .iter()
            .map(|model| model.alias.clone().unwrap_or_else(|| model.name.clone()))
            .filter(|s| !s.trim().is_empty())
            .collect();

        self.pending_remove_custom_endpoint_index = Some(index);
        self.remove_custom_endpoint_confirmation_dialog
            .update(ctx, |dialog, ctx| {
                dialog.show(index, endpoint.name.clone(), model_labels, ctx);
            });
        ctx.notify();
    }

    fn handle_remove_custom_endpoint_confirmation_dialog_event(
        &mut self,
        event: &RemoveCustomEndpointConfirmationDialogEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            RemoveCustomEndpointConfirmationDialogEvent::Cancel => {
                self.pending_remove_custom_endpoint_index = None;
                self.remove_custom_endpoint_confirmation_dialog
                    .update(ctx, |dialog, ctx| {
                        dialog.hide(ctx);
                    });
                ctx.notify();
            }
            RemoveCustomEndpointConfirmationDialogEvent::Confirm(index) => {
                if !Self::can_use_custom_inference_controls(ctx) {
                    self.pending_remove_custom_endpoint_index = None;
                    self.remove_custom_endpoint_confirmation_dialog
                        .update(ctx, |dialog, ctx| {
                            dialog.hide(ctx);
                        });
                    ctx.notify();
                    return;
                }
                ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.remove_custom_endpoint(*index, ctx);
                });
                self.pending_remove_custom_endpoint_index = None;
                self.remove_custom_endpoint_confirmation_dialog
                    .update(ctx, |dialog, ctx| {
                        dialog.hide(ctx);
                    });
                self.sync_custom_endpoint_buttons(ctx);

                let window_id = ctx.window_id();
                crate::ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = crate::view_components::DismissibleToast::success(
                        "Endpoint removed".to_string(),
                    );
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                ctx.notify();
            }
        }
    }

    /// Set the active subpage and rebuild the widget list to show only relevant widgets.
    pub fn set_active_subpage(&mut self, subpage: Option<AISubpage>, ctx: &mut ViewContext<Self>) {
        if self.active_subpage != subpage {
            self.active_subpage = subpage;
            self.page = Self::build_page(subpage, ctx);
            ctx.notify();
        }
    }

    fn build_page(subpage: Option<AISubpage>, ctx: &mut ViewContext<Self>) -> PageType<Self> {
        let ai_settings = AISettings::as_ref(ctx);

        let mut widgets: Vec<Box<dyn SettingsWidget<View = AISettingsPageView>>> = Vec::new();

        // When viewing a specific subpage, only include its widgets.
        // When subpage is None (legacy/backward-compat), show all widgets.
        match subpage {
            None => {
                // Full page: all widgets (legacy behavior)
                widgets.push(Box::new(GlobalAIWidget::default()));
                if !FeatureFlag::UsageBasedPricing.is_enabled() {
                    widgets.push(Box::new(UsageWidget::default()));
                }
                if ai_settings
                    .intelligent_autosuggestions_enabled_internal
                    .is_supported_on_current_platform()
                    || ai_settings
                        .prompt_suggestions_enabled_internal
                        .is_supported_on_current_platform()
                    || (FeatureFlag::PredictAMQueries.is_enabled()
                        && ai_settings
                            .natural_language_autosuggestions_enabled_internal
                            .is_supported_on_current_platform())
                    || (FeatureFlag::SharedBlockTitleGeneration.is_enabled()
                        && ai_settings
                            .shared_block_title_generation_enabled_internal
                            .is_supported_on_current_platform())
                    || (FeatureFlag::GitOperationsInCodeReview.is_enabled()
                        && ai_settings
                            .git_operations_autogen_enabled_internal
                            .is_supported_on_current_platform())
                {
                    widgets.push(Box::new(ActiveAIWidget::default()));
                }
                widgets.push(Box::new(AgentsWidget::default()));
                widgets.push(Box::new(AIInputWidget::default()));
                if MCPServersWidget::should_show_mcp() {
                    widgets.push(Box::new(MCPServersWidget::default()));
                }
                if FeatureFlag::AIRules.is_enabled() {
                    widgets.push(Box::new(AIFactWidget::default()));
                }
                if cfg!(feature = "voice_input")
                    && ai_settings
                        .voice_input_enabled_internal
                        .is_supported_on_current_platform()
                {
                    widgets.push(Box::new(VoiceWidget::default()));
                }
                widgets.push(Box::new(CloudHandoffWidget::default()));
                widgets.push(Box::new(CLIAgentWidget::default()));
                widgets.push(Box::new(ApiKeysWidget::new(ctx)));
                widgets.push(Box::new(AwsBedrockWidget::new(ctx)));
                widgets.push(Box::new(AgentAttributionWidget::default()));
                widgets.push(Box::new(OtherAIWidget::default()));
                if FeatureFlag::AgentModeComputerUse.is_enabled() {
                    widgets.push(Box::new(CloudAgentComputerUseWidget::default()));
                }
            }
            Some(AISubpage::WarpAgent) => {
                // Oz page: global toggle + Active AI + Input + Other
                widgets.push(Box::new(GlobalAIWidget::default()));
                if ai_settings
                    .intelligent_autosuggestions_enabled_internal
                    .is_supported_on_current_platform()
                    || ai_settings
                        .prompt_suggestions_enabled_internal
                        .is_supported_on_current_platform()
                    || (FeatureFlag::PredictAMQueries.is_enabled()
                        && ai_settings
                            .natural_language_autosuggestions_enabled_internal
                            .is_supported_on_current_platform())
                    || (FeatureFlag::SharedBlockTitleGeneration.is_enabled()
                        && ai_settings
                            .shared_block_title_generation_enabled_internal
                            .is_supported_on_current_platform())
                    || (FeatureFlag::GitOperationsInCodeReview.is_enabled()
                        && ai_settings
                            .git_operations_autogen_enabled_internal
                            .is_supported_on_current_platform())
                {
                    widgets.push(Box::new(ActiveAIWidget::default()));
                }
                widgets.push(Box::new(AIInputWidget::default()));
                let voice_supported = cfg!(feature = "voice_input")
                    && ai_settings
                        .voice_input_enabled_internal
                        .is_supported_on_current_platform();
                if voice_supported {
                    widgets.push(Box::new(VoiceWidget::default()));
                }
                widgets.push(Box::new(CloudHandoffWidget::default()));
                widgets.push(Box::new(ApiKeysWidget::new(ctx)));
                widgets.push(Box::new(AwsBedrockWidget::new(ctx)));
                widgets.push(Box::new(AgentAttributionWidget::default()));
                widgets.push(Box::new(OtherAIWidget::default()));
                if FeatureFlag::AgentModeComputerUse.is_enabled() {
                    widgets.push(Box::new(CloudAgentComputerUseWidget::default()));
                }
            }
            Some(AISubpage::Profiles) => {
                if !FeatureFlag::UsageBasedPricing.is_enabled() {
                    widgets.push(Box::new(UsageWidget::default()));
                }
                widgets.push(Box::new(AgentsWidget::default()));
            }
            Some(AISubpage::Knowledge) => {
                if FeatureFlag::AIRules.is_enabled() {
                    widgets.push(Box::new(AIFactWidget::default()));
                }
            }
            Some(AISubpage::ThirdPartyCLIAgents) => {
                widgets.push(Box::new(CLIAgentWidget::default()));
            }
        }

        // Subpage widgets render their own subheader-sized titles internally,
        // so we don't pass a page-level title to PageType.
        let title: Option<&str> = None;
        PageType::new_uncategorized(widgets, title)
    }

    fn handle_context_window_editor_event(
        &mut self,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Blurred | EditorEvent::Enter => {
                if !AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                    self.sync_context_window_editor(ctx, true);
                    return;
                }
                if let Some(cw) = Self::configurable_context_window(ctx) {
                    let buffer_text = self.context_window_editor.as_ref(ctx).buffer_text(ctx);
                    let cleaned: String = buffer_text
                        .chars()
                        .filter(|c| !c.is_whitespace() && *c != ',')
                        .collect();
                    if let Ok(parsed) = cleaned.parse::<u32>() {
                        let clamped = parsed.clamp(cw.min, cw.max);
                        if Some(clamped) != Self::current_context_window_display_value(ctx) {
                            AIExecutionProfilesModel::handle(ctx).update(
                                ctx,
                                |profiles_model, ctx| {
                                    let profile_id = *profiles_model.active_profile(None, ctx).id();
                                    profiles_model.set_context_window_limit(
                                        profile_id,
                                        Some(clamped),
                                        ctx,
                                    );
                                },
                            );
                        }
                    }
                }
                self.sync_context_window_editor(ctx, true);
                if let EditorEvent::Enter = event {
                    ctx.emit(AISettingsPageEvent::FocusModal);
                }
                ctx.notify();
            }
            EditorEvent::Escape => ctx.emit(AISettingsPageEvent::FocusModal),
            _ => {}
        }
    }

    fn active_profile_data(app: &AppContext) -> AIExecutionProfile {
        AIExecutionProfilesModel::as_ref(app)
            .active_profile(None, app)
            .data()
            .clone()
    }

    fn configurable_context_window(app: &AppContext) -> Option<LLMContextWindow> {
        Self::active_profile_data(app).configurable_context_window(app)
    }

    fn current_context_window_display_value(app: &AppContext) -> Option<u32> {
        Self::active_profile_data(app).context_window_display_value(app)
    }

    fn initial_context_window_value(app: &AppContext) -> u32 {
        Self::current_context_window_display_value(app).unwrap_or_else(|| {
            LLMPreferences::as_ref(app)
                .get_active_base_model(app, None)
                .context_window
                .default_max
        })
    }

    fn sync_context_window_editor(&mut self, ctx: &mut ViewContext<Self>, force: bool) {
        let Some(value) = Self::current_context_window_display_value(ctx) else {
            self.last_synced_context_window_editor_value = None;
            self.context_window_slider_state.reset_offset();
            ctx.notify();
            return;
        };

        let formatted = value.to_string();
        let should_update = if force {
            true
        } else {
            match self.last_synced_context_window_editor_value {
                Some(last_value) => {
                    self.context_window_editor.as_ref(ctx).buffer_text(ctx)
                        == last_value.to_string()
                }
                None => true,
            }
        };

        if should_update {
            self.context_window_editor.update(ctx, |editor, ctx| {
                if editor.buffer_text(ctx) != formatted {
                    editor.system_reset_buffer_text(&formatted, ctx);
                }
            });
            self.last_synced_context_window_editor_value = Some(value);
            self.context_window_slider_state.reset_offset();
            ctx.notify();
        }
    }

    fn handle_detection_denylist_editor_event(
        &mut self,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            EditorEvent::Blurred | EditorEvent::Enter => {
                let buffer_text = self
                    .autodetection_denylist_editor
                    .as_ref(ctx)
                    .buffer_text(ctx);
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    if let Err(e) = settings
                        .autodetection_command_denylist
                        .set_value(buffer_text, ctx)
                    {
                        log::warn!("Failed to set AI autodetection blacklist commands: {e:?}");
                    }
                })
            }
            EditorEvent::Escape => ctx.emit(AISettingsPageEvent::FocusModal),
            _ => {}
        }
    }

    fn update_editor_interaction_state(
        editor: ViewHandle<EditorView>,
        is_enabled: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        editor.update(ctx, |editor, ctx| {
            let interaction_state = if is_enabled {
                InteractionState::Editable
            } else {
                InteractionState::Disabled
            };
            editor.set_interaction_state(interaction_state, ctx);
            ctx.notify();
        })
    }

    pub fn refresh_base_model_menu(
        menu: &ViewHandle<Dropdown<AISettingsPageAction>>,
        ctx: &mut ViewContext<Self>,
    ) {
        menu.update(ctx, |menu, ctx| {
            let disabled_by_ai_toggle = !AISettings::as_ref(ctx).is_any_ai_enabled(ctx);

            if disabled_by_ai_toggle {
                menu.set_disabled(ctx);
            } else {
                menu.set_enabled(ctx);
            }

            let choices = LLMPreferences::as_ref(ctx)
                .get_base_llm_choices_for_agent_mode(ctx)
                .collect_vec();

            let items = available_model_menu_items(
                choices,
                |llm| AISettingsPageAction::SetBaseModel(llm.id.clone()).into(),
                None,
                None,
                false,
                false,
                ctx,
            );
            menu.set_rich_items(items, ctx);

            let active = LLMPreferences::as_ref(ctx).get_active_base_model(ctx, None);
            menu.set_selected_by_action(AISettingsPageAction::SetBaseModel(active.id.clone()), ctx);
            ctx.notify();
        });
        ctx.notify();
    }

    pub fn refresh_coding_model_menu(
        menu: &ViewHandle<Dropdown<AISettingsPageAction>>,
        ctx: &mut ViewContext<Self>,
    ) {
        menu.update(ctx, |menu, ctx| {
            let disabled_by_ai_toggle = !AISettings::as_ref(ctx).is_any_ai_enabled(ctx);

            if disabled_by_ai_toggle {
                menu.set_disabled(ctx);
            } else {
                menu.set_enabled(ctx);
            }

            let choices = LLMPreferences::as_ref(ctx)
                .get_coding_llm_choices(ctx)
                .collect_vec();

            let items = available_model_menu_items(
                choices,
                |llm| AISettingsPageAction::SetCodingModel(llm.id.clone()).into(),
                None,
                None,
                false,
                false,
                ctx,
            );
            menu.set_rich_items(items, ctx);
            let active = LLMPreferences::as_ref(ctx).get_active_coding_model(ctx, None);

            menu.set_selected_by_action(
                AISettingsPageAction::SetCodingModel(active.id.clone()),
                ctx,
            );
            ctx.notify();
        });
        ctx.notify();
    }

    fn refresh_autonomy_dropdown_menu(
        menu: &ViewHandle<Dropdown<AISettingsPageAction>>,
        ctx: &mut ViewContext<Self>,
    ) {
        menu.update(ctx, |menu, ctx| {
            if AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                menu.set_enabled(ctx);
            } else {
                menu.set_disabled(ctx);
            }

            menu.set_items(
                vec![
                    DropdownItem::new(
                        i18n::tr(ctx, I18nKey::AiReadOnly),
                        AISettingsPageAction::SetAutonomyReadonlyCommandsSetting,
                    ),
                    DropdownItem::new(
                        i18n::tr(ctx, I18nKey::AiSupervised),
                        AISettingsPageAction::SetAutonomySupervisedSetting,
                    ),
                ],
                ctx,
            );
            let active = if *AISettings::as_ref(ctx).agent_mode_execute_read_only_commands {
                0
            } else {
                1
            };
            menu.set_selected_by_index(active, ctx);
            ctx.notify();
        });
        ctx.notify();
    }

    fn refresh_all_execution_profile_ui(&self, ctx: &mut ViewContext<Self>) {
        let permissions = BlocklistAIPermissions::handle(ctx);

        let apply_code_diffs_setting = permissions
            .as_ref(ctx)
            .get_apply_code_diffs_setting(ctx, None);
        Self::refresh_execution_profile_dropdown_menu(
            &self.apply_code_diffs_dropdown_menu,
            apply_code_diffs_setting,
            !AISettings::as_ref(ctx).is_code_diffs_permissions_editable(ctx),
            ctx,
        );

        let read_files_setting = permissions.as_ref(ctx).get_read_files_setting(ctx, None);
        Self::refresh_execution_profile_dropdown_menu(
            &self.read_files_dropdown_menu,
            read_files_setting,
            !AISettings::as_ref(ctx).is_read_files_permissions_editable(ctx),
            ctx,
        );

        let execute_commands_setting: ActionPermission = permissions
            .as_ref(ctx)
            .get_execute_commands_setting(ctx, None);
        Self::refresh_execution_profile_dropdown_menu(
            &self.execute_commands_dropdown_menu,
            execute_commands_setting,
            !AISettings::as_ref(ctx).is_execute_commands_permissions_editable(ctx),
            ctx,
        );

        let write_to_pty_setting: WriteToPtyPermission =
            permissions.as_ref(ctx).get_write_to_pty_setting(ctx, None);
        Self::refresh_write_to_pty_dropdown_menu(
            &self.write_to_pty_autonomy_dropdown_menu,
            write_to_pty_setting,
            !AISettings::as_ref(ctx).is_write_to_pty_permissions_editable(ctx),
            ctx,
        );

        let mcp_permissions_setting = permissions
            .as_ref(ctx)
            .get_mcp_permissions_setting(ctx, None);
        Self::refresh_execution_profile_dropdown_menu(
            &self.mcp_permissions_dropdown_menu,
            mcp_permissions_setting,
            !AISettings::as_ref(ctx).is_mcp_permission_editable(ctx),
            ctx,
        );
        Self::refresh_mcp_allowlist_dropdown(&self.mcp_allowlist_dropdown, ctx);
        Self::refresh_mcp_denylist_dropdown(&self.mcp_denylist_dropdown, ctx);

        let is_any_ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
        self.add_profile_button.update(ctx, |button, ctx| {
            button.set_disabled(!is_any_ai_enabled, ctx);
        });
    }

    fn refresh_localized_controls(&self, ctx: &mut ViewContext<Self>) {
        self.voice_input_toggle_key_dropdown
            .update(ctx, |dropdown, ctx| {
                dropdown.set_items(voice_input_toggle_key_dropdown_items(ctx), ctx);
                let current_value = *AISettings::as_ref(ctx).voice_input_toggle_key.value();
                dropdown.set_selected_by_action(
                    AISettingsPageAction::SetVoiceInputToggleKey(current_value),
                    ctx,
                );
            });
        self.thinking_display_mode_dropdown
            .update(ctx, |dropdown, ctx| {
                dropdown.set_items(thinking_display_mode_dropdown_items(ctx), ctx);
                let current_mode = *AISettings::as_ref(ctx).thinking_display_mode.value();
                dropdown.set_selected_by_action(
                    AISettingsPageAction::SetThinkingDisplayMode(current_mode),
                    ctx,
                );
            });
        self.code_read_allowlist_editor.update(ctx, |input, ctx| {
            input.set_placeholder_text(i18n::tr(ctx, I18nKey::AiDirectoryPathPlaceholder), ctx);
        });
        self.directory_allowlist_editor.update(ctx, |input, ctx| {
            input.set_placeholder_text(i18n::tr(ctx, I18nKey::AiDirectoryPathPlaceholder), ctx);
        });
        self.command_execution_allowlist_editor
            .update(ctx, |input, ctx| {
                input.set_placeholder_text(i18n::tr(ctx, I18nKey::AiCommandAllowPlaceholder), ctx);
            });
        self.command_execution_denylist_editor
            .update(ctx, |input, ctx| {
                input.set_placeholder_text(i18n::tr(ctx, I18nKey::AiCommandDenyPlaceholder), ctx);
            });
        self.command_allowlist_editor.update(ctx, |input, ctx| {
            input.set_placeholder_text(i18n::tr(ctx, I18nKey::AiCommandAllowPlaceholder), ctx);
        });
        self.command_denylist_editor.update(ctx, |input, ctx| {
            input.set_placeholder_text(i18n::tr(ctx, I18nKey::AiCommandDenyPlaceholder), ctx);
        });
        self.cli_agent_footer_command_editor
            .update(ctx, |input, ctx| {
                input.set_placeholder_text(i18n::tr(ctx, I18nKey::AiCommandRegexPlaceholder), ctx);
            });
        self.autodetection_denylist_editor
            .update(ctx, |editor, ctx| {
                editor.set_placeholder_text(
                    i18n::tr(ctx, I18nKey::AiCommandsCommaSeparatedPlaceholder),
                    ctx,
                );
            });
        self.refresh_cli_agent_dropdowns(ctx);
        Self::refresh_autonomy_dropdown_menu(&self.autonomy_dropdown_menu, ctx);
        Self::refresh_code_read_autonomy_dropdown_menu(&self.code_read_autonomy_dropdown_menu, ctx);
        self.apply_code_diffs_dropdown_menu
            .update(ctx, |dropdown, ctx| {
                dropdown.set_items(
                    action_permission_dropdown_items(ctx, AISettingsPageAction::SetApplyCodeDiffs),
                    ctx,
                );
            });
        self.read_files_dropdown_menu.update(ctx, |dropdown, ctx| {
            dropdown.set_items(
                action_permission_dropdown_items(ctx, AISettingsPageAction::SetReadFiles),
                ctx,
            );
        });
        self.execute_commands_dropdown_menu
            .update(ctx, |dropdown, ctx| {
                dropdown.set_items(
                    action_permission_dropdown_items(ctx, AISettingsPageAction::SetExecuteCommands),
                    ctx,
                );
            });
        self.write_to_pty_autonomy_dropdown_menu
            .update(ctx, |dropdown, ctx| {
                dropdown.set_items(write_to_pty_dropdown_items(ctx), ctx);
            });
        self.mcp_permissions_dropdown_menu
            .update(ctx, |dropdown, ctx| {
                dropdown.set_items(
                    action_permission_dropdown_items(ctx, AISettingsPageAction::SetMCPPermissions),
                    ctx,
                );
            });
        self.mcp_allowlist_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_menu_header_to_static(i18n::tr(ctx, I18nKey::AiSelectMcpServers));
        });
        self.mcp_denylist_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_menu_header_to_static(i18n::tr(ctx, I18nKey::AiSelectMcpServers));
        });
        self.add_profile_button.update(ctx, |button, ctx| {
            button.set_label(i18n::tr(ctx, I18nKey::AiAddProfile), ctx);
        });
        #[cfg(feature = "local_fs")]
        self.conversation_layout_dropdown
            .update(ctx, |dropdown, ctx| {
                use crate::util::file::external_editor::settings::OpenConversationPreference;

                dropdown.set_items(
                    vec![
                        DropdownItem::new(
                            i18n::tr(ctx, I18nKey::CodeExternalNewTab),
                            AISettingsPageAction::SetConversationLayout(
                                OpenConversationPreference::NewTab,
                            ),
                        ),
                        DropdownItem::new(
                            i18n::tr(ctx, I18nKey::CodeExternalSplitPane),
                            AISettingsPageAction::SetConversationLayout(
                                OpenConversationPreference::SplitPane,
                            ),
                        ),
                    ],
                    ctx,
                );
                let current = *crate::util::file::external_editor::EditorSettings::as_ref(ctx)
                    .open_conversation_layout_preference;
                match current {
                    OpenConversationPreference::NewTab => dropdown
                        .set_selected_by_name(i18n::tr(ctx, I18nKey::CodeExternalNewTab), ctx),
                    OpenConversationPreference::SplitPane => dropdown
                        .set_selected_by_name(i18n::tr(ctx, I18nKey::CodeExternalSplitPane), ctx),
                };
            });
        self.refresh_all_execution_profile_ui(ctx);
    }

    fn refresh_cli_agent_dropdowns(&self, ctx: &mut ViewContext<Self>) {
        let entries: Vec<(String, CLIAgent)> = AISettings::as_ref(ctx)
            .cli_agent_footer_enabled_commands
            .value()
            .iter()
            .map(|(pattern, agent_value)| {
                (pattern.clone(), CLIAgent::from_serialized_name(agent_value))
            })
            .collect();

        for (dropdown_handle, (pattern, current_agent)) in self
            .cli_agent_footer_command_agent_dropdowns
            .iter()
            .zip(entries.into_iter())
        {
            dropdown_handle.update(ctx, |dropdown, ctx| {
                let mut items: Vec<MenuItem<DropdownAction<AISettingsPageAction>>> = Vec::new();

                for agent in all::<CLIAgent>() {
                    if matches!(agent, CLIAgent::Unknown) {
                        continue;
                    }
                    let icon = agent.icon();
                    let mut fields = MenuItemFields::new(agent.display_name())
                        .with_on_select_action(DropdownAction::SelectActionAndClose(
                            AISettingsPageAction::SetCLIAgentForCommand {
                                pattern: pattern.clone(),
                                agent: Some(agent),
                            },
                        ));
                    if let Some(icon) = icon {
                        fields = fields.with_icon(icon);
                    }
                    items.push(fields.into_item());
                }

                let other_label = i18n::tr(ctx, I18nKey::AiOther).to_string();
                items.push(
                    MenuItemFields::new(other_label.clone())
                        .with_on_select_action(DropdownAction::SelectActionAndClose(
                            AISettingsPageAction::SetCLIAgentForCommand {
                                pattern: pattern.clone(),
                                agent: None,
                            },
                        ))
                        .into_item(),
                );

                dropdown.set_rich_items(items, ctx);

                let header_other_label = other_label.clone();
                let select_coding_agent_label =
                    i18n::tr(ctx, I18nKey::AiSelectCodingAgent).to_string();
                dropdown.set_menu_header_text_override(move |label| {
                    if label == header_other_label {
                        select_coding_agent_label.clone()
                    } else {
                        label.to_string()
                    }
                });

                let selected_name = if matches!(current_agent, CLIAgent::Unknown) {
                    other_label
                } else {
                    current_agent.display_name().to_string()
                };
                dropdown.set_selected_by_name(&selected_name, ctx);
            });
        }
    }

    fn reset_execution_profile_mouse_state_handles(&mut self, ctx: &mut ViewContext<Self>) {
        let blocklist_permissions = BlocklistAIPermissions::as_ref(ctx);

        self.directory_allowlist_mouse_state_handles = blocklist_permissions
            .get_read_files_allowlist(ctx, None)
            .iter()
            .map(|_| Default::default())
            .collect();

        self.command_denylist_mouse_state_handles = blocklist_permissions
            .get_execute_commands_denylist(ctx, None)
            .iter()
            .map(|_| Default::default())
            .collect();

        let org_denylist = BlocklistAIPermissions::get_org_execute_commands_denylist(ctx);
        self.command_denylist_tooltip_mouse_state_handles =
            org_denylist.iter().map(|_| Default::default()).collect();

        self.command_allowlist_mouse_state_handles = blocklist_permissions
            .get_execute_commands_allowlist(ctx, None)
            .iter()
            .map(|_| Default::default())
            .collect();

        self.mcp_allowlist_mouse_state_handles = blocklist_permissions
            .get_mcp_allowlist(ctx, None)
            .iter()
            .map(|_| Default::default())
            .collect();

        self.mcp_denylist_mouse_state_handles = blocklist_permissions
            .get_mcp_denylist(ctx, None)
            .iter()
            .map(|_| Default::default())
            .collect();
    }

    fn refresh_execution_profile_dropdown_menu(
        menu: &ViewHandle<Dropdown<AISettingsPageAction>>,
        current_permission: ActionPermission,
        disabled: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        menu.update(ctx, |menu, ctx| {
            if !disabled {
                menu.set_enabled(ctx);
            } else {
                menu.set_disabled(ctx);
            }

            let active = match current_permission {
                ActionPermission::AgentDecides | ActionPermission::Unknown => 0,
                ActionPermission::AlwaysAllow => 1,
                ActionPermission::AlwaysAsk => 2,
            };

            menu.set_selected_by_index(active, ctx);
            ctx.notify();
        });
        ctx.notify();
    }

    fn refresh_write_to_pty_dropdown_menu(
        menu: &ViewHandle<Dropdown<AISettingsPageAction>>,
        current_permission: WriteToPtyPermission,
        disabled: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        menu.update(ctx, |menu, ctx| {
            if !disabled {
                menu.set_enabled(ctx);
            } else {
                menu.set_disabled(ctx);
            }

            let active = match current_permission {
                WriteToPtyPermission::AlwaysAllow => 0,
                WriteToPtyPermission::AlwaysAsk | WriteToPtyPermission::Unknown => 1,
                WriteToPtyPermission::AskOnFirstWrite => 2,
            };

            menu.set_selected_by_index(active, ctx);
            ctx.notify();
        });
        ctx.notify();
    }

    /// Currently, the coding permissions only support "read" access.
    fn refresh_code_read_autonomy_dropdown_menu(
        menu: &ViewHandle<Dropdown<AISettingsPageAction>>,
        ctx: &mut ViewContext<Self>,
    ) {
        menu.update(ctx, |menu, ctx| {
            if AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                menu.set_enabled(ctx);
            } else {
                menu.set_disabled(ctx);
            }

            menu.set_items(
                AgentModeCodingPermissionsType::iter()
                    .map(|t| {
                        let display = match t {
                            AgentModeCodingPermissionsType::AlwaysAskBeforeReading => {
                                i18n::tr(ctx, I18nKey::AiAlwaysAsk)
                            }
                            AgentModeCodingPermissionsType::AlwaysAllowReading => {
                                i18n::tr(ctx, I18nKey::AiAlwaysAllow)
                            }
                            AgentModeCodingPermissionsType::AllowReadingSpecificFiles => {
                                i18n::tr(ctx, I18nKey::AiAllowSpecificDirectories)
                            }
                        };
                        DropdownItem::new(display, AISettingsPageAction::SetCodingPermission(t))
                    })
                    .collect(),
                ctx,
            );
            let ai_settings = AISettings::as_ref(ctx);

            let active = if *ai_settings.agent_mode_execute_read_only_commands {
                menu.set_disabled(ctx);
                AgentModeCodingPermissionsType::AlwaysAllowReading
            } else {
                *ai_settings.agent_mode_coding_permissions
            };
            menu.set_selected_by_action(AISettingsPageAction::SetCodingPermission(active), ctx);
            ctx.notify();
        });
        ctx.notify();
    }

    fn get_non_allowlisted_or_denylisted_mcp_servers(
        ctx: &mut ViewContext<Self>,
    ) -> Vec<(uuid::Uuid, String)> {
        let all_mcp_servers = TemplatableMCPServerManager::get_all_cloud_synced_mcp_servers(ctx);
        let already_allowlisted_mcp_servers =
            BlocklistAIPermissions::as_ref(ctx).get_mcp_allowlist(ctx, None);
        let already_denylisted_mcp_servers =
            BlocklistAIPermissions::as_ref(ctx).get_mcp_denylist(ctx, None);

        all_mcp_servers
            .into_iter()
            .filter(|(uuid, _)| {
                let is_allowlisted = already_allowlisted_mcp_servers.contains(uuid);
                let is_denylisted = already_denylisted_mcp_servers.contains(uuid);
                !is_allowlisted && !is_denylisted
            })
            .collect()
    }

    fn refresh_menu_dropdown<F>(
        menu: &ViewHandle<FilterableDropdown<AISettingsPageAction>>,
        action_fn: F,
        ctx: &mut ViewContext<Self>,
    ) where
        F: Fn(uuid::Uuid) -> AISettingsPageAction,
    {
        let mcps_in_dropdown = Self::get_non_allowlisted_or_denylisted_mcp_servers(ctx);
        menu.update(ctx, |menu, ctx| {
            if AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                menu.set_enabled(ctx);
            } else {
                menu.set_disabled(ctx);
            }

            let items: Vec<DropdownItem<AISettingsPageAction>> = mcps_in_dropdown
                .iter()
                .map(|(uuid, server_name)| DropdownItem::new(server_name, action_fn(*uuid)))
                .collect();

            menu.set_items(items, ctx);
            ctx.notify()
        });
        ctx.notify();
    }

    fn refresh_mcp_allowlist_dropdown(
        menu: &ViewHandle<FilterableDropdown<AISettingsPageAction>>,
        ctx: &mut ViewContext<Self>,
    ) {
        Self::refresh_menu_dropdown(menu, AISettingsPageAction::AddToMCPAllowlist, ctx);
    }

    fn create_profile_views(ctx: &mut ViewContext<Self>) -> Vec<ViewHandle<ExecutionProfileView>> {
        let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
        let profile_ids = profiles_model.get_all_profile_ids();

        profile_ids
            .iter()
            .map(|&profile_id| {
                let profile_view =
                    ctx.add_typed_action_view(|ctx| ExecutionProfileView::new(profile_id, ctx));

                ctx.subscribe_to_view(&profile_view, move |_me, _, event, ctx| match event {
                    ExecutionProfileViewEvent::EditProfile => {
                        ctx.emit(AISettingsPageEvent::OpenExecutionProfileEditor(profile_id));
                    }
                });

                profile_view
            })
            .collect()
    }

    fn refresh_profile_views(&mut self, ctx: &mut ViewContext<Self>) {
        let new_profile_views = Self::create_profile_views(ctx);
        self.profile_views = new_profile_views;
    }

    fn refresh_mcp_denylist_dropdown(
        menu: &ViewHandle<FilterableDropdown<AISettingsPageAction>>,
        ctx: &mut ViewContext<Self>,
    ) {
        Self::refresh_menu_dropdown(menu, AISettingsPageAction::AddToMCPDenylist, ctx);
    }

    fn create_cli_agent_dropdowns(
        ctx: &mut ViewContext<Self>,
    ) -> Vec<ViewHandle<Dropdown<AISettingsPageAction>>> {
        let entries: Vec<(String, CLIAgent)> = AISettings::as_ref(ctx)
            .cli_agent_footer_enabled_commands
            .value()
            .iter()
            .map(|(pattern, agent_value)| {
                (pattern.clone(), CLIAgent::from_serialized_name(agent_value))
            })
            .collect();

        entries
            .into_iter()
            .map(|(pattern_clone, current_agent)| {
                ctx.add_typed_action_view(move |ctx| {
                    let mut dropdown = Dropdown::new(ctx);
                    dropdown.set_top_bar_max_width(160.);
                    dropdown.set_menu_width(180., ctx);
                    dropdown.set_main_axis_size(MainAxisSize::Min, ctx);

                    let mut items: Vec<MenuItem<DropdownAction<AISettingsPageAction>>> = Vec::new();

                    for agent in all::<CLIAgent>() {
                        if matches!(agent, CLIAgent::Unknown) {
                            continue;
                        }
                        let icon = agent.icon();
                        let mut fields = MenuItemFields::new(agent.display_name())
                            .with_on_select_action(DropdownAction::SelectActionAndClose(
                                AISettingsPageAction::SetCLIAgentForCommand {
                                    pattern: pattern_clone.clone(),
                                    agent: Some(agent),
                                },
                            ));
                        if let Some(icon) = icon {
                            fields = fields.with_icon(icon);
                        }
                        items.push(fields.into_item());
                    }

                    let other_label = i18n::tr(ctx, I18nKey::AiOther).to_string();
                    items.push(
                        MenuItemFields::new(other_label.clone())
                            .with_on_select_action(DropdownAction::SelectActionAndClose(
                                AISettingsPageAction::SetCLIAgentForCommand {
                                    pattern: pattern_clone.clone(),
                                    agent: None,
                                },
                            ))
                            .into_item(),
                    );

                    dropdown.set_rich_items(items, ctx);

                    let header_other_label = other_label.clone();
                    let select_coding_agent_label =
                        i18n::tr(ctx, I18nKey::AiSelectCodingAgent).to_string();
                    dropdown.set_menu_header_text_override(move |label| {
                        if label == header_other_label {
                            select_coding_agent_label.clone()
                        } else {
                            label.to_string()
                        }
                    });

                    let selected_name = if matches!(current_agent, CLIAgent::Unknown) {
                        other_label
                    } else {
                        current_agent.display_name().to_string()
                    };
                    dropdown.set_selected_by_name(&selected_name, ctx);

                    dropdown
                })
            })
            .collect()
    }
}

impl View for AISettingsPageView {
    fn ui_name() -> &'static str {
        "AISettingsPage"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        self.page.render(self, app)
    }
}

pub enum AISettingsPageEvent {
    FocusModal,
    OpenAIFactCollection,
    OpenMCPServerCollection,
    OpenExecutionProfileEditor(ClientProfileId),
    SignupAnonymousUser,
    ShowModal,
    HideModal,
}

impl Entity for AISettingsPageView {
    type Event = AISettingsPageEvent;
}

#[derive(Debug, Clone, PartialEq)]
pub enum AISettingsPageAction {
    OpenUrl(String),
    SetVoiceInputToggleKey(VoiceInputToggleKey),
    ToggleGlobalAI,
    ToggleActiveAI,
    ToggleIntelligentAutosuggestions,
    TogglePromptSuggestions,
    ToggleCodeSuggestions,
    ToggleNaturalLanguageAutosuggestions,
    ToggleSharedTitleGeneration,
    ToggleGitOperationsAutogen,
    ToggleAIInputAutoDetection,
    ToggleNLDInTerminal,
    ToggleCLIAgentToolbar,
    ToggleUseAgentToolbar,
    ToggleVoiceInput,
    ToggleCanUseWarpCreditsForFallback,
    HyperlinkClick(HyperlinkUrl),
    ToggleCodebaseContext,
    ToggleShowInputHintText,
    ToggleShowAgentTips,
    ToggleShowOzUpdatesInZeroState,
    SetThinkingDisplayMode(ThinkingDisplayMode),
    AttemptLoginGatedUpgrade,
    RemoveCLIAgentToolbarEnabledCommand(String),
    RemoveFromCommandExecutionAllowlist(AgentModeCommandExecutionPredicate),
    RemoveFromCommandExecutionDenylist(AgentModeCommandExecutionPredicate),
    OpenAIFactCollection,
    OpenMCPServerCollection,
    OpenExecutionProfileEditor(ClientProfileId),
    SetBaseModel(LLMId),
    SetCodingModel(LLMId),
    /// Called while the user is actively dragging the context window slider.
    ContextWindowSliderDragged(u32),
    /// Called when the user commits a new context window value (slider drop or
    /// input box commit).
    SetContextWindowSize(u32),
    SetAutonomyReadonlyCommandsSetting,
    SetAutonomySupervisedSetting,
    SetCodingPermission(AgentModeCodingPermissionsType),
    RemoveDirectoryFromCodeReadAllowlist(PathBuf),
    ToggleRules,
    ToggleRuleSuggestions,
    ToggleWarpDriveContext,
    SetApplyCodeDiffs(ActionPermission),
    SetReadFiles(ActionPermission),
    SetExecuteCommands(ActionPermission),
    SetWriteToPty(WriteToPtyPermission),
    SetMCPPermissions(ActionPermission),
    RemoveFromProfileDirectoryAllowlist(PathBuf),
    RemoveFromProfileCommandDenylist(AgentModeCommandExecutionPredicate),
    RemoveFromProfileCommandAllowlist(AgentModeCommandExecutionPredicate),
    ToggleShowBaseModelPickerInPrompt,
    AddToMCPAllowlist(uuid::Uuid),
    RemoveFromMCPAllowlist(uuid::Uuid),
    AddToMCPDenylist(uuid::Uuid),
    RemoveFromMCPDenylist(uuid::Uuid),
    CreateProfile,
    SignupAnonymousUser,
    ToggleAwsBedrockAutoLogin,
    ToggleAwsBedrockCredentialsEnabled,
    RefreshAwsBedrockCredentials,
    ToggleCloudAgentComputerUse,
    ToggleFileBasedMcp,
    ToggleIncludeAgentCommandsInHistory,
    ToggleAgentAttribution,
    ToggleFeedbackBundledSkill,

    // Custom inference
    OpenAddCustomEndpointModal,
    OpenEditCustomEndpointModal(usize),

    #[cfg(feature = "local_fs")]
    SetConversationLayout(crate::util::file::external_editor::settings::OpenConversationPreference),
    ToggleCloudHandoff,
    ToggleAmpersandHandoff,
    ToggleAutoHandoffOnSleep,
    ToggleShowConversationHistory,
    ToggleAutoToggleRichInput,
    ToggleAutoOpenRichInputOnCLIAgentStart,
    ToggleAutoDismissRichInputAfterSubmit,
    SetCLIAgentForCommand {
        pattern: String,
        agent: Option<CLIAgent>,
    },
}

impl From<&AISettingsPageAction> for LoginGatedFeature {
    fn from(val: &AISettingsPageAction) -> LoginGatedFeature {
        use AISettingsPageAction::*;
        match val {
            AttemptLoginGatedUpgrade => "Upgrade AI Usage",
            _ => "Unknown reason",
        }
    }
}

impl TypedActionView for AISettingsPageView {
    type Action = AISettingsPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AISettingsPageAction::OpenUrl(url) => {
                ctx.open_url(url.as_str());
            }
            AISettingsPageAction::SetVoiceInputToggleKey(key) => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.voice_input_toggle_key.set_value(*key, ctx));
                    report_if_error!(settings
                        .explicitly_interacted_with_voice
                        .set_value(true, ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::ToggleGlobalAI => {
                match AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings.is_any_ai_enabled.toggle_and_save_value(ctx)
                }) {
                    Ok(new_value) => {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::ToggleGlobalAI {
                                is_ai_enabled: new_value,
                            },
                            ctx
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to set value for Global AI setting: {e:?}");
                    }
                }
                ctx.notify();
            }
            AISettingsPageAction::ToggleActiveAI => {
                match AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .is_active_ai_enabled_internal
                        .toggle_and_save_value(ctx)
                }) {
                    Ok(new_value) => {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::ToggleActiveAI {
                                is_active_ai_enabled: new_value,
                            },
                            ctx
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to set value for Active AI setting: {e:?}");
                    }
                }
                ctx.notify();
            }
            AISettingsPageAction::ToggleIntelligentAutosuggestions => {
                match AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .intelligent_autosuggestions_enabled_internal
                        .toggle_and_save_value(ctx)
                }) {
                    Ok(new_value) => {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::ToggleIntelligentAutosuggestionsSetting {
                                is_intelligent_autosuggestions_enabled: new_value,
                            },
                            ctx
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to set value for Next Command setting: {e:?}");
                    }
                }
                ctx.notify();
            }
            AISettingsPageAction::TogglePromptSuggestions => {
                if !UserWorkspaces::as_ref(ctx).is_prompt_suggestions_toggleable() {
                    return;
                }
                match AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .prompt_suggestions_enabled_internal
                        .toggle_and_save_value(ctx)
                }) {
                    Ok(new_value) => {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::TogglePromptSuggestionsSetting {
                                is_prompt_suggestions_enabled: new_value,
                            },
                            ctx
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to set value for Prompt Suggestions setting: {e:?}");
                    }
                }
                ctx.notify();
            }
            AISettingsPageAction::ToggleCodeSuggestions => {
                match AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .code_suggestions_enabled_internal
                        .toggle_and_save_value(ctx)
                }) {
                    Ok(new_value) => {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::ToggleCodeSuggestionsSetting {
                                source: ToggleCodeSuggestionsSettingSource::Settings,
                                is_code_suggestions_enabled: new_value,
                            },
                            ctx
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to set value for Code Suggestions setting: {e:?}");
                    }
                }
                ctx.notify();
            }
            AISettingsPageAction::ToggleNaturalLanguageAutosuggestions => {
                match AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .natural_language_autosuggestions_enabled_internal
                        .toggle_and_save_value(ctx)
                }) {
                    Ok(new_value) => {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::ToggleNaturalLanguageAutosuggestionsSetting {
                                is_natural_language_autosuggestions_enabled: new_value,
                            },
                            ctx
                        );
                    }
                    Err(e) => {
                        log::warn!(
                            "Failed to set value for Natural Language Autosuggestions setting: {e:?}"
                        );
                    }
                }
                ctx.notify();
            }
            AISettingsPageAction::ToggleSharedTitleGeneration => {
                match AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .shared_block_title_generation_enabled_internal
                        .toggle_and_save_value(ctx)
                }) {
                    Ok(_new_value) => {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::ToggleSharedBlockTitleGenerationSetting {
                                is_shared_block_title_generation_enabled: true,
                            },
                            ctx
                        );
                    }
                    Err(e) => {
                        log::warn!(
                            "Failed to set value for Shared Block Title Generation setting: {e:?}"
                        );
                    }
                }
                ctx.notify();
            }
            AISettingsPageAction::ToggleGitOperationsAutogen => {
                if !UserWorkspaces::as_ref(ctx).is_git_operations_ai_enabled() {
                    return;
                }
                match AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .git_operations_autogen_enabled_internal
                        .toggle_and_save_value(ctx)
                }) {
                    Ok(new_value) => {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::ToggleGitOperationsAutogenSetting {
                                is_git_operations_autogen_enabled: new_value,
                            },
                            ctx
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to set value for Git Operations Autogen setting: {e:?}");
                    }
                }
                ctx.notify();
            }
            AISettingsPageAction::ToggleAIInputAutoDetection => {
                match AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .ai_autodetection_enabled_internal
                        .toggle_and_save_value(ctx)
                }) {
                    Ok(new_value) => {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::AgentModeToggleAutoDetectionSetting {
                                is_autodetection_enabled: new_value,
                                origin: AgentModeAutoDetectionSettingOrigin::SettingsPage
                            },
                            ctx
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to set value for Input Auto-detection: {e:?}");
                    }
                }
                ctx.notify();
            }
            AISettingsPageAction::ToggleNLDInTerminal => {
                match AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .nld_in_terminal_enabled_internal
                        .toggle_and_save_value(ctx)
                }) {
                    Ok(_new_value) => {}
                    Err(e) => {
                        log::warn!("Failed to set value for NLD in Terminal: {e:?}");
                    }
                }
                ctx.notify();
            }
            AISettingsPageAction::ToggleCLIAgentToolbar => {
                match AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .should_render_cli_agent_footer
                        .toggle_and_save_value(ctx)
                }) {
                    Ok(new_value) => {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::ToggleCLIAgentToolbarSetting {
                                is_enabled: new_value,
                            },
                            ctx
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to set value for CLI Agent Footer setting: {e:?}");
                    }
                }
                ctx.notify();
            }
            AISettingsPageAction::ToggleAutoToggleRichInput => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.auto_toggle_rich_input.toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::ToggleAutoOpenRichInputOnCLIAgentStart => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .auto_open_rich_input_on_cli_agent_start
                        .toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::ToggleAutoDismissRichInputAfterSubmit => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .auto_dismiss_rich_input_after_submit
                        .toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::ToggleUseAgentToolbar => {
                match AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .should_render_use_agent_footer_for_user_commands
                        .toggle_and_save_value(ctx)
                }) {
                    Ok(new_value) => {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::ToggleUseAgentToolbarSetting {
                                is_enabled: new_value,
                            },
                            ctx
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to set value for Use Agent Footer setting: {e:?}");
                    }
                }
                ctx.notify();
            }
            AISettingsPageAction::ToggleCodebaseContext => {
                match CodeSettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings.codebase_context_enabled.toggle_and_save_value(ctx)
                }) {
                    Ok(new_value) => {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::ToggleCodebaseContext {
                                is_codebase_context_enabled: new_value
                            },
                            ctx
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to set value for Codebase Context: {e:?}");
                    }
                }
                ctx.notify();
            }
            AISettingsPageAction::ToggleVoiceInput => {
                match AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .voice_input_enabled_internal
                        .toggle_and_save_value(ctx)
                }) {
                    Ok(new_value) => {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::ToggleVoiceInputSetting {
                                is_voice_input_enabled: new_value,
                            },
                            ctx
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to set value for Voice Input: {e:?}");
                    }
                }
                ctx.notify();
            }
            AISettingsPageAction::ToggleCanUseWarpCreditsForFallback => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .can_use_warp_credits_for_fallback
                        .toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::HyperlinkClick(hyperlink) => {
                ctx.notify();
                ctx.open_url(&hyperlink.url);
            }
            AISettingsPageAction::ToggleShowInputHintText => {
                InputSettings::handle(ctx).update(ctx, |input_settings, ctx| {
                    report_if_error!(input_settings.show_hint_text.toggle_and_save_value(ctx));
                    send_telemetry_from_ctx!(
                        // We purposely keep the FeaturesPageAction event, even though we have moved the setting to AI settings.
                        TelemetryEvent::FeaturesPageAction {
                            action: "ToggleShowInputHintText".to_string(),
                            value: format!("{}", *input_settings.show_hint_text),
                        },
                        ctx
                    );
                });
            }
            AISettingsPageAction::ToggleShowAgentTips => {
                InputSettings::handle(ctx).update(ctx, |input_settings, ctx| match input_settings
                    .show_agent_tips
                    .toggle_and_save_value(ctx)
                {
                    Ok(new_value) => {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::ToggleShowAgentTips {
                                is_enabled: new_value,
                            },
                            ctx
                        );
                    }
                    Err(e) => {
                        log::warn!("Failed to set value for Show Agent Tips setting: {e:?}");
                    }
                });
                ctx.notify();
            }
            AISettingsPageAction::ToggleShowOzUpdatesInZeroState => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .should_show_oz_updates_in_zero_state
                        .toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::SetThinkingDisplayMode(mode) => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.thinking_display_mode.set_value(*mode, ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::AttemptLoginGatedUpgrade => {
                AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
                    auth_manager.attempt_login_gated_feature(
                        action.into(),
                        AuthViewVariant::RequireLoginCloseable,
                        ctx,
                    )
                });
            }
            AISettingsPageAction::RemoveCLIAgentToolbarEnabledCommand(command) => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings.remove_cli_agent_footer_enabled_command(command, ctx);
                });
            }
            AISettingsPageAction::SetCLIAgentForCommand { pattern, agent } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings.set_cli_agent_for_command(pattern, *agent, ctx);
                });
            }
            AISettingsPageAction::RemoveFromCommandExecutionAllowlist(cmd) => {
                BlocklistAIPermissions::handle(ctx).update(ctx, |model, ctx| {
                    report_if_error!(model.remove_command_from_autoexecution_allowlist(cmd, ctx));
                })
            }
            AISettingsPageAction::RemoveFromCommandExecutionDenylist(cmd) => {
                BlocklistAIPermissions::handle(ctx).update(ctx, |model, ctx| {
                    report_if_error!(model.remove_command_from_denylist(cmd, ctx));
                })
            }
            AISettingsPageAction::OpenAIFactCollection => {
                ctx.emit(AISettingsPageEvent::OpenAIFactCollection)
            }
            AISettingsPageAction::OpenMCPServerCollection => {
                ctx.emit(AISettingsPageEvent::OpenMCPServerCollection)
            }
            AISettingsPageAction::OpenExecutionProfileEditor(profile_id) => {
                ctx.emit(AISettingsPageEvent::OpenExecutionProfileEditor(*profile_id))
            }
            AISettingsPageAction::SetBaseModel(id) => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    let profile_id = *profiles_model.active_profile(None, ctx).id();
                    profiles_model.set_base_model(profile_id, Some(id.clone()), ctx);
                    profiles_model.set_context_window_limit(profile_id, None, ctx);
                });
                self.sync_context_window_editor(ctx, true);
                ctx.notify();
            }
            AISettingsPageAction::SetCodingModel(id) => {
                LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
                    prefs.update_preferred_coding_llm(id, None, ctx);
                });
            }
            AISettingsPageAction::ContextWindowSliderDragged(value) => {
                if !AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                    self.sync_context_window_editor(ctx, true);
                    return;
                }
                if Self::configurable_context_window(ctx).is_some() {
                    let formatted = value.to_string();
                    self.context_window_editor.update(ctx, |editor, ctx| {
                        editor.system_reset_buffer_text(&formatted, ctx);
                    });
                    ctx.notify();
                }
            }
            AISettingsPageAction::SetContextWindowSize(value) => {
                if !AISettings::as_ref(ctx).is_any_ai_enabled(ctx) {
                    self.sync_context_window_editor(ctx, true);
                    return;
                }
                let Some(cw) = Self::configurable_context_window(ctx) else {
                    return;
                };
                let clamped = (*value).clamp(cw.min, cw.max);
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    let profile_id = *profiles_model.active_profile(None, ctx).id();
                    profiles_model.set_context_window_limit(profile_id, Some(clamped), ctx);
                });
                self.sync_context_window_editor(ctx, true);
                ctx.notify();
            }
            AISettingsPageAction::SetAutonomyReadonlyCommandsSetting
            | AISettingsPageAction::SetAutonomySupervisedSetting => {
                let readonly_cmd_execution_enabled = matches!(
                    action,
                    AISettingsPageAction::SetAutonomyReadonlyCommandsSetting
                );
                BlocklistAIPermissions::handle(ctx).update(ctx, |model, ctx| {
                    match model.set_should_autoexecute_readonly_commands(
                        readonly_cmd_execution_enabled,
                        ctx,
                    ) {
                        Ok(_) => {
                            send_telemetry_from_ctx!(
                                TelemetryEvent::ToggledAgentModeAutoexecuteReadonlyCommandsSetting {
                                    src: AutonomySettingToggleSource::SettingsPage,
                                    enabled: readonly_cmd_execution_enabled,
                                },
                                ctx);
                        }
                        Err(e) => report_error!(e),
                    }
                });
            }
            AISettingsPageAction::SetCodingPermission(p) => {
                BlocklistAIPermissions::handle(ctx).update(ctx, |model, ctx| {
                    match model.set_coding_permissions(*p, ctx) {
                        Ok(_) => {
                            send_telemetry_from_ctx!(
                                TelemetryEvent::ChangedAgentModeCodingPermissions {
                                    src: AutonomySettingToggleSource::SettingsPage,
                                    new: *p,
                                },
                                ctx
                            );
                        }
                        Err(e) => report_error!(e),
                    }
                });
            }
            AISettingsPageAction::SetApplyCodeDiffs(permission) => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                    let profile = model.default_profile(ctx);
                    model.set_apply_code_diffs(*profile.id(), permission, ctx);
                });
                ctx.notify();
            }
            AISettingsPageAction::SetReadFiles(permission) => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                    let profile = model.default_profile(ctx);
                    model.set_read_files(*profile.id(), permission, ctx);
                });
                ctx.notify();
            }
            AISettingsPageAction::SetExecuteCommands(permission) => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                    let profile = model.default_profile(ctx);
                    model.set_execute_commands(*profile.id(), permission, ctx);
                });
                ctx.notify();
            }
            AISettingsPageAction::SetWriteToPty(permission) => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                    let profile = model.default_profile(ctx);
                    model.set_write_to_pty(*profile.id(), permission, ctx);
                });
                ctx.notify();
            }
            AISettingsPageAction::SetMCPPermissions(permission) => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                    let profile = model.default_profile(ctx);
                    model.set_mcp_permissions(*profile.id(), permission, ctx);
                });
                ctx.notify();
            }
            AISettingsPageAction::RemoveDirectoryFromCodeReadAllowlist(dir) => {
                BlocklistAIPermissions::handle(ctx).update(ctx, |model, ctx| {
                    report_if_error!(
                        model.remove_filepath_from_code_read_allowlist(dir.to_owned(), ctx)
                    );
                });
            }
            AISettingsPageAction::ToggleRules => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings.memory_enabled.toggle_and_save_value(ctx);
                });
                ctx.notify();
            }
            AISettingsPageAction::ToggleRuleSuggestions => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings
                        .rule_suggestions_enabled_internal
                        .toggle_and_save_value(ctx);
                });
                ctx.notify();
            }
            AISettingsPageAction::ToggleWarpDriveContext => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings
                        .warp_drive_context_enabled
                        .toggle_and_save_value(ctx);
                });
                ctx.notify();
            }
            AISettingsPageAction::RemoveFromProfileDirectoryAllowlist(path_buf) => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                    let profile = model.default_profile(ctx);
                    let profile_id = profile.id();
                    model.remove_from_directory_allowlist(
                        *profile_id,
                        &PathBuf::from(path_buf),
                        ctx,
                    );
                });
                ctx.notify();
            }
            AISettingsPageAction::RemoveFromProfileCommandDenylist(cmd) => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                    let profile = model.default_profile(ctx);
                    let profile_id = profile.id();

                    model.remove_from_command_denylist(*profile_id, cmd, ctx);
                });
                ctx.notify();
            }
            AISettingsPageAction::RemoveFromProfileCommandAllowlist(command) => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                    let profile = model.default_profile(ctx);
                    let profile_id = profile.id();

                    model.remove_from_command_allowlist(*profile_id, command, ctx);
                });
                ctx.notify();
            }
            AISettingsPageAction::ToggleShowBaseModelPickerInPrompt => {
                SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                    if let Err(e) = settings
                        .show_model_selectors_in_prompt
                        .toggle_and_save_value(ctx)
                    {
                        log::warn!(
                            "Failed to set value for Show Base Model Picker in Prompt: {e:?}"
                        );
                    }
                });
                ctx.notify();
            }
            AISettingsPageAction::AddToMCPAllowlist(id) => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                    let profile = model.default_profile(ctx);
                    let profile_id = profile.id();
                    model.add_to_mcp_allowlist(*profile_id, id, ctx);
                });
                ctx.notify();
            }
            AISettingsPageAction::RemoveFromMCPAllowlist(id) => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                    let profile = model.default_profile(ctx);
                    let profile_id = profile.id();
                    model.remove_from_mcp_allowlist(*profile_id, id, ctx);
                });
                ctx.notify();
            }
            AISettingsPageAction::AddToMCPDenylist(id) => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                    let profile = model.default_profile(ctx);
                    let profile_id = profile.id();
                    model.add_to_mcp_denylist(*profile_id, id, ctx);
                });
                ctx.notify();
            }
            AISettingsPageAction::RemoveFromMCPDenylist(id) => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |model, ctx| {
                    let profile = model.default_profile(ctx);
                    let profile_id = profile.id();
                    model.remove_from_mcp_denylist(*profile_id, id, ctx);
                });
                ctx.notify();
            }
            AISettingsPageAction::CreateProfile => {
                let new_profile_id = AIExecutionProfilesModel::handle(ctx)
                    .update(ctx, |model, ctx| model.create_profile(ctx));

                if let Some(profile_id) = new_profile_id {
                    self.profile_views = Self::create_profile_views(ctx);
                    ctx.emit(AISettingsPageEvent::OpenExecutionProfileEditor(profile_id));
                }
                ctx.notify();
            }
            AISettingsPageAction::SignupAnonymousUser => {
                ctx.emit(AISettingsPageEvent::SignupAnonymousUser);
            }
            AISettingsPageAction::ToggleAwsBedrockAutoLogin => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.aws_bedrock_auto_login.toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::ToggleAwsBedrockCredentialsEnabled => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .aws_bedrock_credentials_enabled
                        .toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::RefreshAwsBedrockCredentials => {
                #[cfg(not(target_family = "wasm"))]
                ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                    drop(refresh_aws_credentials(manager, ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::ToggleCloudAgentComputerUse => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .cloud_agent_computer_use_enabled
                        .toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::ToggleFileBasedMcp => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.file_based_mcp_enabled.toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::ToggleIncludeAgentCommandsInHistory => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .include_agent_commands_in_history
                        .toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            #[cfg(feature = "local_fs")]
            AISettingsPageAction::SetConversationLayout(layout) => {
                crate::util::file::external_editor::EditorSettings::handle(ctx).update(
                    ctx,
                    |settings, ctx| {
                        report_if_error!(settings
                            .open_conversation_layout_preference
                            .set_value(*layout, ctx));
                    },
                );
                send_telemetry_from_ctx!(
                    TelemetryEvent::FeaturesPageAction {
                        action: "SetConversationLayout".to_string(),
                        value: format!("{layout:?}")
                    },
                    ctx
                );
                ctx.notify();
            }
            AISettingsPageAction::ToggleShowConversationHistory => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .show_conversation_history
                        .toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::ToggleFeedbackBundledSkill => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .feedback_bundled_skill_enabled
                        .toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::OpenAddCustomEndpointModal => {
                self.show_add_custom_endpoint_modal(ctx);
            }
            AISettingsPageAction::OpenEditCustomEndpointModal(index) => {
                self.show_edit_custom_endpoint_modal(*index, ctx);
            }
            AISettingsPageAction::ToggleCloudHandoff => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .should_force_disable_cloud_handoff
                        .toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::ToggleAmpersandHandoff => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .should_force_disable_ampersand_handoff
                        .toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::ToggleAutoHandoffOnSleep => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .auto_handoff_on_sleep_enabled
                        .toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            AISettingsPageAction::ToggleAgentAttribution => {
                // The updated value syncs to warp-server automatically via
                // `CloudPreferencesSyncer` as a `JsonPreference` GSO keyed
                // `Global_AgentAttributionEnabled`; no bespoke server call needed.
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .agent_attribution_enabled
                        .toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
        }
    }
}

impl SettingsPageMeta for AISettingsPageView {
    fn section() -> SettingsSection {
        SettingsSection::AI
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        FeatureFlag::AgentMode.is_enabled()
    }

    fn on_page_selected(&mut self, _: bool, ctx: &mut ViewContext<Self>) {
        AIRequestUsageModel::handle(ctx).update(ctx, |ai_request_usage_model, ctx| {
            ai_request_usage_model.refresh_request_usage_async(ctx)
        });
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<AISettingsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<AISettingsPageView>) -> Self {
        SettingsPageViewHandle::AI(view_handle)
    }
}

fn render_ai_setting_toggle<S: Setting>(
    label: impl Into<String>,
    action: AISettingsPageAction,
    is_setting_enabled: bool,
    is_setting_toggleable: bool,
    switch_state: SwitchStateHandle,
    tooltip_states: &RefCell<HashMap<String, MouseStateHandle>>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    build_toggle_element(
        render_body_item_label::<AISettingsPageAction>(
            label.into(),
            Some(styles::header_font_color(is_setting_toggleable, app)),
            None,
            LocalOnlyIconState::for_setting(
                S::storage_key(),
                S::sync_to_cloud(),
                &mut tooltip_states.borrow_mut(),
                app,
            ),
            ToggleState::Enabled,
            appearance,
        ),
        render_ai_feature_switch(
            switch_state,
            is_setting_enabled,
            is_setting_toggleable,
            action,
            app,
        ),
        appearance,
        None,
    )
}

fn render_ai_setting_label<S: Setting>(
    label: impl Into<String>,
    is_setting_toggleable: bool,
    tooltip_states: &RefCell<HashMap<String, MouseStateHandle>>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    Container::new(render_body_item_label::<AISettingsPageAction>(
        label.into(),
        Some(styles::header_font_color(is_setting_toggleable, app)),
        None,
        LocalOnlyIconState::for_setting(
            S::storage_key(),
            S::sync_to_cloud(),
            &mut tooltip_states.borrow_mut(),
            app,
        ),
        ToggleState::Enabled,
        appearance,
    ))
    .with_margin_bottom(HEADER_PADDING)
    .finish()
}

fn render_ai_setting_description(
    description: impl Into<Cow<'static, str>>,
    is_setting_toggleable: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let default_font_size = Appearance::as_ref(app).ui_font_size();
    render_ai_setting_description_with_font_size(
        description,
        default_font_size,
        is_setting_toggleable,
        app,
    )
}

fn render_ai_setting_description_with_font_size(
    description: impl Into<Cow<'static, str>>,
    font_size: f32,
    is_setting_toggleable: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let ui_builder = Appearance::as_ref(app).ui_builder();
    ui_builder
        .paragraph(description)
        .with_style(UiComponentStyles {
            font_size: Some(font_size),
            font_color: Some(styles::description_font_color(is_setting_toggleable, app).into()),
            margin: Some(
                Coords::default()
                    .top(styles::DESCRIPTION_NEGATIVE_MARGIN_OFFSET)
                    .bottom(styles::DESCRIPTION_MARGIN_BOTTOM)
                    .right(styles::TOGGLE_WIDTH_MARGIN),
            ),
            ..Default::default()
        })
        .build()
        .finish()
}

fn render_toolbar_layout_editor(
    editor: &ViewHandle<AgentToolbarInlineEditor>,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let label = Container::new(
        appearance
            .ui_builder()
            .span(i18n::tr(app, I18nKey::AiToolbarLayout).to_string())
            .with_style(UiComponentStyles {
                font_size: Some(CONTENT_FONT_SIZE),
                ..Default::default()
            })
            .build()
            .finish(),
    )
    .with_margin_bottom(4.)
    .finish();
    let editor = Container::new(ChildView::new(editor).finish())
        .with_margin_bottom(16.)
        .finish();

    Flex::column().with_child(label).with_child(editor).finish()
}

fn render_ai_feature_switch(
    state_handle: SwitchStateHandle,
    is_setting_enabled: bool,
    is_setting_toggleable: bool,
    toggle_action: AISettingsPageAction,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let ui_builder = appearance.ui_builder();
    ui_builder
        .switch(state_handle)
        .check(is_setting_enabled)
        .with_disabled(!is_setting_toggleable)
        .with_disabled_styles(UiComponentStyles {
            background: Some(Fill::Solid(internal_colors::neutral_4(appearance.theme()))),
            foreground: Some(Fill::Solid(internal_colors::neutral_5(appearance.theme()))),
            ..Default::default()
        })
        .build()
        .on_click(move |ctx, _, _| {
            if !is_setting_toggleable {
                return;
            }
            ctx.dispatch_typed_action(toggle_action.clone());
        })
        .finish()
}

fn render_ai_list(
    header: &str,
    description: &str,
    input_list: Box<dyn Element>,
    view: &AISettingsPageView,
    ai_settings: &AISettings,
    app: &AppContext,
) -> Box<dyn Element> {
    let setting_header = render_ai_setting_label::<AgentModeCommandExecutionDenylist>(
        header.to_string(),
        ai_settings.is_any_ai_enabled(app),
        &view.local_only_icon_tooltip_states,
        app,
    );

    let description = render_ai_setting_description(
        description.to_string(),
        ai_settings.is_any_ai_enabled(app),
        app,
    );

    Flex::column()
        .with_child(setting_header)
        .with_child(Container::new(description).with_margin_bottom(-8.).finish())
        .with_child(input_list)
        .finish()
}

#[derive(Default)]
struct GlobalAIWidget {
    switch_state: SwitchStateHandle,
    sign_up_button: MouseStateHandle,
}

impl SettingsWidget for GlobalAIWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "oz warp agent global ai a.i. active next command prompt code diffs suggestion suggested suggestions \
                agent mode natural language detection input hint api keys bring your own byo google anthropic openai"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let is_ai_disabled_due_to_remote_session_org_policy =
            AISettings::as_ref(app).is_ai_disabled_due_to_remote_session_org_policy(app);

        let is_anonymous = AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out();

        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Text::new_inline(
                    i18n::tr(app, I18nKey::AiWarpAgentTitle),
                    appearance.ui_font_family(),
                    PRIMARY_HEADER_FONT_SIZE,
                )
                .with_style(Properties::default().weight(Weight::Bold))
                .with_color(appearance.theme().active_ui_text_color().into())
                .finish(),
            );

        if is_ai_disabled_due_to_remote_session_org_policy {
            row.add_child(
                ConstrainedBox::new(
                    Container::new(
                        Text::new(
                            i18n::tr(app, I18nKey::AiRemoteSessionOrgPolicy),
                            appearance.ui_font_family(),
                            12.,
                        )
                        .with_color(appearance.theme().ui_warning_color())
                        .finish(),
                    )
                    .with_padding_left(8.)
                    .with_padding_right(8.)
                    .finish(),
                )
                .with_max_width(400.)
                .finish(),
            );
        }

        // Show sign-up button for anonymous users, toggle for logged-in users
        if is_anonymous {
            row.add_child(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        Container::new(
                            Text::new_inline(
                                i18n::tr(app, I18nKey::AiSignUpPrompt),
                                appearance.ui_font_family(),
                                14.,
                            )
                            .with_color(
                                appearance
                                    .theme()
                                    .sub_text_color(appearance.theme().surface_2())
                                    .into_solid(),
                            )
                            .finish(),
                        )
                        .with_margin_right(16.)
                        .finish(),
                    )
                    .with_child(
                        Container::new(
                            ui_builder
                                .button(ButtonVariant::Accent, self.sign_up_button.clone())
                                .with_style(UiComponentStyles {
                                    font_size: Some(14.),
                                    font_weight: Some(Weight::Semibold),
                                    border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
                                    padding: Some(Coords {
                                        top: 8.,
                                        bottom: 8.,
                                        left: 24.,
                                        right: 24.,
                                    }),
                                    ..Default::default()
                                })
                                .with_text_label(i18n::tr(app, I18nKey::AccountSignUp).to_owned())
                                .build()
                                .on_click(move |ctx, _, _| {
                                    ctx.dispatch_typed_action(
                                        AISettingsPageAction::SignupAnonymousUser,
                                    );
                                })
                                .finish(),
                        )
                        .with_padding_right(TOGGLE_BUTTON_RIGHT_PADDING)
                        .finish(),
                    )
                    .finish(),
            );
        } else {
            row.add_child(
                Container::new(
                    ui_builder
                        .switch(self.switch_state.clone())
                        .check(AISettings::as_ref(app).is_any_ai_enabled(app))
                        .build()
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(AISettingsPageAction::ToggleGlobalAI);
                        })
                        .finish(),
                )
                .with_padding_right(TOGGLE_BUTTON_RIGHT_PADDING)
                .finish(),
            );
        }

        Container::new(row.finish())
            .with_padding_bottom(15.)
            .finish()
    }
}

#[derive(Default)]
struct UsageWidget {
    requests_highlight_index: HighlightedHyperlink,
}

impl UsageWidget {
    fn render_request_usage_count(
        &self,
        used: usize,
        limit: usize,
        is_unlimited: bool,
        workspace_is_delinquent_due_to_payment_issue: bool,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn warpui::Element> {
        let mut row = Flex::row();
        if used >= limit || workspace_is_delinquent_due_to_payment_issue {
            row.add_child(
                ConstrainedBox::new(
                    Icon::AlertTriangle
                        .to_warpui_icon(appearance.theme().ui_error_color().into())
                        .finish(),
                )
                .with_height(16.)
                .with_width(16.)
                .finish(),
            )
        }

        let request_count_label = if workspace_is_delinquent_due_to_payment_issue {
            i18n::tr(app, I18nKey::AiRestrictedBilling).to_string()
        } else if is_unlimited {
            i18n::tr(app, I18nKey::AiUnlimited).to_string()
        } else {
            format!("{used}/{limit}")
        };

        row.add_child(
            appearance
                .ui_builder()
                .paragraph(request_count_label)
                .with_style(UiComponentStyles {
                    font_color: {
                        if used >= limit {
                            Some(appearance.theme().ui_error_color())
                        } else {
                            Some(blended_colors::text_sub(
                                appearance.theme(),
                                appearance.theme().surface_1(),
                            ))
                        }
                    },
                    font_size: Some(16.),
                    margin: Some(Coords {
                        top: 0.,
                        bottom: 0.,
                        left: 8.,
                        right: 0.,
                    }),
                    ..Default::default()
                })
                .build()
                .finish(),
        );

        row.finish()
    }

    /// Renders a row of what is being limited, along with the current used/limit.
    #[allow(clippy::too_many_arguments)]
    fn render_ai_usage_limit_row(
        &self,
        header: impl Into<Cow<'static, str>>,
        description: impl Into<Cow<'static, str>>,
        used: usize,
        limit: usize,
        is_unlimited: bool,
        workspace_is_delinquent_due_to_payment_issue: bool,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn warpui::Element> {
        let request_usage_details = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::End)
            .with_child(self.render_request_usage_count(
                used,
                limit,
                is_unlimited,
                workspace_is_delinquent_due_to_payment_issue,
                appearance,
                app,
            ));

        let request_usage_description = FormattedTextElement::from_str(
            description,
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(blended_colors::text_sub(
            appearance.theme(),
            appearance.theme().surface_1(),
        ));

        Flex::row()
            .with_child(
                Shrinkable::new(
                    2.,
                    Container::new(
                        Flex::column()
                            .with_child(
                                appearance
                                    .ui_builder()
                                    .paragraph(header)
                                    .with_style(UiComponentStyles {
                                        font_color: Some(blended_colors::text_main(
                                            appearance.theme(),
                                            appearance.theme().surface_1(),
                                        )),
                                        margin: Some(Coords {
                                            top: 0.,
                                            bottom: 4.,
                                            left: 0.,
                                            right: 0.,
                                        }),
                                        ..Default::default()
                                    })
                                    .build()
                                    .finish(),
                            )
                            .with_child(request_usage_description.finish())
                            .finish(),
                    )
                    .with_margin_bottom(16.)
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                Shrinkable::new(
                    1.,
                    Container::new(request_usage_details.finish())
                        .with_margin_bottom(16.)
                        .finish(),
                )
                .finish(),
            )
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .finish()
    }
}

impl SettingsWidget for UsageWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "a.i. ai usage limit plan"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_request_usage_model = AIRequestUsageModel::as_ref(app);
        let next_refresh_time = ai_request_usage_model.next_refresh_time();
        let formatted_next_refresh_time = next_refresh_time.format("%b %d").to_string();
        let workspace_is_delinquent_due_to_payment_issue = UserWorkspaces::as_ref(app)
            .current_team()
            .map(|team| team.billing_metadata.is_delinquent_due_to_payment_issue())
            .unwrap_or_default();

        let usage_header = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    build_sub_header(
                        appearance,
                        i18n::tr(app, I18nKey::AiUsage),
                        Some(styles::header_font_color(true, app)),
                    )
                    .finish(),
                )
                .with_child(
                    appearance
                        .ui_builder()
                        .paragraph(
                            i18n::tr(app, I18nKey::AiResetsDate)
                                .replace("{date}", &formatted_next_refresh_time),
                        )
                        .with_style(UiComponentStyles {
                            font_color: Some(blended_colors::text_sub(
                                appearance.theme(),
                                appearance.theme().surface_1(),
                            )),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .finish(),
        )
        .with_padding_bottom(HEADER_PADDING)
        .finish();

        let request_limit_description = i18n::tr(app, I18nKey::AiUsageLimitDescription).replace(
            "{duration}",
            &ai_request_usage_model.refresh_duration_to_string(),
        );

        let request_usage_row = self.render_ai_usage_limit_row(
            i18n::tr(app, I18nKey::AiCredits),
            request_limit_description,
            ai_request_usage_model.requests_used(),
            ai_request_usage_model.request_limit(),
            ai_request_usage_model.is_unlimited(),
            workspace_is_delinquent_due_to_payment_issue,
            appearance,
            app,
        );

        let auth_state = AuthStateProvider::as_ref(app).get();
        let upgrade_cta_text_fragments = if let Some(team) =
            UserWorkspaces::as_ref(app).current_team()
        {
            let current_user_email = auth_state.user_email().unwrap_or_default();
            let has_admin_permissions = team.has_admin_permissions(&current_user_email);
            if team.billing_metadata.can_upgrade_to_higher_tier_plan() {
                let upgrade_url = UserWorkspaces::upgrade_link_for_team(team.uid);
                if has_admin_permissions {
                    vec![
                        FormattedTextFragment::hyperlink(
                            i18n::tr(app, I18nKey::AiUpgrade),
                            upgrade_url,
                        ),
                        FormattedTextFragment::plain_text(i18n::tr(
                            app,
                            I18nKey::AiUpgradeMoreUsage,
                        )),
                    ]
                } else {
                    // The /upgrade page says to contact their administrator.
                    vec![
                        FormattedTextFragment::hyperlink(
                            i18n::tr(app, I18nKey::AiComparePlans),
                            upgrade_url,
                        ),
                        FormattedTextFragment::plain_text(i18n::tr(app, I18nKey::AiForMoreUsage)),
                    ]
                }
            } else {
                vec![
                    FormattedTextFragment::hyperlink(
                        i18n::tr(app, I18nKey::AiContactSupport),
                        "mailto:support@warp.dev",
                    ),
                    FormattedTextFragment::plain_text(i18n::tr(app, I18nKey::AiForMoreUsage)),
                ]
            }
        } else {
            let user_id = auth_state.user_id().unwrap_or_default();
            let upgrade_url = UserWorkspaces::upgrade_link(user_id);
            vec![
                FormattedTextFragment::hyperlink(i18n::tr(app, I18nKey::AiUpgrade), upgrade_url),
                FormattedTextFragment::plain_text(i18n::tr(app, I18nKey::AiUpgradeMoreUsage)),
            ]
        };

        let mut upgrade_cta = FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(upgrade_cta_text_fragments)]),
            appearance.ui_font_size(),
            appearance.ui_font_family(),
            appearance.ui_font_family(),
            styles::description_font_color(true, app).into(),
            self.requests_highlight_index.clone(),
        )
        .with_hyperlink_font_color(appearance.theme().accent().into_solid());

        if AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out()
        {
            upgrade_cta = upgrade_cta.register_default_click_handlers(|_, ctx, _| {
                ctx.dispatch_typed_action(AISettingsPageAction::AttemptLoginGatedUpgrade);
            });
        } else {
            upgrade_cta = upgrade_cta.register_default_click_handlers(|url, ctx, _| {
                ctx.dispatch_typed_action(AISettingsPageAction::HyperlinkClick(url));
            })
        }

        Flex::column()
            .with_children([
                render_separator(appearance),
                usage_header,
                request_usage_row,
                Container::new(upgrade_cta.finish())
                    .with_margin_bottom(16.)
                    .finish(),
            ])
            .finish()
    }
}

#[derive(Default)]
struct ActiveAIWidget {
    active_ai_toggle: SwitchStateHandle,
    intelligent_autosuggestions_toggle: SwitchStateHandle,
    prompt_suggestions_toggle: SwitchStateHandle,
    code_suggestions_toggle: SwitchStateHandle,
    natural_language_autosuggestions_toggle: SwitchStateHandle,
    shared_block_title_generation_toggle: SwitchStateHandle,
    git_operations_autogen_toggle: SwitchStateHandle,
}

impl ActiveAIWidget {
    fn is_next_command_toggleable(&self, app: &AppContext) -> bool {
        UserWorkspaces::as_ref(app).is_next_command_enabled()
            && AISettings::as_ref(app)
                .intelligent_autosuggestions_enabled_internal
                .is_supported_on_current_platform()
    }

    fn is_prompt_suggestions_toggleable(&self, app: &AppContext) -> bool {
        UserWorkspaces::as_ref(app).is_prompt_suggestions_toggleable()
            && AISettings::as_ref(app)
                .prompt_suggestions_enabled_internal
                .is_supported_on_current_platform()
    }

    fn is_suggested_code_banners_toggleable(&self, app: &AppContext) -> bool {
        (self.is_prompt_suggestions_toggleable(app)
            || UserWorkspaces::as_ref(app).is_code_suggestions_toggleable())
            && AISettings::as_ref(app)
                .code_suggestions_enabled_internal
                .is_supported_on_current_platform()
    }

    fn is_natural_language_autosuggestions_toggleable(&self, app: &AppContext) -> bool {
        FeatureFlag::PredictAMQueries.is_enabled()
            && AISettings::as_ref(app)
                .natural_language_autosuggestions_enabled_internal
                .is_supported_on_current_platform()
    }

    // TODO: Check if the user's enterprise billing policy allows toggling this feature.
    fn is_shared_block_title_generation_toggleable(&self, app: &AppContext) -> bool {
        FeatureFlag::SharedBlockTitleGeneration.is_enabled()
            && AISettings::as_ref(app)
                .shared_block_title_generation_enabled_internal
                .is_supported_on_current_platform()
            && (!UserWorkspaces::as_ref(app)
                .current_team()
                .is_some_and(|team| {
                    team.billing_metadata.customer_type == CustomerType::Enterprise
                })
                // Override the enterprise check for dogfood builds, as our dogfood team
                // is an enterprise team.
                || ChannelState::channel().is_dogfood())
    }

    fn is_git_operations_autogen_toggleable(&self, app: &AppContext) -> bool {
        FeatureFlag::GitOperationsInCodeReview.is_enabled()
            && AISettings::as_ref(app)
                .git_operations_autogen_enabled_internal
                .is_supported_on_current_platform()
            && UserWorkspaces::as_ref(app).is_git_operations_ai_enabled()
    }

    fn render_next_command_section(
        &self,
        view: &AISettingsPageView,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_active_ai_enabled(app);

        Flex::column()
            .with_child(
                render_ai_setting_toggle::<IntelligentAutosuggestionsEnabled>(
                    i18n::tr(app, I18nKey::AiNextCommand),
                    AISettingsPageAction::ToggleIntelligentAutosuggestions,
                    *ai_settings.intelligent_autosuggestions_enabled_internal,
                    is_toggleable,
                    self.intelligent_autosuggestions_toggle.clone(),
                    &view.local_only_icon_tooltip_states,
                    app,
                ),
            )
            .with_child(render_ai_setting_description(
                i18n::tr(app, I18nKey::AiNextCommandDescription),
                is_toggleable,
                app,
            ))
            .finish()
    }

    fn render_prompt_suggestions_section(
        &self,
        view: &AISettingsPageView,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_active_ai_enabled(app);
        Flex::column()
            .with_child(
                render_ai_setting_toggle::<AgentModeQuerySuggestionsEnabled>(
                    i18n::tr(app, I18nKey::AiPromptSuggestions),
                    AISettingsPageAction::TogglePromptSuggestions,
                    *ai_settings.prompt_suggestions_enabled_internal,
                    is_toggleable,
                    self.prompt_suggestions_toggle.clone(),
                    &view.local_only_icon_tooltip_states,
                    app,
                ),
            )
            .with_child(render_ai_setting_description(
                i18n::tr(app, I18nKey::AiPromptSuggestionsDescription),
                is_toggleable,
                app,
            ))
            .finish()
    }

    fn render_suggested_code_banners_section(
        &self,
        view: &AISettingsPageView,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_active_ai_enabled(app);
        Flex::column()
            .with_child(
                render_ai_setting_toggle::<AgentModeQuerySuggestionsEnabled>(
                    i18n::tr(app, I18nKey::AiSuggestedCodeBanners),
                    AISettingsPageAction::ToggleCodeSuggestions,
                    *ai_settings.code_suggestions_enabled_internal,
                    is_toggleable,
                    self.code_suggestions_toggle.clone(),
                    &view.local_only_icon_tooltip_states,
                    app,
                ),
            )
            .with_child(render_ai_setting_description(
                i18n::tr(app, I18nKey::AiSuggestedCodeBannersDescription),
                is_toggleable,
                app,
            ))
            .finish()
    }

    fn render_natural_language_autosuggestions_section(
        &self,
        view: &AISettingsPageView,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_active_ai_enabled(app);
        Flex::column()
            .with_child(render_ai_setting_toggle::<
                NaturalLanguageAutosuggestionsEnabled,
            >(
                i18n::tr(app, I18nKey::AiNaturalLanguageAutosuggestions),
                AISettingsPageAction::ToggleNaturalLanguageAutosuggestions,
                *ai_settings.natural_language_autosuggestions_enabled_internal,
                is_toggleable,
                self.natural_language_autosuggestions_toggle.clone(),
                &view.local_only_icon_tooltip_states,
                app,
            ))
            .with_child(render_ai_setting_description(
                i18n::tr(app, I18nKey::AiNaturalLanguageAutosuggestionsDescription),
                is_toggleable,
                app,
            ))
            .finish()
    }

    fn render_shared_block_title_generation_section(
        &self,
        view: &AISettingsPageView,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_active_ai_enabled(app);
        Flex::column()
            .with_child(
                render_ai_setting_toggle::<SharedBlockTitleGenerationEnabled>(
                    i18n::tr(app, I18nKey::AiSharedBlockTitleGeneration),
                    AISettingsPageAction::ToggleSharedTitleGeneration,
                    *ai_settings.shared_block_title_generation_enabled_internal,
                    is_toggleable,
                    self.shared_block_title_generation_toggle.clone(),
                    &view.local_only_icon_tooltip_states,
                    app,
                ),
            )
            .with_child(render_ai_setting_description(
                i18n::tr(app, I18nKey::AiSharedBlockTitleGenerationDescription),
                is_toggleable,
                app,
            ))
            .finish()
    }

    fn render_git_operations_autogen_section(
        &self,
        view: &AISettingsPageView,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_active_ai_enabled(app);
        Flex::column()
            .with_child(render_ai_setting_toggle::<GitOperationsAutogenEnabled>(
                i18n::tr(app, I18nKey::AiCommitPrGeneration),
                AISettingsPageAction::ToggleGitOperationsAutogen,
                *ai_settings.git_operations_autogen_enabled_internal,
                is_toggleable,
                self.git_operations_autogen_toggle.clone(),
                &view.local_only_icon_tooltip_states,
                app,
            ))
            .with_child(render_ai_setting_description(
                i18n::tr(app, I18nKey::AiCommitPrGenerationDescription),
                is_toggleable,
                app,
            ))
            .finish()
    }
}

impl SettingsWidget for ActiveAIWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "active ai a.i. next command prompt suggestions code diffs suggested banners passive unit tests commit pull request pr git code review autogen generate"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        self.is_next_command_toggleable(app)
            || self.is_prompt_suggestions_toggleable(app)
            || self.is_suggested_code_banners_toggleable(app)
            || self.is_natural_language_autosuggestions_toggleable(app)
            || self.is_shared_block_title_generation_toggleable(app)
            || self.is_git_operations_autogen_toggleable(app)
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);
        let mut column = Flex::column()
            .with_child(render_separator(appearance))
            .with_child(
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .with_child(
                            build_sub_header(
                                appearance,
                                i18n::tr(app, I18nKey::AiActiveAi),
                                Some(styles::header_font_color(is_any_ai_enabled, app)),
                            )
                            .finish(),
                        )
                        .with_child(
                            Container::new(render_ai_feature_switch(
                                self.active_ai_toggle.clone(),
                                *ai_settings.is_active_ai_enabled_internal,
                                is_any_ai_enabled,
                                AISettingsPageAction::ToggleActiveAI,
                                app,
                            ))
                            .with_padding_right(TOGGLE_BUTTON_RIGHT_PADDING)
                            .finish(),
                        )
                        .finish(),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            );

        if self.is_next_command_toggleable(app) {
            column.add_child(self.render_next_command_section(view, app));
        }

        if self.is_prompt_suggestions_toggleable(app) {
            column.add_child(self.render_prompt_suggestions_section(view, app));
        }

        if self.is_suggested_code_banners_toggleable(app) {
            column.add_child(self.render_suggested_code_banners_section(view, app));
        }

        if self.is_natural_language_autosuggestions_toggleable(app) {
            column.add_child(self.render_natural_language_autosuggestions_section(view, app));
        }

        if self.is_shared_block_title_generation_toggleable(app) {
            column.add_child(self.render_shared_block_title_generation_section(view, app));
        }

        if self.is_git_operations_autogen_toggleable(app) {
            column.add_child(self.render_git_operations_autogen_section(view, app));
        }

        column.finish()
    }
}

#[derive(Default)]
struct AgentsWidget {
    codebase_context_toggle: SwitchStateHandle,
    codebase_context_link_index: HighlightedHyperlink,
    show_in_prompt_checkbox: MouseStateHandle,
}

impl SettingsWidget for AgentsWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        if MCPServersWidget::should_show_mcp() {
            "ai a.i. agent autonomy profiles allowlist denylist autoexecute permissions models llms planning mcp server"
        } else {
            "ai a.i. agent autonomy profiles allowlist denylist autoexecute permissions models llms planning"
        }
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);

        let mut column = Flex::column();

        if FeatureFlag::ProfilesDesignRevamp.is_enabled() {
            column.add_child(
                Container::new(self.render_profiles_section(view, ai_settings, appearance, app))
                    .with_margin_bottom(8.)
                    .finish(),
            );
        } else {
            // Legacy layout: show Agents header + Models + Permissions
            let mut agents_header = Flex::column();
            agents_header.add_child(
                build_sub_header(
                    appearance,
                    i18n::tr(app, I18nKey::AiAgents),
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            );
            agents_header.add_child(render_ai_setting_description(
                i18n::tr(app, I18nKey::AiAgentsDescription),
                ai_settings.is_any_ai_enabled(app),
                app,
            ));
            let agents_header = agents_header.finish();
            column.add_children([
                render_separator(appearance),
                Container::new(agents_header)
                    .with_margin_bottom(8.)
                    .finish(),
            ]);
            column.add_children([
                Container::new(self.render_models_section(view, ai_settings, appearance, app))
                    .with_margin_bottom(8.)
                    .finish(),
                Container::new(self.render_permissions_section(view, ai_settings, appearance, app))
                    .with_margin_bottom(8.)
                    .finish(),
            ]);
        };

        column.finish()
    }
}

impl AgentsWidget {
    fn render_profiles_section(
        &self,
        view: &AISettingsPageView,
        ai_settings: &AISettings,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);

        let header_and_description = Flex::column()
            .with_child(
                build_sub_header(
                    appearance,
                    i18n::tr(app, I18nKey::AiProfiles),
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .finish(),
            )
            .with_child(
                Container::new(render_ai_setting_description(
                    i18n::tr(app, I18nKey::AiProfilesDescription),
                    is_any_ai_enabled,
                    app,
                ))
                .with_margin_top(12.)
                .finish(),
            )
            .finish();

        let mut profiles_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(Shrinkable::new(1., header_and_description).finish());

        if FeatureFlag::MultiProfile.is_enabled() {
            profiles_row.add_child(
                Container::new(view.add_profile_button.as_ref(app).render(app))
                    .with_margin_left(16.)
                    .finish(),
            );
        }

        let profiles_header = Container::new(profiles_row.finish())
            .with_margin_bottom(12.0)
            .finish();

        let mut profile_elements = vec![profiles_header];

        for profile_view in &view.profile_views {
            profile_elements.push(
                Container::new(ChildView::new(profile_view).finish())
                    .with_margin_bottom(8.)
                    .finish(),
            );
        }

        Flex::column().with_children(profile_elements).finish()
    }

    fn render_models_section(
        &self,
        view: &AISettingsPageView,
        ai_settings: &AISettings,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);
        let model_subheader = Container::new(render_custom_size_header(
            appearance,
            i18n::tr(app, I18nKey::AiModels),
            14.0,
            Some(styles::header_font_color(is_any_ai_enabled, app)),
        ))
        .with_margin_bottom(8.0)
        .finish();

        let base_model_setting =
            Container::new(self.render_base_model_setting(view, ai_settings, appearance, app))
                .with_margin_bottom(8.0)
                .finish();

        let mut children = vec![model_subheader, base_model_setting];
        if let Some(context_window_setting) =
            self.render_context_window_setting(view, ai_settings, appearance, app)
        {
            children.push(
                Container::new(context_window_setting)
                    .with_margin_bottom(8.0)
                    .finish(),
            );
        }

        Flex::column().with_children(children).finish()
    }

    /// Renders the context window slider + numeric input row shown below the
    /// base model dropdown. Returns `None` if the active base model does not
    /// advertise a configurable context window, global AI is disabled, or the
    /// [`FeatureFlag::ConfigurableContextWindow`] flag is disabled.
    fn render_context_window_setting(
        &self,
        view: &AISettingsPageView,
        ai_settings: &AISettings,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        if !FeatureFlag::ConfigurableContextWindow.is_enabled() {
            return None;
        }
        if !ai_settings.is_any_ai_enabled(app) {
            return None;
        }
        let cw = AISettingsPageView::configurable_context_window(app)?;
        let min = cw.min;
        let max = cw.max;

        let label = Container::new(render_body_item_label::<AISettingsPageAction>(
            i18n::tr(app, I18nKey::AiContextWindowTokens).to_string(),
            None,
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
        ))
        .with_margin_bottom(4.0)
        .finish();

        let min_label = appearance
            .ui_builder()
            .span(format!("{min}"))
            .with_style(UiComponentStyles {
                font_size: Some(CONTENT_FONT_SIZE),
                ..Default::default()
            })
            .build()
            .finish();

        let max_label = appearance
            .ui_builder()
            .span(format!("{max}"))
            .with_style(UiComponentStyles {
                font_size: Some(CONTENT_FONT_SIZE),
                ..Default::default()
            })
            .build()
            .finish();

        let current_value = AISettingsPageView::current_context_window_display_value(app)
            .unwrap_or(cw.default_max)
            .clamp(min, max);
        let slider = appearance
            .ui_builder()
            .slider(view.context_window_slider_state.clone())
            .with_range(min as f32..max as f32)
            .with_default_value(current_value as f32)
            .with_style(UiComponentStyles {
                width: Some(CONTEXT_WINDOW_SLIDER_WIDTH),
                margin: Some(Coords::default().left(8.).right(8.)),
                ..Default::default()
            })
            .on_drag(|ctx, _, val| {
                ctx.dispatch_typed_action(AISettingsPageAction::ContextWindowSliderDragged(
                    val.round() as u32,
                ));
            })
            .on_change(|ctx, _, val| {
                ctx.dispatch_typed_action(AISettingsPageAction::SetContextWindowSize(
                    val.round() as u32
                ));
            })
            .build()
            .finish();

        let context_window_editor = view.context_window_editor.clone();
        let input_box = Dismiss::new(
            appearance
                .ui_builder()
                .text_input(view.context_window_editor.clone())
                .with_style(UiComponentStyles {
                    width: Some(CONTEXT_WINDOW_INPUT_BOX_WIDTH),
                    padding: Some(Coords {
                        top: 6.,
                        bottom: 6.,
                        left: 10.,
                        right: 10.,
                    }),
                    margin: Some(Coords::default().left(12.)),
                    background: Some(appearance.theme().surface_2().into()),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .on_dismiss(move |ctx, app| {
            let buffer_text = context_window_editor.as_ref(app).buffer_text(app);
            let cleaned: String = buffer_text
                .chars()
                .filter(|c| !c.is_whitespace() && *c != ',')
                .collect();
            if let Ok(parsed) = cleaned.parse::<u32>() {
                ctx.dispatch_typed_action(AISettingsPageAction::SetContextWindowSize(parsed));
            }
        })
        .finish();

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(min_label)
            .with_child(slider)
            .with_child(max_label)
            .with_child(input_box)
            .finish();

        Some(Flex::column().with_child(label).with_child(row).finish())
    }

    fn render_permissions_section(
        &self,
        view: &AISettingsPageView,
        ai_settings: &AISettings,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);
        let permissions_subheader = Container::new(render_custom_size_header(
            appearance,
            i18n::tr(app, I18nKey::AiPermissions),
            14.0,
            Some(styles::header_font_color(is_any_ai_enabled, app)),
        ))
        .with_margin_bottom(4.0)
        .finish();

        let code_diff_setting =
            BlocklistAIPermissions::as_ref(app).get_apply_code_diffs_setting(app, None);
        let code_diffs = self.render_execution_profile_dropdown(
            i18n::tr(app, I18nKey::AiApplyCodeDiffs),
            Icon::Code2,
            action_permission_description(app, code_diff_setting),
            &view.apply_code_diffs_dropdown_menu,
            ai_settings,
            appearance,
            app,
        );

        let read_files_setting =
            BlocklistAIPermissions::as_ref(app).get_read_files_setting(app, None);
        let mut read_files_flex = Flex::column().with_main_axis_size(MainAxisSize::Min);
        read_files_flex.add_child(self.render_execution_profile_dropdown(
            i18n::tr(app, I18nKey::AiReadFiles),
            Icon::Notebook,
            action_permission_description(app, read_files_setting),
            &view.read_files_dropdown_menu,
            ai_settings,
            appearance,
            app,
        ));

        if read_files_setting == ActionPermission::AlwaysAsk {
            let directory_allowlist =
                BlocklistAIPermissions::as_ref(app).get_read_files_allowlist(app, None);
            read_files_flex.add_child(
                Container::new(Self::render_directory_allowlist(
                    directory_allowlist,
                    view,
                    ai_settings,
                    appearance,
                    app,
                ))
                .with_margin_bottom(HEADER_PADDING)
                .finish(),
            );
        }
        let read_files = read_files_flex.finish();

        let execute_commands_setting =
            BlocklistAIPermissions::as_ref(app).get_execute_commands_setting(app, None);
        let mut execute_commands_flex = Flex::column().with_main_axis_size(MainAxisSize::Min);
        execute_commands_flex.add_child(self.render_execution_profile_dropdown(
            i18n::tr(app, I18nKey::AiExecuteCommands),
            Icon::Terminal,
            action_permission_description(app, execute_commands_setting),
            &view.execute_commands_dropdown_menu,
            ai_settings,
            appearance,
            app,
        ));

        if execute_commands_setting == ActionPermission::AlwaysAsk
            || execute_commands_setting == ActionPermission::AgentDecides
        {
            let command_allowlist =
                BlocklistAIPermissions::as_ref(app).get_execute_commands_allowlist(app, None);
            execute_commands_flex.add_child(
                Container::new(Self::render_command_allowlist(
                    command_allowlist,
                    view,
                    ai_settings,
                    appearance,
                    app,
                ))
                .with_margin_bottom(8.)
                .finish(),
            );
        }

        if execute_commands_setting != ActionPermission::AlwaysAsk {
            let command_denylist = Container::new(Self::render_command_denylist(
                BlocklistAIPermissions::as_ref(app).get_execute_commands_denylist(app, None),
                view,
                ai_settings,
                appearance,
                app,
            ))
            .with_margin_bottom(8.)
            .finish();
            execute_commands_flex.add_child(command_denylist);
        }
        let execute_commands = execute_commands_flex.finish();

        let mut widget_children = vec![permissions_subheader];

        if UserWorkspaces::as_ref(app)
            .ai_autonomy_settings()
            .has_any_overrides()
        {
            widget_children.push(
                Container::new(render_settings_info_banner(
                    i18n::tr(app, I18nKey::AiPermissionsManagedByWorkspace),
                    None,
                    appearance,
                ))
                .with_margin_bottom(12.0)
                .finish(),
            );
        }

        widget_children.extend([code_diffs, read_files, execute_commands]);

        let write_to_pty_setting =
            BlocklistAIPermissions::as_ref(app).get_write_to_pty_setting(app, None);
        let write_to_pty = self.render_execution_profile_dropdown(
            i18n::tr(app, I18nKey::AiInteractWithRunningCommands),
            Icon::Workflow,
            write_to_pty_permission_description(app, write_to_pty_setting),
            &view.write_to_pty_autonomy_dropdown_menu,
            ai_settings,
            appearance,
            app,
        );
        widget_children.push(write_to_pty);

        if MCPServersWidget::should_show_mcp() {
            let mcp_permissions = self.render_mcp_permissions(view, ai_settings, appearance, app);
            widget_children.push(mcp_permissions);
        }

        if !FeatureFlag::FullSourceCodeEmbedding.is_enabled() {
            let codebase_context = Self::render_codebase_context_outline_generation_setting(
                self.codebase_context_toggle.clone(),
                self.codebase_context_link_index.clone(),
                view,
                ai_settings,
                appearance,
                app,
            );
            widget_children.push(codebase_context);
        }

        Flex::column().with_children(widget_children).finish()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_execution_profile_dropdown(
        &self,
        header_text: &str,
        header_icon: Icon,
        permission_description: &'static str,
        dropdown_menu: &ViewHandle<Dropdown<AISettingsPageAction>>,
        ai_settings: &AISettings,
        appearance: &Appearance,
        app: &warpui::AppContext,
    ) -> Box<dyn Element> {
        let header = Container::new(render_body_item_label_with_icon::<AISettingsPageAction>(
            header_text.into(),
            header_icon,
            Some(styles::header_font_color(
                ai_settings.is_any_ai_enabled(app),
                app,
            )),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
        ))
        .finish();

        let description_color = appearance.theme().disabled_ui_text_color();
        let alert_icon = Container::new(
            ConstrainedBox::new(
                Icon::AlertCircle
                    .to_warpui_icon(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().surface_2()),
                    )
                    .finish(),
            )
            .with_width(14.)
            .with_height(14.)
            .finish(),
        )
        .with_margin_right(4.)
        .finish();
        let text = Text::new(
            permission_description,
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(description_color.into())
        .finish();
        let description = Flex::row()
            .with_children([alert_icon, Shrinkable::new(1.0, text).finish()])
            .finish();

        Container::new(
            Flex::column()
                .with_child(header)
                .with_child(ChildView::new(dropdown_menu).finish())
                .with_child(description)
                .finish(),
        )
        .with_margin_bottom(12.)
        .finish()
    }

    fn render_command_denylist(
        command_denylist: Vec<AgentModeCommandExecutionPredicate>,
        view: &AISettingsPageView,
        ai_settings: &AISettings,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_disabled = !ai_settings.is_any_ai_enabled(app);
        let org_denylist = BlocklistAIPermissions::get_org_execute_commands_denylist(app);
        let mut tooltip_idx = 0usize;
        let list = render_input_list(
            None,
            command_denylist
                .into_iter()
                .zip(view.command_denylist_mouse_state_handles.clone())
                .rev()
                .map(|(cmd, mouse_state_handle)| {
                    let is_org = org_denylist.contains(&cmd);
                    let tooltip_mouse_state = if is_org {
                        let handle = view
                            .command_execution_denylist_tooltip_mouse_state_handles
                            .get(tooltip_idx)
                            .cloned();
                        tooltip_idx += 1;
                        handle
                    } else {
                        None
                    };
                    InputListItem {
                        item: cmd.to_string(),
                        mouse_state_handle,
                        on_remove_action: AISettingsPageAction::RemoveFromProfileCommandDenylist(
                            cmd,
                        ),
                        is_disabled: is_org || ai_disabled,
                        tooltip_mouse_state,
                    }
                }),
            Some(&view.command_denylist_editor),
            appearance,
        );
        render_ai_list(
            i18n::tr(app, I18nKey::AiCommandDenylist),
            i18n::tr(app, I18nKey::AiCommandDenylistDescription),
            list,
            view,
            ai_settings,
            app,
        )
    }

    fn render_command_allowlist(
        command_allowlist: Vec<AgentModeCommandExecutionPredicate>,
        view: &AISettingsPageView,
        ai_settings: &AISettings,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let disabled = !ai_settings.is_command_allowlist_editable(app);
        let list = render_input_list(
            None,
            command_allowlist
                .into_iter()
                .zip(view.command_allowlist_mouse_state_handles.clone())
                .rev()
                .map(move |(cmd, mouse_state_handle)| InputListItem {
                    item: cmd.to_string(),
                    mouse_state_handle,
                    on_remove_action: AISettingsPageAction::RemoveFromProfileCommandAllowlist(cmd),
                    is_disabled: disabled,
                    tooltip_mouse_state: None,
                }),
            Some(&view.command_allowlist_editor),
            appearance,
        );

        render_ai_list(
            i18n::tr(app, I18nKey::AiCommandAllowlist),
            i18n::tr(app, I18nKey::AiCommandAllowlistDescription),
            list,
            view,
            ai_settings,
            app,
        )
    }

    fn render_directory_allowlist(
        directory_allowlist: Vec<PathBuf>,
        view: &AISettingsPageView,
        ai_settings: &AISettings,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let disabled = !ai_settings.is_directory_allowlist_editable(app);
        let list = render_input_list(
            None,
            directory_allowlist
                .clone()
                .into_iter()
                .zip(view.directory_allowlist_mouse_state_handles.clone())
                .rev()
                .map(move |(path, mouse_state_handle)| InputListItem {
                    item: path.display().to_string(),
                    mouse_state_handle,
                    on_remove_action: AISettingsPageAction::RemoveFromProfileDirectoryAllowlist(
                        path,
                    ),
                    is_disabled: disabled,
                    tooltip_mouse_state: None,
                }),
            Some(&view.directory_allowlist_editor),
            appearance,
        );

        render_ai_list(
            i18n::tr(app, I18nKey::AiDirectoryAllowlist),
            i18n::tr(app, I18nKey::AiDirectoryAllowlistDescription),
            list,
            view,
            ai_settings,
            app,
        )
    }

    fn render_base_model_setting(
        &self,
        view: &AISettingsPageView,
        ai_settings: &AISettings,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let show_in_prompt_checkbox = {
            let is_checked = *SessionSettings::as_ref(app).show_model_selectors_in_prompt;

            let mut checkbox = appearance
                .ui_builder()
                .checkbox(self.show_in_prompt_checkbox.clone(), None)
                .check(is_checked);

            if !ai_settings.is_any_ai_enabled(app) {
                checkbox = checkbox.disabled();
            }

            Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_children([
                        checkbox
                            .build()
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(
                                    AISettingsPageAction::ToggleShowBaseModelPickerInPrompt,
                                );
                            })
                            .finish(),
                        appearance
                            .ui_builder()
                            .span(i18n::tr(app, I18nKey::AiShowModelPickerInPrompt).to_string())
                            .with_style(UiComponentStyles {
                                font_color: Some(
                                    theme.sub_text_color(theme.surface_2()).into_solid(),
                                ),
                                font_size: Some(12.0),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    ])
                    .finish(),
            )
            .with_margin_top(-6.0)
            .with_margin_left(-4.0)
            .finish()
        };

        render_dropdown_item(
            appearance,
            i18n::tr(app, I18nKey::AiBaseModel),
            Some(i18n::tr(app, I18nKey::AiBaseModelDescription)),
            Some(show_in_prompt_checkbox),
            LocalOnlyIconState::Hidden,
            (!ai_settings.is_any_ai_enabled(app))
                .then(|| appearance.theme().disabled_ui_text_color()),
            &view.base_model_dropdown,
        )
    }

    fn render_codebase_context_outline_generation_setting(
        codebase_context_toggle: SwitchStateHandle,
        codebase_context_link_index: HighlightedHyperlink,
        view: &AISettingsPageView,
        ai_settings: &AISettings,
        appearance: &Appearance,
        app: &warpui::AppContext,
    ) -> Box<dyn Element> {
        let code_settings = CodeSettings::as_ref(app);
        let toggle = render_ai_setting_toggle::<CodebaseContextEnabled>(
            i18n::tr(app, I18nKey::AiCodebaseContext),
            AISettingsPageAction::ToggleCodebaseContext,
            *code_settings.codebase_context_enabled,
            ai_settings.is_any_ai_enabled(app),
            codebase_context_toggle,
            &view.local_only_icon_tooltip_states,
            app,
        );

        let codebase_context_description = vec![
            FormattedTextFragment::plain_text(i18n::tr(app, I18nKey::AiCodebaseContextDescription)),
            FormattedTextFragment::hyperlink(
                i18n::tr(app, I18nKey::AiLearnMore),
                "https://docs.warp.dev/agent-platform/capabilities/codebase-context",
            ),
        ];
        let description = Container::new(
            FormattedTextElement::new(
                FormattedText::new([FormattedTextLine::Line(codebase_context_description)]),
                CONTENT_FONT_SIZE,
                appearance.ui_font_family(),
                appearance.ui_font_family(),
                styles::description_font_color(ai_settings.is_any_ai_enabled(app), app).into(),
                codebase_context_link_index,
            )
            .with_hyperlink_font_color(appearance.theme().accent().into_solid())
            .register_default_click_handlers(|url, ctx, _| {
                ctx.dispatch_typed_action(AISettingsPageAction::HyperlinkClick(url));
            })
            .finish(),
        )
        .with_margin_top(styles::DESCRIPTION_NEGATIVE_MARGIN_OFFSET)
        .with_margin_bottom(styles::DESCRIPTION_MARGIN_BOTTOM)
        .with_margin_right(styles::TOGGLE_WIDTH_MARGIN)
        .finish();

        Flex::column()
            .with_child(toggle)
            .with_child(description)
            .finish()
    }

    fn render_mcp_permissions(
        &self,
        view: &AISettingsPageView,
        ai_settings: &AISettings,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let all_runnable_mcp_servers =
            TemplatableMCPServerManager::get_all_cloud_synced_mcp_servers(app);
        if all_runnable_mcp_servers.is_empty() {
            self.render_mcp_permissions_zero_state(ai_settings, appearance, app)
        } else {
            self.render_mcp_permissions_with_servers(view, ai_settings, appearance, app)
        }
    }

    fn render_mcp_permissions_zero_state(
        &self,
        ai_settings: &AISettings,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let header = Container::new(render_body_item_label_with_icon::<AISettingsPageAction>(
            i18n::tr(app, I18nKey::AiCallMcpServers).into(),
            Icon::Dataflow,
            Some(styles::header_font_color(
                ai_settings.is_any_ai_enabled(app),
                app,
            )),
            None,
            LocalOnlyIconState::Hidden,
            ToggleState::Enabled,
            appearance,
        ))
        .with_margin_bottom(4.)
        .finish();

        let subtext = {
            let subtext_fragments = vec![
                FormattedTextFragment::plain_text(i18n::tr(app, I18nKey::AiMcpZeroStatePrefix)),
                FormattedTextFragment::hyperlink_action(
                    i18n::tr(app, I18nKey::AiAddServer),
                    AISettingsPageAction::OpenMCPServerCollection,
                ),
                FormattedTextFragment::plain_text(i18n::tr(app, I18nKey::AiMcpZeroStateMiddle)),
                FormattedTextFragment::hyperlink(
                    i18n::tr(app, I18nKey::AiMcpZeroStateLearnMore),
                    "https://docs.warp.dev/agent-platform/capabilities/mcp",
                ),
            ];

            Container::new(
                FormattedTextElement::new(
                    FormattedText::new([FormattedTextLine::Line(subtext_fragments)]),
                    CONTENT_FONT_SIZE,
                    appearance.ui_font_family(),
                    appearance.ui_font_family(),
                    styles::description_font_color(ai_settings.is_any_ai_enabled(app), app).into(),
                    HighlightedHyperlink::default(),
                )
                .with_hyperlink_font_color(appearance.theme().accent().into_solid())
                .register_default_click_handlers_with_action_support(|hyperlink_lens, ctx, _app| {
                    match hyperlink_lens {
                        HyperlinkLens::Url(url) => {
                            ctx.dispatch_typed_action(AISettingsPageAction::HyperlinkClick(
                                HyperlinkUrl {
                                    url: url.to_owned(),
                                },
                            ));
                        }
                        HyperlinkLens::Action(action_ref) => {
                            if let Some(action) =
                                action_ref.as_any().downcast_ref::<AISettingsPageAction>()
                            {
                                ctx.dispatch_typed_action(action.clone());
                            }
                        }
                    }
                })
                .finish(),
            )
            .with_margin_bottom(4.0)
            .finish()
        };

        Flex::column()
            .with_child(header)
            .with_child(subtext)
            .finish()
    }

    fn render_mcp_permissions_with_servers(
        &self,
        view: &AISettingsPageView,
        ai_settings: &AISettings,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut column = Flex::column();

        let current_mcp_setting =
            BlocklistAIPermissions::as_ref(app).get_mcp_permissions_setting(app, None);

        let permission_setting = self.render_execution_profile_dropdown(
            i18n::tr(app, I18nKey::AiCallMcpServers),
            Icon::Dataflow,
            action_permission_description(app, current_mcp_setting),
            &view.mcp_permissions_dropdown_menu,
            ai_settings,
            appearance,
            app,
        );
        column.add_child(permission_setting);

        if current_mcp_setting == ActionPermission::AlwaysAsk
            || current_mcp_setting == ActionPermission::AgentDecides
        {
            let allowlist = self.render_mcp_list(
                i18n::tr(app, I18nKey::AiMcpAllowlist),
                i18n::tr(app, I18nKey::AiMcpAllowlistDescription),
                &view.mcp_allowlist_dropdown,
                BlocklistAIPermissions::as_ref(app).get_mcp_allowlist(app, None),
                view.mcp_allowlist_mouse_state_handles.clone(),
                AISettingsPageAction::RemoveFromMCPAllowlist,
                ai_settings,
                appearance,
                app,
            );
            column.add_child(allowlist);
        }

        if current_mcp_setting == ActionPermission::AlwaysAllow
            || current_mcp_setting == ActionPermission::AgentDecides
        {
            let denylist = self.render_mcp_list(
                i18n::tr(app, I18nKey::AiMcpDenylist),
                i18n::tr(app, I18nKey::AiMcpDenylistDescription),
                &view.mcp_denylist_dropdown,
                BlocklistAIPermissions::as_ref(app).get_mcp_denylist(app, None),
                view.mcp_denylist_mouse_state_handles.clone(),
                AISettingsPageAction::RemoveFromMCPDenylist,
                ai_settings,
                appearance,
                app,
            );
            column.add_child(denylist);
        }

        column.finish()
    }

    // Helper function to render the allow and denylists for mcp servers
    #[allow(clippy::too_many_arguments)]
    fn render_mcp_list(
        &self,
        title: &str,
        description: &str,
        dropdown: &ViewHandle<FilterableDropdown<AISettingsPageAction>>,
        items: Vec<uuid::Uuid>,
        mouse_state_handles: Vec<MouseStateHandle>,
        action: impl Fn(uuid::Uuid) -> AISettingsPageAction,
        ai_settings: &AISettings,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let selector = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_children(vec![
                    Shrinkable::new(
                        1.0,
                        Container::new(render_dropdown_item_label(
                            title.to_string(),
                            Some(description.to_string()),
                            LocalOnlyIconState::Hidden,
                            (!ai_settings.is_any_ai_enabled(app))
                                .then(|| appearance.theme().disabled_ui_text_color()),
                            appearance,
                        ))
                        .with_margin_right(4.)
                        .finish(),
                    )
                    .finish(),
                    ChildView::new(dropdown).finish(),
                ])
                .finish(),
        )
        .with_margin_bottom(2.)
        .finish();

        let disabled = !ai_settings.is_any_ai_enabled(app);
        let items = render_input_list(
            None,
            items
                .into_iter()
                .rev()
                .zip(mouse_state_handles.clone())
                .filter_map(move |(uuid, mouse_state_handle)| {
                    let server_name = TemplatableMCPServerManager::get_mcp_name(&uuid, app);
                    server_name.map(|server_name| InputListItem {
                        item: server_name,
                        mouse_state_handle,
                        on_remove_action: action(uuid),
                        is_disabled: disabled,
                        tooltip_mouse_state: None,
                    })
                }),
            None,
            appearance,
        );

        Container::new(Flex::column().with_children(vec![selector, items]).finish())
            .with_margin_bottom(8.)
            .finish()
    }
}

#[derive(Default)]
struct AIInputWidget {
    incorrect_autodetection_highlight_index: HighlightedHyperlink,
    autodetection_toggle: SwitchStateHandle,
    nld_in_terminal_toggle: SwitchStateHandle,
    show_input_hint_toggle: SwitchStateHandle,
    show_agent_tips_toggle: SwitchStateHandle,
    include_agent_commands_in_history_toggle: SwitchStateHandle,
}

impl SettingsWidget for AIInputWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "oz agent ai input natural language detection autodetection prompt terminal command commands history shell executed execution"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);

        let input_header = build_sub_header(
            appearance,
            i18n::tr(app, I18nKey::AiInput),
            Some(styles::header_font_color(is_any_ai_enabled, app)),
        )
        .with_padding_bottom(HEADER_PADDING)
        .finish();

        let natural_language_detection_section = Self::render_natural_language_detection_section(
            self.incorrect_autodetection_highlight_index.clone(),
            self.autodetection_toggle.clone(),
            self.nld_in_terminal_toggle.clone(),
            view,
            ai_settings,
            appearance,
            app,
        );

        let show_input_hint_text = render_ai_setting_toggle::<ShowHintText>(
            i18n::tr(app, I18nKey::AiShowInputHintText),
            AISettingsPageAction::ToggleShowInputHintText,
            *InputSettings::as_ref(app).show_hint_text,
            is_any_ai_enabled,
            self.show_input_hint_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        );

        let mut widget_children = vec![
            render_separator(appearance),
            input_header,
            natural_language_detection_section,
            show_input_hint_text,
        ];

        if FeatureFlag::AgentTips.is_enabled() {
            let agent_tips_toggle = render_ai_setting_toggle::<ShowAgentTips>(
                i18n::tr(app, I18nKey::AiShowAgentTips),
                AISettingsPageAction::ToggleShowAgentTips,
                *InputSettings::as_ref(app).show_agent_tips,
                is_any_ai_enabled,
                self.show_agent_tips_toggle.clone(),
                &view.local_only_icon_tooltip_states,
                app,
            );
            widget_children.push(agent_tips_toggle);
        }

        widget_children.push(render_ai_setting_toggle::<IncludeAgentCommandsInHistory>(
            i18n::tr(app, I18nKey::AiIncludeAgentCommandsHistory),
            AISettingsPageAction::ToggleIncludeAgentCommandsInHistory,
            *ai_settings.include_agent_commands_in_history,
            is_any_ai_enabled,
            self.include_agent_commands_in_history_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        ));

        Flex::column().with_children(widget_children).finish()
    }
}

impl AIInputWidget {
    fn render_natural_language_detection_section(
        incorrect_autodetection_highlight_index: HighlightedHyperlink,
        autodetection_toggle: SwitchStateHandle,
        nld_in_terminal_toggle: SwitchStateHandle,
        view: &AISettingsPageView,
        ai_settings: &AISettings,
        appearance: &Appearance,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let is_toggleable = ai_settings.is_any_ai_enabled(app);
        let is_nld_enabled = *ai_settings.ai_autodetection_enabled_internal.value();

        let autodetection_denylist_input_field = appearance
            .ui_builder()
            .text_input(view.autodetection_denylist_editor.clone())
            .with_style(UiComponentStyles {
                width: Some(280.),
                padding: Some(Coords {
                    top: 4.,
                    bottom: 4.,
                    left: 6.,
                    right: 6.,
                }),
                background: Some(appearance.theme().surface_2().into()),
                ..Default::default()
            })
            .build()
            .finish();

        let mut section = Flex::column();

        if FeatureFlag::AgentView.is_enabled() {
            let autodetection_description_fragments = vec![
                FormattedTextFragment::plain_text(i18n::tr(
                    app,
                    I18nKey::AiIncorrectDetectionPrefix,
                )),
                FormattedTextFragment::hyperlink(
                    i18n::tr(app, I18nKey::AiLetUsKnow),
                    "https://warpdotdev.typeform.com/to/offrTIpq",
                ),
            ];

            section.add_children([
                render_ai_setting_toggle::<NLDInTerminalEnabled>(
                    i18n::tr(app, I18nKey::AiAutodetectAgentPrompts),
                    AISettingsPageAction::ToggleNLDInTerminal,
                    ai_settings.is_nld_in_terminal_enabled(app),
                    is_toggleable,
                    nld_in_terminal_toggle,
                    &view.local_only_icon_tooltip_states,
                    app,
                ),
                render_ai_setting_toggle::<AIAutoDetectionEnabled>(
                    i18n::tr(app, I18nKey::AiAutodetectTerminalCommands),
                    AISettingsPageAction::ToggleAIInputAutoDetection,
                    is_nld_enabled,
                    is_toggleable,
                    autodetection_toggle,
                    &view.local_only_icon_tooltip_states,
                    app,
                ),
                Container::new(
                    FormattedTextElement::new(
                        FormattedText::new([FormattedTextLine::Line(
                            autodetection_description_fragments,
                        )]),
                        CONTENT_FONT_SIZE,
                        appearance.ui_font_family(),
                        appearance.ui_font_family(),
                        styles::description_font_color(is_toggleable, app).into(),
                        incorrect_autodetection_highlight_index,
                    )
                    .with_hyperlink_font_color(appearance.theme().accent().into_solid())
                    .register_default_click_handlers(|url, ctx, _| {
                        ctx.dispatch_typed_action(AISettingsPageAction::HyperlinkClick(url));
                    })
                    .finish(),
                )
                .with_margin_top(styles::DESCRIPTION_NEGATIVE_MARGIN_OFFSET)
                .with_margin_bottom(styles::DESCRIPTION_MARGIN_BOTTOM)
                .with_margin_right(styles::TOGGLE_WIDTH_MARGIN)
                .finish(),
            ])
        } else {
            let natural_language_detection_description_fragments = vec![
                FormattedTextFragment::plain_text(i18n::tr(
                    app,
                    I18nKey::AiNaturalLanguageDetectionDescription,
                )),
                FormattedTextFragment::plain_text(i18n::tr(
                    app,
                    I18nKey::AiIncorrectInputDetectionPrefix,
                )),
                FormattedTextFragment::hyperlink(
                    i18n::tr(app, I18nKey::AiLetUsKnow),
                    "https://warpdotdev.typeform.com/to/offrTIpq",
                ),
            ];

            section.add_children([
                render_ai_setting_toggle::<AIAutoDetectionEnabled>(
                    i18n::tr(app, I18nKey::AiNaturalLanguageDetection),
                    AISettingsPageAction::ToggleAIInputAutoDetection,
                    is_nld_enabled,
                    is_toggleable,
                    autodetection_toggle,
                    &view.local_only_icon_tooltip_states,
                    app,
                ),
                Container::new(
                    FormattedTextElement::new(
                        FormattedText::new([FormattedTextLine::Line(
                            natural_language_detection_description_fragments,
                        )]),
                        CONTENT_FONT_SIZE,
                        appearance.ui_font_family(),
                        appearance.ui_font_family(),
                        styles::description_font_color(is_toggleable, app).into(),
                        incorrect_autodetection_highlight_index,
                    )
                    .with_hyperlink_font_color(appearance.theme().accent().into_solid())
                    .register_default_click_handlers(|url, ctx, _| {
                        ctx.dispatch_typed_action(AISettingsPageAction::HyperlinkClick(url));
                    })
                    .finish(),
                )
                .with_margin_top(styles::DESCRIPTION_NEGATIVE_MARGIN_OFFSET)
                .with_margin_bottom(styles::DESCRIPTION_MARGIN_BOTTOM)
                .with_margin_right(styles::TOGGLE_WIDTH_MARGIN)
                .finish(),
            ]);
        }

        section
            .with_child(render_ai_setting_label::<AICommandDenylist>(
                i18n::tr(app, I18nKey::AiNaturalLanguageDenylist).to_owned(),
                is_toggleable,
                &view.local_only_icon_tooltip_states,
                app,
            ))
            .with_child(render_ai_setting_description(
                i18n::tr(app, I18nKey::AiNaturalLanguageDenylistDescription),
                is_toggleable,
                app,
            ))
            .with_child(
                Container::new(autodetection_denylist_input_field)
                    .with_margin_bottom(styles::DESCRIPTION_MARGIN_BOTTOM)
                    .finish(),
            )
            .finish()
    }
}

#[derive(Default)]
struct MCPServersWidget {
    manage_mcp_servers_button: MouseStateHandle,
    mcp_docs_link_index: HighlightedHyperlink,
    file_based_mcp_toggle: SwitchStateHandle,
    file_based_mcp_docs_link_index: HighlightedHyperlink,
}

impl MCPServersWidget {
    fn should_show_mcp() -> bool {
        FeatureFlag::McpServer.is_enabled() && ContextFlag::ShowMCPServers.is_enabled()
    }
}

impl SettingsWidget for MCPServersWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "oz agent mcp server servers model context protocol file-based file based project claude .mcp.json .claude/.mcp.json .codex config.toml .codex/config.toml"
    }

    fn should_render(&self, _app: &AppContext) -> bool {
        MCPServersWidget::should_show_mcp()
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let is_any_ai_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);
        let ai_settings = AISettings::as_ref(app);

        let header = build_sub_header(
            appearance,
            i18n::tr(app, I18nKey::AiMcpServers),
            Some(styles::header_font_color(is_any_ai_enabled, app)),
        )
        .with_padding_bottom(HEADER_PADDING)
        .finish();

        let mcp_description = vec![
            FormattedTextFragment::plain_text(i18n::tr(app, I18nKey::AiMcpServersDescription)),
            FormattedTextFragment::hyperlink(
                i18n::tr(app, I18nKey::AiLearnMore),
                "https://docs.warp.dev/agent-platform/capabilities/mcp",
            ),
        ];

        let description = Container::new(
            FormattedTextElement::new(
                FormattedText::new([FormattedTextLine::Line(mcp_description)]),
                CONTENT_FONT_SIZE,
                appearance.ui_font_family(),
                appearance.ui_font_family(),
                styles::description_font_color(is_any_ai_enabled, app).into(),
                self.mcp_docs_link_index.clone(),
            )
            .with_hyperlink_font_color(appearance.theme().accent().into_solid())
            .register_default_click_handlers(|url, ctx, _| {
                ctx.dispatch_typed_action(AISettingsPageAction::HyperlinkClick(url));
            })
            .finish(),
        )
        .with_margin_top(styles::DESCRIPTION_NEGATIVE_MARGIN_OFFSET)
        .with_margin_bottom(styles::DESCRIPTION_MARGIN_BOTTOM)
        .with_margin_right(styles::TOGGLE_WIDTH_MARGIN)
        .finish();

        let file_based_mcp_toggle = if FeatureFlag::FileBasedMcp.is_enabled() {
            Some(
                Flex::column()
                    .with_child(render_ai_setting_toggle::<FileBasedMcpEnabled>(
                        i18n::tr(app, I18nKey::AiAutoSpawnThirdPartyServers),
                        AISettingsPageAction::ToggleFileBasedMcp,
                        *ai_settings.file_based_mcp_enabled,
                        is_any_ai_enabled,
                        self.file_based_mcp_toggle.clone(),
                        &view.local_only_icon_tooltip_states,
                        app,
                    ))
                    .with_child({
                        let file_based_mcp_description_fragments = vec![
                            FormattedTextFragment::plain_text(i18n::tr(
                                app,
                                I18nKey::AiFileBasedMcpDescription,
                            )),
                            FormattedTextFragment::hyperlink(
                                i18n::tr(app, I18nKey::AiSeeSupportedProviders),
                                "https://docs.warp.dev/agent-platform/capabilities/mcp#file-based-mcp-servers",
                            ),
                        ];
                        Container::new(
                            FormattedTextElement::new(
                                FormattedText::new([FormattedTextLine::Line(
                                    file_based_mcp_description_fragments,
                                )]),
                                CONTENT_FONT_SIZE,
                                appearance.ui_font_family(),
                                appearance.ui_font_family(),
                                styles::description_font_color(is_any_ai_enabled, app).into(),
                                self.file_based_mcp_docs_link_index.clone(),
                            )
                            .with_hyperlink_font_color(appearance.theme().accent().into_solid())
                            .register_default_click_handlers(|url, ctx, _| {
                                ctx.dispatch_typed_action(AISettingsPageAction::HyperlinkClick(url));
                            })
                            .finish(),
                        )
                        .with_margin_top(styles::DESCRIPTION_NEGATIVE_MARGIN_OFFSET)
                        .with_margin_bottom(styles::DESCRIPTION_MARGIN_BOTTOM)
                        .with_margin_right(styles::TOGGLE_WIDTH_MARGIN)
                        .finish()
                    })
                    .finish(),
            )
        } else {
            None
        };

        let button = render_full_pane_width_ai_button(
            i18n::tr(app, I18nKey::AiManageMcpServers),
            is_any_ai_enabled,
            self.manage_mcp_servers_button.clone(),
            AISettingsPageAction::OpenMCPServerCollection,
            appearance,
        );

        let mut column = Flex::column()
            .with_child(header)
            .with_child(description)
            .with_child(button);

        if let Some(toggle) = file_based_mcp_toggle {
            column = column.with_child(toggle);
        }
        column.finish()
    }
}

#[derive(Default)]
struct AIFactWidget {
    rules_toggle: SwitchStateHandle,
    rules_link_index: HighlightedHyperlink,
    manage_rules_button: MouseStateHandle,
    rule_suggestions_toggle: SwitchStateHandle,
    warp_drive_context_toggle: SwitchStateHandle,
}

impl AIFactWidget {
    fn render_rules_toggle(
        &self,
        view: &AISettingsPageView,
        ai_settings: &AISettings,
        appearance: &Appearance,
        app: &warpui::AppContext,
    ) -> Box<dyn Element> {
        let toggle = render_ai_setting_toggle::<MemoryEnabled>(
            i18n::tr(app, I18nKey::AiRules),
            AISettingsPageAction::ToggleRules,
            *ai_settings.memory_enabled,
            ai_settings.is_any_ai_enabled(app),
            self.rules_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        );

        let rules_description = vec![
            FormattedTextFragment::plain_text(i18n::tr(app, I18nKey::AiRulesDescription)),
            FormattedTextFragment::hyperlink(
                i18n::tr(app, I18nKey::AiLearnMore),
                "https://docs.warp.dev/agent-platform/capabilities/rules",
            ),
        ];
        let description = Container::new(
            FormattedTextElement::new(
                FormattedText::new([FormattedTextLine::Line(rules_description)]),
                CONTENT_FONT_SIZE,
                appearance.ui_font_family(),
                appearance.ui_font_family(),
                styles::description_font_color(ai_settings.is_any_ai_enabled(app), app).into(),
                self.rules_link_index.clone(),
            )
            .with_hyperlink_font_color(appearance.theme().accent().into_solid())
            .register_default_click_handlers(|url, ctx, _| {
                ctx.dispatch_typed_action(AISettingsPageAction::HyperlinkClick(url));
            })
            .finish(),
        )
        .with_margin_top(styles::DESCRIPTION_NEGATIVE_MARGIN_OFFSET)
        .with_margin_bottom(styles::DESCRIPTION_MARGIN_BOTTOM)
        .with_margin_right(styles::TOGGLE_WIDTH_MARGIN)
        .finish();

        Flex::column()
            .with_child(toggle)
            .with_child(description)
            .finish()
    }

    fn render_rule_suggestions_toggle(
        &self,
        view: &AISettingsPageView,
        ai_settings: &AISettings,
        app: &warpui::AppContext,
    ) -> Box<dyn Element> {
        let toggle = render_ai_setting_toggle::<RuleSuggestionsEnabled>(
            i18n::tr(app, I18nKey::AiSuggestedRules),
            AISettingsPageAction::ToggleRuleSuggestions,
            *ai_settings.rule_suggestions_enabled_internal,
            ai_settings.is_any_ai_enabled(app),
            self.rule_suggestions_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        );

        let description = render_ai_setting_description(
            i18n::tr(app, I18nKey::AiSuggestedRulesDescription),
            ai_settings.is_any_ai_enabled(app),
            app,
        );

        Flex::column()
            .with_child(toggle)
            .with_child(description)
            .finish()
    }

    fn render_warp_drive_context_toggle(
        &self,
        view: &AISettingsPageView,
        ai_settings: &AISettings,
        app: &warpui::AppContext,
    ) -> Box<dyn Element> {
        let toggle = render_ai_setting_toggle::<WarpDriveContextEnabled>(
            i18n::tr(app, I18nKey::AiWarpDriveAgentContext),
            AISettingsPageAction::ToggleWarpDriveContext,
            *ai_settings.warp_drive_context_enabled,
            ai_settings.is_any_ai_enabled(app),
            self.warp_drive_context_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        );

        let description = render_ai_setting_description(
            i18n::tr(app, I18nKey::AiWarpDriveAgentContextDescription),
            ai_settings.is_any_ai_enabled(app),
            app,
        );

        Flex::column()
            .with_child(toggle)
            .with_child(description)
            .finish()
    }
}

impl SettingsWidget for AIFactWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "agent oz ai a.i. knowledge fact memory memories rules warp drive context workflows notebooks environment variables"
    }

    fn should_render(&self, _app: &AppContext) -> bool {
        FeatureFlag::AIRules.is_enabled()
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);

        let header = build_sub_header(
            appearance,
            i18n::tr(app, I18nKey::AiKnowledge),
            Some(styles::header_font_color(is_any_ai_enabled, app)),
        )
        .with_margin_bottom(HEADER_PADDING)
        .finish();

        let button = render_full_pane_width_ai_button(
            i18n::tr(app, I18nKey::AiManageRules),
            is_any_ai_enabled,
            self.manage_rules_button.clone(),
            AISettingsPageAction::OpenAIFactCollection,
            appearance,
        );

        let mut column = Flex::column()
            .with_child(header)
            .with_child(self.render_rules_toggle(view, ai_settings, appearance, app));

        if FeatureFlag::SuggestedRules.is_enabled() {
            column.add_child(self.render_rule_suggestions_toggle(view, ai_settings, app));
        }

        column
            .with_child(button)
            .with_child(self.render_warp_drive_context_toggle(view, ai_settings, app))
            .finish()
    }
}

#[derive(Default)]
struct VoiceWidget {
    voice_input_toggle: SwitchStateHandle,
    wispr_highlight_index: HighlightedHyperlink,
}

impl VoiceWidget {
    fn render_voice_section(
        &self,
        view: &AISettingsPageView,
        appearance: &Appearance,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_any_ai_enabled(app);
        let mut column = Flex::column().with_child(render_ai_setting_toggle::<VoiceInputEnabled>(
            i18n::tr(app, I18nKey::AiVoiceInput),
            AISettingsPageAction::ToggleVoiceInput,
            *ai_settings.voice_input_enabled_internal,
            is_toggleable,
            self.voice_input_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        ));

        let voice_input_description_text_fragments = vec![
            FormattedTextFragment::plain_text(i18n::tr(
                app,
                I18nKey::AiVoiceInputDescriptionPrefix,
            )),
            FormattedTextFragment::hyperlink("Wispr Flow", WISPR_FLOW_URL),
            FormattedTextFragment::plain_text(i18n::tr(
                app,
                I18nKey::AiVoiceInputDescriptionSuffix,
            )),
        ];

        let voice_input_description = FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(
                voice_input_description_text_fragments,
            )]),
            appearance.ui_font_size(),
            appearance.ui_font_family(),
            appearance.ui_font_family(),
            styles::description_font_color(is_toggleable, app).into(),
            self.wispr_highlight_index.clone(),
        )
        .with_hyperlink_font_color(appearance.theme().accent().into_solid())
        .register_default_click_handlers(|url, ctx, _| {
            ctx.dispatch_typed_action(AISettingsPageAction::HyperlinkClick(url));
        });

        column.add_child(
            Container::new(voice_input_description.finish())
                .with_margin_top(styles::DESCRIPTION_NEGATIVE_MARGIN_OFFSET)
                .with_margin_bottom(styles::DESCRIPTION_MARGIN_BOTTOM)
                .with_margin_right(styles::TOGGLE_WIDTH_MARGIN)
                .finish(),
        );

        if ai_settings.is_voice_input_enabled(app) {
            column.add_child(render_dropdown_item(
                appearance,
                i18n::tr(app, I18nKey::AiKeyForActivatingVoiceInput),
                Some(i18n::tr(app, I18nKey::AiPressAndHoldToActivate)),
                None,
                LocalOnlyIconState::for_setting(
                    VoiceInputToggleKey::storage_key(),
                    VoiceInputToggleKey::sync_to_cloud(),
                    &mut view.local_only_icon_tooltip_states.borrow_mut(),
                    app,
                ),
                None,
                &view.voice_input_toggle_key_dropdown,
            ));
        }

        column.finish()
    }
}

impl SettingsWidget for VoiceWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "voice agent oz ai a.i. speech input natural language talk english"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        cfg!(feature = "voice_input") && UserWorkspaces::as_ref(app).is_voice_enabled()
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);
        Flex::column()
            .with_child(render_separator(appearance))
            .with_child(
                build_sub_header(
                    appearance,
                    i18n::tr(app, I18nKey::AiVoice),
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            )
            .with_child(self.render_voice_section(view, appearance, app))
            .finish()
    }
}
#[derive(Default)]
struct OtherAIWidget {
    show_oz_updates_in_zero_state_toggle: SwitchStateHandle,
    use_agent_footer_toggle: SwitchStateHandle,
    show_conversation_history_toggle: SwitchStateHandle,
    feedback_bundled_skill_toggle: SwitchStateHandle,
}

impl OtherAIWidget {
    fn create_thinking_display_mode_dropdown(
        ctx: &mut ViewContext<AISettingsPageView>,
    ) -> ViewHandle<Dropdown<AISettingsPageAction>> {
        ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);
            dropdown.set_menu_max_height(AI_SETTINGS_DROPDOWN_MAX_HEIGHT, ctx);
            dropdown.add_items(thinking_display_mode_dropdown_items(ctx), ctx);
            dropdown
        })
    }
}

impl SettingsWidget for OtherAIWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "other oz updates zero state empty changelog new conversation agent what's new use agent footer toolbar layout chip chips rearrange re-arrange thinking expanded reasoning collapse never show hide conversation history feedback skill bundled github issue"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);
        let is_toggleable = is_any_ai_enabled;

        let mut column = Flex::column()
            .with_child(render_separator(appearance))
            .with_child(
                build_sub_header(
                    appearance,
                    i18n::tr(app, I18nKey::AiOther),
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            );

        if FeatureFlag::AgentView.is_enabled() {
            let mut agent_view_column = Flex::column()
                .with_child(render_ai_setting_toggle::<ShouldShowOzUpdatesInZeroState>(
                    i18n::tr(app, I18nKey::AiShowOzChangelog),
                    AISettingsPageAction::ToggleShowOzUpdatesInZeroState,
                    *ai_settings.should_show_oz_updates_in_zero_state,
                    is_toggleable,
                    self.show_oz_updates_in_zero_state_toggle.clone(),
                    &view.local_only_icon_tooltip_states,
                    app,
                ))
                .with_child(render_ai_setting_toggle::<
                    ShouldRenderUseAgentToolbarForUserCommands,
                >(
                    i18n::tr(app, I18nKey::AiShowUseAgentFooter),
                    AISettingsPageAction::ToggleUseAgentToolbar,
                    *ai_settings.should_render_use_agent_footer_for_user_commands,
                    is_toggleable,
                    self.use_agent_footer_toggle.clone(),
                    &view.local_only_icon_tooltip_states,
                    app,
                ))
                .with_child(render_ai_setting_description(
                    i18n::tr(app, I18nKey::AiUseAgentFooterDescription),
                    is_toggleable,
                    app,
                ));

            if is_toggleable && FeatureFlag::AgentToolbarEditor.is_enabled() {
                agent_view_column.add_child(render_toolbar_layout_editor(
                    &view.agent_toolbar_inline_editor,
                    appearance,
                    app,
                ));
            }

            column.add_child(agent_view_column.finish());
        }

        column.add_child(render_ai_setting_toggle::<ShowConversationHistory>(
            i18n::tr(app, I18nKey::AiShowConversationHistory),
            AISettingsPageAction::ToggleShowConversationHistory,
            *ai_settings.show_conversation_history,
            is_toggleable,
            self.show_conversation_history_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        ));
        column.add_child(render_ai_setting_toggle::<FeedbackBundledSkillEnabled>(
            "Enable built-in feedback skill",
            AISettingsPageAction::ToggleFeedbackBundledSkill,
            *ai_settings.feedback_bundled_skill_enabled,
            is_toggleable,
            self.feedback_bundled_skill_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        ));
        column.add_child(render_ai_setting_description(
            "Let Oz use Warp's built-in skill for turning Warp product feedback into GitHub issues.",
            is_toggleable,
            app,
        ));

        column.add_child(render_dropdown_item(
            appearance,
            i18n::tr(app, I18nKey::AiAgentThinkingDisplay),
            Some(i18n::tr(app, I18nKey::AiAgentThinkingDisplayDescription)),
            None,
            LocalOnlyIconState::for_setting(
                ThinkingDisplayMode::storage_key(),
                ThinkingDisplayMode::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            (!is_any_ai_enabled).then(|| appearance.theme().disabled_ui_text_color()),
            &view.thinking_display_mode_dropdown,
        ));

        // TODO: OpenConversationLayoutPreference should not depend on local_fs, but it lives under the external editor settings
        // which does require local_fs. It was a mistake to put it there, but now we keep it there for backward compatibility.
        #[cfg(feature = "local_fs")]
        if FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
            use crate::util::file::external_editor::settings::OpenConversationLayoutPreference;

            column.add_child(render_dropdown_item(
                appearance,
                i18n::tr(app, I18nKey::AiPreferredConversationLayout),
                None,
                None,
                LocalOnlyIconState::for_setting(
                    OpenConversationLayoutPreference::storage_key(),
                    OpenConversationLayoutPreference::sync_to_cloud(),
                    &mut view.local_only_icon_tooltip_states.borrow_mut(),
                    app,
                ),
                (!is_any_ai_enabled).then(|| appearance.theme().disabled_ui_text_color()),
                &view.conversation_layout_dropdown,
            ));
        }

        column.finish()
    }
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn cli_agent_settings_widget_id() -> &'static str {
    CLIAgentWidget::static_widget_id()
}

#[derive(Default)]
struct CLIAgentWidget {
    cli_agent_footer_toggle: SwitchStateHandle,
    auto_toggle_rich_input_toggle: SwitchStateHandle,
    auto_toggle_rich_input_info_tooltip: MouseStateHandle,
    auto_open_rich_input_on_cli_agent_start_toggle: SwitchStateHandle,
    auto_dismiss_rich_input_toggle: SwitchStateHandle,
}

impl SettingsWidget for CLIAgentWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "third party cli coding agent claude codex gemini toolbar footer layout chip chips rearrange re-arrange bar command regex auto show rich input dismiss"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);

        // The Coding Agents section is always enabled, independent of the
        // global AI toggle, because these settings control third-party coding
        // agents (Claude Code, Codex, Gemini CLI) rather than Warp's own AI.
        let cli_agent_footer_toggle = render_ai_setting_toggle::<ShouldRenderCLIAgentToolbar>(
            i18n::tr(app, I18nKey::AiShowCodingAgentToolbar),
            AISettingsPageAction::ToggleCLIAgentToolbar,
            *ai_settings.should_render_cli_agent_footer,
            true,
            self.cli_agent_footer_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        );

        let description_fragments = vec![
            FormattedTextFragment::plain_text(i18n::tr(
                app,
                I18nKey::AiCodingAgentToolbarDescriptionPrefix,
            )),
            FormattedTextFragment::inline_code("claude"),
            FormattedTextFragment::plain_text(", "),
            FormattedTextFragment::inline_code("codex"),
            FormattedTextFragment::plain_text(i18n::tr(
                app,
                I18nKey::AiCodingAgentToolbarDescriptionMiddle,
            )),
            FormattedTextFragment::inline_code("gemini"),
            FormattedTextFragment::plain_text(i18n::tr(
                app,
                I18nKey::AiCodingAgentToolbarDescriptionSuffix,
            )),
        ];

        let description = FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(description_fragments)]),
            appearance.ui_font_size(),
            appearance.ui_font_family(),
            appearance.monospace_font_family(),
            styles::description_font_color(true, app).into(),
            HighlightedHyperlink::default(),
        );

        let is_footer_enabled = *ai_settings.should_render_cli_agent_footer;

        let mut column = Flex::column()
            .with_child(
                build_sub_header(
                    appearance,
                    i18n::tr(app, I18nKey::AiThirdPartyCliAgents),
                    Some(styles::header_font_color(true, app)),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            )
            .with_child(cli_agent_footer_toggle)
            .with_child(
                Container::new(description.finish())
                    .with_margin_top(styles::DESCRIPTION_NEGATIVE_MARGIN_OFFSET)
                    .with_margin_bottom(styles::DESCRIPTION_MARGIN_BOTTOM)
                    .with_margin_right(styles::TOGGLE_WIDTH_MARGIN)
                    .finish(),
            );

        if is_footer_enabled {
            use super::settings_page::AdditionalInfo;
            use crate::settings::{
                AutoDismissRichInputAfterSubmit, AutoOpenRichInputOnCLIAgentStart,
                AutoToggleRichInput,
            };

            if FeatureFlag::CLIAgentRichInput.is_enabled() {
                // Setting 1: Auto show/hide rich input based on agent status
                let auto_show_toggle_label = render_body_item_label::<AISettingsPageAction>(
                    i18n::tr(app, I18nKey::AiAutoShowHideRichInput).into(),
                    Some(styles::header_font_color(true, app)),
                    Some(AdditionalInfo {
                        mouse_state: self.auto_toggle_rich_input_info_tooltip.clone(),
                        on_click_action: None,
                        secondary_text: None,
                        tooltip_override_text: Some(
                            i18n::tr(app, I18nKey::AiRequiresWarpPlugin).to_owned(),
                        ),
                    }),
                    LocalOnlyIconState::for_setting(
                        AutoToggleRichInput::storage_key(),
                        AutoToggleRichInput::sync_to_cloud(),
                        &mut view.local_only_icon_tooltip_states.borrow_mut(),
                        app,
                    ),
                    ToggleState::Enabled,
                    appearance,
                );
                column.add_child(build_toggle_element(
                    auto_show_toggle_label,
                    render_ai_feature_switch(
                        self.auto_toggle_rich_input_toggle.clone(),
                        *ai_settings.auto_toggle_rich_input,
                        true,
                        AISettingsPageAction::ToggleAutoToggleRichInput,
                        app,
                    ),
                    appearance,
                    None,
                ));

                column.add_child(
                    render_ai_setting_toggle::<AutoOpenRichInputOnCLIAgentStart>(
                        i18n::tr(app, I18nKey::AiAutoOpenRichInput),
                        AISettingsPageAction::ToggleAutoOpenRichInputOnCLIAgentStart,
                        *ai_settings.auto_open_rich_input_on_cli_agent_start,
                        true,
                        self.auto_open_rich_input_on_cli_agent_start_toggle.clone(),
                        &view.local_only_icon_tooltip_states,
                        app,
                    ),
                );

                // Setting 2: Auto dismiss rich input after prompt submission
                column.add_child(render_ai_setting_toggle::<AutoDismissRichInputAfterSubmit>(
                    i18n::tr(app, I18nKey::AiAutoDismissRichInput),
                    AISettingsPageAction::ToggleAutoDismissRichInputAfterSubmit,
                    *ai_settings.auto_dismiss_rich_input_after_submit,
                    true,
                    self.auto_dismiss_rich_input_toggle.clone(),
                    &view.local_only_icon_tooltip_states,
                    app,
                ));
            }

            let command_list = {
                let mut list_column = Flex::column();

                list_column.add_child(
                    appearance
                        .ui_builder()
                        .span(i18n::tr(app, I18nKey::AiCommandsEnableToolbar).to_string())
                        .with_style(UiComponentStyles {
                            font_size: Some(CONTENT_FONT_SIZE),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                );

                list_column
                    .add_child(ChildView::new(&view.cli_agent_footer_command_editor).finish());

                let background = appearance.theme().surface_1();
                let font_color = appearance.theme().foreground();
                let items: Vec<_> = ai_settings
                    .cli_agent_footer_enabled_commands
                    .value()
                    .keys()
                    .cloned()
                    .collect();
                let len = items.len();
                for (rev_i, pattern) in items.iter().rev().enumerate() {
                    let original_i = len - 1 - rev_i;
                    let remove_action =
                        AISettingsPageAction::RemoveCLIAgentToolbarEnabledCommand(pattern.clone());
                    let mouse_state = view
                        .cli_agent_footer_command_mouse_state_handles
                        .get(original_i)
                        .cloned()
                        .unwrap_or_default();

                    let remove_button = appearance
                        .ui_builder()
                        .close_button(16., mouse_state)
                        .build()
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(remove_action.clone());
                        })
                        .finish();

                    let label = appearance
                        .ui_builder()
                        .wrappable_text(pattern.clone(), true)
                        .with_style(UiComponentStyles {
                            font_color: Some(font_color.into_solid()),
                            font_family_id: Some(appearance.monospace_font_family()),
                            font_size: Some(appearance.ui_font_size()),
                            ..Default::default()
                        })
                        .build()
                        .finish();

                    let mut right_side =
                        Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
                    if let Some(dropdown_handle) = view
                        .cli_agent_footer_command_agent_dropdowns
                        .get(original_i)
                    {
                        right_side.add_child(
                            Container::new(ChildView::new(dropdown_handle).finish())
                                .with_margin_right(8.)
                                .finish(),
                        );
                    }
                    right_side.add_child(remove_button);

                    let row = Container::new(
                        Flex::row()
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                            .with_children([
                                Shrinkable::new(1., label).finish(),
                                right_side.finish(),
                            ])
                            .finish(),
                    )
                    .with_background(background)
                    .with_horizontal_padding(8.)
                    .with_vertical_padding(4.)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .with_margin_bottom(4.)
                    .finish();

                    list_column.add_child(row);
                }

                list_column.finish()
            };
            let command_list_description = appearance
                .ui_builder()
                .paragraph(i18n::tr(app, I18nKey::AiToolbarCommandDescription))
                .with_style(UiComponentStyles {
                    font_size: Some(appearance.ui_font_size()),
                    font_color: Some(styles::description_font_color(true, app).into()),
                    margin: Some(
                        Coords::default()
                            .top(4.)
                            .bottom(styles::DESCRIPTION_MARGIN_BOTTOM)
                            .right(styles::TOGGLE_WIDTH_MARGIN),
                    ),
                    ..Default::default()
                })
                .build()
                .finish();

            column.add_child(command_list);
            column.add_child(command_list_description);

            if FeatureFlag::AgentToolbarEditor.is_enabled() {
                column.add_child(render_toolbar_layout_editor(
                    &view.cli_agent_toolbar_inline_editor,
                    appearance,
                    app,
                ));
            }
        }

        column.finish()
    }
}

/// The presentation state of the agent attribution toggle, derived from the
/// org-level [`AdminEnablementSetting`], the user's stored preference, and
/// whether AI is globally enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AgentAttributionToggleState {
    /// Whether the toggle is rendered in the checked state.
    pub(crate) is_enabled: bool,
    /// Whether the org has forced the value (locking the toggle with a tooltip).
    pub(crate) is_forced_by_org: bool,
    /// Whether the toggle should be rendered as non-interactive overall
    /// (forced by the org, or AI globally disabled).
    pub(crate) is_disabled: bool,
}

/// Derive the toggle state from its three inputs.
pub(crate) fn derive_agent_attribution_toggle_state(
    org_setting: &AdminEnablementSetting,
    user_pref: bool,
    is_any_ai_enabled: bool,
) -> AgentAttributionToggleState {
    let is_forced_by_org = match org_setting {
        AdminEnablementSetting::Enable | AdminEnablementSetting::Disable => true,
        AdminEnablementSetting::RespectUserSetting => false,
    };
    let is_enabled = match org_setting {
        AdminEnablementSetting::Enable => true,
        AdminEnablementSetting::Disable => false,
        AdminEnablementSetting::RespectUserSetting => user_pref,
    };
    AgentAttributionToggleState {
        is_enabled,
        is_forced_by_org,
        is_disabled: is_forced_by_org || !is_any_ai_enabled,
    }
}

#[derive(Default)]
struct AgentAttributionWidget {
    toggle: SwitchStateHandle,
}

impl SettingsWidget for AgentAttributionWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "agent attribution commit pull request co-author author credit oz warp"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);

        let org_setting = UserWorkspaces::as_ref(app).get_agent_attribution_setting();
        let state = derive_agent_attribution_toggle_state(
            &org_setting,
            *ai_settings.agent_attribution_enabled,
            is_any_ai_enabled,
        );

        let ui_builder = appearance.ui_builder();
        let toggle = if state.is_forced_by_org {
            ui_builder
                .switch(self.toggle.clone())
                .check(state.is_enabled)
                .with_tooltip(TooltipConfig {
                    text: i18n::tr(app, I18nKey::AiOrgEnforcedTooltip).to_string(),
                    styles: ui_builder.default_tool_tip_styles(),
                })
                .disable()
                .build()
                .finish()
        } else if !is_any_ai_enabled {
            ui_builder
                .switch(self.toggle.clone())
                .check(state.is_enabled)
                .with_disabled(true)
                .build()
                .finish()
        } else {
            ui_builder
                .switch(self.toggle.clone())
                .check(state.is_enabled)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AISettingsPageAction::ToggleAgentAttribution);
                })
                .finish()
        };

        let toggle_row = build_toggle_element(
            render_body_item_label::<AISettingsPageAction>(
                i18n::tr(app, I18nKey::AiEnableAgentAttribution).to_string(),
                Some(styles::header_font_color(!state.is_disabled, app)),
                None,
                LocalOnlyIconState::Hidden,
                ToggleState::Enabled,
                appearance,
            ),
            toggle,
            appearance,
            None,
        );

        Flex::column()
            .with_child(render_separator(appearance))
            .with_child(
                build_sub_header(
                    appearance,
                    i18n::tr(app, I18nKey::AiAgentAttribution),
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            )
            .with_child(toggle_row)
            .with_child(render_ai_setting_description(
                i18n::tr(app, I18nKey::AiAgentAttributionDescription),
                !state.is_disabled,
                app,
            ))
            .finish()
    }
}

#[cfg(test)]
#[path = "ai_page_tests.rs"]
mod tests;

#[derive(Default)]
struct CloudAgentComputerUseWidget {
    toggle: SwitchStateHandle,
}

impl SettingsWidget for CloudAgentComputerUseWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "oz cloud agent computer use orchestration multi-agent"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        use crate::ai::execution_profiles::{CloudAgentComputerUseState, ComputerUsePermission};

        let is_any_ai_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);

        // Determine toggle state based on workspace autonomy setting and user preference
        let CloudAgentComputerUseState {
            enabled: is_checked,
            is_forced_by_org,
        } = ComputerUsePermission::resolve_cloud_agent_state(app);

        // Toggle is disabled if forced by org settings OR if AI is globally disabled
        let is_disabled = is_forced_by_org || !is_any_ai_enabled;

        let ui_builder = appearance.ui_builder();
        let toggle = if is_forced_by_org {
            // Disabled by organization setting - show tooltip on hover
            ui_builder
                .switch(self.toggle.clone())
                .check(is_checked)
                .with_tooltip(TooltipConfig {
                    text: i18n::tr(app, I18nKey::AiOrgEnforcedTooltip).to_string(),
                    styles: ui_builder.default_tool_tip_styles(),
                })
                .disable()
                .build()
                .finish()
        } else if !is_any_ai_enabled {
            // Disabled because AI is off globally - no tooltip needed
            ui_builder
                .switch(self.toggle.clone())
                .check(is_checked)
                .with_disabled(true)
                .build()
                .finish()
        } else {
            // Enabled - allow toggling
            ui_builder
                .switch(self.toggle.clone())
                .check(is_checked)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AISettingsPageAction::ToggleCloudAgentComputerUse);
                })
                .finish()
        };

        let toggle_row = build_toggle_element(
            render_body_item_label::<AISettingsPageAction>(
                i18n::tr(app, I18nKey::AiComputerUseCloudAgents).to_string(),
                Some(styles::header_font_color(!is_disabled, app)),
                None,
                LocalOnlyIconState::Hidden,
                ToggleState::Enabled,
                appearance,
            ),
            toggle,
            appearance,
            None,
        );

        Flex::column()
            .with_child(render_separator(appearance))
            .with_child(
                build_sub_header(
                    appearance,
                    i18n::tr(app, I18nKey::AiExperimental),
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            )
            .with_child(toggle_row)
            .with_child(render_ai_setting_description(
                i18n::tr(app, I18nKey::AiCloudComputerUseDescription),
                !is_disabled,
                app,
            ))
            .finish()
    }
}

#[derive(Default)]
struct CloudHandoffWidget {
    handoff_toggle: SwitchStateHandle,
    auto_handoff_on_sleep_toggle: SwitchStateHandle,
    ampersand_toggle: SwitchStateHandle,
}

impl SettingsWidget for CloudHandoffWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "cloud handoff auto sleep ampersand & move to cloud local"
    }

    fn should_render(&self, _app: &AppContext) -> bool {
        FeatureFlag::OzHandoff.is_enabled() && FeatureFlag::HandoffLocalCloud.is_enabled()
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        use crate::settings::PrivacySettings;

        let ai_settings = AISettings::as_ref(app);
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);

        let privacy = PrivacySettings::as_ref(app);
        let cloud_convos_off = !privacy.is_cloud_conversation_storage_enabled
            || matches!(
                UserWorkspaces::as_ref(app).get_cloud_conversation_storage_enablement_setting(),
                AdminEnablementSetting::Disable
            );
        let is_force_disabled = !is_any_ai_enabled || cloud_convos_off;

        let tooltip_text = if cloud_convos_off {
            "Cloud handoff requires cloud conversations to be enabled."
        } else {
            ""
        };

        let ui_builder = appearance.ui_builder();

        let handoff_toggle = if is_force_disabled {
            let mut builder = ui_builder.switch(self.handoff_toggle.clone()).check(false);
            if !tooltip_text.is_empty() {
                builder = builder.with_tooltip(TooltipConfig {
                    text: tooltip_text.to_string(),
                    styles: ui_builder.default_tool_tip_styles(),
                });
            }
            builder.disable().build().finish()
        } else {
            ui_builder
                .switch(self.handoff_toggle.clone())
                .check(!*ai_settings.should_force_disable_cloud_handoff)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AISettingsPageAction::ToggleCloudHandoff);
                })
                .finish()
        };

        let handoff_row = build_toggle_element(
            render_body_item_label::<AISettingsPageAction>(
                "Cloud handoff".to_string(),
                Some(styles::header_font_color(!is_force_disabled, app)),
                None,
                LocalOnlyIconState::Hidden,
                ToggleState::Enabled,
                appearance,
            ),
            handoff_toggle,
            appearance,
            None,
        );

        let mut column = Flex::column()
            .with_child(render_separator(appearance))
            .with_child(
                build_sub_header(
                    appearance,
                    "Cloud Handoff",
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            )
            .with_child(handoff_row)
            .with_child(render_ai_setting_description(
                "Hand off local agent conversations to a cloud agent.",
                !is_force_disabled,
                app,
            ));

        if ai_settings.is_cloud_handoff_enabled(app) {
            if ai_settings
                .auto_handoff_on_sleep_enabled
                .is_supported_on_current_platform()
            {
                let auto_handoff_on_sleep_toggle = ui_builder
                    .switch(self.auto_handoff_on_sleep_toggle.clone())
                    .check(*ai_settings.auto_handoff_on_sleep_enabled)
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(AISettingsPageAction::ToggleAutoHandoffOnSleep);
                    })
                    .finish();
                let auto_handoff_on_sleep_row = build_toggle_element(
                    render_body_item_label::<AISettingsPageAction>(
                        "Auto-handoff before sleep".to_string(),
                        Some(styles::header_font_color(true, app)),
                        None,
                        LocalOnlyIconState::Hidden,
                        ToggleState::Enabled,
                        appearance,
                    ),
                    auto_handoff_on_sleep_toggle,
                    appearance,
                    None,
                );
                column.add_child(auto_handoff_on_sleep_row);
                column.add_child(render_ai_setting_description(
                    "When macOS is about to sleep, automatically moves the most recently focused running local Warp Agent conversation to Cloud Mode so it can keep working.",
                    true,
                    app,
                ));
            }
            let ampersand_toggle = ui_builder
                .switch(self.ampersand_toggle.clone())
                .check(!*ai_settings.should_force_disable_ampersand_handoff)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(AISettingsPageAction::ToggleAmpersandHandoff);
                })
                .finish();

            let ampersand_row = build_toggle_element(
                render_body_item_label::<AISettingsPageAction>(
                    "Use & to trigger handoff".to_string(),
                    Some(styles::header_font_color(true, app)),
                    None,
                    LocalOnlyIconState::Hidden,
                    ToggleState::Enabled,
                    appearance,
                ),
                ampersand_toggle,
                appearance,
                None,
            );

            column.add_child(ampersand_row);
            column.add_child(render_ai_setting_description(
                "Type & as the first character to enter cloud handoff compose mode.",
                true,
                app,
            ));
        }

        column.finish()
    }
}

struct ApiKeysWidget {
    openai_api_key_editor: ViewHandle<EditorView>,
    anthropic_api_key_editor: ViewHandle<EditorView>,
    google_api_key_editor: ViewHandle<EditorView>,

    can_use_warp_credits_for_fallback: SwitchStateHandle,
    upgrade_highlight_index: HighlightedHyperlink,

    custom_inference_info_tooltip: MouseStateHandle,
    custom_inference_terms_index: HighlightedHyperlink,
    description_learn_more_index: HighlightedHyperlink,
}

impl ApiKeysWidget {
    fn new(ctx: &mut ViewContext<<Self as SettingsWidget>::View>) -> Self {
        let ai_settings = AISettings::as_ref(ctx);
        let workspace_handle = UserWorkspaces::handle(ctx);
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(ctx);
        let is_byo_enabled = workspace_handle.as_ref(ctx).is_byo_api_key_enabled(ctx);

        let ApiKeys {
            openai: openai_key,
            anthropic: anthropic_key,
            google: google_key,
            ..
        } = ApiKeyManager::as_ref(ctx).keys().clone();

        // A helper macro to create and configure an API key editor.  This avoids a lot
        // of code duplication and ensures consistency between the editors.
        macro_rules! create_api_key_editor {
            ($editor:ident, $key:ident, $set_func:ident, $placeholder:literal) => {
                let $editor = ctx.add_typed_action_view(move |ctx| {
                    let appearance = Appearance::handle(ctx).as_ref(ctx);
                    let options = SingleLineEditorOptions {
                        is_password: true,
                        text: TextOptions {
                            font_size_override: Some(appearance.ui_font_size()),
                            font_family_override: Some(appearance.monospace_font_family()),
                            text_colors_override: Some(TextColors {
                                default_color: appearance.theme().active_ui_text_color(),
                                disabled_color: appearance.theme().disabled_ui_text_color(),
                                hint_color: appearance.theme().disabled_ui_text_color(),
                            }),
                            ..Default::default()
                        },
                        ..Default::default()
                    };
                    let mut editor = EditorView::single_line(options, ctx);
                    editor.set_placeholder_text($placeholder, ctx);
                    if let Some(key) = &$key {
                        editor.set_buffer_text(key, ctx);
                    }
                    editor
                });
                AISettingsPageView::update_editor_interaction_state(
                    $editor.clone(),
                    is_any_ai_enabled && is_byo_enabled,
                    ctx,
                );
                ctx.subscribe_to_view(&$editor, |_, $editor, event, ctx| {
                    if matches!(event, EditorEvent::Blurred | EditorEvent::Enter) {
                        let buffer_text = $editor.as_ref(ctx).buffer_text(ctx);
                        let key = buffer_text.is_empty().not().then_some(buffer_text);
                        ApiKeyManager::handle(ctx).update(ctx, |model, ctx| {
                            model.$set_func(key, ctx);
                        });
                    }
                });
                let editor_clone = $editor.clone();
                ctx.subscribe_to_model(&workspace_handle, move |_, workspace, event, ctx| {
                    if let UserWorkspacesEvent::TeamsChanged = event {
                        let is_any_ai_enabled =
                            AISettings::handle(ctx).as_ref(ctx).is_any_ai_enabled(ctx);
                        let is_byo_enabled = workspace.as_ref(ctx).is_byo_api_key_enabled(ctx);
                        let is_enabled = is_any_ai_enabled && is_byo_enabled;
                        let has_key = !editor_clone.as_ref(ctx).is_empty(ctx);

                        // If BYO is disabled, clear the API key from the editor and storage
                        if !is_byo_enabled && has_key {
                            editor_clone.update(ctx, |editor, ctx| {
                                editor.set_buffer_text("", ctx);
                            });
                            ApiKeyManager::handle(ctx).update(ctx, |model, ctx| {
                                model.$set_func(None, ctx);
                            });
                        }

                        AISettingsPageView::update_editor_interaction_state(
                            editor_clone.clone(),
                            is_enabled,
                            ctx,
                        );
                        ctx.notify();
                    }
                })
            };
        }

        create_api_key_editor!(openai_api_key_editor, openai_key, set_openai_key, "sk-...");
        create_api_key_editor!(
            anthropic_api_key_editor,
            anthropic_key,
            set_anthropic_key,
            "sk-ant-..."
        );
        create_api_key_editor!(
            google_api_key_editor,
            google_key,
            set_google_key,
            "AIzaSy..."
        );

        Self {
            openai_api_key_editor,
            anthropic_api_key_editor,
            google_api_key_editor,

            can_use_warp_credits_for_fallback: Default::default(),
            upgrade_highlight_index: Default::default(),

            custom_inference_info_tooltip: Default::default(),
            custom_inference_terms_index: Default::default(),
            description_learn_more_index: Default::default(),
        }
    }

    fn render_api_key_input(
        &self,
        appearance: &Appearance,
        label: &'static str,
        editor: ViewHandle<EditorView>,
        is_enabled: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let padding = Some(Coords {
            top: 10.,
            bottom: 10.,
            left: 16.,
            right: 16.,
        });
        let editor_style = UiComponentStyles {
            padding,
            background: Some(appearance.theme().surface_2().into()),
            ..Default::default()
        };

        let label = Text::new_inline(label, appearance.ui_font_family(), CONTENT_FONT_SIZE)
            .with_color(styles::header_font_color(is_enabled, app).into())
            .finish();

        let input = appearance
            .ui_builder()
            .text_input(editor)
            .with_style(editor_style)
            .build()
            .finish();

        Flex::column()
            .with_spacing(8.)
            .with_child(label)
            .with_child(input)
            .finish()
    }

    fn render_provider_key_editors(
        &self,
        appearance: &Appearance,
        is_enabled: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut column = Flex::column().with_spacing(16.);
        column.add_child(self.render_api_key_input(
            appearance,
            "OpenAI API key",
            self.openai_api_key_editor.clone(),
            is_enabled,
            app,
        ));
        column.add_child(self.render_api_key_input(
            appearance,
            "Anthropic API key",
            self.anthropic_api_key_editor.clone(),
            is_enabled,
            app,
        ));
        column.add_child(self.render_api_key_input(
            appearance,
            "Google API key",
            self.google_api_key_editor.clone(),
            is_enabled,
            app,
        ));
        column.finish()
    }

    fn render_custom_inference_description(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let text_fragments = vec![
            FormattedTextFragment::plain_text(
                "Use your own API keys from model providers for Warp Agent. You can also add custom endpoints to use third-party models. Custom endpoints must support the OpenAI-compatible Chat Completions API. API keys are stored locally and are never synced to the cloud. Using auto models or models from providers you have not provided API keys for will consume Warp credits. ",
            ),
            FormattedTextFragment::hyperlink(
                "Learn more",
                CUSTOM_INFERENCE_LEARN_MORE_URL,
            ),
        ];
        let description = FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(text_fragments)]),
            CONTENT_FONT_SIZE,
            appearance.ui_font_family(),
            appearance.ui_font_family(),
            blended_colors::text_sub(appearance.theme(), appearance.theme().surface_1()),
            self.description_learn_more_index.clone(),
        )
        .with_hyperlink_font_color(appearance.theme().accent().into_solid())
        .register_default_click_handlers(|url, ctx, _| {
            ctx.dispatch_typed_action(AISettingsPageAction::HyperlinkClick(url));
        });
        Container::new(description.finish())
            .with_margin_top(styles::DESCRIPTION_NEGATIVE_MARGIN_OFFSET)
            .with_margin_bottom(styles::DESCRIPTION_MARGIN_BOTTOM)
            .with_margin_right(styles::TOGGLE_WIDTH_MARGIN)
            .finish()
    }

    fn render_custom_inference_info_icon(&self, appearance: &Appearance) -> Box<dyn Element> {
        let icon = Container::new(
            ConstrainedBox::new(
                Icon::Info
                    .to_warpui_icon(appearance.theme().active_ui_text_color())
                    .finish(),
            )
            .with_width(13.)
            .with_height(13.)
            .finish(),
        )
        .finish();

        let tooltip_text = FormattedText::new([FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text(
                "By using BYOK or custom endpoints, you agree to use them only as permitted by ",
            ),
            FormattedTextFragment::hyperlink(
                "Warp's Terms of Service",
                CUSTOM_INFERENCE_TERMS_URL,
            ),
            FormattedTextFragment::plain_text(
                ". BYOK and custom endpoints are intended for individual use and small teams. Companies or organizations with more than 10 employees should use Warp Business or Enterprise.",
            ),
        ])]);
        let tooltip_background = appearance.theme().tooltip_background();

        let info_button =
            Hoverable::new(self.custom_inference_info_tooltip.clone(), move |state| {
                let mut stack = Stack::new().with_child(icon);
                if state.is_hovered() {
                    let tool_tip = ConstrainedBox::new(
                        Container::new(
                            FormattedTextElement::new(
                                tooltip_text.clone(),
                                10.,
                                appearance.ui_font_family(),
                                appearance.ui_font_family(),
                                appearance.theme().background().into_solid(),
                                self.custom_inference_terms_index.clone(),
                            )
                            .with_hyperlink_font_color(
                                appearance
                                    .theme()
                                    .accent()
                                    .on_background(
                                        ThemeFill::Solid(tooltip_background),
                                        MinimumAllowedContrast::Text,
                                    )
                                    .into(),
                            )
                            .register_default_click_handlers(|url, ctx, _| {
                                ctx.dispatch_typed_action(AISettingsPageAction::HyperlinkClick(
                                    url,
                                ));
                            })
                            .finish(),
                        )
                        .with_background_color(tooltip_background)
                        .with_vertical_padding(4.)
                        .with_horizontal_padding(8.)
                        .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                        .finish(),
                    )
                    .with_max_width(CUSTOM_INFERENCE_INFO_TOOLTIP_MAX_WIDTH)
                    .finish();
                    stack.add_positioned_child(
                        tool_tip,
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., -3.),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::TopMiddle,
                            ChildAnchor::BottomMiddle,
                        ),
                    );
                }
                stack.finish()
            })
            .with_cursor(Cursor::PointingHand);

        Container::new(Box::new(info_button))
            .with_margin_left(4.)
            .finish()
    }

    fn render_custom_endpoints_list(
        &self,
        view: &AISettingsPageView,
        appearance: &Appearance,
        is_enabled: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_color = styles::header_font_color(is_enabled, app);
        let endpoints = &ApiKeyManager::as_ref(app).keys().custom_endpoints;
        let chip_border = internal_colors::fg_overlay_3(theme);

        let mut list = Flex::column().with_spacing(12.);
        for (index, endpoint) in endpoints.iter().enumerate() {
            let model_labels = endpoint
                .models
                .iter()
                .map(|model| model.alias.clone().unwrap_or_else(|| model.name.clone()))
                .filter(|s| !s.trim().is_empty());

            let chips = super::render_model_chips(model_labels, appearance, text_color);

            let endpoint_name = Text::new_inline(
                endpoint.name.clone(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_style(Properties::default().weight(Weight::Semibold))
            .with_color(text_color.into())
            .finish();

            let left = Flex::column()
                .with_spacing(8.)
                .with_child(endpoint_name)
                .with_child(chips)
                .finish();

            let edit_button = view
                .custom_endpoint_edit_buttons
                .get(index)
                .map(|button| button.as_ref(app).render(app))
                .unwrap_or_else(|| Empty::new().finish());

            let row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Shrinkable::new(1., left).finish())
                .with_child(edit_button)
                .finish();

            list.add_child(
                Container::new(row)
                    .with_uniform_padding(12.)
                    .with_background(internal_colors::fg_overlay_1(theme))
                    .with_border(Border::all(1.).with_border_fill(chip_border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                    .finish(),
            );
        }
        list.finish()
    }

    fn render_warp_credit_fallback_toggle(
        &self,
        view: &AISettingsPageView,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);

        let toggle = render_ai_setting_toggle::<CanUseWarpCreditsForFallback>(
            "Warp credit fallback",
            AISettingsPageAction::ToggleCanUseWarpCreditsForFallback,
            *ai_settings.can_use_warp_credits_for_fallback,
            ai_settings.is_any_ai_enabled(app),
            self.can_use_warp_credits_for_fallback.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        );

        let description = render_ai_setting_description(
            "When enabled, agent requests may be routed to one of Warp's provided models in the event of an error. Warp will prioritize using your API keys over your Warp credits.",
            ai_settings.is_any_ai_enabled(app),
            app,
        );

        Flex::column()
            .with_child(toggle)
            .with_child(description)
            .finish()
    }
}

impl SettingsWidget for ApiKeysWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "api keys bring your own byo openai anthropic google claude gemini gpt custom inference endpoint"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);
        let is_byo_enabled = UserWorkspaces::as_ref(app).is_byo_api_key_enabled(app);
        let is_custom_inference_enabled =
            UserWorkspaces::as_ref(app).is_custom_inference_enabled(app);
        let provider_keys_enabled = is_any_ai_enabled && is_byo_enabled;
        let custom_inference_controls_enabled = is_any_ai_enabled && is_custom_inference_enabled;
        let custom_inference_flag_on = FeatureFlag::CustomInferenceEndpoints.is_enabled();
        let show_custom_inference = custom_inference_flag_on && is_custom_inference_enabled;

        let mut column = Flex::column().with_child(render_separator(appearance));

        if show_custom_inference {
            // Header row: "Custom inference" + info icon on left, "+ Add custom model" on right
            let header_left = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    build_sub_header(
                        appearance,
                        "Custom inference",
                        Some(styles::header_font_color(
                            custom_inference_controls_enabled,
                            app,
                        )),
                    )
                    .with_margin_bottom(0.)
                    .finish(),
                )
                .with_child(self.render_custom_inference_info_icon(appearance))
                .finish();

            let header_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(header_left)
                .with_child(view.custom_inference_add_button.as_ref(app).render(app))
                .finish();

            column.add_child(
                Container::new(header_row)
                    .with_padding_bottom(HEADER_PADDING)
                    .finish(),
            );

            // Description with Learn more link
            column.add_child(self.render_custom_inference_description(app));
        } else {
            // Fallback: old "API Keys" header only
            column.add_child(
                build_sub_header(
                    appearance,
                    "API Keys",
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            );
        }

        // Provider key editors (always visible)
        column.add_child(self.render_provider_key_editors(appearance, provider_keys_enabled, app));

        // Custom endpoints sub-label + list (only when flag on and endpoints non-empty)
        if show_custom_inference {
            let endpoints = &ApiKeyManager::as_ref(app).keys().custom_endpoints;
            if !endpoints.is_empty() {
                column.add_child(
                    Container::new(
                        Text::new_inline(
                            "Custom endpoints",
                            appearance.ui_font_family(),
                            CONTENT_FONT_SIZE,
                        )
                        .with_color(
                            styles::header_font_color(custom_inference_controls_enabled, app)
                                .into(),
                        )
                        .with_style(Properties::default().weight(Weight::Semibold))
                        .finish(),
                    )
                    .with_margin_top(16.)
                    .with_margin_bottom(8.)
                    .finish(),
                );
                column.add_child(self.render_custom_endpoints_list(
                    view,
                    appearance,
                    custom_inference_controls_enabled,
                    app,
                ));
            }
        }

        // Warp credit fallback toggle (shown when BYO or custom inference is enabled)
        if is_byo_enabled || show_custom_inference {
            column.add_child(
                Container::new(self.render_warp_credit_fallback_toggle(view, app))
                    .with_margin_top(16.)
                    .finish(),
            );
        }

        // Upgrade CTA if BYOK not enabled
        if !is_byo_enabled {
            let auth_state = AuthStateProvider::as_ref(app).get();
            let upgrade_text_fragments = if let Some(team) =
                UserWorkspaces::as_ref(app).current_team()
            {
                if team.billing_metadata.customer_type == CustomerType::Enterprise {
                    vec![
                        FormattedTextFragment::hyperlink("Contact sales", "mailto:sales@warp.dev"),
                        FormattedTextFragment::plain_text(
                            " to enable bringing your own API keys on your Enterprise plan.",
                        ),
                    ]
                } else {
                    let current_user_email = auth_state.user_email().unwrap_or_default();
                    let has_admin_permissions = team.has_admin_permissions(&current_user_email);
                    let upgrade_url = UserWorkspaces::upgrade_link_for_team(team.uid);
                    if has_admin_permissions {
                        vec![
                            FormattedTextFragment::hyperlink(
                                i18n::tr(app, I18nKey::AiUpgradeBuildPlan),
                                upgrade_url,
                            ),
                            FormattedTextFragment::plain_text(i18n::tr(
                                app,
                                I18nKey::AiUseOwnApiKeys,
                            )),
                        ]
                    } else {
                        vec![FormattedTextFragment::plain_text(i18n::tr(
                            app,
                            I18nKey::AiAskAdminUpgradeBuild,
                        ))]
                    }
                }
            } else if FeatureFlag::SoloUserByok.is_enabled()
                && auth_state.is_anonymous_or_logged_out()
            {
                vec![
                    FormattedTextFragment::hyperlink_action(
                        "Create an account",
                        AISettingsPageAction::SignupAnonymousUser,
                    ),
                    FormattedTextFragment::plain_text(" to use your own API keys."),
                ]
            } else {
                let user_id = auth_state.user_id().unwrap_or_default();
                let upgrade_url = UserWorkspaces::upgrade_link(user_id);
                vec![
                    FormattedTextFragment::hyperlink("Upgrade to the Build plan", upgrade_url),
                    FormattedTextFragment::plain_text(" to use your own API keys."),
                ]
            };

            let upgrade_text_element = FormattedTextElement::new(
                FormattedText::new([FormattedTextLine::Line(upgrade_text_fragments)]),
                appearance.ui_font_size(),
                appearance.ui_font_family(),
                appearance.ui_font_family(),
                blended_colors::text_sub(appearance.theme(), appearance.theme().surface_1()),
                self.upgrade_highlight_index.clone(),
            )
            .with_hyperlink_font_color(appearance.theme().accent().into_solid())
            .register_default_click_handlers_with_action_support(|hyperlink_lens, event, ctx| {
                match hyperlink_lens {
                    HyperlinkLens::Url(url) => {
                        ctx.open_url(url);
                    }
                    HyperlinkLens::Action(action_ref) => {
                        if let Some(action) =
                            action_ref.as_any().downcast_ref::<AISettingsPageAction>()
                        {
                            event.dispatch_typed_action(action.clone());
                        }
                    }
                }
            });

            column.add_child(Container::new(upgrade_text_element.finish()).finish());
        }

        column.finish()
    }
}

struct AwsBedrockWidget {
    aws_auth_refresh_command_editor: ViewHandle<EditorView>,
    aws_auth_refresh_profile_editor: ViewHandle<EditorView>,
    credentials_enabled_toggle: SwitchStateHandle,
    auto_login_toggle: SwitchStateHandle,
    refresh_credentials_button: ViewHandle<ActionButton>,
}

impl AwsBedrockWidget {
    fn new(ctx: &mut ViewContext<<Self as SettingsWidget>::View>) -> Self {
        let ai_settings = AISettings::as_ref(ctx);
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(ctx);

        let aws_auth_refresh_command = ai_settings.aws_bedrock_auth_refresh_command.value().clone();
        let aws_auth_refresh_profile = ai_settings.aws_bedrock_profile.value().clone();
        let is_usage_enabled = is_any_ai_enabled
            && UserWorkspaces::as_ref(ctx).is_aws_bedrock_credentials_enabled(ctx);

        let aws_auth_refresh_command_editor = ctx.add_typed_action_view(move |ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = SingleLineEditorOptions {
                is_password: false,
                text: TextOptions {
                    font_size_override: Some(appearance.ui_font_size()),
                    font_family_override: Some(appearance.monospace_font_family()),
                    text_colors_override: Some(TextColors {
                        default_color: appearance.theme().active_ui_text_color(),
                        disabled_color: appearance.theme().disabled_ui_text_color(),
                        hint_color: appearance.theme().disabled_ui_text_color(),
                    }),
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("aws login", ctx);
            editor.set_buffer_text(&aws_auth_refresh_command, ctx);
            editor
        });
        AISettingsPageView::update_editor_interaction_state(
            aws_auth_refresh_command_editor.clone(),
            is_usage_enabled,
            ctx,
        );
        ctx.subscribe_to_view(&aws_auth_refresh_command_editor, |_, editor, event, ctx| {
            if matches!(event, EditorEvent::Blurred | EditorEvent::Enter) {
                let buffer_text = editor.as_ref(ctx).buffer_text(ctx);
                let should_reset = buffer_text.trim().is_empty();
                let value = if should_reset {
                    "aws login".to_string()
                } else {
                    buffer_text
                };
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings
                        .aws_bedrock_auth_refresh_command
                        .set_value(value, ctx);
                });
                if should_reset {
                    editor.update(ctx, |editor, ctx| {
                        editor.set_buffer_text("aws login", ctx);
                    });
                }
            }
        });

        let aws_auth_refresh_profile_editor = ctx.add_typed_action_view(move |ctx| {
            let appearance = Appearance::as_ref(ctx);
            let options = SingleLineEditorOptions {
                is_password: false,
                text: TextOptions {
                    font_size_override: Some(appearance.ui_font_size()),
                    font_family_override: Some(appearance.monospace_font_family()),
                    text_colors_override: Some(TextColors {
                        default_color: appearance.theme().active_ui_text_color(),
                        disabled_color: appearance.theme().disabled_ui_text_color(),
                        hint_color: appearance.theme().disabled_ui_text_color(),
                    }),
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("default", ctx);
            editor.set_buffer_text(&aws_auth_refresh_profile, ctx);
            editor
        });
        AISettingsPageView::update_editor_interaction_state(
            aws_auth_refresh_profile_editor.clone(),
            is_usage_enabled,
            ctx,
        );
        ctx.subscribe_to_view(&aws_auth_refresh_profile_editor, |_, editor, event, ctx| {
            if matches!(event, EditorEvent::Blurred | EditorEvent::Enter) {
                let buffer_text = editor.as_ref(ctx).buffer_text(ctx);
                let should_reset = buffer_text.trim().is_empty();
                let value = if should_reset {
                    "default".to_string()
                } else {
                    buffer_text
                };
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let _ = settings.aws_bedrock_profile.set_value(value, ctx);
                });
                if should_reset {
                    editor.update(ctx, |editor, ctx| {
                        editor.set_buffer_text("default", ctx);
                    });
                }
            }
        });

        let refresh_credentials_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(i18n::tr(ctx, I18nKey::AiRefresh), SecondaryTheme)
                .with_icon(Icon::RefreshCw04)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AISettingsPageAction::RefreshAwsBedrockCredentials);
                })
        });
        refresh_credentials_button.update(ctx, |button, ctx| {
            button.set_disabled(!is_usage_enabled, ctx);
        });
        let refresh_credentials_button_language = refresh_credentials_button.clone();
        ctx.subscribe_to_model(&LanguageSettings::handle(ctx), move |_, _, _, ctx| {
            refresh_credentials_button_language.update(ctx, |button, ctx| {
                button.set_label(i18n::tr(ctx, I18nKey::AiRefresh), ctx);
            });
        });

        // Keep enablement in sync with the Global AI toggle.
        let aws_auth_refresh_command_editor_clone = aws_auth_refresh_command_editor.clone();
        let aws_auth_refresh_profile_editor_clone = aws_auth_refresh_profile_editor.clone();
        let refresh_credentials_button_clone = refresh_credentials_button.clone();
        ctx.subscribe_to_model(&AISettings::handle(ctx), move |_, _, event, ctx| {
            if matches!(
                event,
                AISettingsChangedEvent::IsAnyAIEnabled { .. }
                    | AISettingsChangedEvent::AwsBedrockCredentialsEnabled { .. }
            ) {
                let is_any_ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
                let is_usage_enabled = is_any_ai_enabled
                    && UserWorkspaces::as_ref(ctx).is_aws_bedrock_credentials_enabled(ctx);

                AISettingsPageView::update_editor_interaction_state(
                    aws_auth_refresh_command_editor_clone.clone(),
                    is_usage_enabled,
                    ctx,
                );
                AISettingsPageView::update_editor_interaction_state(
                    aws_auth_refresh_profile_editor_clone.clone(),
                    is_usage_enabled,
                    ctx,
                );
                refresh_credentials_button_clone.update(ctx, |button, ctx| {
                    button.set_disabled(!is_usage_enabled, ctx);
                });

                ctx.notify();
            }
        });

        let aws_auth_refresh_command_editor_clone = aws_auth_refresh_command_editor.clone();
        let aws_auth_refresh_profile_editor_clone = aws_auth_refresh_profile_editor.clone();
        let refresh_credentials_button_clone = refresh_credentials_button.clone();
        ctx.subscribe_to_model(
            &UserWorkspaces::handle(ctx),
            move |_, workspace, event, ctx| {
                if let UserWorkspacesEvent::TeamsChanged = event {
                    let is_any_ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
                    let is_usage_enabled = is_any_ai_enabled
                        && workspace
                            .as_ref(ctx)
                            .is_aws_bedrock_credentials_enabled(ctx);

                    AISettingsPageView::update_editor_interaction_state(
                        aws_auth_refresh_command_editor_clone.clone(),
                        is_usage_enabled,
                        ctx,
                    );
                    AISettingsPageView::update_editor_interaction_state(
                        aws_auth_refresh_profile_editor_clone.clone(),
                        is_usage_enabled,
                        ctx,
                    );
                    refresh_credentials_button_clone.update(ctx, |button, ctx| {
                        button.set_disabled(!is_usage_enabled, ctx);
                    });

                    ctx.notify();
                }
            },
        );

        Self {
            aws_auth_refresh_command_editor,
            aws_auth_refresh_profile_editor,
            credentials_enabled_toggle: SwitchStateHandle::default(),
            auto_login_toggle: SwitchStateHandle::default(),
            refresh_credentials_button,
        }
    }

    fn render_aws_bedrock_section(
        &self,
        appearance: &Appearance,
        app: &AppContext,
        is_bedrock_available: bool,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);
        let user_workspaces = UserWorkspaces::as_ref(app);
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);
        let is_section_enabled = is_any_ai_enabled && is_bedrock_available;
        let is_admin_enforced = matches!(
            user_workspaces.aws_bedrock_host_enablement_setting(),
            crate::workspaces::workspace::HostEnablementSetting::Enforce
        );
        let is_toggleable =
            is_section_enabled && user_workspaces.is_aws_bedrock_credentials_toggleable();
        let are_credentials_enabled = user_workspaces.is_aws_bedrock_credentials_enabled(app);
        let is_usage_enabled = is_section_enabled && are_credentials_enabled;
        let toggle_description = if is_admin_enforced {
            i18n::tr(app, I18nKey::AiAwsManagedDescription).to_string()
        } else {
            i18n::tr(app, I18nKey::AiAwsDescription).to_string()
        };

        let mut column = Flex::column().with_spacing(16.).with_child(
            Flex::column()
                .with_child(render_ai_setting_toggle::<AwsBedrockCredentialsEnabled>(
                    i18n::tr(app, I18nKey::AiUseAwsBedrockCredentials),
                    AISettingsPageAction::ToggleAwsBedrockCredentialsEnabled,
                    are_credentials_enabled,
                    is_toggleable,
                    self.credentials_enabled_toggle.clone(),
                    &RefCell::new(HashMap::new()),
                    app,
                ))
                .with_child(render_ai_setting_description(
                    toggle_description,
                    is_section_enabled,
                    app,
                ))
                .finish(),
        );

        /// Helper function to render the UI for an input field.
        fn render_input(
            appearance: &Appearance,
            label: &'static str,
            editor: ViewHandle<EditorView>,
            is_enabled: bool,
            app: &AppContext,
        ) -> Box<dyn Element> {
            let padding = Some(Coords {
                top: 10.,
                bottom: 10.,
                left: 16.,
                right: 16.,
            });
            let editor_style = UiComponentStyles {
                padding,
                background: Some(appearance.theme().surface_2().into()),
                ..Default::default()
            };

            let label = Text::new_inline(label, appearance.ui_font_family(), CONTENT_FONT_SIZE)
                .with_color(styles::header_font_color(is_enabled, app).into())
                .finish();

            let input = appearance
                .ui_builder()
                .text_input(editor)
                .with_style(editor_style)
                .build()
                .finish();

            Flex::column()
                .with_spacing(8.)
                .with_child(label)
                .with_child(input)
                .finish()
        }

        fn render_credential_status_card(
            refresh_button: &ViewHandle<ActionButton>,
            appearance: &Appearance,
            are_credentials_enabled: bool,
            app: &AppContext,
        ) -> Box<dyn Element> {
            let (title_color, detail_color) = (
                styles::header_font_color(are_credentials_enabled, app),
                styles::description_font_color(are_credentials_enabled, app),
            );
            let (title_text, detail_text, icon) = ApiKeyManager::as_ref(app)
                .aws_credentials_state()
                .user_facing_components();

            let icon = Container::new(
                ConstrainedBox::new(icon.to_warpui_icon(title_color).finish())
                    .with_width(16.)
                    .with_height(16.)
                    .finish(),
            )
            .with_horizontal_padding(4.)
            .finish();

            let text_column = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(4.)
                .with_child(
                    Text::new_inline(title_text, appearance.ui_font_family(), CONTENT_FONT_SIZE)
                        .with_style(Properties::default().weight(Weight::Semibold))
                        .with_color(title_color.into())
                        .finish(),
                )
                .with_child(
                    Text::new(detail_text, appearance.ui_font_family(), CONTENT_FONT_SIZE)
                        .with_color(detail_color.into())
                        .soft_wrap(true)
                        .finish(),
                );

            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(12.)
                    .with_child(
                        Expanded::new(
                            1.,
                            Flex::row()
                                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                .with_spacing(12.)
                                .with_child(icon)
                                .with_child(Expanded::new(1., text_column.finish()).finish())
                                .finish(),
                        )
                        .finish(),
                    )
                    .with_child(ChildView::new(refresh_button).finish())
                    .finish(),
            )
            .with_uniform_padding(12.)
            .with_background(appearance.theme().surface_2())
            .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .finish()
        }

        column.add_child(
            Container::new(render_credential_status_card(
                &self.refresh_credentials_button,
                appearance,
                are_credentials_enabled,
                app,
            ))
            .with_margin_top(-styles::DESCRIPTION_MARGIN_BOTTOM)
            .finish(),
        );
        column.add_child(render_input(
            appearance,
            i18n::tr(app, I18nKey::AiLoginCommand),
            self.aws_auth_refresh_command_editor.clone(),
            is_usage_enabled,
            app,
        ));
        column.add_child(render_input(
            appearance,
            i18n::tr(app, I18nKey::AiAwsProfile),
            self.aws_auth_refresh_profile_editor.clone(),
            is_usage_enabled,
            app,
        ));

        let auto_login_enabled = *AISettings::as_ref(app).aws_bedrock_auto_login.value();

        let toggle = render_ai_setting_toggle::<AwsBedrockAutoLogin>(
            i18n::tr(app, I18nKey::AiAutoRunLoginCommand),
            AISettingsPageAction::ToggleAwsBedrockAutoLogin,
            auto_login_enabled,
            is_usage_enabled,
            self.auto_login_toggle.clone(),
            &RefCell::new(HashMap::new()),
            app,
        );
        let description = render_ai_setting_description(
            i18n::tr(app, I18nKey::AiAutoRunLoginDescription),
            is_usage_enabled,
            app,
        );
        column.add_child(
            Flex::column()
                .with_child(toggle)
                .with_child(description)
                .finish(),
        );

        column.finish()
    }
}

impl SettingsWidget for AwsBedrockWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "aws bedrock amazon credentials login profile"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        // Only show if admin has enabled AWS Bedrock for the workspace
        UserWorkspaces::as_ref(app).is_aws_bedrock_available_from_workspace()
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);
        let is_bedrock_available =
            UserWorkspaces::as_ref(app).is_aws_bedrock_available_from_workspace();

        let column = Flex::column()
            .with_child(render_separator(appearance))
            .with_child(
                build_sub_header(
                    appearance,
                    i18n::tr(app, I18nKey::AiAwsBedrock),
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            )
            .with_child(self.render_aws_bedrock_section(appearance, app, is_bedrock_available));

        Container::new(column.finish())
            .with_margin_bottom(HEADER_PADDING)
            .finish()
    }
}

mod styles {
    use warp_core::ui::{appearance::Appearance, theme::Fill};
    use warpui::{AppContext, SingletonEntity};

    // Apply a negative margin to the description text so it appears closer to the main
    // settings option text.
    pub const DESCRIPTION_NEGATIVE_MARGIN_OFFSET: f32 = -12.;

    /// The space between a description and the next toggle.
    pub const DESCRIPTION_MARGIN_BOTTOM: f32 = 12.;

    /// Margin to leave for switch toggle to the right of the description subtext.
    pub const TOGGLE_WIDTH_MARGIN: f32 = 48.;

    pub fn header_font_color(is_enabled_setting: bool, app: &AppContext) -> Fill {
        let appearance = Appearance::as_ref(app);
        if is_enabled_setting {
            appearance
                .theme()
                .main_text_color(appearance.theme().surface_2())
        } else {
            appearance.theme().disabled_ui_text_color()
        }
    }

    pub fn description_font_color(is_enabled_setting: bool, app: &AppContext) -> Fill {
        let appearance = Appearance::as_ref(app);
        if is_enabled_setting {
            appearance
                .theme()
                .sub_text_color(appearance.theme().surface_1())
        } else {
            appearance.theme().disabled_ui_text_color()
        }
    }
}
