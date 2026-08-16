//! The "Warp Agent" settings page, shown under the Agents umbrella.
//!
//! Covers Warp's own AI: the global toggle, Active AI suggestions, agent
//! input behavior, voice input, credentials (BYO keys, Bedrock, Gemini
//! Enterprise, custom endpoints, custom routers) and the miscellaneous
//! agent display settings.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Not;
#[cfg(feature = "local_fs")]
use std::path::PathBuf;
use std::sync::LazyLock;

use ::ai::api_keys::{ApiKeyManager, ApiKeyManagerEvent, ApiKeys, CustomEndpointParams};
#[cfg(not(target_family = "wasm"))]
use ::ai::grok_subscription::oauth::{self, ManualCodeExchange};
use chrono::{DateTime, Local};
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use pathfinder_geometry::vector::vec2f;
use settings::{Setting, ToggleableSetting};
use strum::IntoEnumIterator;
use warp_core::channel::ChannelState;
use warp_core::context_flag::ContextFlag;
use warp_core::features::FeatureFlag;
use warp_core::ui::color::ContrastingColor;
use warp_core::ui::color::contrast::MinimumAllowedContrast;
use warp_core::ui::theme::Fill as ThemeFill;
use warp_core::ui::theme::color::internal_colors;
use warp_editor::editor::NavigationKey;
use warp_errors::report_if_error;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Empty, Expanded, Flex, FormattedTextElement, HighlightedHyperlink, Hoverable, HyperlinkLens,
    HyperlinkUrl, MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning,
    ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Shrinkable, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::{ContextPredicate, Keystroke};
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::switch::{SwitchStateHandle, TooltipConfig};
use warpui::{
    Action, AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle, WeakViewHandle, id,
};

use super::ai_shared::{
    render_ai_feature_switch, render_ai_setting_description, render_ai_setting_label,
    render_ai_setting_toggle, render_toolbar_layout_editor, styles,
    update_editor_interaction_state,
};
use super::custom_inference_modal::{
    CustomEndpointModal, CustomEndpointModalEvent, CustomEndpointModalViewState,
};
use super::remove_custom_endpoint_confirmation_dialog::{
    RemoveCustomEndpointConfirmationDialog, RemoveCustomEndpointConfirmationDialogEvent,
};
use super::set_default_model_modal::{SetDefaultModelModalBody, SetDefaultModelModalBodyEvent};
use super::settings_page::{
    CONTENT_FONT_SIZE, HEADER_PADDING, LocalOnlyIconState, MatchData, PageType, SettingsPageMeta,
    SettingsPageViewHandle, SettingsWidget, TOGGLE_BUTTON_RIGHT_PADDING, ToggleState,
    build_sub_header, build_toggle_element, render_body_item_label, render_dropdown_item,
    render_filterable_dropdown_item, render_separator,
};
use super::{
    SettingActionPairContexts, SettingActionPairDescriptions, SettingsAction, SettingsSection,
    ToggleSettingActionPair, editor_text_colors, flags,
};
use crate::ai::AIRequestUsageModel;
#[cfg(not(target_family = "wasm"))]
use crate::ai::aws_credentials::refresh_aws_credentials;
use crate::ai::blocklist::agent_view::agent_input_footer::editor::{
    AgentToolbarEditorMode, AgentToolbarInlineEditor,
};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
#[cfg(not(target_family = "wasm"))]
use crate::ai::geap_credentials::force_refresh_geap_credentials;
use crate::ai::llms::{LLMId, LLMPreferences, LLMProvider, is_using_api_key_for_provider};
use crate::appearance::{Appearance, AppearanceEvent};
use crate::auth::AuthStateProvider;
use crate::editor::{
    EditorOptions, EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys,
    SingleLineEditorOptions, TextColors, TextOptions,
};
use crate::modal::{Modal, ModalEvent, ModalViewState};
use crate::server::telemetry::{
    AgentModeAutoDetectionSettingOrigin, ToggleCodeSuggestionsSettingSource,
};
use crate::settings::{
    AIAutoDetectionEnabled, AICommandDenylist, AISettings, AISettingsChangedEvent,
    AgentModeQuerySuggestionsEnabled, AutoApproveBypassesCommandDenylist, AwsBedrockAutoLogin,
    AwsBedrockCredentialsEnabled, CanUseWarpCreditsForFallback, GeminiEnterpriseCredentialsEnabled,
    GitOperationsAutogenEnabled, IncludeAgentCommandsInHistory, InputSettings,
    IntelligentAutosuggestionsEnabled, LongRunningCommandSubmissionMode, NLDInTerminalEnabled,
    NaturalLanguageAutosuggestionsEnabled, OrchestrationMessageDisplayMode, PromptSubmissionMode,
    SharedBlockTitleGenerationEnabled, ShouldRenderUseAgentToolbarForUserCommands,
    ShouldShowOzUpdatesInZeroState, ShowAgentTips, ShowConversationHistory, ShowHintText,
    ThinkingDisplayMode, VOICE_INPUT_LANGUAGES, VoiceInputEnabled, VoiceInputLanguage,
    VoiceInputToggleKey,
};
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::util::bindings;
use crate::view_components::action_button::{
    ActionButton, ButtonSize, DangerSecondaryTheme, SecondaryTheme,
};
use crate::view_components::{Dropdown, DropdownItem, FilterableDropdown};
use crate::workspaces::user_workspaces::UserWorkspacesEvent;
use crate::workspaces::workspace::{AdminEnablementSetting, CustomerType};
use crate::{TelemetryEvent, UserWorkspaces, send_telemetry_from_ctx};

const PRIMARY_HEADER_FONT_SIZE: f32 = 24.;

const AI_SETTINGS_DROPDOWN_WIDTH: f32 = 250.;
const AI_SETTINGS_DROPDOWN_MAX_HEIGHT: f32 = 250.;

const NEXT_COMMAND_DESCRIPTION: &str = "Let AI suggest the next command to run based on your command history, outputs, and common workflows.";
const PROMPT_SUGGESTIONS_DESCRIPTION: &str = "Let AI suggest natural language prompts, as inline banners in the input, based on recent commands and their outputs.";
const SUGGESTED_CODE_BANNERS_DESCRIPTION: &str = "Let AI suggest code diffs and queries as inline banners in the blocklist, based on recent commands and their outputs.";
const NATURAL_LANGUAGE_AUTOSUGGESTIONS: &str =
    "Let AI suggest natural language autosuggestions, based on recent commands and their outputs.";
const SHARED_BLOCK_TITLE_GENERATION_DESCRIPTION: &str =
    "Let AI generate a title for your shared block based on the command and output.";
const GIT_OPERATIONS_AUTOGEN_DESCRIPTION: &str =
    "Let AI generate commit messages and pull request titles and descriptions.";
const WISPR_FLOW_URL: &str = "https://wisprflow.ai/";
const CUSTOM_INFERENCE_LEARN_MORE_URL: &str =
    "https://docs.warp.dev/agents/inference/custom-inference-endpoint/";
const CUSTOM_INFERENCE_TERMS_URL: &str = "https://www.warp.dev/legal/terms-of-service";
const CUSTOM_INFERENCE_INFO_TOOLTIP_MAX_WIDTH: f32 = 320.;
const CUSTOM_ENDPOINT_MODAL_MAX_HEIGHT_PERCENTAGE: f32 = 0.8;

pub fn init_actions_from_parent_view<T: Action + Clone>(
    app: &mut AppContext,
    context: &ContextPredicate,
    builder: fn(SettingsAction) -> T,
) {
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::new(
                "AI",
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleGlobalAI,
                )),
                context,
                flags::IS_ANY_AI_ENABLED,
            )
            .with_group(bindings::BindingGroup::WarpAi),
        ],
        app,
    );

    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::new(
                "Active AI",
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleActiveAI,
                )),
                &(context.clone() & id!(flags::IS_ANY_AI_ENABLED)),
                flags::IS_ACTIVE_AI_ENABLED,
            )
            .with_group(bindings::BindingGroup::WarpAi),
        ],
        app,
    );

    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::new(
                if FeatureFlag::AgentView.is_enabled() {
                    "terminal command autodetection in agent input"
                } else {
                    "natural language detection"
                },
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleAIInputAutoDetection,
                )),
                &(context.clone() & id!(flags::IS_ANY_AI_ENABLED)),
                flags::AI_INPUT_AUTODETECTION_FLAG,
            )
            .with_group(bindings::BindingGroup::WarpAi)
            .with_enabled(|| FeatureFlag::AgentMode.is_enabled()),
        ],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::new(
                "agent prompt autodetection in terminal input",
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleNLDInTerminal,
                )),
                &(context.clone() & id!(flags::IS_ANY_AI_ENABLED)),
                flags::NLD_IN_TERMINAL_FLAG,
            )
            .with_group(bindings::BindingGroup::WarpAi)
            .with_enabled(|| FeatureFlag::AgentView.is_enabled()),
        ],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::new(
                "Next Command",
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleIntelligentAutosuggestions,
                )),
                &(context.clone() & id!(flags::IS_ACTIVE_AI_ENABLED)),
                flags::INTELLIGENT_AUTOSUGGESTIONS_FLAG,
            )
            .with_group(bindings::BindingGroup::WarpAi),
        ],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::new(
                "prompt suggestions",
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::TogglePromptSuggestions,
                )),
                &(context.clone() & id!(flags::IS_ACTIVE_AI_ENABLED)),
                flags::PROMPT_SUGGESTIONS_FLAG,
            )
            .with_group(bindings::BindingGroup::WarpAi),
        ],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::new(
                "code suggestions",
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleCodeSuggestions,
                )),
                &(context.clone()
                    & id!(flags::IS_ACTIVE_AI_ENABLED)
                    & id!(flags::PROMPT_SUGGESTIONS_FLAG)),
                flags::CODE_SUGGESTIONS_FLAG,
            )
            .with_group(bindings::BindingGroup::WarpAi),
        ],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::custom(
                SettingActionPairDescriptions::new("Show agent tips", "Hide agent tips"),
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleShowAgentTips,
                )),
                SettingActionPairContexts::new(
                    context.clone()
                        & id!(flags::IS_ANY_AI_ENABLED)
                        & !id!(flags::SHOW_AGENT_TIPS_FLAG),
                    context.clone()
                        & id!(flags::IS_ANY_AI_ENABLED)
                        & id!(flags::SHOW_AGENT_TIPS_FLAG),
                ),
                None,
            )
            .with_group(bindings::BindingGroup::WarpAi)
            .with_enabled(|| FeatureFlag::AgentTips.is_enabled()),
        ],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::custom(
                SettingActionPairDescriptions::new(
                    "Show Warp Agent changelog in new agent conversation view",
                    "Hide Warp Agent changelog in new agent conversation view",
                ),
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleShowOzUpdatesInZeroState,
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
            .with_enabled(|| FeatureFlag::AgentView.is_enabled()),
        ],
        app,
    );
    {
        use warpui::keymap::FixedBinding;

        use crate::settings::ThinkingDisplayMode;

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
                    builder(SettingsAction::WarpAgent(
                        WarpAgentPageAction::SetThinkingDisplayMode(mode),
                    )),
                    ai_context.clone() & !id!(context_flag),
                )
                .with_group(bindings::BindingGroup::WarpAi.as_str())
            })
            .collect();
        app.register_fixed_bindings(mode_bindings);
    }
    {
        use warpui::keymap::FixedBinding;

        let ai_context = context.clone() & id!(flags::IS_ANY_AI_ENABLED);
        let mode_bindings: Vec<FixedBinding> = OrchestrationMessageDisplayMode::iter()
            .map(|mode| {
                let context_flag = match mode {
                    OrchestrationMessageDisplayMode::ShowAndCollapse => {
                        flags::ORCHESTRATION_MESSAGE_DISPLAY_SHOW_AND_COLLAPSE
                    }
                    OrchestrationMessageDisplayMode::AlwaysShow => {
                        flags::ORCHESTRATION_MESSAGE_DISPLAY_ALWAYS_SHOW
                    }
                    OrchestrationMessageDisplayMode::AlwaysCollapse => {
                        flags::ORCHESTRATION_MESSAGE_DISPLAY_ALWAYS_COLLAPSE
                    }
                };
                FixedBinding::empty(
                    mode.command_palette_description(),
                    builder(SettingsAction::WarpAgent(
                        WarpAgentPageAction::SetOrchestrationMessageDisplayMode(mode),
                    )),
                    ai_context.clone() & !id!(context_flag),
                )
                .with_group(bindings::BindingGroup::WarpAi.as_str())
            })
            .collect();
        app.register_fixed_bindings(mode_bindings);
    }
    if FeatureFlag::QueueSlashCommand.is_enabled() {
        use warpui::keymap::FixedBinding;

        let ai_context = context.clone() & id!(flags::IS_ANY_AI_ENABLED);
        let mode_bindings: Vec<FixedBinding> = PromptSubmissionMode::iter()
            .map(|mode| {
                let context_flag = match mode {
                    PromptSubmissionMode::Interrupt => flags::PROMPT_SUBMISSION_INTERRUPT,
                    PromptSubmissionMode::Queue => flags::PROMPT_SUBMISSION_QUEUE,
                };
                FixedBinding::empty(
                    mode.command_palette_description(),
                    builder(SettingsAction::WarpAgent(
                        WarpAgentPageAction::SetPromptSubmissionMode(mode),
                    )),
                    ai_context.clone() & !id!(context_flag),
                )
                .with_group(bindings::BindingGroup::WarpAi.as_str())
            })
            .collect();
        app.register_fixed_bindings(mode_bindings);

        // The LRC submission mode only applies (and is only shown) when the default
        // prompt submission mode is Interrupt, so its palette entries are gated on it.
        let lrc_mode_bindings: Vec<FixedBinding> = LongRunningCommandSubmissionMode::iter()
            .map(|mode| {
                let context_flag = match mode {
                    LongRunningCommandSubmissionMode::SendImmediately => {
                        flags::LRC_SUBMISSION_SEND_IMMEDIATELY
                    }
                    LongRunningCommandSubmissionMode::QueueUntilCommandCompletes => {
                        flags::LRC_SUBMISSION_QUEUE_UNTIL_COMMAND_COMPLETES
                    }
                };
                FixedBinding::empty(
                    mode.command_palette_description(),
                    builder(SettingsAction::WarpAgent(
                        WarpAgentPageAction::SetLongRunningCommandSubmissionMode(mode),
                    )),
                    ai_context.clone()
                        & id!(flags::PROMPT_SUBMISSION_INTERRUPT)
                        & !id!(context_flag),
                )
                .with_group(bindings::BindingGroup::WarpAi.as_str())
            })
            .collect();
        app.register_fixed_bindings(lrc_mode_bindings);
    }
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::new(
                "natural language autosuggestions",
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleNaturalLanguageAutosuggestions,
                )),
                &(context.clone() & id!(flags::IS_ACTIVE_AI_ENABLED)),
                flags::NATURAL_LANGUAGE_AUTOSUGGESTIONS_FLAG,
            )
            .with_group(bindings::BindingGroup::WarpAi)
            .with_enabled(|| FeatureFlag::PredictAMQueries.is_enabled()),
        ],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::new(
                "shared block title generation",
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleSharedTitleGeneration,
                )),
                &(context.clone() & id!(flags::IS_ACTIVE_AI_ENABLED)),
                flags::SHARED_BLOCK_TITLE_GENERATION_FLAG,
            )
            .with_group(bindings::BindingGroup::WarpAi)
            .with_enabled(|| FeatureFlag::SharedBlockTitleGeneration.is_enabled()),
        ],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::new(
                "commit and pull request generation",
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleGitOperationsAutogen,
                )),
                &(context.clone() & id!(flags::IS_ACTIVE_AI_ENABLED)),
                flags::GIT_OPERATIONS_AUTOGEN_FLAG,
            )
            .with_enabled(|| FeatureFlag::GitOperationsInCodeReview.is_enabled())
            .is_supported_on_current_platform(
                AISettings::as_ref(app)
                    .git_operations_autogen_enabled_internal
                    .is_supported_on_current_platform()
                    && UserWorkspaces::as_ref(app).is_git_operations_ai_enabled(),
            ),
        ],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::new(
                "voice input",
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleVoiceInput,
                )),
                &(context.clone() & id!(flags::IS_ANY_AI_ENABLED)),
                flags::IS_VOICE_INPUT_ENABLED,
            )
            .with_group(bindings::BindingGroup::WarpAi)
            .with_enabled(|| cfg!(feature = "voice_input")),
        ],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::custom(
                SettingActionPairDescriptions::new(
                    "Show \"Use Agent\" footer",
                    "Hide \"Use Agent\" footer",
                ),
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleUseAgentToolbar,
                )),
                SettingActionPairContexts::new(
                    context.clone()
                        & id!(flags::IS_ANY_AI_ENABLED)
                        & !id!(flags::USE_AGENT_FOOTER_FLAG),
                    context.clone()
                        & id!(flags::IS_ANY_AI_ENABLED)
                        & id!(flags::USE_AGENT_FOOTER_FLAG),
                ),
                None,
            )
            .with_group(bindings::BindingGroup::WarpAi),
        ],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::new(
                "include agent-executed commands in history",
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleIncludeAgentCommandsInHistory,
                )),
                &(context.clone() & id!(flags::IS_ANY_AI_ENABLED)),
                flags::INCLUDE_AGENT_COMMANDS_IN_HISTORY_FLAG,
            )
            .with_group(bindings::BindingGroup::WarpAi),
            ToggleSettingActionPair::custom(
                SettingActionPairDescriptions::new(
                    "Allow auto-approve to bypass command denylist",
                    "Require approval for denylisted commands in auto-approve",
                ),
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleAutoApproveBypassesCommandDenylist,
                )),
                SettingActionPairContexts::new(
                    context.clone()
                        & id!(flags::IS_ANY_AI_ENABLED)
                        & !id!(flags::AUTO_APPROVE_BYPASSES_COMMAND_DENYLIST_FLAG),
                    context.clone()
                        & id!(flags::IS_ANY_AI_ENABLED)
                        & id!(flags::AUTO_APPROVE_BYPASSES_COMMAND_DENYLIST_FLAG),
                ),
                None,
            )
            .with_group(bindings::BindingGroup::WarpAi),
            ToggleSettingActionPair::new(
                "conversation history in tools panel",
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleShowConversationHistory,
                )),
                &(context.clone() & id!(flags::IS_ANY_AI_ENABLED)),
                flags::SHOW_CONVERSATION_HISTORY,
            )
            .with_group(bindings::BindingGroup::WarpAi),
        ],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::new(
                "Auto-spawn servers from third-party agents",
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleFileBasedMcp,
                )),
                &(context.clone() & id!(flags::IS_ANY_AI_ENABLED)),
                flags::FILE_BASED_MCP_FLAG,
            )
            .with_group(bindings::BindingGroup::WarpAi)
            .with_enabled(|| {
                FeatureFlag::McpServer.is_enabled()
                    && FeatureFlag::FileBasedMcp.is_enabled()
                    && ContextFlag::ShowMCPServers.is_enabled()
            }),
        ],
        app,
    );
    ToggleSettingActionPair::add_toggle_setting_action_pairs_as_bindings(
        vec![
            ToggleSettingActionPair::new(
                "Warp credit fallback",
                builder(SettingsAction::WarpAgent(
                    WarpAgentPageAction::ToggleCanUseWarpCreditsForFallback,
                )),
                &(context.clone() & id!(flags::IS_ANY_AI_ENABLED)),
                flags::WARP_CREDIT_FALLBACK_FLAG,
            )
            .with_group(bindings::BindingGroup::WarpAi)
            .is_supported_on_current_platform(
                UserWorkspaces::as_ref(app).is_byo_api_key_enabled(app)
                    || UserWorkspaces::as_ref(app).is_custom_inference_enabled(app),
            ),
        ],
        app,
    );
}

