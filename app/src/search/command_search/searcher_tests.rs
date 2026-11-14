use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::Local;
use itertools::Itertools;
use ordered_float::OrderedFloat;
use warp_core::command::ExitCode;
use warp_core::features::FeatureFlag;
use warpui::r#async::Timer;
use warpui::elements::Empty;
use warpui::{App, AppContext, Element, SingletonEntity};

use super::*;
use crate::ai::blocklist::AIQueryHistoryOutputStatus;
use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::search::ai_queries::fuzzy_match::FuzzyMatchAIQueryResults;
use crate::search::command_search::ai_queries::AIQuerySearchResultItem;
use crate::search::command_search::history::{
    history_data_source, history_data_source_for_session,
};
use crate::search::command_search::searcher::CommandSearchMixer;
use crate::search::command_search::workflows::{WorkflowIdentity, WorkflowSearchItem};
use crate::search::data_source::{Query, QueryResult};
use crate::search::item::SearchItem;
use crate::search::mixer::{
    AddAsyncSourceOptions, AsyncDataSource, BoxFuture, DataSourceRunErrorWrapper,
};
use crate::search::result_renderer::ItemHighlightState;
use crate::search::workflows::fuzzy_match::FuzzyMatchWorkflowResult;
use crate::search::{QueryFilter, SyncDataSource};
use crate::server::server_api::ServerApiProvider;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;
use crate::terminal::model::session::{Session, SessionId, SessionInfo};
use crate::terminal::{History, HistoryEntry};
use crate::test_util::assert_eventually;
use crate::workflows::workflow::Workflow;
use crate::workflows::{WorkflowSource, WorkflowType};

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

/// A sync data source returning a fixed, pre-built set of results.
struct FixedResults<T>(Vec<T>);

impl<T: SearchItem<Action = CommandSearchItemAction> + Clone + 'static> SyncDataSource
    for FixedResults<T>
{
    type Action = CommandSearchItemAction;

    fn run_query(
        &self,
        _: &Query,
        _: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        Ok(self.0.iter().cloned().map(Into::into).collect())
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
fn test_history_data_source_reflects_live_exit_status_update() {
    let _flag = FeatureFlag::HistorySearchRankingV2.override_enabled(true);

    App::test((), |mut app| async move {
        crate::test_util::terminal::initialize_app_for_terminal_view(&mut app);

        let session = Arc::new(Session::new(
            SessionInfo::new_for_test().with_id(0),
            Arc::new(TestCommandExecutor::default()),
        ));
        let session_id = session.id();

        let history_handle = History::handle(&app);
        history_handle.update(&mut app, |history, ctx| {
            history.init_session_with(session.clone(), async { vec![] }, ctx);
        });
        assert_eventually!(
            history_handle.read(&app, |history, _ctx| history.is_queryable(&session_id)),
            "history should become queryable once the (empty) histfile read completes"
        );

        let start_ts = Local::now();
        history_handle.update(&mut app, |history, _ctx| {
            let mut entry = HistoryEntry::command_only("deploy prod".to_owned());
            entry.session_id = Some(session_id);
            entry.start_ts = Some(start_ts);
            history.append_commands(session_id, vec![entry]);
        });

        let mixer = app.add_model(|_| CommandSearchMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                history_data_source_for_session(session_id, None),
                HashSet::from([QueryFilter::History]),
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: true,
                },
                ctx,
            );
            mixer.run_query("deploy prod".into(), ctx);
        });
        assert_eventually!(
            app.read(|app| !mixer.as_ref(app).is_loading()),
            "the first query should finish loading"
        );
        let score_while_running = app.read(|app| mixer.as_ref(app).results()[0].score());

        // Mark the command finished, with a failure, while the data source (standing in for a
        // still-open Command Search panel) is never rebuilt.
        history_handle.update(&mut app, |history, _ctx| {
            history.mark_command_as_finished(session_id, start_ts, Local::now(), ExitCode::from(1));
        });
        mixer.update(&mut app, |mixer, ctx| {
            mixer.run_query("deploy prod".into(), ctx);
        });
        assert_eventually!(
            app.read(|app| !mixer.as_ref(app).is_loading()),
            "the second query should finish loading"
        );
        let score_after_failure = app.read(|app| mixer.as_ref(app).results()[0].score());

        assert!(
            score_after_failure < score_while_running,
            "the exit-status prior should reflect the command's completion even though the data \
             source was never rebuilt, proving it re-reads live History state per query rather \
             than a snapshot captured once when the source was created"
        );
    });
}

