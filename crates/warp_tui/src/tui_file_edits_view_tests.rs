use std::path::PathBuf;

use ai::agent::action::FileEdit;
use ai::agent::action_result::{
    AnyFileContent, FileContext, RequestFileEditsResult, UpdatedFileContext,
};
use ai::diff_validation::{DiffDelta, DiffType, ParsedDiff};
use futures::channel::oneshot;
use warp::appearance::Appearance;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::FileDiff;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::model::CoreEditorModel;
use warpui::App;

use super::{
    SectionKey, SectionStates, ToolCallDisplayState, deltas_for, file_edit_header_label,
    file_edits_fallback_label, restored_file_diffs, should_rehydrate_restored_diffs, verb_and_name,
};

fn delta(range: std::ops::Range<usize>, insertion: &str) -> DiffDelta {
    DiffDelta {
        replacement_line_range: range,
        insertion: insertion.to_owned(),
    }
}

#[test]
fn restored_file_edits_rehydrate_non_zero_diff_stats() {
    let file_edits = vec![FileEdit::Edit(ParsedDiff::StrReplaceEdit {
        file: Some("src/lib.rs".to_owned()),
        search: Some("1|old\\n".to_owned()),
        replace: Some("1|new\\n".to_owned()),
    })];

    let diffs = restored_file_diffs(file_edits);

    assert_eq!(diffs.len(), 1);
    assert_eq!(diffs[0].line_stats(), (1, 1));
}

fn updated_file_context(file_name: &str) -> UpdatedFileContext {
    UpdatedFileContext {
        was_edited_by_user: false,
        file_context: FileContext::new(
            file_name.to_string(),
            AnyFileContent::StringContent("new\n".to_string()),
            None,
            None,
        ),
    }
}

/// A restored successful `RequestFileEdits` action rehydrates its
/// originally-requested diffs and renders the per-file summary fallback label
/// from the recorded result.
#[test]
fn restored_successful_file_edits_rehydrate_and_summarize() {
    let success = RequestFileEditsResult::Success {
        diff: String::new(),
        updated_files: vec![updated_file_context("src/lib.rs")],
        deleted_files: Vec::new(),
        lines_added: 3,
        lines_removed: 1,
    };

    assert!(should_rehydrate_restored_diffs(Some(&success)));
    assert_eq!(
        file_edits_fallback_label(Some(&success)),
        "Edited 1 file (+3 −1)"
    );
}

/// Regression: a restored *cancelled* `RequestFileEdits` action must NOT
/// hydrate its originally-requested diffs. Previously the hydration guard was
/// `get_action_result().is_some()`, which is true for `Cancelled`, so restored
/// cancelled edits rendered the requested diffs instead of the terminal
/// fallback label. The guard now restricts hydration to `Success`, so the
/// cancelled action keeps its "File edits cancelled" label (the GUI mirrors
/// this by marking non-success results `CodeDiffState::Rejected`).
#[test]
fn restored_cancelled_file_edits_do_not_rehydrate_and_keep_fallback_label() {
    let cancelled = RequestFileEditsResult::Cancelled;

    assert!(!should_rehydrate_restored_diffs(Some(&cancelled)));
    assert_eq!(
        file_edits_fallback_label(Some(&cancelled)),
        "File edits cancelled"
    );
}

/// Regression: a restored *failed* (`DiffApplicationFailed`) action likewise
/// must NOT hydrate diffs and keeps its "File edits failed" fallback label.
#[test]
fn restored_failed_file_edits_do_not_rehydrate_and_keep_fallback_label() {
    let failed = RequestFileEditsResult::DiffApplicationFailed {
        error: "boom".to_string(),
    };

    assert!(!should_rehydrate_restored_diffs(Some(&failed)));
    assert_eq!(
        file_edits_fallback_label(Some(&failed)),
        "File edits failed"
    );
}

/// A live action (no recorded result yet) is executor-backed, not
/// pre-hydrated, so it renders the pending label until the executor seeds the
/// storage.
#[test]
fn live_file_edits_do_not_prehydrate_and_show_pending_label() {
    assert!(!should_rehydrate_restored_diffs(None));
    assert_eq!(file_edits_fallback_label(None), "Preparing edits…");
}

#[test]
fn all_file_edit_sections_start_collapsed_and_toggle_independently() {
    let states = SectionStates::default();

    assert!(states.is_collapsed(SectionKey::Summary));
    assert!(states.is_collapsed(SectionKey::File(0)));
    assert!(states.is_collapsed(SectionKey::File(1)));

    states.toggle_collapsed(SectionKey::File(0));
    assert!(states.is_collapsed(SectionKey::Summary));
    assert!(!states.is_collapsed(SectionKey::File(0)));
    assert!(states.is_collapsed(SectionKey::File(1)));
}
#[test]
fn blocked_file_edit_headers_use_in_progress_wording() {
    assert_eq!(
        file_edit_header_label(ToolCallDisplayState::Blocked, "Edited", "2 files"),
        "Editing 2 files"
    );
    assert_eq!(
        file_edit_header_label(ToolCallDisplayState::Blocked, "Updated", "lib.rs"),
        "Editing lib.rs"
    );

    assert_eq!(
        file_edit_header_label(ToolCallDisplayState::Succeeded, "Edited", "2 files"),
        "Edited 2 files"
    );
    assert_eq!(
        file_edit_header_label(ToolCallDisplayState::Succeeded, "Updated", "lib.rs"),
        "Updated lib.rs"
    );
}