pub struct WarpAgentPageView {
    page: PageType<Self>,
    voice_input_toggle_key_dropdown: ViewHandle<Dropdown<WarpAgentPageAction>>,
    voice_input_language_dropdown: ViewHandle<FilterableDropdown<WarpAgentPageAction>>,
    local_only_icon_tooltip_states: RefCell<HashMap<String, MouseStateHandle>>,
    autodetection_denylist_editor: ViewHandle<EditorView>,
    agent_toolbar_inline_editor: ViewHandle<AgentToolbarInlineEditor>,

    thinking_display_mode_dropdown: ViewHandle<Dropdown<WarpAgentPageAction>>,
    orchestration_message_display_mode_dropdown: ViewHandle<Dropdown<WarpAgentPageAction>>,
    default_prompt_submission_mode_dropdown: ViewHandle<Dropdown<WarpAgentPageAction>>,
    lrc_submission_mode_dropdown: ViewHandle<Dropdown<WarpAgentPageAction>>,
    #[cfg(feature = "local_fs")]
    conversation_layout_dropdown: ViewHandle<Dropdown<WarpAgentPageAction>>,

    // Custom model router views (gated on FeatureFlag::CustomModelRouters)
    #[cfg(feature = "local_fs")]
    router_views: Vec<ViewHandle<super::custom_router_view::CustomRouterView>>,
    #[cfg(feature = "local_fs")]
    add_router_button: ViewHandle<ActionButton>,

    custom_endpoint_modal_state: CustomEndpointModalViewState,
    remove_custom_endpoint_confirmation_dialog: ViewHandle<RemoveCustomEndpointConfirmationDialog>,
    pending_remove_custom_endpoint_index: Option<usize>,
    custom_inference_add_button: ViewHandle<ActionButton>,
    custom_endpoint_edit_buttons: Vec<ViewHandle<ActionButton>>,

    // Prompt offering to switch the default Agent Mode model after a BYO key or
    // custom endpoint is saved while the default isn't backed by a credential.
    set_default_model_modal: ModalViewState<Modal<SetDefaultModelModalBody>>,
    // Snapshot of the provider keys from the last `KeysUpdated`, used to detect a
    // newly added key and prompt the user to switch their default model.
    last_seen_provider_keys: ApiKeys,

    // In-flight fallback exchange for a pasted SuperGrok authorization code.
    // This stores only the PKCE verifier clone needed by the manual path while
    // `OauthAttempt::finish` owns the full loopback attempt.
    #[cfg(not(target_family = "wasm"))]
    grok_oauth_attempt: Option<ManualCodeExchange>,
    #[cfg(not(target_family = "wasm"))]
    grok_code_editor: ViewHandle<EditorView>,
}

