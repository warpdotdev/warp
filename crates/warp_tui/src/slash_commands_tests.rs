use ai::skills::SkillReference;
use warp::appearance::Appearance;
use warp::editor::CodeEditorModel;
use warp::tui_export::{
    slash_commands, AcceptSlashCommandOrSavedPrompt, DetectedCommand, DetectedSkillCommand,
    ParsedSlashCommandInput, SlashCommandId, SlashCommandMixer,
};
use warp_search_core::inline_menu::InlineMenuSelection;
use warpui_core::App;

use super::{
    argument_hint_text_for_parsed_input, highlighted_prefix_len_for_parsed_input,
    menu_query_for_parsed_input, TuiSlashCommandModel, TuiSlashCommandRow, MAX_VISIBLE_ROWS,
};
use crate::inline_menu::keep_selected_visible;
use crate::input_suggestions_mode::{TuiInputSuggestionsMode, TuiInputSuggestionsModeModel};

fn parsed_skill(argument: Option<&str>) -> ParsedSlashCommandInput {
    ParsedSlashCommandInput::SkillCommand(DetectedSkillCommand {
        reference: SkillReference::BundledSkillId("write-product-spec".to_owned()),
        name: "write-product-spec".to_owned(),
        argument: argument.map(str::to_owned),
    })
}

#[test]
fn argument_hint_uses_shared_static_command_placeholder() {
    let command = ParsedSlashCommandInput::SlashCommand(DetectedCommand {
        command: slash_commands::EXPORT_TO_FILE.clone(),
        argument: Some(String::new()),
    });

    assert_eq!(
        argument_hint_text_for_parsed_input(&command, "/export-to-file "),
        Some("<optional filename>")
    );
    assert_eq!(
        argument_hint_text_for_parsed_input(&command, "/export-to-file notes.md"),
        None
    );
    assert_eq!(
        argument_hint_text_for_parsed_input(&parsed_skill(Some("")), "/write-product-spec "),
        None
    );
}

fn parsed_static_command(argument: Option<&str>) -> ParsedSlashCommandInput {
    ParsedSlashCommandInput::SlashCommand(DetectedCommand {
        command: slash_commands::COMPACT.clone(),
        argument: argument.map(str::to_owned),
    })
}

#[test]
fn exact_static_command_stays_open_when_multiple_results_were_visible() {
    assert_eq!(
        menu_query_for_parsed_input(&parsed_static_command(None), true, 2).as_deref(),
        Some("compact")
    );
}

#[test]
fn only_detected_command_and_skill_prefixes_are_highlighted() {
    let command = ParsedSlashCommandInput::SlashCommand(DetectedCommand {
        command: slash_commands::PLAN.clone(),
        argument: Some("research this".to_owned()),
    });
    assert_eq!(
        highlighted_prefix_len_for_parsed_input(&command, "/plan research this"),
        Some(5)
    );
    assert_eq!(
        highlighted_prefix_len_for_parsed_input(
            &parsed_skill(Some("prompt")),
            "/write-product-spec prompt"
        ),
        Some("/write-product-spec".chars().count())
    );
    assert_eq!(
        highlighted_prefix_len_for_parsed_input(
            &ParsedSlashCommandInput::Composing {
                filter: "pla".to_owned(),
            },
            "/pla",
        ),
        None
    );
}

#[test]
fn exact_static_command_does_not_open_a_closed_menu() {
    assert_eq!(
        menu_query_for_parsed_input(&parsed_static_command(None), false, 2),
        None
    );
}

#[test]
fn unique_exact_static_command_closes_an_open_menu() {
    assert_eq!(
        menu_query_for_parsed_input(&parsed_static_command(None), true, 1),
        None
    );
}

#[test]
fn static_command_argument_entry_closes_menu() {
    assert_eq!(
        menu_query_for_parsed_input(&parsed_static_command(Some("")), true, 2),
        None
    );
    assert_eq!(
        menu_query_for_parsed_input(
            &parsed_static_command(Some("unexpected trailing input")),
            true,
            2,
        ),
        None
    );
}

#[test]
fn exact_skill_stays_open_when_multiple_results_were_visible() {
    assert_eq!(
        menu_query_for_parsed_input(&parsed_skill(None), true, 2).as_deref(),
        Some("write-product-spec")
    );
}

#[test]
fn exact_skill_does_not_open_a_closed_menu() {
    assert_eq!(
        menu_query_for_parsed_input(&parsed_skill(None), false, 2),
        None
    );
}

#[test]
fn unique_exact_skill_closes_an_open_menu() {
    assert_eq!(
        menu_query_for_parsed_input(&parsed_skill(None), true, 1),
        None
    );
}

#[test]
fn skill_argument_entry_closes_menu() {
    assert_eq!(
        menu_query_for_parsed_input(&parsed_skill(Some("")), true, 2),
        None
    );
    assert_eq!(
        menu_query_for_parsed_input(&parsed_skill(Some("here is my prompt")), true, 2),
        None
    );
}

