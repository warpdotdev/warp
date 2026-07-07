use std::borrow::Cow;
use std::boxed::Box;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use pathfinder_color::ColorU;
#[cfg(not(target_family = "wasm"))]
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::color::contrast::{
    foreground_color_with_minimum_contrast, MinimumAllowedContrast,
};
use warp_core::ui::color::{coloru_with_opacity, Opacity, Rgb};
use warp_core::ui::theme;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    ChildView, Clipped, Container, CornerRadius, CrossAxisAlignment, Fill, Flex, MainAxisAlignment,
    MainAxisSize, ParentElement, Radius, Rect, Shrinkable, SizeConstraintCondition,
    SizeConstraintSwitch,
};
use warpui::ui_components::components::UiComponentStyles;
use warpui::ui_components::segmented_control::{
    RenderableOptionConfig, SegmentedControl, SegmentedControlEvent, TooltipConfig,
};
use warpui::{
    AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity as _, TypedActionView,
    View, ViewAsRef, ViewContext, ViewHandle,
};

use crate::ai::blocklist::block::cli_controller::CLISubagentController;
use crate::ai::blocklist::prompt::prompt_alert::{PromptAlertEvent, PromptAlertView};
use crate::ai::blocklist::prompt::PromptIconButtonTheme;
use crate::ai::blocklist::{
    BlocklistAIHistoryEvent, BlocklistAIInputModel, InputConfig, InputType,
};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::llms::LLMPreferences;
use crate::ai::AIRequestUsageModel;
use crate::cloud_object::model::generic_string_model::StringModel;
use crate::network::NetworkStatus;
#[cfg(not(target_family = "wasm"))]
use crate::search::ai_context_menu::view::AIContextMenu;
#[cfg(not(target_family = "wasm"))]
use crate::settings::InputSettings;
use crate::settings::{AISettings, AISettingsChangedEvent};
use crate::settings_view::SettingsSection;
use crate::terminal::input::MenuPositioningProvider;
use crate::terminal::keys::TerminalKeybindings;
use crate::terminal::model::block::BlockMetadata;
#[cfg(not(target_family = "wasm"))]
use crate::terminal::model::session::SessionType;
use crate::terminal::model::session::Sessions;
use crate::terminal::profile_model_selector::{
    calculate_max_profile_name_width, calculate_scaled_font_size, ProfileModelSelector,
    ProfileModelSelectorEvent,
};
use crate::terminal::session_settings::{SessionSettings, SessionSettingsChangedEvent};
use crate::terminal::shared_session::permissions_manager::SessionPermissionsManager;
use crate::terminal::view::ambient_agent::AmbientAgentViewModel;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{
    ActionButton, ActionButtonTheme, ButtonSize, NakedTheme, TooltipAlignment,
};
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::BlocklistAIHistoryModel;

pub enum AtContextMenuDisabledReason {
    #[cfg(target_family = "wasm")]
    Wasm,
    #[cfg(not(target_family = "wasm"))]
    NoObjectsAvailable,
    #[cfg(not(target_family = "wasm"))]
    SshWithoutRemoteServer,
    #[cfg(not(target_family = "wasm"))]
    Subshell,
    #[cfg(not(target_family = "wasm"))]
    DisabledInTerminalMode,
}

impl AtContextMenuDisabledReason {
    fn tooltip_text(&self) -> String {
        match self {
            #[cfg(not(target_family = "wasm"))]
            AtContextMenuDisabledReason::NoObjectsAvailable => {
                "No available objects in the current context.".to_string()
            }
            #[cfg(not(target_family = "wasm"))]
            AtContextMenuDisabledReason::SshWithoutRemoteServer => {
                "Not supported in SSH sessions without remote server".to_string()
            }
            #[cfg(not(target_family = "wasm"))]
            AtContextMenuDisabledReason::Subshell => "Not supported in subshells".to_string(),
            #[cfg(target_family = "wasm")]
            AtContextMenuDisabledReason::Wasm => "Requires a filesystem".to_string(),
            #[cfg(not(target_family = "wasm"))]
            AtContextMenuDisabledReason::DisabledInTerminalMode => {
                "Disabled in terminal mode, re-enable in settings".to_string()
            }
        }
    }

