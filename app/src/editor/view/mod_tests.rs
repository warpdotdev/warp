use super::*;
use crate::auth::AuthStateProvider;
use crate::editor::soft_wrap::FrameLayouts;
use crate::editor::tests::sample_text;
use crate::editor::EditorView;
use crate::report_if_error;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspace::ToastStack;
use crate::workspaces::user_workspaces::UserWorkspaces;
use anyhow::Error;
use itertools::Itertools;
use pathfinder_geometry::vector::vec2f;
use settings::ToggleableSetting;
use unindent::Unindent;
use warpui::color::ColorU;
use warpui::platform::WindowStyle;
use warpui::text_layout::TextFrame;
use warpui::windowing::WindowManager;
use warpui::{AddSingletonModel, App, UpdateModel, UpdateView};

impl EditorView {
    fn selected_ranges(&self, app: &AppContext) -> Vec<Range<DisplayPoint>> {
        self.editor_model
            .as_ref(app)
            .local_selections_intersecting_range(DisplayPoint::zero()..self.max_point(app), app)
            .map(|(_, range)| range)
            .collect()
    }
}

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);

    app.update(|ctx| {
        AppEditorSettings::handle(ctx).update(ctx, |editor_settings, ctx| {
            let _ = editor_settings.vim_mode.set_value(true, ctx);
        })
    });

    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| ToastStack);

    app.add_singleton_model(|_ctx| SyncedInputState::mock());
    app.add_singleton_model(|_ctx| VimRegisters::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    #[cfg(feature = "voice_input")]
    app.add_singleton_model(voice_input::VoiceInput::new);

    let team_client_mock = Arc::new(MockTeamClient::new());
    let workspace_client_mock = Arc::new(MockWorkspaceClient::new());
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            team_client_mock.clone(),
            workspace_client_mock.clone(),
            vec![],
            ctx,
        )
    });
}

#[test]
fn test_selection_with_mouse() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, buffer_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(
                "aaaaaa\nbbbbbb\ncccccc\ndddddd\n",
                Default::default(),
                ctx,
            )
        });

        buffer_view.update(&mut app, |view, ctx| {
            view.begin_selection(DisplayPoint::new(2, 2), Default::default(), false, ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(2, 2)..DisplayPoint::new(2, 2)]
            );
        });

        buffer_view.update(&mut app, |view, ctx| {
            view.update_selection(DisplayPoint::new(3, 3), Vector2F::zero(), ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(2, 2)..DisplayPoint::new(3, 3)]
            );
        });

        buffer_view.update(&mut app, |view, ctx| {
            view.update_selection(DisplayPoint::new(1, 1), Vector2F::zero(), ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(2, 2)..DisplayPoint::new(1, 1)]
            );
        });

        buffer_view.update(&mut app, |view, ctx| {
            view.end_selection(ctx);
            view.update_selection(DisplayPoint::new(3, 3), Vector2F::zero(), ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(2, 2)..DisplayPoint::new(1, 1)]
            );
        });

        buffer_view.update(&mut app, |view, ctx| {
            view.begin_selection(DisplayPoint::new(3, 3), Default::default(), true, ctx);
            view.update_selection(DisplayPoint::new(0, 0), Vector2F::zero(), ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [
                    DisplayPoint::new(2, 2)..DisplayPoint::new(1, 1),
                    DisplayPoint::new(3, 3)..DisplayPoint::new(0, 0)
                ]
            );
        });

        buffer_view.update(&mut app, |view, ctx| {
            view.end_selection(ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(3, 3)..DisplayPoint::new(0, 0)]
            );
        });

        buffer_view.update(&mut app, |view, ctx| {
            view.extend_selection(DisplayPoint::new(1, 1), Vector2F::zero(), ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(3, 3)..DisplayPoint::new(1, 1)]
            );
        });

        buffer_view.update(&mut app, |view, ctx| {
            view.extend_selection(DisplayPoint::new(3, 3), Vector2F::zero(), ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(3, 3)..DisplayPoint::new(3, 3)]
            );
        });

        buffer_view.update(&mut app, |view, ctx| {
            view.extend_selection(DisplayPoint::new(2, 2), Vector2F::zero(), ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(3, 3)..DisplayPoint::new(2, 2)]
            );
        });

        buffer_view.update(&mut app, |view, ctx| {
            view.begin_selection(DisplayPoint::new(0, 0), Default::default(), true, ctx);
            view.update_selection(DisplayPoint::new(1, 1), Vector2F::zero(), ctx);
            view.end_selection(ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [
                    DisplayPoint::new(0, 0)..DisplayPoint::new(1, 1),
                    DisplayPoint::new(3, 3)..DisplayPoint::new(2, 2)
                ]
            );
        });

        buffer_view.update(&mut app, |view, ctx| {
            view.extend_selection(DisplayPoint::new(1, 2), Vector2F::zero(), ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(0, 0)..DisplayPoint::new(1, 2)]
            );
        });
    });
}

// Tests that there's always at least one selection in the `selections` vec, even during the
// case of a single pending selection
#[test]
fn test_pending_selection_always_one_selection() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, buffer_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(
                "aaaaaa\nbbbbbb\ncccccc\ndddddd\n",
                Default::default(),
                ctx,
            )
        });

        buffer_view.update(&mut app, |view, ctx| {
            view.begin_selection(DisplayPoint::new(2, 2), Default::default(), false, ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(2, 2)..DisplayPoint::new(2, 2)]
            );
        });

        // Call a fn on the view that assumes there's at least one selection.
        buffer_view.update(&mut app, |view, ctx| view.indent(ctx));

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(2, 6)..DisplayPoint::new(2, 6)]
            );
        });
    });
}

#[test]
fn test_selected_text() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text(sample_text(6, 6), Default::default(), ctx);
            editor
                .select_ranges(
                    vec![
                        DisplayPoint::new(2, 2)..DisplayPoint::new(3, 3),
                        DisplayPoint::new(4, 4)..DisplayPoint::new(5, 5),
                    ],
                    ctx,
                )
                .unwrap();
            assert_eq!(editor.selected_text(ctx), "cccc\nddd\nee\nfffff");
            editor
        });
    });
}

#[test]
fn test_select_line() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text(sample_text(6, 6), Default::default(), ctx);
            editor.select_line(&DisplayPoint::new(2, 2), ctx);
            assert_eq!(editor.selected_text(ctx), "cccccc");
            editor.model().update(ctx, |model, ctx| {
                for was_selecting in [true, false] {
                    let a11y = model.delta_for_a11y(
                        ByteOffset::from(14)..ByteOffset::from(14),
                        was_selecting,
                        ctx,
                    );
                    assert_eq!(a11y.value, "cccccc");
                    assert_eq!(a11y.help, Some(", selected".to_string()));
                }
            });
            editor
        });
    });
}

#[test]
fn test_selection_mode_lines() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, buffer_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(
                "aaaaaa\nbbbbbb\ncccccc\ndddddd\n",
                Default::default(),
                ctx,
            )
        });

        // Set starting selection and selection mode as line
        buffer_view.update(&mut app, |view, ctx| {
            view.select_line(&DisplayPoint::new(2, 2), ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(2, 0)..DisplayPoint::new(2, 6)]
            );
        });

        // Select next line
        buffer_view.update(&mut app, |view, ctx| {
            view.update_selection(DisplayPoint::new(3, 3), Vector2F::zero(), ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(2, 0)..DisplayPoint::new(3, 6)]
            );
        });

        // Select previous line
        buffer_view.update(&mut app, |view, ctx| {
            view.update_selection(DisplayPoint::new(1, 1), Vector2F::zero(), ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(2, 6)..DisplayPoint::new(1, 0)]
            );
        });

        // Select within starting selection
        buffer_view.update(&mut app, |view, ctx| {
            view.update_selection(DisplayPoint::new(2, 4), Vector2F::zero(), ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(2, 0)..DisplayPoint::new(2, 6)]
            );
        });

        buffer_view.update(&mut app, |view, ctx| {
            view.end_selection(ctx);
            view.update_selection(DisplayPoint::new(3, 3), Vector2F::zero(), ctx);
        });
    });
}

#[test]
fn test_clear_selections() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(sample_text(6, 6), Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.select_ranges(
                vec![
                    DisplayPoint::new(0, 1)..DisplayPoint::new(0, 2),
                    DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3),
                    DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4),
                    DisplayPoint::new(3, 5)..DisplayPoint::new(3, 5),
                ],
                ctx,
            )?;
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model.clear_selections(ctx);

                let a11y_label = editor_model
                    .delta_for_a11y(ByteOffset::from(0)..ByteOffset::from(0), true, ctx)
                    .value;
                assert_eq!(a11y_label, "Unselected");
            });
            assert_eq!(
                view.selected_ranges(ctx),
                vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1)]
            );
            Ok::<(), Error>(())
        })?;

        Ok(())
    })
}

#[test]
fn test_select_word() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Disable smart select and clear the word-char allowlist so basic
        // word-boundary rules apply.
        app.update(|ctx| {
            SemanticSelection::handle(ctx).update(ctx, |selection, ctx| {
                report_if_error!(selection.smart_select_enabled.toggle_and_save_value(ctx));
                report_if_error!(selection.word_char_allowlist.set_value(String::new(), ctx));
                assert!(!selection.smart_select_enabled());
            });
        });

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text("word w0rd\\ word%%", Default::default(), ctx);
            editor.select_word(&DisplayPoint::new(0, 4), ctx);
            editor.model().update(ctx, |editor_model, ctx| {
                let a11y = editor_model.delta_for_a11y(
                    ByteOffset::from(0)..ByteOffset::from(0),
                    false,
                    ctx,
                );
                assert_eq!(a11y.value, "word");
                assert_eq!(a11y.help, Some(", selected".to_string()));
            });
            assert_eq!(editor.selected_text(ctx), "word");
            editor.select_word(&DisplayPoint::new(0, 5), ctx);
            editor.model().update(ctx, |editor_model, ctx| {
                let a11y = editor_model.delta_for_a11y(
                    ByteOffset::from(5)..ByteOffset::from(0),
                    true,
                    ctx,
                );
                assert_eq!(a11y.value, "w0rd");
                assert_eq!(a11y.help, Some(", selected".to_string()));
            });
            assert_eq!(editor.selected_text(ctx), "w0rd");
            editor.select_word(&DisplayPoint::new(0, 11), ctx);
            editor.model().update(ctx, |editor_model, ctx| {
                let a11y = editor_model.delta_for_a11y(
                    ByteOffset::from(11)..ByteOffset::from(0),
                    true,
                    ctx,
                );
                assert_eq!(a11y.value, "word");
                assert_eq!(a11y.help, Some(", selected".to_string()));
            });
            assert_eq!(editor.selected_text(ctx), "word");
            editor
        });
    });
}

#[test]
fn test_select_word_with_smart_select() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Smart select is enabled by default; just verify
        app.update(|ctx| {
            SemanticSelection::handle(ctx).update(ctx, |selection, _ctx| {
                assert!(selection.smart_select_enabled());
            });
        });

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text(
                "word ~/.warp/themes/foo-bar.yaml thing",
                Default::default(),
                ctx,
            );
            editor.select_word(&DisplayPoint::new(0, 8), ctx);
            assert_eq!(editor.selected_text(ctx), "~/.warp/themes/foo-bar.yaml");
            editor
        });
    });
}

#[test]
fn test_select_word_with_custom_boundaries() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.update(|ctx| {
            SemanticSelection::handle(ctx).update(ctx, |selection, ctx| {
                // Disable smart select so custom word boundaries take effect
                report_if_error!(selection.smart_select_enabled.toggle_and_save_value(ctx));
                report_if_error!(selection
                    .word_char_allowlist
                    .set_value("/-.".to_owned(), ctx));
                assert!(!selection.smart_select_enabled());
            });
        });

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text(
                "word ~/.warp/themes/foo-bar.yaml thing",
                Default::default(),
                ctx,
            );
            editor.select_word(&DisplayPoint::new(0, 8), ctx);
            assert_eq!(editor.selected_text(ctx), "/.warp/themes/foo-bar.yaml");
            editor
        });
    });
}

#[test]
fn test_smart_select_with_drag() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Smart select is enabled by default; just verify
        app.update(|ctx| {
            SemanticSelection::handle(ctx).update(ctx, |selection, _ctx| {
                assert!(selection.smart_select_enabled());
            });
        });

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text(
                "word ~/.warp/themes/foo-bar.yaml andy@warp.dev",
                Default::default(),
                ctx,
            );
            editor.select_word(&DisplayPoint::new(0, 8), ctx);
            editor.update_selection(DisplayPoint::new(0, 34), Vector2F::zero(), ctx);
            assert_eq!(
                editor.selected_text(ctx),
                "~/.warp/themes/foo-bar.yaml andy"
            );
            editor
        });
    });
}

#[test]
fn test_selection_mode_words() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, buffer_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text("aaaaaa bbbbbb cccccc", Default::default(), ctx)
        });

        // Set starting selection and selection mode as word
        buffer_view.update(&mut app, |view, ctx| {
            view.select_word(&DisplayPoint::new(0, 9), ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(0, 7)..DisplayPoint::new(0, 13)]
            );
        });

        // Select next word
        buffer_view.update(&mut app, |view, ctx| {
            view.update_selection(DisplayPoint::new(0, 16), Vector2F::zero(), ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(0, 7)..DisplayPoint::new(0, 20)]
            );
        });

        // Select previous word
        buffer_view.update(&mut app, |view, ctx| {
            view.update_selection(DisplayPoint::new(0, 3), Vector2F::zero(), ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(0, 13)..DisplayPoint::new(0, 0)]
            );
        });

        // Select within starting selection
        buffer_view.update(&mut app, |view, ctx| {
            view.update_selection(DisplayPoint::new(0, 11), Vector2F::zero(), ctx);
        });

        buffer_view.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                [DisplayPoint::new(0, 7)..DisplayPoint::new(0, 13)]
            );
        });

        buffer_view.update(&mut app, |view, ctx| {
            view.end_selection(ctx);
            view.update_selection(DisplayPoint::new(0, 3), Vector2F::zero(), ctx);
        });
    });
}

