use std::collections::HashSet;
use std::time::Duration;

use chrono::Local;
use itertools::Itertools;
use ordered_float::OrderedFloat;
use warpui::r#async::Timer;
use warpui::elements::Empty;
use warpui::{App, AppContext, Element};

use super::*;
use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::search::command_search::history::{history_data_source, history_data_source_with_cwd};
use crate::search::command_search::searcher::CommandSearchMixer;
use crate::search::data_source::{Query, QueryResult};
use crate::search::item::SearchItem;
use crate::search::mixer::{
    AddAsyncSourceOptions, AsyncDataSource, BoxFuture, DataSourceRunErrorWrapper,
};
use crate::search::result_renderer::ItemHighlightState;
use crate::search::{QueryFilter, SyncDataSource};
use crate::server::server_api::ServerApiProvider;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::terminal::HistoryEntry;
use crate::terminal::model::session::SessionId;

#[derive(Clone, Debug)]
enum TestItemAction {
    Result,
}
type TestMixer = SearchMixer<TestItemAction>;

#[derive(Clone, Debug)]
struct TestSearchItem {
    is_async: bool,
}

impl SearchItem for TestSearchItem {
    type Action = TestItemAction;

    fn render_icon(&self, _: ItemHighlightState, _: &Appearance) -> Box<dyn Element> {
        Empty::new().finish()
    }

    fn render_item(&self, _: ItemHighlightState, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }

    fn render_details(&self, _: &AppContext) -> Option<Box<dyn Element>> {
        None
    }

    fn score(&self) -> OrderedFloat<f64> {
        if self.is_async {
            OrderedFloat(0.5)
        } else {
            OrderedFloat(0.)
        }
    }

    fn accept_result(&self) -> TestItemAction {
        TestItemAction::Result
    }

    fn execute_result(&self) -> TestItemAction {
        TestItemAction::Result
    }

    fn accessibility_label(&self) -> String {
        if self.is_async {
            "Async Test Result".to_string()
        } else {
            "Sync Test Result".to_string()
        }
    }
}

/// A data source that is both sync and async.
/// When async, waits 100ms before returning a static result.
/// Note: the async data source produces an item with a higher score than the
/// item that the sync data source produces.
struct SlowDataSource {}

impl AsyncDataSource for SlowDataSource {
    type Action = TestItemAction;

    fn run_query(
        &self,
        _: &Query,
        _: &AppContext,
    ) -> BoxFuture<'static, Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper>> {
        Box::pin(async move {
            Timer::after(Duration::from_millis(100)).await;
            Ok(vec![TestSearchItem { is_async: true }.into()])
        })
    }
}

impl SyncDataSource for SlowDataSource {
    type Action = TestItemAction;

    fn run_query(
        &self,
        _: &Query,
        _: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        Ok(vec![TestSearchItem { is_async: false }.into()])
    }
}

fn initialize_app(app: &mut App) {
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(AuthManager::new_for_test);
}

#[test]
fn test_add_source_to_mixer() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let mixer = app.add_model(|_| CommandSearchMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                history_data_source(vec![HistoryEntry::command_only(
                    "git checkout master".to_owned(),
                )]),
                HashSet::from([QueryFilter::History]),
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: true,
                },
                ctx,
            );
        });
        app.read(|app| {
            assert!(
                mixer
                    .as_ref(app)
                    .registered_filters()
                    .any(|filter| filter == QueryFilter::History)
            );
        });
    });
}

#[test]
fn test_exact_matches_rank_above_prefix_matches() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let short_command = "git".to_owned();
        let long_command = "git checkout master".to_owned();
        let unrelated_command = "echo hello!".to_owned();

        let mixer = app.add_model(|_| CommandSearchMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                history_data_source(vec![HistoryEntry::command_only(long_command.clone())]),
                HashSet::from([QueryFilter::History]),
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: true,
                },
                ctx,
            );
            mixer.add_async_source(
                history_data_source(vec![
                    HistoryEntry::command_only(short_command.clone()),
                    HistoryEntry::command_only(unrelated_command),
                ]),
                HashSet::from([QueryFilter::History]),
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: true,
                },
                ctx,
            );

            mixer.run_query("git".into(), ctx);
        });

        Timer::after(Duration::from_millis(200)).await;

        app.read(|app| {
            let results = mixer.as_ref(app).results();

            // The view renders highest-ranked items at the bottom (last index) of the scrollable
            // panel. `short_command` is a whole-line exact match for the query while
            // `long_command` is only a substring match, so it outranks `long_command` and ends up
            // last, independent of which data source was registered first.
            assert_eq!(results.len(), 2);

            assert!(matches!(
            results.first().map(|result| result.accept_result()),
            Some(CommandSearchItemAction::AcceptHistory(AcceptedHistoryItem { command: long, linked_workflow_data: None })) if long == long_command));

            assert!(matches!(
            results.get(1).map(|result| result.accept_result()),
            Some(CommandSearchItemAction::AcceptHistory(AcceptedHistoryItem { command: short, linked_workflow_data: None })) if short == short_command));
        });
    })
}

