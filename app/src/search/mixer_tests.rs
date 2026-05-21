use super::*;
use crate::auth::auth_manager::AuthManager;
use crate::auth::AuthStateProvider;
use crate::search::item::SearchItem;
use crate::server::server_api::ServerApiProvider;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use ordered_float::OrderedFloat;
use std::collections::HashSet;
use std::time::Duration;
use warpui::r#async::Timer;
use warpui::{App, AppContext, Element};

#[derive(Clone, Debug, PartialEq)]
struct TestAction {
    id: String,
}

#[derive(Debug)]
struct TestSearchItem {
    id: String,
    priority_tier: u8,
    score: f64,
    dedup_key: Option<String>,
}

impl SearchItem for TestSearchItem {
    type Action = TestAction;

    fn render_icon(
        &self,
        _highlight_state: crate::search::result_renderer::ItemHighlightState,
        _appearance: &crate::appearance::Appearance,
    ) -> Box<dyn Element> {
        unimplemented!()
    }

    fn render_item(
        &self,
        _highlight_state: crate::search::result_renderer::ItemHighlightState,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        unimplemented!()
    }

    fn priority_tier(&self) -> u8 {
        self.priority_tier
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.score)
    }

    fn accept_result(&self) -> Self::Action {
        TestAction {
            id: self.id.clone(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        TestAction {
            id: self.id.clone(),
        }
    }

    fn accessibility_label(&self) -> String {
        self.id.clone()
    }

    fn dedup_key(&self) -> Option<String> {
        self.dedup_key.clone()
    }
}

struct StaticSyncSource {
    result: TestSearchItem,
}

impl SyncDataSource for StaticSyncSource {
    type Action = TestAction;

    fn run_query(
        &self,
        _: &Query,
        _: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        Ok(vec![QueryResult::from(TestSearchItem {
            id: self.result.id.clone(),
            priority_tier: self.result.priority_tier,
            score: self.result.score,
            dedup_key: self.result.dedup_key.clone(),
        })])
    }
}

struct DelayedAsyncSource {
    delay: Duration,
    result: TestSearchItem,
}

impl AsyncDataSource for DelayedAsyncSource {
    type Action = TestAction;

    fn run_query(
        &self,
        _: &Query,
        _: &AppContext,
    ) -> BoxFuture<'static, Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper>> {
        let delay = self.delay;
        let id = self.result.id.clone();
        let priority_tier = self.result.priority_tier;
        let score = self.result.score;
        let dedup_key = self.result.dedup_key.clone();
        Box::pin(async move {
            Timer::after(delay).await;
            Ok(vec![QueryResult::from(TestSearchItem {
                id,
                priority_tier,
                score,
                dedup_key,
            })])
        })
    }
}

struct QueryDrivenDelayedAsyncSource;

impl AsyncDataSource for QueryDrivenDelayedAsyncSource {
    type Action = TestAction;

    fn run_query(
        &self,
        query: &Query,
        _: &AppContext,
    ) -> BoxFuture<'static, Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper>> {
        let (delay, id) = match query.text.as_str() {
            "first" => (Duration::from_millis(200), "stale_first".to_string()),
            "second" => (Duration::from_millis(300), "fresh_second".to_string()),
            text => (Duration::from_millis(50), text.to_string()),
        };
        Box::pin(async move {
            Timer::after(delay).await;
            Ok(vec![QueryResult::from(TestSearchItem {
                id,
                priority_tier: 0,
                score: 0.0,
                dedup_key: None,
            })])
        })
    }
}

fn initialize_app(app: &mut App) {
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(AuthManager::new_for_test);
}

#[test]
fn test_dedupe_on_keeps_highest_score() {
    // Add items with same dedup key but different scores
    let results = vec![
        QueryResult::from(TestSearchItem {
            id: "item1".to_string(),
            priority_tier: 0,
            score: 1.0,
            dedup_key: Some("key1".to_string()),
        }),
        QueryResult::from(TestSearchItem {
            id: "item2".to_string(),
            priority_tier: 0,
            score: 3.0,
            dedup_key: Some("key1".to_string()),
        }),
        QueryResult::from(TestSearchItem {
            id: "item3".to_string(),
            priority_tier: 0,
            score: 2.0,
            dedup_key: Some("key1".to_string()),
        }),
    ];

    let results = dedupe_score(results);
    // Should keep only the item with highest score (item2 with score 3.0)
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].accept_result().id, "item2");
}

