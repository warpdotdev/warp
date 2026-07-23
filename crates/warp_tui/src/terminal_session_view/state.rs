use warpui_core::AppContext;
use warpui_core::keymap::Context;

use super::AUTO_APPROVE_TOGGLE_BINDING_NAME;
use crate::input_hints;
use crate::input_suggestions_mode::TuiInputSuggestionsMode;
use crate::keybindings::{PLAN_TOGGLE_BINDING_NAME, binding_hint};
use crate::terminal_use::TuiInputTarget;
use crate::tui_cli_subagent_view::{HAND_BACK_KEY_BINDING, TAKE_CONTROL_KEY_BINDING};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TuiTerminalUseState {
    None,
    AgentControlled,
    UserControlled,
    PlainUserCommand,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TuiTerminalSessionPrimaryState {
    AltScreen,
    Blocking,
    UserControlledCommand,
    Pty,
    StartingShell,
    Shortcuts,
    AgentControlledCommand,
    Shell,
    Orchestration,
    Agent,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TuiTerminalSessionStateFacts {
    pub(crate) alt_screen_active: bool,
    pub(crate) blocker_active: bool,
    pub(crate) input_target: TuiInputTarget,
    pub(crate) input_is_shell: bool,
    pub(crate) suggestions_mode: TuiInputSuggestionsMode,
    pub(crate) transcript_is_empty: bool,
    pub(crate) orchestration_available: bool,
    pub(crate) plan_available: bool,
    pub(crate) terminal_use: TuiTerminalUseState,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TuiTerminalSessionState {
    facts: TuiTerminalSessionStateFacts,
    primary: TuiTerminalSessionPrimaryState,
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
    pub(crate) fn new(facts: TuiTerminalSessionStateFacts) -> Self {
        let primary = if facts.alt_screen_active {
            TuiTerminalSessionPrimaryState::AltScreen
        } else if facts.blocker_active {
            TuiTerminalSessionPrimaryState::Blocking
        } else if matches!(
            facts.terminal_use,
            TuiTerminalUseState::UserControlled | TuiTerminalUseState::PlainUserCommand
        ) {
            TuiTerminalSessionPrimaryState::UserControlledCommand
        } else if facts.input_target.pty_owns_input() {
            TuiTerminalSessionPrimaryState::Pty
        } else if matches!(facts.input_target, TuiInputTarget::Disabled) {
            TuiTerminalSessionPrimaryState::StartingShell
        } else if matches!(facts.suggestions_mode, TuiInputSuggestionsMode::Shortcuts) {
            TuiTerminalSessionPrimaryState::Shortcuts
        } else if matches!(facts.terminal_use, TuiTerminalUseState::AgentControlled) {
            TuiTerminalSessionPrimaryState::AgentControlledCommand
        } else if facts.input_is_shell {
            TuiTerminalSessionPrimaryState::Shell
        } else if facts.orchestration_available {
            TuiTerminalSessionPrimaryState::Orchestration
        } else {
            TuiTerminalSessionPrimaryState::Agent
        };
        Self { facts, primary }
    }

    pub(crate) fn for_input(
        input_is_shell: bool,
        suggestions_mode: TuiInputSuggestionsMode,
        transcript_is_empty: bool,
        orchestration_available: bool,
    ) -> Self {
        Self::new(TuiTerminalSessionStateFacts {
            alt_screen_active: false,
            blocker_active: false,
            input_target: TuiInputTarget::AgentEditor,
            input_is_shell,
            suggestions_mode,
            transcript_is_empty,
            orchestration_available,
            plan_available: false,
            terminal_use: TuiTerminalUseState::None,
        })
    }

    pub(crate) fn primary(self) -> TuiTerminalSessionPrimaryState {
        self.primary
    }

    pub(crate) fn input_target(self) -> TuiInputTarget {
        self.facts.input_target
    }

    pub(crate) fn user_owns_running_command(self) -> bool {
        matches!(
            self.facts.terminal_use,
            TuiTerminalUseState::UserControlled | TuiTerminalUseState::PlainUserCommand
        )
    }

    pub(crate) fn orchestration_available(self) -> bool {
        self.facts.orchestration_available
    }

    pub(crate) fn plan_available(self) -> bool {
        self.facts.plan_available
    }

    pub(crate) fn can_hand_back_terminal_use(self) -> bool {
        matches!(self.facts.terminal_use, TuiTerminalUseState::UserControlled)
    }

    pub(crate) fn composer_owns_input(self) -> bool {
        self.facts.input_target.agent_editor_owns_input()
            && !self.facts.suggestions_mode.is_visible()
    }

    pub(crate) fn hint_text(self) -> Option<String> {
        if matches!(
            self.primary,
            TuiTerminalSessionPrimaryState::AltScreen
                | TuiTerminalSessionPrimaryState::Blocking
                | TuiTerminalSessionPrimaryState::UserControlledCommand
                | TuiTerminalSessionPrimaryState::Pty
                | TuiTerminalSessionPrimaryState::Shortcuts
        ) {
            return None;
        }
        if self.facts.input_is_shell {
            Some(input_hints::SHELL_HINT.to_owned())
        } else {
            Some(input_hints::agent_input_hint(
                self.facts.transcript_is_empty,
                self.facts.orchestration_available,
            ))
        }
    }

    pub(crate) fn should_render_shortcuts(self) -> bool {
        matches!(
            self.facts.suggestions_mode,
            TuiInputSuggestionsMode::Shortcuts
        ) && !matches!(
            self.primary,
            TuiTerminalSessionPrimaryState::AltScreen
                | TuiTerminalSessionPrimaryState::Blocking
                | TuiTerminalSessionPrimaryState::UserControlledCommand
                | TuiTerminalSessionPrimaryState::Pty
        )
    }

    pub(crate) fn shortcut_sections(
        self,
        context: &Context,
        ctx: &AppContext,
    ) -> Vec<TuiShortcutSection> {
        if matches!(
            self.primary,
            TuiTerminalSessionPrimaryState::AltScreen
                | TuiTerminalSessionPrimaryState::Blocking
                | TuiTerminalSessionPrimaryState::Pty
        ) {
            return Vec::new();
        }

        if matches!(
            self.facts.terminal_use,
            TuiTerminalUseState::UserControlled | TuiTerminalUseState::PlainUserCommand
        ) {
            let (key, description) = match self.facts.terminal_use {
                TuiTerminalUseState::UserControlled => (HAND_BACK_KEY_BINDING, "hand back control"),
                TuiTerminalUseState::PlainUserCommand => ("ctrl-c", "interrupt command"),
                TuiTerminalUseState::None | TuiTerminalUseState::AgentControlled => unreachable!(),
            };
            return vec![TuiShortcutSection {
                title: "Terminal",
                shortcuts: vec![TuiShortcut {
                    key: key.to_owned(),
                    description,
                }],
            }];
        }

        let mut shortcuts = Vec::new();
        if !self.facts.input_is_shell {
            shortcuts.extend([
                TuiShortcut {
                    key: "?".to_owned(),
                    description: "shortcuts",
                },
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
            ]);
        } else {
            shortcuts.push(TuiShortcut {
                key: "Esc".to_owned(),
                description: "agent mode",
            });
        }
        if let Some(key) = binding_hint(AUTO_APPROVE_TOGGLE_BINDING_NAME, context, ctx) {
            shortcuts.push(TuiShortcut {
                key,
                description: "toggle auto-approve",
            });
        }
        if !self.facts.input_is_shell {
            shortcuts.push(TuiShortcut {
                key: "↑".to_owned(),
                description: "input history",
            });
        }
        if self.facts.plan_available
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
            self.facts.terminal_use,
            TuiTerminalUseState::AgentControlled
        ) {
            sections.push(TuiShortcutSection {
                title: "Terminal use",
                shortcuts: vec![TuiShortcut {
                    key: TAKE_CONTROL_KEY_BINDING.to_owned(),
                    description: "take control",
                }],
            });
        }
        if self.facts.orchestration_available {
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

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
