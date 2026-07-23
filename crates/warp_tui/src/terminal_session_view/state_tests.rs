use warpui_core::keymap::Context;
use warpui_core::{App, TuiView};

use super::{
    TuiTerminalSessionPrimaryState, TuiTerminalSessionState, TuiTerminalSessionStateFacts,
    TuiTerminalUseState,
};
use crate::input_suggestions_mode::TuiInputSuggestionsMode;
use crate::terminal_session_view::TuiTerminalSessionView;
use crate::terminal_use::TuiInputTarget;

fn state(input_is_shell: bool, orchestration_available: bool) -> TuiTerminalSessionStateFacts {
    TuiTerminalSessionStateFacts {
        alt_screen_active: false,
        blocker_active: false,
        input_target: TuiInputTarget::AgentEditor,
        input_is_shell,
        suggestions_mode: TuiInputSuggestionsMode::Closed,
        transcript_is_empty: false,
        orchestration_available,
        plan_available: false,
        terminal_use: TuiTerminalUseState::None,
    }
}

#[test]
fn shell_takes_priority_over_additive_orchestration() {
    let state = TuiTerminalSessionState::new(state(true, true));
    assert_eq!(state.primary(), TuiTerminalSessionPrimaryState::Shell);
    assert_eq!(
        state.hint_text().as_deref(),
        Some(crate::input_hints::SHELL_HINT)
    );
}

#[test]
fn primary_state_priority_is_explicit() {
    let mut facts = state(true, true);
    facts.suggestions_mode = TuiInputSuggestionsMode::Shortcuts;
    facts.terminal_use = TuiTerminalUseState::AgentControlled;
    assert_eq!(
        TuiTerminalSessionState::new(facts).primary(),
        TuiTerminalSessionPrimaryState::Shortcuts
    );

    facts.terminal_use = TuiTerminalUseState::UserControlled;
    assert_eq!(
        TuiTerminalSessionState::new(facts).primary(),
        TuiTerminalSessionPrimaryState::UserControlledCommand
    );

    facts.blocker_active = true;
    assert_eq!(
        TuiTerminalSessionState::new(facts).primary(),
        TuiTerminalSessionPrimaryState::Blocking
    );

    facts.alt_screen_active = true;
    assert_eq!(
        TuiTerminalSessionState::new(facts).primary(),
        TuiTerminalSessionPrimaryState::AltScreen
    );
}

#[test]
fn shortcuts_hide_passive_placeholder_text() {
    let state =
        TuiTerminalSessionState::for_input(false, TuiInputSuggestionsMode::Shortcuts, true, true);
    assert_eq!(state.primary(), TuiTerminalSessionPrimaryState::Shortcuts);
    assert_eq!(state.hint_text(), None);
    assert!(state.should_render_shortcuts());
}

#[test]
fn shell_and_orchestration_contribute_active_shortcut_sections() {
    App::test((), |mut app| async move {
        app.update(crate::keybindings::init);
        app.read(|ctx| {
            let mut facts = state(true, true);
            facts.plan_available = true;
            let state = TuiTerminalSessionState::new(facts);
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
            assert!(
                sections[0]
                    .shortcuts
                    .iter()
                    .any(|shortcut| shortcut.description == "agent mode")
            );
            assert!(
                sections[0]
                    .shortcuts
                    .iter()
                    .any(|shortcut| shortcut.description == "toggle auto-approve")
            );
            assert!(
                sections[0]
                    .shortcuts
                    .iter()
                    .any(|shortcut| shortcut.description == "expand/collapse plans")
            );
            assert!(
                sections[1]
                    .shortcuts
                    .iter()
                    .any(|shortcut| shortcut.description == "navigate to agents")
            );
            assert!(
                sections
                    .iter()
                    .flat_map(|section| &section.shortcuts)
                    .all(|shortcut| shortcut.description != "toggle auto-queue")
            );
        });
    });
}
