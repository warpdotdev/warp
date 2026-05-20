pub(super) mod chips;
pub mod editor;
mod environment_selector;
pub mod toolbar_item;

use crate::{
    ai::{
        blocklist::{
            history_model::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel},
            prompt::prompt_alert::{PromptAlertEvent, PromptAlertView},
            usage::icon_for_context_window_usage,
            BlocklistAIInputModel,
        },
        execution_profiles::profiles::AIExecutionProfilesModel,
        harness_availability::HarnessAvailabilityModel,
        AIRequestUsageModel,
    },
    appearance::Appearance,
    auth::{AuthManager, AuthStateProvider},
    completer::SessionContext,
    context_chips::{
        self,
        display_chip::{DisplayChip, DisplayChipConfig},
        prompt_type::PromptType,
        ContextChipKind,
    },
    features::FeatureFlag,
    network::NetworkStatus,
    send_telemetry_from_ctx,
    server::telemetry::{PluginChipTelemetryKind, TelemetryEvent},
    settings::{AISettings, AISettingsChangedEvent, PrivacySettings, PrivacySettingsChangedEvent},
    settings_view::SettingsSection,
    terminal::{
        cli_agent_sessions::{
            listener::agent_supports_rich_status, CLIAgentInputState, CLIAgentSessionsModel,
            CLIAgentSessionsModelEvent,
        },
        input::{models::InlineModelSelectorTab, HandoffComposeState, MenuPositioningProvider},
        model_events::ModelEvent,
        profile_model_selector::{ProfileModelSelector, ProfileModelSelectorEvent},
        session_settings::{SessionSettings, SessionSettingsChangedEvent, ToolbarChipSelection},
        shared_session::SharedSessionStatus,
        view::ambient_agent::{AmbientAgentViewModel, ModelSelector, ModelSelectorEvent},
        view::init::OPEN_CLI_AGENT_RICH_INPUT_KEYBINDING,
        view::TerminalAction,
        CLIAgent, TerminalModel,
    },
    ui_components::icons::Icon,
    view_components::{
        action_button::{
            ActionButton, ActionButtonTheme, AdjoinedSide, ButtonSize, KeystrokeSource, NakedTheme,
            TooltipAlignment,
        },
        DismissibleToast,
    },
    workspace::{view::TOGGLE_PROJECT_EXPLORER_BINDING_NAME, ToastStack},
    workspaces::user_workspaces::UserWorkspaces,
};
use toolbar_item::AgentToolbarItemKind;
use warp_cli::agent::Harness;

use std::sync::Arc;

#[cfg(feature = "voice_input")]
use crate::server::server_api::TranscribeError;
#[cfg(not(target_family = "wasm"))]
use crate::terminal::local_shell::LocalShellState;
#[cfg(not(target_family = "wasm"))]
use crate::terminal::ShellLaunchData;
use ai::document::{AIDocumentId, AIDocumentVersion};
use parking_lot::FairMutex;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use settings::Setting;
use settings::ToggleableSetting;
#[cfg(not(target_family = "wasm"))]
use std::env;
#[cfg(not(target_family = "wasm"))]
use std::path::PathBuf;
#[cfg(not(target_family = "wasm"))]
use std::time::Duration;
#[cfg(not(target_family = "wasm"))]
use tokio::fs;
#[cfg(feature = "voice_input")]
use voice_input::{StartListeningError, VoiceSessionResult};

use warp_core::{
    context_flag::ContextFlag,
    report_if_error,
    ui::{
        color::{blend::Blend, contrast::MinimumAllowedContrast, ContrastingColor},
        theme::{color::internal_colors, AnsiColorIdentifier, Fill},
    },
};
#[cfg(feature = "voice_input")]
use warpui::r#async::SpawnedFutureHandle;
use warpui::{
    elements::{
        Border, ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, CornerRadius,
        CrossAxisAlignment, DispatchEventResult, Element, EventHandler, Expanded, Flex,
        MainAxisAlignment, MainAxisSize, OffsetPositioning, ParentElement, PositionedElementAnchor,
        PositionedElementOffsetBounds, Radius, Shrinkable, Stack, Text, Wrap, WrapFill,
        WrapFillEntireRun, DEFAULT_UI_LINE_HEIGHT_RATIO,
    },
    AppContext, Entity, EntityId, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

#[cfg(feature = "local_fs")]
pub(crate) use self::environment_selector::sort_environments_by_recency;
#[cfg(not(target_family = "wasm"))]
use warpui::r#async::Timer;

pub(crate) use self::environment_selector::{
    EnvironmentSelector, EnvironmentSelectorEvent, EnvironmentSelectorTarget,
};
#[cfg(not(target_family = "wasm"))]
use crate::server::telemetry::PluginChipTelemetryAction;
#[cfg(not(target_family = "wasm"))]
use crate::terminal::cli_agent_sessions::plugin_manager::{
    compare_versions, plugin_manager_for, plugin_manager_for_with_shell, CliAgentPluginManager,
    PluginInstallError, PluginModalKind,
};
#[cfg(not(target_family = "wasm"))]
use crate::view_components::ToastLink;
#[cfg(not(target_family = "wasm"))]
use crate::workspace::WorkspaceAction;

const ENABLE_NLD_TOOLTIP: &str = "Enable terminal command autodetection";
const DISABLE_NLD_TOOLTIP: &str = "Disable terminal command autodetection";

const FAST_FORWARD_ON_TOOLTIP: &str = "Turn off auto-approve all agent actions";
const FAST_FORWARD_OFF_TOOLTIP: &str = "Auto-approve all agent actions for this task";

const START_REMOTE_CONTROL_TOOLTIP: &str = "Start remote control";
const START_REMOTE_CONTROL_LOGIN_REQUIRED_TOOLTIP: &str = "Log in to use /remote-control";

const CLOUD_MODE_V2_FOOTER_GAP: f32 = 4.;

/// Voice input state for the CLI agent footer. Unlike the editor-based voice
/// flow (which goes through Input → EditorView), this state is self-contained
/// so that transcribed text can be written directly to the PTY.
#[cfg(feature = "voice_input")]
#[derive(Debug, Default, Clone)]
enum CLIVoiceInputState {
    #[default]
    Stopped,
    Listening,
    Transcribing,
}

/// How long to wait after session creation before showing the install chip.
/// Gives the plugin time to connect and send its `SessionStart` event.
#[cfg(not(target_family = "wasm"))]
const PLUGIN_CHIP_DEBOUNCE: Duration = Duration::from_secs(3);

#[cfg_attr(target_family = "wasm", allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginChipKind {
    Install,
    Update,
}

impl From<PluginChipKind> for PluginChipTelemetryKind {
    fn from(kind: PluginChipKind) -> Self {
        match kind {
            PluginChipKind::Install => PluginChipTelemetryKind::Install,
            PluginChipKind::Update => PluginChipTelemetryKind::Update,
        }
    }
}

/// Builds a composite key for per-agent, per-host plugin chip dismissal.
/// Returns `"<agent_prefix>"` for local sessions or `"<agent_prefix>@<host>"` for remote.
fn plugin_chip_key(agent_prefix: &str, remote_host: &Option<String>) -> String {
    match remote_host {
        Some(host) => format!("{agent_prefix}@{host}"),
        None => agent_prefix.to_owned(),
    }
}

/// Footer control bar at the bottom of the agent input.
///
/// Renders in two modes:
/// - **Agent View mode** (default): model selector, NLD toggle, chips, etc.
/// - **CLI agent mode**: agent icon, image, mic, file explorer, view changes, rich input.
///
/// The mode is determined by reading `CLIAgentSessionsModel` at render time.
/// A single `ViewHandle<AgentInputFooter>` is shared between `Input` and
/// `UseAgentToolbar`, rendering the appropriate mode in each context.
pub struct AgentInputFooter {
    terminal_view_id: EntityId,
    #[cfg_attr(not(feature = "voice_input"), allow(unused))]
    mic_button: ViewHandle<ActionButton>,
    nld_button: ViewHandle<ActionButton>,
    file_button: ViewHandle<ActionButton>,
    start_remote_control_button: ViewHandle<ActionButton>,
    stop_remote_control_button: ViewHandle<ActionButton>,
    context_window_button: ViewHandle<ActionButton>,
    model_selector: ViewHandle<ProfileModelSelector>,
    ftu_callout_close_button: ViewHandle<ActionButton>,
    environment_selector: Option<ViewHandle<EnvironmentSelector>>,
    handoff_environment_selector: ViewHandle<EnvironmentSelector>,
    prompt_alert: ViewHandle<PromptAlertView>,
    ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
    handoff_compose_state: ModelHandle<HandoffComposeState>,
    left_display_chips: Vec<ViewHandle<DisplayChip>>,
    right_display_chips: Vec<ViewHandle<DisplayChip>>,
    // Separate set of display chips for the CLI agent footer.
    // Needed because the CLI footer chip selection can include chips not present in the agent view selection.
    cli_display_chips: Vec<ViewHandle<DisplayChip>>,
    display_chip_config: DisplayChipConfig,

    terminal_model: Arc<FairMutex<TerminalModel>>,
    render_ftu_callout: bool,

    // CLI agent-specific buttons (rendered when a CLI agent session is active).
    file_explorer_button: ViewHandle<ActionButton>,
    rich_input_button: ViewHandle<ActionButton>,
    settings_button: ViewHandle<ActionButton>,
    install_plugin_button: ViewHandle<ActionButton>,
    plugin_instructions_button: ViewHandle<ActionButton>,
    update_plugin_button: ViewHandle<ActionButton>,
    update_instructions_button: ViewHandle<ActionButton>,
    dismiss_plugin_chip_button: ViewHandle<ActionButton>,
    plugin_operation_in_progress: bool,
    /// When `true`, the install chip is allowed to render.
    /// Starts `false` and is set to `true` after a debounce timer fires,
    /// giving the plugin time to connect before we prompt installation.
    /// Reset to `false` when a listener connects.
    plugin_chip_ready: bool,

    // Fast-forward (auto-approve) toggle button shown in the agent view footer.
    fast_forward_button: ViewHandle<ActionButton>,

    // "Hand off to cloud" chip. Visibility is gated on native/local handoff
    // availability. Per-conversation eligibility is enforced by
    // `Workspace::start_local_to_cloud_handoff`.
    handoff_to_cloud_button: ViewHandle<ActionButton>,

    // CLI agent voice input state (self-contained, bypasses editor voice flow).
    #[cfg(feature = "voice_input")]
    cli_voice_input_state: CLIVoiceInputState,
    #[cfg(feature = "voice_input")]
    cli_transcription_handle: Option<SpawnedFutureHandle>,
    v2_model_selector: Option<ViewHandle<ModelSelector>>,
}

impl AgentInputFooter {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
        terminal_view_id: EntityId,
        ai_input_model: ModelHandle<BlocklistAIInputModel>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
        handoff_compose_state: ModelHandle<HandoffComposeState>,
        prompt: ModelHandle<PromptType>,
        display_chip_config: DisplayChipConfig,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let button_size = ButtonSize::AgentInputButton;

