use string_offset::{ByteOffset, CharOffset};
use warpui::{text_layout::TextStyle, App};

use crate::editor::{EditorSnapshot, PlainTextEditorViewAction, TextRun, ValidInputType};

use super::{EditOrigin, EditorModel, Edits, InteractionState, UpdateBufferOption};
use vec1::vec1;

#[test]
#[should_panic]
fn test_change_buffer_without_edit() {
    App::test((), |mut app| async move {
        let model =
            app.add_model(|ctx| EditorModel::new("".into(), 0, None, ValidInputType::All, ctx));

        // This should panic because we didn't edit the buffer via [`EditorModel::edit`].
        model.update(&mut app, |model, ctx| model.insert("hello", None, ctx))
    })
}

#[test]
fn test_edit_change_selections() {
    App::test((), |mut app| async move {
        let model =
            app.add_model(|ctx| EditorModel::new("abc".into(), 0, None, ValidInputType::All, ctx));

        model.read(&app, |model, ctx| {
            assert!(model.is_single_cursor_only(ctx));
            let selection = model.first_selection(ctx);
            assert_eq!(
                model.selection_to_byte_offset(selection, ctx),
                ByteOffset::from(0)..ByteOffset::from(0)
            );
        });

        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_change_selections(|model, ctx| {
                    model.cursor_line_end(/* keep_selection */ false, ctx);
                }),
            )
        });

        model.read(&app, |model, ctx| {
            assert!(model.is_single_cursor_only(ctx));
            let selection = model.first_selection(ctx);
            assert_eq!(
                model.selection_to_byte_offset(selection, ctx),
                ByteOffset::from(3)..ByteOffset::from(3)
            );
        });
    })
}

#[test]
fn test_edit_update_buffer() {
    App::test((), |mut app| async move {
        let model =
            app.add_model(|ctx| EditorModel::new("".into(), 0, None, ValidInputType::All, ctx));

        model.read(&app, |model, ctx| {
            assert!(model.buffer_text(ctx).is_empty());
        });

        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_update_buffer(
                    PlainTextEditorViewAction::ReplaceBuffer,
                    EditOrigin::UserInitiated,
                    |model, ctx| model.insert("hello", None, ctx),
                ),
            )
        });

        model.read(&app, |model, ctx| {
            assert_eq!(model.buffer_text(ctx), String::from("hello"));
        });

        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_update_buffer(
                    PlainTextEditorViewAction::ReplaceBuffer,
                    EditOrigin::UserInitiated,
                    |model, ctx| model.insert("world", None, ctx),
                ),
            )
        });

        model.read(&app, |model, ctx| {
            assert_eq!(model.buffer_text(ctx), String::from("helloworld"));
        });

        // `edit` ensures we add stuff to the undo stack correctly, so undo'ing
        // should bring us back to "hello".
        model.update(&mut app, |model, ctx| {
            model.undo(ctx);
            assert_eq!(model.buffer_text(ctx), String::from("hello"));
        });
    })
}

#[test]
fn test_edit_update_buffer_ignoring_undo() {
    App::test((), |mut app| async move {
        let model =
            app.add_model(|ctx| EditorModel::new("".into(), 0, None, ValidInputType::All, ctx));

        model.read(&app, |model, ctx| {
            assert!(model.buffer_text(ctx).is_empty());
            assert!(model.last_action(ctx).is_none());
        });

        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_update_buffer_options(
                    PlainTextEditorViewAction::ReplaceBuffer,
                    EditOrigin::UserInitiated,
                    UpdateBufferOption::SkipUndoRedoRecord,
                    |model, ctx| model.insert("hello", None, ctx),
                ),
            )
        });

        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_update_buffer_options(
                    PlainTextEditorViewAction::ReplaceBuffer,
                    EditOrigin::UserInitiated,
                    UpdateBufferOption::SkipUndoRedoRecord,
                    |model, ctx| model.insert("world", None, ctx),
                ),
            )
        });

        model.read(&app, |model, ctx| {
            assert_eq!(model.buffer_text(ctx), String::from("helloworld"));
        });

        // Since we edited and ignored undo, undo should be a no-op.
        model.update(&mut app, |model, ctx| {
            model.undo(ctx);
            assert_eq!(model.buffer_text(ctx), "helloworld".to_string());
        });
    })
}

