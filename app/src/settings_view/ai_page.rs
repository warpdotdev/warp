use crate::ai::aws_credentials::refresh_aws_credentials;
use crate::ai::blocklist::agent_view::agent_input_footer::editor::{
    AgentToolbarEditorMode, AgentToolbarInlineEditor,
};
use crate::ai::blocklist::BlocklistAIPermissions;
use crate::ai::execution_profiles::model_menu_items::available_model_menu_items;
use crate::ai::execution_profiles::profiles::{
    AIExecutionProfilesModel, AIExecutionProfilesModelEvent, ClientProfileId,
};
use crate::ai::execution_profiles::{ActionPermission, WriteToPtyPermission};
use crate::ai::llms::{LLMId, LLMPreferences, LLMPreferencesEvent};
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::ai::paths::host_native_absolute_path;
use crate::editor::{EditorOptions, InteractionState, SingleLineEditorOptions, TextColors};
use crate::settings::InputSettings;
use crate::settings::{
    AIAutoDetectionEnabled, AICommandDenylist, AISettingsChangedEvent,
    AgentModeCodingPermissionsType, AgentModeCommandExecutionDenylist,
    AgentModeCommandExecutionPredicate, AgentModeQuerySuggestionsEnabled, AwsBedrockAutoLogin,
    AwsBedrockCredentialsEnabled, CodeSettings, CodebaseContextEnabled, FileBasedMcpEnabled,
    GitOperationsAutogenEnabled, IncludeAgentCommandsInHistory, IntelligentAutosuggestionsEnabled,
    MemoryEnabled, NLDInTerminalEnabled, NaturalLanguageAutosuggestionsEnabled,
    RuleSuggestionsEnabled, ShouldRenderCLIAgentToolbar,
    ShouldRenderUseAgentToolbarForUserCommands, ShowAgentTips, ShowConversationHistory,
    ShowHintText, ThinkingDisplayMode, VoiceInputEnabled,
};
use crate::terminal::session_settings::{SessionSettings, SessionSettingsChangedEvent};
use crate::terminal::CLIAgent;
use crate::util::links;
use crate::view_components::{
    action_button::{ActionButton, ButtonSize, SecondaryTheme},
    FilterableDropdown, SubmittableTextInput, SubmittableTextInputEvent,
};
use ::ai::api_keys::{ApiKeyManager, ApiKeys};
use enum_iterator::all;
use itertools::Itertools;
use regex::Regex;
use settings::{Setting, ToggleableSetting};
use strum::IntoEnumIterator;
use warp_core::channel::{Channel, ChannelState};
use warp_core::context_flag::ContextFlag;
use warp_core::features::FeatureFlag;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Border, ChildView, ConstrainedBox, CornerRadius, CrossAxisAlignment, Expanded, Fill,
    HyperlinkLens, MainAxisAlignment, MainAxisSize, MouseStateHandle, Radius, Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::id;
use warpui::keymap::ContextPredicate;
use warpui::{
    elements::{
        Container, Flex, FormattedTextElement, HighlightedHyperlink, HyperlinkUrl, ParentElement,
    },
    ui_components::{
        components::{Coords, UiComponent, UiComponentStyles},
        switch::SwitchStateHandle,
    },
    Action, AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::execution_profile_view::{ExecutionProfileView, ExecutionProfileViewEvent};
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
use crate::menu::{MenuItem, MenuItemFields};
use crate::ui_components::icons::Icon;
use crate::view_components::dropdown::DropdownAction;
use crate::UserWorkspaces;
use crate::{
    appearance::Appearance,
    editor::Event as EditorEvent,
    editor::{EditorView, TextOptions},
    settings::{AISettings, VoiceInputToggleKey},
    util::bindings,
    view_components::{Dropdown, DropdownItem},
};
use crate::{report_error, report_if_error};
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Not;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const CONTENT_FONT_SIZE: f32 = 12.;
const PRIMARY_HEADER_FONT_SIZE: f32 = 24.;

const AI_SETTINGS_DROPDOWN_WIDTH: f32 = 250.;
const AI_SETTINGS_DROPDOWN_MAX_HEIGHT: f32 = 250.;
const NEXT_COMMAND_DESCRIPTION: &str = "Let AI suggest the next command to run based on your command history, outputs, and common workflows.";
const PROMPT_SUGGESTIONS_DESCRIPTION: &str = "Let AI suggest natural language prompts, as inline banners in the input, based on recent commands and their outputs.";
const SUGGESTED_CODE_BANNERS_DESCRIPTION: &str = "Let AI suggest code diffs and queries as inline banners in the blocklist, based on recent commands and their outputs.";
const NATURAL_LANGUAGE_AUTOSUGGESTIONS: &str =
    "Let AI suggest natural language autosuggestions, based on recent commands and their outputs.";
const GIT_OPERATIONS_AUTOGEN_DESCRIPTION: &str =
    "Let AI generate commit messages and pull request titles and descriptions.";
const WISPR_FLOW_URL: &str = "https://wisprflow.ai/";

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
        .with_enabled(|| false)],
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
    command_denylist_editor: ViewHandle<SubmittableTextInput>,

    mcp_allowlist_mouse_state_handles: Vec<MouseStateHandle>,
    mcp_allowlist_dropdown: ViewHandle<FilterableDropdown<AISettingsPageAction>>,

    mcp_denylist_mouse_state_handles: Vec<MouseStateHandle>,
    mcp_denylist_dropdown: ViewHandle<FilterableDropdown<AISettingsPageAction>>,

    base_model_dropdown: ViewHandle<Dropdown<AISettingsPageAction>>,
    coding_model_dropdown: ViewHandle<Dropdown<AISettingsPageAction>>,

    thinking_display_mode_dropdown: ViewHandle<Dropdown<AISettingsPageAction>>,
    #[cfg(feature = "local_fs")]
    conversation_layout_dropdown: ViewHandle<Dropdown<AISettingsPageAction>>,

    // Profile views
    profile_views: Vec<ViewHandle<ExecutionProfileView>>,
    add_profile_button: ViewHandle<ActionButton>,
}