impl WarpAgentPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let is_any_ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);

        let workspace = UserWorkspaces::handle(ctx);
        ctx.subscribe_to_model(&workspace, |me, _workspace, event, ctx| {
            if let UserWorkspacesEvent::TeamsChanged = event {
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

            dropdown.add_items(
                values
                    .into_iter()
                    .map(|val| {
                        DropdownItem::new(
                            val.display_name(),
                            WarpAgentPageAction::SetVoiceInputToggleKey(val),
                        )
                    })
                    .collect(),
                ctx,
            );
            dropdown.set_selected_by_index(selected_index, ctx);

            dropdown
        });

        let voice_input_language_dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = FilterableDropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);
            if !AISettings::as_ref(ctx).is_voice_input_enabled(ctx) {
                dropdown.set_disabled(ctx);
            }

            dropdown.add_items(
                VOICE_INPUT_LANGUAGES
                    .iter()
                    .map(|&(code, name)| {
                        DropdownItem::new(
                            name,
                            WarpAgentPageAction::SetVoiceInputLanguage(code.to_string()),
                        )
                    })
                    .collect(),
                ctx,
            );
            let current_code = AISettings::as_ref(ctx)
                .voice_input_language_code()
                .unwrap_or("")
                .to_string();
            dropdown.set_selected_by_action(
                WarpAgentPageAction::SetVoiceInputLanguage(current_code),
                ctx,
            );

            dropdown
        });

        let thinking_display_mode_dropdown =
            OtherAIWidget::create_thinking_display_mode_dropdown(ctx);
        // Set initial selection based on current setting value.
        {
            let current_mode = AISettings::as_ref(ctx).thinking_display_mode;
            thinking_display_mode_dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_selected_by_action(
                    WarpAgentPageAction::SetThinkingDisplayMode(current_mode),
                    ctx,
                );
            });
        }
        let orchestration_message_display_mode_dropdown =
            OtherAIWidget::create_orchestration_message_display_mode_dropdown(ctx);
        {
            let current_mode = AISettings::as_ref(ctx).orchestration_message_display_mode;
            orchestration_message_display_mode_dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_selected_by_action(
                    WarpAgentPageAction::SetOrchestrationMessageDisplayMode(current_mode),
                    ctx,
                );
            });
        }

        let default_prompt_submission_mode_dropdown =
            OtherAIWidget::create_default_prompt_submission_mode_dropdown(ctx);
        {
            let current_mode = AISettings::as_ref(ctx).default_prompt_submission_mode;
            default_prompt_submission_mode_dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_selected_by_action(
                    WarpAgentPageAction::SetPromptSubmissionMode(current_mode),
                    ctx,
                );
            });
        }

        let lrc_submission_mode_dropdown = OtherAIWidget::create_lrc_submission_mode_dropdown(ctx);
        {
            let current_mode = AISettings::as_ref(ctx).long_running_command_submission_mode;
            lrc_submission_mode_dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_selected_by_action(
                    WarpAgentPageAction::SetLongRunningCommandSubmissionMode(current_mode),
                    ctx,
                );
            });
        }

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

            editor.set_placeholder_text("Commands, comma separated", ctx);

            let current_value = AISettings::as_ref(ctx)
                .autodetection_command_denylist
                .value()
                .clone();
            editor.set_buffer_text(current_value.as_str(), ctx);
            editor
        });
        update_editor_interaction_state(
            autodetection_denylist_editor.clone(),
            is_any_ai_enabled,
            ctx,
        );

        ctx.subscribe_to_view(&autodetection_denylist_editor, move |me, _, event, ctx| {
            me.handle_detection_denylist_editor_event(event, ctx);
        });

        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _handle, _event, ctx| {
            // Re-render if teams-related data changed that may affect whether features such as voice input are enabled.
            me.sync_custom_endpoint_buttons(ctx);
            ctx.notify();
        });

        // Refresh model dropdowns when BYO API keys update so key icons reflect latest state.
        ctx.subscribe_to_model(&ApiKeyManager::handle(ctx), |me, _model, _event, ctx| {
            me.sync_custom_endpoint_buttons(ctx);
            // Driving the prompt off the key-store update (rather than the editor's
            // blur/Enter) means it fires reliably however the key was committed —
            // clicking outside the field, pressing Enter, or tabbing away.
            me.maybe_prompt_for_newly_added_provider_key(ctx);
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

                    update_editor_interaction_state(
                        me.autodetection_denylist_editor.clone(),
                        is_enabled,
                        ctx,
                    );

                    me.update_voice_input_dropdown_enablement(ctx);
                    me.sync_custom_endpoint_buttons(ctx);
                }
                AISettingsChangedEvent::VoiceInputEnabled { .. } => {
                    me.update_voice_input_dropdown_enablement(ctx);
                }
                AISettingsChangedEvent::VoiceInputToggleKey { .. } => {
                    let current_value = AISettings::as_ref(ctx)
                        .voice_input_toggle_key
                        .value()
                        .display_name();
                    me.voice_input_toggle_key_dropdown
                        .update(ctx, |dropdown, ctx| {
                            dropdown.set_selected_by_name(current_value, ctx)
                        });
                }
                AISettingsChangedEvent::VoiceInputLanguage { .. } => {
                    let current_code = AISettings::as_ref(ctx)
                        .voice_input_language_code()
                        .unwrap_or("")
                        .to_string();
                    me.voice_input_language_dropdown
                        .update(ctx, |dropdown, ctx| {
                            dropdown.set_selected_by_action(
                                WarpAgentPageAction::SetVoiceInputLanguage(current_code),
                                ctx,
                            )
                        });
                }
                AISettingsChangedEvent::ThinkingDisplayMode { .. } => {
                    let current_mode = *AISettings::as_ref(ctx).thinking_display_mode.value();
                    me.thinking_display_mode_dropdown
                        .update(ctx, |dropdown, ctx| {
                            dropdown.set_selected_by_action(
                                WarpAgentPageAction::SetThinkingDisplayMode(current_mode),
                                ctx,
                            );
                        });
                }
                AISettingsChangedEvent::OrchestrationMessageDisplayMode { .. } => {
                    let current_mode = AISettings::as_ref(ctx).orchestration_message_display_mode;
                    me.orchestration_message_display_mode_dropdown
                        .update(ctx, |dropdown, ctx| {
                            dropdown.set_selected_by_action(
                                WarpAgentPageAction::SetOrchestrationMessageDisplayMode(
                                    current_mode,
                                ),
                                ctx,
                            );
                        });
                }
                AISettingsChangedEvent::PromptSubmissionMode { .. } => {
                    let current_mode = AISettings::as_ref(ctx).default_prompt_submission_mode;
                    me.default_prompt_submission_mode_dropdown
                        .update(ctx, |dropdown, ctx| {
                            dropdown.set_selected_by_action(
                                WarpAgentPageAction::SetPromptSubmissionMode(current_mode),
                                ctx,
                            );
                        });
                }
                AISettingsChangedEvent::LongRunningCommandSubmissionMode { .. } => {
                    let current_mode = AISettings::as_ref(ctx).long_running_command_submission_mode;
                    me.lrc_submission_mode_dropdown
                        .update(ctx, |dropdown, ctx| {
                            dropdown.set_selected_by_action(
                                WarpAgentPageAction::SetLongRunningCommandSubmissionMode(
                                    current_mode,
                                ),
                                ctx,
                            );
                        });
                }
                _ => (),
            }
            ctx.notify();
        });

        ctx.subscribe_to_model(&InputSettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });

        #[cfg(feature = "local_fs")]
        let router_views = Self::create_router_views(ctx);
        #[cfg(feature = "local_fs")]
        let add_router_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("+ Add router", SecondaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(WarpAgentPageAction::OpenAddCustomRouter);
                })
        });
        #[cfg(feature = "local_fs")]
        {
            let is_enabled = warp_core::features::FeatureFlag::CustomModelRouters.is_enabled()
                && is_any_ai_enabled;
            add_router_button.update(ctx, |button, ctx| {
                button.set_disabled(!is_enabled, ctx);
            });
        }

        let custom_inference_controls_enabled = is_any_ai_enabled
            && UserWorkspaces::as_ref(ctx).is_custom_inference_enabled(ctx)
            && UserWorkspaces::as_ref(ctx).are_member_byo_endpoints_allowed();
        let custom_inference_add_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("+ Add custom model", SecondaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(WarpAgentPageAction::OpenAddCustomEndpointModal);
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
                    right: 0.,
                }),
                ..Default::default()
            })
            .with_background_opacity(100)
            .with_max_height_percentage(CUSTOM_ENDPOINT_MODAL_MAX_HEIGHT_PERCENTAGE)
            .with_dismiss_on_click()
            .with_dismiss_keystroke(Keystroke::parse("escape").unwrap())
        });
        ctx.subscribe_to_view(&custom_endpoint_modal_view, |me, _, event, ctx| {
            me.handle_custom_endpoint_modal_close_event(event, ctx);
        });

        let custom_endpoint_modal_state =
            CustomEndpointModalViewState::new(ModalViewState::new(custom_endpoint_modal_view));

        let set_default_model_modal_body = ctx.add_typed_action_view(SetDefaultModelModalBody::new);
        ctx.subscribe_to_view(&set_default_model_modal_body, |me, _, event, ctx| {
            me.handle_set_default_model_modal_event(event, ctx);
        });
        let set_default_model_modal_view = ctx.add_typed_action_view(|ctx| {
            Modal::new(
                Some("Change your default model?".to_string()),
                set_default_model_modal_body.clone(),
                ctx,
            )
            .with_modal_style(UiComponentStyles {
                width: Some(480.),
                height: Some(380.),
                ..Default::default()
            })
            .with_body_style(UiComponentStyles {
                height: Some(300.),
                ..Default::default()
            })
            .with_background_opacity(100)
            .with_dismiss_on_click()
            .with_dismiss_keystroke(Keystroke::parse("escape").unwrap())
        });
        ctx.subscribe_to_view(
            &set_default_model_modal_view,
            |me, _, event, ctx| match event {
                ModalEvent::Close => me.hide_set_default_model_modal(ctx),
            },
        );
        let set_default_model_modal = ModalViewState::new(set_default_model_modal_view);
        let last_seen_provider_keys = ApiKeyManager::as_ref(ctx).keys().clone();

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

        #[cfg(feature = "local_fs")]
        let conversation_layout_dropdown = ctx.add_typed_action_view(|ctx| {
            use crate::util::file::external_editor::settings::OpenConversationPreference;

            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);

            let items = vec![
                DropdownItem::new(
                    "New Tab",
                    WarpAgentPageAction::SetConversationLayout(OpenConversationPreference::NewTab),
                ),
                DropdownItem::new(
                    "Split Pane",
                    WarpAgentPageAction::SetConversationLayout(
                        OpenConversationPreference::SplitPane,
                    ),
                ),
            ];
            dropdown.set_items(items, ctx);

            let current = *crate::util::file::external_editor::EditorSettings::as_ref(ctx)
                .open_conversation_layout_preference;
            match current {
                OpenConversationPreference::NewTab => dropdown.set_selected_by_name("New Tab", ctx),
                OpenConversationPreference::SplitPane => {
                    dropdown.set_selected_by_name("Split Pane", ctx)
                }
            };
            dropdown
        });

        #[cfg(not(target_family = "wasm"))]
        let grok_code_editor = Self::create_grok_code_editor(ctx);
        #[cfg(not(target_family = "wasm"))]
        ctx.subscribe_to_view(&grok_code_editor, |me, _, event, ctx| {
            if matches!(event, EditorEvent::Enter | EditorEvent::Paste) {
                let code = me.grok_code_editor.as_ref(ctx).buffer_text(ctx);
                me.submit_grok_code(code, ctx);
            }
        });
        // Keep the snapshotted editor text colors in sync with theme changes,
        // like the API key editors above.
        #[cfg(not(target_family = "wasm"))]
        {
            let grok_code_editor = grok_code_editor.clone();
            ctx.subscribe_to_model(&Appearance::handle(ctx), move |_, _, event, ctx| {
                if let AppearanceEvent::ThemeChanged = event {
                    let colors = editor_text_colors(Appearance::as_ref(ctx));
                    grok_code_editor.update(ctx, move |editor, ctx| {
                        editor.set_text_colors(colors, ctx);
                    });
                }
            });
        }
        // Subscribe to WarpConfig to refresh router views when files change.
        #[cfg(feature = "local_fs")]
        ctx.subscribe_to_model(
            &crate::user_config::WarpConfig::handle(ctx),
            |me, _, event, ctx| {
                use crate::user_config::WarpConfigUpdateEvent;
                if matches!(event, WarpConfigUpdateEvent::ModelConfigs) {
                    me.router_views = Self::create_router_views(ctx);
                    ctx.notify();
                }
            },
        );

        Self {
            page: Self::build_page(ctx),
            voice_input_toggle_key_dropdown,
            voice_input_language_dropdown,
            autodetection_denylist_editor,
            local_only_icon_tooltip_states: Default::default(),
            agent_toolbar_inline_editor,
            thinking_display_mode_dropdown,
            orchestration_message_display_mode_dropdown,
            default_prompt_submission_mode_dropdown,
            lrc_submission_mode_dropdown,
            #[cfg(feature = "local_fs")]
            conversation_layout_dropdown,
            #[cfg(feature = "local_fs")]
            router_views,
            #[cfg(feature = "local_fs")]
            add_router_button,
            custom_endpoint_modal_state,
            remove_custom_endpoint_confirmation_dialog,
            pending_remove_custom_endpoint_index: None,
            custom_inference_add_button,
            custom_endpoint_edit_buttons,
            set_default_model_modal,
            last_seen_provider_keys,
            #[cfg(not(target_family = "wasm"))]
            grok_oauth_attempt: None,
            #[cfg(not(target_family = "wasm"))]
            grok_code_editor,
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
        self.voice_input_language_dropdown
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
        } else if self.set_default_model_modal.is_open() {
            Some(self.set_default_model_modal.render())
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

    fn handle_set_default_model_modal_event(
        &mut self,
        event: &SetDefaultModelModalBodyEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            SetDefaultModelModalBodyEvent::Close => self.hide_set_default_model_modal(ctx),
            SetDefaultModelModalBodyEvent::SetDefault(id) => {
                // Mirror `WarpAgentPageAction::SetBaseModel`: set the active
                // profile's base model and clear any stale context-window limit.
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    let profile_id = profiles_model.active_profile(None, ctx).id().clone();
                    profiles_model.set_base_model(&profile_id, Some(id.clone()), ctx);
                    profiles_model.set_context_window_limit(&profile_id, None, ctx);
                });
                // The Profiles page owns the context-window editor and resyncs
                // it from the resulting `ProfileUpdated` event.
                self.hide_set_default_model_modal(ctx);

                let window_id = ctx.window_id();
                crate::ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = crate::view_components::DismissibleToast::success(
                        "Default model updated".to_string(),
                    );
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                ctx.notify();
            }
        }
    }

    fn hide_set_default_model_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.set_default_model_modal.close();
        ctx.emit(WarpAgentPageEvent::HideModal);
        ctx.notify();
    }

    fn show_set_default_model_modal(
        &mut self,
        description: String,
        choices: Vec<(LLMId, String)>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.set_default_model_modal.view.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                body.set_choices(description, choices, ctx);
            });
        });
        self.set_default_model_modal.open();
        // Focus the modal so Escape closes it (the modal's escape binding only
        // fires while something inside the modal holds focus).
        ctx.focus(&self.set_default_model_modal.view);
        ctx.emit(WarpAgentPageEvent::ShowModal);
        ctx.notify();
    }

    /// Returns `true` when the active Agent Mode default model is already served
    /// by a credential the user has: a BYO key/subscription for its provider, or
    /// one of their custom-endpoint models. `auto` models report `false` since
    /// they always consume Warp credits.
    fn active_base_model_is_byo_covered(ctx: &AppContext) -> bool {
        let (active_id, active_provider) = {
            let prefs = LLMPreferences::as_ref(ctx);
            let active = prefs.get_active_base_model(ctx, None);
            (active.id.clone(), active.provider)
        };
        if LLMPreferences::as_ref(ctx)
            .custom_llm_info_for_id(&active_id)
            .is_some()
        {
            return true;
        }
        is_using_api_key_for_provider(&active_provider, ctx)
    }

    /// The display name of the user's current default Agent Mode model, used in
    /// the prompt copy (e.g. "auto (cost-efficient)").
    fn active_base_model_display_name(ctx: &AppContext) -> String {
        LLMPreferences::as_ref(ctx)
            .get_active_base_model(ctx, None)
            .display_name
            .clone()
    }

    /// Whether to offer switching the default model. Scoped to free-plan users
    /// who are out of monthly (base-plan) credits, since only they hit the
    /// "no credits" error with an `auto` model. Also skips when the current
    /// default is already served by a BYO credential.
    fn should_offer_default_model_switch(ctx: &AppContext) -> bool {
        // Exclude only confirmed paid plans. Solo/individual users have no
        // `current_workspace`, and billing may not have loaded yet (Unknown), so
        // treat both as eligible and rely on the out-of-credits check below to
        // filter anyone who can still run Warp-hosted models. (A strict
        // `is_free_plan()` check here meant solo free users — the common case —
        // never saw the prompt.)
        let on_paid_plan = UserWorkspaces::as_ref(ctx)
            .current_workspace()
            .is_some_and(|workspace| workspace.billing_metadata.is_user_on_paid_plan());
        let out_of_monthly_credits =
            !AIRequestUsageModel::as_ref(ctx).has_base_plan_requests_remaining();
        !on_paid_plan && out_of_monthly_credits && !Self::active_base_model_is_byo_covered(ctx)
    }

    /// Detects a provider key that was just added (absent -> present) by diffing
    /// against the last-seen keys, then offers to switch the default model. Run
    /// from `ApiKeyManagerEvent::KeysUpdated` so it fires regardless of how the
    /// key editor was committed.
    fn maybe_prompt_for_newly_added_provider_key(&mut self, ctx: &mut ViewContext<Self>) {
        let current = ApiKeyManager::as_ref(ctx).keys().clone();
        let newly_added = LLMProvider::API_KEY_PROVIDERS.into_iter().find(|provider| {
            let was_present = provider
                .api_key(&self.last_seen_provider_keys)
                .is_some_and(|key| !key.trim().is_empty());
            let now_present = provider
                .api_key(&current)
                .is_some_and(|key| !key.trim().is_empty());
            !was_present && now_present
        });
        self.last_seen_provider_keys = current;
        if let Some(provider) = newly_added {
            self.maybe_prompt_set_default_model_for_provider(provider, ctx);
        }
    }

    /// After a BYO provider key is added, offer to switch the default Agent Mode
    /// model to one from that provider.
    fn maybe_prompt_set_default_model_for_provider(
        &mut self,
        provider: LLMProvider,
        ctx: &mut ViewContext<Self>,
    ) {
        // Only prompt when the key is actually usable for requests (BYO enabled).
        if !is_using_api_key_for_provider(&provider, ctx) {
            return;
        }
        if !Self::should_offer_default_model_switch(ctx) {
            return;
        }
        let choices: Vec<(LLMId, String)> = LLMPreferences::as_ref(ctx)
            .get_base_llm_choices_for_agent_mode(ctx)
            .filter(|llm| llm.provider == provider)
            .map(|llm| (llm.id.clone(), llm.menu_display_name()))
            .collect();
        if choices.is_empty() {
            return;
        }
        let provider_name = provider.display_name();
        let current_default = Self::active_base_model_display_name(ctx);
        let description = format!(
            "You added your own {provider_name} API key, but your default model is currently set \
             to {current_default}, which won't work without Warp credits. Would you like to change \
             your default model?"
        );
        self.show_set_default_model_modal(description, choices, ctx);
    }

    /// After a custom endpoint is added or saved, offer to switch the default
    /// Agent Mode model to one of its models.
    fn maybe_prompt_set_default_model_for_custom_endpoint(
        &mut self,
        endpoint_index: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        if !Self::can_use_custom_inference_controls(ctx) {
            return;
        }
        if !Self::should_offer_default_model_switch(ctx) {
            return;
        }
        let Some(endpoint) = ApiKeyManager::as_ref(ctx)
            .keys()
            .custom_endpoints
            .get(endpoint_index)
            .cloned()
        else {
            return;
        };
        // Build directly from the endpoint's models rather than the synthetic
        // `custom_llms`, which are rebuilt asynchronously on `KeysUpdated`.
        let choices: Vec<(LLMId, String)> = endpoint
            .models
            .iter()
            .filter(|m| !m.name.trim().is_empty() && !m.config_key.is_empty())
            .map(|m| {
                (
                    LLMId::from(m.config_key.clone()),
                    m.display_label().to_string(),
                )
            })
            .collect();
        if choices.is_empty() {
            return;
        }
        let current_default = Self::active_base_model_display_name(ctx);
        let description = format!(
            "You added the \"{}\" custom endpoint, but your default model is currently set to \
             {current_default}, which won't work without Warp credits. Would you like to change \
             your default model?",
            endpoint.name
        );
        self.show_set_default_model_modal(description, choices, ctx);
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
                                WarpAgentPageAction::OpenEditCustomEndpointModal(index),
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
        AISettings::as_ref(app).is_any_ai_enabled(app)
            && UserWorkspaces::as_ref(app).is_custom_inference_enabled(app)
            && UserWorkspaces::as_ref(app).are_member_byo_endpoints_allowed()
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
        ctx.emit(WarpAgentPageEvent::ShowModal);
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
        ctx.emit(WarpAgentPageEvent::ShowModal);
        ctx.notify();
    }

    fn hide_custom_endpoint_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.custom_endpoint_modal_state.close(ctx);
        ctx.emit(WarpAgentPageEvent::HideModal);
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
                schema,
                models,
            } => {
                if !Self::can_use_custom_inference_controls(ctx) {
                    self.hide_custom_endpoint_modal(ctx);
                    return;
                }
                ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.add_custom_endpoint(
                        CustomEndpointParams {
                            name: name.clone(),
                            url: url.clone(),
                            api_key: api_key.clone(),
                            models: models.clone(),
                            schema: *schema,
                        },
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

                // The new endpoint is appended last.
                let new_index = ApiKeyManager::as_ref(ctx)
                    .keys()
                    .custom_endpoints
                    .len()
                    .saturating_sub(1);
                self.maybe_prompt_set_default_model_for_custom_endpoint(new_index, ctx);
                ctx.notify();
            }
            CustomEndpointModalEvent::SaveEndpoint {
                index,
                name,
                url,
                api_key,
                schema,
                models,
            } => {
                if !Self::can_use_custom_inference_controls(ctx) {
                    self.hide_custom_endpoint_modal(ctx);
                    return;
                }
                ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.save_custom_endpoint(
                        *index,
                        CustomEndpointParams {
                            name: name.clone(),
                            url: url.clone(),
                            api_key: api_key.clone(),
                            models: models.clone(),
                            schema: *schema,
                        },
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
                self.maybe_prompt_set_default_model_for_custom_endpoint(*index, ctx);
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

    #[cfg(not(target_family = "wasm"))]
    fn create_grok_code_editor(ctx: &mut ViewContext<Self>) -> ViewHandle<EditorView> {
        ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::handle(ctx).as_ref(ctx);
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_size_override: Some(appearance.ui_font_size()),
                    font_family_override: Some(appearance.monospace_font_family()),
                    text_colors_override: Some(editor_text_colors(appearance)),
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("Paste sign-in code", ctx);
            editor
        })
    }

    /// Kicks off the xAI (Grok) subscription OAuth flow: opens the consent
    /// screen in the browser, runs a loopback PKCE callback server, exchanges
    /// the resulting authorization code for OAuth tokens, and persists them via
    /// `ApiKeyManager` (which then proactively refreshes them before expiry).
    ///
    /// In parallel, this reveals the manual code-entry row so the user can
    /// paste the code xAI displays when the browser can't reach the loopback
    /// callback. Whichever path completes first connects the subscription; the
    /// other completion is ignored once the view-owned attempt state is cleared.
    #[cfg(not(target_family = "wasm"))]
    fn start_grok_oauth(&mut self, ctx: &mut ViewContext<Self>) {
        use warp_core::safe_error;

        use crate::ToastStack;
        use crate::view_components::{DismissibleToast, ToastLink};
        use crate::workspace::WorkspaceAction;

        /// Object id shared by the connect-flow toasts so the completion toast
        /// (success or error) automatically replaces the in-progress one.
        const CONNECT_TOAST_OBJECT_ID: &str = "grok_oauth_connect_toast";

        // Record attempt initiation on click (before we attempt to bind the
        // loopback server). This ensures every terminal SuperGrokSubscriptionConnectFinished
        // (including immediate bind failures) is paired with a preceding Initiated
        // for funnel/drop-off analysis.
        send_telemetry_from_ctx!(TelemetryEvent::SuperGrokSubscriptionConnectInitiated, ctx);

        // Starting the attempt binds the loopback callback server before the
        // browser opens, so a bind failure surfaces immediately, without a
        // dangling browser tab.
        let attempt = match oauth::OauthAttempt::start() {
            Ok(attempt) => attempt,
            Err(err) => {
                safe_error!(
                    safe: ("Failed to start Grok OAuth callback server"),
                    full: ("Failed to start Grok OAuth callback server: {err:#}")
                );
                send_telemetry_from_ctx!(
                    TelemetryEvent::SuperGrokSubscriptionConnectFinished {
                        error: Some("bind_failed".to_string()),
                    },
                    ctx
                );
                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast =
                        DismissibleToast::error(format!("Couldn't start Grok login: {err}"));
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                return;
            }
        };

        // Capture the PKCE verifier so the fallback is ready if xAI shows a
        // code instead of redirecting.
        self.grok_oauth_attempt = Some(attempt.manual_code_exchange());
        self.grok_code_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer(ctx);
        });
        ctx.notify();
        // Open xAI's consent screen in the user's default browser.
        let authorize_url = attempt.authorize_url();
        ctx.open_url(&authorize_url);

        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            // Persistent rather than ephemeral so the copy-URL fallback stays
            // available when the browser fails to open. It can't linger
            // forever: the completion toast below replaces it (shared object
            // id), and the OAuth attempt itself times out when the callback
            // never arrives.
            let toast = DismissibleToast::default(
                "Opening your browser to connect your SuperGrok subscription…".to_string(),
            )
            .with_object_id(CONNECT_TOAST_OBJECT_ID.to_string())
            .with_link(
                ToastLink::new("Copy URL".to_string())
                    .with_onclick_action(WorkspaceAction::CopyTextToClipboard(authorize_url)),
            );
            toast_stack.add_persistent_toast(toast, window_id, ctx);
        });

        ctx.spawn(async move { attempt.finish().await }, |me, result, ctx| {
            // Ignore loopback completion after a successful pasted-code path.
            if me.grok_oauth_attempt.is_none() {
                return;
            }
            let window_id = ctx.window_id();
            let toast = match result {
                Ok(tokens) => {
                    me.grok_oauth_attempt = None;
                    me.grok_code_editor.update(ctx, |editor, ctx| {
                        editor.clear_buffer(ctx);
                    });
                    send_telemetry_from_ctx!(
                        TelemetryEvent::SuperGrokSubscriptionConnectFinished { error: None },
                        ctx
                    );
                    // Persist the tokens to secure storage and kick off the
                    // proactive refresh loop so subsequent requests can
                    // authenticate with the connected subscription.
                    ApiKeyManager::handle(ctx).update(ctx, move |manager, ctx| {
                        manager.store_grok_tokens(tokens, ctx);
                    });
                    DismissibleToast::success("SuperGrok subscription connected".to_string())
                }
                Err(err) => {
                    me.grok_oauth_attempt = None;
                    me.grok_code_editor.update(ctx, |editor, ctx| {
                        editor.clear_buffer(ctx);
                    });
                    safe_error!(
                        safe: ("Grok OAuth loopback callback failed"),
                        full: ("Grok OAuth loopback callback failed: {err:#}")
                    );
                    send_telemetry_from_ctx!(
                        TelemetryEvent::SuperGrokSubscriptionConnectFinished {
                            error: Some("loopback_failed".to_string()),
                        },
                        ctx
                    );
                    DismissibleToast::error(format!("Couldn't connect SuperGrok: {err}"))
                }
            };
            ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                toast_stack.add_ephemeral_toast(
                    toast.with_object_id(CONNECT_TOAST_OBJECT_ID.to_string()),
                    window_id,
                    ctx,
                );
            });
            ctx.notify();
        });
    }

    /// Exchanges a pasted SuperGrok authorization code using the current
    /// attempt's PKCE verifier.
    #[cfg(not(target_family = "wasm"))]
    fn submit_grok_code(&mut self, code: String, ctx: &mut ViewContext<Self>) {
        use warp_core::safe_error;

        use crate::ToastStack;
        use crate::view_components::DismissibleToast;

        // Shared with the browser connect-flow toasts.
        const CONNECT_TOAST_OBJECT_ID: &str = "grok_oauth_connect_toast";
        let Some(exchange) = self.grok_oauth_attempt.clone() else {
            return;
        };
        if code.trim().is_empty() {
            return;
        }

        ctx.spawn(
            async move { exchange.exchange(&code).await },
            |me, result, ctx| {
                if me.grok_oauth_attempt.is_none() {
                    return;
                }
                let window_id = ctx.window_id();
                let toast = match result {
                    Ok(tokens) => {
                        me.grok_oauth_attempt = None;
                        me.grok_code_editor.update(ctx, |editor, ctx| {
                            editor.clear_buffer(ctx);
                        });
                        send_telemetry_from_ctx!(
                            TelemetryEvent::SuperGrokSubscriptionConnectFinished { error: None },
                            ctx
                        );
                        ApiKeyManager::handle(ctx).update(ctx, move |manager, ctx| {
                            manager.store_grok_tokens(tokens, ctx);
                        });
                        DismissibleToast::success("SuperGrok subscription connected".to_string())
                    }
                    Err(err) => {
                        // Keep the row open so the user can correct the code.
                        safe_error!(
                            safe: ("Grok manual code exchange failed"),
                            full: ("Grok manual code exchange failed: {err:#}")
                        );
                        send_telemetry_from_ctx!(
                            TelemetryEvent::SuperGrokSubscriptionConnectFinished {
                                error: Some("manual_code_failed".to_string()),
                            },
                            ctx
                        );
                        DismissibleToast::error(format!("Couldn't connect SuperGrok: {err}"))
                    }
                };
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(
                        toast.with_object_id(CONNECT_TOAST_OBJECT_ID.to_string()),
                        window_id,
                        ctx,
                    );
                });
                ctx.notify();
            },
        );
    }

    fn build_page(ctx: &mut ViewContext<Self>) -> PageType<Self> {
        let ai_settings = AISettings::as_ref(ctx);

        let mut widgets: Vec<Box<dyn SettingsWidget<View = WarpAgentPageView>>> = Vec::new();

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
            widgets.push(Box::new(ActiveAIWidget::new(ctx)));
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
        widgets.push(Box::new(GeminiEnterpriseWidget::new(ctx)));
        if FeatureFlag::CustomModelRouters.is_enabled() {
            widgets.push(Box::new(CustomModelRoutersWidget));
        }
        widgets.push(Box::new(AgentAttributionWidget::default()));
        widgets.push(Box::new(OtherAIWidget::default()));
        if FeatureFlag::AgentModeComputerUse.is_enabled() {
            widgets.push(Box::new(CloudAgentComputerUseWidget::default()));
        }

        // This page is multi-section: it renders its own subheader-sized
        // section titles inside each widget, so it gets no page-level title.
        PageType::new_uncategorized(widgets, None)
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
            EditorEvent::Escape => ctx.emit(WarpAgentPageEvent::FocusModal),
            _ => {}
        }
    }

    #[cfg(feature = "local_fs")]
    fn create_router_views(
        ctx: &mut ViewContext<Self>,
    ) -> Vec<ViewHandle<super::custom_router_view::CustomRouterView>> {
        use super::custom_router_view::{CustomRouterView, CustomRouterViewEvent};
        use crate::user_config::WarpConfig;
        if !warp_core::features::FeatureFlag::CustomModelRouters.is_enabled() {
            return Vec::new();
        }
        let routers: Vec<crate::ai::custom_model_routers::CustomModelRouter> =
            WarpConfig::as_ref(ctx).custom_model_routers().clone();
        routers
            .into_iter()
            .map(|router| {
                let router_clone = router.clone();
                let view = ctx.add_typed_action_view(|ctx| CustomRouterView::new(router, ctx));
                ctx.subscribe_to_view(&view, move |me, _, event, ctx| match event {
                    CustomRouterViewEvent::OpenFile(path) => {
                        ctx.emit(WarpAgentPageEvent::OpenCustomRouterFile(path.clone()));
                    }
                    CustomRouterViewEvent::Edit => {
                        let r = router_clone.clone();
                        ctx.emit(WarpAgentPageEvent::OpenCustomRouterEditor(Some(r)));
                    }
                    CustomRouterViewEvent::Delete => {
                        if let Some(path) = &router_clone.source_path {
                            #[cfg(feature = "local_fs")]
                            {
                                if let Err(e) =
                                    crate::user_config::WarpConfig::delete_custom_model_router(path)
                                {
                                    log::warn!("Failed to delete custom router: {e:?}");
                                }
                            }
                            me.router_views = Self::create_router_views(ctx);
                            ctx.notify();
                        }
                    }
                });
                view
            })
            .collect()
    }
}

