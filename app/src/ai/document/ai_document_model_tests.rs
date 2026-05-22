use std::ops::Range;

use ai::diff_validation::DiffDelta;
use chrono::Local;
use warpui::App;

use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::appearance::Appearance;
use crate::cloud_object::model::persistence::CloudModel;
use crate::test_util::settings::initialize_settings_for_tests;

fn initialize_app_for_ai_document_tests(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| CloudModel::new(None, Vec::new(), None));
}

#[test]
fn test_create_document() {
    App::test((), |mut app| async move {
        initialize_app_for_ai_document_tests(&mut app);
        let model_handle = app.add_model(|_ctx| AIDocumentModel::new_for_test());

        let doc_id = model_handle.update(&mut app, |model, ctx| {
            model.create_document(
                "Test Document",
                "# Hello World\n\nThis is a test.",
                AIConversationId::new(),
                None,
                ctx,
            )
        });

        model_handle.update(&mut app, |model, ctx| {
            let doc = model
                .get_current_document(&doc_id)
                .expect("Document should exist");
            assert_eq!(doc.title, "Test Document");
            assert_eq!(doc.version, AIDocumentVersion::default());

            let content = model
                .get_document_content(&doc_id, ctx)
                .expect("Should have content");
            assert_eq!(content, "# Hello World\n\nThis is a test.");

            // Should have no versions initially
            assert!(model.get_earlier_document_versions(&doc_id).is_none());
        });
    });
}

#[test]
fn test_apply_diffs_creates_version() {
    App::test((), |mut app| async move {
        initialize_app_for_ai_document_tests(&mut app);
        let model_handle = app.add_model(|_ctx| AIDocumentModel::new_for_test());

        let doc_id = model_handle.update(&mut app, |model, ctx| {
            model.create_document(
                "Test Document",
                "# Hello World\n\nThis is a test.",
                AIConversationId::new(),
                None,
                ctx,
            )
        });

        // Apply some diffs
        let diffs = vec![DiffDelta {
            replacement_line_range: Range { start: 0, end: 1 },
            insertion: "# Hello Universe".to_string(),
        }];

        model_handle.update(&mut app, |model, ctx| {
            model.create_new_version_and_apply_diffs(
                &doc_id,
                diffs,
                AIDocumentUpdateSource::Agent,
                ctx,
            );
        });

        model_handle.update(&mut app, |model, ctx| {
            let doc = model
                .get_current_document(&doc_id)
                .expect("Document should exist");
            // Version should have incremented
            assert_eq!(doc.version, AIDocumentVersion::new_for_test(2));

            // Check the latest content after applying diffs
            let current_content = model
                .get_document_content(&doc_id, ctx)
                .expect("Should have current content");
            assert_eq!(
                current_content,
                "# Hello Universe\n# Hello World\n\nThis is a test."
            );

            // Should have one version saved
            let versions = model
                .get_earlier_document_versions(&doc_id)
                .expect("Should have versions");
            assert_eq!(versions.len(), 1);

            let first_version = &versions[0];
            assert_eq!(first_version.version, AIDocumentVersion::new_for_test(1));
            assert_eq!(first_version.title, "Test Document");
            assert_eq!(
                first_version.get_content(ctx),
                "# Hello World\n\nThis is a test."
            );
        });
    });
}