#[test]
fn test_dedupe_on_preserves_items_without_keys() {
    // Add items with and without dedup keys
    let results = vec![
        QueryResult::from(TestSearchItem {
            id: "item1".to_string(),
            priority_tier: 0,
            score: 1.0,
            dedup_key: None,
        }),
        QueryResult::from(TestSearchItem {
            id: "item2".to_string(),
            priority_tier: 0,
            score: 2.0,
            dedup_key: None,
        }),
        QueryResult::from(TestSearchItem {
            id: "item3".to_string(),
            priority_tier: 0,
            score: 3.0,
            dedup_key: Some("key1".to_string()),
        }),
    ];

    let results = dedupe_score(results);
    // Should keep all items without dedup keys
    assert_eq!(results.len(), 3);
}

#[test]
fn test_results_are_sorted_by_tier_then_score() {
    let mut mixer = SearchMixer::<TestAction>::new();

    mixer.results = vec![
        QueryResult::from(TestSearchItem {
            id: "tier0_high".to_string(),
            priority_tier: 0,
            score: 100.0,
            dedup_key: None,
        }),
        QueryResult::from(TestSearchItem {
            id: "tier1_low".to_string(),
            priority_tier: 1,
            score: 1.0,
            dedup_key: None,
        }),
    ];

    mixer
        .results
        .sort_by_key(|r| (r.priority_tier(), r.score()));

    let ordered = mixer.results();
    assert_eq!(ordered[0].accept_result().id, "tier0_high");
    assert_eq!(ordered[1].accept_result().id, "tier1_low");
}

#[test]
fn test_results_with_equal_tier_and_score_use_source_order_as_tiebreaker() {
    let mut mixer = SearchMixer::<TestAction>::new();

    let mut source_0 = QueryResult::from(TestSearchItem {
        id: "source_0".to_string(),
        priority_tier: 0,
        score: 10.0,
        dedup_key: None,
    });
    source_0.source_order = 0;

    let mut source_1 = QueryResult::from(TestSearchItem {
        id: "source_1".to_string(),
        priority_tier: 0,
        score: 10.0,
        dedup_key: None,
    });
    source_1.source_order = 1;

    let mut source_2 = QueryResult::from(TestSearchItem {
        id: "source_2".to_string(),
        priority_tier: 0,
        score: 10.0,
        dedup_key: None,
    });
    source_2.source_order = 2;

    mixer.results = vec![source_2, source_1, source_0];
    mixer
        .results
        .sort_by_key(|r| (r.priority_tier(), r.score(), r.source_order));

    let ordered = mixer.results();
    assert_eq!(ordered[0].accept_result().id, "source_0");
    assert_eq!(ordered[1].accept_result().id, "source_1");
    assert_eq!(ordered[2].accept_result().id, "source_2");
}

#[test]
fn test_results_with_mixed_tiers_scores_and_sources_sort_consistently() {
    let mut mixer = SearchMixer::<TestAction>::new();

    let mut tier_0_high_score = QueryResult::from(TestSearchItem {
        id: "tier_0_score_100_source_2".to_string(),
        priority_tier: 0,
        score: 100.0,
        dedup_key: None,
    });
    tier_0_high_score.source_order = 2;

    let mut tier_0_mid_score_early_source = QueryResult::from(TestSearchItem {
        id: "tier_0_score_50_source_0".to_string(),
        priority_tier: 0,
        score: 50.0,
        dedup_key: None,
    });
    tier_0_mid_score_early_source.source_order = 0;

    let mut tier_0_mid_score_late_source = QueryResult::from(TestSearchItem {
        id: "tier_0_score_50_source_1".to_string(),
        priority_tier: 0,
        score: 50.0,
        dedup_key: None,
    });
    tier_0_mid_score_late_source.source_order = 1;

    let mut tier_1_highest_score = QueryResult::from(TestSearchItem {
        id: "tier_1_score_999_source_0".to_string(),
        priority_tier: 1,
        score: 999.0,
        dedup_key: None,
    });
    tier_1_highest_score.source_order = 0;

    mixer.results = vec![
        tier_1_highest_score,
        tier_0_high_score,
        tier_0_mid_score_late_source,
        tier_0_mid_score_early_source,
    ];
    mixer
        .results
        .sort_by_key(|r| (r.priority_tier(), r.score(), r.source_order));

    let ordered = mixer.results();
    assert_eq!(ordered[0].accept_result().id, "tier_0_score_50_source_0");
    assert_eq!(ordered[1].accept_result().id, "tier_0_score_50_source_1");
    assert_eq!(ordered[2].accept_result().id, "tier_0_score_100_source_2");
    assert_eq!(ordered[3].accept_result().id, "tier_1_score_999_source_0");
}

