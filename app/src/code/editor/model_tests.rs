use futures::channel::oneshot;
use std::path::Path;
use vec1::vec1;
use warp_editor::content::buffer::{InitialBufferState, SelectionOffsets};
use warp_editor::multiline::MultilineString;
use warp_util::content_version::ContentVersion;
use warpui::App;

use crate::{
    code::editor::line::EditorLineLocation, code::editor::view::code_text_styles,
    settings::FontSettings, test_util::settings::initialize_settings_for_tests,
};

use super::*;

fn initialize_deps(app: &mut App) {
    app.add_singleton_model(|_| Appearance::mock());
    initialize_settings_for_tests(app);
}

fn mock_model(app: &mut App, text: &str, version: ContentVersion) -> ModelHandle<CodeEditorModel> {
    app.add_model(|ctx| {
        let styles = code_text_styles(Appearance::as_ref(ctx), FontSettings::as_ref(ctx), None);
        let mut model = CodeEditorModel::new(styles, None, false, None, ctx);
        let state = InitialBufferState::plain_text(text).with_version(version);
        model.reset_content(state, ctx);
        model.set_language_with_local_path(Path::new("/test.rs"), ctx);
        model
    })
}

fn mock_model_with_diff(
    app: &mut App,
    base_text: &str,
    current_text: &str,
    version: ContentVersion,
) -> ModelHandle<CodeEditorModel> {
    app.add_model(|ctx| {
        let styles = code_text_styles(Appearance::as_ref(ctx), FontSettings::as_ref(ctx), None);
        let mut model = CodeEditorModel::new(styles, None, false, None, ctx);
        let state = InitialBufferState::plain_text(current_text).with_version(version);
        model.reset_content(state, ctx);
        model.set_language_with_local_path(Path::new("/test.rs"), ctx);

        // Set up diff model with base text
        model.diff().update(ctx, |diff, _| {
            diff.set_base(MultilineString::apply(base_text));
        });

        model
    })
}

async fn layout_model(app: &mut App, model: &ModelHandle<CodeEditorModel>) {
    app.read(|ctx| model.as_ref(ctx).render_state.as_ref(ctx).layout_complete())
        .await;
}

#[test]
fn test_no_trailing_newline() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = mock_model(&mut app, "", ContentVersion::new());
        // We need to layout the model to be able to select by line boundaries.
        layout_model(&mut app, &editor).await;

        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(CharOffset::from(1), ctx);
        });

        // At the start we should have a trailing newline (the same height as an empty line).
        let old_height = app.read(|ctx| editor.as_ref(ctx).render_state().as_ref(ctx).height());

        editor.update(&mut app, |editor, ctx| {
            editor.insert("a", EditOrigin::UserTyped, ctx);
        });
        layout_model(&mut app, &editor).await;

        // After insertion, we should replace the trailing newline with an actual text line.
        let height_after_insertion =
            app.read(|ctx| editor.as_ref(ctx).render_state().as_ref(ctx).height());
        assert_eq!(old_height, height_after_insertion);

        editor.update(&mut app, |editor, ctx| {
            editor.enter(ctx);
        });
        layout_model(&mut app, &editor).await;

        // Enter should create a newline, which adds to the height.
        let height_after_newline =
            app.read(|ctx| editor.as_ref(ctx).render_state().as_ref(ctx).height());
        assert_ne!(old_height, height_after_newline);
    })
}

#[test]
fn test_toggle_comment() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = mock_model(&mut app, "    First\n    Second", ContentVersion::new());
        // We need to layout the model to be able to select by line boundaries.
        layout_model(&mut app, &editor).await;

        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(CharOffset::from(5), ctx);
            editor.select_to_line_end(ctx);
        });

        editor.update(&mut app, |editor, ctx| {
            assert_eq!(
                editor.selections(ctx),
                vec1![SelectionOffsets {
                    head: CharOffset::from(10),
                    tail: CharOffset::from(5)
                }]
            );
            editor.toggle_comments(ctx);
        });

        // Toggling comment for the first line.
        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.content.as_ref(ctx).text().as_str(),
                "    // First\n    Second"
            );
        });

        editor.update(&mut app, |editor, ctx| {
            editor.select_all(ctx);
        });

        // Not all selected lines have comment prefix. This should add the prefix again.
        editor.update(&mut app, |editor, ctx| {
            editor.toggle_comments(ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.content.as_ref(ctx).text().as_str(),
                "    // // First\n    // Second"
            );
        });

        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(CharOffset::from(20), ctx);
            editor.select_to_line_end(ctx);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.toggle_comments(ctx);
        });

        // Toggling comment on a line with comment should uncomment it.
        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.content.as_ref(ctx).text().as_str(),
                "    // // First\n    Second"
            );
        });
    })
}