#[test]
fn test_multiple_diffs_create_multiple_versions() {
    App::test((), |mut app| async move {
        initialize_app_for_ai_document_tests(&mut app);
        let model_handle = app.add_model(|_ctx| AIDocumentModel::new_for_test());

        let doc_id = model_handle.update(&mut app, |model, ctx| {
            model.create_document(
                "Test Document",
                "# Hello World",
                AIConversationId::new(),
                None,
                ctx,
            )
        });

        // Apply first diff
        let diffs1 = vec![DiffDelta {
            replacement_line_range: Range { start: 0, end: 1 },
            insertion: "# Hello Universe".to_string(),
        }];

        model_handle.update(&mut app, |model, ctx| {
            model.create_new_version_and_apply_diffs(
                &doc_id,
                diffs1,
                AIDocumentUpdateSource::Agent,
                ctx,
            );
        });

        // Apply second diff
        let diffs2 = vec![DiffDelta {
            replacement_line_range: Range { start: 0, end: 1 },
            insertion: "# Hello Galaxy".to_string(),
        }];

        model_handle.update(&mut app, |model, ctx| {
            model.create_new_version_and_apply_diffs(
                &doc_id,
                diffs2,
                AIDocumentUpdateSource::User,
                ctx,
            );
        });

        model_handle.update(&mut app, |model, ctx| {
            let doc = model
                .get_current_document(&doc_id)
                .expect("Document should exist");
            // Version should be v3 now
            assert_eq!(doc.version, AIDocumentVersion::new_for_test(3));

            // Check the latest content after applying both diffs
            let current_content = model
                .get_document_content(&doc_id, ctx)
                .expect("Should have current content");
            assert_eq!(
                current_content,
                "# Hello Galaxy\n# Hello Universe\n# Hello World\n"
            );

            // Should have two versions saved
            let versions = model
                .get_earlier_document_versions(&doc_id)
                .expect("Should have versions");
            assert_eq!(versions.len(), 2);

            // First version (original)
            let first_version = &versions[0];
            assert_eq!(first_version.version, AIDocumentVersion::new_for_test(1));
            assert_eq!(first_version.get_content(ctx), "# Hello World\n");

            // Second version (after first diff)
            let second_version = &versions[1];
            assert_eq!(second_version.version, AIDocumentVersion::new_for_test(2));
            assert_eq!(
                second_version.get_content(ctx),
                "# Hello Universe\n# Hello World\n"
            );
        });
    });
}

#[test]
fn test_restore_document_version() {
    App::test((), |mut app| async move {
        initialize_app_for_ai_document_tests(&mut app);
        let model_handle = app.add_model(|_ctx| AIDocumentModel::new_for_test());

        let doc_id = model_handle.update(&mut app, |model, ctx| {
            model.create_document(
                "Original Title",
                "# Original Content",
                AIConversationId::new(),
                None,
                ctx,
            )
        });

        // Apply a diff to create version 1
        let diffs = vec![DiffDelta {
            replacement_line_range: Range { start: 0, end: 1 },
            insertion: "# Modified Content".to_string(),
        }];

        model_handle.update(&mut app, |model, ctx| {
            model.create_new_version_and_apply_diffs(
                &doc_id,
                diffs,
                AIDocumentUpdateSource::Agent,
                ctx,
            );
        });

        // Update the title too
        model_handle.update(&mut app, |model, ctx| {
            model.update_title(&doc_id, "Modified Title", AIDocumentUpdateSource::User, ctx);
        });

        // Restore to the original version (v1)
        model_handle.update(&mut app, |model, ctx| {
            let result =
                model.revert_to_document_version(&doc_id, AIDocumentVersion::new_for_test(1), ctx);
            assert!(result.is_ok(), "Restore should succeed");
        });

        model_handle.update(&mut app, |model, ctx| {
            let doc = model
                .get_current_document(&doc_id)
                .expect("Document should exist");

            // Version should have incremented to v3
            assert_eq!(doc.version, AIDocumentVersion::new_for_test(3));

            // Title should be restored to original
            assert_eq!(doc.title, "Original Title");

            // Content should be restored to original
            let content = model
                .get_document_content(&doc_id, ctx)
                .expect("Should have content");
            assert_eq!(content, "# Original Content\n");

            // Should have two versions now (original + the modified state before restore)
            let versions = model
                .get_earlier_document_versions(&doc_id)
                .expect("Should have versions");
            assert_eq!(versions.len(), 2);

            // First version (original before first diff)
            let first_version = &versions[0];
            assert_eq!(first_version.version, AIDocumentVersion::new_for_test(1));
            assert_eq!(first_version.get_content(ctx), "# Original Content\n");
            assert_eq!(first_version.title, "Original Title");

            // Second version (modified state before restore)
            let second_version = &versions[1];
            assert_eq!(second_version.version, AIDocumentVersion::new_for_test(2));
            assert_eq!(
                second_version.get_content(ctx),
                "# Modified Content\n# Original Content\n"
            );
            assert_eq!(second_version.title, "Modified Title");
        });
    });
}

#[test]
fn test_restore_nonexistent_version_fails() {
    App::test((), |mut app| async move {
        initialize_app_for_ai_document_tests(&mut app);
        let model_handle = app.add_model(|_ctx| AIDocumentModel::new_for_test());

        let doc_id = model_handle.update(&mut app, |model, ctx| {
            model.create_document(
                "Test Document",
                "# Hello World",
                AIConversationId::new(),
                None,
                ctx,
            )
        });

        model_handle.update(&mut app, |model, ctx| {
            // Try to restore to a version that doesn't exist
            let result =
                model.revert_to_document_version(&doc_id, AIDocumentVersion::new_for_test(5), ctx);
            assert!(
                result.is_err(),
                "Restore should fail for nonexistent version"
            );

            let error_msg = result.unwrap_err().to_string();
            assert!(error_msg.contains("Version v5 not found"));
        });
    });
}

