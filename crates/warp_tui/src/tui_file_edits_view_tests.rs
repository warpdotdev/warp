use std::path::PathBuf;

use ai::diff_validation::{DiffDelta, DiffType};
use futures::channel::oneshot;
use warp::appearance::Appearance;
use warp::editor::{CodeEditorModel, CodeEditorModelEvent};
use warp::tui_export::{
    AIAgentAction, AIAgentActionId, AIAgentActionType, AIConversationId, DiffSessionType, FileDiff,
    RegisteredDiffStorage, TaskId, queue_tui_permission_action,
};
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::model::CoreEditorModel;
use warpui::platform::WindowStyle;
use warpui::{AddWindowOptions, App, WindowInvalidation};
use warpui_core::elements::tui::{Modifier, TuiBufferExt, TuiRect};
use warpui_core::keymap::Keystroke;
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{TuiView, ViewHandle};

use super::{
    FILE_EDITS_PERMISSION_ACTIVE, SectionKey, SectionStates, ToolCallDisplayState,
    TuiFileEditsView, deltas_for, file_edit_header_label, file_edit_header_spans,
    file_edit_stats_label, verb_and_name,
};
use crate::test_fixtures::{TestHostView, add_test_action_model};
use crate::tui_builder::TuiUiBuilder;
use crate::tui_diff_storage::TuiDiffStorageHandle;

fn delta(range: std::ops::Range<usize>, insertion: &str) -> DiffDelta {
    DiffDelta {
        replacement_line_range: range,
        insertion: insertion.to_owned(),
    }
}

#[test]
fn section_states_expand_and_collapse_for_approval_lifecycle() {
    let mut states = SectionStates::default();
    let keys = [
        SectionKey::Summary,
        SectionKey::File(0),
        SectionKey::File(1),
    ];

    states.collapse_all(&keys);
    assert!(states.is_collapsed(SectionKey::Summary));
    assert!(states.is_collapsed(SectionKey::File(0)));
    assert!(states.is_collapsed(SectionKey::File(1)));
    states.expand_all(&keys);
    assert!(!states.is_collapsed(SectionKey::Summary));
    assert!(!states.is_collapsed(SectionKey::File(0)));
    assert!(!states.is_collapsed(SectionKey::File(1)));

    states.toggle_collapsed(SectionKey::File(0));
    assert!(!states.is_collapsed(SectionKey::Summary));
    assert!(states.is_collapsed(SectionKey::File(0)));
    assert!(!states.is_collapsed(SectionKey::File(1)));
    states.collapse_all(&keys);
    assert!(states.is_collapsed(SectionKey::Summary));
    assert!(states.is_collapsed(SectionKey::File(0)));
    assert!(states.is_collapsed(SectionKey::File(1)));
}