#[test]
fn history_data_source_reflects_live_execution_frequency() {
    let _flag = FeatureFlag::HistorySearchRankingV2.override_enabled(true);

    App::test((), |mut app| async move {
        crate::test_util::terminal::initialize_app_for_terminal_view(&mut app);

        let session = Arc::new(Session::new(
            SessionInfo::new_for_test().with_id(0),
            Arc::new(TestCommandExecutor::default()),
        ));
        let session_id = session.id();

        let history_handle = History::handle(&app);
        history_handle.update(&mut app, |history, ctx| {
            history.init_session_with(session.clone(), async { vec![] }, ctx);
        });
        assert_eventually!(
            history_handle.read(&app, |history, _ctx| history.is_queryable(&session_id)),
            "history should become queryable once the (empty) histfile read completes"
        );

        // Same timestamp for both, so only the frequency prior can separate them; same length
        // and shared "npm run " prefix so the query matches both with equal quality (the
        // differing trailing character falls outside the matched region).
        let start_ts = Local::now();
        let make_entry = |command: &str| {
            let mut entry = HistoryEntry::command_only(command.to_owned());
            entry.session_id = Some(session_id);
            entry.start_ts = Some(start_ts);
            entry
        };

        // Repeated executions of the same command dedupe to a single visible entry (see
        // `History::commands`), with the repeat count folded into its execution stats. The
        // 20-execution command is appended (and so occupies the tiebreak-losing position) before
        // the single-execution one, so a tie would rank it *below* -- only a real frequency
        // contribution can move it above.
        history_handle.update(&mut app, |history, _ctx| {
            for _ in 0..20 {
                history.append_commands(session_id, vec![make_entry("npm run flap")]);
            }
            history.append_commands(session_id, vec![make_entry("npm run flag")]);
        });

        let mixer = app.add_model(|_| CommandSearchMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                history_data_source_for_session(session_id, None),
                HashSet::from([QueryFilter::History]),
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: true,
                },
                ctx,
            );
            mixer.run_query("npm run".into(), ctx);
        });
        assert_eventually!(
            app.read(|app| !mixer.as_ref(app).is_loading()),
            "the query should finish loading"
        );

        app.read(|app| {
            let results = mixer.as_ref(app).results();
            assert_eq!(results.len(), 2);

            // Highest-ranked result is ordered last (`SearchResultOrdering::BottomUp`).
            assert!(
                matches!(
                    results.last().map(|result| result.accept_result()),
                    Some(CommandSearchItemAction::AcceptHistory(AcceptedHistoryItem {
                        command,
                        linked_workflow_data: None,
                    })) if command == "npm run flap"
                ),
                "the command run 20 times should outrank the one run once despite an \
                 unfavorable tiebreak, proving the frequency prior is wired through \
                 `history_data_source_for_session`'s live `command_execution_stats` lookup, not \
                 just the `rank.rs` formula in isolation"
            );
        });
    });
}

#[test]
fn history_data_source_reflects_current_working_directory() {
    let _flag = FeatureFlag::HistorySearchRankingV2.override_enabled(true);

    App::test((), |mut app| async move {
        crate::test_util::terminal::initialize_app_for_terminal_view(&mut app);

        let session = Arc::new(Session::new(
            SessionInfo::new_for_test().with_id(0),
            Arc::new(TestCommandExecutor::default()),
        ));
        let session_id = session.id();

        let history_handle = History::handle(&app);
        history_handle.update(&mut app, |history, ctx| {
            history.init_session_with(session.clone(), async { vec![] }, ctx);
        });
        assert_eventually!(
            history_handle.read(&app, |history, _ctx| history.is_queryable(&session_id)),
            "history should become queryable once the (empty) histfile read completes"
        );

        let current_cwd = "/home/user/project".to_owned();
        let other_cwd = "/home/user/other".to_owned();

        // Same timestamp and a single execution each, so only the cwd prior can separate them;
        // same length and shared "npm run " prefix so the query matches both with equal
        // quality (the differing trailing character falls outside the matched region).
        let start_ts = Local::now();
        let make_entry = |command: &str, pwd: &str| {
            let mut entry = HistoryEntry::command_only(command.to_owned());
            entry.session_id = Some(session_id);
            entry.start_ts = Some(start_ts);
            entry.pwd = Some(pwd.to_owned());
            entry
        };

        // The command that was run from `current_cwd` is appended *first*, so it occupies the
        // tiebreak-losing position (see `history_data_source_reflects_live_execution_frequency`
        // for why tiebreaks favor later-appended entries) -- only a real cwd-affinity
        // contribution can move it above the other command.
        history_handle.update(&mut app, |history, _ctx| {
            history.append_commands(
                session_id,
                vec![make_entry("npm run flap", current_cwd.as_str())],
            );
            history.append_commands(
                session_id,
                vec![make_entry("npm run flag", other_cwd.as_str())],
            );
        });

        let mixer = app.add_model(|_| CommandSearchMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                history_data_source_for_session(session_id, Some(current_cwd.clone())),
                HashSet::from([QueryFilter::History]),
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: true,
                },
                ctx,
            );
            mixer.run_query("npm run".into(), ctx);
        });
        assert_eventually!(
            app.read(|app| !mixer.as_ref(app).is_loading()),
            "the query should finish loading"
        );

        app.read(|app| {
            let results = mixer.as_ref(app).results();
            assert_eq!(results.len(), 2);

            // Highest-ranked result is ordered last (`SearchResultOrdering::BottomUp`).
            assert!(
                matches!(
                    results.last().map(|result| result.accept_result()),
                    Some(CommandSearchItemAction::AcceptHistory(AcceptedHistoryItem {
                        command,
                        linked_workflow_data: None,
                    })) if command == "npm run flap"
                ),
                "the command executed in the current working directory should outrank the one \
                 executed elsewhere despite an unfavorable tiebreak, proving that `current_cwd` \
                 is captured by `history_data_source_for_session` and propagated into each \
                 candidate's live `command_execution_stats` lookup, not just the `rank.rs` \
                 formula in isolation"
            );
        });
    });
}

