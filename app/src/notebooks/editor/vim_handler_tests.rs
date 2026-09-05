use settings::Setting as _;
use string_offset::CharOffset;
use vim::vim::{MotionType, VimMode};
use warp_editor::content::buffer::{ToBufferCharOffset, ToBufferPoint};
use warp_editor::model::CoreEditorModel;
use warp_util::user_input::UserInput;
use warpui::text::point::Point;
use warpui::{App, SingletonEntity, TypedActionView, UpdateModel, ViewHandle};

use super::tests::initialize_editor;
use super::{EditorViewAction, RichTextEditorView};
use crate::editor::InteractionState;
use crate::features::FeatureFlag;
use crate::settings::AppEditorSettings;
use crate::vim_registers::{RegisterContent, VimRegisters};

fn enable_vim_notebook_flag() -> impl Drop {
    FeatureFlag::VimNotebook.override_enabled(true)
}

fn enable_vim_setting(app: &mut App) {
    app.add_singleton_model(|_| VimRegisters::new());
    app.update_model(
        &AppEditorSettings::handle(app),
        |settings: &mut AppEditorSettings, ctx| {
            settings.vim_mode.set_value(true, ctx).unwrap();
        },
    );
}

fn prepare_notebook(editor: &ViewHandle<RichTextEditorView>, markdown: &str, app: &mut App) {
    editor.update(app, |view, ctx| {
        ctx.focus_self();
        view.set_interaction_state(InteractionState::Editable, ctx);
        view.reset_with_markdown(markdown, ctx);
        view.model.update(ctx, |model, ctx| {
            vim::handler::jump_to_first_line(model, false, ctx);
        });
    });
}

fn enable_vim(editor: &ViewHandle<RichTextEditorView>, app: &mut App) {
    enable_vim_setting(app);
    prepare_notebook(editor, "hello world\nsecond line", app);
}

fn vim_type(editor: &ViewHandle<RichTextEditorView>, text: &str, app: &mut App) {
    editor.update(app, |view, ctx| {
        ctx.focus_self();
        view.handle_action(
            &EditorViewAction::VimUserTyped(UserInput::new(text.to_string())),
            ctx,
        );
    });
}

fn vim_escape(editor: &ViewHandle<RichTextEditorView>, app: &mut App) {
    editor.update(app, |view, ctx| {
        ctx.focus_self();
        view.handle_action(&EditorViewAction::VimEscape, ctx);
    });
}

fn markdown(editor: &ViewHandle<RichTextEditorView>, app: &App) -> String {
    editor.read(app, |view, ctx| view.markdown(ctx))
}

fn vim_mode(editor: &ViewHandle<RichTextEditorView>, app: &App) -> Option<VimMode> {
    editor.read(app, |view, ctx| view.vim_mode(ctx))
}

fn cursor_offset(editor: &ViewHandle<RichTextEditorView>, app: &App) -> CharOffset {
    editor.read(app, |view, ctx| {
        view.model
            .as_ref(ctx)
            .buffer_selection_model()
            .as_ref(ctx)
            .first_selection_head()
    })
}

fn selection_offsets(
    editor: &ViewHandle<RichTextEditorView>,
    app: &App,
) -> (CharOffset, CharOffset) {
    editor.read(app, |view, ctx| {
        let selection = *view
            .model
            .as_ref(ctx)
            .buffer_selection_model()
            .as_ref(ctx)
            .selection_offsets()
            .first();
        (selection.head, selection.tail)
    })
}

fn unnamed_register(app: &mut App) -> Option<RegisterContent> {
    VimRegisters::handle(app).update(app, |registers, ctx| registers.read_from_register('"', ctx))
}

fn find_bar_open(editor: &ViewHandle<RichTextEditorView>, app: &App) -> bool {
    editor.read(app, |view, _| view.find_bar.is_open())
}

fn dispatch_user_typed(editor: &ViewHandle<RichTextEditorView>, text: &str, app: &mut App) {
    editor.update(app, |view, ctx| {
        view.handle_action(
            &EditorViewAction::UserTyped(UserInput::new(text.to_string())),
            ctx,
        );
    });
}