        let nld_button = ctx.add_typed_action_view(|ctx| {
            let is_nld_enabled = AISettings::as_ref(ctx).is_ai_autodetection_enabled(ctx);
            let mut button = ActionButton::new("", NLDButtonTheme)
                .with_icon(Icon::NLD)
                .with_size(button_size)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AgentInputFooterAction::ToggleAutodetectionSetting);
                });
            button.set_active(is_nld_enabled, ctx);
            button.set_tooltip(
                Some(if is_nld_enabled {
                    DISABLE_NLD_TOOLTIP
                } else {
                    ENABLE_NLD_TOOLTIP
                }),
                ctx,
            );
            button
        });
        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, settings, event, ctx| {
            let AISettingsChangedEvent::AIAutoDetectionEnabled { .. } = event else {
                return;
            };
            let is_nld_enabled = settings.as_ref(ctx).is_ai_autodetection_enabled(ctx);
            me.nld_button.update(ctx, |button, ctx| {
                button.set_active(is_nld_enabled, ctx);
                button.set_tooltip(
                    Some(if is_nld_enabled {
                        DISABLE_NLD_TOOLTIP
                    } else {
                        ENABLE_NLD_TOOLTIP
                    }),
                    ctx,
                );
            });
        });

        let mic_button = ctx.add_typed_action_view(|_ctx| {
            let button = ActionButton::new("", ActiveMicButtonTheme)
                .with_icon(Icon::Microphone)
                .with_tooltip("Voice input")
                .with_size(button_size)
                .with_tooltip_alignment(TooltipAlignment::Left);
            #[cfg(feature = "voice_input")]
            let button = button.on_click(|ctx| {
                ctx.dispatch_typed_action(AgentInputFooterAction::ToggleVoiceInput);
            });
            button
        });

        #[cfg(feature = "voice_input")]
        {
            let tooltip = AISettings::as_ref(ctx)
                .voice_input_toggle_key
                .value()
                .tooltip_message();
            mic_button.update(ctx, |button, ctx| {
                button.set_tooltip(Some(tooltip), ctx);
            });

            ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, event, ctx| {
                if let AISettingsChangedEvent::VoiceInputToggleKey { .. } = event {
                    let tooltip = AISettings::as_ref(ctx)
                        .voice_input_toggle_key
                        .value()
                        .tooltip_message();
                    me.mic_button.update(ctx, |button, ctx| {
                        button.set_tooltip(Some(tooltip), ctx);
                    });
                }
            });
        }

        let file_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", AgentInputButtonTheme)
                .with_icon(Icon::Plus)
                .with_tooltip("Attach file")
                .with_size(button_size)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AgentInputFooterAction::SelectFile);
                })
        });

        // Fast-forward (auto-approve) toggle button.
        // Uses FastForwardButtonTheme so the button keeps its one-off semantics.
        // The theme still delegates its fill to the shared chip background.
        let fast_forward_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", FastForwardButtonTheme)
                .with_icon(Icon::FastForward)
                .with_tooltip(FAST_FORWARD_OFF_TOOLTIP)
                .with_size(button_size)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(TerminalAction::ToggleAutoexecuteMode);
                })
        });

        // "Hand off to cloud" chip. On click dispatches the workspace action that
        // splits a new cloud-mode pane next to the local pane; that pane handles
        // the rest of the handoff flow when native/local handoff is available.
        let handoff_to_cloud_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", AgentInputButtonTheme)
                .with_icon(Icon::UploadCloud)
                .with_tooltip("Hand off to cloud (or type &)")
                .with_size(button_size)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AgentInputFooterAction::OpenHandoffPane);
                })
        });

        // CLI agent-specific buttons (only rendered when a CLI agent session is active).
        let cli_button_size = ButtonSize::AgentInputButton;
        let file_explorer_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new("File explorer", AgentInputButtonTheme)
                .with_icon(Icon::FileCopy)
                .with_tooltip("Open file explorer")
                .with_size(cli_button_size)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .with_keybinding(
                    KeystrokeSource::Binding(TOGGLE_PROJECT_EXPLORER_BINDING_NAME),
                    ctx,
                )
                .with_compact_keybinding(true)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AgentInputFooterAction::ToggleFileExplorer);
                })
        });
        let rich_input_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new("Rich Input", AgentInputButtonTheme)
                .with_icon(Icon::TextInput)
                .with_tooltip("Open Rich Input")
                .with_size(cli_button_size)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .with_keybinding(
                    KeystrokeSource::Binding(OPEN_CLI_AGENT_RICH_INPUT_KEYBINDING),
                    ctx,
                )
                .with_compact_keybinding(true)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AgentInputFooterAction::ToggleRichInput);
                })
        });
        let settings_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", AgentInputButtonTheme)
                .with_icon(Icon::Settings)
                .with_tooltip("Open coding agent settings")
                .with_size(cli_button_size)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AgentInputFooterAction::OpenCodingAgentSettings);
                })
        });

        let install_plugin_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Enable notifications", InstallPluginButtonTheme)
                .with_icon(Icon::Download)
                .with_tooltip(
                    "Install the Warp plugin to enable rich agent notifications within Warp",
                )
                .with_size(cli_button_size)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .with_adjoined_side(AdjoinedSide::Right)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AgentInputFooterAction::InstallPlugin);
                })
        });

        let plugin_instructions_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Notifications setup instructions", InstallPluginButtonTheme)
                .with_icon(Icon::Info)
                .with_tooltip("View instructions to install the Warp plugin")
                .with_size(cli_button_size)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .with_adjoined_side(AdjoinedSide::Right)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(
                        AgentInputFooterAction::OpenPluginInstallInstructionsPane,
                    );
                })
        });

        let update_plugin_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Update Warp plugin", InstallPluginButtonTheme)
                .with_icon(Icon::Download)
                .with_tooltip("A new version of the Warp plugin is available")
                .with_size(cli_button_size)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .with_adjoined_side(AdjoinedSide::Right)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AgentInputFooterAction::UpdatePlugin);
                })
        });

        let update_instructions_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Plugin update instructions", InstallPluginButtonTheme)
                .with_icon(Icon::Info)
                .with_tooltip("View instructions to update the Warp plugin")
                .with_size(cli_button_size)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .with_adjoined_side(AdjoinedSide::Right)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(
                        AgentInputFooterAction::OpenPluginUpdateInstructionsPane,
                    );
                })
        });

        let dismiss_plugin_chip_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", InstallPluginButtonTheme)
                .with_icon(Icon::X)
                .with_size(cli_button_size)
                .with_tooltip("Dismiss")
                .with_tooltip_alignment(TooltipAlignment::Left)
                .with_adjoined_side(AdjoinedSide::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AgentInputFooterAction::DismissPluginChip);
                })
        });

        // Toggle rich input button label when CLI input session opens/closes.
        // Also reset CLI voice state if the session ends while voice is active.
        ctx.subscribe_to_model(
            &CLIAgentSessionsModel::handle(ctx),
            move |me, _, event, ctx| {
                if event.terminal_view_id() != terminal_view_id {
                    return;
                }

                // Reset the debounce when a session ends so the next
                // session gets a fresh debounce window.
                if let CLIAgentSessionsModelEvent::Ended { .. } = event {
                    #[cfg(feature = "voice_input")]
                    me.stop_cli_voice_and_reset(ctx);
                    me.plugin_chip_ready = false;
                }

                // When a listener connects for an agent with rich status,
                // the plugin is verified installed — hide the chip.
                // (Codex always has a listener but no actual plugin to install.)
                if CLIAgentSessionsModel::as_ref(ctx)
                    .session(me.terminal_view_id)
                    .is_some_and(|s| s.listener.is_some() && agent_supports_rich_status(&s.agent))
                {
                    me.plugin_chip_ready = false;
                }

                // When a session starts, update the install chip label and
                // start a debounce timer for non-auto-install agents.
                #[cfg(not(target_family = "wasm"))]
                if let CLIAgentSessionsModelEvent::Started { .. } = event {
                    if let Some(agent) = me.cli_agent(ctx) {
                        let label = format!("Enable {} notifications", agent.display_name());
                        me.install_plugin_button.update(ctx, |button, ctx| {
                            button.set_label(label, ctx);
                        });
                        if let Some(manager) = plugin_manager_for(agent) {
                            if !manager.can_auto_install() {
                                ctx.spawn(
                                    Timer::after(PLUGIN_CHIP_DEBOUNCE),
                                    |me, _, ctx: &mut ViewContext<Self>| {
                                        let suppress = CLIAgentSessionsModel::as_ref(ctx)
                                            .session(me.terminal_view_id)
                                            .is_some_and(|s| {
                                                s.listener.is_some()
                                                    && agent_supports_rich_status(&s.agent)
                                            });
                                        if !suppress {
                                            me.plugin_chip_ready = true;
                                            ctx.notify();
                                        }
                                    },
                                );
                            }
                        }
                    }
                }

                let CLIAgentSessionsModelEvent::InputSessionChanged {
                    new_input_state, ..
                } = event
                else {
                    ctx.notify();
                    return;
                };
                let is_open = matches!(new_input_state, CLIAgentInputState::Open { .. });
                me.rich_input_button.update(ctx, |button, ctx| {
                    if is_open {
                        button.set_label("Hide Rich Input", ctx);
                        button.set_tooltip(Some("Hide Rich Input"), ctx);
                        button.set_keybinding(
                            Some(KeystrokeSource::Binding(
                                OPEN_CLI_AGENT_RICH_INPUT_KEYBINDING,
                            )),
                            ctx,
                        );
                    } else {
                        button.set_label("Rich Input", ctx);
                        button.set_tooltip(Some("Open Rich Input"), ctx);
                        button.set_keybinding(
                            Some(KeystrokeSource::Binding(
                                OPEN_CLI_AGENT_RICH_INPUT_KEYBINDING,
                            )),
                            ctx,
                        );
                    }
                });
                ctx.notify();
            },
        );

        let start_remote_control_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("/remote-control", RemoteControlButtonTheme)
                .with_icon(Icon::Phone01)
                .with_tooltip(START_REMOTE_CONTROL_TOOLTIP)
                .with_size(cli_button_size)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AgentInputFooterAction::StartRemoteControl);
                })
        });

        let stop_remote_control_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Stop sharing", RemoteControlButtonTheme)
                .with_icon(Icon::StopFilled)
                .with_icon_ansi_color(AnsiColorIdentifier::Red)
                .with_tooltip("Stop sharing")
                .with_size(cli_button_size)
                .with_tooltip_alignment(TooltipAlignment::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(AgentInputFooterAction::StopRemoteControl);
                })
        });

        let context_window_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", AgentInputButtonTheme)
                .with_icon(Icon::ConversationContext0)
                .with_tooltip("Context window usage")
                .with_size(button_size)
                .with_tooltip_alignment(TooltipAlignment::Left)
        });

        let profile_model_selector_full = ctx.add_typed_action_view(|ctx| {
            let mut selector = ProfileModelSelector::new(
                menu_positioning_provider.clone(),
                terminal_view_id,
                ai_input_model,
                ambient_agent_view_model.clone(),
                terminal_model.clone(),
                None,
                ctx,
            );
            selector.set_render_compact(false, ctx);
            selector
        });

        ctx.subscribe_to_view(&profile_model_selector_full, |me, _, event, ctx| {
            me.handle_profile_model_selector_event(event, ctx);
        });

        let environment_selector =
            ambient_agent_view_model
                .as_ref()
                .map(|ambient_agent_view_model| {
                    ctx.add_typed_action_view(|ctx| {
                        EnvironmentSelector::new(
                            menu_positioning_provider.clone(),
                            EnvironmentSelectorTarget::CloudPane(ambient_agent_view_model.clone()),
                            ctx,
                        )
                    })
                });

        let handoff_environment_selector = ctx.add_typed_action_view(|ctx| {
            EnvironmentSelector::new(
                menu_positioning_provider.clone(),
                EnvironmentSelectorTarget::Handoff(handoff_compose_state.clone()),
                ctx,
            )
        });

        if let Some(environment_selector) = environment_selector.as_ref() {
            ctx.subscribe_to_view(environment_selector, |_, _, event, ctx| match event {
                EnvironmentSelectorEvent::MenuVisibilityChanged { open } => {
                    ctx.emit(AgentInputFooterEvent::ToggledChipMenu { open: *open });
                    if !*open {
                        ctx.emit(AgentInputFooterEvent::EnvironmentSelectorClosed);
                    }
                }
                EnvironmentSelectorEvent::OpenEnvironmentManagementPane => {
                    ctx.emit(AgentInputFooterEvent::OpenEnvironmentManagementPane);
                }
            });
        }

        ctx.subscribe_to_view(
            &handoff_environment_selector,
            |_, _, event, ctx| match event {
                EnvironmentSelectorEvent::MenuVisibilityChanged { open } => {
                    ctx.emit(AgentInputFooterEvent::ToggledChipMenu { open: *open });
                    if !*open {
                        ctx.emit(AgentInputFooterEvent::EnvironmentSelectorClosed);
                    }
                }
                EnvironmentSelectorEvent::OpenEnvironmentManagementPane => {
                    ctx.emit(AgentInputFooterEvent::OpenEnvironmentManagementPane);
                }
            },
        );

        if let Some(ambient_agent_view_model) = ambient_agent_view_model.as_ref() {
            ctx.subscribe_to_model(ambient_agent_view_model, |_, _, _, ctx| {
                ctx.notify();
            });
        }

        ctx.subscribe_to_model(
            &handoff_compose_state,
            |me, handoff_compose_state, _, ctx| {
                if !handoff_compose_state.as_ref(ctx).is_active() {
                    me.handoff_environment_selector
                        .update(ctx, |selector, ctx| {
                            selector.set_menu_visibility(false, ctx)
                        });
                }
                ctx.notify();
            },
        );

        let prompt_alert = ctx.add_typed_action_view(PromptAlertView::new);
        ctx.subscribe_to_view(&prompt_alert, |_, _, event, ctx| {
            ctx.emit(AgentInputFooterEvent::PromptAlert(event.clone()));
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
        ctx.subscribe_to_model(&AISettings::handle(ctx), |_, _, event, ctx| {
            if matches!(
                event,
                AISettingsChangedEvent::AIAutoDetectionEnabled { .. }
                    | AISettingsChangedEvent::ShouldForceDisableCloudHandoff { .. }
            ) {
                ctx.notify()
            }
        });
        ctx.subscribe_to_model(&PrivacySettings::handle(ctx), |_, _, event, ctx| {
            if matches!(
                event,
                PrivacySettingsChangedEvent::UpdateIsCloudConversationStorageEnabled { .. }
            ) {
                ctx.notify()
            }
        });
        ctx.subscribe_to_model(&display_chip_config.model_events, |me, _, event, ctx| {
            if let ModelEvent::AgentTaggedInChanged { .. } = event {
                me.update_ftu_callout_render_state(ctx);
            }
        });

        // Keep the remote-control chip in sync with login state so we can
        // disable it and swap the tooltip when the user is anonymous or
        // logged out.
        ctx.subscribe_to_model(&AuthManager::handle(ctx), |me, _, _, ctx| {
            me.sync_remote_control_button(ctx);
        });

        let prompt_for_session_settings = prompt.clone();
        ctx.subscribe_to_model(
            &SessionSettings::handle(ctx),
            move |me, _, event, ctx| match event {
                SessionSettingsChangedEvent::ShowModelSelectorsInPrompt { .. } => {
                    ctx.notify();
                }
                SessionSettingsChangedEvent::AgentToolbarChipSelectionSetting { .. }
                | SessionSettingsChangedEvent::CLIAgentToolbarChipSelectionSetting { .. } => {
                    me.update_display_chips(&prompt_for_session_settings, ctx);
                    ctx.notify();
                }
                _ => {}
            },
        );
        // Subscribe to AIExecutionProfilesModel to potentially show/hide the profile selector button when profiles are added/removed
        ctx.subscribe_to_model(&AIExecutionProfilesModel::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });

        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            |me, _, event, ctx| {
                if event
                    .terminal_view_id()
                    .is_some_and(|id| id != me.terminal_view_id)
                {
                    return;
                }
                me.update_ftu_callout_render_state(ctx);

                match event {
                    BlocklistAIHistoryEvent::StartedNewConversation { .. }
                    | BlocklistAIHistoryEvent::SetActiveConversation { .. }
                    | BlocklistAIHistoryEvent::ClearedActiveConversation { .. }
                    | BlocklistAIHistoryEvent::ClearedConversationsInTerminalView { .. }
                    | BlocklistAIHistoryEvent::RemoveConversation { .. }
                    | BlocklistAIHistoryEvent::UpdatedAutoexecuteOverride { .. } => {
                        me.sync_fast_forward_button(ctx);
                        me.update_context_window_button(ctx);
                        me.model_selector.update(ctx, |_, ctx| ctx.notify());
                        ctx.notify();
                    }
                    BlocklistAIHistoryEvent::UpdatedTodoList { .. }
                    | BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
                    | BlocklistAIHistoryEvent::AppendedExchange { .. }
                    | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. } => {
                        me.update_context_window_button(ctx);
                        me.model_selector.update(ctx, |_, ctx| ctx.notify());
                        ctx.notify();
                    }
                    _ => (),
                }
            },
        );

        ctx.observe(&prompt, |me, model, ctx| {
            me.update_display_chips(&model, ctx);
        });

        let v2_model_selector = if FeatureFlag::CloudModeInputV2.is_enabled() {
            let ambient_agent_view_model_for_selector = ambient_agent_view_model.clone();
            let view = ctx.add_typed_action_view(|ctx| {
                ModelSelector::new(
                    menu_positioning_provider.clone(),
                    terminal_view_id,
                    ambient_agent_view_model_for_selector,
                    ctx,
                )
            });
            ctx.subscribe_to_view(&view, |_, _, event, ctx| match event {
                ModelSelectorEvent::MenuVisibilityChanged { open } => {
                    if *open {
                        ctx.emit(AgentInputFooterEvent::ModelSelectorOpened);
                    } else {
                        ctx.emit(AgentInputFooterEvent::ModelSelectorClosed);
                    }
                }
            });
            Some(view)
        } else {
            None
        };

        let mut me = Self {
            terminal_view_id,
            ambient_agent_view_model,
            nld_button,
            mic_button,
            file_button,
            file_explorer_button,
            rich_input_button,
            settings_button,
            start_remote_control_button,
            stop_remote_control_button,
            install_plugin_button,
            plugin_instructions_button,
            update_plugin_button,
            update_instructions_button,
            dismiss_plugin_chip_button,
            plugin_operation_in_progress: false,
            plugin_chip_ready: false,
            context_window_button,
            model_selector: profile_model_selector_full,
            environment_selector,
            handoff_environment_selector,
            prompt_alert,
            terminal_model,
            handoff_compose_state,
            render_ftu_callout: false,
            left_display_chips: vec![],
            right_display_chips: vec![],
            cli_display_chips: vec![],
            display_chip_config,
            fast_forward_button,
            handoff_to_cloud_button,
            #[cfg(feature = "voice_input")]
            cli_voice_input_state: CLIVoiceInputState::default(),
            #[cfg(feature = "voice_input")]
            cli_transcription_handle: None,
            ftu_callout_close_button: ctx.add_typed_action_view(|_ctx| {
                ActionButton::new("", NakedTheme)
                    .with_icon(Icon::X)
                    .with_size(ButtonSize::XSmall)
                    .on_click(|ctx| {
                        ctx.dispatch_typed_action(AgentInputFooterAction::DismissFtuModelCallout);
                    })
            }),
            v2_model_selector,
        };
        me.sync_fast_forward_button(ctx);
        me.sync_remote_control_button(ctx);
        me.update_context_window_button(ctx);
        me.update_display_chips(&prompt, ctx);
        me.update_ftu_callout_render_state(ctx);
        me
    }

    pub fn set_current_repo_path(
        &mut self,
        repo_path: Option<std::path::PathBuf>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.display_chip_config.current_repo_path = repo_path;
        // Chips will be rebuilt on the next GitRepoStatusEvent::MetadataChanged.
        // Notify to ensure any existing chips reflect the change.
        ctx.notify();
    }

    pub fn is_v2_model_selector_open(&self, app: &AppContext) -> bool {
        self.v2_model_selector
            .as_ref()
            .is_some_and(|s| s.as_ref(app).is_menu_open())
    }

    pub fn open_v2_model_selector(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(selector) = self.v2_model_selector.clone() {
            selector.update(ctx, |s, ctx| s.open_menu(ctx));
        }
    }

    pub fn is_v2_environment_selector_open(&self, app: &AppContext) -> bool {
        self.environment_selector
            .as_ref()
            .is_some_and(|s| s.as_ref(app).is_menu_open())
    }

    pub fn open_v2_environment_selector(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(selector) = self.environment_selector.clone() {
            selector.update(ctx, |s, ctx| s.open_menu(ctx));
        }
    }

    fn should_render_cloud_mode_v2(&self, app: &AppContext) -> bool {
        FeatureFlag::CloudModeInputV2.is_enabled()
            && FeatureFlag::CloudMode.is_enabled()
            && self
                .ambient_agent_view_model
                .as_ref()
                .is_some_and(|ambient_agent_model| {
                    ambient_agent_model
                        .as_ref(app)
                        .is_configuring_ambient_agent()
                })
    }

    fn render_cloud_mode_v2_footer(&self, app: &AppContext) -> Box<dyn Element> {
        // `app` is only consumed under the `voice_input` cfg below; reference it here so the
        // parameter doesn't trip the unused-variable lint when the feature is disabled.
        #[cfg(not(feature = "voice_input"))]
        let _ = app;

        let mut left = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(CLOUD_MODE_V2_FOOTER_GAP);
        if let Some(environment_selector) = self.environment_selector.as_ref() {
            left = left.with_child(ChildView::new(environment_selector).finish());
        }

        let mut right = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(CLOUD_MODE_V2_FOOTER_GAP);

        // Only show the mic button when voice input is compiled in *and* the
        // user has voice input enabled in settings, matching V1's behavior.
        #[cfg(feature = "voice_input")]
        if AISettings::as_ref(app).is_voice_input_enabled(app) {
            right = right.with_child(ChildView::new(&self.mic_button).finish());
        }

        right = right.with_child(ChildView::new(&self.file_button).finish());

        if let Some(model_selector) = self.v2_model_selector.as_ref() {
            // Only show the model selector when the active harness has available models.
            // Some harnesses (e.g. Gemini) may not have any server-provided model options.
            let show_selector = self
                .ambient_agent_view_model
                .as_ref()
                .map(|m| m.as_ref(app).selected_harness())
                .is_none_or(|harness| match harness {
                    Harness::Oz | Harness::Unknown => true,
                    _ => HarnessAvailabilityModel::as_ref(app)
                        .models_for(harness)
                        .is_some_and(|models| !models.is_empty()),
                });
            if show_selector {
                right = right.with_child(ChildView::new(model_selector).finish());
            }
        }

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(left.finish())
            .with_child(right.finish())
            .finish()
    }

    fn all_display_chips(&self) -> impl Iterator<Item = &ViewHandle<DisplayChip>> {
        self.left_display_chips
            .iter()
            .chain(self.right_display_chips.iter())
            .chain(self.cli_display_chips.iter())
    }

    pub fn update_session_context(
        &mut self,
        session_context: Option<SessionContext>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.display_chip_config.session_context = session_context.clone();
        for chip_view in self.all_display_chips() {
            chip_view.update(ctx, |chip, chip_ctx| {
                chip.update_session_context(session_context.clone(), chip_ctx);
            });
        }
    }

    fn has_active_cli_agent_input_session(&self, app: &AppContext) -> bool {
        CLIAgentSessionsModel::as_ref(app).is_input_open(self.terminal_view_id)
    }

    fn cli_agent(&self, app: &AppContext) -> Option<CLIAgent> {
        CLIAgentSessionsModel::as_ref(app)
            .session(self.terminal_view_id)
            .map(|session| session.agent)
    }

    fn is_cli_agent_session_active(&self, app: &AppContext) -> bool {
        CLIAgentSessionsModel::as_ref(app)
            .session(self.terminal_view_id)
            .is_some()
    }

    fn select_cli_file(&mut self, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        let view_id = ctx.view_id();
        let file_picker_config = warpui::platform::FilePickerConfiguration::new();

        ctx.open_file_picker(
            move |result, ctx| match result {
                Ok(paths) => {
                    if let Some(path) = paths.first() {
                        ctx.dispatch_typed_action_for_view(
                            window_id,
                            view_id,
                            &AgentInputFooterAction::InsertFilePath(path.clone()),
                        );
                    }
                }
                Err(err) => {
                    let window_id = ctx.window_id();
                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::error(format!("{err}")),
                            window_id,
                            ctx,
                        );
                    });
                }
            },
            file_picker_config,
        );
    }

    /// Which plugin chip to show, if any.
    fn plugin_chip_kind(&self, app: &AppContext) -> Option<PluginChipKind> {
        #[cfg(target_family = "wasm")]
        {
            let _ = (app, self.plugin_operation_in_progress);
            None
        }
        #[cfg(not(target_family = "wasm"))]
        {
            if self.plugin_operation_in_progress
                || self.terminal_model.lock().is_shared_ambient_agent_session()
                || !FeatureFlag::HOANotifications.is_enabled()
            {
                return None;
            }

            let ai_settings = AISettings::as_ref(app);
            if !*ai_settings.show_agent_notifications {
                return None;
            }

            let session = CLIAgentSessionsModel::as_ref(app).session(self.terminal_view_id)?;

            let manager = plugin_manager_for(session.agent)?;
            let min_version = manager.minimum_plugin_version();
            let chip_key = plugin_chip_key(session.agent.command_prefix(), &session.remote_host);

            // If the plugin is connected (listener present) and this agent supports
            // version-based updates, check the reported version.
            if session.listener.is_some() && manager.supports_update() {
                let needs_update = match &session.plugin_version {
                    // No version reported = pre-versioning plugin, definitely outdated.
                    None => true,
                    Some(v) => compare_versions(v, min_version).is_lt(),
                };
                if !needs_update {
                    return None;
                }
                // Check update chip dismissal.
                let dismissed_version = ai_settings.plugin_update_chip_dismissed_version(&chip_key);
                if !dismissed_version.is_empty()
                    && compare_versions(dismissed_version, min_version).is_ge()
                {
                    return None;
                }
                return Some(PluginChipKind::Update);
            }

            // For agents without auto-install, wait for the debounce timer
            // before showing the install chip.
            if !manager.can_auto_install() && !self.plugin_chip_ready {
                return None;
            }

            let install_chip_dismissed = ai_settings.is_plugin_install_chip_dismissed(&chip_key);

            // For remote sessions, we can't check the filesystem.
            if session.is_remote() {
                return (!install_chip_dismissed).then_some(PluginChipKind::Install);
            }

            if manager.is_installed() {
                // Installed but no listener yet. Check the on-disk version as a fallback
                // — the plugin may be too old to send structured events.
                if manager.needs_update() {
                    let dismissed_version =
                        ai_settings.plugin_update_chip_dismissed_version(&chip_key);
                    if !dismissed_version.is_empty()
                        && compare_versions(dismissed_version, min_version).is_ge()
                    {
                        return None;
                    }
                    return Some(PluginChipKind::Update);
                }
                // Up to date on disk — wait for the listener to connect.
                return None;
            }

            // Not installed locally.
            (!install_chip_dismissed).then_some(PluginChipKind::Install)
        }
    }

    /// Whether the chip should open the manual instructions modal instead of auto-operating.
    fn should_use_manual_mode(&self, app: &AppContext) -> bool {
        let sessions_model = CLIAgentSessionsModel::as_ref(app);
        let session = match sessions_model.session(self.terminal_view_id) {
            Some(s) => s,
            None => return false,
        };

        // Custom toolbar commands always use manual mode because the user's
        // binary may differ from the agent's standard CLI tool.
        if session.custom_command_prefix.is_some() {
            return true;
        }

        #[cfg(not(target_family = "wasm"))]
        if let Some(manager) = plugin_manager_for(session.agent) {
            if !manager.can_auto_install() {
                return true;
            }
        }
        if session.is_remote() {
            return true;
        }
        // Docker sandbox sessions run inside a container; our auto-install
        // path operates on the host's shell config and would install the
        // plugin in the wrong place. Fall back to the manual-instructions
        // modal so the user can paste the install command into the sandbox
        // PTY and install inside the container.
        //
        // `ShellLaunchData` is only available on native builds; wasm builds
        // can't produce a `DockerSandbox` variant either way.
        #[cfg(not(target_family = "wasm"))]
        {
            let shell_data = {
                let model = self.terminal_model.lock();
                model.active_shell_launch_data().cloned()
            };
            if matches!(shell_data, Some(ShellLaunchData::DockerSandbox { .. })) {
                return true;
            }
        }
        sessions_model.has_plugin_auto_failed(session.agent, &session.remote_host)
    }

    /// Records that the auto plugin operation could not start, shows an error toast,
    /// and re-renders so the chip switches to manual-instructions mode.
    #[cfg(not(target_family = "wasm"))]
    fn record_plugin_auto_failure_and_notify(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(agent) = self.cli_agent(ctx) {
            let remote_host = CLIAgentSessionsModel::as_ref(ctx)
                .session(self.terminal_view_id)
                .and_then(|s| s.remote_host.clone());
            CLIAgentSessionsModel::handle(ctx).update(ctx, |model, _| {
                model.record_plugin_auto_failure(agent, remote_host);
            });
        }
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            toast_stack.add_ephemeral_toast(
                DismissibleToast::error(
                    "Could not automatically install plugin. \
                     Please click the chip again for manual installation steps."
                        .to_owned(),
                ),
                window_id,
                ctx,
            );
        });
        ctx.notify();
    }

    /// Shared handler for both install and update plugin operations.
    /// `progress_toast` is shown while the operation runs; `success_toast` on success.
    #[cfg(not(target_family = "wasm"))]
    fn handle_plugin_operation<F, Fut>(
        &mut self,
        progress_toast: &str,
        error_label: &str,
        success_toast: &str,
        operation_kind: PluginChipTelemetryKind,
        operation: F,
        ctx: &mut ViewContext<Self>,
    ) -> bool
    where
        F: FnOnce(Box<dyn CliAgentPluginManager>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), PluginInstallError>> + Send + 'static,
    {
        let Some(agent) = self.cli_agent(ctx) else {
            return false;
        };
        let shell_data = {
            let model = self.terminal_model.lock();
            model.active_shell_launch_data().cloned()
        };
        let (shell_path, shell_type) = match shell_data {
            Some(ShellLaunchData::Executable {
                executable_path,
                shell_type,
            })
            | Some(ShellLaunchData::MSYS2 {
                executable_path,
                shell_type,
            }) => (Some(executable_path), Some(shell_type)),
            // Shell not yet resolved (e.g. still bootstrapping).
            None => (None, None),
            // WSL is not supported for auto-install.
            Some(ShellLaunchData::WSL { .. }) => return false,
            // Auto-install isn't supported for Docker sandbox sessions — the
            // install would run against the *host's* shell config, not the
            // container's. `should_use_manual_mode` already routes sandbox
            // sessions to the manual-instructions modal, so this arm is a
            // defensive fallthrough; users can still install the plugin by
            // pasting the command into the sandbox PTY themselves.
            //
            // TODO(advait): Add native auto-install support for sandboxes,
            // e.g. by routing the install through the session's in-band
            // executor so it runs inside the container and targets the
            // container's shell / package layout. A common use case will be
            // running a 3p harness (e.g. Claude Code) inside a sandbox and
            // needing the Warp plugin to integrate with it.
            Some(ShellLaunchData::DockerSandbox { .. }) => return false,
        };

        // Await the interactive PATH so nvm-installed tools like `claude`
        // are on PATH, matching how LSP operations capture the PATH.
        let path_future = LocalShellState::handle(ctx).update(ctx, |shell_state, ctx| {
            shell_state.get_interactive_path_env_var(ctx)
        });

        self.plugin_operation_in_progress = true;
        ctx.notify();

        let window_id = ctx.window_id();
        let toast_id = "cli-agent-plugin-operation".to_owned();

        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            toast_stack.add_persistent_toast(
                DismissibleToast::default(progress_toast.to_owned())
                    .with_object_id(toast_id.clone()),
                window_id,
                ctx,
            );
        });

        let toast_id_for_callback = toast_id.clone();
        let error_label = error_label.to_owned();
        let success_toast = success_toast.to_owned();
        ctx.spawn(
            async move {
                let path_env_var = path_future.await;
                let Some(manager) =
                    plugin_manager_for_with_shell(agent, shell_path, shell_type, path_env_var)
                else {
                    return Err((
                        PluginInstallError {
                            message: "No plugin manager available".to_owned(),
                            log: String::new(),
                        },
                        None,
                    ));
                };

                match operation(manager).await {
                    Ok(()) => Ok(()),
                    Err(err) => {
                        let log_path = write_install_log(agent, &err).await;
                        Err((err, log_path))
                    }
                }
            },
            move |me, result, ctx| {
                me.plugin_operation_in_progress = false;

                if result.is_ok() {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::CLIAgentPluginOperationSucceeded {
                            cli_agent: agent.into(),
                            operation: operation_kind,
                        },
                        ctx
                    );
                    ctx.emit(AgentInputFooterEvent::PluginInstalled(agent));
                } else {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::CLIAgentPluginOperationFailed {
                            cli_agent: agent.into(),
                            operation: operation_kind,
                        },
                        ctx
                    );
                }

                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = match result {
                        Ok(()) => DismissibleToast::success(success_toast.clone()),
                        Err((err, log_path)) => {
                            let remote_host = CLIAgentSessionsModel::as_ref(ctx)
                                .session(me.terminal_view_id)
                                .and_then(|s| s.remote_host.clone());
                            CLIAgentSessionsModel::handle(ctx).update(ctx, |model, _| {
                                model.record_plugin_auto_failure(agent, remote_host);
                            });
                            log::error!(
                                "Failed plugin operation for {agent:?}: {err}\n{log}",
                                log = err.log,
                            );
                            let mut toast =
                                DismissibleToast::error(format!("{error_label}: {err}"));
                            if let Some(log_path) = log_path {
                                toast = toast.with_link(
                                    ToastLink::new("See logs for details".to_owned())
                                        .with_onclick_action(WorkspaceAction::OpenFilePath {
                                            path: log_path,
                                        }),
                                );
                            }
                            toast
                        }
                    };
                    toast_stack.add_ephemeral_toast(
                        toast.with_object_id(toast_id_for_callback),
                        window_id,
                        ctx,
                    );
                });
                ctx.notify();
            },
        );
        true
    }

    #[cfg(not(target_family = "wasm"))]
    fn handle_install_plugin(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let success_msg = self
            .cli_agent(ctx)
            .and_then(plugin_manager_for)
            .map(|m| m.install_success_message())
            .unwrap_or("Warp plugin installed. Please restart the session to activate.");
        self.handle_plugin_operation(
            "Installing Warp plugin...",
            "Failed to install Warp plugin",
            success_msg,
            PluginChipTelemetryKind::Install,
            |manager| async move { manager.install().await },
            ctx,
        )
    }

    #[cfg(not(target_family = "wasm"))]
    fn handle_update_plugin(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let success_msg = self
            .cli_agent(ctx)
            .and_then(plugin_manager_for)
            .map(|m| m.update_success_message())
            .unwrap_or("Warp plugin updated. Please restart the session to activate.");
        self.handle_plugin_operation(
            "Updating Warp plugin...",
            "Failed to update Warp plugin",
            success_msg,
            PluginChipTelemetryKind::Update,
            |manager| async move { manager.update().await },
            ctx,
        )
    }

    fn cli_display_chip(
        &self,
        chip_kind: ContextChipKind,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        self.cli_display_chips
            .iter()
            .find(|chip| chip.as_ref(app).chip_kind() == &chip_kind)
            .filter(|chip| chip.as_ref(app).should_render(app))
            .map(|chip| ChildView::new(chip).finish())
    }

    fn render_cli_toolbar_item(
        &self,
        item: &AgentToolbarItemKind,
        shared_status: &SharedSessionStatus,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        if !item.available_in().is_available_for_cli()
            || !item.available_to_session_viewer(shared_status, false)
        {
            return None;
        }

        // Hide ShareSession for shared ambient (cloud) agent sessions —
        // it doesn't make sense to offer remote-control when already
        // viewing a cloud agent's shared session.
        if matches!(item, AgentToolbarItemKind::ShareSession)
            && self.terminal_model.lock().is_shared_ambient_agent_session()
        {
            return None;
        }

        match item {
            AgentToolbarItemKind::ContextChip(chip_kind) => {
                self.cli_display_chip(chip_kind.clone(), app)
            }
            AgentToolbarItemKind::FileExplorer => {
                Some(ChildView::new(&self.file_explorer_button).finish())
            }
            AgentToolbarItemKind::RichInput => FeatureFlag::CLIAgentRichInput
                .is_enabled()
                .then(|| ChildView::new(&self.rich_input_button).finish()),
            AgentToolbarItemKind::FileAttach => Some(ChildView::new(&self.file_button).finish()),
            AgentToolbarItemKind::VoiceInput => {
                #[cfg(feature = "voice_input")]
                {
                    let enabled = AISettings::as_ref(app).is_voice_input_enabled(app);
                    enabled.then(|| ChildView::new(&self.mic_button).finish())
                }
                #[cfg(not(feature = "voice_input"))]
                None
            }
            AgentToolbarItemKind::ShareSession => {
                let enabled = FeatureFlag::CreatingSharedSessions.is_enabled()
                    && FeatureFlag::HOARemoteControl.is_enabled()
                    && ContextFlag::CreateSharedSession.is_enabled();
                if !enabled {
                    return None;
                }

                let button = if shared_status.is_sharer() {
                    &self.stop_remote_control_button
                } else {
                    &self.start_remote_control_button
                };
                Some(ChildView::new(button).finish())
            }
            AgentToolbarItemKind::Settings => Some(ChildView::new(&self.settings_button).finish()),
            // Handled by the available_in() guard above; included for exhaustiveness.
            AgentToolbarItemKind::ModelSelector
            | AgentToolbarItemKind::NLDToggle
            | AgentToolbarItemKind::ContextWindowUsage
            | AgentToolbarItemKind::FastForwardToggle
            | AgentToolbarItemKind::HandoffToCloud => None,
        }
    }

    fn render_cli_mode_footer(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let cli_icon_size = ButtonSize::AgentInputButton.icon_size(appearance, app);

        // Extract everything we need from the terminal model up front and drop
        // the lock before calling into helpers like `should_use_manual_mode`
        // and `render_cli_toolbar_item`, which may re-lock the same model and
        // would deadlock since the lock is non-reentrant.
        let (background_color, shared_status) = {
            let terminal_model = self.terminal_model.lock();
            let background_color = if terminal_model.is_alt_screen_active() {
                terminal_model
                    .alt_screen()
                    .inferred_bg_color()
                    .unwrap_or_else(|| appearance.theme().surface_1().into_solid())
            } else {
                appearance.theme().surface_1().into_solid()
            };
            let shared_status = terminal_model.shared_session_status().clone();
            (background_color, shared_status)
        };

        let session_settings = SessionSettings::as_ref(app);
        let left_items = session_settings
            .cli_agent_footer_chip_selection
            .left_items();
        let right_items = session_settings
            .cli_agent_footer_chip_selection
            .right_items();

        let mut left_buttons = Wrap::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_run_spacing(4.)
            .with_spacing(4.);

        // CLI agent brand icon is always rendered (not configurable).
        if let Some(agent) = self.cli_agent(app) {
            if let Some(icon) = agent.icon() {
                let icon_color = agent
                    .brand_color()
                    .map(|c| c.on_background(background_color, MinimumAllowedContrast::NonText))
                    .unwrap_or_else(|| appearance.theme().foreground().into_solid());
                left_buttons.add_child(
                    Container::new(
                        ConstrainedBox::new(icon.to_warpui_icon(Fill::Solid(icon_color)).finish())
                            .with_width(cli_icon_size)
                            .with_height(cli_icon_size)
                            .finish(),
                    )
                    .with_padding_right(8.)
                    .finish(),
                );
            }
        }

        if let Some(chip_kind) = self.plugin_chip_kind(app) {
            let manual = self.should_use_manual_mode(app);
            let chip = match (chip_kind, manual) {
                (PluginChipKind::Install, false) => {
                    ChildView::new(&self.install_plugin_button).finish()
                }
                (PluginChipKind::Install, true) => {
                    ChildView::new(&self.plugin_instructions_button).finish()
                }
                (PluginChipKind::Update, false) => {
                    ChildView::new(&self.update_plugin_button).finish()
                }
                (PluginChipKind::Update, true) => {
                    ChildView::new(&self.update_instructions_button).finish()
                }
            };
            let chip_with_dismiss = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(chip)
                .with_child(ChildView::new(&self.dismiss_plugin_chip_button).finish())
                .finish();
            left_buttons.add_child(chip_with_dismiss);
        }

        for item in &left_items {
            if let Some(element) = self.render_cli_toolbar_item(item, &shared_status, app) {
                left_buttons.add_child(element);
            }
        }

        let mut right_buttons = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(4.);

        for item in &right_items {
            if let Some(element) = self.render_cli_toolbar_item(item, &shared_status, app) {
                right_buttons.add_child(element);
            }
        }

        let content = Wrap::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(WrapFillEntireRun::new(left_buttons.finish()).finish())
            .with_child(WrapFill::new(0., right_buttons.finish()).finish())
            .with_run_spacing(context_chips::spacing::UDI_ROW_RUN_SPACING)
            .finish();
        let content = EventHandler::new(content)
            .on_right_mouse_down(|ctx, _, position| {
                ctx.dispatch_typed_action(AgentInputFooterAction::ShowContextMenu { position });
                DispatchEventResult::StopPropagation
            })
            .finish();

        Container::new(content).with_vertical_padding(4.).finish()
    }

    pub fn has_open_chip_menu(&self, app: &AppContext) -> bool {
        let has_open_display_chip = self
            .all_display_chips()
            .any(|chip| chip.as_ref(app).display_chip_kind().has_open_menu());

        let has_open_env_selector = self
            .environment_selector
            .as_ref()
            .is_some_and(|selector| selector.as_ref(app).is_menu_open());
        let has_open_handoff_env_selector =
            self.handoff_environment_selector.as_ref(app).is_menu_open();

        has_open_display_chip || has_open_env_selector || has_open_handoff_env_selector
    }

    pub fn is_model_selector_open(&self, app: &AppContext) -> bool {
        self.model_selector.as_ref(app).is_open()
    }

    fn update_ftu_callout_render_state(&mut self, ctx: &mut ViewContext<Self>) {
        let ftu_dismissed = *AISettings::as_ref(ctx).ftu_model_callout_dismissed;
        if !self.render_ftu_callout && ftu_dismissed {
            return;
        }

        let showing_ftu_model_picker = FeatureFlag::InlineMenuHeaders.is_enabled()
            && self
                .terminal_model
                .lock()
                .block_list()
                .active_block()
                .is_agent_in_control_or_tagged_in();
        if showing_ftu_model_picker && !ftu_dismissed {
            if !self.render_ftu_callout {
                self.render_ftu_callout = true;
                ctx.notify();
            }
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                // This setting actually indicates whether we've shown the ftu callout at all,
                // but it originally tracked whether the user manually dismissed the callout and
                // we don't want to resurface the callout to folks who have already dismissed.
                let _ = settings.ftu_model_callout_dismissed.set_value(true, ctx);
            });
        } else if !showing_ftu_model_picker && self.render_ftu_callout {
            self.render_ftu_callout = false;
            ctx.notify();
        }
    }

    fn handle_profile_model_selector_event(
        &mut self,
        event: &ProfileModelSelectorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ProfileModelSelectorEvent::MenuVisibilityChanged { open } => {
                if *open {
                    ctx.emit(AgentInputFooterEvent::ModelSelectorOpened);
                } else {
                    ctx.emit(AgentInputFooterEvent::ModelSelectorClosed);
                }
            }
            ProfileModelSelectorEvent::OpenSettings(section) => {
                ctx.emit(AgentInputFooterEvent::OpenSettings(*section));
            }
            ProfileModelSelectorEvent::ToggleInlineModelSelector => {
                if self.render_ftu_callout {
                    self.render_ftu_callout = false;
                    ctx.notify();
                }

                let initial_tab = if self
                    .terminal_model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_agent_in_control_or_tagged_in()
                {
                    InlineModelSelectorTab::FullTerminalUse
                } else {
                    InlineModelSelectorTab::BaseAgent
                };

                ctx.emit(AgentInputFooterEvent::ToggleInlineModelSelector { initial_tab });
            }
        }
    }

    pub fn set_voice_is_active(&mut self, is_active: bool, ctx: &mut ViewContext<Self>) {
        self.mic_button.update(ctx, |button, ctx| {
            button.set_active(is_active, ctx);
        });
    }

    #[cfg(feature = "voice_input")]
    fn stop_cli_voice_and_reset(&mut self, ctx: &mut ViewContext<Self>) {
        if matches!(self.cli_voice_input_state, CLIVoiceInputState::Stopped) {
            return;
        }

        if matches!(self.cli_voice_input_state, CLIVoiceInputState::Listening) {
            voice_input::VoiceInput::handle(ctx).update(ctx, |voice_input, _| {
                voice_input.abort_listening();
            });
        }

        if matches!(self.cli_voice_input_state, CLIVoiceInputState::Transcribing) {
            if let Some(handle) = self.cli_transcription_handle.take() {
                handle.abort();
            }

            voice_input::VoiceInput::handle(ctx).update(ctx, |voice, _| {
                voice.set_transcribing_active(false);
            });
        }

        self.cli_voice_input_state = CLIVoiceInputState::Stopped;
        self.cli_transcription_handle = None;
        self.update_cli_mic_button_state(ctx);
    }

    // ── CLI agent voice input (self-contained, bypasses editor) ──────

    /// Toggle voice input for CLI agent mode. Records audio and writes the
    /// transcription directly to the PTY, bypassing the editor voice flow.
    #[cfg(feature = "voice_input")]
    pub fn toggle_cli_voice_input(
        &mut self,
        source: &voice_input::VoiceInputToggledFrom,
        ctx: &mut ViewContext<Self>,
    ) {
        if !UserWorkspaces::as_ref(ctx).is_voice_enabled() {
            return;
        }

        if !AISettings::as_ref(ctx).is_voice_input_enabled(ctx) {
            return;
        }

        // For key-based toggling, validate the key state against current voice state.
        if let voice_input::VoiceInputToggledFrom::Key { state } = source {
            match (&self.cli_voice_input_state, state) {
                (CLIVoiceInputState::Stopped, warpui::event::KeyState::Released) => return,
                (CLIVoiceInputState::Listening, warpui::event::KeyState::Pressed) => return,
                _ => {}
            }
        }

        match &self.cli_voice_input_state {
            CLIVoiceInputState::Stopped => {
                if !crate::ai::AIRequestUsageModel::as_ref(ctx).can_request_voice() {
                    self.show_cli_voice_error_toast("Voice input limit reached", ctx);
                    return;
                }

                let session_result = voice_input::VoiceInput::handle(ctx)
                    .update(ctx, |voice_input, ctx| {
                        voice_input.start_listening(ctx, source.clone())
                    });

                match session_result {
                    Ok(session) => {
                        self.cli_voice_input_state = CLIVoiceInputState::Listening;
                        self.update_cli_mic_button_state(ctx);

                        if let Some(agent) = self.cli_agent(ctx) {
                            send_telemetry_from_ctx!(
                                TelemetryEvent::CLIAgentToolbarVoiceInputUsed {
                                    cli_agent: agent.into(),
                                },
                                ctx
                            );
                        }

                        if matches!(*source, voice_input::VoiceInputToggledFrom::Button) {
                            self.maybe_show_first_time_cli_voice_toast(ctx);
                        }

                        ctx.spawn(
                            async move { session.await_result().await },
                            Self::handle_cli_voice_session_result,
                        );
                    }
                    Err(StartListeningError::AccessDenied) => {
                        self.show_cli_microphone_access_toast(ctx);
                    }
                    Err(e) => {
                        log::error!("Failed to start CLI voice input: {e:?}");
                    }
                }
            }
            CLIVoiceInputState::Listening => {
                voice_input::VoiceInput::handle(ctx).update(ctx, |voice_input, ctx| {
                    if let Err(e) = voice_input.stop_listening(ctx) {
                        log::error!("Failed to stop CLI voice input: {e:?}");
                    }
                });
            }
            CLIVoiceInputState::Transcribing => {
                // Don't allow toggling while transcribing.
            }
        }
        ctx.notify();
    }

    #[cfg(feature = "voice_input")]
    fn handle_cli_voice_session_result(
        &mut self,
        result: VoiceSessionResult,
        ctx: &mut ViewContext<Self>,
    ) {
        use crate::editor::VoiceTranscriber;

        match result {
            VoiceSessionResult::Audio {
                wav_base64,
                session_duration_ms: _,
            } => {
                let voice_transcriber = VoiceTranscriber::as_ref(ctx);
                if let Some(transcriber) = voice_transcriber.transcriber() {
                    let transcriber = transcriber.clone();
                    self.cli_voice_input_state = CLIVoiceInputState::Transcribing;

                    voice_input::VoiceInput::handle(ctx).update(ctx, |voice, _| {
                        voice.set_transcribing_active(true);
                    });

                    self.cli_transcription_handle = Some(ctx.spawn(
                        async move { transcriber.transcribe(wav_base64).await },
                        Self::apply_cli_transcribed_voice_input,
                    ));
                } else {
                    self.cli_voice_input_state = CLIVoiceInputState::Stopped;
                }
            }
            VoiceSessionResult::Aborted { .. } => {
                self.cli_voice_input_state = CLIVoiceInputState::Stopped;
            }
        }
        self.update_cli_mic_button_state(ctx);
        ctx.notify();
    }

    #[cfg(feature = "voice_input")]
    fn apply_cli_transcribed_voice_input(
        &mut self,
        result: Result<String, TranscribeError>,
        ctx: &mut ViewContext<Self>,
    ) {
        voice_input::VoiceInput::handle(ctx).update(ctx, |voice, _| {
            voice.set_transcribing_active(false);
        });

        match result {
            Ok(transcribed_text) => {
                if !transcribed_text.is_empty() {
                    if self.has_active_cli_agent_input_session(ctx) {
                        ctx.emit(AgentInputFooterEvent::InsertIntoCLIRichInput(
                            transcribed_text,
                        ));
                    } else {
                        ctx.emit(AgentInputFooterEvent::WriteToPty(transcribed_text));
                    }
                }
            }
            Err(e) => match e {
                TranscribeError::QuotaLimit => {
                    self.show_cli_voice_error_toast("Voice input limit reached", ctx);
                }
                _ => {
                    log::error!("Failed to transcribe CLI voice input: {e:?}");
                    self.show_cli_voice_error_toast("Failed to transcribe voice input", ctx);
                }
            },
        }

        self.cli_voice_input_state = CLIVoiceInputState::Stopped;
        self.cli_transcription_handle = None;
        self.update_cli_mic_button_state(ctx);
        ctx.notify();
    }

    #[cfg(feature = "voice_input")]
    fn update_cli_mic_button_state(&self, ctx: &mut ViewContext<Self>) {
        let icon = match &self.cli_voice_input_state {
            CLIVoiceInputState::Stopped => Icon::Microphone,
            CLIVoiceInputState::Listening => Icon::Stop,
            CLIVoiceInputState::Transcribing => Icon::DotsHorizontal,
        };
        let is_transcribing =
            matches!(self.cli_voice_input_state, CLIVoiceInputState::Transcribing);

        self.mic_button.update(ctx, |button, ctx| {
            button.set_icon(Some(icon), ctx);
            button.set_active(is_transcribing, ctx);
        });
    }

    #[cfg(feature = "voice_input")]
    fn show_cli_voice_error_toast(&self, message: &str, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast = DismissibleToast::error(message.to_string());
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    #[cfg(feature = "voice_input")]
    fn show_cli_microphone_access_toast(&self, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
            let toast = DismissibleToast::error(String::from(
                "Failed to start voice input (you may need to enable Microphone access)",
            ));
            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
        });
    }

    #[cfg(feature = "voice_input")]
    fn maybe_show_first_time_cli_voice_toast(&self, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            if let Some(toggle_key) = settings.maybe_setup_first_time_voice(ctx) {
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = DismissibleToast::success(format!(
                        "Voice input is enabled. You can also press and hold the `{}` key to activate voice input (configure in Settings > AI > Voice)",
                        toggle_key.display_name()
                    ));
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
            }
        });
    }

    fn sync_fast_forward_button(&self, ctx: &mut ViewContext<Self>) {
        // Read directly from the conversation, same data source as the warping
        // indicator footer's auto-approve chip.
        let is_active = BlocklistAIHistoryModel::as_ref(ctx)
            .active_conversation(self.terminal_view_id)
            .map(|c| c.autoexecute_any_action())
            .unwrap_or(false);
        let icon = if is_active {
            Icon::FastForwardFilled
        } else {
            Icon::FastForward
        };
        let tooltip = if is_active {
            FAST_FORWARD_ON_TOOLTIP
        } else {
            FAST_FORWARD_OFF_TOOLTIP
        };
        self.fast_forward_button.update(ctx, |button, ctx| {
            button.set_icon(Some(icon), ctx);
            button.set_tooltip(Some(tooltip), ctx);
            button.set_active(is_active, ctx);
        });
    }

    /// Disable the start-remote-control chip and swap its tooltip when the
    /// user is anonymous or logged out, since session sharing requires a
    /// real account.
    fn sync_remote_control_button(&self, ctx: &mut ViewContext<Self>) {
        let login_required = AuthStateProvider::as_ref(ctx)
            .get()
            .is_anonymous_or_logged_out();
        let tooltip = if login_required {
            START_REMOTE_CONTROL_LOGIN_REQUIRED_TOOLTIP
        } else {
            START_REMOTE_CONTROL_TOOLTIP
        };
        self.start_remote_control_button.update(ctx, |button, ctx| {
            button.set_disabled(login_required, ctx);
            button.set_tooltip(Some(tooltip), ctx);
        });
    }

    fn update_context_window_button(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(conversation) =
            BlocklistAIHistoryModel::as_ref(ctx).active_conversation(self.terminal_view_id)
        {
            let usage = conversation.context_window_usage();
            let icon = icon_for_context_window_usage(usage);
            let remaining_pct = ((1.0 - usage) * 100.0).round() as i32;
            let tooltip = format!("{remaining_pct}% context remaining");

            self.context_window_button.update(ctx, |button, ctx| {
                button.set_icon(Some(icon), ctx);
                button.set_tooltip(Some(tooltip), ctx);
            });
        }
    }

    fn render_toolbar_item(
        &self,
        item: &AgentToolbarItemKind,
        shared_status: &SharedSessionStatus,
        is_cloud_context: bool,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let is_cloud_mode = FeatureFlag::CloudModeImageContext.is_enabled()
            && self
                .ambient_agent_view_model
                .as_ref()
                .is_some_and(|ambient_agent_model| {
                    ambient_agent_model.as_ref(app).is_ambient_agent()
                });
        if !item.available_in().is_available_for_agent_view()
            || !item.available_to_session_viewer(shared_status, is_cloud_mode)
        {
            return None;
        }

        if self.handoff_compose_state.as_ref(app).is_active()
            && !item.is_available_during_handoff_compose()
        {
            return None;
        }

        match item {
            AgentToolbarItemKind::ContextChip(chip_kind) => {
                let chips = match SessionSettings::as_ref(app)
                    .agent_footer_chip_selection
                    .left_chips()
                    .contains(chip_kind)
                {
                    true => &self.left_display_chips,
                    false => &self.right_display_chips,
                };
                chips
                    .iter()
                    .find(|chip| chip.as_ref(app).chip_kind() == chip_kind)
                    .filter(|chip| chip.as_ref(app).should_render(app))
                    .map(|chip| ChildView::new(chip).finish())
            }
            AgentToolbarItemKind::ModelSelector => {
                let show = FeatureFlag::ProfilesDesignRevamp.is_enabled()
                    || *SessionSettings::as_ref(app).show_model_selectors_in_prompt;
                show.then(|| ChildView::new(&self.model_selector).finish())
            }
            AgentToolbarItemKind::NLDToggle => Some(ChildView::new(&self.nld_button).finish()),
            AgentToolbarItemKind::VoiceInput => {
                #[cfg(feature = "voice_input")]
                {
                    let enabled =
                        crate::settings::AISettings::as_ref(app).is_voice_input_enabled(app);
                    enabled.then(|| ChildView::new(&self.mic_button).finish())
                }
                #[cfg(not(feature = "voice_input"))]
                None
            }
            AgentToolbarItemKind::FileAttach => Some(ChildView::new(&self.file_button).finish()),
            AgentToolbarItemKind::ContextWindowUsage => {
                let has_conversation = FeatureFlag::ContextWindowUsageV2.is_enabled()
                    && BlocklistAIHistoryModel::as_ref(app)
                        .active_conversation(self.terminal_view_id)
                        .is_some();
                has_conversation.then(|| ChildView::new(&self.context_window_button).finish())
            }
            AgentToolbarItemKind::ShareSession => {
                let enabled = FeatureFlag::CreatingSharedSessions.is_enabled()
                    && FeatureFlag::HOARemoteControl.is_enabled()
                    && ContextFlag::CreateSharedSession.is_enabled();
                if !enabled {
                    return None;
                }
                let button = if shared_status.is_sharer() {
                    &self.stop_remote_control_button
                } else {
                    &self.start_remote_control_button
                };
                Some(ChildView::new(button).finish())
            }
            AgentToolbarItemKind::FastForwardToggle => FeatureFlag::FastForwardAutoexecuteButton
                .is_enabled()
                .then(|| ChildView::new(&self.fast_forward_button).finish()),
            AgentToolbarItemKind::HandoffToCloud => {
                if !AISettings::as_ref(app)
                    .is_cloud_handoff_enabled_for_terminal_view(self.terminal_view_id, app)
                    || is_cloud_context
                {
                    return None;
                }

                Some(ChildView::new(&self.handoff_to_cloud_button).finish())
            }
            // Handled by the available_in() guard above; included for exhaustiveness.
            AgentToolbarItemKind::FileExplorer
            | AgentToolbarItemKind::RichInput
            | AgentToolbarItemKind::Settings => None,
        }
    }

    #[cfg(test)]
    pub fn displayed_chip_kinds(
        &self,
        app: &AppContext,
    ) -> (
        Vec<crate::context_chips::ContextChipKind>,
        Vec<crate::context_chips::ContextChipKind>,
    ) {
        let collect_chip_kinds = |chips: &[ViewHandle<DisplayChip>]| {
            chips
                .iter()
                .map(|chip| chip.as_ref(app).chip_kind().clone())
                .collect()
        };

        (
            collect_chip_kinds(&self.left_display_chips),
            collect_chip_kinds(&self.right_display_chips),
        )
    }

    #[cfg(test)]
    pub fn cli_display_chip_kinds(
        &self,
        app: &AppContext,
    ) -> Vec<crate::context_chips::ContextChipKind> {
        self.cli_display_chips
            .iter()
            .map(|chip| chip.as_ref(app).chip_kind().clone())
            .collect()
    }
}

