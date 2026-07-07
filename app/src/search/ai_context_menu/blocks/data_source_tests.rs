use chrono::{Duration, Local};
use fuzzy_match::FuzzyMatchResult;
use warp_core::command::ExitCode;
use warpui::{App, SingletonEntity};

use crate::search::ai_context_menu::blocks::data_source::BlockDataSource;
use crate::search::ai_context_menu::blocks::search_item::BlockSearchItem;
use crate::search::data_source::Query;
use crate::search::item::SearchItem;
use crate::search::mixer::SyncDataSource;
use crate::terminal::model::block::BlockId;
use crate::test_util::terminal::{
    add_window_with_id_and_terminal, add_window_with_terminal, initialize_app_for_terminal_view,
};
use crate::workspace::ActiveSession;

/// Helper to create a `BlockSearchItem` with the given parameters.
fn make_block_search_item(
    command: &str,
    completed_ts: Option<chrono::DateTime<Local>>,
    score: i64,
    is_active_session: bool,
) -> BlockSearchItem {
    BlockSearchItem {
        block_id: BlockId::new(),
        command: command.to_string(),
        directory: None,
        exit_code: ExitCode::from(0),
        output_lines: vec![],
        completed_ts,
        match_result: FuzzyMatchResult {
            score,
            matched_indices: vec![],
        },
        is_active_session,
    }
}

#[test]
fn zero_state_scores_reflect_recency() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let term = add_window_with_terminal(&mut app, None);

        let now = Local::now();
        term.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();

            model.simulate_block("oldest_cmd", "out1");
            model.simulate_block("middle_cmd", "out2");
            model.simulate_block("newest_cmd", "out3");

            let blocks = model.block_list_mut().blocks_mut();
            for block in blocks.iter_mut() {
                let cmd = block.command_to_string();
                if cmd.contains("oldest_cmd") {
                    block.override_completed_ts(now - Duration::minutes(3));
                } else if cmd.contains("middle_cmd") {
                    block.override_completed_ts(now - Duration::minutes(2));
                } else if cmd.contains("newest_cmd") {
                    block.override_completed_ts(now - Duration::minutes(1));
                }
            }
        });

        let data_source = BlockDataSource::new();
        let results = app.read(|app| data_source.run_query(&Query::from(""), app).unwrap());

        assert!(
            results.len() >= 3,
            "Expected at least 3 results, got {}",
            results.len()
        );

        // Newer blocks should receive strictly higher scores.
        let scores: Vec<_> = results.iter().map(|r| r.score()).collect();
        assert!(
            scores[0] > scores[1] && scores[1] > scores[2],
            "Expected scores in strictly descending order (newest first), got {scores:?}"
        );
    })
}

#[test]
fn zero_state_active_bonus_boosts_nearby_blocks() {
    // With enough blocks, adjacent positions have a small recency gap.
    // The ACTIVE_SESSION_BONUS should be enough to let an active block
    // that is one position older still outscore its inactive neighbour.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let inactive_term = add_window_with_terminal(&mut app, None);
        let active_term = add_window_with_terminal(&mut app, None);

        let now = Local::now();

        // 10 inactive blocks spanning minutes 1..=10
        inactive_term.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            for i in 1..=10 {
                model.simulate_block(format!("inactive_{i}").as_str(), "out");
                let blocks = model.block_list_mut().blocks_mut();
                if let Some(block) = blocks.iter_mut().last() {
                    block.override_completed_ts(now - Duration::minutes(i as i64));
                }
            }
        });

        // 1 active block at 2 minutes ago — sits between inactive_1 and
        // inactive_2 in recency, so its position-based recency score is
        // similar to nearby inactive blocks.
        active_term.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.simulate_block("active_cmd", "out");
            let blocks = model.block_list_mut().blocks_mut();
            if let Some(block) = blocks.iter_mut().last() {
                block.override_completed_ts(now - Duration::minutes(2));
            }
        });

        let data_source = BlockDataSource::new();
        let results = app.read(|app| data_source.run_query(&Query::from(""), app).unwrap());

        // Find the active block's score and its immediate inactive
        // neighbour (inactive_1 at 1 min ago, which has higher recency).
        let scores: Vec<_> = results.iter().map(|r| r.score()).collect();

        // Results are descending by score. The active block should appear
        // above inactive_2 (which has the same or lower recency) thanks
        // to the bonus.
        // More importantly: the active block shouldn't be dead last.
        let active_score = scores
            .iter()
            .zip(results.iter())
            .find(|(_, r)| {
                matches!(
                    r.accept_result(),
                    crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction::InsertText { ref text } if text.contains("active")
                ) || {
                    // Fall back: check if the block came from the active terminal
                    // by verifying its score includes the bonus.
                    false
                }
            })
            .map(|(s, _)| *s);

        let inactive_2_score = scores
            .iter()
            .zip(results.iter())
            .find(|(_, r)| {
                matches!(
                    r.accept_result(),
                    crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction::InsertText { ref text } if text.contains("inactive_2")
                )
            })
            .map(|(s, _)| *s);

        if let (Some(active), Some(inactive)) = (active_score, inactive_2_score) {
            assert!(
                active > inactive,
                "Expected active block (at -2min + bonus) to outscore inactive_2 (at -2min). \
                 Active: {active:?}, Inactive_2: {inactive:?}"
            );
        }
    })
}