#[test]
fn test_restore_nonexistent_document_fails() {
    App::test((), |mut app| async move {
        initialize_app_for_ai_document_tests(&mut app);
        let model_handle = app.add_model(|_ctx| AIDocumentModel::new_for_test());
        let fake_id = AIDocumentId::new();

        model_handle.update(&mut app, |model, ctx| {
            let result =
                model.revert_to_document_version(&fake_id, AIDocumentVersion::new_for_test(1), ctx);
            assert!(
                result.is_err(),
                "Restore should fail for nonexistent document"
            );

            let error_msg = result.unwrap_err().to_string();
            assert!(error_msg.contains("not found"));
        });
    });
}

#[test]
fn test_create_document_removes_extra_newlines() {
    App::test((), |mut app| async move {
        initialize_app_for_ai_document_tests(&mut app);
        let model_handle = app.add_model(|_ctx| AIDocumentModel::new_for_test());

        // Create a document with extra newlines
        let doc_id = model_handle.update(&mut app, |model, ctx| {
            model.create_document(
                "Test Document",
                "# Hello World\n\n\nThis is a test.\n\n\nEnd.",
                AIConversationId::new(),
                None,
                ctx,
            )
        });

        model_handle.update(&mut app, |model, ctx| {
            let content = model
                .get_document_content(&doc_id, ctx)
                .expect("Should have content");
            assert_eq!(content, "# Hello World\n\n\nThis is a test.\n\n\nEnd.");
        });
    });
}

#[test]
fn test_new_version_editor_isolation() {
    App::test((), |mut app| async move {
        initialize_app_for_ai_document_tests(&mut app);
        let model_handle = app.add_model(|_ctx| AIDocumentModel::new_for_test());

        let original_content = "# Original Content";
        let doc_id = model_handle.update(&mut app, |model, ctx| {
            model.create_document(
                "Test Document",
                original_content,
                AIConversationId::new(),
                None,
                ctx,
            )
        });

        // Apply a diff to create version 1
        let diffs = vec![DiffDelta {
            replacement_line_range: Range { start: 0, end: 1 },
            insertion: "# Modified by agent".to_string(),
        }];

        model_handle.update(&mut app, |model, ctx| {
            model.create_new_version_and_apply_diffs(
                &doc_id,
                diffs,
                AIDocumentUpdateSource::Agent,
                ctx,
            );
        });

        let modified_content = "# Modified by agent\n# Modified by user\n";

        // Directly edit the new version
        model_handle.update(&mut app, |model, ctx| {
            let doc = model
                .get_current_document(&doc_id)
                .expect("Document should exist");
            doc.editor.update(ctx, |editor, editor_ctx| {
                editor.reset_with_markdown(modified_content, editor_ctx);
            });
        });

        model_handle.update(&mut app, |model, ctx| {
            let current_content = model
                .get_document_content(&doc_id, ctx)
                .expect("Should have content");
            assert_eq!(current_content, modified_content);

            // The original version (v1) should still have its original content unchanged.
            // Ensures we've created a new editor instance that can't mutate the previous editor.
            let versions = model
                .get_earlier_document_versions(&doc_id)
                .expect("Should have versions");
            let first_version = versions
                .iter()
                .find(|v| v.version == AIDocumentVersion::new_for_test(1))
                .expect("Should have v1");
            assert_eq!(first_version.get_content(ctx), "# Original Content\n");
        });
    });
}

