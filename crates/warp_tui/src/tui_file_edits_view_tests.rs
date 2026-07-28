use std::path::PathBuf;

use ai::diff_validation::{DiffDelta, DiffType};
use futures::channel::oneshot;
use warp::appearance::Appearance;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{
    AIAgentAction, AIAgentActionId, AIAgentActionType, AIConversationId, FileDiff, TaskId,
    queue_tui_permission_action,
};
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::model::CoreEditorModel;
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, App, WindowInvalidation};
use warpui_core::elements::tui::{TuiBufferExt, TuiRect};
use warpui_core::keymap::Keystroke;
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{TuiView, ViewHandle};

use super::{
    FILE_EDITS_PERMISSION_ACTIVE, SectionKey, SectionStates, ToolCallDisplayState,
    TuiFileEditsView, deltas_for, file_edit_header_label, verb_and_name,
};
use crate::test_fixtures::{TestHostView, add_test_action_model};

fn delta(range: std::ops::Range<usize>, insertion: &str) -> DiffDelta {
    DiffDelta {
        replacement_line_range: range,
        insertion: insertion.to_owned(),
    }
}

/// Section state uses `default_collapsed` when no explicit toggle exists.
/// Non-blocked rendering (default_collapsed=true) → collapsed; blocked
/// rendering (default_collapsed=false) → expanded; toggling stores an
/// explicit entry that overrides the default.
#[test]
fn section_states_respect_default_collapsed_and_toggle_independently() {
    let states = SectionStates::default();

    // Non-blocked: sections collapse by default.
    assert!(states.is_collapsed(SectionKey::Summary, true));
    assert!(states.is_collapsed(SectionKey::File(0), true));
    assert!(states.is_collapsed(SectionKey::File(1), true));

    // Blocked: sections expand by default (no explicit entry yet).
    assert!(!states.is_collapsed(SectionKey::Summary, false));
    assert!(!states.is_collapsed(SectionKey::File(0), false));
    assert!(!states.is_collapsed(SectionKey::File(1), false));

    // Toggle File(0) while blocked (default_collapsed=false → was expanded →
    // explicit entry records collapsed=true).
    states.toggle_collapsed(SectionKey::File(0), false);
    assert!(!states.is_collapsed(SectionKey::Summary, false)); // unchanged
    assert!(states.is_collapsed(SectionKey::File(0), false)); // now explicitly collapsed
    assert!(!states.is_collapsed(SectionKey::File(1), false)); // still default expanded
}

/// reset_states clears explicit entries so sections revert to their
/// context-dependent default on the next render.
#[test]
fn reset_states_clears_explicit_toggles() {
    let states = SectionStates::default();
    // Record an explicit collapsed entry.
    states.toggle_collapsed(SectionKey::File(0), true);
    assert!(!states.is_collapsed(SectionKey::File(0), true)); // was collapsed, now expanded
    // After reset, reverts to default.
    states.reset_states();
    assert!(states.is_collapsed(SectionKey::File(0), true));
    assert!(!states.is_collapsed(SectionKey::File(0), false));
}