/// toggle_expand_all collapses all when any are expanded, and expands all
/// when all are collapsed.
#[test]
fn toggle_expand_all_collapses_then_expands() {
    let mut states = SectionStates::default();
    let keys = [
        SectionKey::Summary,
        SectionKey::File(0),
        SectionKey::File(1),
    ];

    states.expand_all(&keys);
    states.toggle_expand_all(&keys);
    for &key in &keys {
        assert!(
            states.is_collapsed(key),
            "{key:?} should be collapsed after first toggle"
        );
    }

    states.toggle_expand_all(&keys);
    for &key in &keys {
        assert!(
            !states.is_collapsed(key),
            "{key:?} should be expanded after second toggle"
        );
    }

    states.toggle_collapsed(SectionKey::File(0));
    states.toggle_expand_all(&keys);
    for &key in &keys {
        assert!(
            states.is_collapsed(key),
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

#[test]
fn file_edit_stats_omit_zero_sides() {
    assert_eq!(file_edit_stats_label(3, 0).as_deref(), Some("+3"));
    assert_eq!(file_edit_stats_label(0, 2).as_deref(), Some("−2"));
    assert_eq!(file_edit_stats_label(3, 2).as_deref(), Some("+3 −2"));
    assert_eq!(file_edit_stats_label(0, 0), None);
}

#[test]
fn file_edit_header_spans_style_action_details_and_nonzero_stats() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|ctx| {
            let builder = TuiUiBuilder::from_app(ctx);
            let (added_only, _) = file_edit_header_spans(
                ToolCallDisplayState::Succeeded,
                "Updated lib.rs",
                Some((3, 0)),
                false,
                &builder,
            );
            assert_eq!(
                added_only
                    .iter()
                    .map(|(text, _)| text.as_str())
                    .collect::<Vec<_>>(),
                vec!["✓ ", "Updated", " lib.rs", " +3"]
            );
            assert_eq!(added_only[1].1.fg, builder.primary_text_style().fg);
            assert!(added_only[1].1.add_modifier.contains(Modifier::BOLD));
            assert_eq!(added_only[2].1.fg, builder.neutral_7_text_style().fg);
            assert!(!added_only[2].1.add_modifier.contains(Modifier::BOLD));
            assert!(!added_only[3].1.add_modifier.contains(Modifier::BOLD));

            let (removed_only, _) = file_edit_header_spans(
                ToolCallDisplayState::Succeeded,
                "Deleted old.rs",
                Some((0, 2)),
                false,
                &builder,
            );
            assert_eq!(
                removed_only
                    .iter()
                    .map(|(text, _)| text.as_str())
                    .collect::<Vec<_>>(),
                vec!["✓ ", "Deleted", " old.rs", " −2"]
            );

            let (zero_stats, _) = file_edit_header_spans(
                ToolCallDisplayState::Succeeded,
                "Updated empty.rs",
                Some((0, 0)),
                false,
                &builder,
            );
            assert_eq!(
                zero_stats
                    .iter()
                    .map(|(text, _)| text.as_str())
                    .collect::<Vec<_>>(),
                vec!["✓ ", "Updated", " empty.rs"]
            );
        });
    });
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

/// Seeds `TuiDiffStorage` on the given view with two update diffs so that
/// `render_diff_content` builds real collapsible sections.
fn seed_two_file_diffs(app: &mut App, view: &ViewHandle<TuiFileEditsView>) {
    let handle = app.read(|ctx| TuiDiffStorageHandle::new(view.as_ref(ctx).storage.clone()));
    app.update(|ctx| {
        handle.set_candidate_diffs(
            vec![
                update_diff("/tmp/a/lib.rs", None),
                update_diff("/tmp/b/main.rs", None),
            ],
            DiffSessionType::Local,
            ctx,
        );
    });
}

/// The blocked file-edits card renders the permission header, the
/// `e to expand/collapse` affordance, and the yes/no/Other options, and the
/// diff sections are expanded by default (AC 1, AC 3).
/// Also asserts repaint-stability: a second render with no input still shows
/// the expanded sections (guards the hover_state regression).
#[test]
fn blocked_file_edits_card_shows_expand_hint_sections_and_options() {
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
        // Seed two real diffs so sections exist and the card can render them expanded.
        seed_two_file_diffs(&mut app, &view);

        // First render: the blocked card must show the expanded summary header
        // and both file-section headers (AC 1).
        let lines1 = present_file_edits_view(&mut app, &view).buffer.to_lines();

        let has_header = lines1
            .iter()
            .any(|l| l.contains("Is it OK if I make these file edits?"));
        assert!(has_header, "blocked card header missing in {lines1:?}");

        // AC 3: the header row carries the `e to expand/collapse` affordance.
        let has_expand_hint = lines1.iter().any(|l| l.contains("e to expand/collapse"));
        assert!(
            has_expand_hint,
            "e-to-expand-collapse hint missing in {lines1:?}"
        );

        let has_yes_option = lines1.iter().any(|l| l.contains("yes"));
        assert!(has_yes_option, "yes option missing in {lines1:?}");

        // AC 1: the summary header and both file sections must be visible on first render.
        // The multi-file wrapper shows "Editing 2 files" and each file shows "Editing lib.rs".
        let has_summary = lines1.iter().any(|l| l.contains("Editing 2 files"));
        assert!(has_summary, "summary header missing in {lines1:?}");
        let has_lib_section = lines1.iter().any(|l| l.contains("Editing lib.rs"));
        assert!(has_lib_section, "lib.rs file section missing in {lines1:?}");
        let has_main_section = lines1.iter().any(|l| l.contains("Editing main.rs"));
        assert!(
            has_main_section,
            "main.rs file section missing in {lines1:?}"
        );

        // Repaint-stability (AC 1): a second render with no input must still show the
        // expanded sections — the hover_state call must not fabricate a collapsed entry.
        let lines2 = present_file_edits_view(&mut app, &view).buffer.to_lines();
        let has_summary2 = lines2.iter().any(|l| l.contains("Editing 2 files"));
        assert!(
            has_summary2,
            "summary header collapsed on second render — hover_state regression: {lines2:?}"
        );
        let has_lib2 = lines2.iter().any(|l| l.contains("Editing lib.rs"));
        assert!(
            has_lib2,
            "lib.rs section collapsed on second render — hover_state regression: {lines2:?}"
        );
    });
}