fn update_diff(path: &str, rename: Option<&str>) -> FileDiff {
    FileDiff::new(
        "old\n".to_owned(),
        path.to_owned(),
        DiffType::Update {
            deltas: vec![delta(1..2, "new\n")],
            rename: rename.map(PathBuf::from),
        },
    )
}

#[test]
fn verbs_follow_the_diff_op() {
    let create = FileDiff::new(
        String::new(),
        "/tmp/a/new.rs".to_owned(),
        DiffType::creation("fn main() {}\n".to_owned()),
    );
    assert_eq!(verb_and_name(&create), ("Created", "new.rs".to_owned()));

    assert_eq!(
        verb_and_name(&update_diff("/tmp/a/lib.rs", None)),
        ("Updated", "lib.rs".to_owned())
    );

    let delete = FileDiff::new(
        "gone\n".to_owned(),
        "/tmp/a/old.rs".to_owned(),
        DiffType::Delete {
            delta: delta(1..2, ""),
        },
    );
    assert_eq!(verb_and_name(&delete), ("Deleted", "old.rs".to_owned()));
}

#[test]
fn renames_display_old_and_new_names() {
    assert_eq!(
        verb_and_name(&update_diff("/tmp/a/old.rs", Some("/tmp/a/new.rs"))),
        ("Updated", "old.rs → new.rs".to_owned())
    );
    // A rename to the same file name (e.g. a directory move) shows one name.
    assert_eq!(
        verb_and_name(&update_diff("/tmp/a/lib.rs", Some("/tmp/b/lib.rs"))),
        ("Updated", "lib.rs".to_owned())
    );
}

/// Drives the full body pipeline headlessly: seed a char-cell editor with base
/// content, apply deltas (buffer becomes post-edit and the diff recomputes),
/// expand the hunks, and assert the added-line ranges and the removed-line
/// ghost blocks that the diff body renders from.
#[test]
fn diff_pipeline_computes_added_lines_and_ghost_blocks() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let editor = app.add_model(|ctx| CodeEditorModel::new_tui(80, ctx));

        let (tx, rx) = oneshot::channel();
        app.update(|ctx| {
            let mut tx = Some(tx);
            ctx.subscribe_to_model(&editor, move |_, event, _| {
                if matches!(event, CodeEditorModelEvent::DiffUpdated)
                    && let Some(tx) = tx.take()
                {
                    let _ = tx.send(());
                }
            });
            editor.update(ctx, |editor, ctx| {
                editor.reset_content(InitialBufferState::plain_text("a\nold\nc\n"), ctx);
                // Replace line 2 ("old") with "new"; delta line ranges are
                // 1-indexed like the executor's resolved deltas.
                editor.apply_diffs(
                    vec![DiffDelta {
                        replacement_line_range: 2..3,
                        insertion: "new\n".to_owned(),
                    }],
                    ctx,
                );
            });
        });
        rx.await.expect("diff computation should complete");

        editor.update(&mut app, |editor, ctx| editor.expand_diffs(ctx));

        // Ghost blocks land via the render state's async layout channel, which
        // is drained on a background thread before the foreground handler stores
        // them. Await the render state's layout-complete signal (outstanding
        // layout actions draining to zero) rather than busy-polling a fixed
        // number of no-op yields, which races that background thread and flakes
        // under load.
        app.read(|app| {
            editor
                .as_ref(app)
                .render_state()
                .as_ref(app)
                .layout_complete()
        })
        .await;

        let ghosts = app.read(|app| {
            editor
                .as_ref(app)
                .render_state()
                .as_ref(app)
                .char_cell()
                .expect("TUI editor renders in char-cell mode")
                .display_lattice(&[])
                .ghosts()
                .to_vec()
        });

        assert_eq!(ghosts.len(), 1);
        assert_eq!(ghosts[0].content, "old\n");
        // The ghost interleaves before the replacement line (0-based line 1).
        assert_eq!(ghosts[0].insert_before.as_u32(), 1);

        app.read(|app| {
            let editor = editor.as_ref(app);
            let diff = editor.diff().as_ref(app);
            let added: Vec<_> = diff.added_or_changed_lines().collect();
            assert_eq!(added, vec![1..2]);
            // Header counts read from this same computed diff, so they always
            // agree with the rendered body (one line replaced by one line).
            assert_eq!(diff.diff_status().get_diff_lines(), (1, 1));
        });
    });
}

#[test]
fn deltas_cover_every_diff_op() {
    let d = delta(1..2, "x\n");
    assert_eq!(
        deltas_for(&DiffType::Create { delta: d.clone() }),
        vec![d.clone()]
    );
    assert_eq!(
        deltas_for(&DiffType::Delete { delta: d.clone() }),
        vec![d.clone()]
    );
    assert_eq!(
        deltas_for(&DiffType::Update {
            deltas: vec![d.clone(), delta(4..5, "y\n")],
            rename: None,
        }),
        vec![d, delta(4..5, "y\n")]
    );
}