    #[cfg(target_family = "wasm")]
    pub fn get_disable_reason(
        _active_block_metadata: Option<&BlockMetadata>,
        _sessions: &Sessions,
        _input_config: &InputConfig,
        _ctx: &AppContext,
    ) -> Option<AtContextMenuDisabledReason> {
        Some(AtContextMenuDisabledReason::Wasm)
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn get_disable_reason(
        active_block_metadata: Option<&BlockMetadata>,
        sessions: &Sessions,
        input_config: &InputConfig,
        ctx: &AppContext,
    ) -> Option<AtContextMenuDisabledReason> {
        // Derive session information from block metadata and sessions
        let (is_ssh_without_remote_server, is_subshell) = active_block_metadata
            .and_then(|metadata| metadata.session_id())
            .and_then(|session_id| sessions.get(session_id))
            .map(|session| {
                let session_type = session.session_type();
                let has_connected_remote_server = matches!(
                    session_type,
                    SessionType::WarpifiedRemote { host_id: Some(_) }
                );
                // The @ menu requires repo metadata which is only available for:
                // - Local sessions
                // - WarpifiedRemote sessions with a connected remote server (host_id is Some)
                //
                // Block when:
                // - SSH wrapper session without a remote server upgrade
                // - WarpifiedRemote still connecting (host_id is None)
                //
                // Note: is_ssh_wrapper_session() is set at bootstrap time and stays true
                // even after the session transitions to WarpifiedRemote with a host_id.
                // So we must check has_connected_remote_server first to avoid
                // incorrectly blocking upgraded sessions.
                let is_ssh_without_remote_server = !has_connected_remote_server
                    && (session.is_ssh_wrapper_session()
                        || matches!(session_type, SessionType::WarpifiedRemote { host_id: None }));
                let is_subshell = session.subshell_info().is_some();
                (is_ssh_without_remote_server, is_subshell)
            })
            .unwrap_or((false, false));

        // Only check the setting if we're in shell mode
        if input_config.input_type == InputType::Shell
            && !*InputSettings::as_ref(ctx)
                .at_context_menu_in_terminal_mode
                .value()
        {
            return Some(AtContextMenuDisabledReason::DisabledInTerminalMode);
        }

        if is_ssh_without_remote_server {
            return Some(AtContextMenuDisabledReason::SshWithoutRemoteServer);
        }
        if is_subshell {
            return Some(AtContextMenuDisabledReason::Subshell);
        }

        // Allow @ context menu outside of git repositories, when we have categories available.
        // Repo-based restrictions will be enforced at the category level
        // (e.g., Code will only be available inside a git repository).

        // This condition kicks in if we're locked in shell mode and not in a git repository, so we have
        // no categories available.
        if AIContextMenu::get_categories_for_mode(
            input_config.input_type.is_ai() || !input_config.is_locked,
            false,
            false, /* is_in_ambient_agent */
            false, /* is_cli_agent_input */
            ctx,
        )
        .is_empty()
        {
            return Some(AtContextMenuDisabledReason::NoObjectsAvailable);
        }

        None
    }
}

const AT_CONTEXT_TOOLTIP: &str = "Attach context";

const BLURRED_OPACITY: Opacity = 50;

// Threshold calculation that estimates the width needed for the profile/model selector
// This is used for determining whether the selector should be rendered as full or compact
fn calculate_profile_model_selector_threshold(
    terminal_view_id: EntityId,
    appearance: &Appearance,
    ctx: &AppContext,
) -> f32 {
    let font_size = appearance.monospace_font_size();
    let has_multiple_profiles = AIExecutionProfilesModel::as_ref(ctx).has_multiple_profiles();

    // base_constant represents a constant width for padding in the UDI.
    // We estimate the width of the remaining UDI elements with a scaling factor multiplied by font size.
    // We consider both profile name and model name lengths since they are variable width.
    let base_constant = 50.0;

    // Calculate text width using em_width for accurate character width
    let scaled_font_size = calculate_scaled_font_size(appearance);
    let em_width = ctx
        .font_cache()
        .em_width(appearance.monospace_font_family(), scaled_font_size);

    let llm_preferences = LLMPreferences::as_ref(ctx);
    let active_llm = llm_preferences.get_active_base_model(ctx, Some(terminal_view_id));
    let model_name_char_count = active_llm.menu_display_name().chars().count() as f32;
    let model_text_width = model_name_char_count * em_width;

    let result = if has_multiple_profiles {
        let profile_name_char_count = AIExecutionProfilesModel::as_ref(ctx)
            .active_profile(Some(terminal_view_id), ctx)
            .data()
            .display_name()
            .chars()
            .count();
        let profile_text_width = (profile_name_char_count as f32 * em_width)
            .min(calculate_max_profile_name_width(appearance));

        font_size * 20.0 + profile_text_width + model_text_width
    } else {
        20.0 * font_size + base_constant + model_text_width
    };
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputToggleMode {
    Terminal,
    AgentMode,
    AutoDetection,
}

/// Custom disabled theme for UDI buttons that preserves background but changes font color
struct UDIDisabledButtonTheme;

impl ActionButtonTheme for UDIDisabledButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<theme::Fill> {
        // Use the same background as the enabled state
        NakedTheme.background(hovered, appearance)
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<theme::Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        internal_colors::text_disabled(appearance.theme(), appearance.theme().surface_1())
    }
}

impl From<&BlocklistAIInputModel> for InputToggleMode {
    fn from(input_model: &BlocklistAIInputModel) -> Self {
        if input_model.is_input_type_locked() {
            match input_model.input_type() {
                InputType::Shell => InputToggleMode::Terminal,
                InputType::AI => InputToggleMode::AgentMode,
            }
        } else {
            InputToggleMode::AutoDetection
        }
    }
}

/// Denormalized state that is required to render UDI styles that isn't easily directly accessible
/// because its owned by other views in other parts of the hierarchy and was never extracted to a
/// Model.
///
/// Kind of an 80/20 situation here.
struct CachedUIState {
    is_input_empty: bool,
    is_hovered: bool,
    is_in_active_terminal: bool,
}

impl CachedUIState {
    fn is_button_bar_blurred(&self) -> bool {
        !self.is_hovered && !self.is_in_active_terminal
    }
}

pub struct UniversalDeveloperInputButtonBar {
    terminal_view_id: EntityId,
    mic_button: ViewHandle<ActionButton>,
    at_button: ViewHandle<ActionButton>,
    file_button: ViewHandle<ActionButton>,
    slash_command_button: ViewHandle<ActionButton>,
    profile_model_selector_full: ViewHandle<ProfileModelSelector>,
    profile_model_selector_compact: ViewHandle<ProfileModelSelector>,
    segmented_control: ViewHandle<SegmentedControl<InputToggleMode>>,
    prompt_alert: ViewHandle<PromptAlertView>,

