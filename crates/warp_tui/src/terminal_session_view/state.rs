#[cfg(test)]
use std::rc::Rc;
use std::sync::{Arc, Weak};
use std::{error, fmt};

use parking_lot::FairMutex;
use warp::tui_export::{BlocklistAIInputModel, CLISubagentController, TerminalModel};
use warpui_core::keymap::Context;
use warpui_core::{AppContext, Entity, ModelHandle, ViewHandle, WeakModelHandle, WeakViewHandle};

use super::{AUTO_APPROVE_TOGGLE_BINDING_NAME, BlockingInputSource};
use crate::input_mode_policy;
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};
use crate::keybindings::{PLAN_TOGGLE_BINDING_NAME, binding_hint};
use crate::tab_bar::TuiTabBarView;
use crate::terminal_use::{TuiInputTarget, inline_process_owns_input, tui_input_target};
use crate::transcript_view::TuiTranscriptView;
use crate::tui_cli_subagent_view::{HAND_BACK_KEY_BINDING, TAKE_CONTROL_KEY_BINDING};

const ASK_AGENT_HINT: &str = "Ask the agent anything";
const ORCHESTRATION_HINT: &str = "Shift + ↑ for other agents";
const SHORTCUTS_HINT: &str = "? for shortcuts";
const SHELL_MODE_HINT: &str = "! for shell mode";
const COMMANDS_HINT: &str = "/ for commands";
const CONVERSATIONS_HINT: &str = "← for conversations";
const HINT_SEPARATOR: &str = " • ";
pub(crate) const SHELL_HINT: &str = "Run a shell command • ? for shortcuts • esc for agent mode";
enum TuiTerminalSessionStateSource {
    Session {
        terminal_model: Weak<FairMutex<TerminalModel>>,
        cli_subagent_controller: WeakModelHandle<CLISubagentController>,
        transcript: WeakViewHandle<TuiTranscriptView>,
        input_mode: WeakModelHandle<BlocklistAIInputModel>,
        suggestions_mode: WeakModelHandle<TuiInputSuggestionsModeModel>,
        orchestration_tab_bar: WeakViewHandle<TuiTabBarView>,
    },
    #[cfg(test)]
    InputTest {
        input_mode: WeakModelHandle<BlocklistAIInputModel>,
        suggestions_mode: WeakModelHandle<TuiInputSuggestionsModeModel>,
        orchestration_tabs_available: Rc<dyn Fn(&AppContext) -> bool>,
    },
}

/// Persistent session-owned model that resolves a live state snapshot.
///
/// The source entities remain authoritative and are held weakly to avoid
/// extending their lifetimes. Resolving on demand avoids a cached derivative
/// that could become stale while still giving the session, input, and other
/// presentation components one shared state source.
pub(crate) struct TuiTerminalSessionStateModel {
    source: TuiTerminalSessionStateSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TuiTerminalSessionStateResolveError {
    TerminalModel,
    CliSubagentController,
    Transcript,
    InputMode,
    SuggestionsMode,
    OrchestrationTabBar,
}

impl fmt::Display for TuiTerminalSessionStateResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TerminalModel => "terminal model is unavailable",
            Self::CliSubagentController => "CLI subagent controller is unavailable",
            Self::Transcript => "transcript view is unavailable",
            Self::InputMode => "input-mode model is unavailable",
            Self::SuggestionsMode => "suggestions-mode model is unavailable",
            Self::OrchestrationTabBar => "orchestration tab bar is unavailable",
        })
    }
}

impl error::Error for TuiTerminalSessionStateResolveError {}

fn upgrade_terminal_model(
    terminal_model: &Weak<FairMutex<TerminalModel>>,
) -> Result<Arc<FairMutex<TerminalModel>>, TuiTerminalSessionStateResolveError> {
    terminal_model
        .upgrade()
        .ok_or(TuiTerminalSessionStateResolveError::TerminalModel)
}

impl Entity for TuiTerminalSessionStateModel {
    type Event = ();
}