impl View for AgentInputFooter {
    fn ui_name() -> &'static str {
        "AgentViewFooter"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        if self.should_render_cloud_mode_v2(app) {
            return self.render_cloud_mode_v2_footer(app);
        }
        // When a CLI agent session is active, render the CLI agent toolbar instead.
        if self.is_cli_agent_session_active(app) {
            return self.render_cli_mode_footer(app);
        }

        let session_settings = SessionSettings::as_ref(app);
        let left_items = session_settings.agent_footer_chip_selection.left_items();
        let right_items = session_settings.agent_footer_chip_selection.right_items();

        let mut left_buttons = Wrap::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_run_spacing(4.)
            .with_spacing(4.);

        let is_ambient_agent = FeatureFlag::CloudMode.is_enabled()
            && self
                .ambient_agent_view_model
                .as_ref()
                .is_some_and(|ambient_agent_model| {
                    ambient_agent_model.as_ref(app).is_ambient_agent()
                });
        if is_ambient_agent {
            if let Some(environment_selector) = self.environment_selector.as_ref() {
                left_buttons =
                    left_buttons.with_child(ChildView::new(environment_selector).finish());
            }
        } else if self.handoff_compose_state.as_ref(app).is_active() {
            left_buttons = left_buttons
                .with_child(ChildView::new(&self.handoff_environment_selector).finish());
        }