    cached_ui_state: Rc<RefCell<CachedUIState>>,
    terminal_model: std::sync::Arc<parking_lot::FairMutex<crate::terminal::TerminalModel>>,
}

#[derive(Debug, Clone)]
pub enum UniversalDeveloperInputButtonBarAction {
    #[cfg(feature = "voice_input")]
    ToggleVoiceInput,
    SelectFile,
    SetAIContextMenuOpen(bool),
    OpenSlashCommandMenu,
}

pub enum UniversalDeveloperInputButtonBarEvent {
    #[cfg(feature = "voice_input")]
    ToggleVoiceInput(voice_input::VoiceInputToggledFrom),
    InputTypeSelected(InputType),
    EnableAutoDetection,
    SelectFile,
    SetAIContextMenuOpen(bool),
    PromptAlert(PromptAlertEvent),
    ModelSelectorOpened,
    ModelSelectorClosed,
    OpenSettings(SettingsSection),
    OpenSlashCommandMenu,
}

impl UniversalDeveloperInputButtonBar {
    pub fn new(
        menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
        terminal_view_id: EntityId,
        input_model: ModelHandle<BlocklistAIInputModel>,
        cli_subagent_controller: ModelHandle<CLISubagentController>,
        ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
        terminal_model: std::sync::Arc<parking_lot::FairMutex<crate::terminal::TerminalModel>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let button_size = ButtonSize::UDIButton;

        let mic_button_view = ctx.add_typed_action_view(|_ctx| {
            #[cfg_attr(not(feature = "voice_input"), allow(unused_mut))]
            let mut button = ActionButton::new("", PromptIconButtonTheme::new(false))
                .with_icon(Icon::Microphone)
                .with_tooltip("Voice input")
                .with_size(button_size)
                .with_tooltip_alignment(TooltipAlignment::Left);
            #[cfg(feature = "voice_input")]
            {
                button = button.on_click(|ctx| {
                    ctx.dispatch_typed_action(
                        UniversalDeveloperInputButtonBarAction::ToggleVoiceInput,
                    );
                });
            }
            button
        });

        let at_button_view = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", PromptIconButtonTheme::new(false))
                .with_icon(Icon::AtSign)
                .with_tooltip(AT_CONTEXT_TOOLTIP)
                .with_size(button_size)
                .with_disabled_theme(UDIDisabledButtonTheme)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(
                        UniversalDeveloperInputButtonBarAction::SetAIContextMenuOpen(true),
                    );
                })
        });

        let file_button_view = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", PromptIconButtonTheme::new(false))
                .with_icon(Icon::Plus)
                .with_tooltip("Attach file")
                .with_size(button_size)
                .with_disabled_theme(UDIDisabledButtonTheme)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(UniversalDeveloperInputButtonBarAction::SelectFile);
                })
        });

        let slash_command_menu_view = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", PromptIconButtonTheme::new(false))
                .with_icon(Icon::SlashCommands)
                .with_tooltip("Slash commands")
                .with_size(button_size)
                .with_disabled_theme(UDIDisabledButtonTheme)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(
                        UniversalDeveloperInputButtonBarAction::OpenSlashCommandMenu,
                    );
                })
        });

        let profile_model_selector_full = ctx.add_typed_action_view(|ctx| {
            let mut selector = ProfileModelSelector::new(
                menu_positioning_provider.clone(),
                terminal_view_id,
                input_model.clone(),
                ambient_agent_view_model.clone(),
                terminal_model.clone(),
                None,
                ctx,
            );
            selector.set_render_compact(false, ctx);
            selector
        });

        let profile_model_selector_compact = ctx.add_typed_action_view(|ctx| {
            let mut selector = ProfileModelSelector::new(
                menu_positioning_provider.clone(),
                terminal_view_id,
                input_model.clone(),
                ambient_agent_view_model.clone(),
                terminal_model.clone(),
                None,
                ctx,
            );
            selector.set_render_compact(true, ctx);
            selector
        });

        ctx.subscribe_to_view(&profile_model_selector_full, |me, _, event, ctx| {
            me.handle_profile_model_selector_event(event, ctx);
        });

        ctx.subscribe_to_view(&profile_model_selector_compact, |me, _, event, ctx| {
            me.handle_profile_model_selector_event(event, ctx);
        });

        // Create segmented control options based on auto-detection setting
        let ai_settings = AISettings::as_ref(ctx);
        let is_autodetection_enabled = ai_settings.is_ai_autodetection_enabled(ctx);

        let mut options = vec![InputToggleMode::Terminal, InputToggleMode::AgentMode];

        let mut default_option = input_model.as_ref(ctx).into();
        if is_autodetection_enabled {
            options.push(InputToggleMode::AutoDetection);
        } else if default_option == InputToggleMode::AutoDetection {
            // Don't set the default to auto-detection if it's not enabled.
            default_option = InputToggleMode::Terminal;
        }

        let cached_ui_state = Rc::new(RefCell::new(CachedUIState {
            is_input_empty: true,
            is_hovered: false,
            is_in_active_terminal: false,
        }));

        let ui_state_clone = cached_ui_state.clone();
        let input_model_clone = input_model.clone();
        let segmented_control_view = ctx.add_typed_action_view(|ctx| {
            SegmentedControl::new(
                options,
                move |option, is_selected, app| {
                    let ui_state = ui_state_clone.borrow();
                    build_renderable_option_config(
                        option,
                        is_selected,
                        &input_model_clone,
                        &ui_state,
                        app,
                    )
                },
                default_option,
                segmented_control_styles(ctx),
            )
        });
        // Subscribe to segmented control events
        ctx.subscribe_to_view(&segmented_control_view, |_, _, event, ctx| match event {
            SegmentedControlEvent::OptionSelected(input_mode) => match input_mode {
                InputToggleMode::Terminal => {
                    ctx.emit(UniversalDeveloperInputButtonBarEvent::InputTypeSelected(
                        InputType::Shell,
                    ));
                }
                InputToggleMode::AgentMode => {
                    ctx.emit(UniversalDeveloperInputButtonBarEvent::InputTypeSelected(
                        InputType::AI,
                    ));
                }
                InputToggleMode::AutoDetection => {
                    ctx.emit(UniversalDeveloperInputButtonBarEvent::EnableAutoDetection);
                }
            },
        });

        ctx.subscribe_to_model(&Appearance::handle(ctx), |me, _, _, ctx| {
            me.segmented_control.update(ctx, |segmented_control, ctx| {
                segmented_control.set_styles(segmented_control_styles(ctx), ctx);
            });
            ctx.notify();
        });

        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, ai_settings, event, ctx| {
            // Re-render when AI settings change (like voice input enabled/disabled)
            // Also update segmented control options when auto-detection setting changes
            if let AISettingsChangedEvent::AIAutoDetectionEnabled { .. } = event {
                let is_autodection_enabled =
                    ai_settings.as_ref(ctx).is_ai_autodetection_enabled(ctx);
                me.segmented_control.update(ctx, |segmented_control, ctx| {
                    if is_autodection_enabled {
                        segmented_control.update_options(
                            vec![
                                InputToggleMode::Terminal,
                                InputToggleMode::AgentMode,
                                InputToggleMode::AutoDetection,
                            ],
                            ctx,
                        );
                    } else {
                        segmented_control.update_options(
                            vec![InputToggleMode::Terminal, InputToggleMode::AgentMode],
                            ctx,
                        );
                    }
                });
            }
            ctx.notify();
        });

        let prompt_alert = ctx.add_typed_action_view(PromptAlertView::new);
        ctx.subscribe_to_view(&prompt_alert, |_, _, event, ctx| {
            ctx.emit(UniversalDeveloperInputButtonBarEvent::PromptAlert(
                event.clone(),
            ));
        });

        ctx.subscribe_to_model(&NetworkStatus::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });
        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });
        ctx.subscribe_to_model(&AIRequestUsageModel::handle(ctx), |_, _, _, ctx| {
            ctx.notify()
        });

        ctx.subscribe_to_model(&SessionSettings::handle(ctx), |_, _, event, ctx| {
            if let SessionSettingsChangedEvent::ShowModelSelectorsInPrompt { .. } = event {
                ctx.notify();
            }
        });

        // Subscribe to AIExecutionProfilesModel to potentially show/hide the profile selector button when profiles are added/removed
        ctx.subscribe_to_model(&AIExecutionProfilesModel::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });

        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |me, _, event, ctx| {
                if event
                    .terminal_surface_id()
                    .is_some_and(|id| id != me.terminal_view_id)
                {
                    return;
                }

                match event {
                    BlocklistAIHistoryEvent::StartedNewConversation { .. }
                    | BlocklistAIHistoryEvent::SetActiveConversation { .. }
                    | BlocklistAIHistoryEvent::UpdatedTodoList { .. }
                    | BlocklistAIHistoryEvent::UpdatedConversationStatus { .. } => {
                        ctx.notify();
                    }
                    _ => (),
                }
            },
        );

        ctx.subscribe_to_model(&input_model, move |me, input_model, event, ctx| {
            if !event.did_update_input_config() {
                return;
            }
            let input_mode = InputToggleMode::from(input_model.as_ref(ctx));
            me.segmented_control.update(ctx, |control, ctx| {
                control.set_selected_option(input_mode, ctx);
            });
            me.notify_and_notify_children(ctx);
        });

        // Keep the control disabled state in sync with role changes
        ctx.subscribe_to_model(&SessionPermissionsManager::handle(ctx), |me, _, _, ctx| {
            me.update_segmented_control_disabled_state(ctx);
        });
        // Keep the control disabled state in sync with agent control state
        ctx.subscribe_to_model(&cli_subagent_controller, move |me, _, _, ctx| {
            me.update_segmented_control_disabled_state(ctx);
        });

        let mut me = Self {
            terminal_view_id,
            mic_button: mic_button_view,
            at_button: at_button_view,
            file_button: file_button_view,
            slash_command_button: slash_command_menu_view,
            profile_model_selector_full,
            profile_model_selector_compact,
            segmented_control: segmented_control_view,
            prompt_alert,
            cached_ui_state,
            terminal_model,
        };

        // Initialize disabled state based on current role
        me.update_segmented_control_disabled_state(ctx);

        me
    }

    pub fn set_voice_is_listening(&mut self, is_listening: bool, ctx: &mut ViewContext<Self>) {
        self.mic_button.update(ctx, |mic_button, ctx| {
            if is_listening {
                mic_button.set_icon(Some(Icon::Stop), ctx);
            } else {
                mic_button.set_icon(Some(Icon::Microphone), ctx);
            }
        });
    }

    /// Update the input empty state and refresh the autodetection label
    pub fn update_input_empty_state(&mut self, is_empty: bool, ctx: &mut ViewContext<Self>) {
        if self.cached_ui_state.borrow().is_input_empty == is_empty {
            return;
        }
        self.cached_ui_state.borrow_mut().is_input_empty = is_empty;
        self.notify_and_notify_children(ctx);
    }

    fn handle_profile_model_selector_event(
        &mut self,
        event: &ProfileModelSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ProfileModelSelectorEvent::OpenSettings(settings_section) => {
                ctx.emit(UniversalDeveloperInputButtonBarEvent::OpenSettings(
                    *settings_section,
                ));
            }
            ProfileModelSelectorEvent::MenuVisibilityChanged { open } => {
                if *open {
                    // When model selector menu opens, close other overlays
                    ctx.emit(UniversalDeveloperInputButtonBarEvent::ModelSelectorOpened);
                } else {
                    ctx.emit(UniversalDeveloperInputButtonBarEvent::ModelSelectorClosed);
                }
            }
            ProfileModelSelectorEvent::ToggleInlineModelSelector => {
                // UDI button bar doesn't need to handle this; it's only relevant in AgentInputFooter.
            }
        }
    }

    fn notify_and_notify_children(&self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
        self.segmented_control.update(ctx, |_, ctx| ctx.notify());
    }

    pub fn update_segmented_control_disabled_state(&mut self, ctx: &mut ViewContext<Self>) {
        let (is_reader, is_agent_in_control) = {
            let terminal_model = self.terminal_model.lock();
            (
                terminal_model.shared_session_status().is_reader(),
                terminal_model
                    .block_list()
                    .active_block()
                    .is_active_and_long_running(),
            )
        };

        let tooltip = if is_reader {
            Some("Request edit access to change input mode".to_string())
        } else if is_agent_in_control {
            Some("Input mode locked while agent is monitoring a command".to_string())
        } else {
            None
        };

        self.segmented_control
            .update(ctx, |segmented_control, ctx| {
                segmented_control.set_disabled_tooltip(tooltip.clone().map(Into::into), ctx);
            });
    }

    /// Update the at button's disabled state based on whether AI context menu should render
    pub fn set_at_button_disabled(
        &mut self,
        disable_reason: Option<AtContextMenuDisabledReason>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.at_button.update(ctx, |button, ctx| {
            button.set_disabled(disable_reason.is_some(), ctx);
            button.set_tooltip(
                disable_reason
                    .map(|reason| reason.tooltip_text())
                    .or(Some(AT_CONTEXT_TOOLTIP.to_string())),
                ctx,
            );
            ctx.notify();
        });
    }

    /// Update the slash button's disabled state based on whether the buffer is empty.
    pub fn set_slash_button_disabled(&mut self, should_disable: bool, ctx: &mut ViewContext<Self>) {
        self.slash_command_button.update(ctx, |button, ctx| {
            button.set_disabled(should_disable, ctx);
            ctx.notify();
        });
    }

    pub fn set_is_in_active_terminal(
        &mut self,
        is_in_active_terminal: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.cached_ui_state.borrow().is_in_active_terminal == is_in_active_terminal {
            return;
        }

        self.cached_ui_state.borrow_mut().is_in_active_terminal = is_in_active_terminal;
        self.update_button_bar_styles(ctx);
        self.notify_and_notify_children(ctx);
    }

    /// Set the parent UDI input hovered state.
    /// Used to fade the button bar when the input is not in focus.
    pub fn set_udi_hovered(&mut self, is_hovered: bool, ctx: &mut ViewContext<Self>) {
        if self.cached_ui_state.borrow().is_hovered == is_hovered {
            return;
        }
        self.cached_ui_state.borrow_mut().is_hovered = is_hovered;

        self.update_button_bar_styles(ctx);
        self.notify_and_notify_children(ctx);
    }

    fn update_button_bar_styles(&self, ctx: &mut ViewContext<Self>) {
        self.update_icon_button_themes(ctx);

        let is_blurred = self.cached_ui_state.borrow().is_button_bar_blurred();
        self.profile_model_selector_compact
            .update(ctx, |selector, ctx| selector.set_blurred(is_blurred, ctx));
        self.profile_model_selector_full
            .update(ctx, |selector, ctx| selector.set_blurred(is_blurred, ctx));
    }

    /// Update the themes of the icon buttons to reflect the blurred state
    fn update_icon_button_themes(&self, ctx: &mut ViewContext<Self>) {
        let is_blurred = self.cached_ui_state.borrow().is_button_bar_blurred();
        let theme = PromptIconButtonTheme::new(is_blurred);

        self.mic_button.update(ctx, |button, ctx| {
            button.set_theme(theme.clone(), ctx);
        });

        self.at_button.update(ctx, |button, ctx| {
            button.set_theme(theme.clone(), ctx);
        });

        self.slash_command_button.update(ctx, |button, ctx| {
            button.set_theme(theme.clone(), ctx);
        });

        self.file_button.update(ctx, |button, ctx| {
            button.set_theme(theme.clone(), ctx);
        });
    }

    pub fn is_profile_model_selector_open(&self, ctx: &impl ViewAsRef) -> bool {
        self.profile_model_selector_full.as_ref(ctx).is_open()
            || self.profile_model_selector_compact.as_ref(ctx).is_open()
    }
}