impl View for WarpAgentPageView {
    fn ui_name() -> &'static str {
        "WarpAgentPage"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        self.page.render(self, app)
    }
}

#[allow(clippy::large_enum_variant)]
pub enum WarpAgentPageEvent {
    FocusModal,
    #[cfg(feature = "local_fs")]
    OpenCustomRouterEditor(Option<crate::ai::custom_model_routers::CustomModelRouter>),
    #[cfg(feature = "local_fs")]
    OpenCustomRouterFile(PathBuf),
    SignupAnonymousUser,
    ShowModal,
    HideModal,
}

impl Entity for WarpAgentPageView {
    type Event = WarpAgentPageEvent;
}

#[derive(Debug, Clone, PartialEq)]
pub enum WarpAgentPageAction {
    OpenUrl(String),
    SetVoiceInputToggleKey(VoiceInputToggleKey),
    SetVoiceInputLanguage(String),
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
    ToggleUseAgentToolbar,
    ToggleVoiceInput,
    ToggleCanUseWarpCreditsForFallback,
    HyperlinkClick(HyperlinkUrl),
    ToggleShowInputHintText,
    ToggleShowAgentTips,
    ToggleShowOzUpdatesInZeroState,
    SetThinkingDisplayMode(ThinkingDisplayMode),
    SetOrchestrationMessageDisplayMode(OrchestrationMessageDisplayMode),
    SetPromptSubmissionMode(PromptSubmissionMode),
    SetLongRunningCommandSubmissionMode(LongRunningCommandSubmissionMode),
    SignupAnonymousUser,
    ToggleAwsBedrockAutoLogin,
    ToggleAwsBedrockCredentialsEnabled,
    RefreshAwsBedrockCredentials,
    RefreshGeminiEnterpriseCredentials,
    ToggleGeminiEnterpriseCredentialsEnabled,
    ToggleCloudAgentComputerUse,
    ToggleFileBasedMcp,
    ToggleIncludeAgentCommandsInHistory,
    ToggleAutoApproveBypassesCommandDenylist,
    ToggleAgentAttribution,

    // Custom model routers
    #[cfg(feature = "local_fs")]
    OpenAddCustomRouter,

    // Custom inference
    OpenAddCustomEndpointModal,
    OpenEditCustomEndpointModal(usize),
    ConnectGrokSubscription,
    DisconnectGrokSubscription,

    #[cfg(feature = "local_fs")]
    SetConversationLayout(crate::util::file::external_editor::settings::OpenConversationPreference),
    ToggleCloudHandoff,
    ToggleAmpersandHandoff,
    ToggleAutoHandoffOnSleep,
    ToggleShowConversationHistory,
}

impl TypedActionView for WarpAgentPageView {
    type Action = WarpAgentPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            WarpAgentPageAction::OpenUrl(url) => {
                ctx.open_url(url.as_str());
            }
            WarpAgentPageAction::SetVoiceInputToggleKey(key) => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.voice_input_toggle_key.set_value(*key, ctx));
                    report_if_error!(
                        settings
                            .explicitly_interacted_with_voice
                            .set_value(true, ctx)
                    );
                });
                ctx.notify();
            }
            WarpAgentPageAction::SetVoiceInputLanguage(language) => {
                let language = language.clone();
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.voice_input_language.set_value(language, ctx));
                });
                ctx.notify();
            }
            WarpAgentPageAction::ToggleGlobalAI => {
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
            WarpAgentPageAction::ToggleActiveAI => {
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
            WarpAgentPageAction::ToggleIntelligentAutosuggestions => {
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
            WarpAgentPageAction::TogglePromptSuggestions => {
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
            WarpAgentPageAction::ToggleCodeSuggestions => {
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
            WarpAgentPageAction::ToggleNaturalLanguageAutosuggestions => {
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
            WarpAgentPageAction::ToggleSharedTitleGeneration => {
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
            WarpAgentPageAction::ToggleGitOperationsAutogen => {
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
            WarpAgentPageAction::ToggleAIInputAutoDetection => {
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
            WarpAgentPageAction::ToggleNLDInTerminal => {
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
            WarpAgentPageAction::ToggleUseAgentToolbar => {
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
            WarpAgentPageAction::ToggleVoiceInput => {
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
            WarpAgentPageAction::ToggleCanUseWarpCreditsForFallback => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .can_use_warp_credits_for_fallback
                            .toggle_and_save_value(ctx)
                    );
                });
                ctx.notify();
            }
            WarpAgentPageAction::HyperlinkClick(hyperlink) => {
                ctx.notify();
                ctx.open_url(&hyperlink.url);
            }
            WarpAgentPageAction::ToggleShowInputHintText => {
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
            WarpAgentPageAction::ToggleShowAgentTips => {
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
            WarpAgentPageAction::ToggleShowOzUpdatesInZeroState => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .should_show_oz_updates_in_zero_state
                            .toggle_and_save_value(ctx)
                    );
                });
                ctx.notify();
            }
            WarpAgentPageAction::SetThinkingDisplayMode(mode) => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.thinking_display_mode.set_value(*mode, ctx));
                });
                ctx.notify();
            }
            WarpAgentPageAction::SetOrchestrationMessageDisplayMode(mode) => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .orchestration_message_display_mode
                            .set_value(*mode, ctx)
                    );
                });
                ctx.notify();
            }
            WarpAgentPageAction::SetPromptSubmissionMode(mode) => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .default_prompt_submission_mode
                            .set_value(*mode, ctx)
                    );
                });
                ctx.notify();
            }
            WarpAgentPageAction::SetLongRunningCommandSubmissionMode(mode) => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .long_running_command_submission_mode
                            .set_value(*mode, ctx)
                    );
                });
                ctx.notify();
            }
            WarpAgentPageAction::SignupAnonymousUser => {
                ctx.emit(WarpAgentPageEvent::SignupAnonymousUser);
            }
            WarpAgentPageAction::ToggleAwsBedrockAutoLogin => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.aws_bedrock_auto_login.toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            WarpAgentPageAction::ToggleAwsBedrockCredentialsEnabled => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .aws_bedrock_credentials_enabled
                            .toggle_and_save_value(ctx)
                    );
                });
                ctx.notify();
            }
            WarpAgentPageAction::RefreshAwsBedrockCredentials => {
                #[cfg(not(target_family = "wasm"))]
                ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                    drop(refresh_aws_credentials(manager, ctx));
                });
                ctx.notify();
            }
            WarpAgentPageAction::RefreshGeminiEnterpriseCredentials => {
                #[cfg(not(target_family = "wasm"))]
                ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                    force_refresh_geap_credentials(manager, ctx);
                });
                ctx.notify();
            }
            WarpAgentPageAction::ToggleGeminiEnterpriseCredentialsEnabled => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .gemini_enterprise_credentials_enabled
                            .toggle_and_save_value(ctx)
                    );
                });
                ctx.notify();
            }
            WarpAgentPageAction::ToggleCloudAgentComputerUse => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .cloud_agent_computer_use_enabled
                            .toggle_and_save_value(ctx)
                    );
                });
                ctx.notify();
            }
            WarpAgentPageAction::ToggleFileBasedMcp => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.file_based_mcp_enabled.toggle_and_save_value(ctx));
                });
                ctx.notify();
            }
            WarpAgentPageAction::ToggleIncludeAgentCommandsInHistory => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .include_agent_commands_in_history
                            .toggle_and_save_value(ctx)
                    );
                });
                ctx.notify();
            }
            WarpAgentPageAction::ToggleAutoApproveBypassesCommandDenylist => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .auto_approve_bypasses_command_denylist
                            .toggle_and_save_value(ctx)
                    );
                });
                ctx.notify();
            }
            #[cfg(feature = "local_fs")]
            WarpAgentPageAction::SetConversationLayout(layout) => {
                crate::util::file::external_editor::EditorSettings::handle(ctx).update(
                    ctx,
                    |settings, ctx| {
                        report_if_error!(
                            settings
                                .open_conversation_layout_preference
                                .set_value(*layout, ctx)
                        );
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
            WarpAgentPageAction::ToggleShowConversationHistory => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .show_conversation_history
                            .toggle_and_save_value(ctx)
                    );
                });
                ctx.notify();
            }
            #[cfg(feature = "local_fs")]
            WarpAgentPageAction::OpenAddCustomRouter => {
                ctx.emit(WarpAgentPageEvent::OpenCustomRouterEditor(None));
            }
            WarpAgentPageAction::OpenAddCustomEndpointModal => {
                self.show_add_custom_endpoint_modal(ctx);
            }
            WarpAgentPageAction::OpenEditCustomEndpointModal(index) => {
                self.show_edit_custom_endpoint_modal(*index, ctx);
            }
            WarpAgentPageAction::ToggleCloudHandoff => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .should_force_disable_cloud_handoff
                            .toggle_and_save_value(ctx)
                    );
                });
                ctx.notify();
            }
            WarpAgentPageAction::ToggleAmpersandHandoff => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .should_force_disable_ampersand_handoff
                            .toggle_and_save_value(ctx)
                    );
                });
                ctx.notify();
            }
            WarpAgentPageAction::ToggleAutoHandoffOnSleep => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .auto_handoff_on_sleep_enabled
                            .toggle_and_save_value(ctx)
                    );
                });
                ctx.notify();
            }
            WarpAgentPageAction::ToggleAgentAttribution => {
                // The updated value syncs to warp-server automatically via
                // `CloudPreferencesSyncer` as a `JsonPreference` GSO keyed
                // `Global_AgentAttributionEnabled`; no bespoke server call needed.
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(
                        settings
                            .agent_attribution_enabled
                            .toggle_and_save_value(ctx)
                    );
                });
                ctx.notify();
            }
            WarpAgentPageAction::ConnectGrokSubscription => {
                #[cfg(not(target_family = "wasm"))]
                self.start_grok_oauth(ctx);
            }
            WarpAgentPageAction::DisconnectGrokSubscription => {
                #[cfg(not(target_family = "wasm"))]
                {
                    self.grok_oauth_attempt = None;
                    self.grok_code_editor.update(ctx, |editor, ctx| {
                        editor.clear_buffer(ctx);
                    });
                }
                ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                    manager.set_grok_tokens(None, ctx);
                });

                let window_id = ctx.window_id();
                crate::ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = crate::view_components::DismissibleToast::default(
                        "SuperGrok subscription disconnected".to_string(),
                    );
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                ctx.notify();
            }
        }
    }
}