#[test]
fn test_select_right_by_word() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text("word word", Default::default(), ctx);
            editor
                .select_ranges(vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1)], ctx)
                .unwrap();
            editor.cursor_forward_one_word(true /* select */, ctx);
            assert_eq!(editor.selected_text(ctx), "ord");
            editor.model().update(ctx, |editor_model, ctx| {
                let a11y = editor_model.delta_for_a11y(
                    ByteOffset::from(1)..ByteOffset::from(1),
                    false,
                    ctx,
                );
                assert_eq!(a11y.value, "ord");
                assert_eq!(a11y.help, Some(", selected".to_string()));
            });
            editor.cursor_forward_one_word(true /* select */, ctx);
            assert_eq!(editor.selected_text(ctx), "ord word");
            editor.model().update(ctx, |editor_model, ctx| {
                let a11y = editor_model.delta_for_a11y(
                    ByteOffset::from(5)..ByteOffset::from(1),
                    true,
                    ctx,
                );
                assert_eq!(a11y.value, "word");
                assert_eq!(a11y.help, Some(", selected".to_string()));
            });
            editor.cursor_forward_one_word(true /* select */, ctx);
            assert_eq!(editor.selected_text(ctx), "ord word");
            editor
                .select_ranges(vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 0)], ctx)
                .unwrap();
            editor.cursor_forward_one_word(true /* select */, ctx);
            assert_eq!(editor.selected_text(ctx), "ord");
            editor
                .select_ranges(vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 0)], ctx)
                .unwrap();
            editor.cursor_forward_one_word(true /* select */, ctx);
            assert_eq!(editor.selected_text(ctx), " ");
            editor
        });
    });
}

#[test]
fn test_select_left_by_word() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text("word word", Default::default(), ctx);
            editor
                .select_ranges(vec![DisplayPoint::new(0, 6)..DisplayPoint::new(0, 6)], ctx)
                .unwrap();
            editor.cursor_backward_one_word(true /* select */, ctx);
            assert_eq!(editor.selected_text(ctx), "w");
            editor.model().update(ctx, |editor_model, ctx| {
                let a11y = editor_model.delta_for_a11y(
                    ByteOffset::from(6)..ByteOffset::from(6),
                    false,
                    ctx,
                );
                assert_eq!(a11y.value, "w");
                assert_eq!(a11y.help, Some(", selected".to_string()));
            });
            editor.cursor_backward_one_word(true /* select */, ctx);
            assert_eq!(editor.selected_text(ctx), "word w");
            editor.model().update(ctx, |editor_model, ctx| {
                let a11y = editor_model.delta_for_a11y(
                    ByteOffset::from(5)..ByteOffset::from(6),
                    true,
                    ctx,
                );
                assert_eq!(a11y.value, "word ");
                assert_eq!(a11y.help, Some(", selected".to_string()));
            });
            editor.cursor_backward_one_word(true /* select */, ctx);
            assert_eq!(editor.selected_text(ctx), "word w");
            editor
                .select_ranges(vec![DisplayPoint::new(0, 6)..DisplayPoint::new(0, 9)], ctx)
                .unwrap();
            editor.cursor_backward_one_word(true /* select */, ctx);
            assert_eq!(editor.selected_text(ctx), "w");
            editor
                .select_ranges(vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 5)], ctx)
                .unwrap();
            editor.cursor_backward_one_word(true /* select */, ctx);
            assert_eq!(editor.selected_text(ctx), "");
            editor
        });
    });
}

#[test]
fn test_delete_word_left() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text("word word\\ word%%", Default::default(), ctx);
            editor
                .select_ranges(
                    vec![DisplayPoint::new(0, 13)..DisplayPoint::new(0, 13)],
                    ctx,
                )
                .unwrap();
            editor.delete_word_left(ctx);
            assert_eq!(editor.buffer_text(ctx), "word word\\ rd%%");
            editor.delete_word_left(ctx);
            assert_eq!(editor.buffer_text(ctx), "word rd%%");
            editor.delete_word_left(ctx);
            assert_eq!(editor.buffer_text(ctx), "rd%%");

            editor.undo(ctx);
            assert_eq!(editor.buffer_text(ctx), "word rd%%");
            editor.redo(ctx);
            assert_eq!(editor.buffer_text(ctx), "rd%%");

            editor
        });
    });
}

#[test]
fn test_select_linewise() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let base_text = "abcd
                efg

                hijk
                lmnop
                q
                r"
        .unindent();

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |view_ctx| {
            let editor = EditorView::new_with_base_text(base_text, Default::default(), view_ctx);

            assert_eq!(
                editor.buffer_text(view_ctx),
                "abcd\nefg\n\nhijk\nlmnop\nq\nr"
            );

            editor
        });

        // given a cursor before "a", select "abcd\n"
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, ctx| {
                model.extend_selection_linewise(/* include_newline */ true, ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(1, 0),]
            );
        });

        // given a cursor between "e" and "f", select "efg\n"
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(1, 1)..DisplayPoint::new(1, 1)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, ctx| {
                model.extend_selection_linewise(/* include_newline */ true, ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(2, 0),]
            );
        });

        // given a cursor on a blank line, select the newline ("\n")
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(2, 0)..DisplayPoint::new(2, 0)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, ctx| {
                model.extend_selection_linewise(/* include_newline */ true, ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(2, 0)..DisplayPoint::new(3, 0),]
            );
        });

        // given a cursor after "k", select "hijk\n"
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(3, 4)..DisplayPoint::new(3, 4)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, ctx| {
                model.extend_selection_linewise(/* include_newline */ true, ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(3, 0)..DisplayPoint::new(4, 0),]
            );
        });

        // given a cursor between "m" and "n", select "lmnop"
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(4, 3)..DisplayPoint::new(4, 3)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, ctx| {
                model.extend_selection_linewise(/* include_newline */ false, ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(4, 0)..DisplayPoint::new(4, 5),]
            );
        });

        // given a cursor on "q", select "q"
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(5, 0)..DisplayPoint::new(5, 0)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, ctx| {
                model.extend_selection_linewise(/* include_newline */ false, ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(5, 0)..DisplayPoint::new(5, 1),]
            );
        });

        // given a cursor on "r", select "\nr"
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(6, 0)..DisplayPoint::new(6, 0)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, ctx| {
                model.extend_selection_linewise(/* include_newline */ true, ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(5, 1)..DisplayPoint::new(6, 1),]
            );
        });

        Ok(())
    })
}

#[test]
fn test_select_lines_below() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let base_text = "The quick brown fox
                jumped over
                the lazy dog."
            .unindent();

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |view_ctx| {
            EditorView::new_with_base_text(base_text, Default::default(), view_ctx)
        });

        // Given a cursor on "q" from "quick", extend the selection to cover one line below.
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, model_ctx| {
                model.extend_selection_below(1, model_ctx);
                model.selection_line_start_test(model_ctx);

                model.cursor_line_end(true, model_ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(1, 11),]
            );
        });

        // Given a cursor on "q" from "quick", extend the selection to
        // the character directly below ("d").
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, model_ctx| {
                model.extend_selection_below(1, model_ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(1, 4),]
            );
        });

        // Given a cursor on "n" from "brown", extend the selection to cover one line below.
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 15)..DisplayPoint::new(0, 15)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, model_ctx| {
                model.extend_selection_below(1, model_ctx);
                model.selection_line_start_test(model_ctx);
                model.cursor_line_end(true, model_ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(1, 11),]
            );
        });

        // Given a cursor on "n" from "brown", extend the selection to the character
        // directly below, which should default to the end of the line.
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 15)..DisplayPoint::new(0, 15)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, model_ctx| {
                model.extend_selection_below(1, model_ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(0, 15)..DisplayPoint::new(1, 11),]
            );
        });

        // Given a cursor on "l" from "lazy", attempt to extend the selection
        // to cover one line below, even though there is nothing there.
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(2, 4)..DisplayPoint::new(2, 4)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, model_ctx| {
                model.extend_selection_below(1, model_ctx);
                model.selection_line_start_test(model_ctx);
                model.cursor_line_end(true, model_ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(2, 0)..DisplayPoint::new(2, 13),]
            );
        });
    });
}

#[test]
fn test_select_lines_above() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let base_text = "The quick brown fox
                jumped
                over the lazy dog."
            .unindent();

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |view_ctx| {
            EditorView::new_with_base_text(base_text, Default::default(), view_ctx)
        });

        // Given a cursor on "r" from "over", extend the selection to cover one line above.
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(2, 3)..DisplayPoint::new(2, 3)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, model_ctx| {
                model.extend_selection_above(1, model_ctx);
                model.selection_line_start_test(model_ctx);
                model.cursor_line_end(true, model_ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(2, 18),]
            );
        });

        // Given a cursor on "r" from "over", extend the selection to
        // the character directly above ("p").
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(2, 3)..DisplayPoint::new(2, 3)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, model_ctx| {
                model.extend_selection_above(1, model_ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(1, 3)..DisplayPoint::new(2, 3),]
            );
        });

        // Given a cursor on "l" from "lazy", extend the selection to cover
        // the line above.
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(2, 9)..DisplayPoint::new(2, 9)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, model_ctx| {
                model.extend_selection_above(1, model_ctx);
                model.selection_line_start_test(model_ctx);
                model.cursor_line_end(true, model_ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(1, 0)..DisplayPoint::new(2, 18),]
            );
        });

        // Given a cursor on "l" from "lazy", extend the selection to the
        // character directly above, which should default to the end of the line.
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(2, 9)..DisplayPoint::new(2, 9)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, model_ctx| {
                model.extend_selection_above(1, model_ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(1, 6)..DisplayPoint::new(2, 9),]
            );
        });

        // Given a cursor on "q" from "quick", attempt to extend the selection
        // to cover one line above, even though there is nothing there.
        editor.update(&mut app, |view, view_ctx| {
            view.select_ranges(
                vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)],
                view_ctx,
            )
            .unwrap();

            view.change_selections(view_ctx, |model, model_ctx| {
                model.extend_selection_above(1, model_ctx);
                model.selection_line_start_test(model_ctx);
                model.cursor_line_end(true, model_ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 19),]
            );
        });
    });
}

#[test]
fn test_delete_word_right() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text("word word\\ word%%", Default::default(), ctx);
            editor
                .select_ranges(vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 13)], ctx)
                .unwrap();
            editor.delete_word_right(ctx);
            assert_eq!(editor.buffer_text(ctx), "w word\\ word%%");
            editor.delete_word_right(ctx);
            assert_eq!(editor.buffer_text(ctx), "w\\ word%%");
            editor.delete_word_right(ctx);
            assert_eq!(editor.buffer_text(ctx), "w%%");
            editor.delete_word_right(ctx);
            assert_eq!(editor.buffer_text(ctx), "w");

            editor.undo(ctx);
            assert_eq!(editor.buffer_text(ctx), "w%%");
            editor.redo(ctx);
            assert_eq!(editor.buffer_text(ctx), "w");

            editor
        });
    });
}

#[test]
fn test_backspace() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text(sample_text(6, 6), Default::default(), ctx);
            editor
                .select_ranges(
                    vec![
                        DisplayPoint::new(2, 2)..DisplayPoint::new(2, 2),
                        DisplayPoint::new(3, 3)..DisplayPoint::new(3, 3),
                        DisplayPoint::new(4, 4)..DisplayPoint::new(5, 5),
                    ],
                    ctx,
                )
                .unwrap();
            editor.backspace(ctx);
            assert_eq!(
                editor.buffer_text(ctx),
                "aaaaaa\nbbbbbb\nccccc\nddddd\neeeef"
            );

            editor.undo(ctx);
            assert_eq!(
                editor.buffer_text(ctx),
                "aaaaaa\nbbbbbb\ncccccc\ndddddd\neeeeee\nffffff"
            );

            editor
        });
    });
}

#[test]
fn test_page_up() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(
                "aaaaa\nbbbbbbb\nccccc\nddddddd\neeeeee",
                Default::default(),
                ctx,
            )
        });

        // Single cursor on top line
        editor.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(0, 2)..DisplayPoint::new(0, 2)], ctx)
                .unwrap();
            view.page_up(ctx);
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });

        // Single cursor below top line, fewer columns than length of top line
        editor.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(2, 2)..DisplayPoint::new(2, 2)], ctx)
                .unwrap();
            view.page_up(ctx);
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(0, 2)..DisplayPoint::new(0, 2)]
            );
        });

        // Single cursor below top line, more columns than length of top line
        editor.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(1, 6)..DisplayPoint::new(1, 6)], ctx)
                .unwrap();
            view.page_up(ctx);
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5)]
            );
        });

        // Multiple cursors, same column
        editor.update(&mut app, |view, ctx| {
            view.select_ranges(
                vec![
                    DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1),
                    DisplayPoint::new(1, 1)..DisplayPoint::new(1, 1),
                    DisplayPoint::new(2, 1)..DisplayPoint::new(2, 1),
                ],
                ctx,
            )
            .unwrap();
            view.page_up(ctx);
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1)]
            );
        });

        // Multiple cursors, different columns
        editor.update(&mut app, |view, ctx| {
            view.select_ranges(
                vec![
                    DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1),
                    DisplayPoint::new(1, 2)..DisplayPoint::new(1, 2),
                    DisplayPoint::new(2, 3)..DisplayPoint::new(4, 3),
                ],
                ctx,
            )
            .unwrap();
            view.page_up(ctx);
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
        });
    })
}