// Implement Entity trait for UniversalDeveloperInputButtonBar
impl Entity for UniversalDeveloperInputButtonBar {
    type Event = UniversalDeveloperInputButtonBarEvent;
}

// Implement View trait for UniversalDeveloperInputButtonBar
impl View for UniversalDeveloperInputButtonBar {
    fn ui_name() -> &'static str {
        "UniversalDeveloperInputButtonBar"
    }

    fn render(&self, app: &AppContext) -> Box<dyn warpui::Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        #[cfg(feature = "voice_input")]
        let is_voice_input_enabled = AISettings::as_ref(app).is_voice_input_enabled(app);

        // Helper function to create a 1px vertical divider
        let create_divider = || {
            Container::new(
                warpui::elements::ConstrainedBox::new(
                    Rect::new().with_background(theme.surface_3()).finish(),
                )
                .with_width(1.0)
                .with_height(20.0) // Match button height approximately
                .finish(),
            )
            .with_margin_left(4.0)
            .with_margin_right(4.0)
            .finish()
        };

        let build_buttons = |model_selector_element: Box<dyn warpui::Element>| {
            // Create a horizontal layout with buttons arranged in a row
            let mut buttons = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_alignment(MainAxisAlignment::Start)
                .with_child(
                    Container::new(ChildView::new(&self.segmented_control).finish())
                        .with_padding_right(4.0)
                        .finish(),
                );
            buttons = buttons.with_child(create_divider());

            buttons = buttons.with_child(ChildView::new(&self.slash_command_button).finish());

            #[cfg(feature = "voice_input")]
            if is_voice_input_enabled {
                buttons = buttons.with_child(ChildView::new(&self.mic_button).finish());
            }

            buttons = buttons.with_child(ChildView::new(&self.at_button).finish());

            // Viewers cannot attach files in shared sessions at this point.
            if !self
                .terminal_model
                .lock()
                .shared_session_status()
                .is_viewer()
            {
                buttons = buttons.with_child(ChildView::new(&self.file_button).finish());
            }

            let show_model_selector = FeatureFlag::ProfilesDesignRevamp.is_enabled()
                || *SessionSettings::as_ref(app).show_model_selectors_in_prompt;
            if show_model_selector {
                buttons = buttons
                    .with_child(create_divider())
                    .with_child(model_selector_element);
            }

            if !self.prompt_alert.as_ref(app).is_no_alert() {
                buttons = buttons.with_child(
                    Shrinkable::new(
                        1.,
                        Clipped::new(ChildView::new(&self.prompt_alert).finish()).finish(),
                    )
                    .finish(),
                );
            }

            buttons.finish()
        };

        let compact_threshold =
            calculate_profile_model_selector_threshold(self.terminal_view_id, appearance, app);
        let content = SizeConstraintSwitch::new(
            // We only need to add left padding to the full profile model selector because the
            // compact selector icons follow the UDI button styling with ~4px margin horizontally.
            build_buttons(
                Container::new(ChildView::new(&self.profile_model_selector_full).finish())
                    .with_padding_left(4.0)
                    .finish(),
            ),
            vec![(
                SizeConstraintCondition::WidthLessThan(compact_threshold),
                build_buttons(ChildView::new(&self.profile_model_selector_compact).finish()),
            )],
        )
        .finish();

        Container::new(Clipped::new(content).finish())
            .with_padding_bottom(12.0)
            .with_padding_right(8.0)
            .finish()
    }
}

