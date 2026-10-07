use std::future::Future;
use std::rc::Rc;

use warp::appearance::Appearance;
use warp::editor::CodeEditorModel;
use warp::settings::AISettingsChangedEvent;
use warp::tui_export::{
    BlocklistAIInputModel, ConversationSelectionEvent, History, HistoryEvent, InputConfig,
    InputModePolicy, InputType, PolicyConfigUpdate, TuiUpArrowHistoryItemKind,
    add_tui_history_test_models, append_tui_history_test_command,
    blocklist_ai_history_model_with_queries, register_tui_input_mode_test_settings,
};
use warp_core::features::FeatureFlag;
use warp_editor::model::CoreEditorModel;
use warpui_core::elements::tui::{Modifier, TuiBufferExt, TuiRect};
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, AppContext, EntityId, ModelHandle, SingletonEntity};

use super::{
    TuiPromptAndCommandHistoryMenuModel, TuiPromptAndCommandHistoryRow, reconciled_selection_index,
};
use crate::inline_menu::{
    TuiInlineMenuRowPrefixStyle, TuiInlineMenuStatus, render_inline_menu, single_line_menu_title,
};
use crate::input_mode_policy::AI_LOCKED_CONFIG;
use crate::input_suggestions_mode::TuiInputSuggestionsModeModel;
use crate::test_fixtures::add_test_semantic_selection;
use crate::tui_builder::TuiUiBuilder;

const W: u16 = 80;

struct TestInputModePolicy;

impl InputModePolicy for TestInputModePolicy {
    fn initial_config(&self, _app: &AppContext) -> InputConfig {
        AI_LOCKED_CONFIG
    }

    fn allows_locked_ai_input(&self, _app: &AppContext) -> bool {
        true
    }

    fn is_autodetection_enabled(&self, _app: &AppContext) -> bool {
        false
    }

    fn config_on_conversation_selection_changed(
        &self,
        _event: &ConversationSelectionEvent,
        _current: InputConfig,
        _app: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        None
    }

    fn config_on_ai_settings_changed(
        &self,
        _event: &AISettingsChangedEvent,
        _current: InputConfig,
        _is_autodetection_enabled_for_current_context: bool,
        _app: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        None
    }
}

fn agent_mode_test<T: 'static, F: Future<Output = T> + 'static>(test: impl FnOnce(App) -> F) -> T {
    let _agent_mode = FeatureFlag::AgentMode.override_enabled(true);
    App::test((), test)
}

struct MenuSetup {
    input: ModelHandle<CodeEditorModel>,
    input_mode: ModelHandle<BlocklistAIInputModel>,
    menu: ModelHandle<TuiPromptAndCommandHistoryMenuModel>,
    session_id: warp::tui_export::SessionId,
}

fn setup(
    ctx: &mut AppContext,
    prompts: &[&str],
    commands: &[&str],
    input_type: InputType,
) -> (MenuSetup, impl Future<Output = ()> + use<>) {
    register_tui_input_mode_test_settings(ctx);
    ctx.add_singleton_model(|_| Appearance::mock());
    add_test_semantic_selection(ctx);
    ctx.add_singleton_model(|_| {
        blocklist_ai_history_model_with_queries(
            prompts.iter().map(|prompt| (*prompt).to_owned()).collect(),
        )
    });
    let (active_session, session_id, initialized) = add_tui_history_test_models(
        commands
            .iter()
            .map(|command| (*command).to_owned())
            .collect(),
        ctx,
    );
    let input = ctx.add_model(|ctx| CodeEditorModel::new_tui(W, ctx));
    let input_mode = BlocklistAIInputModel::mock(Rc::new(TestInputModePolicy), ctx);
    input_mode.update(ctx, |input_mode, ctx| {
        input_mode.set_input_type(input_type, None, ctx);
    });
    let suggestions_mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
    let menu = ctx.add_model(|ctx| {
        TuiPromptAndCommandHistoryMenuModel::new(
            input.clone(),
            input_mode.clone(),
            suggestions_mode,
            active_session,
            EntityId::new(),
            ctx,
        )
    });
    (
        MenuSetup {
            input,
            input_mode,
            menu,
            session_id,
        },
        initialized,
    )
}

async fn initialized_setup(
    app: &mut App,
    prompts: &[&str],
    commands: &[&str],
    input_type: InputType,
) -> MenuSetup {
    let (setup, initialized) = app.update(|ctx| setup(ctx, prompts, commands, input_type));
    initialized.await;
    setup
}

fn set_text(input: &ModelHandle<CodeEditorModel>, text: &str, ctx: &mut AppContext) {
    input.update(ctx, |editor, ctx| {
        editor.clear_buffer(ctx);
        editor.user_insert(text, ctx);
    });
}