#[test]
fn test_exact_matches_rank_above_prefix_matches() {
    let _flag = FeatureFlag::HistorySearchRankingV2.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let short_command = "git".to_owned();
        let long_command = "git checkout master".to_owned();
        let unrelated_command = "echo hello!".to_owned();

        let mixer = app.add_model(|_| CommandSearchMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            // `short_command`'s source is registered *first*, so on a raw-Skim tie (i.e. without
            // the exact-whole-line bonus) the mixer's `source_order` tiebreak alone would place
            // it *before* `long_command`, not after. Only the bonus, by giving `short_command` a
            // strictly higher score, can make it outrank `long_command` here -- so this ordering
            // can't pass by tiebreak coincidence the way registering it second would.
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

            mixer.run_query("git".into(), ctx);
        });

        Timer::after(Duration::from_millis(200)).await;

        app.read(|app| {
            let results = mixer.as_ref(app).results();

            // The view renders highest-ranked items at the bottom (last index) of the scrollable
            // panel. `short_command` is a whole-line exact match for the query while
            // `long_command` is only a substring match, so it outranks `long_command` and ends up
            // last, despite its source being registered first (an unfavorable tiebreak it can
            // only overcome by actually scoring higher).
            assert_eq!(results.len(), 2);

            assert!(matches!(
            results.first().map(|result| result.accept_result()),
            Some(CommandSearchItemAction::AcceptHistory(AcceptedHistoryItem { command: long, linked_workflow_data: None })) if long == long_command),
            "a substring-only match should rank below a whole-line exact match");

            assert!(matches!(
            results.get(1).map(|result| result.accept_result()),
            Some(CommandSearchItemAction::AcceptHistory(AcceptedHistoryItem { command: short, linked_workflow_data: None })) if short == short_command));
        });
    })
}