#[test]
fn test_edit_post_buffer_edit_change_selections() {
    App::test((), |mut app| async move {
        let model =
            app.add_model(|ctx| EditorModel::new("".into(), 0, None, ValidInputType::All, ctx));

        model.read(&app, |model, ctx| {
            assert!(model.buffer_text(ctx).is_empty());

            assert!(model.is_single_cursor_only(ctx));
            let selection = model.first_selection(ctx);
            assert_eq!(
                model.selection_to_byte_offset(selection, ctx),
                ByteOffset::from(0)..ByteOffset::from(0)
            );
        });

        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new()
                    .with_update_buffer(
                        PlainTextEditorViewAction::ReplaceBuffer,
                        EditOrigin::UserInitiated,
                        |model, ctx| model.insert("hello", None, ctx),
                    )
                    .with_post_buffer_edit_change_selections(|buffer, ctx| {
                        buffer.move_to_buffer_end(false, ctx);
                    }),
            )
        });

        model.read(&app, |model, ctx| {
            assert_eq!(model.buffer_text(ctx), String::from("hello"));

            assert!(model.is_single_cursor_only(ctx));
            let selection = model.first_selection(ctx);
            assert_eq!(
                model.selection_to_byte_offset(selection, ctx),
                ByteOffset::from(5)..ByteOffset::from(5)
            );
        });
    })
}

#[test]
fn test_edit_merge_selections() {
    App::test((), |mut app| async move {
        let model = app
            .add_model(|ctx| EditorModel::new("hello".into(), 0, None, ValidInputType::All, ctx));

        model.read(&app, |model, ctx| {
            assert_eq!(model.buffer_text(ctx), "hello".to_string());

            assert!(model.is_single_cursor_only(ctx));
            let selection = model.first_selection(ctx);
            assert_eq!(
                model.selection_to_byte_offset(selection, ctx),
                ByteOffset::from(0)..ByteOffset::from(0)
            );
        });

        // Duplicate the first selection (the cursor).
        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_change_selections(|model, ctx| {
                    let first_selection = model.first_selection(ctx).to_owned();
                    model.change_selections(vec1![first_selection.clone(), first_selection], ctx);
                }),
            );
        });

        // `edit` should merge selections and de-dupe.
        model.read(&app, |model, ctx| {
            assert_eq!(model.buffer_text(ctx), "hello".to_string());

            assert!(model.is_single_cursor_only(ctx));
            let selection = model.first_selection(ctx);
            assert_eq!(
                model.selection_to_byte_offset(selection, ctx),
                ByteOffset::from(0)..ByteOffset::from(0)
            );
        });

        // Add a cursor before h and after h, and then backspace.
        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new()
                    .with_change_selections(|model, ctx| {
                        model
                            .select_ranges_by_offset(
                                [
                                    CharOffset::from(0)..CharOffset::from(0),
                                    CharOffset::from(1)..CharOffset::from(1),
                                ],
                                ctx,
                            )
                            .expect("can select ranges by offset")
                    })
                    .with_update_buffer(
                        PlainTextEditorViewAction::ReplaceBuffer,
                        EditOrigin::UserInitiated,
                        |model, ctx| model.backspace(ctx),
                    ),
            )
        });

        // The cursors should collapse into one.
        model.read(&app, |model, ctx| {
            assert_eq!(model.buffer_text(ctx), "ello".to_string());

            assert!(model.is_single_cursor_only(ctx));
            let selection = model.first_selection(ctx);
            assert_eq!(
                model.selection_to_byte_offset(selection, ctx),
                ByteOffset::from(0)..ByteOffset::from(0)
            );
        });
    })
}