#[test]
fn test_page_down() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(
                "aaaaa\nbbbbbbb\nccccc\nddddddd\neeeee",
                Default::default(),
                ctx,
            )
        });

        // Single cursor on bottom line
        editor.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(4, 2)..DisplayPoint::new(4, 2)], ctx)
                .unwrap();
            view.page_down(ctx);
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(4, 5)..DisplayPoint::new(4, 5)]
            );
        });

        // Single cursor above bottom line, fewer columns than length of bottom line
        editor.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(2, 2)..DisplayPoint::new(2, 2)], ctx)
                .unwrap();
            view.page_down(ctx);
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(4, 2)..DisplayPoint::new(4, 2)]
            );
        });

        // Single cursor above bottom line, more columns than length of bottom line
        editor.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(3, 7)..DisplayPoint::new(3, 7)], ctx)
                .unwrap();
            view.page_down(ctx);
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(4, 5)..DisplayPoint::new(4, 5)]
            );
        });

        // Multiple cursors, same column
        editor.update(&mut app, |view, ctx| {
            view.select_ranges(
                vec![
                    DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1),
                    DisplayPoint::new(1, 1)..DisplayPoint::new(1, 1),
                    DisplayPoint::new(2, 1)..DisplayPoint::new(2, 1),
                ],
                ctx,
            )
            .unwrap();
            view.page_down(ctx);
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(4, 1)..DisplayPoint::new(4, 1)]
            );
        });

        // Multiple cursors, different columns
        editor.update(&mut app, |view, ctx| {
            view.select_ranges(
                vec![
                    DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1),
                    DisplayPoint::new(1, 2)..DisplayPoint::new(1, 2),
                    DisplayPoint::new(2, 3)..DisplayPoint::new(4, 3),
                ],
                ctx,
            )
            .unwrap();
            view.page_down(ctx);
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(4, 5)..DisplayPoint::new(4, 5)]
            );
        });
    })
}

#[test]
fn test_delete_basic() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text(sample_text(6, 6), Default::default(), ctx);
            editor
                .select_ranges(
                    vec![
                        DisplayPoint::new(2, 2)..DisplayPoint::new(2, 2),
                        DisplayPoint::new(3, 3)..DisplayPoint::new(3, 3),
                        DisplayPoint::new(4, 4)..DisplayPoint::new(5, 5),
                    ],
                    ctx,
                )
                .unwrap();
            editor.delete(ctx);
            assert_eq!(
                editor.buffer_text(ctx),
                "aaaaaa\nbbbbbb\nccccc\nddddd\neeeef"
            );

            editor.undo(ctx);
            assert_eq!(
                editor.buffer_text(ctx),
                "aaaaaa\nbbbbbb\ncccccc\ndddddd\neeeeee\nffffff"
            );
            editor
        });
    });
}

#[test]
fn test_indent_empty_buffer() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new(Default::default(), ctx);
            editor.handle_tab(ctx);
            assert_eq!(editor.buffer_text(ctx), "    ");

            editor.undo(ctx);
            assert_eq!(editor.buffer_text(ctx), "");
            editor.redo(ctx);
            assert_eq!(editor.buffer_text(ctx), "    ");

            editor
        });
    });
}

#[test]
fn test_undo_redo_text_styles() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let black_text_style = TextStyle::new().with_background_color(ColorU::black());
        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new(Default::default(), ctx);
            editor.insert_with_styles(
                "foo",
                &[(ByteOffset::from(0)..ByteOffset::from(3), black_text_style)],
                PlainTextEditorViewAction::SystemInsert,
                ctx,
            );
            assert_eq!(editor.buffer_text(ctx), "foo");

            editor.clear_buffer(ctx);
            assert_eq!(editor.buffer_text(ctx), "");

            editor.undo(ctx);
            assert_eq!(editor.buffer_text(ctx), "foo");
            assert_eq!(
                editor.text_style_runs(ctx).collect_vec(),
                vec![TextRun::new(
                    "foo".into(),
                    black_text_style,
                    ByteOffset::from(0)..ByteOffset::from(3)
                )]
            );

            editor.redo(ctx);
            assert_eq!(editor.buffer_text(ctx), "");
            assert_eq!(editor.text_style_runs(ctx).count(), 0);

            editor
        });
    })
}

#[test]
fn test_indent() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text(sample_text(6, 6), Default::default(), ctx);
            editor
                .select_ranges(
                    vec![
                        DisplayPoint::new(0, 0)..DisplayPoint::new(0, 2),
                        DisplayPoint::new(1, 1)..DisplayPoint::new(1, 5),
                    ],
                    ctx,
                )
                .unwrap();
            editor.handle_tab(ctx);
            assert_eq!(
                editor.buffer_text(ctx),
                "    aaaa\nb    b\ncccccc\ndddddd\neeeeee\nffffff"
            );

            editor
                .select_ranges(vec![DisplayPoint::new(2, 0)..DisplayPoint::new(2, 2)], ctx)
                .unwrap();
            editor.handle_tab(ctx);
            assert_eq!(
                editor.buffer_text(ctx),
                "    aaaa\nb    b\n    cccccc\ndddddd\neeeeee\nffffff"
            );

            editor
                .select_ranges(vec![DisplayPoint::new(3, 0)..DisplayPoint::new(3, 6)], ctx)
                .unwrap();
            editor.handle_tab(ctx);
            assert_eq!(
                editor.buffer_text(ctx),
                "    aaaa\nb    b\n    cccccc\n    dddddd\neeeeee\nffffff"
            );

            editor
                .select_ranges(vec![DisplayPoint::new(4, 3)..DisplayPoint::new(5, 6)], ctx)
                .unwrap();
            editor.handle_tab(ctx);
            assert_eq!(
                editor.buffer_text(ctx),
                "    aaaa\nb    b\n    cccccc\n    dddddd\n    eeeeee\n    ffffff"
            );

            editor.undo(ctx);
            assert_eq!(
                editor.buffer_text(ctx),
                "    aaaa\nb    b\n    cccccc\n    dddddd\neeeeee\nffffff"
            );
            editor.redo(ctx);
            assert_eq!(
                editor.buffer_text(ctx),
                "    aaaa\nb    b\n    cccccc\n    dddddd\n    eeeeee\n    ffffff"
            );
            editor
        });
    })
}

#[test]
fn test_unindent() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text(
                "word\n  word\n    word\n        word\n",
                Default::default(),
                ctx,
            );
            editor
                .select_ranges(
                    vec![
                        DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0),
                        DisplayPoint::new(1, 1)..DisplayPoint::new(1, 1),
                        DisplayPoint::new(2, 2)..DisplayPoint::new(2, 2),
                        DisplayPoint::new(3, 5)..DisplayPoint::new(3, 5),
                    ],
                    ctx,
                )
                .unwrap();
            editor.unindent(ctx);
            assert_eq!(editor.buffer_text(ctx), "word\nword\nword\n    word\n");

            editor.undo(ctx);
            assert_eq!(
                editor.buffer_text(ctx),
                "word\n  word\n    word\n        word\n"
            );
            editor.redo(ctx);
            assert_eq!(editor.buffer_text(ctx), "word\nword\nword\n    word\n");

            editor
        });
    })
}

#[test]
fn test_single_line_multi_cursor_unindent() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text("    hello world", Default::default(), ctx);
            editor
                .select_ranges(
                    vec![
                        DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4),
                        DisplayPoint::new(0, 5)..DisplayPoint::new(0, 6),
                        DisplayPoint::new(0, 8)..DisplayPoint::new(0, 8),
                    ],
                    ctx,
                )
                .unwrap();
            editor.unindent(ctx);
            assert_eq!(editor.buffer_text(ctx), "hello world");

            editor
        });
    })
}

#[test]
fn test_multi_line_unindent() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new(Default::default(), ctx)
        });

        // Suppose the buffer text starts with an empty line followed by a line
        // with leading whitespace + text.
        //
        // Unindent'ing the whole buffer in this case should leave the
        // first line untouched while removing the leading whitespace from the first line.
        editor.update(&mut app, |editor, ctx| {
            editor.set_buffer_text("\n    hello", ctx);
            editor.select_all(ctx);
            editor.unindent(ctx);
            assert_eq!(editor.buffer_text(ctx), "\nhello");
        });

        // Suppose the buffer text starts with a line with some whitespace
        // followed by some text. Similarly, the second line starts with some
        // whitespace followed by some text.
        //
        // Unindent'ing the whole buffer in this case should remove the leading
        // whitespace from both the first and second lines.
        editor.update(&mut app, |editor, ctx| {
            editor.set_buffer_text("  hello\n    world", ctx);
            editor.select_all(ctx);
            editor.unindent(ctx);
            assert_eq!(editor.buffer_text(ctx), "hello\nworld");
        });
    })
}

#[test]
fn test_cursor_line_start() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text(
                "word\n  word\n    word\n        word\n",
                Default::default(),
                ctx,
            );
            editor
                .select_ranges(
                    vec![
                        DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0),
                        DisplayPoint::new(1, 1)..DisplayPoint::new(1, 1),
                        DisplayPoint::new(2, 2)..DisplayPoint::new(2, 2),
                        DisplayPoint::new(3, 5)..DisplayPoint::new(3, 5),
                    ],
                    ctx,
                )
                .unwrap();
            editor.change_selections(ctx, |editor_model, ctx| {
                editor_model.cursor_line_start(false /* keep_selection */, ctx)
            });
            editor
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![
                    DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0),
                    DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0),
                    DisplayPoint::new(2, 0)..DisplayPoint::new(2, 0),
                    DisplayPoint::new(3, 0)..DisplayPoint::new(3, 0),
                ]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 3)], ctx)
                .unwrap();
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model.cursor_line_start(true /* keep_selection */, ctx)
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 0),]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 1)], ctx)
                .unwrap();
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model.cursor_line_start(true /* keep_selection */, ctx)
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 0),]
            );
        });
    })
}

#[test]
fn test_cursor_line_end() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text(
                "word\n  word\n    word\n        word\n",
                Default::default(),
                ctx,
            );
            editor
                .select_ranges(
                    vec![
                        DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0),
                        DisplayPoint::new(1, 1)..DisplayPoint::new(1, 1),
                        DisplayPoint::new(2, 2)..DisplayPoint::new(2, 2),
                        DisplayPoint::new(3, 5)..DisplayPoint::new(3, 5),
                    ],
                    ctx,
                )
                .unwrap();
            editor.change_selections(ctx, |editor_model, ctx| {
                editor_model.cursor_line_end(false, ctx);
            });
            editor
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![
                    DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4),
                    DisplayPoint::new(1, 6)..DisplayPoint::new(1, 6),
                    DisplayPoint::new(2, 8)..DisplayPoint::new(2, 8),
                    DisplayPoint::new(3, 12)..DisplayPoint::new(3, 12),
                ]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 1)], ctx)
                .unwrap();
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model.cursor_line_end(true, ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 4),]
            );
        });

        editor.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 1)], ctx)
                .unwrap();
            view.change_selections(ctx, |editor_model, ctx| {
                editor_model.cursor_line_end(true, ctx);
            });
        });

        editor.read(&app, |view, app| {
            assert_eq!(
                view.selected_ranges(app),
                vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 4),]
            );
        });
    })
}

#[test]
fn test_clear_lines() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(sample_text(6, 6), Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.select_ranges(
                vec![
                    DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1),
                    DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3),
                    DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4),
                    DisplayPoint::new(5, 0)..DisplayPoint::new(5, 0),
                    DisplayPoint::new(5, 2)..DisplayPoint::new(5, 2),
                ],
                ctx,
            )?;
            view.clear_lines(ctx);
            assert_eq!(view.buffer_text(ctx), "\ncccccc\ndddddd\neeeeee\n");

            view.undo(ctx);
            assert_eq!(
                view.buffer_text(ctx),
                "aaaaaa\nbbbbbb\ncccccc\ndddddd\neeeeee\nffffff"
            );
            view.redo(ctx);
            assert_eq!(view.buffer_text(ctx), "\ncccccc\ndddddd\neeeeee\n");

            Ok::<(), Error>(())
        })?;

        Ok(())
    })
}

#[test]
fn test_clear_and_copy_lines() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(sample_text(6, 6), Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.select_ranges(
                vec![
                    DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1),
                    DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3),
                    DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4),
                    DisplayPoint::new(5, 0)..DisplayPoint::new(5, 0),
                    DisplayPoint::new(5, 2)..DisplayPoint::new(5, 2),
                ],
                ctx,
            )?;
            view.clear_and_copy_lines(ctx);
            assert_eq!(view.buffer_text(ctx), "\n\ncccccc\ndddddd\neeeeee\n");

            view.undo(ctx);
            assert_eq!(
                view.buffer_text(ctx),
                "aaaaaa\nbbbbbb\ncccccc\ndddddd\neeeeee\nffffff"
            );
            view.redo(ctx);
            assert_eq!(view.buffer_text(ctx), "\n\ncccccc\ndddddd\neeeeee\n");

            Ok::<(), Error>(())
        })?;

        view.read(&app, |view, _| {
            assert_eq!(view.internal_clipboard, "aaaaaa\nbbbbbb\nffffff");
            Result::<()>::Ok(())
        })?;

        Ok(())
    })
}