#[test]
fn test_blank_query_preserves_chronological_order_despite_differing_priors() {
    let _flag = FeatureFlag::HistorySearchRankingV2.override_enabled(true);

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

        assert_eventually!(
            app.read(|app| !mixer.as_ref(app).is_loading()),
            "the history query should finish loading"
        );

        app.read(|app| {
            let results = mixer.as_ref(app).results();
            assert_eq!(
                results.len(),
                2,
                "history should still populate the zero state for a blank query"
            );

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
fn test_history_score_stays_comparable_to_other_sources_raw_skim_scale() {
    let _flag = FeatureFlag::HistorySearchRankingV2.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let history_command = "npm test -- widgets".to_owned();
        let weak_match_text = "archive old logs then send email summary tonight";

        let history_raw_score =
            fuzzy_match::match_indices_case_insensitive(&history_command, "test")
                .expect("the history command should fuzzy-match \"test\"")
                .score;
        let weak_raw_score = fuzzy_match::match_indices_case_insensitive(weak_match_text, "test")
            .expect("the weak match text should fuzzy-match \"test\"")
            .score;
        assert!(
            weak_raw_score < history_raw_score,
            "fixture premise: the competitors' shared text must score lower on the raw Skim scale \
             than the history command (weak={weak_raw_score}, history={history_raw_score}), not \
             merely be discounted by field weighting"
        );

        let weak_workflow = Workflow::Command {
            name: "Unrelated maintenance task".to_owned(),
            command: weak_match_text.to_owned(),
            tags: vec![],
            description: None,
            arguments: vec![],
            source_url: None,
            author: None,
            author_url: None,
            shells: vec![],
            environment_variables: None,
        };
        let fuzzy_matched_workflow =
            FuzzyMatchWorkflowResult::try_match("test", &weak_workflow, "")
                .expect("the workflow's command should fuzzy-match \"test\"");
        let workflow_item = WorkflowSearchItem {
            identity: WorkflowIdentity::Local(Box::new(WorkflowType::Local(weak_workflow))),
            source: WorkflowSource::Local,
            fuzzy_matched_workflow,
        };

        let weak_saved_prompt = Workflow::AgentMode {
            name: "Unrelated saved prompt".to_owned(),
            query: weak_match_text.to_owned(),
            description: None,
            arguments: vec![],
        };
        let fuzzy_matched_saved_prompt =
            FuzzyMatchWorkflowResult::try_match("test", &weak_saved_prompt, "")
                .expect("the saved prompt's query should fuzzy-match \"test\"");
        let saved_prompt_item = WorkflowSearchItem {
            identity: WorkflowIdentity::Local(Box::new(WorkflowType::Local(weak_saved_prompt))),
            source: WorkflowSource::Local,
            fuzzy_matched_workflow: fuzzy_matched_saved_prompt,
        };

        let ai_prompt_item = AIQuerySearchResultItem {
            query_text: weak_match_text.to_owned(),
            start_time: Local::now(),
            output_status: AIQueryHistoryOutputStatus::Completed,
            working_directory: None,
            fuzzy_match_results: FuzzyMatchAIQueryResults::try_match("test", weak_match_text)
                .expect("the AI query text should fuzzy-match \"test\""),
        };

        let mixer = app.add_model(|_| CommandSearchMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_sync_source(
                FixedResults(vec![workflow_item, saved_prompt_item]),
                HashSet::from([QueryFilter::Workflows]),
            );
            mixer.add_sync_source(
                FixedResults(vec![ai_prompt_item]),
                HashSet::from([QueryFilter::PromptHistory]),
            );
            mixer.add_async_source(
                history_data_source(vec![HistoryEntry::command_only(history_command.clone())]),
                HashSet::from([QueryFilter::History]),
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: true,
                },
                ctx,
            );
            mixer.run_query("test".into(), ctx);
        });

        assert_eventually!(
            app.read(|app| !mixer.as_ref(app).is_loading()),
            "the query should finish loading"
        );

        app.read(|app| {
            let results = mixer.as_ref(app).results();
            assert_eq!(results.len(), 4);

            assert!(matches!(
                results.last().map(|result| result.accept_result()),
                Some(CommandSearchItemAction::AcceptHistory(AcceptedHistoryItem {
                    command,
                    ..
                })) if command == history_command
            ));
        });
    });
}

#[test]
fn test_no_query_filter_runs_all_data_sources() {
    let _flag = FeatureFlag::HistorySearchRankingV2.override_enabled(true);

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
    let _flag = FeatureFlag::HistorySearchRankingV2.override_enabled(true);

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
fn test_history_search_disabled_flag_skips_whitespace_tokenization() {
    let _flag = FeatureFlag::HistorySearchRankingV2.override_enabled(false);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let mixer = app.add_model(|_| CommandSearchMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                history_data_source(vec![HistoryEntry::command_only(
                    "cd ~/projects/history_orm".to_owned(),
                )]),
                HashSet::from([QueryFilter::History]),
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: true,
                },
                ctx,
            );
            mixer.run_query("cd hi orm".into(), ctx);
        });

        assert_eventually!(
            app.read(|app| !mixer.as_ref(app).is_loading()),
            "the query should finish loading"
        );

        app.read(|app| {
            let results = mixer.as_ref(app).results();
            assert!(
                results.is_empty(),
                "the legacy path shouldn't AND-tokenize \"cd hi orm\" against a single command"
            );
        });
    });
}

#[test]
fn test_history_search_disabled_flag_scores_raw_skim_with_no_priors() {
    let _flag = FeatureFlag::HistorySearchRankingV2.override_enabled(false);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let command = "git status".to_owned();
        let raw_score = fuzzy_match::match_indices_case_insensitive(&command, "git status")
            .expect("the command should fuzzy-match itself")
            .score;

        let mut old_entry = HistoryEntry::command_only(command.clone());
        old_entry.start_ts = Some(Local::now() - chrono::Duration::days(365));
        old_entry.exit_code = Some(warp_core::command::ExitCode::from(1));

        let mixer = app.add_model(|_| CommandSearchMixer::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                history_data_source(vec![old_entry]),
                HashSet::from([QueryFilter::History]),
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: true,
                },
                ctx,
            );
            mixer.run_query("git status".into(), ctx);
        });

        assert_eventually!(
            app.read(|app| !mixer.as_ref(app).is_loading()),
            "the query should finish loading"
        );

        app.read(|app| {
            let results = mixer.as_ref(app).results();
            assert_eq!(results.len(), 1);
            assert_eq!(
                results[0].score(),
                OrderedFloat(raw_score as f64),
                "the legacy path's score must be exactly the raw Skim score"
            );
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