fn open_link_editor_overlay(editor: &ViewHandle<RichTextEditorView>, app: &mut App) {
    editor.update(app, |view, ctx| {
        view.handle_action(&EditorViewAction::SelectForwardsByWord, ctx);
    });
    editor.update(app, |view, ctx| {
        view.handle_action(&EditorViewAction::CreateOrEditLink, ctx);
    });
    editor.update(app, |view, ctx| {
        assert!(view.can_edit(ctx));
        assert!(view.link_editor.as_ref(ctx).editors_focused(ctx));
    });
}

#[test]
fn notebook_vim_starts_in_normal_mode() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim(&editor, &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn notebook_user_typed_does_not_bypass_vim_in_normal_mode() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim(&editor, &mut app);
        let before = markdown(&editor, &app);

        editor.update(&mut app, |view, ctx| {
            ctx.focus_self();
            view.handle_action(
                &EditorViewAction::UserTyped(UserInput::new("hjkl".to_string())),
                ctx,
            );
        });

        assert_eq!(markdown(&editor, &app), before);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn notebook_user_typed_is_noop_when_overlay_owns_input() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim(&editor, &mut app);

        open_link_editor_overlay(&editor, &mut app);
        let before = markdown(&editor, &app);
        let cursor_before = cursor_offset(&editor, &app);
        dispatch_user_typed(&editor, "hjkl", &mut app);
        assert_eq!(markdown(&editor, &app), before);
        assert_eq!(cursor_offset(&editor, &app), cursor_before);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));

        editor.update(&mut app, |_, ctx| {
            ctx.focus_self();
        });
        vim_type(&editor, "i", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));

        open_link_editor_overlay(&editor, &mut app);
        let insert_before = markdown(&editor, &app);
        dispatch_user_typed(&editor, "x", &mut app);
        assert_eq!(markdown(&editor, &app), insert_before);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));
    });
}

#[test]
fn notebook_vim_enable_after_construction_starts_normal() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        assert_eq!(vim_mode(&editor, &app), None);

        enable_vim_setting(&mut app);
        prepare_notebook(&editor, "hello world", &mut app);

        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
        let before = markdown(&editor, &app);
        vim_type(&editor, "l", &mut app);
        assert_eq!(markdown(&editor, &app), before);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
        vim_type(&editor, "i", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));
    });
}

#[test]
fn notebook_vim_insert_and_escape_moves_left() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim_setting(&mut app);
        prepare_notebook(&editor, "hello", &mut app);

        vim_type(&editor, "A", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));
        vim_type(&editor, "x", &mut app);
        assert_eq!(markdown(&editor, &app), "hellox");
        let insert_cursor = cursor_offset(&editor, &app);
        vim_escape(&editor, &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
        assert_eq!(markdown(&editor, &app), "hellox");
        assert!(cursor_offset(&editor, &app) < insert_cursor);
    });
}

#[test]
fn notebook_vim_replace_char_noops_when_count_exceeds_line() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim_setting(&mut app);
        prepare_notebook(&editor, "abc\ndef", &mut app);

        vim_type(&editor, "5rx", &mut app);
        assert_eq!(markdown(&editor, &app), "abc\ndef");
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));

        vim_type(&editor, "2rx", &mut app);
        assert_eq!(markdown(&editor, &app), "xxc\ndef");
    });
}

