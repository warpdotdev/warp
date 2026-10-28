use std::cell::RefCell;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use instant::Instant;
use warp_editor::content::buffer::{Buffer, BufferSnapshot};
use warp_editor::content::selection_model::BufferSelectionModel;
use warp_editor::content::text::IndentBehavior;
use warpui_core::color::ColorU;
use warpui_core::{App, ModelHandle};

use super::*;

/// Dense, deeply nested, syntactically-invalid SQL: unmatched parens force
/// tree-sitter's error-recovery machinery (`ts_parser__recover`) to repeatedly
/// fork and merge stack versions, guaranteeing its progress callback (checked
/// roughly every 100 parse actions) fires at least once. `repeat` controls the
/// input size; the caller picks a value that stays under [`MAX_PARSE_BYTES`] so
/// the test exercises the parse budget rather than the cheap size guard.
fn build_pathological_sql(repeat: usize) -> String {
    let mut source = String::from("SELECT * FROM t WHERE ");
    for _ in 0..repeat {
        source.push_str("(a = b AND (c OR (d = (");
    }
    source
}

fn test_color_map() -> ColorMap {
    let black = ColorU {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    ColorMap {
        keyword_color: black,
        function_color: black,
        string_color: black,
        type_color: black,
        number_color: black,
        comment_color: black,
        property_color: black,
        tag_color: black,
    }
}

/// A no-op model used only to subscribe to a [`SyntaxTreeState`]'s events from
/// tests (a model cannot subscribe to its own events).
struct EventSink;

impl Entity for EventSink {
    type Event = ();
}

#[test]
fn test_classify_parse_result_distinguishes_none_reasons() {
    // Pure-function coverage of the deadline-vs-superseded-vs-failed
    // classification, independent of whether a real tree-sitter
    // parser/scanner failure can be provoked in a test.
    assert!(matches!(
        classify_parse_result(None, Some(CancelReason::DeadlineExceeded)),
        ParseOutcome::BudgetExceeded
    ));
    assert!(matches!(
        classify_parse_result(None, Some(CancelReason::Superseded)),
        ParseOutcome::Superseded
    ));
    assert!(matches!(
        classify_parse_result(None, None),
        ParseOutcome::Failed
    ));
}

#[test]
fn test_parse_exceeding_deadline_falls_back_instead_of_completing() {
    let language = languages::language_by_name("sql").expect("sql language should be registered");
    let text_content = build_pathological_sql(2_000);
    assert!(
        text_content.len() < MAX_PARSE_BYTES,
        "test input must stay under the size cap to actually exercise the deadline rather than MAX_PARSE_BYTES"
    );

    // An already-past deadline deterministically trips on the first
    // progress-callback check, regardless of machine speed.
    let outcome = warpui_core::r#async::block_on(async {
        SyntaxTreeState::parse_text(
            BufferSnapshot::from_plain_text(&text_content),
            None,
            &language,
            Instant::now(),
            Arc::new(AtomicBool::new(false)),
        )
        .await
    });

    assert!(
        matches!(outcome, ParseOutcome::BudgetExceeded),
        "a dense-error parse well within MAX_PARSE_BYTES should trip the deadline instead of running to completion"
    );
}

#[test]
fn test_parse_returns_superseded_when_cancel_flag_is_set() {
    let language = languages::language_by_name("sql").expect("sql language should be registered");
    let text_content = build_pathological_sql(2_000);

    // The cancel flag is pre-armed and the deadline is far in the future, so a
    // BudgetExceeded outcome here would indicate the deadline check is
    // incorrectly winning over (or masking) the cancellation flag.
    let outcome = warpui_core::r#async::block_on(async {
        SyntaxTreeState::parse_text(
            BufferSnapshot::from_plain_text(&text_content),
            None,
            &language,
            Instant::now() + Duration::from_secs(60),
            Arc::new(AtomicBool::new(true)),
        )
        .await
    });

    assert!(
        matches!(outcome, ParseOutcome::Superseded),
        "a pre-cancelled parse should report Superseded, not BudgetExceeded or Failed"
    );
}

#[test]
fn test_parse_skips_oversized_buffer_without_attempting_to_parse() {
    let language = languages::language_by_name("sql").expect("sql language should be registered");
    let text_content = "a".repeat(MAX_PARSE_BYTES + 1);

    let outcome = warpui_core::r#async::block_on(async {
        SyntaxTreeState::parse_text(
            BufferSnapshot::from_plain_text(&text_content),
            None,
            &language,
            Instant::now() + Duration::from_secs(60),
            Arc::new(AtomicBool::new(false)),
        )
        .await
    });

    assert!(matches!(outcome, ParseOutcome::TooLarge));
}