impl SettingsPageMeta for WarpAgentPageView {
    fn section() -> SettingsSection {
        SettingsSection::WarpAgent
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        FeatureFlag::AgentMode.is_enabled()
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

impl From<ViewHandle<WarpAgentPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<WarpAgentPageView>) -> Self {
        SettingsPageViewHandle::WarpAgent(view_handle)
    }
}

#[derive(Default)]
struct GlobalAIWidget {
    switch_state: SwitchStateHandle,
    sign_up_button: MouseStateHandle,
}

impl SettingsWidget for GlobalAIWidget {
    type View = WarpAgentPageView;

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
                    "Warp Agent",
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
                        Text::new("Your organization disallows AI when the active pane contains content from a remote session", appearance.ui_font_family(), 12.)
                            .with_color(appearance.theme().ui_warning_color())
                            .finish()
                    )
                    .with_padding_left(8.)
                    .with_padding_right(8.)
                    .finish()
                )
                .with_max_width(400.)
                .finish()
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
                                "To use AI features, please create an account.",
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
                                .with_text_label("Sign up".to_owned())
                                .build()
                                .on_click(move |ctx, _, _| {
                                    ctx.dispatch_typed_action(
                                        WarpAgentPageAction::SignupAnonymousUser,
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
                            ctx.dispatch_typed_action(WarpAgentPageAction::ToggleGlobalAI);
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

struct ActiveAIWidget {
    view_handle: WeakViewHandle<WarpAgentPageView>,
    active_ai_toggle: SwitchStateHandle,
    intelligent_autosuggestions_toggle: SwitchStateHandle,
    prompt_suggestions_toggle: SwitchStateHandle,
    code_suggestions_toggle: SwitchStateHandle,
    natural_language_autosuggestions_toggle: SwitchStateHandle,
    shared_block_title_generation_toggle: SwitchStateHandle,
    git_operations_autogen_toggle: SwitchStateHandle,
}

impl ActiveAIWidget {
    fn new(ctx: &ViewContext<WarpAgentPageView>) -> Self {
        Self {
            view_handle: ctx.handle(),
            active_ai_toggle: Default::default(),
            intelligent_autosuggestions_toggle: Default::default(),
            prompt_suggestions_toggle: Default::default(),
            code_suggestions_toggle: Default::default(),
            natural_language_autosuggestions_toggle: Default::default(),
            shared_block_title_generation_toggle: Default::default(),
            git_operations_autogen_toggle: Default::default(),
        }
    }
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
                .team_for_view_handle(&self.view_handle, app)
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
        view: &WarpAgentPageView,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_active_ai_enabled(app);

        Flex::column()
            .with_child(
                render_ai_setting_toggle::<IntelligentAutosuggestionsEnabled>(
                    "Next Command",
                    WarpAgentPageAction::ToggleIntelligentAutosuggestions,
                    *ai_settings.intelligent_autosuggestions_enabled_internal,
                    is_toggleable,
                    self.intelligent_autosuggestions_toggle.clone(),
                    &view.local_only_icon_tooltip_states,
                    app,
                ),
            )
            .with_child(render_ai_setting_description(
                NEXT_COMMAND_DESCRIPTION,
                is_toggleable,
                app,
            ))
            .finish()
    }

    fn render_prompt_suggestions_section(
        &self,
        view: &WarpAgentPageView,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_active_ai_enabled(app);
        Flex::column()
            .with_child(
                render_ai_setting_toggle::<AgentModeQuerySuggestionsEnabled>(
                    "Prompt Suggestions",
                    WarpAgentPageAction::TogglePromptSuggestions,
                    *ai_settings.prompt_suggestions_enabled_internal,
                    is_toggleable,
                    self.prompt_suggestions_toggle.clone(),
                    &view.local_only_icon_tooltip_states,
                    app,
                ),
            )
            .with_child(render_ai_setting_description(
                PROMPT_SUGGESTIONS_DESCRIPTION,
                is_toggleable,
                app,
            ))
            .finish()
    }

    fn render_suggested_code_banners_section(
        &self,
        view: &WarpAgentPageView,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_active_ai_enabled(app);
        Flex::column()
            .with_child(
                render_ai_setting_toggle::<AgentModeQuerySuggestionsEnabled>(
                    "Suggested Code Banners",
                    WarpAgentPageAction::ToggleCodeSuggestions,
                    *ai_settings.code_suggestions_enabled_internal,
                    is_toggleable,
                    self.code_suggestions_toggle.clone(),
                    &view.local_only_icon_tooltip_states,
                    app,
                ),
            )
            .with_child(render_ai_setting_description(
                SUGGESTED_CODE_BANNERS_DESCRIPTION,
                is_toggleable,
                app,
            ))
            .finish()
    }

    fn render_natural_language_autosuggestions_section(
        &self,
        view: &WarpAgentPageView,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_active_ai_enabled(app);
        Flex::column()
            .with_child(render_ai_setting_toggle::<
                NaturalLanguageAutosuggestionsEnabled,
            >(
                "Natural Language Autosuggestions",
                WarpAgentPageAction::ToggleNaturalLanguageAutosuggestions,
                *ai_settings.natural_language_autosuggestions_enabled_internal,
                is_toggleable,
                self.natural_language_autosuggestions_toggle.clone(),
                &view.local_only_icon_tooltip_states,
                app,
            ))
            .with_child(render_ai_setting_description(
                NATURAL_LANGUAGE_AUTOSUGGESTIONS,
                is_toggleable,
                app,
            ))
            .finish()
    }

    fn render_shared_block_title_generation_section(
        &self,
        view: &WarpAgentPageView,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_active_ai_enabled(app);
        Flex::column()
            .with_child(
                render_ai_setting_toggle::<SharedBlockTitleGenerationEnabled>(
                    "Shared Block Title Generation",
                    WarpAgentPageAction::ToggleSharedTitleGeneration,
                    *ai_settings.shared_block_title_generation_enabled_internal,
                    is_toggleable,
                    self.shared_block_title_generation_toggle.clone(),
                    &view.local_only_icon_tooltip_states,
                    app,
                ),
            )
            .with_child(render_ai_setting_description(
                SHARED_BLOCK_TITLE_GENERATION_DESCRIPTION,
                is_toggleable,
                app,
            ))
            .finish()
    }

    fn render_git_operations_autogen_section(
        &self,
        view: &WarpAgentPageView,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_active_ai_enabled(app);
        Flex::column()
            .with_child(render_ai_setting_toggle::<GitOperationsAutogenEnabled>(
                "Commit & Pull Request Generation",
                WarpAgentPageAction::ToggleGitOperationsAutogen,
                *ai_settings.git_operations_autogen_enabled_internal,
                is_toggleable,
                self.git_operations_autogen_toggle.clone(),
                &view.local_only_icon_tooltip_states,
                app,
            ))
            .with_child(render_ai_setting_description(
                GIT_OPERATIONS_AUTOGEN_DESCRIPTION,
                is_toggleable,
                app,
            ))
            .finish()
    }
}

impl SettingsWidget for ActiveAIWidget {
    type View = WarpAgentPageView;

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
                                "Active AI",
                                Some(styles::header_font_color(is_any_ai_enabled, app)),
                            )
                            .finish(),
                        )
                        .with_child(
                            Container::new(render_ai_feature_switch(
                                self.active_ai_toggle.clone(),
                                *ai_settings.is_active_ai_enabled_internal,
                                is_any_ai_enabled,
                                WarpAgentPageAction::ToggleActiveAI,
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
struct AIInputWidget {
    incorrect_autodetection_highlight_index: HighlightedHyperlink,
    autodetection_toggle: SwitchStateHandle,
    nld_in_terminal_toggle: SwitchStateHandle,
    show_input_hint_toggle: SwitchStateHandle,
    show_agent_tips_toggle: SwitchStateHandle,
    include_agent_commands_in_history_toggle: SwitchStateHandle,
    auto_approve_bypasses_command_denylist_toggle: SwitchStateHandle,
}

impl SettingsWidget for AIInputWidget {
    type View = WarpAgentPageView;

    fn search_terms(&self) -> &str {
        "oz agent ai input natural language detection autodetection prompt terminal command commands history shell executed execution queue interrupt submission submit auto-queue response while responding default long-running long running lrc auto-approve fast forward denylist permissions"
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
            "Input",
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
            "Show input hint text",
            WarpAgentPageAction::ToggleShowInputHintText,
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
                "Show agent tips",
                WarpAgentPageAction::ToggleShowAgentTips,
                *InputSettings::as_ref(app).show_agent_tips,
                is_any_ai_enabled,
                self.show_agent_tips_toggle.clone(),
                &view.local_only_icon_tooltip_states,
                app,
            );
            widget_children.push(agent_tips_toggle);
        }

        widget_children.push(render_ai_setting_toggle::<IncludeAgentCommandsInHistory>(
            "Include agent-executed commands in history",
            WarpAgentPageAction::ToggleIncludeAgentCommandsInHistory,
            *ai_settings.include_agent_commands_in_history,
            is_any_ai_enabled,
            self.include_agent_commands_in_history_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        ));

        widget_children.push(
            Flex::column()
                .with_child(render_ai_setting_toggle::<
                    AutoApproveBypassesCommandDenylist,
                >(
                    "Allow auto-approve to bypass command denylist",
                    WarpAgentPageAction::ToggleAutoApproveBypassesCommandDenylist,
                    *ai_settings.auto_approve_bypasses_command_denylist,
                    is_any_ai_enabled,
                    self.auto_approve_bypasses_command_denylist_toggle
                        .clone(),
                    &view.local_only_icon_tooltip_states,
                    app,
                ))
                .with_child(render_ai_setting_description(
                    "When enabled, fast forward and auto-approve run denylisted commands without asking for confirmation.",
                    is_any_ai_enabled,
                    app,
                ))
                .finish(),
        );

        if FeatureFlag::QueueSlashCommand.is_enabled() {
            widget_children.push(render_dropdown_item(
                appearance,
                "Default prompt submission mode",
                Some(
                    "What happens when you submit a new prompt while the agent is still \
                     responding. You can override this per conversation using the auto-queue \
                     toggle.",
                ),
                None,
                LocalOnlyIconState::for_setting(
                    PromptSubmissionMode::storage_key(),
                    PromptSubmissionMode::sync_to_cloud(),
                    &mut view.local_only_icon_tooltip_states.borrow_mut(),
                    app,
                ),
                (!is_any_ai_enabled).then(|| appearance.theme().disabled_ui_text_color()),
                &view.default_prompt_submission_mode_dropdown,
            ));

            // Only meaningful in Interrupt mode: with Queue selected, prompts already
            // queue until the end of the full response, so the LRC mode is hidden.
            if ai_settings.default_prompt_submission_mode == PromptSubmissionMode::Interrupt {
                widget_children.push(
                    Container::new(render_dropdown_item(
                        appearance,
                        "Default long-running command submission mode",
                        Some(
                            "What happens when you submit a prompt while an agent is driving an \
                             agent-requested long-running command. Queued prompts are sent to the \
                             agent when the command finishes.",
                        ),
                        None,
                        LocalOnlyIconState::for_setting(
                            LongRunningCommandSubmissionMode::storage_key(),
                            LongRunningCommandSubmissionMode::sync_to_cloud(),
                            &mut view.local_only_icon_tooltip_states.borrow_mut(),
                            app,
                        ),
                        (!is_any_ai_enabled).then(|| appearance.theme().disabled_ui_text_color()),
                        &view.lrc_submission_mode_dropdown,
                    ))
                    .with_margin_top(styles::DESCRIPTION_MARGIN_BOTTOM)
                    .finish(),
                );
            }
        }

        Flex::column().with_children(widget_children).finish()
    }
}

impl AIInputWidget {
    fn render_natural_language_detection_section(
        incorrect_autodetection_highlight_index: HighlightedHyperlink,
        autodetection_toggle: SwitchStateHandle,
        nld_in_terminal_toggle: SwitchStateHandle,
        view: &WarpAgentPageView,
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
            static AUTODETECTION_DESCRIPTION_FRAGMENTS: LazyLock<Vec<FormattedTextFragment>> =
                LazyLock::new(|| {
                    vec![
                        FormattedTextFragment::plain_text("Encountered an incorrect detection? "),
                        FormattedTextFragment::hyperlink(
                            "Let us know",
                            "https://warpdotdev.typeform.com/to/offrTIpq",
                        ),
                    ]
                });

            section.add_children([
                render_ai_setting_toggle::<NLDInTerminalEnabled>(
                    "Autodetect agent prompts in terminal input",
                    WarpAgentPageAction::ToggleNLDInTerminal,
                    ai_settings.is_nld_in_terminal_enabled(app),
                    is_toggleable,
                    nld_in_terminal_toggle,
                    &view.local_only_icon_tooltip_states,
                    app,
                ),
                render_ai_setting_toggle::<AIAutoDetectionEnabled>(
                    "Autodetect terminal commands in agent input",
                    WarpAgentPageAction::ToggleAIInputAutoDetection,
                    is_nld_enabled,
                    is_toggleable,
                    autodetection_toggle,
                    &view.local_only_icon_tooltip_states,
                    app,
                ),
                Container::new(
                    FormattedTextElement::new(
                        FormattedText::new([FormattedTextLine::Line(
                            (*AUTODETECTION_DESCRIPTION_FRAGMENTS).clone(),
                        )]),
                        CONTENT_FONT_SIZE,
                        appearance.ui_font_family(),
                        appearance.ui_font_family(),
                        styles::description_font_color(is_toggleable, app).into(),
                        incorrect_autodetection_highlight_index,
                    )
                    .with_hyperlink_font_color(appearance.theme().accent().into_solid())
                    .register_default_click_handlers(|url, ctx, _| {
                        ctx.dispatch_typed_action(WarpAgentPageAction::HyperlinkClick(url));
                    })
                    .finish(),
                )
                .with_margin_top(styles::DESCRIPTION_NEGATIVE_MARGIN_OFFSET)
                .with_margin_bottom(styles::DESCRIPTION_MARGIN_BOTTOM)
                .with_margin_right(styles::TOGGLE_WIDTH_MARGIN)
                .finish(),
            ])
        } else {
            static NATURAL_LANGUAGE_DETECTION_DESCRIPTION_FRAGMENTS: LazyLock<
                Vec<FormattedTextFragment>,
            > = LazyLock::new(|| {
                vec![
                    FormattedTextFragment::plain_text(
                        "Enabling natural language detection will detect when natural language is written in the terminal input, and then automatically switch to Agent Mode for AI queries.",
                    ),
                    FormattedTextFragment::plain_text(
                        " Encountered an incorrect input detection? ",
                    ),
                    FormattedTextFragment::hyperlink(
                        "Let us know",
                        "https://warpdotdev.typeform.com/to/offrTIpq",
                    ),
                ]
            });

            section.add_children([
                render_ai_setting_toggle::<AIAutoDetectionEnabled>(
                    "Natural language detection",
                    WarpAgentPageAction::ToggleAIInputAutoDetection,
                    is_nld_enabled,
                    is_toggleable,
                    autodetection_toggle,
                    &view.local_only_icon_tooltip_states,
                    app,
                ),
                Container::new(
                    FormattedTextElement::new(
                        FormattedText::new([FormattedTextLine::Line(
                            (*NATURAL_LANGUAGE_DETECTION_DESCRIPTION_FRAGMENTS).clone(),
                        )]),
                        CONTENT_FONT_SIZE,
                        appearance.ui_font_family(),
                        appearance.ui_font_family(),
                        styles::description_font_color(is_toggleable, app).into(),
                        incorrect_autodetection_highlight_index,
                    )
                    .with_hyperlink_font_color(appearance.theme().accent().into_solid())
                    .register_default_click_handlers(|url, ctx, _| {
                        ctx.dispatch_typed_action(WarpAgentPageAction::HyperlinkClick(url));
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
                "Natural language denylist".to_owned(),
                is_toggleable,
                &view.local_only_icon_tooltip_states,
                app,
            ))
            .with_child(render_ai_setting_description(
                "Commands listed here will never trigger natural language detection.",
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
struct VoiceWidget {
    voice_input_toggle: SwitchStateHandle,
    wispr_highlight_index: HighlightedHyperlink,
}

impl VoiceWidget {
    fn render_voice_section(
        &self,
        view: &WarpAgentPageView,
        appearance: &Appearance,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_any_ai_enabled(app);
        let mut column = Flex::column().with_child(render_ai_setting_toggle::<VoiceInputEnabled>(
            "Voice Input",
            WarpAgentPageAction::ToggleVoiceInput,
            *ai_settings.voice_input_enabled_internal,
            is_toggleable,
            self.voice_input_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        ));

        let voice_input_description_text_fragments = vec![
            FormattedTextFragment::plain_text(
                "Voice input allows you to control Warp by speaking directly to your terminal (powered by ",
            ),
            FormattedTextFragment::hyperlink("Wispr Flow", WISPR_FLOW_URL),
            FormattedTextFragment::plain_text(")."),
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
            ctx.dispatch_typed_action(WarpAgentPageAction::HyperlinkClick(url));
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
                "Key for Activating Voice Input",
                Some("Press and hold to activate."),
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
            column.add_child(render_filterable_dropdown_item(
                appearance,
                "Speech Language",
                Some("Language used when transcribing voice input."),
                None,
                LocalOnlyIconState::for_setting(
                    VoiceInputLanguage::storage_key(),
                    VoiceInputLanguage::sync_to_cloud(),
                    &mut view.local_only_icon_tooltip_states.borrow_mut(),
                    app,
                ),
                None,
                &view.voice_input_language_dropdown,
            ));
        }

        column.finish()
    }
}

impl SettingsWidget for VoiceWidget {
    type View = WarpAgentPageView;

    fn search_terms(&self) -> &str {
        "voice agent oz ai a.i. speech input natural language talk english spanish french german estonian finnish"
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
                    "Voice",
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
}

impl OtherAIWidget {
    fn create_thinking_display_mode_dropdown(
        ctx: &mut ViewContext<WarpAgentPageView>,
    ) -> ViewHandle<Dropdown<WarpAgentPageAction>> {
        let items: Vec<DropdownItem<WarpAgentPageAction>> = ThinkingDisplayMode::iter()
            .map(|mode| {
                DropdownItem::new(
                    mode.display_name(),
                    WarpAgentPageAction::SetThinkingDisplayMode(mode),
                )
            })
            .collect();

        ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);
            dropdown.set_menu_max_height(AI_SETTINGS_DROPDOWN_MAX_HEIGHT, ctx);
            dropdown.add_items(items, ctx);
            dropdown
        })
    }

    fn create_default_prompt_submission_mode_dropdown(
        ctx: &mut ViewContext<WarpAgentPageView>,
    ) -> ViewHandle<Dropdown<WarpAgentPageAction>> {
        let items: Vec<DropdownItem<WarpAgentPageAction>> = PromptSubmissionMode::iter()
            .map(|mode| {
                DropdownItem::new(
                    mode.display_name(),
                    WarpAgentPageAction::SetPromptSubmissionMode(mode),
                )
            })
            .collect();

        ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);
            dropdown.set_menu_max_height(AI_SETTINGS_DROPDOWN_MAX_HEIGHT, ctx);
            dropdown.add_items(items, ctx);
            dropdown
        })
    }

    fn create_lrc_submission_mode_dropdown(
        ctx: &mut ViewContext<WarpAgentPageView>,
    ) -> ViewHandle<Dropdown<WarpAgentPageAction>> {
        let items: Vec<DropdownItem<WarpAgentPageAction>> =
            LongRunningCommandSubmissionMode::iter()
                .map(|mode| {
                    DropdownItem::new(
                        mode.display_name(),
                        WarpAgentPageAction::SetLongRunningCommandSubmissionMode(mode),
                    )
                })
                .collect();

        ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);
            dropdown.set_menu_max_height(AI_SETTINGS_DROPDOWN_MAX_HEIGHT, ctx);
            dropdown.add_items(items, ctx);
            dropdown
        })
    }

    fn create_orchestration_message_display_mode_dropdown(
        ctx: &mut ViewContext<WarpAgentPageView>,
    ) -> ViewHandle<Dropdown<WarpAgentPageAction>> {
        let items: Vec<DropdownItem<WarpAgentPageAction>> = OrchestrationMessageDisplayMode::iter()
            .map(|mode| {
                DropdownItem::new(
                    mode.display_name(),
                    WarpAgentPageAction::SetOrchestrationMessageDisplayMode(mode),
                )
            })
            .collect();

        ctx.add_typed_action_view(|ctx| {
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_top_bar_max_width(AI_SETTINGS_DROPDOWN_WIDTH);
            dropdown.set_menu_width(AI_SETTINGS_DROPDOWN_WIDTH, ctx);
            dropdown.set_menu_max_height(AI_SETTINGS_DROPDOWN_MAX_HEIGHT, ctx);
            dropdown.add_items(items, ctx);
            dropdown
        })
    }
}

impl SettingsWidget for OtherAIWidget {
    type View = WarpAgentPageView;

    fn search_terms(&self) -> &str {
        "other oz updates zero state empty changelog new conversation agent what's new use agent footer toolbar layout chip chips rearrange re-arrange thinking expanded reasoning collapse never show orchestration messages child agents collapse expand hide conversation history"
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
                    "Other",
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            );

        if FeatureFlag::AgentView.is_enabled() {
            let mut agent_view_column = Flex::column()
                .with_child(render_ai_setting_toggle::<ShouldShowOzUpdatesInZeroState>(
                    "Show Warp Agent changelog in new conversation view",
                    WarpAgentPageAction::ToggleShowOzUpdatesInZeroState,
                    *ai_settings.should_show_oz_updates_in_zero_state,
                    is_toggleable,
                    self.show_oz_updates_in_zero_state_toggle.clone(),
                    &view.local_only_icon_tooltip_states,
                    app,
                ))
                .with_child(render_ai_setting_toggle::<ShouldRenderUseAgentToolbarForUserCommands>(
                    "Show \"Use Agent\" footer",
                    WarpAgentPageAction::ToggleUseAgentToolbar,
                    *ai_settings.should_render_use_agent_footer_for_user_commands,
                    is_toggleable,
                    self.use_agent_footer_toggle.clone(),
                    &view.local_only_icon_tooltip_states,
                    app,
                ))
                .with_child(render_ai_setting_description(
                    "Shows hint to use the \"Full Terminal Use\"-enabled agent in long running commands.",
                    is_toggleable,
                    app,
                ));

            if is_toggleable && FeatureFlag::AgentToolbarEditor.is_enabled() {
                agent_view_column.add_child(render_toolbar_layout_editor(
                    &view.agent_toolbar_inline_editor,
                    appearance,
                ));
            }

            column.add_child(agent_view_column.finish());
        }

        column.add_child(render_ai_setting_toggle::<ShowConversationHistory>(
            "Show conversation history in tools panel",
            WarpAgentPageAction::ToggleShowConversationHistory,
            *ai_settings.show_conversation_history,
            is_toggleable,
            self.show_conversation_history_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        ));

        column.add_child(render_dropdown_item(
            appearance,
            "Agent thinking display",
            Some("Controls how reasoning/thinking traces are displayed."),
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

        column.add_child(render_dropdown_item(
            appearance,
            "Orchestration message display",
            Some("Controls whether orchestration messages stay expanded."),
            None,
            LocalOnlyIconState::for_setting(
                OrchestrationMessageDisplayMode::storage_key(),
                OrchestrationMessageDisplayMode::sync_to_cloud(),
                &mut view.local_only_icon_tooltip_states.borrow_mut(),
                app,
            ),
            (!is_any_ai_enabled).then(|| appearance.theme().disabled_ui_text_color()),
            &view.orchestration_message_display_mode_dropdown,
        ));

        // TODO: OpenConversationLayoutPreference should not depend on local_fs, but it lives under the external editor settings
        // which does require local_fs. It was a mistake to put it there, but now we keep it there for backward compatibility.
        #[cfg(feature = "local_fs")]
        if FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
            use crate::util::file::external_editor::settings::OpenConversationLayoutPreference;

            column.add_child(render_dropdown_item(
                appearance,
                "Preferred layout when opening existing agent conversations",
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
    type View = WarpAgentPageView;

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
                    text: "This option is enforced by your organization's settings and cannot be customized.".to_string(),
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
                    ctx.dispatch_typed_action(WarpAgentPageAction::ToggleAgentAttribution);
                })
                .finish()
        };

        let toggle_row = build_toggle_element(
            render_body_item_label::<WarpAgentPageAction>(
                "Enable agent attribution".to_string(),
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
                    "Agent Attribution",
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            )
            .with_child(toggle_row)
            .with_child(render_ai_setting_description(
                "Warp Agent can add attribution to commit messages and pull requests it creates",
                !state.is_disabled,
                app,
            ))
            .finish()
    }
}

#[cfg(test)]
#[path = "warp_agent_page_tests.rs"]
mod tests;

#[derive(Default)]
struct CloudAgentComputerUseWidget {
    toggle: SwitchStateHandle,
}

impl SettingsWidget for CloudAgentComputerUseWidget {
    type View = WarpAgentPageView;

    fn search_terms(&self) -> &str {
        "oz cloud agent computer use orchestration multi-agent"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        use crate::ai::execution_profiles::{
            CloudAgentComputerUseState, resolve_cloud_agent_computer_use_state,
        };

        let is_any_ai_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);

        // Determine toggle state based on workspace autonomy setting and user preference
        let CloudAgentComputerUseState {
            enabled: is_checked,
            is_forced_by_org,
        } = resolve_cloud_agent_computer_use_state(app);

        // Toggle is disabled if forced by org settings OR if AI is globally disabled
        let is_disabled = is_forced_by_org || !is_any_ai_enabled;

        let ui_builder = appearance.ui_builder();
        let toggle = if is_forced_by_org {
            // Disabled by organization setting - show tooltip on hover
            ui_builder
                .switch(self.toggle.clone())
                .check(is_checked)
                .with_tooltip(TooltipConfig {
                    text: "This option is enforced by your organization's settings and cannot be customized.".to_string(),
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
                    ctx.dispatch_typed_action(WarpAgentPageAction::ToggleCloudAgentComputerUse);
                })
                .finish()
        };

        let toggle_row = build_toggle_element(
            render_body_item_label::<WarpAgentPageAction>(
                "Computer use in Cloud Agents".to_string(),
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
                    "Experimental",
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            )
            .with_child(toggle_row)
            .with_child(render_ai_setting_description(
                "Enable computer use in cloud agent conversations started from the Warp app.",
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
    type View = WarpAgentPageView;

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
                    ctx.dispatch_typed_action(WarpAgentPageAction::ToggleCloudHandoff);
                })
                .finish()
        };

        let handoff_row = build_toggle_element(
            render_body_item_label::<WarpAgentPageAction>(
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
                        ctx.dispatch_typed_action(WarpAgentPageAction::ToggleAutoHandoffOnSleep);
                    })
                    .finish();
                let auto_handoff_on_sleep_row = build_toggle_element(
                    render_body_item_label::<WarpAgentPageAction>(
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
                    ctx.dispatch_typed_action(WarpAgentPageAction::ToggleAmpersandHandoff);
                })
                .finish();

            let ampersand_row = build_toggle_element(
                render_body_item_label::<WarpAgentPageAction>(
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

struct ProviderApiKeyEditor {
    provider: LLMProvider,
    editor: ViewHandle<EditorView>,
    team_key_info_tooltip: MouseStateHandle,
}

struct ApiKeysWidget {
    view_handle: WeakViewHandle<WarpAgentPageView>,
    provider_api_key_editors: Vec<ProviderApiKeyEditor>,
    /// Buttons for the SuperGrok (xAI) subscription row; which one renders
    /// depends on whether OAuth tokens are stored or a connect attempt is in
    /// progress.
    grok_connect_button: ViewHandle<ActionButton>,
    grok_connecting_button: ViewHandle<ActionButton>,
    grok_disconnect_button: ViewHandle<ActionButton>,

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
        let member_byo_keys_allowed = workspace_handle.as_ref(ctx).are_member_byo_keys_allowed();

        let provider_api_key_editors = LLMProvider::API_KEY_PROVIDERS
            .into_iter()
            .filter(|provider| provider.supports_pasted_api_key())
            .map(|provider| {
                let key = provider
                    .api_key(ApiKeyManager::as_ref(ctx).keys())
                    .map(str::to_owned);
                let placeholder = provider
                    .api_key_placeholder()
                    .expect("API-key providers have input placeholders");
                let editor = ctx.add_typed_action_view(move |ctx| {
                    let appearance = Appearance::handle(ctx).as_ref(ctx);
                    let options = SingleLineEditorOptions {
                        is_password: true,
                        propagate_and_no_op_vertical_navigation_keys:
                            PropagateAndNoOpNavigationKeys::Always,
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
                    editor.set_placeholder_text(placeholder, ctx);
                    if let Some(key) = &key {
                        editor.set_buffer_text(key, ctx);
                    }
                    editor
                });
                update_editor_interaction_state(
                    editor.clone(),
                    is_any_ai_enabled && is_byo_enabled && member_byo_keys_allowed,
                    ctx,
                );
                ctx.subscribe_to_view(&editor, move |_, editor, event, ctx| {
                    if matches!(event, EditorEvent::Blurred | EditorEvent::Enter) {
                        let buffer_text = editor.as_ref(ctx).buffer_text(ctx);
                        let key = buffer_text.is_empty().not().then_some(buffer_text);
                        ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                            manager.set_provider_key(provider, key, ctx);
                        });
                    }
                });
                let editor_clone = editor.clone();
                ctx.subscribe_to_model(&workspace_handle, move |_, workspace, event, ctx| {
                    if let UserWorkspacesEvent::TeamsChanged = event {
                        let is_any_ai_enabled =
                            AISettings::handle(ctx).as_ref(ctx).is_any_ai_enabled(ctx);
                        let is_byo_enabled = workspace.as_ref(ctx).is_byo_api_key_enabled(ctx);
                        let member_byo_keys_allowed =
                            workspace.as_ref(ctx).are_member_byo_keys_allowed();
                        let is_enabled = is_any_ai_enabled && is_byo_enabled;
                        let has_key = !editor_clone.as_ref(ctx).is_empty(ctx);
                        if !is_byo_enabled && has_key {
                            editor_clone.update(ctx, |editor, ctx| {
                                editor.set_buffer_text("", ctx);
                            });
                            ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                                manager.set_provider_key(provider, None, ctx);
                            });
                        }
                        update_editor_interaction_state(
                            editor_clone.clone(),
                            is_enabled && member_byo_keys_allowed,
                            ctx,
                        );
                        ctx.notify();
                    }
                });
                ProviderApiKeyEditor {
                    provider,
                    editor,
                    team_key_info_tooltip: MouseStateHandle::default(),
                }
            })
            .collect::<Vec<_>>();

        // Tab / Shift-Tab move focus between the provider key fields instead of
        // inserting whitespace.
        let provider_key_editors = provider_api_key_editors
            .iter()
            .map(|provider| provider.editor.clone())
            .collect::<Vec<_>>();
        for (index, editor) in provider_key_editors.iter().enumerate() {
            let next = provider_key_editors.get(index + 1).cloned();
            let previous = index
                .checked_sub(1)
                .and_then(|prev_index| provider_key_editors.get(prev_index).cloned());
            ctx.subscribe_to_view(editor, move |_, _, event, ctx| match event {
                EditorEvent::Navigate(NavigationKey::Tab) => {
                    if let Some(next) = &next {
                        ctx.focus(next);
                    }
                }
                EditorEvent::Navigate(NavigationKey::ShiftTab) => {
                    if let Some(previous) = &previous {
                        ctx.focus(previous);
                    }
                }
                _ => {}
            });
        }

        // Editor text colors are snapshotted at construction via
        // `text_colors_override`, so refresh them whenever the theme changes.
        let api_key_editors = provider_key_editors.clone();
        ctx.subscribe_to_model(&Appearance::handle(ctx), move |_, _, event, ctx| {
            if let AppearanceEvent::ThemeChanged = event {
                let text_colors = editor_text_colors(Appearance::as_ref(ctx));
                for editor in &api_key_editors {
                    let colors = text_colors.clone();
                    editor.update(ctx, move |editor, ctx| {
                        editor.set_text_colors(colors, ctx);
                    });
                }
            }
        });

        let grok_connect_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Connect", SecondaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(WarpAgentPageAction::ConnectGrokSubscription);
                })
        });
        let grok_connecting_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Connecting", SecondaryTheme).with_size(ButtonSize::Small)
        });
        grok_connecting_button.update(ctx, |button, ctx| {
            button.set_disabled(true, ctx);
        });
        let grok_disconnect_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Disconnect", DangerSecondaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(WarpAgentPageAction::DisconnectGrokSubscription);
                })
        });
        for button in [&grok_connect_button, &grok_disconnect_button] {
            button.update(ctx, |button, ctx| {
                button.set_disabled(
                    !(is_any_ai_enabled && is_byo_enabled && member_byo_keys_allowed),
                    ctx,
                );
            });
        }

        // The Grok subscription is BYO auth, so keep the buttons' enablement
        // in sync with the BYO API key policy, like the editors above.
        let grok_buttons = [grok_connect_button.clone(), grok_disconnect_button.clone()];
        ctx.subscribe_to_model(&workspace_handle, move |_, workspace, event, ctx| {
            if let UserWorkspacesEvent::TeamsChanged = event {
                let is_any_ai_enabled = AISettings::handle(ctx).as_ref(ctx).is_any_ai_enabled(ctx);
                let is_byo_enabled = workspace.as_ref(ctx).is_byo_api_key_enabled(ctx);
                let member_byo_keys_allowed = workspace.as_ref(ctx).are_member_byo_keys_allowed();
                for button in &grok_buttons {
                    button.update(ctx, |button, ctx| {
                        button.set_disabled(
                            !(is_any_ai_enabled && is_byo_enabled && member_byo_keys_allowed),
                            ctx,
                        );
                    });
                }
                ctx.notify();
            }
        });

        // Re-render the SuperGrok row whenever the stored tokens change (the
        // connect flow completes, a disconnect, or a background refresh).
        ctx.subscribe_to_model(&ApiKeyManager::handle(ctx), |_, _, event, ctx| {
            if matches!(event, ApiKeyManagerEvent::KeysUpdated) {
                ctx.notify();
            }
        });

        Self {
            view_handle: ctx.handle(),
            provider_api_key_editors,

            grok_connect_button,
            grok_connecting_button,
            grok_disconnect_button,

            can_use_warp_credits_for_fallback: Default::default(),
            upgrade_highlight_index: Default::default(),

            custom_inference_info_tooltip: Default::default(),
            custom_inference_terms_index: Default::default(),
            description_learn_more_index: Default::default(),
        }
    }
    fn has_team_first_party_key(provider: &LLMProvider, app: &AppContext) -> bool {
        UserWorkspaces::as_ref(app)
            .current_workspace()
            .is_some_and(|workspace| {
                workspace.billing_metadata.is_managed_byok_byoe_enabled()
                    && workspace
                        .settings
                        .team_byo
                        .as_ref()
                        .is_some_and(|team_byo| {
                            team_byo.first_party_enabled
                                && team_byo
                                    .first_party_keys
                                    .iter()
                                    .any(|key| key.provider == *provider)
                        })
            })
    }

    fn render_team_key_info_icon(
        &self,
        provider: &LLMProvider,
        mouse_state: MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let provider_name = provider.display_name();
        let tooltip_text = FormattedText::new([FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text(format!(
                "Your organization has provided an API key for {provider_name}. A key entered here takes precedence for {provider_name} requests."
            )),
        ])]);
        let tooltip_background = appearance.theme().tooltip_background();
        let icon_color = appearance.theme().active_ui_text_color();

        Hoverable::new(mouse_state, move |state| {
            let icon = ConstrainedBox::new(Icon::Info.to_warpui_icon(icon_color).finish())
                .with_width(13.)
                .with_height(13.)
                .finish();
            let mut stack = Stack::new().with_child(icon);
            if state.is_hovered() {
                let tooltip = ConstrainedBox::new(
                    Container::new(
                        FormattedTextElement::new(
                            tooltip_text.clone(),
                            10.,
                            appearance.ui_font_family(),
                            appearance.ui_font_family(),
                            appearance.theme().background().into_solid(),
                            HighlightedHyperlink::default(),
                        )
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
                stack.add_positioned_overlay_child(
                    tooltip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -3.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopMiddle,
                        ChildAnchor::BottomLeft,
                    ),
                );
            }
            stack.finish()
        })
        .finish()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_api_key_input(
        &self,
        appearance: &Appearance,
        label: String,
        provider: LLMProvider,
        team_key_info_tooltip: MouseStateHandle,
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
        let mut label_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(label);
        if Self::has_team_first_party_key(&provider, app) {
            label_row.add_child(
                Container::new(self.render_team_key_info_icon(
                    &provider,
                    team_key_info_tooltip,
                    appearance,
                ))
                .with_margin_left(4.)
                .finish(),
            );
        }

        let input = appearance
            .ui_builder()
            .text_input(editor)
            .with_style(editor_style)
            .build()
            .finish();

        Flex::column()
            .with_spacing(8.)
            .with_child(label_row.finish())
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
        for provider_editor in &self.provider_api_key_editors {
            column.add_child(self.render_api_key_input(
                appearance,
                format!("{} API key", provider_editor.provider.display_name()),
                provider_editor.provider,
                provider_editor.team_key_info_tooltip.clone(),
                provider_editor.editor.clone(),
                is_enabled,
                app,
            ));
        }
        column.finish()
    }

    fn render_custom_inference_description(
        &self,
        show_provider_keys: bool,
        show_custom_endpoints: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut lines = Vec::new();
        let mut add_paragraph = |fragments| {
            if !lines.is_empty() {
                lines.push(FormattedTextLine::LineBreak);
            }
            lines.push(FormattedTextLine::Line(fragments));
        };

        if show_provider_keys {
            add_paragraph(vec![FormattedTextFragment::plain_text(
                "Use your own API keys from model providers for Warp Agent. API keys are used to make requests to your chosen model provider. Using auto models or models you do not have available API keys for will consume Warp credits.",
            )]);
        }

        if show_custom_endpoints {
            add_paragraph(vec![FormattedTextFragment::plain_text(
                "Add custom endpoints to use third-party models. Custom endpoints must support OpenAI Chat Completions, OpenAI Responses, or Anthropic Messages.",
            )]);
        }

        if show_provider_keys || show_custom_endpoints {
            add_paragraph(vec![FormattedTextFragment::plain_text(
                "API keys added here are stored only on this device, not on Warp's servers.",
            )]);
            add_paragraph(vec![FormattedTextFragment::hyperlink(
                "Learn more",
                CUSTOM_INFERENCE_LEARN_MORE_URL,
            )]);
        }
        let description = FormattedTextElement::new(
            FormattedText::new(lines),
            CONTENT_FONT_SIZE,
            appearance.ui_font_family(),
            appearance.ui_font_family(),
            blended_colors::text_sub(appearance.theme(), appearance.theme().surface_1()),
            self.description_learn_more_index.clone(),
        )
        .with_hyperlink_font_color(appearance.theme().accent().into_solid())
        .register_default_click_handlers(|url, ctx, _| {
            ctx.dispatch_typed_action(WarpAgentPageAction::HyperlinkClick(url));
        });
        Container::new(description.finish())
            .with_margin_top(styles::DESCRIPTION_NEGATIVE_MARGIN_OFFSET)
            .with_margin_bottom(styles::DESCRIPTION_MARGIN_BOTTOM)
            .with_margin_right(styles::TOGGLE_WIDTH_MARGIN)
            .finish()
    }

    fn render_custom_inference_info_icon(
        &self,
        appearance: &Appearance,
        managed_byok_byoe_enabled: bool,
    ) -> Box<dyn Element> {
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

        let tooltip_text = if managed_byok_byoe_enabled {
            FormattedText::new([FormattedTextLine::Line(vec![
                FormattedTextFragment::plain_text(
                    "Custom inference settings are managed by your organization.",
                ),
            ])])
        } else {
            FormattedText::new([FormattedTextLine::Line(vec![
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
            ])])
        };
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
                                ctx.dispatch_typed_action(WarpAgentPageAction::HyperlinkClick(url));
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
        view: &WarpAgentPageView,
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

    /// The "Connect SuperGrok subscription" row: label and description on the
    /// left, a Connect/Disconnect button on the right, and a "Connected on
    /// ..." status line underneath while a subscription is connected.
    fn render_grok_subscription_row(
        &self,
        appearance: &Appearance,
        is_enabled: bool,
        is_connecting: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let grok_tokens = ApiKeyManager::as_ref(app).grok_tokens();

        let text_color = styles::header_font_color(is_enabled, app);
        let label = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.)
            .with_child(
                Text::new_inline("Use your", appearance.ui_font_family(), CONTENT_FONT_SIZE)
                    .with_color(text_color.into())
                    .finish(),
            )
            .with_child(
                ConstrainedBox::new(Icon::XLogo.to_warpui_icon(text_color).finish())
                    .with_width(14.)
                    .with_height(14.)
                    .finish(),
            )
            .with_child(
                Text::new_inline(
                    "Premium or SuperGrok subscription",
                    appearance.ui_font_family(),
                    CONTENT_FONT_SIZE,
                )
                .with_color(text_color.into())
                .finish(),
            )
            .finish();

        let button = if grok_tokens.is_some() {
            &self.grok_disconnect_button
        } else if is_connecting {
            &self.grok_connecting_button
        } else {
            &self.grok_connect_button
        };

        let header_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., label).finish())
            .with_child(button.as_ref(app).render(app))
            .finish();

        let description = Container::new(
            Text::new(
                "Connect your SuperGrok subscription to use Grok models in the Warp Agent through your xAI account.",
                appearance.ui_font_family(),
                CONTENT_FONT_SIZE,
            )
            .with_color(styles::description_font_color(is_enabled, app).into())
            .soft_wrap(true)
            .finish(),
        )
        .with_margin_right(styles::TOGGLE_WIDTH_MARGIN)
        .finish();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(header_row)
            .with_child(description);

        if let Some(tokens) = grok_tokens {
            let connected_text = match tokens.connected_at.map(DateTime::<Local>::from) {
                Some(connected_at) => format!(
                    "Connected on {}.",
                    connected_at.format("%m/%d/%Y at %-I:%M%P")
                ),
                // Tokens stored before the connection time was tracked.
                None => "Connected.".to_string(),
            };
            let check = ConstrainedBox::new(
                Icon::Check
                    .to_warpui_icon(appearance.theme().ansi_fg_green().into())
                    .finish(),
            )
            .with_width(12.)
            .with_height(12.)
            .finish();
            let status_text = Text::new_inline(
                connected_text,
                appearance.ui_font_family(),
                CONTENT_FONT_SIZE,
            )
            .with_color(styles::description_font_color(is_enabled, app).into())
            .finish();
            column.add_child(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(4.)
                    .with_child(check)
                    .with_child(status_text)
                    .finish(),
            );
        }

        column.finish()
    }

    /// Paste-the-code fallback for the current SuperGrok connect attempt.
    #[cfg(not(target_family = "wasm"))]
    fn render_grok_manual_code_entry(
        &self,
        view: &WarpAgentPageView,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let editor_style = UiComponentStyles {
            padding: Some(Coords {
                top: 10.,
                bottom: 10.,
                left: 16.,
                right: 16.,
            }),
            background: Some(theme.surface_2().into()),
            ..Default::default()
        };
        let input = appearance
            .ui_builder()
            .text_input(view.grok_code_editor.clone())
            .with_style(editor_style)
            .build()
            .finish();

        let row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(Shrinkable::new(1., input).finish())
            .finish();

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(8.)
            .with_child(row)
            .finish()
    }

    fn render_warp_credit_fallback_toggle(
        &self,
        view: &WarpAgentPageView,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);

        let toggle = render_ai_setting_toggle::<CanUseWarpCreditsForFallback>(
            "Warp credit fallback",
            WarpAgentPageAction::ToggleCanUseWarpCreditsForFallback,
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

/// Visibility and enabled-state rules for the member-facing Custom Inference
/// settings section (provider API keys + custom endpoints).
#[derive(Clone, Copy)]
struct CustomInferenceVisibility {
    is_any_ai_enabled: bool,
    is_byo_enabled: bool,
    show_provider_keys: bool,
    provider_keys_enabled: bool,
    show_custom_inference: bool,
    custom_inference_controls_enabled: bool,
    managed_byok_byoe_enabled: bool,
}

impl CustomInferenceVisibility {
    fn compute(app: &AppContext) -> Self {
        let workspaces = UserWorkspaces::as_ref(app);
        let is_any_ai_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);
        let is_byo_enabled = workspaces.is_byo_api_key_enabled(app);
        let is_custom_inference_enabled = workspaces.is_custom_inference_enabled(app);
        let member_byo_keys_allowed = workspaces.are_member_byo_keys_allowed();
        let member_byo_endpoints_allowed = workspaces.are_member_byo_endpoints_allowed();

        // BYOK: shown even when BYO is off so the upgrade CTA can render.
        let show_provider_keys = member_byo_keys_allowed;
        let provider_keys_enabled = show_provider_keys && is_any_ai_enabled && is_byo_enabled;

        // BYOE (custom endpoints).
        let show_custom_inference = is_custom_inference_enabled && member_byo_endpoints_allowed;
        let custom_inference_controls_enabled = show_custom_inference && is_any_ai_enabled;

        Self {
            is_any_ai_enabled,
            is_byo_enabled,
            show_provider_keys,
            provider_keys_enabled,
            show_custom_inference,
            custom_inference_controls_enabled,
            managed_byok_byoe_enabled: workspaces
                .current_workspace()
                .is_some_and(|workspace| workspace.billing_metadata.is_managed_byok_byoe_enabled()),
        }
    }

    /// Whether any member-facing Custom Inference content renders at all.
    fn show_section(&self) -> bool {
        self.show_provider_keys || self.show_custom_inference
    }

    /// Whether the section header renders in the enabled color.
    fn section_enabled(&self) -> bool {
        self.provider_keys_enabled || self.custom_inference_controls_enabled
    }
}