#[test]
fn notebook_vim_charwise_delete_yank_change() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim_setting(&mut app);
        prepare_notebook(&editor, "hello world", &mut app);

        vim_type(&editor, "dw", &mut app);
        assert_eq!(markdown(&editor, &app), "world");
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
        let yanked = unnamed_register(&mut app).expect("dw yanks the deleted word");
        assert_eq!(yanked.text, "hello ");
        assert_eq!(yanked.motion_type, MotionType::Charwise);

        vim_type(&editor, "yw", &mut app);
        vim_type(&editor, "P", &mut app);
        assert_eq!(markdown(&editor, &app), "worldworld");

        prepare_notebook(&editor, "hello world", &mut app);
        vim_type(&editor, "cw", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));
        vim_type(&editor, "hey", &mut app);
        vim_escape(&editor, &mut app);
        assert_eq!(markdown(&editor, &app), "hey world");
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn notebook_vim_linewise_delete_yank_change() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim_setting(&mut app);
        prepare_notebook(&editor, "hello world\nsecond line\nthird", &mut app);

        vim_type(&editor, "dd", &mut app);
        assert_eq!(markdown(&editor, &app), "second line\nthird");
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
        let yanked = unnamed_register(&mut app).expect("dd yanks the deleted line");
        assert!(yanked.text.contains("hello world"));
        assert_eq!(yanked.motion_type, MotionType::Linewise);

        vim_type(&editor, "yy", &mut app);
        vim_type(&editor, "p", &mut app);
        assert_eq!(markdown(&editor, &app), "second line\nsecond line\n\nthird");

        vim_type(&editor, "gg", &mut app);
        vim_type(&editor, "cc", &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Insert));
        vim_type(&editor, "changed", &mut app);
        vim_escape(&editor, &mut app);
        assert!(markdown(&editor, &app).starts_with("changed"));
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn notebook_vim_counts_repeat_line_delete() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim_setting(&mut app);
        prepare_notebook(&editor, "one\ntwo\nthree", &mut app);

        vim_type(&editor, "2dd", &mut app);
        assert_eq!(markdown(&editor, &app), "three");
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn notebook_vim_visual_delete_yank_paste() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim_setting(&mut app);
        prepare_notebook(&editor, "abcdef", &mut app);

        vim_type(&editor, "vllld", &mut app);
        assert_eq!(markdown(&editor, &app), "ef");
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));

        prepare_notebook(&editor, "abcdef", &mut app);
        vim_type(&editor, "vllly", &mut app);
        assert_eq!(markdown(&editor, &app), "abcdef");
        vim_type(&editor, "p", &mut app);
        assert_eq!(markdown(&editor, &app), "aabcdbcdef");
    });
}

#[test]
fn notebook_vim_undo_restores_deleted_line() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim_setting(&mut app);
        prepare_notebook(&editor, "hello world\nsecond line", &mut app);

        vim_type(&editor, "dd", &mut app);
        assert_eq!(markdown(&editor, &app), "second line");
        vim_type(&editor, "u", &mut app);
        assert_eq!(markdown(&editor, &app), "hello world\nsecond line");
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn notebook_vim_search_opens_find_bar() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim(&editor, &mut app);

        assert!(!find_bar_open(&editor, &app));
        vim_type(&editor, "/", &mut app);
        assert!(find_bar_open(&editor, &app));
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

#[test]
fn notebook_vim_unsupported_comment_is_exact_noop() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim_setting(&mut app);
        prepare_notebook(&editor, "hello world", &mut app);

        let before = markdown(&editor, &app);
        let cursor_before = cursor_offset(&editor, &app);
        let selection_before = selection_offsets(&editor, &app);

        vim_type(&editor, "gcc", &mut app);
        assert_eq!(markdown(&editor, &app), before);
        assert_eq!(cursor_offset(&editor, &app), cursor_before);
        assert_eq!(selection_offsets(&editor, &app), selection_before);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));

        vim_type(&editor, "vll", &mut app);
        let after_visual = (
            markdown(&editor, &app),
            cursor_offset(&editor, &app),
            selection_offsets(&editor, &app),
        );
        vim_type(&editor, "gc", &mut app);
        assert_eq!(markdown(&editor, &app), after_visual.0);
        assert_eq!(cursor_offset(&editor, &app), after_visual.1);
        assert_eq!(selection_offsets(&editor, &app), after_visual.2);
    });
}

#[test]
fn notebook_vim_unsupported_text_object_is_a_noop() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim(&editor, &mut app);
        let before = markdown(&editor, &app);
        let cursor_before = cursor_offset(&editor, &app);
        let selection_before = selection_offsets(&editor, &app);

        vim_type(&editor, "diw", &mut app);
        assert_eq!(markdown(&editor, &app), before);
        assert_eq!(cursor_offset(&editor, &app), cursor_before);
        assert_eq!(selection_offsets(&editor, &app), selection_before);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
    });
}

fn set_vim_mode_value(app: &mut App, enabled: bool) {
    app.update_model(
        &AppEditorSettings::handle(app),
        |settings: &mut AppEditorSettings, ctx| {
            settings.vim_mode.set_value(enabled, ctx).unwrap();
        },
    );
}