#[test]
fn test_blank_query_preserves_chronological_order_despite_differing_priors() {
    // `SearchMixer` invokes history with an empty query for the zero state (`run_in_zero_state:
    // true`), where match quality is necessarily zero. Bypassing only the score floor for a
    // blank query still lets priors reorder the zero state away from
    // `History::commands_shared()`'s established chronological order (oldest first, newest
    // last/closest to the input) -- e.g. an older command whose session_id happens to match
    // would leapfrog a newer one. Priors must be ignored entirely for a blank query, not just
    // let through the floor.
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Both entries share a timestamp so recency can't be what preserves order; only ignoring
        // the session prior can.
        let same_ts = Local::now();
        let mut older_matches_session = HistoryEntry::command_only("git status".to_owned());
        older_matches_session.start_ts = Some(same_ts);
        // `history_data_source` fixes the data source's own session at `SessionId::from(0)`, so
        // this entry would earn a session-prior bonus if priors weren't bypassed.
        older_matches_session.session_id = Some(SessionId::from(0));

        let mut newer_different_session = HistoryEntry::command_only("git log".to_owned());
        newer_different_session.start_ts = Some(same_ts);
        newer_different_session.session_id = Some(SessionId::from(1));

        let mixer = app.add_model(|_| CommandSearchMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                history_data_source(vec![older_matches_session, newer_different_session]),
                HashSet::from([QueryFilter::History]),
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: true,
                    run_when_unfiltered: true,
                },
                ctx,
            );
            mixer.run_query(
                Query {
                    text: "".to_owned(),
                    filters: HashSet::new(),
                },
                ctx,
            );
        });

        Timer::after(Duration::from_millis(200)).await;

        app.read(|app| {
            let results = mixer.as_ref(app).results();
            assert_eq!(
                results.len(),
                2,
                "history should still populate the zero state for a blank query"
            );

            // The newer entry must stay last (closest to the input), matching insertion order,
            // even though the older entry's session_id happens to match the data source's
            // session and would otherwise earn a session-prior bonus.
            assert!(matches!(
                results.last().map(|result| result.accept_result()),
                Some(CommandSearchItemAction::AcceptHistory(AcceptedHistoryItem {
                    command,
                    ..
                })) if command == "git log"
            ));
        });
    });
}

#[test]
fn test_current_cwd_prior_uses_live_cwd_not_stale_last_entry_pwd() {
    // Regression test: `HistoryEntry::pwd` is captured when a command *starts*, so the most
    // recently executed command's `pwd` reflects the directory it was run *from*, not the live
    // cwd after it runs (e.g. immediately after `cd /repo`). The cwd prior must use the live cwd
    // passed in from the caller, not fall back to guessing from history entries.
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        // Both entries share a timestamp so recency is identical between them, isolating the cwd
        // prior as the only difference in their final score.
        let same_ts = Local::now();
        let mut matches_live_cwd = HistoryEntry::command_only("npm test -- repo".to_owned());
        matches_live_cwd.start_ts = Some(same_ts);
        matches_live_cwd.pwd = Some("/repo".to_owned());

        // The most recently executed command (last in the list), whose pwd is exactly what the
        // old, buggy heuristic (the last candidate's `pwd`) would have picked as "current_cwd",
        // even though the live cwd is actually `/repo`.
        let mut stale_last_entry = HistoryEntry::command_only("npm test -- home".to_owned());
        stale_last_entry.start_ts = Some(same_ts);
        stale_last_entry.pwd = Some("/home/user".to_owned());

        let mixer = app.add_model(|_| CommandSearchMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                history_data_source_with_cwd(
                    vec![matches_live_cwd, stale_last_entry],
                    Some("/repo".to_owned()),
                ),
                HashSet::from([QueryFilter::History]),
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: true,
                },
                ctx,
            );
            mixer.run_query("npm test".into(), ctx);
        });

        Timer::after(Duration::from_millis(200)).await;

        app.read(|app| {
            let results = mixer.as_ref(app).results();
            assert_eq!(results.len(), 2);

            // The entry whose pwd matches the live cwd should rank last (best), even though it
            // isn't the most recently executed command.
            assert!(matches!(
                results.last().map(|result| result.accept_result()),
                Some(CommandSearchItemAction::AcceptHistory(AcceptedHistoryItem {
                    command,
                    ..
                })) if command == "npm test -- repo"
            ));
        });
    });
}