fn buffer_text(input: &ModelHandle<CodeEditorModel>, ctx: &AppContext) -> String {
    let buffer = input.as_ref(ctx).content().as_ref(ctx);
    if buffer.is_empty() {
        String::new()
    } else {
        buffer.text().into_string()
    }
}

fn row_titles(
    menu: &ModelHandle<TuiPromptAndCommandHistoryMenuModel>,
    ctx: &AppContext,
) -> Vec<String> {
    menu.as_ref(ctx)
        .snapshot(ctx)
        .map(|snapshot| snapshot.rows.iter().map(|row| row.title.clone()).collect())
        .unwrap_or_default()
}

#[test]
fn agent_mode_combines_ordered_deduped_prompts_and_commands() {
    agent_mode_test(|mut app| async move {
        let setup = initialized_setup(
            &mut app,
            &["deploy", "test", "deploy", "   "],
            &["ls", "pwd", "ls", "   "],
            InputType::AI,
        )
        .await;
        app.update(|ctx| {
            setup.menu.update(ctx, |menu, ctx| menu.open(ctx));

            assert_eq!(
                row_titles(&setup.menu, ctx),
                vec!["pwd", "ls", "test", "deploy"]
            );
            let snapshot = setup.menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            assert_eq!(snapshot.selected_index, Some(3));
            assert_eq!(buffer_text(&setup.input, ctx), "deploy");
            assert_eq!(setup.input_mode.as_ref(ctx).input_type(), InputType::AI);
            assert!(snapshot.rows[2].prefix.is_none());
            assert!(matches!(
                snapshot.rows[0].prefix.as_ref().map(|prefix| prefix.style),
                Some(TuiInlineMenuRowPrefixStyle::ShellCommand)
            ));
        });
    });
}

#[test]
fn shell_mode_excludes_prompts_and_previews_commands() {
    agent_mode_test(|mut app| async move {
        let setup = initialized_setup(
            &mut app,
            &["deploy the app"],
            &["git status", "cargo test"],
            InputType::Shell,
        )
        .await;
        app.update(|ctx| {
            setup.menu.update(ctx, |menu, ctx| menu.open(ctx));

            assert_eq!(
                row_titles(&setup.menu, ctx),
                vec!["git status", "cargo test"]
            );
            assert_eq!(buffer_text(&setup.input, ctx), "cargo test");
            assert_eq!(setup.input_mode.as_ref(ctx).input_type(), InputType::Shell);
        });
    });
}

#[test]
fn command_history_initialization_refreshes_an_open_menu() {
    agent_mode_test(|mut app| async move {
        let setup = initialized_setup(&mut app, &[], &[], InputType::AI).await;
        app.update(|ctx| {
            setup.menu.update(ctx, |menu, ctx| menu.open(ctx));
            assert!(row_titles(&setup.menu, ctx).is_empty());

            append_tui_history_test_command(setup.session_id, "git status".to_owned(), ctx);
            History::handle(ctx).update(ctx, |_, ctx| {
                ctx.emit(HistoryEvent::Initialized(setup.session_id));
            });
        });

        app.read(|ctx| {
            assert_eq!(row_titles(&setup.menu, ctx), vec!["git status"]);
            assert_eq!(buffer_text(&setup.input, ctx), "git status");
            assert_eq!(setup.input_mode.as_ref(ctx).input_type(), InputType::Shell);
        });
    });
}
#[test]
fn prompt_and_command_with_same_text_remain_distinct() {
    agent_mode_test(|mut app| async move {
        let setup = initialized_setup(&mut app, &["build"], &["build"], InputType::AI).await;
        app.update(|ctx| {
            setup.menu.update(ctx, |menu, ctx| menu.open(ctx));
            let snapshot = setup.menu.as_ref(ctx).snapshot(ctx).expect("menu is open");

            assert_eq!(row_titles(&setup.menu, ctx), vec!["build", "build"]);
            assert!(snapshot.rows[0].prefix.is_some());
            assert!(snapshot.rows[1].prefix.is_none());
        });
    });
}

#[test]
fn prefix_filter_matches_any_line_without_changing_source_text() {
    agent_mode_test(|mut app| async move {
        let prompt = "deploy the app\nverify the deployment";
        let setup = initialized_setup(
            &mut app,
            &[prompt, "unrelated prompt"],
            &["verify shell"],
            InputType::AI,
        )
        .await;
        app.update(|ctx| {
            set_text(&setup.input, "verify", ctx);
            setup.menu.update(ctx, |menu, ctx| menu.open(ctx));

            assert_eq!(
                row_titles(&setup.menu, ctx),
                vec!["verify shell", "deploy the app..."]
            );
            assert_eq!(
                buffer_text(&setup.input, ctx),
                "deploy the app\nverify the deployment"
            );
        });
    });
}