impl TypedActionView for UniversalDeveloperInputButtonBar {
    type Action = UniversalDeveloperInputButtonBarAction;

    fn handle_action(
        &mut self,
        action: &UniversalDeveloperInputButtonBarAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            #[cfg(feature = "voice_input")]
            UniversalDeveloperInputButtonBarAction::ToggleVoiceInput => {
                ctx.emit(UniversalDeveloperInputButtonBarEvent::ToggleVoiceInput(
                    voice_input::VoiceInputToggledFrom::Button,
                ));
            }
            UniversalDeveloperInputButtonBarAction::SelectFile => {
                ctx.emit(UniversalDeveloperInputButtonBarEvent::SelectFile);
            }
            UniversalDeveloperInputButtonBarAction::SetAIContextMenuOpen(open) => {
                ctx.emit(UniversalDeveloperInputButtonBarEvent::SetAIContextMenuOpen(
                    *open,
                ));
            }
            UniversalDeveloperInputButtonBarAction::OpenSlashCommandMenu => {
                ctx.emit(UniversalDeveloperInputButtonBarEvent::OpenSlashCommandMenu);
            }
        }
    }
}

fn segmented_control_styles(app: &AppContext) -> UiComponentStyles {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let button_size = 5.0 + appearance.monospace_font_size();
    // Use a smaller base font size that scales properly
    let base_font_size = 10.0; // Start with smaller font
    let scaled_ui_font_size = base_font_size * appearance.monospace_ui_scalar();

    let background = Some(internal_colors::fg_overlay_1(theme).into());

    UiComponentStyles {
        width: Some(button_size), // Match InputPrompt button height for square buttons
        height: Some(button_size), // Match InputPrompt button height
        border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.0))),
        border_width: Some(1.0),
        border_color: Some(Fill::Solid(theme.outline().into_solid())),
        font_family_id: Some(appearance.ui_font_family()),
        font_size: Some(scaled_ui_font_size),
        background,
        ..Default::default()
    }
}

