use warpui_core::keymap::Context;
use warpui_core::{App, TuiView};

use super::{
    ASK_AGENT_HINT, COMMANDS_HINT, CONVERSATIONS_HINT, SHELL_HINT, SHELL_MODE_HINT, SHORTCUTS_HINT,
    TuiAgentEditorState, TuiBlockSessionState, TuiComposerMode, TuiInteractionState, TuiPtyState,
    TuiTerminalSessionState, TuiTerminalSessionStateResolveError, agent_input_hint,
    upgrade_terminal_model,
};
use crate::input_suggestions_mode::TuiInputSuggestionsMode;
use crate::read_only_menu::TuiReadOnlyMenuKind;
use crate::terminal_session_view::{
    BlockingInputSource, SESSION_CAN_DETACH_AGENT_FROM_RUNNING_COMMAND_FLAG, TuiTerminalSessionView,
};
use crate::terminal_use::TuiInputTarget;

fn agent_editor_state(
    mode: TuiComposerMode,
    orchestration_available: bool,
) -> TuiTerminalSessionState {
    TuiTerminalSessionState::Block(TuiBlockSessionState {
        interaction: TuiInteractionState::AgentEditor(TuiAgentEditorState {
            mode,
            suggestions_mode: TuiInputSuggestionsMode::Closed,
        }),
        transcript_is_empty: false,
        orchestration_available,
        plan_available: false,
        can_attach_agent_to_running_command: false,
        agent_is_tagged_in: false,
    })
}

#[test]
fn tagged_in_composer_exposes_detach_shortcut() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        app.read(|ctx| {
            let mut state = agent_editor_state(
                TuiComposerMode::Agent {
                    agent_controlled_terminal_use: false,
                },
                false,
            );
            let TuiTerminalSessionState::Block(block) = &mut state else {
                unreachable!();
            };
            block.agent_is_tagged_in = true;
            let mut context = Context::default();
            context.set.insert(TuiTerminalSessionView::ui_name());
            context
                .set
                .insert(SESSION_CAN_DETACH_AGENT_FROM_RUNNING_COMMAND_FLAG);

            let sections = state.shortcut_sections(&context, ctx);

            assert_eq!(
                sections
                    .iter()
                    .map(|section| section.title)
                    .collect::<Vec<_>>(),
                vec!["Shortcuts", "Terminal use"]
            );
            assert_eq!(sections[1].shortcuts[0].key, "Escape");
            assert_eq!(
                sections[1].shortcuts[0].description,
                "return control to command"
            );
        });
    });
}
fn alt_screen_state(
    input_target: TuiInputTarget,
    interaction: TuiInteractionState,
) -> TuiTerminalSessionState {
    TuiTerminalSessionState::AltScreen {
        input_target,
        state: TuiBlockSessionState {
            interaction,
            transcript_is_empty: false,
            orchestration_available: false,
            plan_available: false,
            can_attach_agent_to_running_command: false,
            agent_is_tagged_in: false,
        },
    }
}

fn block_state(interaction: TuiInteractionState) -> TuiTerminalSessionState {
    TuiTerminalSessionState::Block(TuiBlockSessionState {
        interaction,
        transcript_is_empty: false,
        orchestration_available: false,
        plan_available: false,
        can_attach_agent_to_running_command: false,
        agent_is_tagged_in: false,
    })
}

#[test]
fn resolve_returns_error_after_terminal_model_owner_drops() {
    let terminal_model = std::sync::Arc::new(parking_lot::FairMutex::new(
        warp::tui_export::TerminalModel::mock(None, None),
    ));
    let weak_terminal_model = std::sync::Arc::downgrade(&terminal_model);
    drop(terminal_model);

    assert!(matches!(
        upgrade_terminal_model(&weak_terminal_model),
        Err(TuiTerminalSessionStateResolveError::TerminalModel)
    ));
}