#[test]
fn test_cut_word_left() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text("word1 word2 word3", Default::default(), ctx);
            editor.set_shell_family(ShellFamily::Posix);
            editor
        });

        view.update(&mut app, |view, ctx| {
            view.move_to_buffer_end(ctx);
            // Cutting a word (Ctrl-W) should use the internal shell clipboard
            // (sometimes called the kill ring), not the system clipboard.
            view.cut_word_left(ctx);
            assert_eq!(view.buffer_text(ctx), "word1 word2 ");
        });

        view.update(&mut app, |view, ctx| {
            // Verify the clipboard states in a separate update, so that the
            // shell cut action is dispatched.
            assert_eq!(view.internal_clipboard, "word3");
            assert!(view.clipboard_content(ctx).is_empty());

            // Yanking should then paste the cut word from the shell clipboard.
            view.yank(ctx);
            assert_eq!(view.buffer_text(ctx), "word1 word2 word3");
            assert_eq!(view.internal_clipboard, "word3");
        });

        view.update(&mut app, |view, ctx| {
            // Cutting should also be undoable / redoable.
            view.cut_word_left(ctx);
            assert_eq!(view.buffer_text(ctx), "word1 word2 ");
            view.undo(ctx);
            assert_eq!(view.buffer_text(ctx), "word1 word2 word3");
            view.redo(ctx);
            assert_eq!(view.buffer_text(ctx), "word1 word2 ");
        });
    })
}

#[test]
fn test_cut_word_right() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text("word1 word2 word3", Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.move_to_buffer_start(ctx);
            // Cutting a word to the right (Alt-D) should use the internal
            //shell clipboard, not the system clipboard.
            view.cut_word_right(ctx);
            assert_eq!(view.buffer_text(ctx), " word2 word3");
        });

        view.update(&mut app, |view, ctx| {
            // Verify the clipboard states in a separate update, so that the
            // shell cut action is dispatched.
            assert_eq!(view.internal_clipboard, "word1");
            assert!(view.clipboard_content(ctx).is_empty());

            // Yanking should then paste the cut word from the shell clipboard.
            view.yank(ctx);
            assert_eq!(view.buffer_text(ctx), "word1 word2 word3");
            assert_eq!(view.internal_clipboard, "word1");
        });

        view.update(&mut app, |view, ctx| {
            view.move_to_buffer_start(ctx);

            // Cutting should also be undoable / redoable.
            view.cut_word_right(ctx);
            assert_eq!(view.buffer_text(ctx), " word2 word3");
            view.undo(ctx);
            assert_eq!(view.buffer_text(ctx), "word1 word2 word3");
            view.redo(ctx);
            assert_eq!(view.buffer_text(ctx), " word2 word3");
        });
    })
}

#[test]
fn test_delete_and_cut_all_right() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(sample_text(6, 6), Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.select_ranges(
                vec![
                    DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1),
                    DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3),
                    DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4),
                    DisplayPoint::new(3, 5)..DisplayPoint::new(3, 5),
                ],
                ctx,
            )?;
            view.delete_all(CutDirection::Right, true, ctx);
            assert_eq!(
                view.buffer_text(ctx),
                "a\nbbbb\ncccccc\nddddd\neeeeee\nffffff"
            );

            view.undo(ctx);
            assert_eq!(
                view.buffer_text(ctx),
                "aaaaaa\nbbbbbb\ncccccc\ndddddd\neeeeee\nffffff"
            );
            view.redo(ctx);
            assert_eq!(
                view.buffer_text(ctx),
                "a\nbbbb\ncccccc\nddddd\neeeeee\nffffff"
            );

            Ok::<(), Error>(())
        })?;

        view.read(&app, |view, _| {
            assert_eq!(view.internal_clipboard, "aaaaa\nbb\nd");
            Result::<()>::Ok(())
        })?;

        Ok(())
    })
}

#[test]
fn test_autosuggestions() -> Result<()> {
    use warpui::text_layout::LayoutCache;

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let layout_cache = LayoutCache::new();
        let size = vec2f(f32::MAX, f32::MAX);

        // Create a view with autosuggestion text "foo\nbar". At this point the editor should
        // have "bazz" within its buffer and "foo\nbar" as an autosuggestion.
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut view = EditorView::new_with_base_text("bazz", Default::default(), ctx);
            view.set_autosuggestion(
                "foo\nbar",
                AutosuggestionLocation::EndOfBuffer,
                AutosuggestionType::Command {
                    was_intelligent_autosuggestion: false,
                },
                ctx,
            );
            view
        });

        // The editor should have 2 lines
        view.read(&app, |view, app| {
            let snapshot = view.snapshot(app);
            let layouts =
                snapshot.layout_autosuggestion(0.0, app.font_cache(), &layout_cache, &size, false);
            assert_eq!(layouts.len(), 2);
            Result::<()>::Ok(())
        })?;

        // Insert "foo\n" into the buffer, the autosuggestion should now be a single line with
        // the text "bar".
        view.update(&mut app, |view, ctx| {
            view.move_to_buffer_end(ctx);
            view.user_insert("foo\n", ctx);
        });

        view.read(&app, |view, app| {
            assert_eq!("bazzfoo\n", view.displayed_text(app));

            let snapshot = view.snapshot(app);
            let layouts =
                snapshot.layout_autosuggestion(0.0, app.font_cache(), &layout_cache, &size, false);
            assert_eq!(layouts.len(), 1);

            assert_eq!(Some("bar"), view.current_autosuggestion_text());
            Result::<()>::Ok(())
        })?;

        // Replace all the text in the buffer with "a"--the autosuggestion should no longer be
        // visible.
        view.update(&mut app, |view, ctx| {
            view.select_all(ctx);
            view.user_insert("a", ctx);
        });

        view.read(&app, |view, app| {
            let snapshot = view.snapshot(app);
            let layouts =
                snapshot.layout_autosuggestion(0.0, app.font_cache(), &layout_cache, &size, false);
            assert_eq!(layouts.len(), 0);

            assert_eq!(None, view.current_autosuggestion_text());
            Result::<()>::Ok(())
        })?;

        // Insert "bazz" back into the editor and then try to move right--the autosuggestion
        // text should be inserted into the buffer.
        view.update(&mut app, |view, ctx| {
            view.select_all(ctx);
            view.user_insert("bazz", ctx);
        });

        view.read(&app, |view, app| {
            assert_eq!("bazz", view.displayed_text(app));
        });

        view.update(&mut app, |view, ctx| {
            view.move_to_buffer_end(ctx);
            view.move_right(/* stop at line end */ false, ctx);
        });

        view.read(&app, |view, app| {
            let snapshot = view.snapshot(app);
            let layouts =
                snapshot.layout_autosuggestion(0.0, app.font_cache(), &layout_cache, &size, false);
            assert_eq!(layouts.len(), 0);

            assert_eq!(None, view.current_autosuggestion_text());
            assert_eq!("bazzfoo\nbar", view.displayed_text(app));

            Result::<()>::Ok(())
        })?;

        Ok(())
    })
}

#[test]
fn test_partial_autosuggestion() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Create a view with autosuggestion text "foo\nbar". At this point the editor should
        // have "bazz" within its buffer and "foo\nbar" as an autosuggestion.
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut view = EditorView::new_with_base_text("bazz", Default::default(), ctx);
            view.set_autosuggestion(
                "foo\nbar",
                AutosuggestionLocation::EndOfBuffer,
                AutosuggestionType::Command {
                    was_intelligent_autosuggestion: false,
                },
                ctx,
            );
            view
        });

        // Case 1: Test out completion within a word and completion including newlines
        view.update(&mut app, |view, ctx| {
            view.move_to_buffer_end(ctx);
            view.insert_autosuggestion(move_single_word, ctx);
        });

        view.read(&app, |view, app| {
            assert_eq!("bazzfoo", view.displayed_text(app));
            Result::<()>::Ok(())
        })?;

        view.update(&mut app, |view, ctx| {
            view.insert_autosuggestion(move_single_word, ctx);
        });

        view.read(&app, |view, app| {
            assert_eq!("bazzfoo\nbar", view.displayed_text(app));
            Result::<()>::Ok(())
        })?;

        // Case 2: Test out word boundaries in multibyte characters
        view.update(&mut app, |view, ctx| {
            view.select_all(ctx);
            view.user_insert("test", ctx);
            view.set_autosuggestion(
                " {восибing}",
                AutosuggestionLocation::EndOfBuffer,
                AutosuggestionType::Command {
                    was_intelligent_autosuggestion: false,
                },
                ctx,
            );
        });

        view.update(&mut app, |view, ctx| {
            view.insert_autosuggestion(move_single_word, ctx);
        });
        view.read(&app, |view, app| {
            assert_eq!("test {восибing", view.displayed_text(app));
            Result::<()>::Ok(())
        })?;

        view.update(&mut app, |view, ctx| {
            view.insert_autosuggestion(move_single_word, ctx);
        });
        view.read(&app, |view, app| {
            assert_eq!("test {восибing}", view.displayed_text(app));
            Result::<()>::Ok(())
        })?;

        // Case 3: Test out word boundaries within same line
        // Using example supplied by user in Github issue #488
        view.update(&mut app, |view, ctx| {
            view.select_all(ctx);
            view.user_insert("grep", ctx);
            view.set_autosuggestion(
                " --files-without-match \"git\" ~/.oh-my-zsh/themes/*",
                AutosuggestionLocation::EndOfBuffer,
                AutosuggestionType::Command {
                    was_intelligent_autosuggestion: false,
                },
                ctx,
            );
        });

        view.update(&mut app, |view, ctx| {
            view.insert_autosuggestion(move_single_word, ctx);
        });
        view.read(&app, |view, app| {
            assert_eq!("grep --files", view.displayed_text(app));
            Result::<()>::Ok(())
        })?;

        view.update(&mut app, |view, ctx| {
            view.insert_autosuggestion(move_single_word, ctx);
        });
        view.read(&app, |view, app| {
            assert_eq!("grep --files-without", view.displayed_text(app));
            Result::<()>::Ok(())
        })?;

        view.update(&mut app, |view, ctx| {
            view.insert_autosuggestion(move_single_word, ctx);
        });
        view.read(&app, |view, app| {
            assert_eq!("grep --files-without-match", view.displayed_text(app));
            Result::<()>::Ok(())
        })?;

        view.update(&mut app, |view, ctx| {
            view.insert_autosuggestion(move_single_word, ctx);
        });
        view.read(&app, |view, app| {
            assert_eq!("grep --files-without-match \"git", view.displayed_text(app));
            Result::<()>::Ok(())
        })?;

        view.update(&mut app, |view, ctx| {
            view.insert_autosuggestion(move_single_word, ctx);
        });
        view.read(&app, |view, app| {
            assert_eq!(
                "grep --files-without-match \"git\" ~/.oh",
                view.displayed_text(app)
            );
            Result::<()>::Ok(())
        })?;

        view.update(&mut app, |view, ctx| {
            view.insert_autosuggestion(move_single_word, ctx);
        });
        view.read(&app, |view, app| {
            assert_eq!(
                "grep --files-without-match \"git\" ~/.oh-my",
                view.displayed_text(app)
            );
            Result::<()>::Ok(())
        })?;

        view.update(&mut app, |view, ctx| {
            view.insert_autosuggestion(move_single_word, ctx);
        });
        view.read(&app, |view, app| {
            assert_eq!(
                "grep --files-without-match \"git\" ~/.oh-my-zsh",
                view.displayed_text(app)
            );
            Result::<()>::Ok(())
        })?;

        view.update(&mut app, |view, ctx| {
            view.insert_autosuggestion(move_single_word, ctx);
        });
        view.read(&app, |view, app| {
            assert_eq!(
                "grep --files-without-match \"git\" ~/.oh-my-zsh/themes",
                view.displayed_text(app)
            );
            Result::<()>::Ok(())
        })?;

        view.update(&mut app, |view, ctx| {
            view.insert_autosuggestion(move_single_word, ctx);
        });
        view.read(&app, |view, app| {
            assert_eq!(
                "grep --files-without-match \"git\" ~/.oh-my-zsh/themes/*",
                view.displayed_text(app)
            );
            Result::<()>::Ok(())
        })?;

        Ok(())
    })
}

#[test]
fn test_placeholder_text() {
    use warpui::text_layout::LayoutCache;

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let layout_cache = LayoutCache::new();

        // Create a view with placeholder text
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut view = EditorView::new(Default::default(), ctx);
            view.set_placeholder_text("sample instruction\ntwo lines", ctx);
            view
        });

        let no_constraint_size = Vector2F::new(f32::MAX, f32::MAX);

        // The editor should have 2 line, from placeholder text (no indent)
        view.read(&app, |view, app| {
            let snapshot = view.snapshot(app);
            let default_placeholder = snapshot.placeholder_texts.get("").unwrap();
            let layouts = snapshot.layout_placeholder_text(
                default_placeholder,
                0.,
                app.font_cache(),
                &layout_cache,
                &no_constraint_size,
                false,
            );
            assert_eq!(layouts.len(), 2);
            assert!(view.displayed_text(app).is_empty());
        });

        // Call focus, still there
        view.update(&mut app, |_view, ctx| {
            ctx.focus_self();
        });

        view.read(&app, |view, app| {
            let snapshot = view.snapshot(app);
            let default_placeholder = snapshot.placeholder_texts.get("").unwrap();
            let layouts = snapshot.layout_placeholder_text(
                default_placeholder,
                0.,
                app.font_cache(),
                &layout_cache,
                &no_constraint_size,
                false,
            );
            assert_eq!(layouts.len(), 2);
            assert!(view.displayed_text(app).is_empty());
        });

        // Test with indent > 0
        view.read(&app, |view, app| {
            let snapshot = view.snapshot(app);
            let default_placeholder = snapshot.placeholder_texts.get("").unwrap();
            let layouts = snapshot.layout_placeholder_text(
                default_placeholder,
                50.,
                app.font_cache(),
                &layout_cache,
                &no_constraint_size,
                false,
            );
            assert_eq!(layouts.len(), 2);
            assert!(view.displayed_text(app).is_empty());
        });

        // Placeholder text cannot be accepted (while autosuggestion text can)
        view.update(&mut app, |view, ctx| {
            view.move_to_buffer_end(ctx);
            view.move_right(/* stop at line end */ false, ctx);
        });

        view.read(&app, |view, app| {
            assert!(view.displayed_text(app).is_empty());
        });

        // Shows autosuggestion instead if available
        view.update(&mut app, |view, ctx| {
            view.set_autosuggestion(
                "single line",
                AutosuggestionLocation::EndOfBuffer,
                AutosuggestionType::Command {
                    was_intelligent_autosuggestion: false,
                },
                ctx,
            );
        });

        view.read(&app, |view, app| {
            assert!(view.displayed_text(app).is_empty());
        });

        view.update(&mut app, |view, ctx| {
            view.move_to_buffer_end(ctx);
            view.move_right(/* stop at line end */ false, ctx);
        });

        view.read(&app, |view, app| {
            assert_eq!("single line", view.displayed_text(app));
        });

        view.update(&mut app, |view, ctx| {
            view.clear_autosuggestion(ctx);
            view.clear_buffer_and_reset_undo_stack(ctx);
        });

        // Type out matching/anything overwrites placeholder text
        view.update(&mut app, |view, ctx| {
            view.user_insert("ls", ctx);
        });

        view.read(&app, |view, app| {
            assert_eq!("ls", view.displayed_text(app));
        });
    });
}