/// Pressing `e` while the file-edits permission card's option list is focused
/// dispatches `ToggleExpandAll` through the full responder chain, including
/// `TuiFileEditsView`, and the sections actually toggle (AC 4).
/// Also asserts repaint-stability: sections remain expanded after a repaint
/// with no input, then collapse on `e`, then expand again on a second `e`.
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
        // Seed two real diffs so ToggleExpandAll operates on actual sections.
        seed_two_file_diffs(&mut app, &view);

        // `e` is only gated when FILE_EDITS_PERMISSION_ACTIVE is set, which
        // requires the action to be blocked AND the list to be focused.
        app.read(|ctx| {
            let keymap_ctx = view.as_ref(ctx).keymap_context(ctx);
            assert!(
                keymap_ctx.set.contains(FILE_EDITS_PERMISSION_ACTIVE),
                "FILE_EDITS_PERMISSION_ACTIVE not set — list not focused or action not blocked"
            );
        });

        // Repaint-stability: render once so hover_state entries are populated,
        // then re-render with no input — sections must still be visible (expanded).
        let lines_before = present_file_edits_view(&mut app, &view).buffer.to_lines();
        assert!(
            lines_before.iter().any(|l| l.contains("Editing 2 files")),
            "summary unexpanded on first render: {lines_before:?}"
        );
        let lines_repaint = present_file_edits_view(&mut app, &view).buffer.to_lines();
        assert!(
            lines_repaint.iter().any(|l| l.contains("Editing 2 files")),
            "summary collapsed on repaint — hover_state regression: {lines_repaint:?}"
        );

        // Dispatch `e` through the focused view's responder chain. Because
        // `TuiFileEditsView` is an ancestor and its context predicate is
        // satisfied, the keystroke must be consumed and emit LayoutChanged.
        assert!(
            dispatch_focused_key(&mut app, &view, "e"),
            "`e` should be consumed as ToggleExpandAll"
        );
        // After first `e` all sections collapse (any-expanded → collapse-all).
        let lines_after_e1 = present_file_edits_view(&mut app, &view).buffer.to_lines();
        assert!(
            !lines_after_e1.iter().any(|l| l.contains("Editing lib.rs")),
            "lib.rs section must be collapsed after first `e`: {lines_after_e1:?}"
        );

        // Second `e` re-expands all sections (all-collapsed → expand-all).
        assert!(
            dispatch_focused_key(&mut app, &view, "e"),
            "second `e` should be consumed as ToggleExpandAll"
        );
        let lines_after_e2 = present_file_edits_view(&mut app, &view).buffer.to_lines();
        assert!(
            lines_after_e2.iter().any(|l| l.contains("Editing lib.rs")),
            "lib.rs section must be expanded after second `e`: {lines_after_e2:?}"
        );
    });
}