#[test]
fn shell_hint_is_selected_with_additive_orchestration() {
    let state = agent_editor_state(TuiComposerMode::Shell, true);
    assert_eq!(state.hint_text().as_deref(), Some(SHELL_HINT));
}

#[test]
fn transcript_state_selects_the_applicable_hint_segments() {
    let zero_state = agent_input_hint(true, false);
    assert!(zero_state.contains(COMMANDS_HINT));
    assert!(zero_state.contains(CONVERSATIONS_HINT));
    assert!(zero_state.contains(SHORTCUTS_HINT));
    assert!(!zero_state.contains(ASK_AGENT_HINT));
    assert!(!zero_state.contains(SHELL_MODE_HINT));

    let started = agent_input_hint(false, false);
    assert!(started.contains(ASK_AGENT_HINT));
    assert!(started.contains(SHORTCUTS_HINT));
    assert!(started.contains(SHELL_MODE_HINT));
    assert!(started.contains(COMMANDS_HINT));
    assert!(!started.contains(CONVERSATIONS_HINT));
    assert!(SHELL_HINT.contains(SHORTCUTS_HINT));
}

#[test]
fn only_composer_interactions_produce_input_hints() {
    for state in [
        alt_screen_state(
            TuiInputTarget::Pty,
            TuiInteractionState::Pty(TuiPtyState::Process),
        ),
        block_state(TuiInteractionState::Blocking(
            BlockingInputSource::LongRunningCommand,
        )),
        block_state(TuiInteractionState::StartingShell),
        block_state(TuiInteractionState::Pty(TuiPtyState::Process)),
        block_state(TuiInteractionState::Pty(
            TuiPtyState::UserControlledTerminalUse,
        )),
    ] {
        assert_eq!(state.hint_text(), None);
    }

    let mut state = agent_editor_state(
        TuiComposerMode::Agent {
            agent_controlled_terminal_use: false,
        },
        false,
    );
    assert!(state.hint_text().is_some());
    {
        let TuiTerminalSessionState::Block(block) = &mut state else {
            unreachable!();
        };
        let TuiInteractionState::AgentEditor(agent_editor) = &mut block.interaction else {
            unreachable!();
        };
        agent_editor.suggestions_mode =
            TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Shortcuts);
    }
    assert_eq!(state.hint_text(), None);
    // The dedicated status overlay also suppresses the footer hint.
    {
        let TuiTerminalSessionState::Block(block) = &mut state else {
            unreachable!();
        };
        let TuiInteractionState::AgentEditor(agent_editor) = &mut block.interaction else {
            unreachable!();
        };
        agent_editor.suggestions_mode =
            TuiInputSuggestionsMode::ReadOnlyMenu(TuiReadOnlyMenuKind::Status);
    }
    assert_eq!(state.hint_text(), None);
}

#[test]
fn hierarchy_selects_one_input_surface() {
    assert_eq!(
        alt_screen_state(
            TuiInputTarget::Pty,
            TuiInteractionState::Pty(TuiPtyState::Process),
        )
        .input_target(),
        TuiInputTarget::Pty
    );
    assert_eq!(
        block_state(TuiInteractionState::Blocking(
            BlockingInputSource::LongRunningCommand,
        ))
        .input_target(),
        TuiInputTarget::Pty
    );
    assert_eq!(
        block_state(TuiInteractionState::StartingShell).input_target(),
        TuiInputTarget::Disabled
    );
    assert_eq!(
        block_state(TuiInteractionState::Pty(TuiPtyState::Process)).input_target(),
        TuiInputTarget::Pty
    );

    let state = agent_editor_state(TuiComposerMode::Shell, false);
    assert_eq!(state.input_target(), TuiInputTarget::AgentEditor);
    assert!(state.composer_shortcuts_active());
}