#[test]
fn test_edit_when_interaction_disabled() {
    App::test((), |mut app| async move {
        let model = app.add_model(|ctx| {
            let mut model = EditorModel::new("".into(), 0, None, ValidInputType::All, ctx);
            model.set_interaction_state(InteractionState::Disabled);
            model
        });

        model.read(&app, |model, ctx| {
            assert!(model.buffer_text(ctx).is_empty());

            assert!(model.is_single_cursor_only(ctx));
            let selection = model.first_selection(ctx);
            assert_eq!(
                model.selection_to_byte_offset(selection, ctx),
                ByteOffset::from(0)..ByteOffset::from(0)
            );
        });

        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new()
                    .with_change_selections(|model, ctx| {
                        model.cursor_line_end(/* keep_selection */ false, ctx);
                    })
                    .with_update_buffer(
                        PlainTextEditorViewAction::ReplaceBuffer,
                        EditOrigin::UserInitiated,
                        |model, ctx| model.insert("hello", None, ctx),
                    ),
            )
        });

        // Neither the text nor selection state should have changed.
        model.read(&app, |model, ctx| {
            assert!(model.buffer_text(ctx).is_empty());

            assert!(model.is_single_cursor_only(ctx));
            let selection = model.first_selection(ctx);
            assert_eq!(
                model.selection_to_byte_offset(selection, ctx),
                ByteOffset::from(0)..ByteOffset::from(0)
            );
        });
    })
}

#[test]
fn test_edit_when_only_selectable() {
    App::test((), |mut app| async move {
        let model = app.add_model(|ctx| {
            let mut model = EditorModel::new("hello".into(), 0, None, ValidInputType::All, ctx);
            model.set_interaction_state(InteractionState::Selectable);
            model
        });

        model.read(&app, |model, ctx| {
            assert_eq!(model.buffer_text(ctx), "hello".to_string());

            assert!(model.is_single_cursor_only(ctx));
            let selection = model.first_selection(ctx);
            assert_eq!(
                model.selection_to_byte_offset(selection, ctx),
                ByteOffset::from(0)..ByteOffset::from(0)
            );
        });

        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new()
                    .with_change_selections(|model, ctx| {
                        model.cursor_line_end(/* keep_selection */ false, ctx);
                    })
                    .with_update_buffer(
                        PlainTextEditorViewAction::ReplaceBuffer,
                        EditOrigin::UserInitiated,
                        |model, ctx| model.insert("world", None, ctx),
                    ),
            )
        });

        // Only the selection state should have changed.
        model.read(&app, |model, ctx| {
            assert_eq!(model.buffer_text(ctx), "hello".to_string());

            assert!(model.is_single_cursor_only(ctx));
            let selection = model.first_selection(ctx);
            assert_eq!(
                model.selection_to_byte_offset(selection, ctx),
                ByteOffset::from(5)..ByteOffset::from(5)
            );
        });
    })
}

#[test]
fn test_ephemeral_edit() {
    App::test((), |mut app| async move {
        let model =
            app.add_model(|ctx| EditorModel::new("".into(), 0, None, ValidInputType::All, ctx));

        model.read(&app, |model, ctx| {
            assert!(model.buffer_text(ctx).is_empty());
            assert!(!model.is_ephemeral());
        });

        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_update_buffer(
                    PlainTextEditorViewAction::ReplaceBuffer,
                    EditOrigin::UserInitiated,
                    |model, ctx| model.insert("hello", None, ctx),
                ),
            )
        });

        // Ephemeral edits should use the latest buffer as the "base text".
        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_update_buffer_options(
                    PlainTextEditorViewAction::ReplaceBuffer,
                    EditOrigin::UserInitiated,
                    UpdateBufferOption::IsEphemeral,
                    |model, ctx| model.insert("world", None, ctx),
                ),
            )
        });

        model.read(&app, |model, ctx| {
            assert!(model.is_ephemeral());
            assert_eq!(model.buffer_text(ctx), String::from("helloworld"));
        });

        model.update(&mut app, |model, ctx| model.undo(ctx));
        model.read(&app, |model, ctx| {
            assert!(!model.is_ephemeral());
            assert_eq!(model.buffer_text(ctx), String::from("hello"));
        });
    })
}