        let terminal_model = self.terminal_model.lock();
        let shared_status = terminal_model.shared_session_status();
        let is_cloud_context = super::is_in_cloud_context(
            terminal_model.block_list().agent_view_state(),
            &terminal_model,
        );

        for item in &left_items {
            if let Some(element) =
                self.render_toolbar_item(item, shared_status, is_cloud_context, app)
            {
                left_buttons.add_child(element);
            }
        }

        let mut right_buttons = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(4.);

        let has_prompt_alert = !self.prompt_alert.as_ref(app).is_no_alert();
        if has_prompt_alert {
            right_buttons.add_child(
                Shrinkable::new(
                    1.,
                    Clipped::new(ChildView::new(&self.prompt_alert).finish()).finish(),
                )
                .finish(),
            );
        } else {
            for item in &right_items {
                if let Some(element) =
                    self.render_toolbar_item(item, shared_status, is_cloud_context, app)
                {
                    right_buttons.add_child(element);
                }
            }
        }

        let content = Wrap::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(WrapFill::new(0., left_buttons.finish()).finish())
            .with_child(WrapFill::new(0., right_buttons.finish()).finish())
            .with_run_spacing(context_chips::spacing::UDI_ROW_RUN_SPACING)
            .finish();
        let content = EventHandler::new(content)
            .on_right_mouse_down(|ctx, _, position| {
                ctx.dispatch_typed_action(AgentInputFooterAction::ShowContextMenu { position });
                DispatchEventResult::StopPropagation
            })
            .finish();