#[test]
fn test_no_query_filter_runs_all_data_sources() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let mixer = app.add_model(|_| CommandSearchMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                history_data_source(vec![HistoryEntry::command_only("git".to_owned())]),
                HashSet::from([QueryFilter::History]),
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: true,
                },
                ctx,
            );
            mixer.add_async_source(
                history_data_source(vec![HistoryEntry::command_only("git checkout".to_owned())]),
                HashSet::from([QueryFilter::Workflows]),
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: true,
                },
                ctx,
            );

            // Running a query with no filters should produce results from both sources.
            mixer.run_query("git".into(), ctx);
        });

        Timer::after(Duration::from_millis(200)).await;

        app.read(|app| {
            let results = mixer.as_ref(app).results();

            // "git" ranks above "git checkout" because it's a whole-line exact match for the
            // query, not merely a substring match, so it appears last (closest to the input).
            assert_eq!(
                results
                    .iter()
                    .map(|result| result.accessibility_label())
                    .collect_vec(),
                vec!["History item: git checkout", "History item: git"]
            );
        });
    });
}

#[test]
fn test_query_filter_limits_data_sources() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let mixer = app.add_model(|_| CommandSearchMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                history_data_source(vec![HistoryEntry::command_only("git".to_owned())]),
                HashSet::from([QueryFilter::History]),
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: true,
                },
                ctx,
            );
            mixer.add_async_source(
                history_data_source(vec![HistoryEntry::command_only("git checkout".to_owned())]),
                HashSet::from([QueryFilter::Workflows]),
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: true,
                },
                ctx,
            );

            // Limiting results to a single query filter should only produces results from that source.
            mixer.run_query(
                Query {
                    filters: HashSet::from([QueryFilter::History]),
                    text: "git".into(),
                },
                ctx,
            );
        });

        Timer::after(Duration::from_millis(200)).await;

        app.read(|app| {
            let results = mixer.as_ref(app).results();
            assert_eq!(
                results
                    .iter()
                    .map(|result| result.accessibility_label())
                    .collect_vec(),
                vec!["History item: git"]
            );
        });

        mixer.update(&mut app, |mixer, ctx| {
            // Specifying both filters should produce results from both sources.
            mixer.run_query(
                Query {
                    filters: HashSet::from([QueryFilter::History, QueryFilter::Workflows]),
                    text: "git".into(),
                },
                ctx,
            );
        });

        Timer::after(Duration::from_millis(200)).await;

        app.read(|app| {
            let results = mixer.as_ref(app).results();
            // "git" ranks above "git checkout" because it's a whole-line exact match for the
            // query, not merely a substring match, so it appears last (closest to the input).
            assert_eq!(
                results
                    .iter()
                    .map(|result| result.accessibility_label())
                    .collect_vec(),
                vec!["History item: git checkout", "History item: git"]
            );
        });
    });
}

#[test]
fn test_async_data_source() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let mixer = app.add_model(|_| TestMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                SlowDataSource {},
                [QueryFilter::Actions],
                AddAsyncSourceOptions {
                    debounce_interval: Some(Duration::from_millis(100)),
                    run_in_zero_state: false,
                    run_when_unfiltered: false,
                },
                ctx,
            );

            // We need to run with a non-empty text and a matching filter
            // to ensure the async source matches the query.
            mixer.run_query(
                Query {
                    text: "a".to_owned(),
                    filters: HashSet::from([QueryFilter::Actions]),
                },
                ctx,
            );
        });

        // Since the debounce period is 100ms and the SlowDataSource
        // takes 100ms, waiting 500ms should be more than sufficient.
        Timer::after(Duration::from_millis(500)).await;

        app.read(|app| {
            let results = mixer.as_ref(app).results();
            assert_eq!(
                results
                    .iter()
                    .map(|result| result.accessibility_label())
                    .collect_vec(),
                vec!["Async Test Result"]
            );
        });
    });
}

#[test]
fn test_async_data_source_run_twice_with_debounce() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let mixer = app.add_model(|_| TestMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                SlowDataSource {},
                [QueryFilter::Actions],
                AddAsyncSourceOptions {
                    debounce_interval: Some(Duration::from_millis(10)),
                    run_in_zero_state: false,
                    run_when_unfiltered: false,
                },
                ctx,
            );

            // We need to run with a non-empty text and a matching filter
            // to ensure the async source matches the query.
            mixer.run_query(
                Query {
                    text: "a".to_owned(),
                    filters: HashSet::from_iter([QueryFilter::Actions]),
                },
                ctx,
            );
        });

        // After 50ms, the query would have started to run (because 10ms have elapsed)
        // but it wouldn't have completed because it takes 100ms to complete.
        Timer::after(Duration::from_millis(50)).await;

        // Start another query while the other one has started but not completed.
        mixer.update(&mut app, |mixer, ctx| {
            // We need to run with a non-empty text and a matching filter
            // to ensure the async source matches the query.
            mixer.run_query(
                Query {
                    text: "a".to_owned(),
                    filters: HashSet::from_iter([QueryFilter::Actions]),
                },
                ctx,
            );
        });

        // Wait till all queries are complete.
        Timer::after(Duration::from_millis(500)).await;

        // There should only be one result.
        app.read(|app| {
            let results = mixer.as_ref(app).results();
            assert_eq!(
                results
                    .iter()
                    .map(|result| result.accessibility_label())
                    .collect_vec(),
                vec!["Async Test Result"]
            );
        });
    });
}