impl SettingsWidget for ApiKeysWidget {
    type View = WarpAgentPageView;

    fn search_terms(&self) -> &str {
        "api keys bring your own byo openai anthropic google claude gemini gpt custom inference endpoint grok supergrok xai subscription"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let visibility = CustomInferenceVisibility::compute(app);
        let CustomInferenceVisibility {
            is_any_ai_enabled,
            is_byo_enabled,
            show_provider_keys,
            provider_keys_enabled,
            show_custom_inference,
            custom_inference_controls_enabled,
            managed_byok_byoe_enabled,
        } = visibility;

        let mut column = Flex::column().with_child(render_separator(appearance));

        if visibility.show_section() {
            // Header row: "Custom Inference" + info icon on left, "+ Add custom model" on right
            let header_left = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    build_sub_header(
                        appearance,
                        "Custom Inference",
                        Some(styles::header_font_color(visibility.section_enabled(), app)),
                    )
                    .with_margin_bottom(0.)
                    .finish(),
                )
                .with_child(
                    self.render_custom_inference_info_icon(appearance, managed_byok_byoe_enabled),
                )
                .finish();

            let header_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(header_left);
            let header_row = if show_custom_inference {
                header_row.with_child(view.custom_inference_add_button.as_ref(app).render(app))
            } else {
                header_row
            }
            .finish();

            column.add_child(
                Container::new(header_row)
                    .with_padding_bottom(HEADER_PADDING)
                    .finish(),
            );

            // Description with Learn more link
            column.add_child(self.render_custom_inference_description(
                show_provider_keys,
                show_custom_inference,
                app,
            ));
        } else if managed_byok_byoe_enabled {
            column.add_child(
                build_sub_header(
                    appearance,
                    "Custom Inference",
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            );
            column.add_child(render_ai_setting_description(
                "Your organization manages custom inference. Personal API keys and custom endpoints are currently disabled.",
                is_any_ai_enabled,
                app,
            ));
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

        if show_provider_keys {
            column.add_child(self.render_provider_key_editors(
                appearance,
                provider_keys_enabled,
                app,
            ));
        }

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
                let endpoints_list = self.render_custom_endpoints_list(
                    view,
                    appearance,
                    custom_inference_controls_enabled,
                    app,
                );
                // When the provider-key rows are hidden, this list is the
                // section's last child, so pad it from the next separator.
                let endpoints_list = if show_provider_keys {
                    endpoints_list
                } else {
                    Container::new(endpoints_list)
                        .with_margin_bottom(16.)
                        .finish()
                };
                column.add_child(endpoints_list);
            }
        }