#[test]
fn test_restored_version_editor_isolation() {
    App::test((), |mut app| async move {
        initialize_app_for_ai_document_tests(&mut app);
        let model_handle = app.add_model(|_ctx| AIDocumentModel::new_for_test());

        let doc_id = model_handle.update(&mut app, |model, ctx| {
            model.create_document(
                "Test Document",
                "# Original Content",
                AIConversationId::new(),
                None,
                ctx,
            )
        });

        // Apply a diff to create version 1
        let diffs = vec![DiffDelta {
            replacement_line_range: Range { start: 0, end: 1 },
            insertion: "# Modified Content".to_string(),
        }];

        model_handle.update(&mut app, |model, ctx| {
            model.create_new_version_and_apply_diffs(
                &doc_id,
                diffs,
                AIDocumentUpdateSource::Agent,
                ctx,
            );
        });

        // Restore to the original version (v1)
        model_handle.update(&mut app, |model, ctx| {
            let result =
                model.revert_to_document_version(&doc_id, AIDocumentVersion::new_for_test(1), ctx);
            assert!(result.is_ok(), "Revert to document version should succeed");
        });

        // Directly edit the current (restored) version's editor
        model_handle.update(&mut app, |model, ctx| {
            let doc = model
                .get_current_document(&doc_id)
                .expect("Document should exist");
            doc.editor.update(ctx, |editor, editor_ctx| {
                editor.reset_with_markdown("# Original Content\n# Added After Restore", editor_ctx);
            });
        });

        model_handle.update(&mut app, |model, ctx| {
            // Current version should be v3 with the new content added
            let doc = model
                .get_current_document(&doc_id)
                .expect("Document should exist");
            assert_eq!(doc.version, AIDocumentVersion::new_for_test(3));

            let current_content = model
                .get_document_content(&doc_id, ctx)
                .expect("Should have content");
            assert_eq!(
                current_content,
                "# Original Content\n# Added After Restore\n"
            );

            // The original version (v1) should still have its original content unchanged.
            // Ensures we've created a new editor instance that can't mutate the previous editor
            let versions = model
                .get_earlier_document_versions(&doc_id)
                .expect("Should have versions");
            let first_version = versions
                .iter()
                .find(|v| v.version == AIDocumentVersion::new_for_test(1))
                .expect("Should have v1");
            assert_eq!(first_version.get_content(ctx), "# Original Content\n");
        });
    });
}

#[test]
fn test_version_string_formatting() {
    let v1 = AIDocumentVersion::new_for_test(1);
    let v42 = AIDocumentVersion::new_for_test(42);
    let v_default = AIDocumentVersion::default();

    assert_eq!(v1.to_string(), "v1");
    assert_eq!(v42.to_string(), "v42");
    assert_eq!(v_default.to_string(), "v1");
    assert_eq!(format!("{}", v1), "v1");

    // Test next() method
    assert_eq!(v1.next(), AIDocumentVersion::new_for_test(2));
    assert_eq!(v42.next(), AIDocumentVersion::new_for_test(43));
}

#[test]
fn test_restored_from_tracking() {
    App::test((), |mut app| async move {
        initialize_app_for_ai_document_tests(&mut app);
        let model_handle = app.add_model(|_ctx| AIDocumentModel::new_for_test());

        let doc_id = model_handle.update(&mut app, |model, ctx| {
            model.create_document(
                "Original Title",
                "# Original Content",
                AIConversationId::new(),
                None,
                ctx,
            )
        });

        // Initial document should have no restored_from
        model_handle.update(&mut app, |model, _ctx| {
            let doc = model
                .get_current_document(&doc_id)
                .expect("Document should exist");
            assert!(doc.restored_from.is_none());
        });

        // Apply a diff to create v2
        let diffs = vec![DiffDelta {
            replacement_line_range: Range { start: 0, end: 1 },
            insertion: "# Modified Content".to_string(),
        }];

        model_handle.update(&mut app, |model, ctx| {
            model.create_new_version_and_apply_diffs(
                &doc_id,
                diffs,
                AIDocumentUpdateSource::Agent,
                ctx,
            );
        });

        // v2 should have no restored_from (agent edit, not a restore)
        model_handle.update(&mut app, |model, _ctx| {
            let doc = model
                .get_current_document(&doc_id)
                .expect("Document should exist");
            assert_eq!(doc.version, AIDocumentVersion::new_for_test(2));
            assert!(doc.restored_from.is_none());
        });

        // Restore to v1
        model_handle.update(&mut app, |model, ctx| {
            let result =
                model.revert_to_document_version(&doc_id, AIDocumentVersion::new_for_test(1), ctx);
            assert!(result.is_ok());
        });

        // v3 should track that it was restored from v1
        model_handle.update(&mut app, |model, _ctx| {
            let doc = model
                .get_current_document(&doc_id)
                .expect("Document should exist");
            assert_eq!(doc.version, AIDocumentVersion::new_for_test(3));
            assert_eq!(doc.restored_from, Some(AIDocumentVersion::new_for_test(1)));

            // Earlier version v2 should have no restored_from since it was created by agent
            let versions = model
                .get_earlier_document_versions(&doc_id)
                .expect("Should have versions");
            let v2 = versions
                .iter()
                .find(|v| v.version == AIDocumentVersion::new_for_test(2))
                .expect("Should have v2");
            assert!(v2.restored_from.is_none());
        });

        // Apply another diff to create v4
        let diffs2 = vec![DiffDelta {
            replacement_line_range: Range { start: 0, end: 1 },
            insertion: "# New Content".to_string(),
        }];

        model_handle.update(&mut app, |model, ctx| {
            model.create_new_version_and_apply_diffs(
                &doc_id,
                diffs2,
                AIDocumentUpdateSource::Agent,
                ctx,
            );
        });

        // v4 should have no restored_from (normal agent edit)
        // v3 in earlier_versions should still have restored_from = v1
        model_handle.update(&mut app, |model, _ctx| {
            let doc = model
                .get_current_document(&doc_id)
                .expect("Document should exist");
            assert_eq!(doc.version, AIDocumentVersion::new_for_test(4));
            assert!(doc.restored_from.is_none());

            let versions = model
                .get_earlier_document_versions(&doc_id)
                .expect("Should have versions");
            let v3 = versions
                .iter()
                .find(|v| v.version == AIDocumentVersion::new_for_test(3))
                .expect("Should have v3");
            assert_eq!(v3.restored_from, Some(AIDocumentVersion::new_for_test(1)));
        });
    });
}