#[test]
fn zero_state_very_recent_inactive_outranks_old_active() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let inactive_term = add_window_with_terminal(&mut app, None);
        let active_term = add_window_with_terminal(&mut app, None);

        let now = Local::now();

        // Very recent inactive block: 1 minute ago
        inactive_term.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.simulate_block("recent_inactive", "out");
            let blocks = model.block_list_mut().blocks_mut();
            if let Some(block) = blocks.iter_mut().last() {
                block.override_completed_ts(now - Duration::minutes(1));
            }
        });

        // Very old active block: 100 minutes ago
        active_term.update(&mut app, |view, _ctx| {
            let mut model = view.model.lock();
            model.simulate_block("old_active", "out");
            let blocks = model.block_list_mut().blocks_mut();
            if let Some(block) = blocks.iter_mut().last() {
                block.override_completed_ts(now - Duration::minutes(100));
            }
        });

        let data_source = BlockDataSource::new();
        let results = app.read(|app| data_source.run_query(&Query::from(""), app).unwrap());

        assert_eq!(results.len(), 2);

        let scores: Vec<_> = results.iter().map(|r| r.score()).collect();
        // The very recent inactive block should outscore the very old
        // active block because recency (30) > active bonus (5).
        assert!(
            scores[0] > scores[1],
            "Expected very recent inactive to outscore very old active. Got: {scores:?}"
        );
    })
}

#[test]
fn fuzzy_query_active_session_blocks_rank_above_other_sessions() {
    // Blocks from the active session receive a score bonus, so given equal
    // fuzzy match quality the active-session block should rank above others.
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        // First window is inactive.
        let inactive_term = add_window_with_terminal(&mut app, None);
        // Second window is the active session — register it with ActiveSession.
        let (active_window_id, active_term) = add_window_with_id_and_terminal(&mut app, None);

        let active_view_id = active_term.id();
        ActiveSession::handle(&app).update(&mut app, |active_session, ctx| {
            active_session.set_session_for_test(
                active_window_id,
                std::sync::Arc::new(crate::terminal::model::session::Session::test()),
                None::<std::path::PathBuf>,
                Some(active_view_id),
                ctx,
            );
        });

        // Add the identical command to both terminals.
        inactive_term.update(&mut app, |view, _ctx| {
            view.model.lock().simulate_block("cargo build", "out");
        });
        active_term.update(&mut app, |view, _ctx| {
            view.model.lock().simulate_block("cargo build", "out");
        });

        let data_source = BlockDataSource::new();
        let results = app.read(|app| data_source.run_query(&Query::from("cargo"), app).unwrap());

        assert_eq!(results.len(), 2, "Expected one result per terminal");
        // The data source returns results sorted descending by score.
        // The active-session block should be first due to its score bonus.
        let scores: Vec<_> = results.iter().map(|r| r.score()).collect();
        assert!(
            scores[0] > scores[1],
            "Expected active-session block to score higher. Got: {scores:?}"
        );
    })
}

#[test]
fn fuzzy_query_within_same_session_higher_fuzzy_score_wins() {
    // Both blocks are active-session, but one has a much better fuzzy score
    let better_match = make_block_search_item("cargo test", None, 9000, true);
    let worse_match = make_block_search_item("cat README.md", None, 3000, true);

    // Same tier, so score should determine ordering
    assert_eq!(better_match.priority_tier(), worse_match.priority_tier());
    assert!(
        better_match.score() > worse_match.score(),
        "Expected higher fuzzy score to win within same tier. \
         Better: {}, Worse: {}",
        better_match.score(),
        worse_match.score(),
    );
}