        // Entrypoint for connecting a SuperGrok (xAI) subscription via OAuth.
        if FeatureFlag::SuperGrok.is_enabled() && show_provider_keys {
            #[cfg(not(target_family = "wasm"))]
            let grok_tokens = ApiKeyManager::as_ref(app).grok_tokens();
            #[cfg(not(target_family = "wasm"))]
            let has_grok_oauth_attempt = view.grok_oauth_attempt.is_some();
            #[cfg(not(target_family = "wasm"))]
            let is_grok_connecting = grok_tokens.is_none() && has_grok_oauth_attempt;
            #[cfg(target_family = "wasm")]
            let is_grok_connecting = false;
            column.add_child(
                Container::new(self.render_grok_subscription_row(
                    appearance,
                    provider_keys_enabled,
                    is_grok_connecting,
                    app,
                ))
                .with_margin_top(16.)
                .finish(),
            );

            #[cfg(not(target_family = "wasm"))]
            if has_grok_oauth_attempt {
                column.add_child(
                    Container::new(self.render_grok_manual_code_entry(view, appearance))
                        .with_margin_top(8.)
                        .finish(),
                );
            }
        }

        // Warp credit fallback applies to member-provided API keys, not custom endpoints.
        if is_byo_enabled && show_provider_keys {
            column.add_child(
                Container::new(self.render_warp_credit_fallback_toggle(view, app))
                    .with_margin_top(16.)
                    .finish(),
            );
        }