        let mut container = Container::new(content).with_padding_bottom(8.0);
        if !has_prompt_alert {
            container = container.with_padding_right(16.);
        }

        // If the model chip has switched to show the ftu model options
        // (and this is the first time this has happened)
        // we show a little callout explaining the change.
        let showing_ftu_model_picker = FeatureFlag::InlineMenuHeaders.is_enabled()
            && terminal_model
                .block_list()
                .active_block()
                .is_agent_in_control_or_tagged_in();
        if showing_ftu_model_picker && self.render_ftu_callout {
            let mut stack = Stack::new();
            stack.add_child(container.finish());
            stack.add_positioned_overlay_child(
                render_ftu_callout(&self.ftu_callout_close_button, app),
                OffsetPositioning::offset_from_save_position_element(
                    "profile_model_selector_model_button",
                    vec2f(8., -8.),
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::TopRight,
                    ChildAnchor::BottomRight,
                ),
            );
            stack.finish()
        } else {
            container.finish()
        }
    }
}

/// Render a message bubble calling out that the model has switched now that we're in FTU mode.
/// This callout is dismissable and does not re-appear once you've dismissed it once.
fn render_ftu_callout(
    close_button: &ViewHandle<ActionButton>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let background = theme.background().blend(&theme.accent().with_opacity(50));
    let text_color = internal_colors::text_main(theme, background.into_solid());

    let callout_box = ConstrainedBox::new(
        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(8.)
                .with_child(
                    Expanded::new(
                        1.,
                        Text::new(
                            "Now using Full Terminal Agent's default model.",
                            appearance.ui_font_family(),
                            appearance.monospace_font_size() - 2.,
                        )
                        .with_color(text_color)
                        .with_line_height_ratio(DEFAULT_UI_LINE_HEIGHT_RATIO)
                        .with_selectable(false)
                        .finish(),
                    )
                    .finish(),
                )
                .with_child(
                    Container::new(ChildView::new(close_button).finish())
                        .with_margin_top(-3.)
                        .finish(),
                )
                .finish(),
        )
        .with_vertical_padding(12.)
        .with_horizontal_padding(16.)
        .with_background(background)
        .with_border(Border::all(1.).with_border_fill(theme.accent()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .finish(),
    )
    .with_width(348.)
    .finish();

    // The way that we render the little triangle in the bottom of the message bubble
    // is by rendering two triangle icons (a filled triangle and an outlined triangle) and then
    // stacking them on top of each other below the message bubble. I don't think there's a simpler
    // way to do this with our UI framework.
    let triangle_stack = Stack::new()
        .with_child(
            ConstrainedBox::new(
                Icon::CalloutTriangleBorderDown
                    .to_warpui_icon(Fill::Solid(theme.accent().into_solid()))
                    .finish(),
            )
            .with_width(24.)
            .with_height(24.)
            .finish(),
        )
        .with_child(
            ConstrainedBox::new(
                Icon::CalloutTriangleFillDown
                    .to_warpui_icon(background)
                    .finish(),
            )
            .with_width(24.)
            .with_height(24.)
            .finish(),
        );

    Flex::column()
        .with_main_axis_size(MainAxisSize::Min)
        .with_child(callout_box)
        .with_child(
            Container::new(triangle_stack.finish())
                .with_margin_left(300.)
                .with_margin_top(-3.)
                .finish(),
        )
        .finish()
}

#[derive(Debug, Clone)]
pub enum AgentInputFooterAction {
    #[cfg(feature = "voice_input")]
    ToggleVoiceInput,
    SelectFile,
    InsertFilePath(String),
    ToggleCodeReview,
    ToggleFileExplorer,
    ToggleRichInput,
    ToggleAutodetectionSetting,
    DismissFtuModelCallout,
    InstallPlugin,
    UpdatePlugin,
    OpenPluginInstallInstructionsPane,
    OpenPluginUpdateInstructionsPane,
    DismissPluginChip,
    StartRemoteControl,
    StopRemoteControl,
    OpenCodingAgentSettings,
    /// Open the local-to-cloud handoff pane. Dispatched by the
    /// "Hand off to cloud" footer chip.
    OpenHandoffPane,
    ShowContextMenu {
        position: Vector2F,
    },
}

impl TypedActionView for AgentInputFooter {
    type Action = AgentInputFooterAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut warpui::ViewContext<Self>) {
        match action {
            #[cfg(feature = "voice_input")]
            AgentInputFooterAction::ToggleVoiceInput => {
                // In CLI agent mode, handle voice recording/transcription
                // directly so text is written to the PTY instead of the editor.
                if self.is_cli_agent_session_active(ctx) {
                    self.toggle_cli_voice_input(&voice_input::VoiceInputToggledFrom::Button, ctx);
                } else {
                    ctx.emit(AgentInputFooterEvent::ToggleVoiceInput(
                        voice_input::VoiceInputToggledFrom::Button,
                    ));
                }
            }
            AgentInputFooterAction::SelectFile => {
                // Fork based on CLI agent session: in CLI mode, open a file
                // picker and insert/write the path; in normal mode, use the
                // standard AI file attachment flow.
                if self.is_cli_agent_session_active(ctx) {
                    self.select_cli_file(ctx);
                } else {
                    ctx.emit(AgentInputFooterEvent::SelectFile);
                }
            }
            AgentInputFooterAction::InsertFilePath(path) => {
                if let Some(agent) = self.cli_agent(ctx) {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::CLIAgentToolbarImageAttached {
                            cli_agent: agent.into(),
                        },
                        ctx
                    );
                }
                let path_with_space = format!("{path} ");
                if self.has_active_cli_agent_input_session(ctx) {
                    ctx.emit(AgentInputFooterEvent::InsertIntoCLIRichInput(
                        path_with_space,
                    ));
                } else {
                    ctx.emit(AgentInputFooterEvent::WriteToPty(path_with_space));
                }
            }
            AgentInputFooterAction::ToggleCodeReview => {
                if let Some(agent) = self.cli_agent(ctx) {
                    ctx.emit(AgentInputFooterEvent::ToggleCodeReviewPane(agent));
                }
            }
            AgentInputFooterAction::ToggleFileExplorer => {
                if let Some(agent) = self.cli_agent(ctx) {
                    ctx.emit(AgentInputFooterEvent::ToggleFileExplorer(agent));
                }
            }
            AgentInputFooterAction::ToggleRichInput => {
                if self.has_active_cli_agent_input_session(ctx) {
                    ctx.emit(AgentInputFooterEvent::HideRichInput);
                } else {
                    ctx.emit(AgentInputFooterEvent::OpenRichInput);
                }
            }
            AgentInputFooterAction::ToggleAutodetectionSetting => {
                let ai_settings = AISettings::handle(ctx);
                ai_settings.update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .ai_autodetection_enabled_internal
                        .toggle_and_save_value(ctx));
                });
            }
            AgentInputFooterAction::DismissFtuModelCallout => {
                if self.render_ftu_callout {
                    self.render_ftu_callout = false;
                    ctx.notify();
                }
            }
            AgentInputFooterAction::InstallPlugin => {
                #[cfg(not(target_family = "wasm"))]
                {
                    if let Some(agent) = self.cli_agent(ctx) {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::CLIAgentPluginChipClicked {
                                cli_agent: agent.into(),
                                action: PluginChipTelemetryAction::Install,
                            },
                            ctx
                        );
                    }
                    if !self.handle_install_plugin(ctx) {
                        self.record_plugin_auto_failure_and_notify(ctx);
                    }
                }
            }
            AgentInputFooterAction::UpdatePlugin => {
                #[cfg(not(target_family = "wasm"))]
                {
                    if let Some(agent) = self.cli_agent(ctx) {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::CLIAgentPluginChipClicked {
                                cli_agent: agent.into(),
                                action: PluginChipTelemetryAction::Update,
                            },
                            ctx
                        );
                    }
                    if !self.handle_update_plugin(ctx) {
                        self.record_plugin_auto_failure_and_notify(ctx);
                    }
                }
            }
            AgentInputFooterAction::OpenPluginInstallInstructionsPane => {
                #[cfg(not(target_family = "wasm"))]
                if let Some(agent) = self.cli_agent(ctx) {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::CLIAgentPluginChipClicked {
                            cli_agent: agent.into(),
                            action: PluginChipTelemetryAction::InstallInstructions,
                        },
                        ctx
                    );
                    ctx.emit(AgentInputFooterEvent::OpenPluginInstructionsPane(
                        agent,
                        PluginModalKind::Install,
                    ));
                }
            }
            AgentInputFooterAction::OpenPluginUpdateInstructionsPane => {
                #[cfg(not(target_family = "wasm"))]
                if let Some(agent) = self.cli_agent(ctx) {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::CLIAgentPluginChipClicked {
                            cli_agent: agent.into(),
                            action: PluginChipTelemetryAction::UpdateInstructions,
                        },
                        ctx
                    );
                    ctx.emit(AgentInputFooterEvent::OpenPluginInstructionsPane(
                        agent,
                        PluginModalKind::Update,
                    ));
                }
            }
            AgentInputFooterAction::DismissPluginChip => {
                let chip_kind = self.plugin_chip_kind(ctx);
                let is_update = matches!(chip_kind, Some(PluginChipKind::Update));
                if let Some(agent) = self.cli_agent(ctx) {
                    if let Some(kind) = chip_kind {
                        send_telemetry_from_ctx!(
                            TelemetryEvent::CLIAgentPluginChipDismissed {
                                cli_agent: agent.into(),
                                chip_kind: kind.into(),
                            },
                            ctx
                        );
                    }
                }
                let session = CLIAgentSessionsModel::as_ref(ctx)
                    .session(self.terminal_view_id)
                    .cloned();
                if let Some(session) = session {
                    let chip_key =
                        plugin_chip_key(session.agent.command_prefix(), &session.remote_host);
                    if is_update {
                        #[cfg(not(target_family = "wasm"))]
                        if let Some(manager) = plugin_manager_for(session.agent) {
                            let version = manager.minimum_plugin_version().to_owned();
                            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                                settings.dismiss_plugin_update_chip(&chip_key, version, ctx);
                            });
                        }
                    } else {
                        AISettings::handle(ctx).update(ctx, |settings, ctx| {
                            settings.dismiss_plugin_install_chip(&chip_key, ctx);
                        });
                    }
                }
                ctx.notify();
            }
            AgentInputFooterAction::StartRemoteControl => {
                ctx.emit(AgentInputFooterEvent::StartRemoteControl);
            }
            AgentInputFooterAction::StopRemoteControl => {
                ctx.emit(AgentInputFooterEvent::StopRemoteControl);
            }
            AgentInputFooterAction::OpenCodingAgentSettings => {
                #[cfg(not(target_family = "wasm"))]
                ctx.dispatch_typed_action_deferred(WorkspaceAction::ScrollToSettingsWidget {
                    page: SettingsSection::ThirdPartyCLIAgents,
                    widget_id: crate::settings_view::cli_agent_settings_widget_id(),
                });
            }
            AgentInputFooterAction::OpenHandoffPane => {
                if FeatureFlag::OzHandoff.is_enabled()
                    && FeatureFlag::HandoffLocalCloud.is_enabled()
                    && cfg!(all(feature = "local_fs", not(target_family = "wasm")))
                {
                    ctx.emit(AgentInputFooterEvent::OpenHandoffPane);
                }
            }
            AgentInputFooterAction::ShowContextMenu { position } => {
                ctx.emit(AgentInputFooterEvent::ShowContextMenu {
                    position: *position,
                });
            }
        }
    }
}