#[test]
fn test_delete_newline_and_end_of_line() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text("abcde\nfghijk", Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.select_ranges(
                vec![
                    // Selection should get deleted.
                    DisplayPoint::new(0, 1)..DisplayPoint::new(0, 2),
                    // Single cursor should forward-delete.
                    DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3),
                    // Single cursor at end of line should delete newline.
                    DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5),
                    // Cursor at very end of buffer should no-op.
                    DisplayPoint::new(1, 6)..DisplayPoint::new(1, 6),
                ],
                ctx,
            )?;
            view.delete(ctx);
            assert_eq!(view.buffer_text(ctx), "acefghijk");

            let model = view.model().as_ref(ctx);
            let mut selections = view
                .selections(ctx)
                .iter()
                .map(|selection| {
                    assert!(
                        model.is_cursor_only(selection, ctx),
                        "Should only have cursors now"
                    );
                    selection
                        .start()
                        .to_display_point(model.display_map(ctx), ctx)
                        .unwrap()
                })
                .collect::<Vec<_>>();
            selections.sort();
            assert_eq!(
                selections,
                vec![
                    DisplayPoint::new(0, 1),
                    DisplayPoint::new(0, 2),
                    DisplayPoint::new(0, 3),
                    DisplayPoint::new(0, 9),
                ],
                "Expected cursors at particular locations"
            );

            view.undo(ctx);
            assert_eq!(view.buffer_text(ctx), "abcde\nfghijk");
            let selections = view
                .selections(ctx)
                .iter()
                .map(|selection| {
                    selection
                        .start()
                        .to_display_point(view.editor_model.as_ref(ctx).display_map(ctx), ctx)
                        .unwrap()
                        ..selection
                            .end()
                            .to_display_point(view.editor_model.as_ref(ctx).display_map(ctx), ctx)
                            .unwrap()
                })
                .collect::<Vec<_>>();
            assert_eq!(
                selections,
                vec![
                    DisplayPoint::new(0, 1)..DisplayPoint::new(0, 2),
                    DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3),
                    DisplayPoint::new(0, 5)..DisplayPoint::new(0, 5),
                    DisplayPoint::new(1, 6)..DisplayPoint::new(1, 6),
                ],
                "Expected selections to be restored to the original state"
            );

            view.redo(ctx);
            assert_eq!(view.buffer_text(ctx), "acefghijk");

            let model = view.model().as_ref(ctx);
            let mut selections = view
                .selections(ctx)
                .iter()
                .map(|selection| {
                    assert!(
                        model.is_cursor_only(selection, ctx),
                        "Should only have cursors now"
                    );
                    selection
                        .start()
                        .to_display_point(model.display_map(ctx), ctx)
                        .unwrap()
                })
                .collect::<Vec<_>>();
            selections.sort();
            assert_eq!(
                selections,
                vec![
                    DisplayPoint::new(0, 1),
                    DisplayPoint::new(0, 2),
                    DisplayPoint::new(0, 3),
                    DisplayPoint::new(0, 9),
                ],
                "Expected cursors at particular locations"
            );
            Ok::<(), Error>(())
        })?;

        Ok(())
    })
}

#[test]
fn test_delete_empty_buffer() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new(Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)], ctx)?;
            view.delete(ctx);
            assert_eq!(view.buffer_text(ctx), "".to_string());

            view.undo(ctx);
            assert_eq!(view.buffer_text(ctx), "".to_string());
            view.redo(ctx);
            assert_eq!(view.buffer_text(ctx), "".to_string());

            Ok::<(), Error>(())
        })?;

        Ok(())
    })
}

#[test]
fn test_delete_end_of_buffer() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text("abc", Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)], ctx)?;
            view.delete(ctx);
            assert_eq!(view.buffer_text(ctx), "abc".to_string());

            view.undo(ctx);
            assert_eq!(view.buffer_text(ctx), "abc".to_string());
            view.redo(ctx);
            assert_eq!(view.buffer_text(ctx), "abc".to_string());

            Ok::<(), Error>(())
        })?;

        Ok(())
    })
}

#[test]
fn test_fold() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let base_text = "
            impl Foo {
                // Hello!

                fn a() {
                    1
                }

                fn b() {
                    2
                }

                fn c() {
                    3
                }
            }
        "
        .unindent();

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(base_text, Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.select_ranges(Some(DisplayPoint::new(8, 0)..DisplayPoint::new(12, 0)), ctx)?;
            view.fold(ctx);
            assert_eq!(
                view.displayed_text(ctx),
                "
                    impl Foo {
                        // Hello!

                        fn a() {
                            1
                        }

                        fn b() {…
                        }

                        fn c() {…
                        }
                    }
                "
                .unindent(),
            );

            view.fold(ctx);
            assert_eq!(
                view.displayed_text(ctx),
                "
                    impl Foo {…
                    }
                "
                .unindent(),
            );

            view.unfold(ctx);
            assert_eq!(
                view.displayed_text(ctx),
                "
                    impl Foo {
                        // Hello!

                        fn a() {
                            1
                        }

                        fn b() {…
                        }

                        fn c() {…
                        }
                    }
                    "
                .unindent(),
            );

            view.unfold(ctx);
            assert_eq!(view.displayed_text(ctx), view.buffer_text(ctx));

            Ok::<(), Error>(())
        })?;

        Ok(())
    })
}

#[test]
fn test_cursor_top_cursor_bottom() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(sample_text(6, 6), Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.select_ranges(
                vec![
                    DisplayPoint::new(0, 1)..DisplayPoint::new(0, 2),
                    DisplayPoint::new(3, 5)..DisplayPoint::new(3, 5),
                ],
                ctx,
            )?;
            view.cursor_top(ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            view.cursor_bottom(ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(5, 6)..DisplayPoint::new(5, 6)]
            );
            Ok::<(), Error>(())
        })?;
        Ok(())
    })
}

#[test]
fn test_move_cursor() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(sample_text(6, 6), Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.edit(
                ctx,
                Edits::new().with_update_buffer(
                    PlainTextEditorViewAction::Tab,
                    EditOrigin::UserInitiated,
                    |model, ctx| {
                        model
                            .buffer_edit(
                                vec![
                                    Point::new(1, 0)..Point::new(1, 0),
                                    Point::new(1, 1)..Point::new(1, 1),
                                ],
                                "\t",
                                ctx,
                            )
                            .unwrap()
                    },
                ),
            )
        });

        view.update(&mut app, |view, ctx| {
            view.move_down(ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
            view.move_right(/* stop at line end */ false, ctx);
            view.model().update(ctx, |editor_model, ctx| {
                let a11y_label = editor_model
                    .delta_for_a11y(ByteOffset::from(7)..ByteOffset::from(7), false, ctx)
                    .value;
                assert_eq!(a11y_label, "tab");
            });
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4)]
            );
            Ok::<(), Error>(())
        })?;

        view.update(&mut app, |view, ctx| {
            view.select_ranges(
                vec![
                    DisplayPoint::new(0, 0)..DisplayPoint::new(1, 2),
                    DisplayPoint::new(3, 3)..DisplayPoint::new(3, 3),
                ],
                ctx,
            )?;
            view.cursor_home(ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[
                    DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0),
                    DisplayPoint::new(3, 0)..DisplayPoint::new(3, 0)
                ]
            );
            view.select_ranges(
                vec![
                    DisplayPoint::new(3, 3)..DisplayPoint::new(3, 3),
                    DisplayPoint::new(4, 4)..DisplayPoint::new(4, 2),
                ],
                ctx,
            )?;
            view.cursor_end(ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[
                    DisplayPoint::new(3, 6)..DisplayPoint::new(3, 6),
                    DisplayPoint::new(4, 6)..DisplayPoint::new(4, 6),
                ]
            );
            Ok::<(), Error>(())
        })?;

        view.update(&mut app, |view, ctx| {
            view.edit(
                ctx,
                Edits::new().with_update_buffer(
                    PlainTextEditorViewAction::Tab,
                    EditOrigin::UserInitiated,
                    |model, ctx| {
                        model
                            .buffer_edit(vec![Point::new(3, 0)..Point::new(3, 0)], "    ", ctx)
                            .unwrap()
                    },
                ),
            )
        });

        view.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(3, 5)..DisplayPoint::new(3, 6)], ctx)?;
            view.cursor_home(ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(3, 4)..DisplayPoint::new(3, 4)]
            );
            view.cursor_home(ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(3, 0)..DisplayPoint::new(3, 0)]
            );
            Ok::<(), Error>(())
        })?;

        Ok(())
    })
}

#[test]
fn test_move_forward_one_word() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let base_text = r"
            aaa bbb..ccc
            ddd eee"
            .unindent();

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(base_text, Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1)], ctx)?;
            view.cursor_forward_one_word(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
            view.cursor_forward_one_word(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(0, 7)..DisplayPoint::new(0, 7)]
            );
            view.cursor_forward_one_word(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(0, 12)..DisplayPoint::new(0, 12)]
            );
            view.cursor_forward_one_word(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(1, 3)..DisplayPoint::new(1, 3)]
            );
            view.cursor_forward_one_word(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(1, 7)..DisplayPoint::new(1, 7)]
            );
            view.cursor_forward_one_word(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(1, 7)..DisplayPoint::new(1, 7)]
            );
            Ok::<(), Error>(())
        })?;

        Ok(())
    })
}

#[test]
fn test_move_backward_one_word() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let base_text = r"
            aaa bbb..ccc
            ddd eee"
            .unindent();
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(base_text, Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(1, 7)..DisplayPoint::new(1, 7)], ctx)?;
            view.cursor_backward_one_word(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4)]
            );
            view.cursor_backward_one_word(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
            view.cursor_backward_one_word(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(0, 9)..DisplayPoint::new(0, 9)]
            );
            view.cursor_backward_one_word(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
            view.cursor_backward_one_word(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            view.cursor_backward_one_word(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            Ok::<(), Error>(())
        })?;

        Ok(())
    })
}

#[test]
fn test_move_forward_one_subword() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let base_text = r"
            aaa_bbb..ccc
            ddd_eEe"
            .unindent();
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(base_text, Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1)], ctx)?;
            view.cursor_forward_one_subword(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)]
            );
            view.cursor_forward_one_subword(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(0, 7)..DisplayPoint::new(0, 7)]
            );
            view.cursor_forward_one_subword(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(0, 12)..DisplayPoint::new(0, 12)]
            );
            view.cursor_forward_one_subword(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(1, 3)..DisplayPoint::new(1, 3)]
            );
            view.cursor_forward_one_subword(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(1, 5)..DisplayPoint::new(1, 5)]
            );
            view.cursor_forward_one_subword(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(1, 7)..DisplayPoint::new(1, 7)]
            );
            view.cursor_forward_one_subword(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(1, 7)..DisplayPoint::new(1, 7)]
            );
            Ok::<(), Error>(())
        })?;

        Ok(())
    })
}

#[test]
fn test_move_backward_one_subword() -> Result<()> {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let base_text = r"
            aaa_bbb..ccc
            ddd_eEe"
            .unindent();
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(base_text, Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(1, 7)..DisplayPoint::new(1, 7)], ctx)?;
            view.cursor_backward_one_subword(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(1, 5)..DisplayPoint::new(1, 5)]
            );
            view.cursor_backward_one_subword(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4)]
            );
            view.cursor_backward_one_subword(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)]
            );
            view.cursor_backward_one_subword(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(0, 9)..DisplayPoint::new(0, 9)]
            );
            view.cursor_backward_one_subword(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)]
            );
            view.cursor_backward_one_subword(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            view.cursor_backward_one_subword(false /* select */, ctx);
            assert_eq!(
                view.selected_ranges(ctx),
                &[DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0)]
            );
            Ok::<(), Error>(())
        })?;

        Ok(())
    })
}