fn build_renderable_option_config(
    option: InputToggleMode,
    is_selected: bool,
    input_model: &ModelHandle<BlocklistAIInputModel>,
    ui_state: &CachedUIState,
    app: &AppContext,
) -> Option<RenderableOptionConfig> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let input_model = input_model.as_ref(app);
    let current_input_type = input_model.input_type();

    let terminal_keybindings = TerminalKeybindings::as_ref(app);

    let compute_colors = |input_type, accent_color| {
        let fg_color = if is_selected {
            theme.background().into_solid()
        } else if input_type == current_input_type && !ui_state.is_input_empty {
            accent_color
        } else {
            theme.sub_text_color(theme.surface_1()).into_solid()
        };

        let bg_color = if is_selected {
            accent_color
        } else {
            ColorU::transparent_black()
        };

        (fg_color, bg_color)
    };

    let mut config = match option {
        InputToggleMode::Terminal => {
            let accent_color = theme.terminal_colors().normal.blue.into();
            let (fg_color, bg_color) = compute_colors(InputType::Shell, accent_color);

            RenderableOptionConfig {
                icon_path: Icon::Terminal.into(),
                icon_color: fg_color,
                label: None,
                tooltip: Some(tooltip_config(
                    "Terminal",
                    Some(terminal_mode_tooltip_subtext(terminal_keybindings)),
                    app,
                )),
                background: bg_color.into(),
            }
        }
        InputToggleMode::AgentMode => {
            let accent_color = theme.terminal_colors().normal.yellow.into();
            let (fg_color, bg_color) = compute_colors(InputType::AI, accent_color);

            RenderableOptionConfig {
                icon_path: Icon::AgentMode.into(),
                icon_color: fg_color,
                label: None,
                tooltip: Some(tooltip_config(
                    "Agent Mode",
                    Some(agent_mode_tooltip_subtext(terminal_keybindings)),
                    app,
                )),
                background: bg_color.into(),
            }
        }
        InputToggleMode::AutoDetection => {
            // Should not actually render anything, when using the new two-option
            // UDI control.
            return None;
        }
    };

    if ui_state.is_button_bar_blurred() {
        config.background = Fill::Solid(coloru_with_opacity(
            config.background.start_color(),
            BLURRED_OPACITY,
        ));
        config.icon_color = coloru_with_opacity(config.icon_color, BLURRED_OPACITY);

        if let Some(tooltip_config) = config.tooltip.as_mut() {
            tooltip_config.background_color =
                coloru_with_opacity(tooltip_config.background_color, BLURRED_OPACITY);
            tooltip_config.border_color =
                coloru_with_opacity(tooltip_config.background_color, BLURRED_OPACITY);
            // Shift text color to ensure sufficient contrast against the blurred background
            tooltip_config.text_color = foreground_color_with_minimum_contrast(
                tooltip_config.text_color,
                Rgb::from(tooltip_config.background_color),
                MinimumAllowedContrast::Text,
            );
        }
    }

    Some(config)
}