#[test]
fn test_apply_diffs() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        // Historically, applying diffs at the very start of an empty buffer (line range 0..0)
        // could preserve the buffer's internal trailing-newline/block marker, resulting in an
        // extra newline in the final content. This guards against that by ensuring we replace the
        // implicit initial block correctly and do not end up with an extra trailing newline.
        let editor_zero = mock_model(&mut app, "", ContentVersion::new());
        layout_model(&mut app, &editor_zero).await;
        editor_zero.update(&mut app, |editor, ctx| {
            editor.apply_diffs(
                vec![DiffDelta {
                    replacement_line_range: 0..0,
                    insertion: "First\nSecond\n".to_string(),
                }],
                ctx,
            );
        });
        editor_zero.read(&app, |editor, ctx| {
            assert_eq!(
                editor.content.as_ref(ctx).text().as_str(),
                "First\nSecond\n"
            );
        });
        let editor = mock_model(
            &mut app,
            "    First\n    Second\n        Third",
            ContentVersion::new(),
        );
        // We need to layout the model to be able to select by line boundaries.
        layout_model(&mut app, &editor).await;

        // Put cursor on the second line between S and e.
        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(CharOffset::from(16), ctx);
        });

        editor.update(&mut app, |editor, ctx| {
            editor.apply_diffs(
                vec![DiffDelta {
                    replacement_line_range: 2..3,
                    insertion: "    Fourth\n    Fifth\n".to_string(),
                }],
                ctx,
            );
        });

        // Inserted content should have the same indentation level.
        // The selections should not change after the insertion.
        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.content.as_ref(ctx).text().as_str(),
                "    First\n    Fourth\n    Fifth\n        Third"
            );
            assert_eq!(
                editor.selections(ctx),
                vec1![SelectionOffsets {
                    head: CharOffset::from(16),
                    tail: CharOffset::from(16)
                }]
            );
        });
    })
}

#[test]
fn test_apply_diff_on_empty_buffer() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = mock_model(&mut app, "", ContentVersion::new());
        layout_model(&mut app, &editor).await;

        editor.update(&mut app, |editor, ctx| {
            editor.apply_diffs(
                vec![DiffDelta {
                    replacement_line_range: 0..0,
                    insertion: "First line\nSecond line\n".to_string(),
                }],
                ctx,
            );
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.content.as_ref(ctx).text().as_str(),
                "First line\nSecond line\n"
            );
            assert_eq!(
                editor.selections(ctx),
                vec1![SelectionOffsets {
                    head: CharOffset::from(24),
                    tail: CharOffset::from(24)
                }]
            );
        });
    })
}

#[test]
fn test_apply_single_deletion() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = mock_model(
            &mut app,
            "    First\n    Second\n        Third",
            ContentVersion::new(),
        );
        layout_model(&mut app, &editor).await;

        editor.update(&mut app, |editor, ctx| {
            editor.apply_diffs(
                vec![DiffDelta {
                    replacement_line_range: 1..2,
                    insertion: "".to_string(),
                }],
                ctx,
            );
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.content.as_ref(ctx).text().as_str(),
                "    Second\n        Third"
            );
        });

        editor.update(&mut app, |editor, ctx| {
            editor.apply_diffs(
                vec![DiffDelta {
                    replacement_line_range: 2..3,
                    insertion: "".to_string(),
                }],
                ctx,
            );
        });
        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.content.as_ref(ctx).text().as_str(), "    Second\n");
        });
    })
}

// #[test]
// fn test_indent() {
//     App::test((), |mut app| async move {
//         initialize_deps(&mut app);
//         let editor = mock_model(&mut app, "fn test() {\ns\n{\ntest\n}\n}");
//         layout_model(&mut app, &editor).await;

//         editor.update(&mut app, |editor, ctx| {
//             editor.cursor_at(CharOffset::from(13), ctx);
//         });

//         editor.update(&mut app, |editor, ctx| {
//             editor.indent(false, ctx);
//         });