#[test]
fn test_add_next_occurrence() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text(
                "start word word hello word \n hi",
                Default::default(),
                ctx,
            );
            editor
                .select_ranges(vec![DisplayPoint::new(0, 6)..DisplayPoint::new(0, 6)], ctx)
                .unwrap();
            // Select two occurrences of "word"
            editor.add_next_occurrence(ctx);
            editor.add_next_occurrence(ctx);
            editor.backspace(ctx);
            assert_eq!(editor.buffer_text(ctx), "start   hello word \n hi");

            editor.undo(ctx);
            assert_eq!(editor.buffer_text(ctx), "start word word hello word \n hi");
            editor.redo(ctx);
            assert_eq!(editor.buffer_text(ctx), "start   hello word \n hi");

            editor.delete_all(CutDirection::Left, false /* cut */, ctx);
            editor.user_insert("a", ctx);
            assert_eq!(editor.buffer_text(ctx), "a hello word \n hi");

            editor
        });
    });

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text("word word word", Default::default(), ctx);
            editor
                .select_ranges(
                    vec![
                        DisplayPoint::new(0, 2)..DisplayPoint::new(0, 2),
                        DisplayPoint::new(0, 8)..DisplayPoint::new(0, 8),
                    ],
                    ctx,
                )
                .unwrap();
            editor.add_next_occurrence(ctx);
            editor.backspace(ctx);
            assert_eq!(editor.buffer_text(ctx), "  word");

            editor.undo(ctx);
            assert_eq!(editor.buffer_text(ctx), "word word word");
            editor.redo(ctx);
            assert_eq!(editor.buffer_text(ctx), "  word");

            editor
        });
    });

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text("warpwordwarpwordwarp", Default::default(), ctx);
            editor
                .select_ranges(vec![DisplayPoint::new(0, 0)..DisplayPoint::new(0, 4)], ctx)
                .unwrap();
            // Select three occurrences of "word"
            editor.add_next_occurrence(ctx);
            editor.add_next_occurrence(ctx);
            editor.backspace(ctx);
            assert_eq!(editor.buffer_text(ctx), "wordword");

            editor.undo(ctx);
            assert_eq!(editor.buffer_text(ctx), "warpwordwarpwordwarp");
            editor.redo(ctx);
            assert_eq!(editor.buffer_text(ctx), "wordword");

            editor
        });
    });
}

#[test]
fn test_undo_redo() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text("abcd", Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(0, 4)..DisplayPoint::new(0, 4)], ctx)
                .unwrap();
            view.backspace(ctx);
            assert_eq!(view.buffer_text(ctx), "abc");

            view.move_left(/* stop at line start */ false, ctx);
            view.backspace(ctx);
            assert_eq!(view.buffer_text(ctx), "ac".to_string());

            view.backspace(ctx);
            assert_eq!(view.buffer_text(ctx), "c".to_string());
        });

        // Here and below, we make sure to query the displayed text in the next
        // event loop tick to ensure events were processed by the event loop
        // before we read.
        view.update(&mut app, |view, ctx| {
            view.undo(ctx);
        });
        view.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "abc");
            assert_eq!(view.displayed_text(ctx), "abc");
        });

        view.update(&mut app, |view, ctx| {
            view.backspace(ctx);
            assert_eq!(view.buffer_text(ctx), "ac");
        });

        view.update(&mut app, |view, ctx| {
            // This do nothing instead of redoing to "c" because we have already
            // made edits on the current state.
            view.redo(ctx);
        });
        view.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "ac");
            assert_eq!(view.displayed_text(ctx), "ac");
        });

        view.update(&mut app, |view, ctx| {
            view.set_buffer_text_ignoring_undo("foofoofoo", ctx);
            assert_eq!(view.buffer_text(ctx), "foofoofoo");
        });

        view.update(&mut app, |view, ctx| {
            // Undoing here should remove the temporary replace and restore the last state
            view.undo(ctx);
        });
        view.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "ac");
            assert_eq!(view.displayed_text(ctx), "ac");
        });

        view.update(&mut app, |view, ctx| {
            // Redo should be no-op as we should already be at the last active
            // state in the undo stack.
            view.redo(ctx);
        });
        view.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "ac");
            assert_eq!(view.displayed_text(ctx), "ac");
        });

        view.update(&mut app, |view, ctx| {
            view.set_buffer_text_ignoring_undo("foofoofoo", ctx);
            view.backspace(ctx);
            assert_eq!(view.buffer_text(ctx), "foofoofo");
        });

        view.update(&mut app, |view, ctx| {
            // Temporary replace should be inserted into the undo stack after
            // a real edit.
            view.undo(ctx);
        });
        view.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "foofoofoo");
            assert_eq!(view.displayed_text(ctx), "foofoofoo");
        });

        view.update(&mut app, |view, ctx| {
            view.redo(ctx);
        });
        view.read(&app, |view, ctx| {
            assert_eq!(view.buffer_text(ctx), "foofoofo");
            assert_eq!(view.displayed_text(ctx), "foofoofo");
        });
    });
}

#[test]
fn test_delete_all_on_ephemeral_edit() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text("a", Default::default(), ctx)
        });

        view.update(&mut app, |view, ctx| {
            view.set_buffer_text_ignoring_undo("abc\ndef", ctx);
            assert_eq!(view.buffer_text(ctx), "abc\ndef");
        });

        view.update(&mut app, |view, ctx| {
            view.select_ranges(vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)], ctx)
                .unwrap();
            view.delete_all(CutDirection::Right, true, ctx);
            assert_eq!(view.buffer_text(ctx), "abc\n");
        });
    });
}

#[test]
fn test_autocomplete_symbols() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text("abc", Default::default(), ctx);
            editor.set_autocomplete_symbols_allowed(true);
            editor
                .select_ranges(vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)], ctx)
                .unwrap();

            editor.user_insert("(", ctx);
            assert_eq!(editor.buffer_text(ctx), "abc()");

            editor.backspace(ctx);
            editor.user_insert("\"", ctx);
            assert_eq!(editor.buffer_text(ctx), "abc\"");

            editor.user_insert(" ", ctx);
            editor.user_insert("(", ctx);
            assert_eq!(editor.buffer_text(ctx), "abc\" ()");

            editor.undo(ctx);
            assert_eq!(editor.buffer_text(ctx), "abc\" ");
            editor.redo(ctx);
            assert_eq!(editor.buffer_text(ctx), "abc\" ()");

            editor.backspace(ctx);
            assert_eq!(editor.buffer_text(ctx), "abc\" ");

            editor.user_insert("(", ctx);
            editor.user_insert(")", ctx);
            assert_eq!(editor.buffer_text(ctx), "abc\" ()");

            editor
        });
    });

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text("word warp word", Default::default(), ctx);

            editor.set_autocomplete_symbols_allowed(true);
            editor
                .select_ranges(
                    vec![
                        DisplayPoint::new(0, 1)..DisplayPoint::new(0, 1),
                        DisplayPoint::new(0, 11)..DisplayPoint::new(0, 11),
                    ],
                    ctx,
                )
                .unwrap();
            editor.add_next_occurrence(ctx);
            editor.user_insert("[", ctx);
            assert_eq!(editor.buffer_text(ctx), "[word] warp [word]");

            editor.undo(ctx);
            assert_eq!(editor.buffer_text(ctx), "word warp word");
            editor.redo(ctx);
            assert_eq!(editor.buffer_text(ctx), "[word] warp [word]");

            editor
        });
    });

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Emulate a user disabling symbol autocompletion in settings.
        app.update_model(&AppEditorSettings::handle(&app), |editor_settings, ctx| {
            let _ = editor_settings.autocomplete_symbols.set_value(false, ctx);
            ctx.notify();
        });
        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            // Construct an editor which allows for symbol autocompletion,
            // if the user has enabled it.
            let options = EditorOptions {
                autocomplete_symbols: true,
                ..Default::default()
            };
            let mut editor = EditorView::new_with_base_text("word warp word", options, ctx);

            editor.cursor_end(ctx);
            editor.user_insert("(", ctx);
            assert_eq!(editor.buffer_text(ctx), "word warp word(");

            editor
        });
    });
}

#[test]
fn test_clear_text_styles() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text("word\n word\n word", Default::default(), ctx);

            // Move cursor to the beginning of the buffer.
            editor.cursor_top(ctx);

            let black_text_style = TextStyle::new().with_background_color(ColorU::black());
            editor.insert_with_styles(
                "foo ",
                &[(0.into()..3.into(), black_text_style)],
                PlainTextEditorViewAction::SystemInsert,
                ctx,
            );
            assert_eq!(editor.buffer_text(ctx), "foo word\n word\n word");

            assert_eq!(
                editor
                    .text_style_runs(ctx)
                    .filter(|text_style| text_style.text_style().background_color.is_some())
                    .collect::<Vec<_>>(),
                vec![TextRun::new(
                    "foo".into(),
                    black_text_style,
                    0.into()..3.into(),
                )],
            );

            editor.clear_text_style_runs(ctx);

            // The editor text should stay the same but the text styles should be gone.
            assert_eq!(editor.buffer_text(ctx), "foo word\n word\n word");

            assert_eq!(
                editor
                    .text_style_runs(ctx)
                    .filter(|text_style| text_style.text_style().background_color.is_some())
                    .count(),
                0
            );

            // Undoing should undo the edit, not the style.
            editor.undo(ctx);
            assert_eq!(editor.buffer_text(ctx), "word\n word\n word");
            assert_eq!(
                editor
                    .text_style_runs(ctx)
                    .filter(|text_style| text_style.text_style().background_color.is_some())
                    .count(),
                0
            );

            // Reinsert the text w/ style.
            editor.insert_with_styles(
                "foo ",
                &[(0.into()..3.into(), black_text_style)],
                PlainTextEditorViewAction::SystemInsert,
                ctx,
            );
            assert_eq!(
                editor
                    .text_style_runs(ctx)
                    .filter(|text_style| text_style.text_style().background_color.is_some())
                    .collect::<Vec<_>>(),
                vec![TextRun::new(
                    "foo".into(),
                    black_text_style,
                    0.into()..3.into(),
                )],
            );

            // Select the top line and ensure it still remains even after the text styles are
            // cleared.
            editor.select_to_line_end(ctx);
            assert_eq!(
                editor
                    .model()
                    .as_ref(ctx)
                    .selections(ctx)
                    .iter()
                    .map(|selection| editor
                        .model()
                        .as_ref(ctx)
                        .selection_to_byte_offset(selection, ctx))
                    .collect::<Vec<_>>(),
                vec![ByteOffset::from(8)..ByteOffset::from(4)]
            );

            editor.clear_text_style_runs(ctx);
            assert_eq!(
                editor
                    .model()
                    .as_ref(ctx)
                    .selections(ctx)
                    .iter()
                    .map(|selection| editor
                        .model()
                        .as_ref(ctx)
                        .selection_to_byte_offset(selection, ctx))
                    .collect::<Vec<_>>(),
                vec![ByteOffset::from(8)..ByteOffset::from(4)]
            );
            assert_eq!(editor.buffer_text(ctx), "foo word\n word\n word");

            assert_eq!(
                editor
                    .text_style_runs(ctx)
                    .filter(|text_style| text_style.text_style().background_color.is_some())
                    .count(),
                0
            );

            editor
        });
    });
}