pub enum AgentInputFooterEvent {
    #[cfg(feature = "voice_input")]
    ToggleVoiceInput(voice_input::VoiceInputToggledFrom),
    SelectFile,
    WriteToPty(String),
    /// Insert text into the CLI agent rich input.
    InsertIntoCLIRichInput(String),
    ToggleCodeReviewPane(CLIAgent),
    ToggleFileExplorer(CLIAgent),
    StartRemoteControl,
    StopRemoteControl,
    OpenRichInput,
    HideRichInput,
    ToggledChipMenu {
        open: bool,
    },
    TryExecuteChipCommand(String),
    PromptAlert(PromptAlertEvent),
    ModelSelectorOpened,
    ModelSelectorClosed,
    EnvironmentSelectorClosed,
    ToggleInlineModelSelector {
        initial_tab: InlineModelSelectorTab,
    },
    OpenSettings(SettingsSection),
    OpenCodeReview,
    OpenAIDocument {
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
    },
    ShowContextMenu {
        position: Vector2F,
    },
    OpenEnvironmentManagementPane,
    PluginInstalled(CLIAgent),
    #[cfg(not(target_family = "wasm"))]
    OpenPluginInstructionsPane(CLIAgent, PluginModalKind),
    /// Local-to-cloud handoff chip clicked. The terminal `Input` subscriber
    /// activates `&` handoff-compose mode on the local input.
    OpenHandoffPane,
}