impl AISettingsPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let is_any_ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);

        let ai_autonomy_settings = UserWorkspaces::as_ref(ctx).ai_autonomy_settings();

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
                            AISettingsPageAction::SetVoiceInputToggleKey(val),
                        )
                    })
                    .collect(),
                ctx,
            );
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
            input.set_placeholder_text("e.g. ~/code-repos/repo", ctx);
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

            editor.set_placeholder_text("Commands, comma separated", ctx);

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
            input.set_placeholder_text("e.g. ls .*", ctx);
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
            input.set_placeholder_text("e.g. rm .*", ctx);
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
            input.set_placeholder_text("command (supports regex)", ctx);
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
                    }
                    AIExecutionProfilesModelEvent::UpdatedActiveProfile { .. } => (),
                }
                ctx.notify();
            },
        );

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
                }
                LLMPreferencesEvent::UpdatedActiveAgentModeLLM => {
                    Self::refresh_base_model_menu(&me.base_model_dropdown, ctx);
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
                        is_enabled
                            && !ai_autonomy_settings.has_override_for_execute_commands_denylist(),
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
                    let current_value = AISettings::as_ref(ctx)
                        .voice_input_toggle_key
                        .value()
                        .display_name();
                    me.voice_input_toggle_key_dropdown
                        .update(ctx, |dropdown, ctx| {
                            dropdown.set_selected_by_name(current_value, ctx)
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
                vec![
                    DropdownItem::new(
                        "Agent decides",
                        AISettingsPageAction::SetApplyCodeDiffs(ActionPermission::AgentDecides),
                    ),
                    DropdownItem::new(
                        "Always allow",
                        AISettingsPageAction::SetApplyCodeDiffs(ActionPermission::AlwaysAllow),
                    ),
                    DropdownItem::new(
                        "Always ask",
                        AISettingsPageAction::SetApplyCodeDiffs(ActionPermission::AlwaysAsk),
                    ),
                ],
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
                vec![
                    DropdownItem::new(
                        "Agent decides",
                        AISettingsPageAction::SetReadFiles(ActionPermission::AgentDecides),
                    ),
                    DropdownItem::new(
                        "Always allow",
                        AISettingsPageAction::SetReadFiles(ActionPermission::AlwaysAllow),
                    ),
                    DropdownItem::new(
                        "Always ask",
                        AISettingsPageAction::SetReadFiles(ActionPermission::AlwaysAsk),
                    ),
                ],
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
                vec![
                    DropdownItem::new(
                        "Agent decides",
                        AISettingsPageAction::SetExecuteCommands(ActionPermission::AgentDecides),
                    ),
                    DropdownItem::new(
                        "Always allow",
                        AISettingsPageAction::SetExecuteCommands(ActionPermission::AlwaysAllow),
                    ),
                    DropdownItem::new(
                        "Always ask",
                        AISettingsPageAction::SetExecuteCommands(ActionPermission::AlwaysAsk),
                    ),
                ],
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
            dropdown.set_items(
                vec![
                    DropdownItem::new(
                        "Always allow",
                        AISettingsPageAction::SetWriteToPty(WriteToPtyPermission::AlwaysAllow),
                    ),
                    DropdownItem::new(
                        "Always ask",
                        AISettingsPageAction::SetWriteToPty(WriteToPtyPermission::AlwaysAsk),
                    ),
                    DropdownItem::new(
                        "Ask on first write",
                        AISettingsPageAction::SetWriteToPty(WriteToPtyPermission::AskOnFirstWrite),
                    ),
                ],
                ctx,
            );
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
                vec![
                    DropdownItem::new(
                        "Agent decides",
                        AISettingsPageAction::SetMCPPermissions(ActionPermission::AgentDecides),
                    ),
                    DropdownItem::new(
                        "Always allow",
                        AISettingsPageAction::SetMCPPermissions(ActionPermission::AlwaysAllow),
                    ),
                    DropdownItem::new(
                        "Always ask",
                        AISettingsPageAction::SetMCPPermissions(ActionPermission::AlwaysAsk),
                    ),
                ],
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
            dropdown.set_menu_header_to_static("Select MCP servers");
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
            dropdown.set_menu_header_to_static("Select MCP servers");
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
            input.set_placeholder_text("e.g. ~/code-repos/repo", ctx);
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

        let command_denylist_mouse_state_handles = current_permission
            .command_denylist
            .iter()
            .map(|_| Default::default())
            .collect();

        let command_denylist_editor = ctx.add_typed_action_view(|ctx| {
            let mut input =
                SubmittableTextInput::new(ctx).validate_on_edit(|s| Regex::new(s).is_ok());
            input.set_placeholder_text("e.g. rm .*", ctx);
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
            input.set_placeholder_text("e.g. ls .*", ctx);
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

        let profile_views = Self::create_profile_views(ctx);

        let add_profile_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Add Profile", SecondaryTheme)
                .with_icon(Icon::Plus)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AISettingsPageAction::CreateProfile);
                })
        });

        add_profile_button.update(ctx, |button, ctx| {
            button.set_disabled(!is_any_ai_enabled, ctx);
        });
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
                    "New Tab",
                    AISettingsPageAction::SetConversationLayout(OpenConversationPreference::NewTab),
                ),
                DropdownItem::new(
                    "Split Pane",
                    AISettingsPageAction::SetConversationLayout(
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
            cli_agent_footer_command_editor,
            cli_agent_footer_command_mouse_state_handles,
            cli_agent_footer_command_agent_dropdowns: Self::create_cli_agent_dropdowns(ctx),
            agent_toolbar_inline_editor,
            cli_agent_toolbar_inline_editor,
            base_model_dropdown,
            coding_model_dropdown,
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
        let is_oss = ChannelState::channel() == Channel::Oss;

        let mut widgets: Vec<Box<dyn SettingsWidget<View = AISettingsPageView>>> = Vec::new();

        // When viewing a specific subpage, only include its widgets.
        // When subpage is None (legacy/backward-compat), show all widgets.
        match subpage {
            None => {
                // Full page: all widgets (legacy behavior)
                widgets.push(Box::new(GlobalAIWidget::default()));
                if ai_settings
                    .intelligent_autosuggestions_enabled_internal
                    .is_supported_on_current_platform()
                    || ai_settings
                        .prompt_suggestions_enabled_internal
                        .is_supported_on_current_platform()
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
                widgets.push(Box::new(CLIAgentWidget::default()));
                widgets.push(Box::new(ApiKeysWidget::new(ctx)));
                if !is_oss {
                    widgets.push(Box::new(AwsBedrockWidget::new(ctx)));
                }
                widgets.push(Box::new(OtherAIWidget::default()));
            }
            Some(AISubpage::WarpAgent) => {
                // Agent page: global toggle + Active AI + Input + Other
                widgets.push(Box::new(GlobalAIWidget::default()));
                if ai_settings
                    .intelligent_autosuggestions_enabled_internal
                    .is_supported_on_current_platform()
                    || ai_settings
                        .prompt_suggestions_enabled_internal
                        .is_supported_on_current_platform()
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
                widgets.push(Box::new(ApiKeysWidget::new(ctx)));
                if !is_oss {
                    widgets.push(Box::new(AwsBedrockWidget::new(ctx)));
                }
                widgets.push(Box::new(OtherAIWidget::default()));
            }
            Some(AISubpage::Profiles) => {
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
                .get_base_llm_choices_for_agent_mode()
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
                .get_coding_llm_choices()
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
                        "Read only",
                        AISettingsPageAction::SetAutonomyReadonlyCommandsSetting,
                    ),
                    DropdownItem::new(
                        "Supervised",
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
                            AgentModeCodingPermissionsType::AlwaysAskBeforeReading => "Always ask",
                            AgentModeCodingPermissionsType::AlwaysAllowReading => "Always allow",
                            AgentModeCodingPermissionsType::AllowReadingSpecificFiles => {
                                "Allow in specific directories"
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
        let all_mcp_servers = TemplatableMCPServerManager::get_all_runnable_mcp_servers(ctx);
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

                    items.push(
                        MenuItemFields::new("Other")
                            .with_on_select_action(DropdownAction::SelectActionAndClose(
                                AISettingsPageAction::SetCLIAgentForCommand {
                                    pattern: pattern_clone.clone(),
                                    agent: None,
                                },
                            ))
                            .into_item(),
                    );

                    dropdown.set_rich_items(items, ctx);

                    dropdown.set_menu_header_text_override(|label| {
                        if label == "Other" {
                            "Select coding agent".to_string()
                        } else {
                            label.to_string()
                        }
                    });

                    let selected_name = if matches!(current_agent, CLIAgent::Unknown) {
                        "Other"
                    } else {
                        current_agent.display_name()
                    };
                    dropdown.set_selected_by_name(selected_name, ctx);

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
    ToggleGitOperationsAutogen,
    ToggleAIInputAutoDetection,
    ToggleNLDInTerminal,
    ToggleCLIAgentToolbar,
    ToggleUseAgentToolbar,
    ToggleVoiceInput,
    HyperlinkClick(HyperlinkUrl),
    ToggleCodebaseContext,
    ToggleShowInputHintText,
    ToggleShowAgentTips,
    SetThinkingDisplayMode(ThinkingDisplayMode),
    RemoveCLIAgentToolbarEnabledCommand(String),
    RemoveFromCommandExecutionAllowlist(AgentModeCommandExecutionPredicate),
    RemoveFromCommandExecutionDenylist(AgentModeCommandExecutionPredicate),
    OpenAIFactCollection,
    OpenMCPServerCollection,
    OpenExecutionProfileEditor(ClientProfileId),
    SetBaseModel(LLMId),
    SetCodingModel(LLMId),
    SetAutonomyReadonlyCommandsSetting,
    SetAutonomySupervisedSetting,
    SetCodingPermission(AgentModeCodingPermissionsType),
    RemoveDirectoryFromCodeReadAllowlist(PathBuf),
    ToggleRules,
    ToggleRuleSuggestions,
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
    ToggleAwsBedrockAutoLogin,
    ToggleAwsBedrockCredentialsEnabled,
    RefreshAwsBedrockCredentials,
    ToggleFileBasedMcp,
    ToggleIncludeAgentCommandsInHistory,
    #[cfg(feature = "local_fs")]
    SetConversationLayout(crate::util::file::external_editor::settings::OpenConversationPreference),
    ToggleShowConversationHistory,
    ToggleAutoToggleRichInput,
    ToggleAutoOpenRichInputOnCLIAgentStart,
    ToggleAutoDismissRichInputAfterSubmit,
    SetCLIAgentForCommand {
        pattern: String,
        agent: Option<CLIAgent>,
    },
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
                    Ok(_) => {}
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
                    Ok(_) => {}
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
                    Ok(_) => {}
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
                    Ok(_) => {}
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
                    Ok(_) => {}
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
                    Ok(_) => {}
                    Err(e) => {
                        log::warn!(
                            "Failed to set value for Natural Language Autosuggestions setting: {e:?}"
                        );
                    }
                }
                ctx.notify();
            }
            AISettingsPageAction::ToggleGitOperationsAutogen => {
                match AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .git_operations_autogen_enabled_internal
                        .toggle_and_save_value(ctx)
                }) {
                    Ok(_) => {}
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
                    Ok(_) => {}
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
                    Ok(_) => {}
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
                    Ok(_) => {}
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
                    Ok(_) => {}
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
                    Ok(_) => {}
                    Err(e) => {
                        log::warn!("Failed to set value for Voice Input: {e:?}");
                    }
                }
                ctx.notify();
            }
            AISettingsPageAction::HyperlinkClick(hyperlink) => {
                ctx.notify();
                ctx.open_url(&hyperlink.url);
            }
            AISettingsPageAction::ToggleShowInputHintText => {
                InputSettings::handle(ctx).update(ctx, |input_settings, ctx| {
                    report_if_error!(input_settings.show_hint_text.toggle_and_save_value(ctx));
                });
            }
            AISettingsPageAction::ToggleShowAgentTips => {
                InputSettings::handle(ctx).update(ctx, |input_settings, ctx| match input_settings
                    .show_agent_tips
                    .toggle_and_save_value(ctx)
                {
                    Ok(_) => {}
                    Err(e) => {
                        log::warn!("Failed to set value for Show Agent Tips setting: {e:?}");
                    }
                });
                ctx.notify();
            }
            AISettingsPageAction::SetThinkingDisplayMode(mode) => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.thinking_display_mode.set_value(*mode, ctx));
                });
                ctx.notify();
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
                    let profile = profiles_model.default_profile(ctx);
                    profiles_model.set_base_model(*profile.id(), Some(id.clone()), ctx);
                });
                ctx.notify();
            }
            AISettingsPageAction::SetCodingModel(id) => {
                LLMPreferences::handle(ctx).update(ctx, |prefs, ctx| {
                    prefs.update_preferred_coding_llm(id, None, ctx);
                });
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
                        Ok(_) => {}
                        Err(e) => report_error!(e),
                    }
                });
            }
            AISettingsPageAction::SetCodingPermission(p) => {
                BlocklistAIPermissions::handle(ctx).update(ctx, |model, ctx| {
                    match model.set_coding_permissions(*p, ctx) {
                        Ok(_) => {}
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
                ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                    drop(refresh_aws_credentials(manager, ctx));
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
) -> Box<dyn Element> {
    let label = Container::new(
        appearance
            .ui_builder()
            .span("Toolbar layout".to_string())
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
}

impl SettingsWidget for GlobalAIWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "agent global ai a.i. active next command prompt code diffs suggestion suggested suggestions \
                agent mode natural language detection input hint api keys bring your own byo google anthropic openai"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let is_ai_disabled_due_to_remote_session_local_policy =
            AISettings::as_ref(app).is_ai_disabled_due_to_remote_session_local_policy(app);

        let agent_title = "Warper Agent";

        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Text::new_inline(
                    agent_title,
                    appearance.ui_font_family(),
                    PRIMARY_HEADER_FONT_SIZE,
                )
                .with_style(Properties::default().weight(Weight::Bold))
                .with_color(appearance.theme().active_ui_text_color().into())
                .finish(),
            );

        if is_ai_disabled_due_to_remote_session_local_policy {
            row.add_child(
                ConstrainedBox::new(
                    Container::new(
                        Text::new("Local policy disallows AI when the active pane contains content from a remote session", appearance.ui_font_family(), 12.)
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

        Container::new(row.finish())
            .with_padding_bottom(15.)
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
        let _ = app;
        false
    }

    fn is_git_operations_autogen_toggleable(&self, app: &AppContext) -> bool {
        FeatureFlag::GitOperationsInCodeReview.is_enabled()
            && AISettings::as_ref(app)
                .git_operations_autogen_enabled_internal
                .is_supported_on_current_platform()
            && UserWorkspaces::as_ref(app).ai_allowed_by_local_policy()
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
                    "Next Command",
                    AISettingsPageAction::ToggleIntelligentAutosuggestions,
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
        view: &AISettingsPageView,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_active_ai_enabled(app);
        Flex::column()
            .with_child(
                render_ai_setting_toggle::<AgentModeQuerySuggestionsEnabled>(
                    "Prompt Suggestions",
                    AISettingsPageAction::TogglePromptSuggestions,
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
        view: &AISettingsPageView,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_active_ai_enabled(app);
        Flex::column()
            .with_child(
                render_ai_setting_toggle::<AgentModeQuerySuggestionsEnabled>(
                    "Suggested Code Banners",
                    AISettingsPageAction::ToggleCodeSuggestions,
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
        view: &AISettingsPageView,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_active_ai_enabled(app);
        Flex::column()
            .with_child(render_ai_setting_toggle::<
                NaturalLanguageAutosuggestionsEnabled,
            >(
                "Natural Language Autosuggestions",
                AISettingsPageAction::ToggleNaturalLanguageAutosuggestions,
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

    fn render_git_operations_autogen_section(
        &self,
        view: &AISettingsPageView,
        app: &warpui::AppContext,
    ) -> Box<dyn warpui::Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_toggleable = ai_settings.is_active_ai_enabled(app);
        Flex::column()
            .with_child(render_ai_setting_toggle::<GitOperationsAutogenEnabled>(
                "Commit & Pull Request Generation",
                AISettingsPageAction::ToggleGitOperationsAutogen,
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
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "active ai a.i. next command prompt suggestions code diffs suggested banners passive unit tests commit pull request pr git code review autogen generate"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        self.is_next_command_toggleable(app)
            || self.is_prompt_suggestions_toggleable(app)
            || self.is_suggested_code_banners_toggleable(app)
            || self.is_natural_language_autosuggestions_toggleable(app)
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
                    "Agents",
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            );
            agents_header.add_child(render_ai_setting_description(
                "Set the boundaries for how your Agent operates. Choose what it can access, how much autonomy it has, and when it must ask for your approval. You can also fine-tune behavior around natural language input, codebase awareness, and more.",
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
                    "Profiles",
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .finish(),
            )
            .with_child(
                Container::new(
                    render_ai_setting_description(
                        "Profiles let you define how your Agent operates — from the actions it can take and when it needs approval, to the models it uses for tasks like coding and planning. You can also scope them to individual projects.",
                        is_any_ai_enabled,
                        app,
                    )
                )
                .with_margin_top(12.)
                .finish()
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
            "Models",
            14.0,
            Some(styles::header_font_color(is_any_ai_enabled, app)),
        ))
        .with_margin_bottom(8.0)
        .finish();

        let base_model_setting =
            Container::new(self.render_base_model_setting(view, ai_settings, appearance, app))
                .with_margin_bottom(8.0)
                .finish();

        Flex::column()
            .with_children([model_subheader, base_model_setting])
            .finish()
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
            "Permissions",
            14.0,
            Some(styles::header_font_color(is_any_ai_enabled, app)),
        ))
        .with_margin_bottom(4.0)
        .finish();

        let code_diff_setting =
            BlocklistAIPermissions::as_ref(app).get_apply_code_diffs_setting(app, None);
        let code_diffs = self.render_execution_profile_dropdown(
            "Apply code diffs",
            Icon::Code2,
            code_diff_setting.description(),
            &view.apply_code_diffs_dropdown_menu,
            ai_settings,
            appearance,
            app,
        );

        let read_files_setting =
            BlocklistAIPermissions::as_ref(app).get_read_files_setting(app, None);
        let mut read_files_flex = Flex::column().with_main_axis_size(MainAxisSize::Min);
        read_files_flex.add_child(self.render_execution_profile_dropdown(
            "Read files",
            Icon::Notebook,
            read_files_setting.description(),
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
            "Execute commands",
            Icon::Terminal,
            execute_commands_setting.description(),
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
                    "Some of your permissions are managed by your workspace.",
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
            "Interact with running commands",
            Icon::Workflow,
            write_to_pty_setting.description(),
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
        let list = render_input_list(
            None,
            command_denylist
                .into_iter()
                .zip(view.command_denylist_mouse_state_handles.clone())
                .rev()
                .map(|(cmd, mouse_state_handle)| InputListItem {
                    item: cmd.to_string(),
                    mouse_state_handle,
                    on_remove_action: AISettingsPageAction::RemoveFromProfileCommandDenylist(cmd),
                }),
            Some(&view.command_denylist_editor),
            !ai_settings.is_command_denylist_editable(app),
            appearance,
        );
        render_ai_list(
            "Command denylist",
            "Regular expressions to match commands that the Warper Agent should always ask permission to execute.",
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
        let list = render_input_list(
            None,
            command_allowlist
                .into_iter()
                .zip(view.command_allowlist_mouse_state_handles.clone())
                .rev()
                .map(|(cmd, mouse_state_handle)| InputListItem {
                    item: cmd.to_string(),
                    mouse_state_handle,
                    on_remove_action: AISettingsPageAction::RemoveFromProfileCommandAllowlist(cmd),
                }),
            Some(&view.command_allowlist_editor),
            !ai_settings.is_command_allowlist_editable(app),
            appearance,
        );

        render_ai_list(
            "Command allowlist",
            "Regular expressions to match commands that can be automatically executed by the Warper Agent.",
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
        let list = render_input_list(
            None,
            directory_allowlist
                .clone()
                .into_iter()
                .zip(view.directory_allowlist_mouse_state_handles.clone())
                .rev()
                .map(|(path, mouse_state_handle)| InputListItem {
                    item: path.display().to_string(),
                    mouse_state_handle,
                    on_remove_action: AISettingsPageAction::RemoveFromProfileDirectoryAllowlist(
                        path,
                    ),
                }),
            Some(&view.directory_allowlist_editor),
            !ai_settings.is_directory_allowlist_editable(app),
            appearance,
        );

        render_ai_list(
            "Directory allowlist",
            "Give the agent file access to certain directories.",
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
                            .span("Show model picker in prompt".to_string())
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
            "Base model",
            Some(
                "This model serves as the primary engine behind the Warper Agent. It powers most interactions and invokes other models for tasks like planning or code generation when necessary. Warper may automatically switch to alternate models based on model availability or for auxiliary tasks such as conversation summarization.",
            ),
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
            "Codebase Context",
            AISettingsPageAction::ToggleCodebaseContext,
            *code_settings.codebase_context_enabled,
            ai_settings.is_any_ai_enabled(app),
            codebase_context_toggle,
            &view.local_only_icon_tooltip_states,
            app,
        );

        let codebase_context_description = vec![
            FormattedTextFragment::plain_text(
                "Allow the local agent to generate an outline of your codebase that can be used for context.",
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
            TemplatableMCPServerManager::get_all_runnable_mcp_servers(app);
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
            "Call MCP servers".into(),
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
                FormattedTextFragment::plain_text(
                    "You haven't added any MCP servers yet. Once you do, you'll be able to control how much autonomy the agent has when interacting with them. ",
                ),
                FormattedTextFragment::hyperlink_action(
                    "Add a server",
                    AISettingsPageAction::OpenMCPServerCollection,
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
            "Call MCP servers",
            Icon::Dataflow,
            current_mcp_setting.description(),
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
                "MCP allowlist",
                "Allow the Warper Agent to call these MCP servers.",
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
                "MCP denylist",
                "The Warper Agent will always ask for permission before calling any MCP servers on this list.",
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

        let items = render_input_list(
            None,
            items
                .into_iter()
                .rev()
                .zip(mouse_state_handles.clone())
                .filter_map(|(uuid, mouse_state_handle)| {
                    let server_name = TemplatableMCPServerManager::get_mcp_name(&uuid, app);
                    server_name.map(|server_name| InputListItem {
                        item: server_name,
                        mouse_state_handle,
                        on_remove_action: action(uuid),
                    })
                }),
            None,
            !ai_settings.is_any_ai_enabled(app),
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
        "agent ai input natural language detection autodetection prompt terminal command commands history shell executed execution"
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
                "Show agent tips",
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
            "Include agent-executed commands in history",
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
            static AUTODETECTION_DESCRIPTION_FRAGMENTS: LazyLock<Vec<FormattedTextFragment>> =
                LazyLock::new(|| {
                    vec![
                        FormattedTextFragment::plain_text("Encountered an incorrect detection? "),
                        FormattedTextFragment::hyperlink("Let us know", links::GITHUB_ISSUES_URL),
                    ]
                });

            section.add_children([
                render_ai_setting_toggle::<NLDInTerminalEnabled>(
                    "Autodetect agent prompts in terminal input",
                    AISettingsPageAction::ToggleNLDInTerminal,
                    ai_settings.is_nld_in_terminal_enabled(app),
                    is_toggleable,
                    nld_in_terminal_toggle,
                    &view.local_only_icon_tooltip_states,
                    app,
                ),
                render_ai_setting_toggle::<AIAutoDetectionEnabled>(
                    "Autodetect terminal commands in agent input",
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
                        links::GITHUB_ISSUES_URL,
                    ),
                ]
            });

            section.add_children([
                render_ai_setting_toggle::<AIAutoDetectionEnabled>(
                    "Natural language detection",
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
        "agent mcp server servers model context protocol file-based file based project claude .mcp.json .claude/.mcp.json .codex config.toml .codex/config.toml"
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
            "MCP Servers",
            Some(styles::header_font_color(is_any_ai_enabled, app)),
        )
        .with_padding_bottom(HEADER_PADDING)
        .finish();

        let mcp_description = vec![
            FormattedTextFragment::plain_text(
                "Add MCP servers to extend the local agent's capabilities. \
            MCP servers expose data sources or tools to agents through a standardized interface, essentially acting like plugins. ",
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
                        "Auto-spawn servers from third-party agents",
                        AISettingsPageAction::ToggleFileBasedMcp,
                        *ai_settings.file_based_mcp_enabled,
                        is_any_ai_enabled,
                        self.file_based_mcp_toggle.clone(),
                        &view.local_only_icon_tooltip_states,
                        app,
                    ))
                    .with_child({
                        static FILE_BASED_MCP_DESCRIPTION_FRAGMENTS: LazyLock<
                            Vec<FormattedTextFragment>,
                        > = LazyLock::new(|| {
                            vec![
                                FormattedTextFragment::plain_text(
                                    "Automatically detect and spawn MCP servers from globally-scoped third-party AI agent configuration files (e.g. in your home directory). Servers detected inside a repository are never spawned automatically and must be enabled individually from the MCP settings page. ",
                                ),
                                FormattedTextFragment::plain_text("Supported providers are listed above."),
                            ]
                        });
                        Container::new(
                            FormattedTextElement::new(
                                FormattedText::new([FormattedTextLine::Line(
                                    (*FILE_BASED_MCP_DESCRIPTION_FRAGMENTS).clone(),
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
            "Manage MCP servers",
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
            "Rules",
            AISettingsPageAction::ToggleRules,
            *ai_settings.memory_enabled,
            ai_settings.is_any_ai_enabled(app),
            self.rules_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        );

        let rules_description = vec![
            FormattedTextFragment::plain_text(
                "Rules help the local agent follow your conventions, whether for codebases or specific workflows.",
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
            "Suggested Rules",
            AISettingsPageAction::ToggleRuleSuggestions,
            *ai_settings.rule_suggestions_enabled_internal,
            ai_settings.is_any_ai_enabled(app),
            self.rule_suggestions_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        );

        let description = render_ai_setting_description(
            "Let AI suggest rules to save based on your interactions.",
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
        "agent ai a.i. knowledge fact memory memories rules"
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
            "Knowledge",
            Some(styles::header_font_color(is_any_ai_enabled, app)),
        )
        .with_margin_bottom(HEADER_PADDING)
        .finish();

        let button = render_full_pane_width_ai_button(
            "Manage rules",
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

        column.with_child(button).finish()
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
            "Voice Input",
            AISettingsPageAction::ToggleVoiceInput,
            *ai_settings.voice_input_enabled_internal,
            is_toggleable,
            self.voice_input_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        ));

        let voice_input_description_text_fragments = vec![
            FormattedTextFragment::plain_text(
                "Voice input allows you to control Warper by speaking directly to your terminal (powered by ",
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
        }

        column.finish()
    }
}

impl SettingsWidget for VoiceWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "voice agent ai a.i. speech input natural language talk english"
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
    use_agent_footer_toggle: SwitchStateHandle,
    show_conversation_history_toggle: SwitchStateHandle,
}

impl OtherAIWidget {
    fn create_thinking_display_mode_dropdown(
        ctx: &mut ViewContext<AISettingsPageView>,
    ) -> ViewHandle<Dropdown<AISettingsPageAction>> {
        let items: Vec<DropdownItem<AISettingsPageAction>> = ThinkingDisplayMode::iter()
            .map(|mode| {
                DropdownItem::new(
                    mode.display_name(),
                    AISettingsPageAction::SetThinkingDisplayMode(mode),
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
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "other zero state empty new conversation agent use agent footer toolbar layout chip chips rearrange re-arrange thinking expanded reasoning collapse never show hide conversation history"
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
                .with_child(render_ai_setting_toggle::<ShouldRenderUseAgentToolbarForUserCommands>(
                    "Show \"Use Agent\" footer",
                    AISettingsPageAction::ToggleUseAgentToolbar,
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
            AISettingsPageAction::ToggleShowConversationHistory,
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
            "Show coding agent toolbar",
            AISettingsPageAction::ToggleCLIAgentToolbar,
            *ai_settings.should_render_cli_agent_footer,
            true,
            self.cli_agent_footer_toggle.clone(),
            &view.local_only_icon_tooltip_states,
            app,
        );

        let description_fragments = vec![
            FormattedTextFragment::plain_text(
                "Show a toolbar with quick actions when running coding agents like ",
            ),
            FormattedTextFragment::inline_code("claude"),
            FormattedTextFragment::plain_text(", "),
            FormattedTextFragment::inline_code("codex"),
            FormattedTextFragment::plain_text(", or "),
            FormattedTextFragment::inline_code("gemini"),
            FormattedTextFragment::plain_text("."),
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
                    "Third party CLI agents",
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
                    "Auto show/hide Rich Input based on agent status".into(),
                    Some(styles::header_font_color(true, app)),
                    Some(AdditionalInfo {
                        mouse_state: self.auto_toggle_rich_input_info_tooltip.clone(),
                        on_click_action: None,
                        secondary_text: None,
                        tooltip_override_text: Some(
                            "Requires the Warper plugin for your coding agent".to_owned(),
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
                        "Auto open Rich Input when a coding agent session starts",
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
                    "Auto dismiss Rich Input after prompt submission",
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
                        .span("Commands that enable the toolbar".to_string())
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
                .paragraph(
                    "Add regex patterns to show the coding agent toolbar for matching commands.",
                )
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
                ));
            }
        }

        column.finish()
    }
}

struct ApiKeysWidget {
    openai_api_key_editor: ViewHandle<EditorView>,
    open_router_api_key_editor: ViewHandle<EditorView>,
    open_router_model_editor: ViewHandle<EditorView>,
    anthropic_api_key_editor: ViewHandle<EditorView>,
    google_api_key_editor: ViewHandle<EditorView>,
}

impl ApiKeysWidget {
    fn new(ctx: &mut ViewContext<<Self as SettingsWidget>::View>) -> Self {
        ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.load_keys_from_secure_storage_if_needed(ctx);
        });

        let ai_settings = AISettings::as_ref(ctx);
        let workspace_handle = UserWorkspaces::handle(ctx);
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(ctx);
        let is_byo_enabled = workspace_handle.as_ref(ctx).is_byo_api_key_enabled();

        let ApiKeys {
            openai: openai_key,
            open_router: open_router_key,
            open_router_model,
            anthropic: anthropic_key,
            google: google_key,
        } = ApiKeyManager::as_ref(ctx).keys().clone();

        // A helper macro to create and configure local provider setting editors. This avoids a lot
        // of code duplication and ensures consistency between the editors.
        macro_rules! create_provider_setting_editor {
            ($editor:ident, $value:ident, $set_func:ident, $placeholder:literal, $is_password:literal, $allow_without_byo:expr) => {
                let $editor = ctx.add_typed_action_view(move |ctx| {
                    let appearance = Appearance::handle(ctx).as_ref(ctx);
                    let options = SingleLineEditorOptions {
                        is_password: $is_password,
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
                    if let Some(value) = &$value {
                        editor.set_buffer_text(value, ctx);
                    }
                    editor
                });
                let allow_without_byo = $allow_without_byo;
                AISettingsPageView::update_editor_interaction_state(
                    $editor.clone(),
                    is_any_ai_enabled && (is_byo_enabled || allow_without_byo),
                    ctx,
                );
                ctx.subscribe_to_view(&$editor, |_, $editor, event, ctx| {
                    if matches!(event, EditorEvent::Blurred | EditorEvent::Enter) {
                        let buffer_text = $editor.as_ref(ctx).buffer_text(ctx);
                        let value = buffer_text.is_empty().not().then_some(buffer_text);
                        ApiKeyManager::handle(ctx).update(ctx, |model, ctx| {
                            model.$set_func(value, ctx);
                        });
                    }
                });
            };
        }

        create_provider_setting_editor!(
            openai_api_key_editor,
            openai_key,
            set_openai_key,
            "sk-...",
            true,
            false
        );
        create_provider_setting_editor!(
            open_router_api_key_editor,
            open_router_key,
            set_open_router_key,
            "sk-or-v1-...",
            true,
            ChannelState::channel() == Channel::Oss
        );
        create_provider_setting_editor!(
            open_router_model_editor,
            open_router_model,
            set_open_router_model,
            "openrouter/auto",
            false,
            ChannelState::channel() == Channel::Oss
        );
        create_provider_setting_editor!(
            anthropic_api_key_editor,
            anthropic_key,
            set_anthropic_key,
            "sk-ant-...",
            true,
            false
        );
        create_provider_setting_editor!(
            google_api_key_editor,
            google_key,
            set_google_key,
            "google-api-key",
            true,
            false
        );

        Self {
            openai_api_key_editor,
            open_router_api_key_editor,
            open_router_model_editor,
            anthropic_api_key_editor,
            google_api_key_editor,
        }
    }

    fn render_api_keys_section(
        &self,
        appearance: &Appearance,
        app: &AppContext,
        is_byo_enabled: bool,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);
        let is_oss = ChannelState::channel() == Channel::Oss;
        let is_enabled = is_any_ai_enabled && (is_byo_enabled || is_oss);

        let description = if is_oss {
            "Warper sends agent requests directly to OpenRouter when an OpenRouter key is set. API keys and the OpenRouter model ID are stored locally."
        } else {
            "Use your own API keys from model providers. API keys are stored locally."
        };

        let mut column = Flex::column().with_spacing(16.).with_child(
            Container::new(render_ai_setting_description(description, is_enabled, app))
                // Remove the bottom margin of the description so that it doesn't
                // create extra space between the description and the API key inputs.
                .with_margin_bottom(-styles::DESCRIPTION_MARGIN_BOTTOM)
                .finish(),
        );

        /// Helper function to render the UI for an API key input field.
        fn render_api_key_input(
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

        if !is_oss {
            column.add_child(render_api_key_input(
                appearance,
                "OpenAI API Key",
                self.openai_api_key_editor.clone(),
                is_enabled,
                app,
            ));
        }
        column.add_child(render_api_key_input(
            appearance,
            "OpenRouter API Key",
            self.open_router_api_key_editor.clone(),
            is_enabled,
            app,
        ));
        column.add_child(render_api_key_input(
            appearance,
            "OpenRouter Model",
            self.open_router_model_editor.clone(),
            is_enabled,
            app,
        ));
        if !is_oss {
            column.add_child(render_api_key_input(
                appearance,
                "Anthropic API Key",
                self.anthropic_api_key_editor.clone(),
                is_enabled,
                app,
            ));
            column.add_child(render_api_key_input(
                appearance,
                "Google API Key",
                self.google_api_key_editor.clone(),
                is_enabled,
                app,
            ));
        }

        column.finish()
    }
}

impl SettingsWidget for ApiKeysWidget {
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "api keys bring your own byo openai openrouter open router model anthropic google claude gemini gpt"
    }

    fn render(
        &self,
        _view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ai_settings = AISettings::as_ref(app);
        let is_any_ai_enabled = ai_settings.is_any_ai_enabled(app);
        let is_byo_enabled = UserWorkspaces::as_ref(app).is_byo_api_key_enabled();

        let column = Flex::column()
            .with_child(render_separator(appearance))
            .with_child(
                build_sub_header(
                    appearance,
                    "API Keys",
                    Some(styles::header_font_color(is_any_ai_enabled, app)),
                )
                .with_padding_bottom(HEADER_PADDING)
                .finish(),
            )
            .with_child(self.render_api_keys_section(appearance, app, is_byo_enabled));

        Container::new(column.finish())
            .with_margin_bottom(HEADER_PADDING)
            .finish()
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

        let refresh_credentials_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Refresh", SecondaryTheme)
                .with_icon(Icon::RefreshCw04)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AISettingsPageAction::RefreshAwsBedrockCredentials);
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
        let is_local_policy_enforced = matches!(
            user_workspaces.aws_bedrock_host_enablement_setting(),
            crate::workspaces::workspace::HostEnablementSetting::Enforce
        );
        let is_toggleable =
            is_section_enabled && user_workspaces.is_aws_bedrock_credentials_toggleable();
        let are_credentials_enabled = user_workspaces.is_aws_bedrock_credentials_enabled(app);
        let is_usage_enabled = is_section_enabled && are_credentials_enabled;
        let toggle_description = if is_local_policy_enforced {
            "Warper loads and sends local AWS CLI credentials for Bedrock-supported models. This setting is managed by local policy.".to_string()
        } else {
            "Warper loads and sends local AWS CLI credentials for Bedrock-supported models."
                .to_string()
        };

        let mut column = Flex::column().with_spacing(16.).with_child(
            Flex::column()
                .with_child(render_ai_setting_toggle::<AwsBedrockCredentialsEnabled>(
                    "Use AWS Bedrock credentials",
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
            AISettingsPageAction::ToggleAwsBedrockAutoLogin,
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
    type View = AISettingsPageView;

    fn search_terms(&self) -> &str {
        "aws bedrock amazon credentials login profile"
    }

    fn should_render(&self, app: &AppContext) -> bool {
        // Only show if local policy enables AWS Bedrock for the workspace.
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