const AGENT_MODE_TOOLTIP_PREFIX: &str = "* + space";
const TERMINAL_MODE_TOOLTIP_PREFIX: &str = "! + space";

fn agent_mode_tooltip_subtext(terminal_keybindings: &TerminalKeybindings) -> String {
    let keybinding = terminal_keybindings.set_input_mode_agent_keybinding();
    let Some(keybinding) = keybinding else {
        return AGENT_MODE_TOOLTIP_PREFIX.into();
    };

    format!("{keybinding} or {AGENT_MODE_TOOLTIP_PREFIX}")
}

fn terminal_mode_tooltip_subtext(terminal_keybindings: &TerminalKeybindings) -> String {
    let keybinding = terminal_keybindings.set_input_mode_terminal_keybinding();
    let Some(keybinding) = keybinding else {
        return TERMINAL_MODE_TOOLTIP_PREFIX.into();
    };

    format!("{keybinding} or {TERMINAL_MODE_TOOLTIP_PREFIX}")
}

fn tooltip_config(
    text: impl Into<Cow<'static, str>>,
    subtext: Option<impl Into<Cow<'static, str>>>,
    app: &AppContext,
) -> TooltipConfig {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    TooltipConfig {
        text: text.into(),
        sub_text: subtext.map(Into::into),
        background_color: theme.tooltip_background(),
        // Match UiBuilder::default_tool_tip_styles text color (same as agent picker tooltip)
        text_color: theme.background().into_solid(),
        border_color: theme.outline().into_solid(),
    }
}