#[test]
fn test_add_cursor() {
    // Tests functionality of add cursors above/below when line lengths are equal.
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text("word\nword\nword", Default::default(), ctx);

            // Move cursor to start of second line.
            editor
                .select_ranges(vec![DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0)], ctx)
                .unwrap();

            // Mock the `FrameLayouts` to reflect the contents of the buffer.
            let frame_layouts = FrameLayouts::new(
                vec![
                    Arc::new(TextFrame::mock("word")),
                    Arc::new(TextFrame::mock("word")),
                    Arc::new(TextFrame::mock("word")),
                ],
                0,
                4,
            );
            editor
                .editor_model
                .as_ref(ctx)
                .display_map(ctx)
                .soft_wrap_state()
                .update(frame_layouts.clone());

            // Add cursor above and below.
            // Here, the goal columns are not used because all lines are long enough.
            editor.add_cursor(NewCursorDirection::Up, ctx);

            editor
                .editor_model
                .as_ref(ctx)
                .display_map(ctx)
                .soft_wrap_state()
                .update(frame_layouts);
            editor.add_cursor(NewCursorDirection::Down, ctx);

            // Cursors should have been added above and below.
            assert_eq!(
                editor.selected_ranges(ctx),
                &[
                    DisplayPoint::new(0, 0)..DisplayPoint::new(0, 0),
                    DisplayPoint::new(1, 0)..DisplayPoint::new(1, 0),
                    DisplayPoint::new(2, 0)..DisplayPoint::new(2, 0),
                ]
            );

            // Add a space and ensure it's added in front of every cursor.
            editor.user_insert(" ", ctx);
            assert_eq!(editor.buffer_text(ctx), " word\n word\n word");

            editor
        });
    });

    // Tests functionality of add cursors when line lengths are not equal (empty selections).
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text("word\nwo\nword", Default::default(), ctx);

            // Move cursor to second last column of first line.
            editor
                .select_ranges(vec![DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3)], ctx)
                .unwrap();

            // Mock the `FrameLayouts` to reflect the contents of the buffer.
            let frame_layouts = FrameLayouts::new(
                vec![
                    Arc::new(TextFrame::mock("word")),
                    Arc::new(TextFrame::mock("wo")),
                    Arc::new(TextFrame::mock("word")),
                ],
                0,
                4,
            );
            editor
                .editor_model
                .as_ref(ctx)
                .display_map(ctx)
                .soft_wrap_state()
                .update(frame_layouts.clone());

            // Add cursors below.
            editor.add_cursor(NewCursorDirection::Down, ctx);

            editor
                .editor_model
                .as_ref(ctx)
                .display_map(ctx)
                .soft_wrap_state()
                .update(frame_layouts);
            editor.add_cursor(NewCursorDirection::Down, ctx);

            // Cursors should have been added below.
            // Since second line is too short, should just go at end of line.
            assert_eq!(
                editor.selected_ranges(ctx),
                &[
                    DisplayPoint::new(0, 3)..DisplayPoint::new(0, 3),
                    DisplayPoint::new(1, 2)..DisplayPoint::new(1, 2),
                    DisplayPoint::new(2, 3)..DisplayPoint::new(2, 3),
                ]
            );

            // Add a space and ensure it's added in front of every cursor.
            editor.user_insert(" ", ctx);
            assert_eq!(editor.buffer_text(ctx), "wor d\nwo \nwor d");

            editor
        });
    });

    // Tests functionality of add cursors when line lengths are not equal (non-empty selections).
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text(
                "words\nwo\nwor\nword\nwords",
                Default::default(),
                ctx,
            );

            // Create a selection of last 2 chars on last line.
            editor
                .select_ranges(vec![DisplayPoint::new(4, 3)..DisplayPoint::new(4, 5)], ctx)
                .unwrap();

            // Mock the `FrameLayouts` to reflect the contents of the buffer.
            let frame_layouts = FrameLayouts::new(
                vec![
                    Arc::new(TextFrame::mock("words")),
                    Arc::new(TextFrame::mock("wo")),
                    Arc::new(TextFrame::mock("wor")),
                    Arc::new(TextFrame::mock("word")),
                    Arc::new(TextFrame::mock("words")),
                ],
                0,
                5,
            );
            editor
                .editor_model
                .as_ref(ctx)
                .display_map(ctx)
                .soft_wrap_state()
                .update(frame_layouts);

            // Add cursors above.
            editor.add_cursor(NewCursorDirection::Up, ctx);
            editor.add_cursor(NewCursorDirection::Up, ctx);
            editor.add_cursor(NewCursorDirection::Up, ctx);
            editor.add_cursor(NewCursorDirection::Up, ctx);

            // Cursors should have been added above.
            // Specifically, since we started with a non-empty selection,
            // we should create new selections wherever appropriate.
            assert_eq!(
                editor.selected_ranges(ctx),
                &[
                    DisplayPoint::new(0, 3)..DisplayPoint::new(0, 5),
                    DisplayPoint::new(1, 2)..DisplayPoint::new(1, 2),
                    DisplayPoint::new(2, 3)..DisplayPoint::new(2, 3),
                    DisplayPoint::new(3, 3)..DisplayPoint::new(3, 4),
                    DisplayPoint::new(4, 3)..DisplayPoint::new(4, 5),
                ]
            );

            // Replace selections with a space.
            editor.user_insert(" ", ctx);
            assert_eq!(editor.buffer_text(ctx), "wor \nwo \nwor \nwor \nwor ");

            editor
        });
    });

    // Tests functionality of add cursors when line lengths are not equal (non-empty, reversed selections).
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text(
                "words\nwo\nwor\nword\nwords",
                Default::default(),
                ctx,
            );

            // Mock the `FrameLayouts` to reflect the contents of the buffer.
            let frame_layouts = FrameLayouts::new(
                vec![
                    Arc::new(TextFrame::mock("words")),
                    Arc::new(TextFrame::mock("wo")),
                    Arc::new(TextFrame::mock("wor")),
                    Arc::new(TextFrame::mock("word")),
                    Arc::new(TextFrame::mock("words")),
                ],
                0,
                5,
            );
            editor
                .editor_model
                .as_ref(ctx)
                .display_map(ctx)
                .soft_wrap_state()
                .update(frame_layouts.clone());

            // Create a reversed selection of 2 chars (last 2 chars of last line).
            editor
                .select_ranges(vec![DisplayPoint::new(4, 5)..DisplayPoint::new(4, 5)], ctx)
                .unwrap();
            editor.select_left(ctx);
            editor.select_left(ctx);

            // Add cursors above.
            for _ in 0..4 {
                editor
                    .editor_model
                    .as_ref(ctx)
                    .display_map(ctx)
                    .soft_wrap_state()
                    .update(frame_layouts.clone());
                editor.add_cursor(NewCursorDirection::Up, ctx);
            }

            // Cursors should have been added above.
            // Specifically, since we started with a non-empty selection,
            // we should create new selections wherever appropriate.
            assert_eq!(
                editor.selected_ranges(ctx),
                &[
                    DisplayPoint::new(0, 5)..DisplayPoint::new(0, 3),
                    DisplayPoint::new(1, 2)..DisplayPoint::new(1, 2),
                    DisplayPoint::new(2, 3)..DisplayPoint::new(2, 3),
                    DisplayPoint::new(3, 4)..DisplayPoint::new(3, 3),
                    DisplayPoint::new(4, 5)..DisplayPoint::new(4, 3),
                ]
            );

            // Replace selections with a space.
            editor.user_insert(" ", ctx);
            assert_eq!(editor.buffer_text(ctx), "wor \nwo \nwor \nwor \nwor ");

            editor
        });
    });

    // Tests that existing multi-cursors stay intact.
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text("word\nword\nword", Default::default(), ctx);

            // Mock the `FrameLayouts` to reflect the contents of the buffer.
            let frame_layouts = FrameLayouts::new(
                vec![
                    Arc::new(TextFrame::mock("words")),
                    Arc::new(TextFrame::mock("word")),
                    Arc::new(TextFrame::mock("word")),
                    Arc::new(TextFrame::mock("word")),
                ],
                0,
                4,
            );
            editor
                .editor_model
                .as_ref(ctx)
                .display_map(ctx)
                .soft_wrap_state()
                .update(frame_layouts);

            // Add cursors (mimicking cursors added via mouse).
            let existing_cursors = vec![
                DisplayPoint::new(1, 1)..DisplayPoint::new(1, 2),
                DisplayPoint::new(2, 2)..DisplayPoint::new(2, 3),
                DisplayPoint::new(2, 4)..DisplayPoint::new(2, 4),
            ];
            editor.select_ranges(existing_cursors, ctx).unwrap();

            // Add cursor above.
            editor.add_cursor(NewCursorDirection::Up, ctx);

            // Existing cursors should remain in tact and new cursors
            // should merge with old ones if possible.
            assert_eq!(
                editor.selected_ranges(ctx),
                &[
                    DisplayPoint::new(0, 1)..DisplayPoint::new(0, 2),
                    DisplayPoint::new(1, 1)..DisplayPoint::new(1, 3),
                    DisplayPoint::new(1, 4)..DisplayPoint::new(1, 4),
                    DisplayPoint::new(2, 2)..DisplayPoint::new(2, 3),
                    DisplayPoint::new(2, 4)..DisplayPoint::new(2, 4),
                ]
            );

            // Add a space and ensure it was inserted at existing/new cursors.
            editor.user_insert(" ", ctx);
            assert_eq!(editor.buffer_text(ctx), "w rd\nw d \nwo d ");

            editor
        });
    });

    // Tests that adding cursors above first row and below last row is a no-op.
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text("word", Default::default(), ctx);

            // Mock the `FrameLayouts` to reflect the contents of the buffer.
            let frame_layouts = FrameLayouts::new(vec![Arc::new(TextFrame::mock("word"))], 0, 2);

            // Move cursor to middle of first (and only) line.
            editor
                .select_ranges(vec![DisplayPoint::new(0, 2)..DisplayPoint::new(0, 2)], ctx)
                .unwrap();

            // Add cursor above and below.
            editor.add_cursor(NewCursorDirection::Up, ctx);

            editor
                .editor_model
                .as_ref(ctx)
                .display_map(ctx)
                .soft_wrap_state()
                .update(frame_layouts);
            editor.add_cursor(NewCursorDirection::Down, ctx);

            // Ensure that there aren't any new cursors.
            assert_eq!(
                editor.selected_ranges(ctx),
                &[DisplayPoint::new(0, 2)..DisplayPoint::new(0, 2),]
            );

            editor
        });
    });
}

#[test]
fn test_autoscroll_vertical() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Initialize a frame layout with a single logical line laid out into
        // three softwrapped lines.
        let frame_layouts = FrameLayouts::new(
            vec![
                Arc::new(TextFrame::mock("word")),
                Arc::new(TextFrame::mock("word")),
                Arc::new(TextFrame::mock("word")),
            ],
            0,
            1,
        );

        let frame_layouts_clone = frame_layouts.clone();

        // Update the softwrapped state in the editor model.
        let (_, view) = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
            let editor = EditorView::new_with_base_text("wordwordword", Default::default(), ctx);

            editor
                .editor_model
                .as_ref(ctx)
                .display_map(ctx)
                .soft_wrap_state()
                .update(frame_layouts_clone);

            editor
        });

        // By default, the scroll position should be fixed to 0.
        view.read(&app, |view, app| {
            let snapshot = view.snapshot(app);
            let scroll_state: ScrollState = view.into();

            snapshot.autoscroll_vertically(&scroll_state, 3., 1., 0., &frame_layouts, app);
            assert_eq!(scroll_state.scroll_position().y(), 0.);
        });

        // Scrolling down to 2. should keep the scroll position to 2. since this is valid.
        view.update(&mut app, |view, app| {
            view.scroll(vec2f(0., 2.), app);
            let snapshot = view.snapshot(app);
            let scroll_state = ScrollState::from(&*view);

            assert_eq!(scroll_state.scroll_position().y(), 2.);
            snapshot.autoscroll_vertically(&scroll_state, 3., 1., 0., &frame_layouts, app);
            assert_eq!(scroll_state.scroll_position().y(), 2.);
        });

        // Scrolling down to 3. should clamp the scroll position to 2. since this is the max scroll position.
        view.update(&mut app, |view, app| {
            view.scroll(vec2f(0., 3.), app);
            let snapshot = view.snapshot(app);
            let scroll_state = ScrollState::from(&*view);

            assert_eq!(scroll_state.scroll_position().y(), 3.);
            snapshot.autoscroll_vertically(&scroll_state, 3., 1., 0., &frame_layouts, app);
            assert_eq!(scroll_state.scroll_position().y(), 2.);
        });
    });
}

#[test]
fn test_cursor_blink() {
    // Tests functionality of cursor blinking behavior
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (window_id, editor_handle) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::single_line(Default::default(), ctx);
            editor.on_focus(&FocusContext::SelfFocused, ctx);
            // cursors not visible in non-active window
            assert!(!editor.cursors_visible);
            editor
        });
        // set active window ID to enable cursor drawing and blinking
        let windowing_state = app.get_singleton_model_handle::<WindowManager>();
        app.update_model(&windowing_state, |state, ctx| {
            state.overwrite_for_test(state.stage(), Some(window_id));
            ctx.notify();
        });
        app.update_view(&editor_handle, |editor, ctx| {
            let mut epoch = editor.next_blink_epoch();
            editor.blink_cursors(epoch, ctx);
            assert!(editor.cursors_visible);
            epoch = editor.next_blink_epoch();
            editor.blink_cursors(epoch, ctx);
            assert!(!editor.cursors_visible);
        });
        // change the blink setting to disabled
        let editor_settings_handle = app.get_singleton_model_handle::<AppEditorSettings>();
        app.update_model(&editor_settings_handle, |settings, ctx| {
            let _ = settings.cursor_blink.set_value(CursorBlink::Disabled, ctx);
            ctx.notify();
        });
        app.update_view(&editor_handle, |editor, ctx| {
            let mut epoch = editor.next_blink_epoch();
            editor.blink_cursors(epoch, ctx);
            assert!(editor.cursors_visible);
            epoch = editor.next_blink_epoch();
            editor.blink_cursors(epoch, ctx);
            assert!(editor.cursors_visible);
        });
    });
}

#[test]
fn test_interaction_state() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor =
                EditorView::new_with_base_text("some test sentence", Default::default(), ctx);

            // When InteractionState::Editable, edits and selections registered
            assert!(matches!(
                editor.interaction_state(ctx),
                InteractionState::Editable
            ));
            editor.insert_selected_text("test string and ", ctx);
            let post_edit_str = "test string and some test sentence";
            assert_eq!(editor.buffer_text(ctx), post_edit_str.to_string());

            let prev_selection = editor.model().as_ref(ctx).first_selection(ctx).clone();
            editor.select_to_line_end(ctx);
            let new_selection = editor.model().as_ref(ctx).first_selection(ctx).clone();
            assert_ne!(prev_selection, new_selection);

            // When InteractionState::Selectable, selections registered but edits are not
            editor.set_interaction_state(InteractionState::Selectable, ctx);
            editor.insert_selected_text("another entry", ctx);
            assert_eq!(editor.buffer_text(ctx), post_edit_str.to_string());

            let prev_selection = editor.model().as_ref(ctx).first_selection(ctx).clone();
            editor.move_left(/* stop at line start */ false, ctx);
            let new_selection = editor.model().as_ref(ctx).first_selection(ctx).clone();
            assert_ne!(prev_selection, new_selection);

            // When InteractionState::Disabled, selections and edits are not registered
            editor.set_interaction_state(InteractionState::Disabled, ctx);
            editor.insert_selected_text("one more entry", ctx);
            assert_eq!(editor.buffer_text(ctx), post_edit_str.to_string());

            let prev_selection = editor.model().as_ref(ctx).first_selection(ctx).clone();
            editor.move_to_buffer_end(ctx);
            let new_selection = editor.model().as_ref(ctx).first_selection(ctx).clone();
            assert_eq!(prev_selection, new_selection);

            editor
        });
    });
}

#[test]
fn test_select_next_occurrence() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new(Default::default(), ctx);
            editor.user_insert("foo bar foo", ctx);

            editor
        });

        editor.update(&mut app, |editor, ctx| {
            editor.add_next_occurrence(ctx);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.add_next_occurrence(ctx);
        });

        // Both instances of foo should be selected
        editor.update(&mut app, |editor, ctx| {
            assert_eq!(
                editor.selected_ranges(ctx),
                &[
                    DisplayPoint::new(0, 0)..DisplayPoint::new(0, 3),
                    DisplayPoint::new(0, 8)..DisplayPoint::new(0, 11),
                ]
            );
        })
    });
}

#[test]
fn test_select_next_occurrence_multi_byte() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new(Default::default(), ctx);
            editor.user_insert("分分分 bar 分分分", ctx);

            editor
        });

        editor.update(&mut app, |editor, ctx| {
            editor.add_next_occurrence(ctx);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.add_next_occurrence(ctx);
        });

        // Both instances of `分分分` should be selected.
        editor.update(&mut app, |editor, ctx| {
            assert_eq!(
                editor.selected_ranges(ctx),
                &[
                    DisplayPoint::new(0, 0)..DisplayPoint::new(0, 3),
                    DisplayPoint::new(0, 8)..DisplayPoint::new(0, 11),
                ]
            );
        })
    });
}