//         // Indent with one expected indent level.
//         editor.read(&app, |editor, ctx| {
//             assert_eq!(
//                 editor.content.as_ref(ctx).text(),
//                 "fn test() {\n    s\n{\ntest\n}\n}"
//             );
//         });

//         editor.update(&mut app, |editor, ctx| {
//             editor.cursor_at(CharOffset::from(21), ctx);
//         });

//         editor.update(&mut app, |editor, ctx| {
//             editor.indent(false, ctx);
//         });

//         // Indent with two expected indent level.
//         editor.read(&app, |editor, ctx| {
//             assert_eq!(
//                 editor.content.as_ref(ctx).text(),
//                 "fn test() {\n    s\n{\n        test\n}\n}"
//             );
//         });

//         editor.update(&mut app, |editor, ctx| {
//             editor.undo(ctx);
//         });

//         editor.update(&mut app, |editor, ctx| {
//             editor.cursor_at(CharOffset::from(22), ctx);
//         });

//         editor.update(&mut app, |editor, ctx| {
//             editor.indent(false, ctx);
//         });

//         // Indent in the middle of the text should only advance one indent level.
//         editor.read(&app, |editor, ctx| {
//             // TODO(kevin): Looks like there is a bug here where the indentation should be 4,
//             // it is currently 3.
//             assert_eq!(
//                 editor.content.as_ref(ctx).text(),
//                 "fn test() {\n    s\n{\nt   est\n}\n}"
//             );
//         });
//     })
// }

// This test flakes occasionally. I think it could be due to how we are initializing tree-sitter
// in unit tests.
// TODO(kevin): Re-enable this
// #[test]
// fn test_bracket_expansion() {
//     App::test((), |mut app| async move {
//         initialize_deps(&mut app);
//         let editor = mock_model(&mut app, "fn test() {}");
//         layout_model(&mut app, &editor).await;

//         editor.update(&mut app, |editor, ctx| {
//             editor.cursor_at(CharOffset::from(9), ctx);
//         });

//         // Bracket expansion in parentheses.
//         editor.update(&mut app, |editor, ctx| {
//             editor.enter(ctx);
//         });

//         editor.read(&app, |editor, ctx| {
//             assert_eq!(
//                 editor.selections(ctx),
//                 vec1![SelectionOffsets {
//                     head: CharOffset::from(14),
//                     tail: CharOffset::from(14)
//                 }]
//             );
//             assert_eq!(editor.content.as_ref(ctx).text(), "fn test(\n    \n) {}");
//         });

//         editor.update(&mut app, |editor, ctx| {
//             editor.cursor_at(CharOffset::from(18), ctx);
//         });

//         editor.update(&mut app, |editor, ctx| {
//             editor.enter(ctx);
//         });

//         // Bracket expansion in brackets.
//         editor.read(&app, |editor, ctx| {
//             assert_eq!(
//                 editor.selections(ctx),
//                 vec1![SelectionOffsets {
//                     head: CharOffset::from(23),
//                     tail: CharOffset::from(23)
//                 }]
//             );
//             assert_eq!(
//                 editor.content.as_ref(ctx).text(),
//                 "fn test(\n    \n) {\n    \n}"
//             );
//         });
//     })
// }

#[test]
fn test_move_by_word() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = mock_model(
            &mut app,
            "fn test() {}\nfn test2() {}",
            ContentVersion::new(),
        );
        layout_model(&mut app, &editor).await;

        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(CharOffset::from(1), ctx);
            editor.add_cursor_at(14.into(), ctx);

            editor.forward_word_with_unit(true, word_unit(ctx), ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.selections(ctx),
                vec1![
                    SelectionOffsets {
                        head: CharOffset::from(3),
                        tail: CharOffset::from(1)
                    },
                    SelectionOffsets {
                        head: CharOffset::from(16),
                        tail: CharOffset::from(14)
                    }
                ]
            );
        });

        editor.update(&mut app, |editor, ctx| {
            editor.backward_word_with_unit(false, word_unit(ctx), ctx);
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.selections(ctx),
                vec1![
                    SelectionOffsets {
                        head: CharOffset::from(1),
                        tail: CharOffset::from(1)
                    },
                    SelectionOffsets {
                        head: CharOffset::from(14),
                        tail: CharOffset::from(14)
                    }
                ]
            );
        });
    })
}