impl TuiTerminalSessionStateModel {
    pub(crate) fn new(
        terminal_model: &Arc<FairMutex<TerminalModel>>,
        cli_subagent_controller: &ModelHandle<CLISubagentController>,
        transcript: &ViewHandle<TuiTranscriptView>,
        input_mode: &ModelHandle<BlocklistAIInputModel>,
        suggestions_mode: &ModelHandle<TuiInputSuggestionsModeModel>,
        orchestration_tab_bar: &ViewHandle<TuiTabBarView>,
    ) -> Self {
        Self {
            source: TuiTerminalSessionStateSource::Session {
                terminal_model: Arc::downgrade(terminal_model),
                cli_subagent_controller: cli_subagent_controller.downgrade(),
                transcript: transcript.downgrade(),
                input_mode: input_mode.downgrade(),
                suggestions_mode: suggestions_mode.downgrade(),
                orchestration_tab_bar: orchestration_tab_bar.downgrade(),
            },
        }
    }
    #[cfg(test)]
    pub(crate) fn new_for_input(
        input_mode: &ModelHandle<BlocklistAIInputModel>,
        suggestions_mode: &ModelHandle<TuiInputSuggestionsModeModel>,
        orchestration_tabs_available: impl Fn(&AppContext) -> bool + 'static,
    ) -> Self {
        Self {
            source: TuiTerminalSessionStateSource::InputTest {
                input_mode: input_mode.downgrade(),
                suggestions_mode: suggestions_mode.downgrade(),
                orchestration_tabs_available: Rc::new(orchestration_tabs_available),
            },
        }
    }

    pub(crate) fn resolve(
        &self,
        ctx: &AppContext,
    ) -> Result<TuiTerminalSessionState, TuiTerminalSessionStateResolveError> {
        match &self.source {
            TuiTerminalSessionStateSource::Session {
                terminal_model,
                cli_subagent_controller,
                transcript,
                input_mode,
                suggestions_mode,
                orchestration_tab_bar,
            } => {
                let terminal_model = upgrade_terminal_model(terminal_model)?;
                let cli_subagent_controller = cli_subagent_controller
                    .upgrade(ctx)
                    .ok_or(TuiTerminalSessionStateResolveError::CliSubagentController)?;
                let transcript = transcript
                    .upgrade(ctx)
                    .ok_or(TuiTerminalSessionStateResolveError::Transcript)?;
                let input_mode = input_mode
                    .upgrade(ctx)
                    .ok_or(TuiTerminalSessionStateResolveError::InputMode)?;
                let suggestions_mode = suggestions_mode
                    .upgrade(ctx)
                    .ok_or(TuiTerminalSessionStateResolveError::SuggestionsMode)?;
                let orchestration_tab_bar = orchestration_tab_bar
                    .upgrade(ctx)
                    .ok_or(TuiTerminalSessionStateResolveError::OrchestrationTabBar)?;
                let (alt_screen_active, input_target, user_owns_running_command) = {
                    let terminal_model = terminal_model.lock();
                    (
                        terminal_model.is_alt_screen_active(),
                        tui_input_target(&terminal_model),
                        inline_process_owns_input(&terminal_model),
                    )
                };
                let terminal_use_control = cli_subagent_controller
                    .as_ref(ctx)
                    .active_target()
                    .map(|target| target.control_state);
                let interaction = if let Some(source) =
                    transcript.as_ref(ctx).active_blocking_input_source(ctx)
                {
                    TuiInteractionState::Blocking(source)
                } else if terminal_use_control
                    .as_ref()
                    .is_some_and(|control| control.is_user_in_control())
                {
                    TuiInteractionState::Pty(TuiPtyState::UserControlledTerminalUse)
                } else if user_owns_running_command {
                    TuiInteractionState::Blocking(BlockingInputSource::LongRunningCommand)
                } else {
                    match input_target {
                        TuiInputTarget::Disabled => TuiInteractionState::StartingShell,
                        TuiInputTarget::Pty => TuiInteractionState::Pty(TuiPtyState::Process),
                        TuiInputTarget::AgentEditor => {
                            let mode = if terminal_use_control
                                .as_ref()
                                .is_some_and(|control| control.is_agent_in_control())
                            {
                                TuiComposerMode::Agent {
                                    agent_controlled_terminal_use: true,
                                }
                            } else if input_mode_policy::is_shell_mode(input_mode.as_ref(ctx)) {
                                TuiComposerMode::Shell
                            } else {
                                TuiComposerMode::Agent {
                                    agent_controlled_terminal_use: false,
                                }
                            };
                            TuiInteractionState::Composer(TuiComposerState {
                                mode,
                                suggestions_mode: suggestions_mode.as_ref(ctx).mode(),
                            })
                        }
                    }
                };
                let state = TuiBlockSessionState {
                    interaction,
                    transcript_is_empty: transcript.as_ref(ctx).is_empty(),
                    orchestration_available: orchestration_tab_bar.as_ref(ctx).has_tabs(),
                    plan_available: transcript.as_ref(ctx).has_toggleable_plan(ctx),
                };
                Ok(if alt_screen_active {
                    TuiTerminalSessionState::AltScreen {
                        input_target,
                        state,
                    }
                } else {
                    TuiTerminalSessionState::Block(state)
                })
            }
            #[cfg(test)]
            TuiTerminalSessionStateSource::InputTest {
                input_mode,
                suggestions_mode,
                orchestration_tabs_available,
            } => {
                let input_mode = input_mode
                    .upgrade(ctx)
                    .ok_or(TuiTerminalSessionStateResolveError::InputMode)?;
                let suggestions_mode = suggestions_mode
                    .upgrade(ctx)
                    .ok_or(TuiTerminalSessionStateResolveError::SuggestionsMode)?;
                Ok(TuiTerminalSessionState::for_input(
                    input_mode_policy::is_shell_mode(input_mode.as_ref(ctx)),
                    suggestions_mode.as_ref(ctx).mode(),
                    true,
                    orchestration_tabs_available(ctx),
                ))
            }
        }
    }
}