impl Entity for AgentInputFooter {
    type Event = AgentInputFooterEvent;
}

pub(crate) struct AgentInputButtonTheme;

impl ActionButtonTheme for AgentInputButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        // Solid surface fills keep the button readable even when its parent
        // isn't `theme.background()` (for example, over an alt-screen CLI agent).
        let theme = appearance.theme();
        Some(if hovered {
            theme.surface_2()
        } else {
            theme.surface_1()
        })
    }

    fn text_color(
        &self,
        _hovered: bool,
        background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        // If a caller overrides `background()` with a translucent fill, blend
        // it over `surface_1` so text contrast is computed against the actual
        // rendered color rather than the raw overlay.
        let base_bg = appearance.theme().surface_1();
        let effective_bg = background
            .map(|overlay| base_bg.blend(&overlay))
            .unwrap_or(base_bg);

        appearance.theme().sub_text_color(effective_bg).into_solid()
    }

    fn border(&self, appearance: &Appearance) -> Option<ColorU> {
        Some(internal_colors::neutral_3(appearance.theme()))
    }

    fn should_opt_out_of_contrast_adjustment(&self) -> bool {
        true
    }

    fn font_properties(&self) -> Option<warpui::fonts::Properties> {
        if crate::features::FeatureFlag::CloudModeInputV2.is_enabled() {
            Some(warpui::fonts::Properties {
                weight: warpui::fonts::Weight::Semibold,
                ..Default::default()
            })
        } else {
            None
        }
    }
}