#[test]
fn test_version_match() {
    App::test((), |mut app| async move {
        let initial_version = ContentVersion::new();

        initialize_deps(&mut app);
        let editor = mock_model(&mut app, "fn test() {}\nfn test2() {}", initial_version);
        layout_model(&mut app, &editor).await;

        editor.update(&mut app, |editor, ctx| {
            assert!(editor.content().as_ref(ctx).version_match(&initial_version));

            editor.insert("hey", EditOrigin::UserTyped, ctx);

            // The version should no longer match.
            assert!(!editor.content().as_ref(ctx).version_match(&initial_version));
        });
    });
}

#[test]
fn test_reset_version() {
    App::test((), |mut app| async move {
        let initial_version = ContentVersion::new();

        initialize_deps(&mut app);
        let editor = mock_model(&mut app, "fn test() {}\nfn test2() {}", initial_version);
        layout_model(&mut app, &editor).await;

        editor.update(&mut app, |editor, ctx| {
            let version2 = ContentVersion::new();

            let state = InitialBufferState::plain_text("heyo").with_version(version2);
            editor.reset(state, ctx);

            assert!(!editor.content().as_ref(ctx).version_match(&initial_version));
            assert!(editor.content().as_ref(ctx).version_match(&version2));
        });
    });
}

#[test]
fn test_retrieve_unified_diff_windows_line_endings() {
    // This is a regression test for a bug where we treated *every* line as an edit for files with non-newline line endings.
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        let editor = mock_model(&mut app, "One\r\nTwo\r\nThree\r\n", ContentVersion::new());
        layout_model(&mut app, &editor).await;

        // Modify the middle line.
        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(CharOffset::from(8), ctx);
            editor.insert("!!", EditOrigin::UserTyped, ctx);
        });

        let (diff_tx, diff_rx) = oneshot::channel();
        app.update(|ctx| {
            let mut diff_tx = Some(diff_tx);
            ctx.subscribe_to_model(&editor, move |_, event, _| {
                if let CodeEditorModelEvent::UnifiedDiffComputed(diff) = event {
                    diff_tx
                        .take()
                        .expect("Should only receive one event")
                        .send(diff.clone())
                        .expect("Receiver should exist");
                }
            });

            editor.update(ctx, |editor, ctx| {
                editor.retrieve_unified_diff("test.rs".to_string(), ctx);
            });
        });

        // Verify that we received a diff and it has the expected content.
        let diff = diff_rx.await.expect("Event should be sent");

        // One addition and one deletion, for the single modified line.
        assert_eq!(diff.lines_added, 1);
        assert_eq!(diff.lines_removed, 1);
        assert_eq!(
            diff.unified_diff,
            "--- test.rs\n+++ test.rs\n@@ -1,3 +1,3 @@\n One\n-Two\n+Two!!\n Three\n"
        );

        // Verify that the content uses the original line endings.
        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.content_string(ctx).as_str(),
                "One\r\nTwo!!\r\nThree\r\n"
            );
        });
    });
}

#[test]
fn test_retrieve_unified_diff_mixed_line_endings() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        let editor = mock_model(&mut app, "One\nTwo\nThree\r\n", ContentVersion::new());
        layout_model(&mut app, &editor).await;

        // Modify the middle line.
        editor.update(&mut app, |editor, ctx| {
            editor.cursor_at(CharOffset::from(8), ctx);
            editor.insert("!!", EditOrigin::UserTyped, ctx);
        });

        let (diff_tx, diff_rx) = oneshot::channel();
        app.update(|ctx| {
            let mut diff_tx = Some(diff_tx);
            ctx.subscribe_to_model(&editor, move |_, event, _| {
                if let CodeEditorModelEvent::UnifiedDiffComputed(diff) = event {
                    diff_tx
                        .take()
                        .expect("Should only receive one event")
                        .send(diff.clone())
                        .expect("Receiver should exist");
                }
            });

            editor.update(ctx, |editor, ctx| {
                editor.retrieve_unified_diff("test.rs".to_string(), ctx);
            });
        });

        // Verify that we received a diff and it has the expected content.
        let diff = diff_rx.await.expect("Event should be sent");

        // One addition and one deletion, for the single modified line.
        assert_eq!(diff.lines_added, 1);
        assert_eq!(diff.lines_removed, 1);
        assert_eq!(
            diff.unified_diff,
            "--- test.rs\n+++ test.rs\n@@ -1,3 +1,3 @@\n One\n-Two\n+Two!!\n Three\n"
        );

        // Verify that the content uses the original, inferred line endings.
        editor.read(&app, |editor, ctx| {
            assert_eq!(editor.content_string(ctx).as_str(), "One\nTwo!!\nThree\n");
        });
    });
}