#[test]
fn suggestions_overlay_disables_composer_shortcuts_without_changing_the_input_surface() {
    let mut state = agent_editor_state(TuiComposerMode::Shell, false);
    let TuiTerminalSessionState::Block(block) = &mut state else {
        unreachable!();
    };
    let TuiInteractionState::AgentEditor(agent_editor) = &mut block.interaction else {
        unreachable!();
    };
    agent_editor.suggestions_mode = TuiInputSuggestionsMode::SlashCommands;

    assert_eq!(state.input_target(), TuiInputTarget::AgentEditor);
    assert!(!state.composer_shortcuts_active());
}
#[test]
fn alt_screen_can_retain_an_agent_composer() {
    let state = alt_screen_state(
        TuiInputTarget::AgentEditor,
        TuiInteractionState::AgentEditor(TuiAgentEditorState {
            mode: TuiComposerMode::Agent {
                agent_controlled_terminal_use: true,
            },
            suggestions_mode: TuiInputSuggestionsMode::Closed,
        }),
    );

    assert!(state.is_alt_screen());
    assert_eq!(state.input_target(), TuiInputTarget::AgentEditor);
    assert!(state.composer_shortcuts_active());
    assert!(state.hint_text().is_some());
}

#[test]
fn shell_and_orchestration_contribute_active_shortcut_sections() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        app.read(|ctx| {
            let mut state = agent_editor_state(TuiComposerMode::Shell, true);
            let TuiTerminalSessionState::Block(block) = &mut state else {
                unreachable!();
            };
            block.plan_available = true;
            let mut context = Context::default();
            context.set.insert(TuiTerminalSessionView::ui_name());
            let sections = state.shortcut_sections(&context, ctx);

            assert_eq!(
                sections
                    .iter()
                    .map(|section| section.title)
                    .collect::<Vec<_>>(),
                vec!["Shortcuts", "Orchestration"]
            );
            let descriptions = sections[0]
                .shortcuts
                .iter()
                .map(|shortcut| shortcut.description)
                .collect::<Vec<_>>();
            assert!(descriptions.contains(&"shortcuts"));
            assert!(descriptions.contains(&"agent mode"));
            assert!(descriptions.contains(&"toggle auto-approve"));
            assert!(descriptions.contains(&"expand/collapse plans"));
            assert!(!descriptions.contains(&"commands"));
            assert!(!descriptions.contains(&"shell mode"));
            assert!(!descriptions.contains(&"conversations"));
            assert!(!descriptions.contains(&"input history"));
            assert!(
                sections[1]
                    .shortcuts
                    .iter()
                    .any(|shortcut| shortcut.description == "navigate to agents")
            );
        });
    });
}

#[test]
fn agent_terminal_use_and_orchestration_are_additive() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        app.read(|ctx| {
            let state = agent_editor_state(
                TuiComposerMode::Agent {
                    agent_controlled_terminal_use: true,
                },
                true,
            );
            let mut context = Context::default();
            context.set.insert(TuiTerminalSessionView::ui_name());
            let sections = state.shortcut_sections(&context, ctx);

            assert_eq!(
                sections
                    .iter()
                    .map(|section| section.title)
                    .collect::<Vec<_>>(),
                vec!["Shortcuts", "Terminal use", "Orchestration"]
            );
            assert!(
                sections[1]
                    .shortcuts
                    .iter()
                    .any(|shortcut| shortcut.description == "take control")
            );
            assert!(
                sections[2]
                    .shortcuts
                    .iter()
                    .any(|shortcut| shortcut.description == "navigate to agents")
            );
        });
    });
}

#[test]
fn user_controlled_terminal_use_has_terminal_only_shortcuts() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let state = block_state(TuiInteractionState::Pty(
                TuiPtyState::UserControlledTerminalUse,
            ));
            let sections = state.shortcut_sections(&Context::default(), ctx);

            assert_eq!(sections.len(), 1);
            assert_eq!(sections[0].title, "Terminal");
            assert_eq!(sections[0].shortcuts.len(), 1);
            assert_eq!(sections[0].shortcuts[0].description, "hand back control");
            assert!(state.user_owns_running_command());
            assert!(state.can_hand_back_terminal_use());
            assert!(!state.composer_shortcuts_active());
        });
    });
}