#[test]
fn best_result_is_selected_and_scrolled_into_view() {
    let result_count = MAX_VISIBLE_ROWS + 1;
    let mut selection = InlineMenuSelection::default();
    let selected_index = selection
        .reset_to_best(result_count, |_| true)
        .expect("non-empty results should have a selection");
    let mut scroll_offset = 0;

    keep_selected_visible(
        result_count,
        selected_index,
        MAX_VISIBLE_ROWS,
        &mut scroll_offset,
    );

    assert_eq!(selected_index, result_count - 1);
    assert_eq!(scroll_offset, 1);
}

#[test]
fn completed_empty_results_close_the_menu() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let input_editor = app.add_model(|ctx| CodeEditorModel::new_tui(80, ctx));
        let suggestions_mode = app.add_model(|_| TuiInputSuggestionsModeModel::new());
        suggestions_mode.update(&mut app, |mode, ctx| {
            mode.set_mode(TuiInputSuggestionsMode::SlashCommands, ctx);
        });
        let mixer = app.add_model(|_| SlashCommandMixer::new());
        let model = app.add_model(|_| {
            TuiSlashCommandModel::new_for_test(input_editor, suggestions_mode, mixer, Vec::new(), 0)
        });

        model.update(&mut app, |model, ctx| model.refresh_rows(ctx));

        model.read(&app, |model, ctx| {
            assert!(!model.is_open(ctx));
        });
    });
}

fn assert_explicit_menu_blocks_slash_commands(explicit_mode: TuiInputSuggestionsMode) {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let input_editor = app.add_model(|ctx| CodeEditorModel::new_tui(80, ctx));
        let suggestions_mode = app.add_model(|_| TuiInputSuggestionsModeModel::new());
        suggestions_mode.update(&mut app, |mode, ctx| {
            mode.set_mode(TuiInputSuggestionsMode::SlashCommands, ctx);
        });
        let mixer = app.add_model(|_| SlashCommandMixer::new());
        let model = app.add_model(|_| {
            TuiSlashCommandModel::new_for_test(
                input_editor,
                suggestions_mode.clone(),
                mixer,
                vec![TuiSlashCommandRow {
                    title: "Test command".to_owned(),
                    description: None,
                    action: AcceptSlashCommandOrSavedPrompt::SlashCommand {
                        id: SlashCommandId::new(),
                    },
                }],
                0,
            )
        });

        model.update(&mut app, |model, ctx| {
            model.accept_selected(ctx);
        });
        suggestions_mode.update(&mut app, |mode, ctx| {
            mode.set_mode(explicit_mode, ctx);
        });
        model.update(&mut app, |model, ctx| {
            model.set_highlighted_prefix_len_for_test(Some(6));
            model.set_argument_hint_text_for_test(Some("<argument>"));
            model.update_from_input(false, ctx);
            model.run_query("model".to_owned(), false, ctx);
            assert!(!model.is_open(ctx));
            assert_eq!(model.highlighted_prefix_range(), None);
            assert_eq!(model.argument_hint_text(), None);
            assert_eq!(model.suggestions_mode.as_ref(ctx).mode(), explicit_mode);
        });
    });
}

#[test]
fn conversation_menu_blocks_slash_command_activation() {
    assert_explicit_menu_blocks_slash_commands(TuiInputSuggestionsMode::ConversationMenu);
}

#[test]
fn model_menu_blocks_slash_command_activation() {
    assert_explicit_menu_blocks_slash_commands(TuiInputSuggestionsMode::ModelSelector);
}
#[test]
fn skill_menu_blocks_slash_command_activation() {
    assert_explicit_menu_blocks_slash_commands(TuiInputSuggestionsMode::SkillMenu);
}

#[test]
fn accepting_a_result_does_not_disable_input_driven_lifecycle() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let input_editor = app.add_model(|ctx| CodeEditorModel::new_tui(80, ctx));
        let suggestions_mode = app.add_model(|_| TuiInputSuggestionsModeModel::new());
        suggestions_mode.update(&mut app, |mode, ctx| {
            mode.set_mode(TuiInputSuggestionsMode::SlashCommands, ctx);
        });
        let mixer = app.add_model(|_| SlashCommandMixer::new());
        let command_id = SlashCommandId::new();
        let model = app.add_model(|_| {
            TuiSlashCommandModel::new_for_test(
                input_editor,
                suggestions_mode,
                mixer,
                vec![TuiSlashCommandRow {
                    title: "Test command".to_owned(),
                    description: None,
                    action: AcceptSlashCommandOrSavedPrompt::SlashCommand { id: command_id },
                }],
                0,
            )
        });

        model.update(&mut app, |model, ctx| {
            assert_eq!(
                model.accept_selected(ctx),
                Some(AcceptSlashCommandOrSavedPrompt::SlashCommand { id: command_id })
            );
            assert!(model.lifecycle.input_changed(false, true));
        });
    });
}