#[test]
fn test_materialize_ephemeral_edit() {
    App::test((), |mut app| async move {
        let model =
            app.add_model(|ctx| EditorModel::new("".into(), 0, None, ValidInputType::All, ctx));

        model.read(&app, |model, ctx| {
            assert!(model.buffer_text(ctx).is_empty());
            assert!(!model.is_ephemeral());
        });

        // Make an ephemeral edit.
        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_update_buffer_options(
                    PlainTextEditorViewAction::ReplaceBuffer,
                    EditOrigin::UserInitiated,
                    UpdateBufferOption::IsEphemeral,
                    |model, ctx| model.insert("hello world", None, ctx),
                ),
            )
        });

        model.read(&app, |model, ctx| {
            assert!(model.is_ephemeral());
            assert_eq!(model.buffer_text(ctx), String::from("hello world"));
        });

        // Materialize it.
        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new()
                    .with_update_buffer(
                        PlainTextEditorViewAction::ReplaceBuffer,
                        EditOrigin::UserInitiated,
                        |model, ctx| model.insert("earth", None, ctx),
                    )
                    .with_change_selections(|model, ctx| {
                        model.select_word_left(ctx);
                    }),
            )
        });

        model.read(&app, |model, ctx| {
            assert!(!model.is_ephemeral());
            assert_eq!(model.buffer_text(ctx), String::from("hello earth"));
        });

        // Materialized ephemeral edits should be properly put on the undo stack.
        model.update(&mut app, |model, ctx| model.undo(ctx));
        model.read(&app, |model, ctx| {
            assert!(!model.is_ephemeral());
            assert_eq!(model.buffer_text(ctx), String::from("hello world"));
        });

        model.update(&mut app, |model, ctx| model.undo(ctx));
        model.read(&app, |model, ctx| {
            assert!(!model.is_ephemeral());
            assert!(model.buffer_text(ctx).is_empty());
        });
    })
}

#[test]
fn test_replace_ephemeral_edit_with_ephemeral() {
    App::test((), |mut app| async move {
        let model =
            app.add_model(|ctx| EditorModel::new("".into(), 0, None, ValidInputType::All, ctx));

        model.read(&app, |model, ctx| {
            assert!(model.buffer_text(ctx).is_empty());
            assert!(!model.is_ephemeral());
        });

        // Make an ephemeral edit.
        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_update_buffer_options(
                    PlainTextEditorViewAction::ReplaceBuffer,
                    EditOrigin::UserInitiated,
                    UpdateBufferOption::IsEphemeral,
                    |model, ctx| model.insert("hello", None, ctx),
                ),
            )
        });

        model.read(&app, |model, ctx| {
            assert!(model.is_ephemeral());
            assert_eq!(model.buffer_text(ctx), String::from("hello"));
        });

        // Make another ephemeral edit.
        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_update_buffer_options(
                    PlainTextEditorViewAction::ReplaceBuffer,
                    EditOrigin::UserInitiated,
                    UpdateBufferOption::IsEphemeral,
                    |model, ctx| model.insert("world", None, ctx),
                ),
            )
        });

        model.read(&app, |model, ctx| {
            assert!(model.is_ephemeral());
            assert_eq!(model.buffer_text(ctx), String::from("helloworld"));
        });

        // Materialize the edits.
        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_update_buffer(
                    PlainTextEditorViewAction::ReplaceBuffer,
                    EditOrigin::UserInitiated,
                    |model, ctx| model.insert("!", None, ctx),
                ),
            )
        });

        model.read(&app, |model, ctx| {
            assert!(!model.is_ephemeral());
            assert_eq!(model.buffer_text(ctx), String::from("helloworld!"));
        });

        // There should only be two entries on the undo stack: the non-ephemeral edit and one edit for the ephemeral edits.
        model.update(&mut app, |model, ctx| model.undo(ctx));
        model.read(&app, |model, ctx| {
            assert!(!model.is_ephemeral());
            assert_eq!(model.buffer_text(ctx), String::from("helloworld"));
        });

        model.update(&mut app, |model, ctx| model.undo(ctx));
        model.read(&app, |model, ctx| {
            assert!(!model.is_ephemeral());
            assert!(model.buffer_text(ctx).is_empty());
        });
    })
}