#[test]
fn test_text_for_line() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = mock_model(
            &mut app,
            "First line\nSecond line\nThird line",
            ContentVersion::new(),
        );
        layout_model(&mut app, &editor).await;

        editor.read(&app, |editor, ctx| {
            // Test getting first line (line numbers are 1-based)
            let line1 = editor.text_for_line(LineCount::from(1), ctx);
            assert_eq!(line1, "First line\n");

            // Test getting second line
            let line2 = editor.text_for_line(LineCount::from(2), ctx);
            assert_eq!(line2, "Second line\n");

            // Test getting third line (no trailing newline in original)
            let line3 = editor.text_for_line(LineCount::from(3), ctx);
            assert_eq!(line3, "Third line");
        });
    })
}

#[test]
fn test_match_line_to_text_exact_match() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = mock_model(
            &mut app,
            "First line\nSecond line\nThird line",
            ContentVersion::new(),
        );
        layout_model(&mut app, &editor).await;

        editor.read(&app, |editor, ctx| {
            // Test exact match at the same position
            let result = editor.match_line_to_text(
                "Second line",
                1, // current_idx (0-based, so line 2)
                2, // max_line
                |me, original_text, line, ctx| {
                    let line_text = me.text_for_line(LineCount::from(line + 1), ctx);
                    line_text.trim_end_matches('\n') == original_text
                },
                ctx,
            );
            assert_eq!(result, Some(1)); // Should find at the same position
        });
    })
}

#[test]
fn test_match_line_to_text_nearby_match() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = mock_model(
            &mut app,
            "First line\nModified line\nThird line\nThird line",
            ContentVersion::new(),
        );
        layout_model(&mut app, &editor).await;

        editor.read(&app, |editor, ctx| {
            // Test finding a match nearby when original position doesn't match
            let result = editor.match_line_to_text(
                "Third line",
                1, // Look at position 1 (second line)
                3, // max_line
                |me, original_text, line, ctx| {
                    let line_text = me.text_for_line(LineCount::from(line + 1), ctx);
                    line_text.trim_end_matches('\n') == original_text
                },
                ctx,
            );
            assert_eq!(result, Some(2)); // Should find "Third line" at position 2
        });
    })
}

#[test]
fn test_match_line_to_text_search_window_expansion() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let editor = mock_model(&mut app, "A\nB\nC\nTarget\nE\nF", ContentVersion::new());
        layout_model(&mut app, &editor).await;

        editor.read(&app, |editor, ctx| {
            // Test that search window expands to find distant matches
            let result = editor.match_line_to_text(
                "Target",
                1, // Start searching from position 1 (line "B")
                5, // max_line
                |me, original_text, line, ctx| {
                    let line_text = me.text_for_line(LineCount::from(line + 1), ctx);
                    line_text.trim_end_matches('\n') == original_text
                },
                ctx,
            );
            assert_eq!(result, Some(3)); // Should find "Target" at position 3
        });
    })
}

#[test]
fn test_update_comment_location_current_moved_line() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let base_text = "First line\nSecond line\nThird line";
        let current_text = "First line\nThird line\nSecond line"; // Lines 2 and 3 swapped
        let editor = mock_model_with_diff(&mut app, base_text, current_text, ContentVersion::new());
        layout_model(&mut app, &editor).await;

        let original_text = "Second line";
        let original_location = EditorLineLocation::Current {
            line_number: LineCount::from(1),
            line_range: LineCount::from(1)..LineCount::from(2),
        };

        let (line, content, _) = editor.update(&mut app, |editor, ctx| {
            editor.get_new_line_location(&original_location, original_text.to_string(), ctx)
        });

        // Comment should be moved to line 3 where "Second line" now appears
        match line {
            EditorLineLocation::Current { line_number, .. } => {
                assert_eq!(line_number, LineCount::from(2));
            }
            _ => panic!("Expected Current location"),
        }

        // Now delete the second line (`Third line`)
        editor.update(&mut app, |editor, ctx| {
            editor.apply_diffs(
                vec![DiffDelta {
                    replacement_line_range: 2..3,
                    insertion: "".to_string(),
                }],
                ctx,
            );
        });

        let (line, content, _) = editor.update(&mut app, |editor, ctx| {
            editor.get_new_line_location(&line, content.original_text(), ctx)
        });

        // Comment should be moved to line 2
        match line {
            EditorLineLocation::Current { line_number, .. } => {
                assert_eq!(line_number, LineCount::from(1));
            }
            _ => panic!("Expected Current location"),
        }

        // Adding new lines should push the comment down
        editor.update(&mut app, |editor, ctx| {
            editor.apply_diffs(
                vec![DiffDelta {
                    replacement_line_range: 1..1,
                    insertion: "New line".to_string(),
                }],
                ctx,
            );
        });

        let (line, _, _) = editor.update(&mut app, |editor, ctx| {
            editor.get_new_line_location(&line, content.original_text(), ctx)
        });

        editor.read(&app, |editor, ctx| {
            assert_eq!(
                editor.content.as_ref(ctx).text().as_str(),
                "New line\nFirst line\nSecond line"
            );
        });

        // Comment should be moved to line 2
        match line {
            EditorLineLocation::Current { line_number, .. } => {
                assert_eq!(line_number, LineCount::from(2));
            }
            _ => panic!("Expected Current location"),
        }
    })
}