        // Upgrade CTA if BYOK not enabled
        if !is_byo_enabled && show_provider_keys {
            let auth_state = AuthStateProvider::as_ref(app).get();
            let upgrade_text_fragments = if let Some(team) =
                UserWorkspaces::as_ref(app).team_for_view_handle(&self.view_handle, app)
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
                                "Upgrade to the Build plan",
                                upgrade_url,
                            ),
                            FormattedTextFragment::plain_text(" to use your own API keys."),
                        ]
                    } else {
                        vec![FormattedTextFragment::plain_text(
                            "Ask your team's admin to upgrade to the Build plan to use your own API keys.",
                        )]
                    }
                }
            } else if FeatureFlag::SoloUserByok.is_enabled()
                && auth_state.is_anonymous_or_logged_out()
            {
                vec![
                    FormattedTextFragment::hyperlink_action(
                        "Create an account",
                        WarpAgentPageAction::SignupAnonymousUser,
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
                            action_ref.as_any().downcast_ref::<WarpAgentPageAction>()
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
        update_editor_interaction_state(
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
        update_editor_interaction_state(
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

        let refresh_credentials_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Refresh", SecondaryTheme)
                .with_icon(Icon::RefreshCw04)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(WarpAgentPageAction::RefreshAwsBedrockCredentials);
                })
        });
        refresh_credentials_button.update(ctx, |button, ctx| {
            button.set_disabled(!is_usage_enabled, ctx);
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

                update_editor_interaction_state(
                    aws_auth_refresh_command_editor_clone.clone(),
                    is_usage_enabled,
                    ctx,
                );
                update_editor_interaction_state(
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

                    update_editor_interaction_state(
                        aws_auth_refresh_command_editor_clone.clone(),
                        is_usage_enabled,
                        ctx,
                    );
                    update_editor_interaction_state(
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
            "Warp loads and sends local AWS CLI credentials for Bedrock-supported models. This setting is managed by your organization.".to_string()
        } else {
            "Warp loads and sends local AWS CLI credentials for Bedrock-supported models."
                .to_string()
        };

        let mut column = Flex::column().with_spacing(16.).with_child(
            Flex::column()
                .with_child(render_ai_setting_toggle::<AwsBedrockCredentialsEnabled>(
                    "Use AWS Bedrock credentials",
                    WarpAgentPageAction::ToggleAwsBedrockCredentialsEnabled,
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
            "Login Command",
            self.aws_auth_refresh_command_editor.clone(),
            is_usage_enabled,
            app,
        ));
        column.add_child(render_input(
            appearance,
            "AWS Profile",
            self.aws_auth_refresh_profile_editor.clone(),
            is_usage_enabled,
            app,
        ));

        let auto_login_enabled = *AISettings::as_ref(app).aws_bedrock_auto_login.value();

        let toggle = render_ai_setting_toggle::<AwsBedrockAutoLogin>(
            "Automatically run login command",
            WarpAgentPageAction::ToggleAwsBedrockAutoLogin,
            auto_login_enabled,
            is_usage_enabled,
            self.auto_login_toggle.clone(),
            &RefCell::new(HashMap::new()),
            app,
        );
        let description = render_ai_setting_description(
            "When enabled, the login command will run automatically when AWS Bedrock credentials expire.",
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
    type View = WarpAgentPageView;

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
                    "AWS Bedrock",
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

struct GeminiEnterpriseWidget {
    credentials_enabled_toggle: SwitchStateHandle,
    refresh_credentials_button: ViewHandle<ActionButton>,
}

impl GeminiEnterpriseWidget {
    fn is_refresh_enabled(app: &AppContext) -> bool {
        AISettings::as_ref(app).is_any_ai_enabled(app)
            && UserWorkspaces::as_ref(app).is_gemini_enterprise_credentials_enabled(app)
            && !ApiKeyManager::as_ref(app)
                .geap_credentials_state()
                .requires_admin_action()
    }

    fn new(ctx: &mut ViewContext<<Self as SettingsWidget>::View>) -> Self {
        let refresh_credentials_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Refresh", SecondaryTheme)
                .with_icon(Icon::RefreshCw04)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(
                        WarpAgentPageAction::RefreshGeminiEnterpriseCredentials,
                    );
                })
        });
        refresh_credentials_button.update(ctx, |button, ctx| {
            button.set_disabled(!Self::is_refresh_enabled(ctx), ctx);
        });

        let refresh_credentials_button_clone = refresh_credentials_button.clone();
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), move |_, _, event, ctx| {
            if matches!(
                event,
                UserWorkspacesEvent::TeamsChanged
                    | UserWorkspacesEvent::UpdateWorkspaceSettingsSuccess
            ) {
                refresh_credentials_button_clone.update(ctx, |button, ctx| {
                    button.set_disabled(!Self::is_refresh_enabled(ctx), ctx);
                });
                ctx.notify();
            }
        });

        let refresh_credentials_button_clone = refresh_credentials_button.clone();
        ctx.subscribe_to_model(&AISettings::handle(ctx), move |_, _, event, ctx| {
            if matches!(
                event,
                AISettingsChangedEvent::GeminiEnterpriseCredentialsEnabled { .. }
                    | AISettingsChangedEvent::IsAnyAIEnabled { .. }
            ) {
                refresh_credentials_button_clone.update(ctx, |button, ctx| {
                    button.set_disabled(!Self::is_refresh_enabled(ctx), ctx);
                });
                ctx.notify();
            }
        });

        let refresh_credentials_button_clone = refresh_credentials_button.clone();
        ctx.subscribe_to_model(&ApiKeyManager::handle(ctx), move |_, _, event, ctx| {
            if matches!(event, ApiKeyManagerEvent::KeysUpdated) {
                refresh_credentials_button_clone.update(ctx, |button, ctx| {
                    button.set_disabled(!Self::is_refresh_enabled(ctx), ctx);
                });
            }
        });

        Self {
            credentials_enabled_toggle: SwitchStateHandle::default(),
            refresh_credentials_button,
        }
    }

    fn render_gemini_enterprise_section(
        &self,
        appearance: &Appearance,
        app: &AppContext,
        is_gemini_enterprise_available: bool,
    ) -> Box<dyn Element> {
        let user_workspaces = UserWorkspaces::as_ref(app);
        let is_any_ai_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);
        let is_section_enabled = is_any_ai_enabled && is_gemini_enterprise_available;
        let is_admin_enforced = matches!(
            user_workspaces.gemini_enterprise_host_enablement_setting(),
            crate::workspaces::workspace::HostEnablementSetting::Enforce
        );
        let is_toggleable =
            is_section_enabled && user_workspaces.is_gemini_enterprise_credentials_toggleable();
        let are_credentials_enabled = user_workspaces.is_gemini_enterprise_credentials_enabled(app);
        let toggle_description = if is_admin_enforced {
            "Warp routes eligible requests through your workspace's Gemini Enterprise Google Cloud \
             project. This setting is managed by your organization."
                .to_string()
        } else {
            "Warp routes eligible requests through your workspace's Gemini Enterprise Google Cloud \
             project."
                .to_string()
        };

        let mut column = Flex::column().with_spacing(16.).with_child(
            Flex::column()
                .with_child(
                    render_ai_setting_toggle::<GeminiEnterpriseCredentialsEnabled>(
                        "Use Gemini Enterprise credentials",
                        WarpAgentPageAction::ToggleGeminiEnterpriseCredentialsEnabled,
                        are_credentials_enabled,
                        is_toggleable,
                        self.credentials_enabled_toggle.clone(),
                        &RefCell::new(HashMap::new()),
                        app,
                    ),
                )
                .with_child(render_ai_setting_description(
                    toggle_description,
                    is_section_enabled,
                    app,
                ))
                .finish(),
        );

        column.add_child(
            Container::new(self.render_credential_status_card(
                appearance,
                are_credentials_enabled,
                app,
            ))
            .with_margin_top(-styles::DESCRIPTION_MARGIN_BOTTOM)
            .finish(),
        );

        column.finish()
    }

    fn render_credential_status_card(
        &self,
        appearance: &Appearance,
        are_credentials_enabled: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let manager = ApiKeyManager::as_ref(app);
        let (title_text, detail_text, icon) =
            manager.geap_credentials_state().user_facing_components();

        let (title_color, detail_color) = (
            styles::header_font_color(are_credentials_enabled, app),
            styles::description_font_color(are_credentials_enabled, app),
        );

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

        let row = Flex::row()
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
            .with_child(ChildView::new(&self.refresh_credentials_button).finish());

        Container::new(row.finish())
            .with_uniform_padding(12.)
            .with_background(appearance.theme().surface_2())
            .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .finish()
    }
}

impl SettingsWidget for GeminiEnterpriseWidget {
    type View = WarpAgentPageView;

    fn search_terms(&self) -> &str {
        "gemini enterprise geap google vertex credentials"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        FeatureFlag::GeminiEnterprise.is_enabled()
            && UserWorkspaces::as_ref(app).is_gemini_enterprise_available_from_workspace()
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let is_any_ai_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);
        let is_gemini_enterprise_available =
            UserWorkspaces::as_ref(app).is_gemini_enterprise_available_from_workspace();
        let column = Flex::column()
            .with_child(render_separator(appearance))
            .with_child(
                build_sub_header(
                    appearance,
                    "Gemini Enterprise",
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            )
            .with_child(self.render_gemini_enterprise_section(
                appearance,
                app,
                is_gemini_enterprise_available,
            ));

        Container::new(column.finish())
            .with_margin_bottom(HEADER_PADDING)
            .finish()
    }
}

/// Stable `&'static str` id for the custom model routers settings widget,
/// exposed for the `warp://settings?widget=custom_router` deeplink (see
/// `settings_widget_deeplink_target`).
pub(crate) fn custom_model_routers_widget_id() -> &'static str {
    CustomModelRoutersWidget::static_widget_id()
}

#[derive(Default)]
struct CustomModelRoutersWidget;

impl SettingsWidget for CustomModelRoutersWidget {
    type View = WarpAgentPageView;

    fn search_terms(&self) -> &str {
        "custom model router complexity prompt auto model routing"
    }

    fn should_render(&self, _app: &AppContext) -> bool {
        FeatureFlag::CustomModelRouters.is_enabled()
    }

    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let is_any_ai_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);
        let header_color = styles::header_font_color(is_any_ai_enabled, app);

        // Header row: "Custom Model Routers" + add button
        let header_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(build_sub_header(appearance, "Custom Routers", Some(header_color)).finish())
            .with_child({
                #[cfg(feature = "local_fs")]
                {
                    warpui::elements::Container::new(view.add_router_button.as_ref(app).render(app))
                        .with_margin_bottom(4.)
                        .with_margin_top(-4.)
                        .finish()
                }
                #[cfg(not(feature = "local_fs"))]
                {
                    warpui::elements::Empty::new().finish()
                }
            })
            .finish();

        let column = Flex::column()
            .with_child(render_separator(appearance))
            .with_child(
                Container::new(header_row)
                    .with_padding_bottom(HEADER_PADDING)
                    .finish(),
            )
            .with_child(render_ai_setting_description(
                "Automatically route tasks to specific models based on task complexity or custom rules. Custom routers will appear in your model selector menu.",
                is_any_ai_enabled,
                app,
            ));

        // Error cards and router summary cards (local_fs only)
        #[cfg(feature = "local_fs")]
        let column = {
            use super::custom_router_view::render_router_error_card;
            use crate::user_config::WarpConfig;
            let mut c = column;
            // Error cards (files that failed to parse) — shown first
            let errors = WarpConfig::as_ref(app).custom_model_router_errors();
            for error in errors.iter() {
                c.add_child(
                    Container::new(render_router_error_card(
                        &error.file_name,
                        &error.error_message,
                        appearance,
                    ))
                    .with_margin_top(8.)
                    .finish(),
                );
            }
            // Router summary cards
            for view_handle in &view.router_views {
                c.add_child(
                    Container::new(warpui::elements::ChildView::new(view_handle).finish())
                        .with_margin_top(8.)
                        .finish(),
                );
            }
            c
        };

        // Add trailing space beneath this section (matching sibling sections
        // like AWS Bedrock) so the following section's title isn't crowded
        // against the router cards.
        Container::new(column.finish())
            .with_margin_bottom(HEADER_PADDING)
            .finish()
    }
}