/// Builds a buffer + wired-up [`SyntaxTreeState`] model for a given source text.
fn setup_syntax_tree_state(
    app: &mut App,
    text_content: &str,
    language: Arc<Language>,
) -> (ModelHandle<SyntaxTreeState>, BufferVersion, BufferSnapshot) {
    let buffer_handle = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
    let selection = app.add_model(|_| BufferSelectionModel::new(buffer_handle.clone()));
    buffer_handle.update(app, |buffer, ctx| {
        *buffer = Buffer::from_plain_text(
            text_content,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            selection,
            ctx,
        );
    });

    let buffer_version = buffer_handle.read(app, |buffer, _| buffer.buffer_version());
    let buffer_snapshot = buffer_handle.read(app, |buffer, _| buffer.buffer_snapshot());

    let syntax_tree_handle = app.add_model(|_| {
        let mut state =
            SyntaxTreeState::new(buffer_handle.downgrade(), buffer_version, test_color_map());
        state.set_language(language);
        state
    });

    (syntax_tree_handle, buffer_version, buffer_snapshot)
}

/// Drives a real `ParseOutcome::BudgetExceeded` completion through
/// `update_internal_state_with_delta` (by forcing an already-elapsed parse
/// budget) and asserts on the actual production side effects: the buffer
/// latches, its tree is discarded, `DecorationUpdated` fires, and no
/// replacement parse is left scheduled. None of this depends on wall-clock
/// timing.
#[test]
fn test_budget_exceeded_completion_latches_discards_tree_and_stops_reparsing() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let language =
            languages::language_by_name("sql").expect("sql language should be registered");
        let text_content = build_pathological_sql(2_000);
        let (syntax_tree_handle, buffer_version, buffer_snapshot) =
            setup_syntax_tree_state(app, &text_content, language);

        syntax_tree_handle.update(app, |state, _ctx| {
            state.set_parse_budget_for_test(Duration::ZERO);
        });

        let (tx, rx) = futures::channel::oneshot::channel();
        let tx = RefCell::new(Some(tx));
        let _sink = app.add_model(|ctx| {
            ctx.subscribe_to_model(&syntax_tree_handle, move |_sink, _emitter, _event, _ctx| {
                if let Some(tx) = tx.borrow_mut().take() {
                    let _ = tx.send(());
                }
            });
            EventSink
        });

        syntax_tree_handle.update(app, |state, ctx| {
            state.update_internal_state_with_delta(&[], buffer_version, buffer_snapshot, ctx);
        });

        rx.await
            .expect("DecorationUpdated should fire for the budget-exceeded completion");

        syntax_tree_handle.read(app, |state, _ctx| {
            assert!(
                state.parse_budget_exceeded,
                "a confirmed deadline cancellation should latch"
            );
            assert!(
                !state.syntax_tree.lock().contains_key(&buffer_version),
                "the tree for the budget-exceeded version should have been discarded"
            );
            assert!(
                state.active_parse_cancel.is_none(),
                "no parse should be left in flight after the completion settles"
            );
            assert!(
                state.pending_edit.is_none(),
                "no replacement parse should be scheduled after a budget-exceeded completion"
            );
        });

        // A further edit must not re-attempt a parse: the latch persists and no
        // background parse gets dispatched (this is what would otherwise
        // re-spend the parse budget on every keystroke).
        syntax_tree_handle.update(app, |state, ctx| {
            state.update_internal_state_with_delta(
                &[],
                buffer_version,
                BufferSnapshot::from_plain_text(&text_content),
                ctx,
            );
        });
        syntax_tree_handle.read(app, |state, _ctx| {
            assert!(
                state.active_parse_cancel.is_none(),
                "a latched buffer must not dispatch another parse on the next edit"
            );
            assert!(state.parse_budget_exceeded);
        });
    });
}

/// Two edits arriving before the first parse's completion is observed must not
/// run two concurrent tree-sitter parses: the in-flight one is signalled to
/// cancel and the newer edit is coalesced (queued) instead of starting a
/// second parse right away. This is the core fix for the "abort() doesn't
/// actually stop the blocking parse" issue -- without it, rapid edits on a
/// pathological buffer would pile up additional full-budget parses on the
/// background executor.
#[test]
fn test_rapid_edits_coalesce_instead_of_running_concurrent_parses() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let language =
            languages::language_by_name("sql").expect("sql language should be registered");
        let (syntax_tree_handle, buffer_version, buffer_snapshot) =
            setup_syntax_tree_state(app, "SELECT 1;", language);

        // No `.await` between these two updates, so the foreground executor
        // never gets a chance to process the background parse's completion in
        // between -- the assertions below reflect exactly what
        // `update_internal_state_with_delta` does synchronously.
        syntax_tree_handle.update(app, |state, ctx| {
            state.update_internal_state_with_delta(
                &[],
                buffer_version,
                buffer_snapshot.clone(),
                ctx,
            );
        });

        let cancel_flag = syntax_tree_handle.read(app, |state, _ctx| {
            assert!(
                state.active_parse_cancel.is_some(),
                "the first edit should dispatch a parse"
            );
            assert!(state.pending_edit.is_none());
            state.active_parse_cancel.clone().unwrap()
        });

        syntax_tree_handle.update(app, |state, ctx| {
            state.update_internal_state_with_delta(&[], buffer_version, buffer_snapshot, ctx);
        });

        assert!(
            cancel_flag.load(Ordering::Relaxed),
            "a second edit while a parse is in flight must signal it to cancel"
        );
        syntax_tree_handle.read(app, |state, _ctx| {
            assert!(
                state.pending_edit.is_some(),
                "the second edit should be coalesced rather than starting a concurrent parse"
            );
        });
    });
}