#[test]
fn selection_preview_switches_input_type_and_dismiss_restores_both() {
    agent_mode_test(|mut app| async move {
        let setup = initialized_setup(&mut app, &["deploy"], &[], InputType::AI).await;
        app.update(|ctx| {
            append_tui_history_test_command(setup.session_id, "deploy.sh".to_owned(), ctx);
            set_text(&setup.input, "de", ctx);
            setup.menu.update(ctx, |menu, ctx| menu.open(ctx));
            assert_eq!(buffer_text(&setup.input, ctx), "deploy.sh");
            assert_eq!(setup.input_mode.as_ref(ctx).input_type(), InputType::Shell);

            setup
                .menu
                .update(ctx, |menu, ctx| menu.select_previous(ctx));
            assert_eq!(buffer_text(&setup.input, ctx), "deploy");
            assert_eq!(setup.input_mode.as_ref(ctx).input_type(), InputType::AI);

            setup.menu.update(ctx, |menu, ctx| menu.select_next(ctx));
            assert_eq!(setup.input_mode.as_ref(ctx).input_type(), InputType::Shell);
            setup.menu.update(ctx, |menu, ctx| menu.dismiss(ctx));

            assert_eq!(buffer_text(&setup.input, ctx), "de");
            assert_eq!(setup.input_mode.as_ref(ctx).input_type(), InputType::AI);
        });
    });
}

#[test]
fn accepting_selected_item_returns_its_kind() {
    agent_mode_test(|mut app| async move {
        let command_setup = initialized_setup(&mut app, &["prompt"], &[], InputType::AI).await;
        app.update(|ctx| {
            append_tui_history_test_command(
                command_setup.session_id,
                "echo command".to_owned(),
                ctx,
            );
            command_setup.menu.update(ctx, |menu, ctx| menu.open(ctx));
            let accepted = command_setup
                .menu
                .update(ctx, |menu, ctx| menu.accept_selected(ctx))
                .expect("selected command is accepted");
            assert_eq!(accepted.text, "echo command");
            assert!(matches!(
                accepted.kind,
                TuiUpArrowHistoryItemKind::Command {
                    linked_workflow_data: None
                }
            ));
        });
    });

    agent_mode_test(|mut app| async move {
        let prompt_setup = initialized_setup(&mut app, &["prompt"], &[], InputType::AI).await;
        app.update(|ctx| {
            prompt_setup.menu.update(ctx, |menu, ctx| menu.open(ctx));
            let accepted = prompt_setup
                .menu
                .update(ctx, |menu, ctx| menu.accept_selected(ctx))
                .expect("selected prompt is accepted");
            assert_eq!(accepted.text, "prompt");
            assert_eq!(accepted.kind, TuiUpArrowHistoryItemKind::Prompt);
        });
    });
}

#[test]
fn editing_a_previewed_history_item_dismisses_the_palette_and_keeps_edited_text() {
    // Regression test for APP-5052: editing a recalled command while the
    // history palette is open must close the palette without restoring the
    // original buffer or input type, leaving the user's edit in place.
    agent_mode_test(|mut app| async move {
        // Seed a shell command so the menu has something to preview.
        let setup = initialized_setup(&mut app, &[], &["echo command"], InputType::AI).await;
        app.update(|ctx| {
            // Open menu: "echo command" is previewed and input type switches to Shell.
            setup.menu.update(ctx, |menu, ctx| menu.open(ctx));
            assert_eq!(buffer_text(&setup.input, ctx), "echo command");
            assert_eq!(setup.input_mode.as_ref(ctx).input_type(), InputType::Shell);
            assert!(setup.menu.as_ref(ctx).is_open(ctx));
        });
        // User edits the previewed text (deletes a character).
        app.update(|ctx| {
            set_text(&setup.input, "echo comman", ctx);
        });
        // The palette must be closed and the edited text kept.
        // Input type stays Shell (the type visible during preview, not original AI).
        app.update(|ctx| {
            assert!(
                !setup.menu.as_ref(ctx).is_open(ctx),
                "palette must close when user edits the previewed text"
            );
            assert_eq!(
                buffer_text(&setup.input, ctx),
                "echo comman",
                "edited text must be preserved after palette closes"
            );
            assert_eq!(
                setup.input_mode.as_ref(ctx).input_type(),
                InputType::Shell,
                "input type from preview must be kept, not restored to original"
            );
            // Accepting has no effect now that the palette is closed.
            assert!(
                setup
                    .menu
                    .update(ctx, |menu, ctx| menu.accept_selected(ctx))
                    .is_none(),
                "accept_selected must return None after palette was dismissed by edit"
            );
        });
    });
}