#[test]
fn notebook_vim_toggle_off_on_stays_normal_without_inserting() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim(&editor, &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));

        set_vim_mode_value(&mut app, false);
        assert_eq!(vim_mode(&editor, &app), None);

        set_vim_mode_value(&mut app, true);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));

        let before = markdown(&editor, &app);
        vim_type(&editor, "l", &mut app);
        assert_eq!(markdown(&editor, &app), before);
        vim_escape(&editor, &mut app);
        vim_escape(&editor, &mut app);
        assert_eq!(vim_mode(&editor, &app), Some(VimMode::Normal));
        assert_eq!(markdown(&editor, &app), before);

        vim_type(&editor, "/", &mut app);
        assert!(find_bar_open(&editor, &app));
    });
}

#[test]
fn notebook_vim_read_only_does_not_edit() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim_setting(&mut app);
        prepare_notebook(&editor, "hello world", &mut app);
        editor.update(&mut app, |view, ctx| {
            view.set_interaction_state(InteractionState::Selectable, ctx);
        });

        let before = markdown(&editor, &app);
        vim_type(&editor, "x", &mut app);
        vim_type(&editor, "dd", &mut app);
        assert_eq!(markdown(&editor, &app), before);
    });
}

fn cursor_at(editor: &ViewHandle<RichTextEditorView>, row: u32, col: u32, app: &mut App) {
    editor.update(app, |view, ctx| {
        view.model.update(ctx, |model, ctx| {
            let offset = Point::new(row, col).to_buffer_char_offset(model.content().as_ref(ctx));
            model.cursor_at(offset, ctx);
        });
    });
}

fn cursor_row_col(editor: &ViewHandle<RichTextEditorView>, app: &App) -> (u32, u32) {
    editor.read(app, |view, ctx| {
        let model = view.model.as_ref(ctx);
        let head = model
            .buffer_selection_model()
            .as_ref(ctx)
            .first_selection_head();
        let point = head.to_buffer_point(model.content().as_ref(ctx));
        (point.row, point.column)
    })
}

#[test]
fn notebook_vim_vertical_restores_goal_column_after_short_line() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim_setting(&mut app);
        prepare_notebook(&editor, "xxxx\nab\nxxxx", &mut app);

        vim_type(&editor, "lll", &mut app);
        assert_eq!(cursor_row_col(&editor, &app), (1, 3));
        vim_type(&editor, "j", &mut app);
        assert_eq!(cursor_row_col(&editor, &app), (2, 1));
        vim_type(&editor, "j", &mut app);
        assert_eq!(cursor_row_col(&editor, &app), (3, 3));
    });
}

#[test]
fn notebook_vim_visual_vertical_keeps_tail_and_goal_column() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim_setting(&mut app);
        prepare_notebook(&editor, "xxxx\nab\nxxxx", &mut app);

        vim_type(&editor, "lll", &mut app);
        let origin = cursor_offset(&editor, &app);
        vim_type(&editor, "vjj", &mut app);
        assert_eq!(
            vim_mode(&editor, &app),
            Some(VimMode::Visual(MotionType::Charwise))
        );
        assert_eq!(cursor_row_col(&editor, &app), (3, 3));
        let (head, tail) = selection_offsets(&editor, &app);
        assert_eq!(tail, origin);
        assert!(head > tail);
    });
}

#[test]
fn notebook_vim_j_after_direct_cursor_mutation_starts_from_new_column() {
    let _vim_notebook = enable_vim_notebook_flag();
    App::test((), |mut app| async move {
        let (_window, editor, _test_view) = initialize_editor(&mut app);
        enable_vim_setting(&mut app);
        prepare_notebook(&editor, "xxxx\nab\nxxxx\nzzzz", &mut app);

        vim_type(&editor, "lll", &mut app);
        assert_eq!(cursor_row_col(&editor, &app), (1, 3));
        vim_type(&editor, "j", &mut app);
        assert_eq!(cursor_row_col(&editor, &app), (2, 1));
        cursor_at(&editor, 3, 0, &mut app);
        vim_type(&editor, "j", &mut app);
        assert_eq!(cursor_row_col(&editor, &app), (4, 0));
    });
}