#[test]
fn test_initial_results_timeout_and_appends_late_async_results_without_reordering() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let mixer = app.add_model(|_| SearchMixer::<TestAction>::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_sync_source(
                StaticSyncSource {
                    result: TestSearchItem {
                        id: "sync".to_string(),
                        priority_tier: 0,
                        score: 10.0,
                        dedup_key: None,
                    },
                },
                [QueryFilter::Actions],
            );
            mixer.add_async_source(
                DelayedAsyncSource {
                    delay: Duration::from_millis(700),
                    result: TestSearchItem {
                        id: "late_async".to_string(),
                        priority_tier: 0,
                        score: 0.0,
                        dedup_key: None,
                    },
                },
                [QueryFilter::Actions],
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: false,
                },
                ctx,
            );
            mixer.run_query(
                Query {
                    text: "a".to_string(),
                    filters: HashSet::from([QueryFilter::Actions]),
                },
                ctx,
            );
        });

        app.read(|app| {
            let mixer = mixer.as_ref(app);
            assert!(mixer.is_loading());
            assert!(!mixer.initial_results_emitted);
            assert_eq!(
                mixer
                    .results()
                    .iter()
                    .map(|result| result.accept_result().id)
                    .collect::<Vec<_>>(),
                Vec::<&str>::new()
            );
        });

        // After the initial timeout, we should show partial results (sync), without waiting for
        // the slow async source to complete.
        Timer::after(Duration::from_millis(600)).await;

        app.read(|app| {
            let mixer = mixer.as_ref(app);
            assert!(!mixer.is_loading());
            assert!(mixer.initial_results_emitted);
            assert_eq!(
                mixer
                    .results()
                    .iter()
                    .map(|result| result.accept_result().id)
                    .collect::<Vec<_>>(),
                vec!["sync"]
            );
        });

        // When the async source finishes later, its results are appended to the end without
        // reordering the already-visible sync results.
        Timer::after(Duration::from_millis(200)).await;

        app.read(|app| {
            let mixer = mixer.as_ref(app);
            assert!(!mixer.is_loading());
            assert_eq!(
                mixer
                    .results()
                    .iter()
                    .map(|result| result.accept_result().id)
                    .collect::<Vec<_>>(),
                vec!["sync", "late_async"]
            );
        });
    });
}

#[test]
fn test_initial_results_commit_keeps_sorted_results_when_async_finishes_before_timeout() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let mixer = app.add_model(|_| SearchMixer::<TestAction>::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_sync_source(
                StaticSyncSource {
                    result: TestSearchItem {
                        id: "sync".to_string(),
                        priority_tier: 0,
                        score: 10.0,
                        dedup_key: None,
                    },
                },
                [QueryFilter::Actions],
            );
            mixer.add_async_source(
                DelayedAsyncSource {
                    delay: Duration::from_millis(50),
                    result: TestSearchItem {
                        id: "fast_async".to_string(),
                        priority_tier: 0,
                        score: 0.0,
                        dedup_key: None,
                    },
                },
                [QueryFilter::Actions],
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: false,
                },
                ctx,
            );
            mixer.run_query(
                Query {
                    text: "a".to_string(),
                    filters: HashSet::from([QueryFilter::Actions]),
                },
                ctx,
            );
        });

        Timer::after(Duration::from_millis(600)).await;

        app.read(|app| {
            let mixer = mixer.as_ref(app);
            assert!(!mixer.is_loading());
            assert!(mixer.initial_results_emitted);
            assert_eq!(
                mixer
                    .results()
                    .iter()
                    .map(|result| result.accept_result().id)
                    .collect::<Vec<_>>(),
                vec!["fast_async", "sync"]
            );
        });
    });
}

#[test]
fn test_stale_async_results_do_not_poison_newer_query() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let mixer = app.add_model(|_| SearchMixer::<TestAction>::new());
        mixer.update(&mut app, |mixer, ctx| {
            mixer.add_async_source(
                QueryDrivenDelayedAsyncSource,
                [QueryFilter::Actions],
                AddAsyncSourceOptions {
                    debounce_interval: None,
                    run_in_zero_state: false,
                    run_when_unfiltered: false,
                },
                ctx,
            );
            mixer.run_query(
                Query {
                    text: "first".to_string(),
                    filters: HashSet::from([QueryFilter::Actions]),
                },
                ctx,
            );
        });

        Timer::after(Duration::from_millis(50)).await;

        mixer.update(&mut app, |mixer, ctx| {
            mixer.run_query(
                Query {
                    text: "second".to_string(),
                    filters: HashSet::from([QueryFilter::Actions]),
                },
                ctx,
            );
        });

        Timer::after(Duration::from_millis(400)).await;

        app.read(|app| {
            let mixer = mixer.as_ref(app);
            assert_eq!(
                mixer
                    .results()
                    .iter()
                    .map(|result| result.accept_result().id)
                    .collect::<Vec<_>>(),
                vec!["fresh_second"]
            );
        });
    });
}