#[test]
fn test_update_comment_location_removed_line() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);
        let base_text = "First line\nThird line\nSecond line\nFourth line\n";
        let current_text = "First line\nThird line\nFourth line"; // Lines 2 is deleted
        let editor = mock_model_with_diff(&mut app, base_text, current_text, ContentVersion::new());
        layout_model(&mut app, &editor).await;

        let (diff_tx, diff_rx) = oneshot::channel();
        app.update(|ctx| {
            let mut diff_tx = Some(diff_tx);
            ctx.subscribe_to_model(&editor, move |_, event, _| {
                if let CodeEditorModelEvent::DiffUpdated = event {
                    diff_tx
                        .take()
                        .expect("Should only receive one event")
                        .send(())
                        .expect("Receiver should exist");
                }
            });

            editor.update(ctx, |editor, ctx| {
                let new = editor.content.as_ref(ctx).text();
                editor.diff().update(ctx, |diff, ctx| {
                    diff.compute_diff(new, false, BufferVersion::new(), ctx);
                });
            });
        });

        // Verify that we received a diff and it has the expected content.
        diff_rx.await.expect("Event should be sent");

        let line = EditorLineLocation::Removed {
            line_number: LineCount::from(0),
            line_range: LineCount::from(0)..LineCount::from(1),
            index: 0,
        };
        let content = "Second line";

        let (line, _, _) = editor.update(&mut app, |editor, ctx| {
            editor.get_new_line_location(&line, content.to_string(), ctx)
        });

        // Comment should be moved to line 2
        match line {
            EditorLineLocation::Removed { line_number, .. } => {
                assert_eq!(line_number, LineCount::from(2));
            }
            _ => panic!("Expected Removed location"),
        }
    })
}

/// Helper: compute diff, wait for DiffUpdated, expand diffs, and re-layout.
async fn compute_diff_and_expand(app: &mut App, editor: &ModelHandle<CodeEditorModel>) {
    let (diff_tx, diff_rx) = oneshot::channel();
    app.update(|ctx| {
        let mut diff_tx = Some(diff_tx);
        ctx.subscribe_to_model(editor, move |_, event, _| {
            if let CodeEditorModelEvent::DiffUpdated = event {
                if let Some(tx) = diff_tx.take() {
                    let _ = tx.send(());
                }
            }
        });

        editor.update(ctx, |editor, ctx| {
            let new = editor.content.as_ref(ctx).text();
            editor.diff().update(ctx, |diff, ctx| {
                diff.compute_diff(new, false, BufferVersion::new(), ctx);
            });
        });
    });
    diff_rx.await.expect("DiffUpdated event should be sent");

    // Expand diffs so temporary blocks are inserted for removed lines.
    editor.update(app, |editor, ctx| {
        editor.expand_diffs(ctx);
    });
    layout_model(app, editor).await;
}