#[test]
fn test_add_non_expanding_space() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text("word", Default::default(), ctx);
            editor.move_to_buffer_end(ctx);

            editor.add_non_expanding_space(ctx);
            assert_eq!(editor.buffer_text(ctx), "word ");

            editor.undo(ctx);
            assert_eq!(editor.buffer_text(ctx), "word");

            editor
        });
    });
}

#[test]
fn test_last_buffer_text() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new_with_base_text("a", Default::default(), ctx);
            editor.move_to_buffer_end(ctx);
            assert_eq!(editor.buffer_text(ctx), "a");

            editor.user_insert("b", ctx);
            assert_eq!(editor.buffer_text(ctx), "ab");
            assert_eq!(editor.last_buffer_text(ctx), "a");

            editor.undo(ctx);
            assert_eq!(editor.buffer_text(ctx), "a");
            assert_eq!(editor.last_buffer_text(ctx), "ab");

            editor
        });
    });
}

// This should probably be an element test but it's
// hard to construct an editor element without an editor view.
#[test]
fn test_buffer_points_to_cache() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (window_id, editor_handle) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::single_line(Default::default(), ctx);
            editor.user_insert("hello", ctx);
            editor
        });

        let point = Point::new(0, 3);
        let id = "custom_id";
        editor_handle.update(&mut app, |editor, ctx| {
            editor.cache_buffer_point(point, id, ctx);
        });

        app.update(move |ctx| {
            // We need to start and end the presenter manually because
            // currently we only do so when rendering within a Stack.
            // We might want to consider calling start / end at the start / end
            // of build_scene (respectively).
            let presenter = ctx.presenter(window_id).unwrap();
            presenter.borrow_mut().position_cache_mut().start();
            presenter
                .borrow_mut()
                .build_scene(vec2f(100., 100.), 1., None, ctx);
            presenter.borrow_mut().position_cache_mut().end();

            let expected_pos_id = position_id_for_cached_point(editor_handle.id(), id);
            assert!(presenter
                .borrow()
                .position_cache()
                .get_position(expected_pos_id)
                .is_some())
        });
    });
}

#[test]
fn test_paste_clipboard_with_text_only_should_paste_text_normally() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new(Default::default(), ctx);
            // Enable image context options to allow image attachment functionality
            // This simulates the state when Agent Mode is active and image attachments are supported
            editor.image_context_options = ImageContextOptions::Enabled {
                unsupported_model: false,
                is_processing_attached_images: false,
                num_images_attached: 0,
                num_images_in_conversation: 0,
            };
            editor
        });

        // Text-only clipboard - should paste text normally
        app.update(|ctx| {
            let clipboard_content = warpui::clipboard::ClipboardContent {
                plain_text: "hello world".to_string(),
                paths: None,
                html: None,
                images: None,
            };
            ctx.clipboard().write(clipboard_content);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.paste(ctx);
            assert_eq!(editor.buffer_text(ctx), "hello world");
        });

        // Empty images array should also fall back to text paste
        app.update(|ctx| {
            let clipboard_content = warpui::clipboard::ClipboardContent {
                plain_text: "fallback text".to_string(),
                paths: None,
                html: None,
                images: Some(vec![]), // Empty images array
            };
            ctx.clipboard().write(clipboard_content);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.clear_buffer(ctx);
            editor.paste(ctx);
            // Empty images array should be treated as text-only clipboard content
            // This tests the fallback behavior when images field exists but is empty
            assert_eq!(editor.buffer_text(ctx), "fallback text");
        });
    })
}

#[test]
fn test_paste_clipboard_with_image_only_should_switch_to_agent_mode() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new(Default::default(), ctx);
            // Enable image context options for testing
            editor.image_context_options = ImageContextOptions::Enabled {
                unsupported_model: false,
                is_processing_attached_images: false,
                num_images_attached: 0,
                num_images_in_conversation: 0,
            };
            editor
        });

        // Image-only clipboard - should switch to Agent Mode and attach image
        app.update(|ctx| {
            let png_image = warpui::clipboard::ImageData {
                data: vec![137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13], // PNG header + minimal data
                mime_type: "image/png".to_string(),
                filename: None,
            };
            let clipboard_content = warpui::clipboard::ClipboardContent {
                plain_text: "".to_string(), // No text
                paths: None,
                html: None,
                images: Some(vec![png_image]),
            };
            ctx.clipboard().write(clipboard_content);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.paste(ctx);
            // Image-only clipboard should not paste any text to the buffer
            // The image data should be processed separately via Agent Mode switching
            assert_eq!(editor.buffer_text(ctx), "");
            // TODO: Add assertions for Agent Mode switch and image attachment
        });
    })
}

#[test]
fn test_paste_clipboard_with_supported_image_and_text_should_handle_both() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new(Default::default(), ctx);
            // Enable image context options for testing
            editor.image_context_options = ImageContextOptions::Enabled {
                unsupported_model: false,
                is_processing_attached_images: false,
                num_images_attached: 0,
                num_images_in_conversation: 0,
            };
            editor
        });

        // PNG (supported) image and text clipboard - should switch to Agent Mode, attach image, and paste text
        app.update(|ctx| {
            let png_image = warpui::clipboard::ImageData {
                data: vec![137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13], // PNG header + minimal data
                mime_type: "image/png".to_string(),
                filename: Some("test.png".to_string()),
            };
            let clipboard_content = warpui::clipboard::ClipboardContent {
                plain_text: "some descriptive text".to_string(),
                paths: None,
                html: None,
                images: Some(vec![png_image]),
            };
            ctx.clipboard().write(clipboard_content);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.paste(ctx);
            // When clipboard contains both supported image and text, both should be handled:
            // - Text content gets pasted to the buffer
            // - Image triggers Agent Mode switch and attachment process
            assert_eq!(editor.buffer_text(ctx), "some descriptive text");
            // TODO: Add assertions for Agent Mode switch and image attachment
        });
    })
}

#[test]
fn test_paste_clipboard_with_unsupported_image_and_text_should_show_error() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut editor = EditorView::new(Default::default(), ctx);
            // Enable image context options for testing
            editor.image_context_options = ImageContextOptions::Enabled {
                unsupported_model: false,
                is_processing_attached_images: false,
                num_images_attached: 0,
                num_images_in_conversation: 0,
            };
            editor
        });

        // BMP (unsupported) image and text clipboard - should show error and paste text only
        app.update(|ctx| {
            let bmp_image = warpui::clipboard::ImageData {
                data: vec![66, 77, 54, 0, 0, 0, 0, 0, 0, 0, 54, 0, 0, 0], // BMP header + minimal data
                mime_type: "image/bmp".to_string(),
                filename: Some("test.bmp".to_string()),
            };
            let clipboard_content = warpui::clipboard::ClipboardContent {
                plain_text: "text with unsupported image".to_string(),
                paths: None,
                html: None,
                images: Some(vec![bmp_image]),
            };
            ctx.clipboard().write(clipboard_content);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.paste(ctx);
            // When clipboard contains unsupported image format:
            // - Text content should still be pasted normally
            // - Unsupported image should be ignored with appropriate error feedback
            assert_eq!(editor.buffer_text(ctx), "text with unsupported image");
            // TODO: Add assertions for error toast being shown
        });
    })
}

#[test]
fn test_system_delete_basic() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text("Hello, World!", Default::default(), ctx)
        });

        editor.update(&mut app, |editor, ctx| {
            // Delete "Hello" (bytes 0-5)
            editor.system_delete(ByteOffset::from(0)..ByteOffset::from(5), ctx);
            assert_eq!(editor.buffer_text(ctx), ", World!");
        });

        editor.update(&mut app, |editor, ctx| {
            // Delete ", " (bytes 0-2)
            editor.system_delete(ByteOffset::from(0)..ByteOffset::from(2), ctx);
            assert_eq!(editor.buffer_text(ctx), "World!");
        });

        editor.update(&mut app, |editor, ctx| {
            // Delete "!") (byte 5)
            editor.system_delete(ByteOffset::from(5)..ByteOffset::from(6), ctx);
            assert_eq!(editor.buffer_text(ctx), "World");
        });
    })
}

#[test]
fn test_system_delete_multibyte_characters_basic() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Test with various multi-byte UTF-8 characters
        let text_with_multibyte = "Hello 世界 🌍 café";
        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(text_with_multibyte, Default::default(), ctx)
        });

        editor.update(&mut app, |editor, ctx| {
            // Delete "Hello " (6 bytes, 6 chars)
            editor.system_delete(ByteOffset::from(0)..ByteOffset::from(6), ctx);
            assert_eq!(editor.buffer_text(ctx), "世界 🌍 café");
        });

        editor.update(&mut app, |editor, ctx| {
            // Delete "世界" (6 bytes, 2 chars - each Chinese character is 3 bytes)
            editor.system_delete(ByteOffset::from(0)..ByteOffset::from(6), ctx);
            assert_eq!(editor.buffer_text(ctx), " 🌍 café");
        });

        editor.update(&mut app, |editor, ctx| {
            // Delete " 🌍" (5 bytes: 1 space + 4 bytes for emoji)
            editor.system_delete(ByteOffset::from(0)..ByteOffset::from(5), ctx);
            assert_eq!(editor.buffer_text(ctx), " café");
        });
    })
}

#[test]
fn test_system_delete_multibyte_characters_edge_cases() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Test with only multi-byte characters
        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text("🚀🌟⭐️💫", Default::default(), ctx)
        });

        editor.update(&mut app, |editor, ctx| {
            let original_text = editor.buffer_text(ctx);
            log::info!(
                "Original text: '{}', bytes: {:?}",
                original_text,
                original_text.as_bytes()
            );

            // Delete first emoji 🚀 (4 bytes)
            editor.system_delete(ByteOffset::from(0)..ByteOffset::from(4), ctx);
            let after_first_delete = editor.buffer_text(ctx);
            log::info!(
                "After first delete: '{}', bytes: {:?}",
                after_first_delete,
                after_first_delete.as_bytes()
            );
            assert_eq!(after_first_delete, "🌟⭐️💫");
        });

        editor.update(&mut app, |editor, ctx| {
            // Delete 🌟 (4 bytes)
            editor.system_delete(ByteOffset::from(0)..ByteOffset::from(4), ctx);
            let after_second_delete = editor.buffer_text(ctx);
            log::info!(
                "After second delete: '{}', bytes: {:?}",
                after_second_delete,
                after_second_delete.as_bytes()
            );
            assert_eq!(after_second_delete, "⭐️💫");
        });
    })
}

#[test]
fn test_system_delete_various_unicode_categories() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Test with various Unicode categories: Latin, Cyrillic, Arabic, Emoji, etc.
        let unicode_text = "Hello Привет مرحبا 你好 🌍🎉";
        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new_with_base_text(unicode_text, Default::default(), ctx)
        });

        editor.update(&mut app, |editor, ctx| {
            // Delete "Hello " (ASCII - 6 bytes)
            editor.system_delete(ByteOffset::from(0)..ByteOffset::from(6), ctx);
            let text1 = editor.buffer_text(ctx);
            assert_eq!(text1, "Привет مرحبا 你好 🌍🎉");

            // Delete "Привет " (Cyrillic - 13 bytes: 6 chars × 2 bytes each + 1 space)
            editor.system_delete(ByteOffset::from(0)..ByteOffset::from(13), ctx);
            let text2 = editor.buffer_text(ctx);
            assert_eq!(text2, "مرحبا 你好 🌍🎉");

            // Delete "مرحبا " (Arabic - 11 bytes: 5 chars × 2 bytes each + 1 space)
            editor.system_delete(ByteOffset::from(0)..ByteOffset::from(11), ctx);
            let text3 = editor.buffer_text(ctx);
            assert_eq!(text3, "你好 🌍🎉");

            // Delete "你好 " (Chinese - 7 bytes: 2 chars × 3 bytes each + 1 space)
            editor.system_delete(ByteOffset::from(0)..ByteOffset::from(7), ctx);
            let text4 = editor.buffer_text(ctx);
            assert_eq!(text4, "🌍🎉");

            // Delete emojis (8 bytes: 2 emojis × 4 bytes each)
            editor.system_delete(ByteOffset::from(0)..ByteOffset::from(8), ctx);
            let text5 = editor.buffer_text(ctx);
            assert_eq!(text5, "");
        });
    })
}

#[test]
fn test_drag_and_drop_files_applies_path_transformer() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let (_, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            EditorView::new(Default::default(), ctx)
        });

        let paths = || {
            vec![
                UserInput::new(r"C:\foo\bar".to_string()),
                UserInput::new(r"D:\baz".to_string()),
            ]
        };

        view.update(&mut app, |view, ctx| {
            view.set_drag_drop_path_transformer(None);
            view.drag_and_drop_files(&paths(), ctx);
            assert_eq!(view.buffer_text(ctx), r"C:\foo\bar D:\baz ");
        });

        view.update(&mut app, |view, ctx| {
            view.clear_buffer(ctx);
            view.set_drag_drop_path_transformer(Some(Box::new(
                warp_util::path::convert_windows_path_to_wsl,
            )));
            view.drag_and_drop_files(&paths(), ctx);
            assert_eq!(view.buffer_text(ctx), "/mnt/c/foo/bar /mnt/d/baz ");
        });

        view.update(&mut app, |view, ctx| {
            view.clear_buffer(ctx);
            view.set_drag_drop_path_transformer(Some(Box::new(
                warp_util::path::convert_windows_path_to_msys2,
            )));
            view.drag_and_drop_files(&paths(), ctx);
            assert_eq!(view.buffer_text(ctx), "/c/foo/bar /d/baz ");
        });
    });
}

#[path = "vim_handler_tests.rs"]
mod vim_handler_tests;

#[path = "marked_text_tests.rs"]
mod marked_text_tests;