/// toggle_expand_all collapses all when any are expanded, and expands all
/// when all are collapsed.
#[test]
fn toggle_expand_all_collapses_then_expands() {
    let states = SectionStates::default();
    let keys = [
        SectionKey::Summary,
        SectionKey::File(0),
        SectionKey::File(1),
    ];

    // Default blocked (default_collapsed=false): all expanded.
    // First toggle → collapse all.
    states.toggle_expand_all(&keys, false);
    for &key in &keys {
        assert!(
            states.is_collapsed(key, false),
            "{key:?} should be collapsed after first toggle"
        );
    }

    // Second toggle → expand all.
    states.toggle_expand_all(&keys, false);
    for &key in &keys {
        assert!(
            !states.is_collapsed(key, false),
            "{key:?} should be expanded after second toggle"
        );
    }

    // Mixed state (one collapsed) → collapse all.
    states.toggle_collapsed(SectionKey::File(0), false); // File(0) → collapsed
    states.toggle_expand_all(&keys, false);
    for &key in &keys {
        assert!(
            states.is_collapsed(key, false),
            "{key:?} should be collapsed after mixed toggle"
        );
    }
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

/// Renders the blocked file-edits permission card and returns the presenter
/// frame so callers can inspect rendered lines and cell colors.
fn present_file_edits_view(
    app: &mut App,
    view: &ViewHandle<TuiFileEditsView>,
) -> warpui_core::presenter::tui::TuiFrame {
    let mut presenter = TuiPresenter::new();
    app.update(|ctx| {
        let view_ref = view.as_ref(ctx);
        let prompt = &view_ref.permission_prompt;
        let mut invalidation = WindowInvalidation::default();
        invalidation.updated.insert(view.id());
        invalidation.updated.insert(prompt.id());
        invalidation
            .updated
            .extend(prompt.as_ref(ctx).child_view_ids(ctx));
        presenter.invalidate(&invalidation, ctx, view.window_id(ctx));
        presenter.present(ctx, view, TuiRect::new(0, 0, 80, 20))
    })
}

/// Dispatches a keystroke through the responder chain starting from the
/// currently focused view in the file-edits view's window.
fn dispatch_focused_key(app: &mut App, view: &ViewHandle<TuiFileEditsView>, key: &str) -> bool {
    present_file_edits_view(app, view);
    let (window_id, responder_chain) = app.read(|ctx| {
        let window_id = view.window_id(ctx);
        let focused = ctx
            .focused_view_id(window_id)
            .expect("file-edits permission card has a focused view");
        (window_id, ctx.view_ancestors(window_id, focused))
    });
    app.dispatch_keystroke(
        window_id,
        &responder_chain,
        &Keystroke::parse(key).expect("valid keystroke"),
        false,
    )
    .expect("keystroke dispatch succeeds")
}

/// Creates a `TuiFileEditsView` for the given `action_id` inside a test
/// TUI window and returns its handle.
fn add_file_edits_view(app: &mut App, action_id: AIAgentActionId) -> ViewHandle<TuiFileEditsView> {
    let action_model = add_test_action_model(app);
    app.update(|ctx| {
        let (window_id, _) = ctx.add_tui_window(
            AddWindowOptions {
                window_style: WindowStyle::NotStealFocus,
                ..Default::default()
            },
            |_| TestHostView,
        );
        let action_id = action_id.clone();
        ctx.add_typed_action_tui_view(window_id, move |ctx| {
            TuiFileEditsView::new(
                action_id,
                AIConversationId::new(),
                Vec::new(),
                &action_model,
                ctx,
            )
        })
    })
}

/// Builds a `RequestFileEdits` agent action for the given action id.
fn file_edits_action(id: &str) -> AIAgentAction {
    AIAgentAction {
        id: AIAgentActionId::from(id.to_owned()),
        task_id: TaskId::new("task-1".to_owned()),
        action: AIAgentActionType::RequestFileEdits {
            file_edits: Vec::new(),
            title: None,
        },
        requires_result: true,
    }
}

/// The blocked file-edits card renders the permission header, the
/// `e to expand/collapse` affordance, and the yes/no/Other options.
/// This mirrors `blocked_command_card_matches_permission_layout` for
/// `TuiShellCommandView` and covers AC 3 (header affordance).
#[test]
fn blocked_file_edits_card_shows_expand_hint_and_options() {
    App::test((), |mut app| async move {
        app.update(super::init);
        let action_id = AIAgentActionId::from("file-edits-1".to_owned());
        let view = add_file_edits_view(&mut app, action_id.clone());
        let (action_model, conversation_id) = app.read(|ctx| {
            let view = view.as_ref(ctx);
            (view.action_model.clone(), view.conversation_id)
        });
        action_model.update(&mut app, |model, ctx| {
            queue_tui_permission_action(
                model,
                file_edits_action("file-edits-1"),
                conversation_id,
                ctx,
            );
        });

        let frame = present_file_edits_view(&mut app, &view);
        let lines = frame.buffer.to_lines();

        let has_header = lines
            .iter()
            .any(|l| l.contains("Is it OK if I make these file edits?"));
        assert!(has_header, "blocked card header missing in {lines:?}");

        // AC 3: the header row carries the `e to expand/collapse` affordance.
        let has_expand_hint = lines.iter().any(|l| l.contains("e to expand/collapse"));
        assert!(
            has_expand_hint,
            "e-to-expand-collapse hint missing in {lines:?}"
        );

        let has_yes_option = lines.iter().any(|l| l.contains("yes"));
        assert!(has_yes_option, "yes option missing in {lines:?}");
    });
}

/// Pressing `e` while the file-edits permission card's option list is focused
/// dispatches `ToggleExpandAll` through the full responder chain, including
/// `TuiFileEditsView`. This covers AC 4 (keymap wiring end-to-end).
#[test]
fn e_key_dispatches_toggle_expand_all_on_blocked_card() {
    App::test((), |mut app| async move {
        app.update(super::init);
        app.update(crate::tui_permission_prompt::init);
        app.update(crate::option_selector::init);
        let action_id = AIAgentActionId::from("file-edits-2".to_owned());
        let view = add_file_edits_view(&mut app, action_id.clone());
        let (action_model, conversation_id) = app.read(|ctx| {
            let view = view.as_ref(ctx);
            (view.action_model.clone(), view.conversation_id)
        });
        action_model.update(&mut app, |model, ctx| {
            queue_tui_permission_action(
                model,
                file_edits_action("file-edits-2"),
                conversation_id,
                ctx,
            );
        });

        // `e` is only gated when FILE_EDITS_PERMISSION_ACTIVE is set, which
        // requires the action to be blocked AND the list to be focused.
        app.read(|ctx| {
            let keymap_ctx = view.as_ref(ctx).keymap_context(ctx);
            assert!(
                keymap_ctx.set.contains(FILE_EDITS_PERMISSION_ACTIVE),
                "FILE_EDITS_PERMISSION_ACTIVE not set — list not focused or action not blocked"
            );
        });

        // Dispatch `e` through the focused view's responder chain. Because
        // `TuiFileEditsView` is an ancestor and its context predicate is
        // satisfied, the keystroke must be consumed and emit LayoutChanged.
        assert!(
            dispatch_focused_key(&mut app, &view, "e"),
            "`e` should be consumed as ToggleExpandAll"
        );
    });
}