#[test]
fn test_line_at_vertical_offset_current_line_insert_above() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        // Base has 2 lines; current adds a line between them.
        let base_text = "AAA\nBBB";
        let current_text = "AAA\nNEW_ADDED\nBBB";
        let editor = mock_model_with_diff(&mut app, base_text, current_text, ContentVersion::new());
        layout_model(&mut app, &editor).await;
        compute_diff_and_expand(&mut app, &editor).await;

        // Get the offset of line 1 (the added line) via line_at_vertical_offset.
        // Use the line height to target line 1.
        let line_height = app.read(|ctx| editor.as_ref(ctx).line_height(ctx));
        let target_offset = Pixels::new(line_height);

        let (stable_line, intra_line_offset) = editor
            .update(&mut app, |model, ctx| {
                model.line_at_vertical_offset(target_offset, ctx)
            })
            .expect("line_at_vertical_offset should return Some for a valid offset");
        assert_eq!(intra_line_offset, Pixels::zero());

        // Verify round-trip: line_top should return the same offset.
        let offset_before = app
            .read(|ctx| editor.as_ref(ctx).line_top(&stable_line, ctx))
            .expect("line_top should return Some for a valid current line");
        assert_eq!(offset_before, target_offset);

        // Insert a new line above line 0.
        editor.update(&mut app, |editor, ctx| {
            editor.apply_diffs(
                vec![DiffDelta {
                    replacement_line_range: 0..0,
                    insertion: "INSERTED".to_string(),
                }],
                ctx,
            );
        });
        layout_model(&mut app, &editor).await;

        // The anchor inside stable_line tracks through the edit, so line_top
        // should now return a larger offset without needing to re-identify
        // the line.
        let offset_after = app
            .read(|ctx| editor.as_ref(ctx).line_top(&stable_line, ctx))
            .expect("line_top should return Some after insert");

        assert!(
            offset_after > offset_before,
            "Expected offset to increase after inserting a line above (before={offset_before:?}, after={offset_after:?})"
        );
    })
}

#[test]
fn test_line_at_vertical_offset_removed_line_insert_above() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        // Base has 3 lines; current removes the middle one.
        let base_text = "AAA\nREMOVED\nBBB";
        let current_text = "AAA\nBBB";
        let editor = mock_model_with_diff(&mut app, base_text, current_text, ContentVersion::new());
        layout_model(&mut app, &editor).await;
        compute_diff_and_expand(&mut app, &editor).await;

        // The removed line sits as a temporary block between line 0 ("AAA")
        // and line 1 ("BBB"). Its offset is after one line height.
        let line_height = app.read(|ctx| editor.as_ref(ctx).line_height(ctx));
        let target_offset = Pixels::new(line_height);

        let (stable_line, intra_line_offset) = editor
            .update(&mut app, |model, ctx| {
                model.line_at_vertical_offset(target_offset, ctx)
            })
            .expect("line_at_vertical_offset should return Some for removed line offset");
        assert_eq!(intra_line_offset, Pixels::zero());

        // Verify round-trip.
        let offset_before = app
            .read(|ctx| editor.as_ref(ctx).line_top(&stable_line, ctx))
            .expect("line_top should return Some for a valid removed line");
        assert_eq!(offset_before, target_offset);

        // Insert a line at the top.
        editor.update(&mut app, |editor, ctx| {
            editor.apply_diffs(
                vec![DiffDelta {
                    replacement_line_range: 0..0,
                    insertion: "INSERTED".to_string(),
                }],
                ctx,
            );
        });
        // Re-compute diff and re-expand so temporary blocks are placed correctly.
        compute_diff_and_expand(&mut app, &editor).await;

        // The anchor tracks through the edit, so line_top should return a
        // larger offset.
        let offset_after = app
            .read(|ctx| editor.as_ref(ctx).line_top(&stable_line, ctx))
            .expect("line_top should return Some after insert for removed line");

        assert!(
            offset_after > offset_before,
            "Expected removed-line offset to increase after inserting a line above (before={offset_before:?}, after={offset_after:?})"
        );
    })
}

#[test]
fn test_line_at_vertical_offset_returns_none_for_invalid() {
    App::test((), |mut app| async move {
        initialize_deps(&mut app);

        let base_text = "AAA\nBBB";
        let current_text = "AAA\nBBB";
        let editor = mock_model_with_diff(&mut app, base_text, current_text, ContentVersion::new());
        layout_model(&mut app, &editor).await;
        compute_diff_and_expand(&mut app, &editor).await;

        // Offset far beyond content height should return None.
        let beyond = editor.update(&mut app, |model, ctx| {
            let content_height = model.render_state().as_ref(ctx).height();
            model.line_at_vertical_offset(content_height + Pixels::new(1000.0), ctx)
        });
        assert!(
            beyond.is_none(),
            "Expected None for offset beyond content height"
        );
    })
}