#[test]
fn up_down_previews_do_not_close_the_palette() {
    // Preview writes from Up/Down navigation must not be treated as user edits.
    agent_mode_test(|mut app| async move {
        let setup =
            initialized_setup(&mut app, &["prompt one", "prompt two"], &[], InputType::AI).await;
        app.update(|ctx| {
            setup.menu.update(ctx, |menu, ctx| menu.open(ctx));
            assert!(setup.menu.as_ref(ctx).is_open(ctx));
            // select_previous writes preview text — must not close the palette.
            setup
                .menu
                .update(ctx, |menu, ctx| menu.select_previous(ctx));
            assert!(
                setup.menu.as_ref(ctx).is_open(ctx),
                "select_previous must not close the palette"
            );
            // select_next writes preview text — must not close the palette.
            setup.menu.update(ctx, |menu, ctx| menu.select_next(ctx));
            assert!(
                setup.menu.as_ref(ctx).is_open(ctx),
                "select_next must not close the palette"
            );
        });
    });
}

#[test]
fn empty_and_filtered_empty_states_use_history_copy() {
    agent_mode_test(|mut app| async move {
        let empty = initialized_setup(&mut app, &[], &[], InputType::AI).await;
        app.update(|ctx| {
            empty.menu.update(ctx, |menu, ctx| menu.open(ctx));
            let snapshot = empty.menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            assert_eq!(
                snapshot.header.and_then(|header| header.title),
                Some("History".to_owned())
            );
            assert_eq!(
                snapshot.status,
                Some(TuiInlineMenuStatus::Empty("No history".to_owned()))
            );
        });
    });

    agent_mode_test(|mut app| async move {
        let filtered = initialized_setup(&mut app, &["deploy"], &["build"], InputType::AI).await;
        app.update(|ctx| {
            set_text(&filtered.input, "no match", ctx);
            filtered.menu.update(ctx, |menu, ctx| menu.open(ctx));
            let snapshot = filtered
                .menu
                .as_ref(ctx)
                .snapshot(ctx)
                .expect("menu is open");
            assert_eq!(
                snapshot.status,
                Some(TuiInlineMenuStatus::Empty("No matching history".to_owned()))
            );
        });
    });
}

#[test]
fn command_prefix_matches_the_row_text_style() {
    agent_mode_test(|mut app| async move {
        let setup = initialized_setup(&mut app, &[], &["older", "newer"], InputType::Shell).await;
        app.update(|ctx| {
            setup.menu.update(ctx, |menu, ctx| menu.open(ctx));
            let snapshot = setup.menu.as_ref(ctx).snapshot(ctx).expect("menu is open");
            let builder = TuiUiBuilder::from_app(ctx);
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                render_inline_menu(&snapshot, &builder),
                TuiRect::new(0, 0, 50, 12),
                ctx,
            );
            let rendered = frame.buffer.to_lines().join("\n");
            let expected = builder.shell_command_prefix_style();

            assert!(rendered.contains("History"));
            assert!(rendered.contains("! older"));
            assert!(rendered.contains("! newer"));
            // Unselected commands retain the transcript's bright-green prefix.
            assert_eq!(
                frame.buffer[(0, 1)].fg,
                expected.fg.expect("prefix has a color")
            );
            assert!(frame.buffer[(0, 1)].modifier.contains(Modifier::BOLD));
            assert_ne!(frame.buffer[(0, 1)].bg, builder.shell_command_background());
            // The selected prefix uses the same inverted colors as its row text.
            assert_eq!(frame.buffer[(0, 2)].fg, frame.buffer[(2, 2)].fg);
            assert_eq!(frame.buffer[(0, 2)].bg, frame.buffer[(2, 2)].bg);
        });
    });
}

#[test]
fn multiline_history_title_handles_windows_line_endings() {
    assert_eq!(
        single_line_menu_title("deploy the app\r\nthen verify it"),
        "deploy the app..."
    );
}

#[test]
fn reconciled_selection_preserves_full_row_identity() {
    let prompt = TuiPromptAndCommandHistoryRow {
        text: "build".to_owned(),
        kind: TuiUpArrowHistoryItemKind::Prompt,
    };
    let command = TuiPromptAndCommandHistoryRow {
        text: "build".to_owned(),
        kind: TuiUpArrowHistoryItemKind::Command {
            linked_workflow_data: None,
        },
    };
    let rows = vec![prompt.clone(), command.clone()];

    assert_eq!(
        reconciled_selection_index(&rows, Some(&prompt), Some(1)),
        Some(0)
    );
    assert_eq!(
        reconciled_selection_index(&rows, Some(&command), Some(0)),
        Some(1)
    );
    assert_eq!(reconciled_selection_index(&rows, None, None), Some(1));
    assert_eq!(
        reconciled_selection_index(&[], Some(&prompt), Some(0)),
        None
    );
}