#[test]
fn test_streamed_agent_update_matches_reset_with_markdown_for_code_block() {
    let full_content = "# Sample Markdown Document\nThis document demonstrates markdown formatting with code examples and explanatory text.\n## Python Code Example\nHere's a Python function that calculates the factorial of a number:\n```python path=null start=null\ndef factorial(n):\n    \"\"\"Calculate the factorial of a positive integer.\"\"\"\n    if n < 0:\n        raise ValueError(\"Factorial is not defined for negative numbers\")\n    elif n == 0 or n == 1:\n        return 1\n    else:\n        result = 1\n        for i in range(2, n + 1):\n            result *= i\n        return result\n# Example usage\nprint(f\"5! = {factorial(5)}\")  # Output: 5! = 120\nprint(f\"0! = {factorial(0)}\")  # Output: 0! = 1\n```\n## About This Implementation\nThe factorial function above uses an iterative approach to calculate the factorial of a given number. It includes error handling for negative inputs and handles the special cases where n equals 0 or 1 (both return 1 by mathematical definition).\nThis example demonstrates several Python concepts including function definition, docstrings, conditional statements, exception handling, and loop iteration. The factorial calculation is a classic programming problem that showcases how to build up a result through repeated multiplication.";

    App::test((), |mut app| async move {
        initialize_app_for_ai_document_tests(&mut app);
        let model_handle = app.add_model(|_ctx| AIDocumentModel::new_for_test());

        let conversation_id = AIConversationId::new();

        // Create a streaming document.
        let doc_id = model_handle.update(&mut app, |model, ctx| {
            model.create_document("Streaming Test", "", conversation_id, None, ctx)
        });

        // Split the content into N equal chunks and simulate streaming where
        // each update is the full accumulated markdown so far.
        let n = 15;
        let len = full_content.len();

        for i in 1..=n {
            let upto = len * i / n;
            let chunk = &full_content[..upto];

            // Apply streamed update to the streaming document.
            model_handle.update(&mut app, |model, ctx| {
                model.apply_streamed_agent_update(&doc_id, "new title", chunk, ctx);
            });
            assert_eq!(
                model_handle.read(&app, |model, app_ctx| {
                    model.get_current_document(&doc_id).unwrap().title
                }),
                "new title"
            );

            let reset_doc_id = model_handle.update(&mut app, |model, ctx| {
                model.create_document("Reset Test", chunk, conversation_id, None, ctx)
            });

            // Capture markdown from both documents and ensure they match.
            let (streamed_markdown, reset_markdown) = model_handle.read(&app, |model, app_ctx| {
                let streamed = model
                    .get_document_content(&doc_id, app_ctx)
                    .expect("Should have content");
                let reset = model
                    .get_document_content(&reset_doc_id, app_ctx)
                    .expect("Should have content");
                (streamed, reset)
            });

            assert_eq!(streamed_markdown.trim(), reset_markdown.trim());
        }
    });
}