struct RemoteControlButtonTheme;

impl ActionButtonTheme for RemoteControlButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        AgentInputButtonTheme.background(hovered, appearance)
    }

    fn text_color(
        &self,
        hovered: bool,
        background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        AgentInputButtonTheme.text_color(hovered, background, appearance)
    }

    fn border(&self, appearance: &Appearance) -> Option<ColorU> {
        AgentInputButtonTheme.border(appearance)
    }

    fn should_opt_out_of_contrast_adjustment(&self) -> bool {
        AgentInputButtonTheme.should_opt_out_of_contrast_adjustment()
    }
}

/// Theme for the mic button.
/// Uses a blue icon when active (hovered, listening, or transcribing).
pub(crate) struct ActiveMicButtonTheme;

impl ActionButtonTheme for ActiveMicButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        AgentInputButtonTheme.background(hovered, appearance)
    }

    fn text_color(
        &self,
        hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        if hovered {
            appearance.theme().ansi_fg_blue()
        } else {
            appearance
                .theme()
                .sub_text_color(appearance.theme().surface_1())
                .into_solid()
        }
    }

    fn border(&self, appearance: &Appearance) -> Option<ColorU> {
        AgentInputButtonTheme.border(appearance)
    }

    fn should_opt_out_of_contrast_adjustment(&self) -> bool {
        true
    }

    fn font_properties(&self) -> Option<warpui::fonts::Properties> {
        AgentInputButtonTheme.font_properties()
    }
}

/// Green-accented theme for the "Install Warp plugin" chip.
struct InstallPluginButtonTheme;

impl ActionButtonTheme for InstallPluginButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        let green = appearance.theme().ansi_fg_green();
        let base = appearance.theme().surface_1();
        Some(if hovered {
            base.blend(&Fill::Solid(green).with_opacity(30))
        } else {
            base.blend(&Fill::Solid(green).with_opacity(15))
        })
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        appearance.theme().ansi_fg_green()
    }

    fn border(&self, appearance: &Appearance) -> Option<ColorU> {
        let green = appearance.theme().ansi_fg_green();
        Some(ColorU::new(green.r, green.g, green.b, 80))
    }

    fn should_opt_out_of_contrast_adjustment(&self) -> bool {
        true
    }
}

/// Writes the detailed plugin installation log to a temp file.
/// Returns the log file path on success, or `None` if writing failed.
#[cfg(not(target_family = "wasm"))]
async fn write_install_log(agent: CLIAgent, err: &PluginInstallError) -> Option<PathBuf> {
    let log_path = env::temp_dir().join("warp-plugin-install.log");
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
    let contents = format!(
        "Warp plugin installation — {agent:?}\n\
         {now}\n\
         \n\
         {log}",
        log = err.log,
    );
    fs::write(&log_path, contents).await.ok()?;
    Some(log_path)
}

/// Keeps the auto-approve chip's muted text semantics while using the shared opaque chip fill.
struct FastForwardButtonTheme;

impl ActionButtonTheme for FastForwardButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        AgentInputButtonTheme.background(hovered, appearance)
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        appearance
            .theme()
            .sub_text_color(appearance.theme().surface_1())
            .into_solid()
    }

    fn border(&self, appearance: &Appearance) -> Option<ColorU> {
        AgentInputButtonTheme.border(appearance)
    }

    fn should_opt_out_of_contrast_adjustment(&self) -> bool {
        true
    }
}

/// Same as `AgentInputButtonTheme`, except with one-off special active styling for the NLD button.
struct NLDButtonTheme;

impl ActionButtonTheme for NLDButtonTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        AgentInputButtonTheme.background(hovered, appearance)
    }

    fn text_color(
        &self,
        hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> ColorU {
        if hovered {
            appearance.theme().ansi_fg_blue()
        } else {
            appearance
                .theme()
                .disabled_text_color(appearance.theme().surface_1())
                .into_solid()
        }
    }

    fn border(&self, appearance: &Appearance) -> Option<ColorU> {
        AgentInputButtonTheme.border(appearance)
    }

    fn should_opt_out_of_contrast_adjustment(&self) -> bool {
        true
    }
}