/// The terminal surface plus its current interaction projection.
///
/// Alternate-screen commands can still expose an agent composer beneath the
/// terminal, so the surface and interaction state are represented separately.
#[derive(Clone)]
pub(crate) enum TuiTerminalSessionState {
    AltScreen {
        input_target: TuiInputTarget,
        state: TuiBlockSessionState,
    },
    Block(TuiBlockSessionState),
}

/// State available only while the block UI is the active surface.
///
/// `interaction` is exclusive, while orchestration and plan availability are
/// additive capabilities that may contribute shortcuts to a composer.
#[derive(Clone)]
pub(crate) struct TuiBlockSessionState {
    pub(super) interaction: TuiInteractionState,
    pub(super) transcript_is_empty: bool,
    pub(super) orchestration_available: bool,
    pub(super) plan_available: bool,
}

/// The single interaction that currently owns the block UI's input area.
#[derive(Clone)]
pub(super) enum TuiInteractionState {
    Blocking(BlockingInputSource),
    StartingShell,
    Composer(TuiComposerState),
    Pty(TuiPtyState),
}

/// Composer state, which cannot exist under alt-screen, blocking, or PTY input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TuiComposerState {
    pub(super) mode: TuiComposerMode,
    pub(super) suggestions_mode: TuiInputSuggestionsMode,
}