#[test]
fn test_selection_change_doesnt_materialize_ephemeral_edit() {
    App::test((), |mut app| async move {
        let model =
            app.add_model(|ctx| EditorModel::new("".into(), 0, None, ValidInputType::All, ctx));

        model.read(&app, |model, ctx| {
            assert!(model.buffer_text(ctx).is_empty());
            assert!(!model.is_ephemeral());
        });

        // Make an ephemeral edit.
        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_update_buffer_options(
                    PlainTextEditorViewAction::ReplaceBuffer,
                    EditOrigin::UserInitiated,
                    UpdateBufferOption::IsEphemeral,
                    |model, ctx| model.insert("hello", None, ctx),
                ),
            )
        });

        model.read(&app, |model, ctx| {
            assert!(model.is_ephemeral());
            assert_eq!(model.buffer_text(ctx), String::from("hello"));
        });

        // Change selections.
        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_change_selections(|model, ctx| model.select_word_left(ctx)),
            )
        });

        // The ephemeral buffer should still be active.
        model.read(&app, |model, ctx| {
            assert!(!model.selections(ctx).is_empty());
            assert!(model.is_ephemeral());
        });
    })
}

// Regression test for CORE-1549.
#[test]
fn test_restoring_invalid_selections() {
    App::test((), |mut app| async move {
        let model =
            app.add_model(|ctx| EditorModel::new("".into(), 0, None, ValidInputType::All, ctx));

        model.read(&app, |model, ctx| {
            assert!(model.buffer_text(ctx).is_empty());
            assert!(!model.is_ephemeral());
        });

        // Restore from a snapshot where the selection range is invalid.
        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_update_buffer(
                    PlainTextEditorViewAction::ReplaceBuffer,
                    EditOrigin::UserInitiated,
                    |model, ctx| {
                        let snapshot = EditorSnapshot {
                            selections: vec1![CharOffset::from(5)..CharOffset::from(6)],
                            buffer_text_runs: vec![TextRun::new(
                                "foo".into(),
                                TextStyle::new(),
                                ByteOffset::from(0)..ByteOffset::from(3),
                            )],
                        };
                        model.restore_from_snapshot(snapshot, ctx);
                    },
                ),
            )
        });

        // The selection range should fallback to (0, 0).
        model.read(&app, |model, ctx| {
            assert_eq!(model.buffer_text(ctx), "foo");
            assert_eq!(
                model.as_snapshot(ctx).selections,
                vec1![CharOffset::from(0)..CharOffset::from(0)]
            );
        });
    });
}

#[test]
fn test_undo_redo() {
    App::test((), |mut app| async move {
        let model = app
            .add_model(|ctx| EditorModel::new("foo bar".into(), 0, None, ValidInputType::All, ctx));

        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_change_selections(|model, ctx| {
                    model
                        .select_ranges_by_offset([CharOffset::from(2)..CharOffset::from(5)], ctx)
                        .unwrap();
                }),
            )
        });

        model.update(&mut app, |model, ctx| {
            model.edit(
                ctx,
                Edits::new().with_update_buffer(
                    PlainTextEditorViewAction::ReplaceBuffer,
                    EditOrigin::UserInitiated,
                    |model, ctx| {
                        model.insert("z", None, ctx);
                    },
                ),
            )
        });

        model.read(&app, |model, ctx| {
            assert_eq!(model.buffer_text(ctx), "fozar");
            assert_eq!(model.displayed_text(ctx), "fozar");
            assert_eq!(
                model.as_snapshot(ctx).selections,
                vec1![CharOffset::from(3)..CharOffset::from(3)]
            );
        });

        model.update(&mut app, |model, ctx| {
            model.undo(ctx);
        });

        model.read(&app, |model, ctx| {
            assert_eq!(model.buffer_text(ctx), "foo bar");
            assert_eq!(model.displayed_text(ctx), "foo bar");
            // TODO: we should consider making snapshot selections
            // more intuitive rather than switching between start / end
            // when snapshotting and restoring.
            assert_eq!(
                model.as_snapshot(ctx).selections,
                vec1![CharOffset::from(5)..CharOffset::from(2)]
            );
        });

        model.update(&mut app, |model, ctx| {
            model.redo(ctx);
        });

        model.read(&app, |model, ctx| {
            assert_eq!(model.buffer_text(ctx), "fozar");
            assert_eq!(model.displayed_text(ctx), "fozar");
            assert_eq!(
                model.as_snapshot(ctx).selections,
                vec1![CharOffset::from(3)..CharOffset::from(3)]
            );
        });
    });
}
