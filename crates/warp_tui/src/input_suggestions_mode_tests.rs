use std::cell::RefCell;
use std::rc::Rc;

use warpui_core::App;

use super::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

#[test]
fn replacing_a_visible_mode_emits_close_before_the_new_mode() {
    App::test((), |mut app| async move {
        let (mode, events) = app.update(|ctx| {
            let events = Rc::new(RefCell::new(Vec::new()));
            let events_for_subscription = events.clone();
            let mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
            ctx.subscribe_to_model(&mode, move |_, event, _| {
                events_for_subscription.borrow_mut().push(event.mode);
            });
            (mode, events)
        });

        app.update(|ctx| {
            mode.update(ctx, |mode, ctx| {
                mode.set_mode(TuiInputSuggestionsMode::ConversationMenu, ctx);
                mode.set_mode(TuiInputSuggestionsMode::ModelSelector, ctx);
            });
        });

        assert_eq!(
            events.borrow().as_slice(),
            &[
                TuiInputSuggestionsMode::ConversationMenu,
                TuiInputSuggestionsMode::Closed,
                TuiInputSuggestionsMode::ModelSelector,
            ]
        );
    });
}

#[test]
fn opening_a_menu_does_not_replace_an_active_menu() {
    App::test((), |mut app| async move {
        let mode = app.add_model(|_| TuiInputSuggestionsModeModel::new());
        mode.update(&mut app, |mode, ctx| {
            assert!(mode.try_open(TuiInputSuggestionsMode::ConversationMenu, ctx));
            assert!(!mode.try_open(TuiInputSuggestionsMode::SlashCommands, ctx));
            assert!(!mode.try_open(TuiInputSuggestionsMode::ModelSelector, ctx));
            assert!(!mode.try_open(TuiInputSuggestionsMode::SkillMenu, ctx));
            assert_eq!(mode.mode(), TuiInputSuggestionsMode::ConversationMenu);

            mode.set_mode(TuiInputSuggestionsMode::Closed, ctx);
            assert!(mode.try_open(TuiInputSuggestionsMode::SkillMenu, ctx));
            assert!(!mode.try_open(TuiInputSuggestionsMode::SlashCommands, ctx));
            assert!(!mode.try_open(TuiInputSuggestionsMode::ConversationMenu, ctx));
            assert!(!mode.try_open(TuiInputSuggestionsMode::ModelSelector, ctx));
            assert_eq!(mode.mode(), TuiInputSuggestionsMode::SkillMenu);
        });
    });
}

#[test]
fn closing_an_inactive_mode_preserves_the_active_mode() {
    App::test((), |mut app| async move {
        let mode = app.add_model(|_| TuiInputSuggestionsModeModel::new());
        mode.update(&mut app, |mode, ctx| {
            mode.set_mode(TuiInputSuggestionsMode::ConversationMenu, ctx);
            mode.close_if_active(TuiInputSuggestionsMode::SlashCommands, ctx);
            assert_eq!(mode.mode(), TuiInputSuggestionsMode::ConversationMenu);
        });
    });
}
