use warp_completer::completer::{
    EngineFileType, Match, MatchedSuggestion, Priority, Suggestion, SuggestionType,
};
use warpui_core::App;

use super::*;

fn matched_suggestion(
    display: &str,
    replacement: &str,
    file_type: Option<EngineFileType>,
) -> MatchedSuggestion {
    let mut suggestion = Suggestion::new(
        display,
        replacement,
        Some(format!("{display} description")),
        SuggestionType::Argument,
        Priority::default(),
    );
    suggestion.file_type = file_type;
    MatchedSuggestion::new(
        suggestion,
        Match::Prefix {
            is_case_sensitive: true,
        },
    )
}

#[test]
fn show_reuses_inline_menu_rows_and_accepts_the_selected_span() {
    App::test((), |mut app| async move {
        let suggestions_mode = app.add_model(|_| TuiInputSuggestionsModeModel::new());
        let menu = app.add_model(|_| TuiCompletionMenuModel::new(suggestions_mode.clone()));

        menu.update(&mut app, |menu, ctx| {
            menu.show(
                vec![
                    matched_suggestion("alpha", "alpha", None),
                    matched_suggestion("assets", "assets/", Some(EngineFileType::Directory)),
                ],
                4..7,
                true,
                ctx,
            );
        });
        app.read(|ctx| {
            let snapshot = menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            assert_eq!(snapshot.header, None);
            assert_eq!(snapshot.rows[0].title, "alpha");
            assert_eq!(
                snapshot.rows[0].description.as_deref(),
                Some("alpha description")
            );
            assert_eq!(snapshot.selected_index, Some(0));
        });

        menu.update(&mut app, |menu, ctx| menu.select_next(ctx));
        let accepted = menu.update(&mut app, |menu, ctx| menu.accept_selected(ctx));
        assert_eq!(
            accepted,
            Some(TuiCompletionAcceptance {
                replacement: "assets/".to_owned(),
                replacement_range: 4..7,
                append_space: false,
            })
        );
        app.read(|ctx| {
            assert!(!menu.as_ref(ctx).is_open(ctx));
            assert_eq!(
                suggestions_mode.as_ref(ctx).mode(),
                TuiInputSuggestionsMode::Closed
            );
        });
    });
}

#[test]
fn show_does_not_replace_an_existing_inline_menu() {
    App::test((), |mut app| async move {
        let suggestions_mode = app.add_model(|_| TuiInputSuggestionsModeModel::new());
        suggestions_mode.update(&mut app, |mode, ctx| {
            mode.set_mode(TuiInputSuggestionsMode::SlashCommands, ctx);
        });
        let menu = app.add_model(|_| TuiCompletionMenuModel::new(suggestions_mode.clone()));

        menu.update(&mut app, |menu, ctx| {
            menu.show(
                vec![matched_suggestion("alpha", "alpha", None)],
                0..1,
                true,
                ctx,
            );
        });
        app.read(|ctx| {
            assert!(!menu.as_ref(ctx).is_open(ctx));
            assert_eq!(
                suggestions_mode.as_ref(ctx).mode(),
                TuiInputSuggestionsMode::SlashCommands
            );
        });
    });
}