/// Mutually exclusive composer modes.
///
/// Agent-controlled terminal use retains the agent composer. Shell mode cannot
/// represent terminal use, and user-controlled terminal use moves to `Pty`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TuiComposerMode {
    Agent { agent_controlled_terminal_use: bool },
    Shell,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TuiPtyState {
    Process,
    UserControlledTerminalUse,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TuiShortcut {
    pub(crate) key: String,
    pub(crate) description: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TuiShortcutSection {
    pub(crate) title: &'static str,
    pub(crate) shortcuts: Vec<TuiShortcut>,
}

impl TuiTerminalSessionState {
    pub(super) fn with_blocking_input_source(mut self, source: BlockingInputSource) -> Self {
        let state = match &mut self {
            Self::AltScreen { state, .. } | Self::Block(state) => state,
        };
        state.interaction = TuiInteractionState::Blocking(source);
        self
    }
    fn state(&self) -> &TuiBlockSessionState {
        match self {
            Self::AltScreen { state, .. } | Self::Block(state) => state,
        }
    }

    fn interaction(&self) -> &TuiInteractionState {
        &self.state().interaction
    }

    pub(crate) fn is_alt_screen(&self) -> bool {
        matches!(self, Self::AltScreen { .. })
    }

    pub(crate) fn has_blocking_interaction(&self) -> bool {
        matches!(
            self.interaction(),
            TuiInteractionState::Blocking(source) if source.is_interactive()
        )
    }

    pub(super) fn blocking_input_source(&self) -> Option<&BlockingInputSource> {
        match self.interaction() {
            TuiInteractionState::Blocking(source) => Some(source),
            TuiInteractionState::StartingShell
            | TuiInteractionState::Composer(_)
            | TuiInteractionState::Pty(_) => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_input(
        input_is_shell: bool,
        suggestions_mode: TuiInputSuggestionsMode,
        transcript_is_empty: bool,
        orchestration_available: bool,
    ) -> Self {
        let mode = if input_is_shell {
            TuiComposerMode::Shell
        } else {
            TuiComposerMode::Agent {
                agent_controlled_terminal_use: false,
            }
        };
        Self::Block(TuiBlockSessionState {
            interaction: TuiInteractionState::Composer(TuiComposerState {
                mode,
                suggestions_mode,
            }),
            transcript_is_empty,
            orchestration_available,
            plan_available: false,
        })
    }

    pub(crate) fn input_target(&self) -> TuiInputTarget {
        match self {
            Self::AltScreen { input_target, .. } => *input_target,
            Self::Block(state) => match &state.interaction {
                TuiInteractionState::Blocking(source) => match source {
                    BlockingInputSource::LongRunningCommand => TuiInputTarget::Pty,
                    BlockingInputSource::AskQuestion(_)
                    | BlockingInputSource::Permission(_)
                    | BlockingInputSource::Orchestration(_)
                    | BlockingInputSource::Handoff(_) => TuiInputTarget::Disabled,
                },
                TuiInteractionState::StartingShell => TuiInputTarget::Disabled,
                TuiInteractionState::Composer(_) => TuiInputTarget::AgentEditor,
                TuiInteractionState::Pty(_) => TuiInputTarget::Pty,
            },
        }
    }

    pub(crate) fn user_owns_running_command(&self) -> bool {
        matches!(
            self.interaction(),
            TuiInteractionState::Blocking(BlockingInputSource::LongRunningCommand)
                | TuiInteractionState::Pty(TuiPtyState::UserControlledTerminalUse)
        )
    }

    pub(crate) fn orchestration_available(&self) -> bool {
        self.state().orchestration_available
    }

    pub(crate) fn plan_available(&self) -> bool {
        self.state().plan_available
    }

    pub(crate) fn can_hand_back_terminal_use(&self) -> bool {
        matches!(
            self.interaction(),
            TuiInteractionState::Pty(TuiPtyState::UserControlledTerminalUse)
        )
    }

    pub(crate) fn composer_owns_input(&self) -> bool {
        matches!(
            self.interaction(),
            TuiInteractionState::Composer(TuiComposerState {
                suggestions_mode,
                ..
            }) if !suggestions_mode.is_visible()
        )
    }

    pub(crate) fn hint_text(&self) -> Option<String> {
        let state = self.state();
        let TuiInteractionState::Composer(composer) = &state.interaction else {
            return None;
        };
        if matches!(
            composer.suggestions_mode,
            TuiInputSuggestionsMode::Shortcuts
        ) {
            return None;
        }
        Some(match composer.mode {
            TuiComposerMode::Shell => SHELL_HINT.to_owned(),
            TuiComposerMode::Agent { .. } => {
                agent_input_hint(state.transcript_is_empty, state.orchestration_available)
            }
        })
    }

    pub(crate) fn should_render_shortcuts(&self) -> bool {
        matches!(
            self.interaction(),
            TuiInteractionState::Composer(TuiComposerState {
                suggestions_mode: TuiInputSuggestionsMode::Shortcuts,
                ..
            })
        )
    }

    pub(crate) fn shortcut_sections(
        &self,
        context: &Context,
        ctx: &AppContext,
    ) -> Vec<TuiShortcutSection> {
        let state = self.state();
        let composer = match &state.interaction {
            TuiInteractionState::Blocking(BlockingInputSource::LongRunningCommand) => {
                return vec![TuiShortcutSection {
                    title: "Terminal",
                    shortcuts: vec![TuiShortcut {
                        key: "ctrl-c".to_owned(),
                        description: "interrupt command",
                    }],
                }];
            }
            TuiInteractionState::Blocking(
                BlockingInputSource::AskQuestion(_)
                | BlockingInputSource::Permission(_)
                | BlockingInputSource::Orchestration(_)
                | BlockingInputSource::Handoff(_),
            )
            | TuiInteractionState::StartingShell
            | TuiInteractionState::Pty(TuiPtyState::Process) => return Vec::new(),
            TuiInteractionState::Pty(TuiPtyState::UserControlledTerminalUse) => {
                return vec![TuiShortcutSection {
                    title: "Terminal",
                    shortcuts: vec![TuiShortcut {
                        key: HAND_BACK_KEY_BINDING.to_owned(),
                        description: "hand back control",
                    }],
                }];
            }
            TuiInteractionState::Composer(composer) => composer,
        };

        let mut shortcuts = vec![TuiShortcut {
            key: "?".to_owned(),
            description: "shortcuts",
        }];
        match composer.mode {
            TuiComposerMode::Agent { .. } => shortcuts.extend([
                TuiShortcut {
                    key: "/".to_owned(),
                    description: "commands",
                },
                TuiShortcut {
                    key: "!".to_owned(),
                    description: "shell mode",
                },
                TuiShortcut {
                    key: "←".to_owned(),
                    description: "conversations",
                },
            ]),
            TuiComposerMode::Shell => shortcuts.push(TuiShortcut {
                key: "Esc".to_owned(),
                description: "agent mode",
            }),
        }
        if let Some(key) = binding_hint(AUTO_APPROVE_TOGGLE_BINDING_NAME, context, ctx) {
            shortcuts.push(TuiShortcut {
                key,
                description: "toggle auto-approve",
            });
        }
        if matches!(composer.mode, TuiComposerMode::Agent { .. }) {
            shortcuts.push(TuiShortcut {
                key: "↑".to_owned(),
                description: "input history",
            });
        }
        if state.plan_available
            && let Some(key) = binding_hint(PLAN_TOGGLE_BINDING_NAME, context, ctx)
        {
            shortcuts.push(TuiShortcut {
                key,
                description: "expand/collapse plans",
            });
        }

        let mut sections = vec![TuiShortcutSection {
            title: "Shortcuts",
            shortcuts,
        }];
        if matches!(
            composer.mode,
            TuiComposerMode::Agent {
                agent_controlled_terminal_use: true
            }
        ) {
            sections.push(TuiShortcutSection {
                title: "Terminal use",
                shortcuts: vec![TuiShortcut {
                    key: TAKE_CONTROL_KEY_BINDING.to_owned(),
                    description: "take control",
                }],
            });
        }
        if state.orchestration_available {
            sections.push(TuiShortcutSection {
                title: "Orchestration",
                shortcuts: vec![TuiShortcut {
                    key: "Shift+↑".to_owned(),
                    description: "navigate to agents",
                }],
            });
        }
        sections
    }
}

fn agent_input_hint(transcript_is_empty: bool, orchestration_tabs_available: bool) -> String {
    let mut hints = Vec::with_capacity(5);
    if transcript_is_empty {
        hints.push(SHORTCUTS_HINT);
        if orchestration_tabs_available {
            hints.push(ORCHESTRATION_HINT);
        }
        hints.extend([COMMANDS_HINT, CONVERSATIONS_HINT]);
    } else {
        hints.push(ASK_AGENT_HINT);
        hints.push(SHORTCUTS_HINT);
        if orchestration_tabs_available {
            hints.push(ORCHESTRATION_HINT);
        }
        hints.extend([SHELL_MODE_HINT, COMMANDS_HINT]);
    }
    hints.join(HINT_SEPARATOR)
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