#[test]
fn test_async_data_source_run_twice_without_debounce() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let mixer = app.add_model(|_| TestMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                SlowDataSource {},
                [QueryFilter::Actions],
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: false,
                },
                ctx,
            );

            // We need to run with a non-empty text and a matching filter
            // to ensure the async source matches the query.
            mixer.run_query(
                Query {
                    text: "a".to_owned(),
                    filters: HashSet::from_iter([QueryFilter::Actions]),
                },
                ctx,
            );
            mixer.run_query(
                Query {
                    text: "a".to_owned(),
                    filters: HashSet::from_iter([QueryFilter::Actions]),
                },
                ctx,
            );
        });

        // Since the debounce period is 100ms and the SlowDataSource
        // takes 100ms, waiting 500ms should be more than sufficient.
        Timer::after(Duration::from_millis(500)).await;

        // There should only be one result.
        app.read(|app| {
            let results = mixer.as_ref(app).results();
            assert_eq!(
                results
                    .iter()
                    .map(|result| result.accessibility_label())
                    .collect_vec(),
                vec!["Async Test Result"]
            );
        });
    });
}

#[test]
fn test_async_source_with_include_in_unfiltered_runs_on_empty_filters() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let mixer = app.add_model(|_| TestMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                SlowDataSource {},
                [QueryFilter::Files],
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: true,
                },
                ctx,
            );

            // Run with non-empty text but no filters (unfiltered mode).
            mixer.run_query(
                Query {
                    text: "a".to_owned(),
                    filters: HashSet::new(),
                },
                ctx,
            );
        });

        Timer::after(Duration::from_millis(500)).await;

        app.read(|app| {
            let results = mixer.as_ref(app).results();
            assert_eq!(
                results
                    .iter()
                    .map(|result| result.accessibility_label())
                    .collect_vec(),
                vec!["Async Test Result"]
            );
        });
    });
}

#[test]
fn test_async_source_without_include_in_unfiltered_skipped_on_empty_filters() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let mixer = app.add_model(|_| TestMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                SlowDataSource {},
                [QueryFilter::Files],
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: false,
                },
                ctx,
            );

            // Run with non-empty text but no filters (unfiltered mode).
            mixer.run_query(
                Query {
                    text: "a".to_owned(),
                    filters: HashSet::new(),
                },
                ctx,
            );
        });

        Timer::after(Duration::from_millis(500)).await;

        // The async source should NOT have run because run_when_unfiltered is false.
        app.read(|app| {
            let results = mixer.as_ref(app).results();
            assert!(results.is_empty());
        });
    });
}

#[test]
fn test_sync_and_async_data_sources() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let mixer = app.add_model(|_| TestMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_sync_source(SlowDataSource {}, [QueryFilter::Actions]);
            mixer.add_async_source(
                SlowDataSource {},
                [QueryFilter::Actions],
                AddAsyncSourceOptions {
                    debounce_interval: Some(Duration::from_millis(100)),
                    run_in_zero_state: false,
                    run_when_unfiltered: false,
                },
                ctx,
            );

            // We need to run with a non-empty text and a matching filter
            // to ensure the async source matches the query.
            mixer.run_query(
                Query {
                    text: "a".to_owned(),
                    filters: HashSet::from_iter([QueryFilter::Actions]),
                },
                ctx,
            );
        });

        // Results are buffered until all sources finish, so nothing is visible yet.
        app.read(|app| {
            let results = mixer.as_ref(app).results();
            assert!(results.is_empty());
        });

        // Since the debounce period is 100ms and the SlowDataSource
        // takes 100ms, waiting 500ms should be more than sufficient.
        Timer::after(Duration::from_millis(500)).await;

        // After the async data source runs, there should just be two items with the async data
        // source item having a higher score (so it appears after).
        app.read(|app| {
            let results = mixer.as_ref(app).results();
            assert_eq!(
                results
                    .iter()
                    .map(|result| result.accessibility_label())
                    .collect_vec(),
                vec!["Sync Test Result", "Async Test Result"]
            );
        });
    });
}
