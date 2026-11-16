use std::collections::HashSet;
use std::time::Duration;

use ordered_float::OrderedFloat;
use warp_core::telemetry::testing::MockTelemetryContextProvider;
use warpui_core::r#async::Timer;
use warpui_core::{App, AppContext, Element};

use super::*;
use crate::item::SearchItem;

#[derive(Clone, Debug, PartialEq)]
struct TestAction {
    id: String,
}

#[derive(Debug)]
struct TestSearchItem {
    id: String,
    priority_tier: u8,
    score: f64,
}

impl SearchItem for TestSearchItem {
    type Action = TestAction;

    fn render_icon(
        &self,
        _highlight_state: crate::result_renderer::ItemHighlightState,
        _appearance: &warp_core::ui::appearance::Appearance,
    ) -> Box<dyn Element> {
        unimplemented!()
    }

    fn render_item(
        &self,
        _highlight_state: crate::result_renderer::ItemHighlightState,
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
        Box::pin(async move {
            Timer::after(delay).await;
            Ok(vec![QueryResult::from(TestSearchItem {
                id,
                priority_tier,
                score,
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
            })])
        })
    }
}

fn initialize_app(app: &mut App) {
    app.update(MockTelemetryContextProvider::register);
}

#[test]
fn test_results_are_sorted_by_tier_then_score() {
    let mut mixer = SearchMixer::<TestAction>::new();

    mixer.results = vec![
        QueryResult::from(TestSearchItem {
            id: "tier0_high".to_string(),
            priority_tier: 0,
            score: 100.0,
        }),
        QueryResult::from(TestSearchItem {
            id: "tier1_low".to_string(),
            priority_tier: 1,
            score: 1.0,
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
    });
    source_0.source_order = 0;

    let mut source_1 = QueryResult::from(TestSearchItem {
        id: "source_1".to_string(),
        priority_tier: 0,
        score: 10.0,
    });
    source_1.source_order = 1;

    let mut source_2 = QueryResult::from(TestSearchItem {
        id: "source_2".to_string(),
        priority_tier: 0,
        score: 10.0,
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
    });
    tier_0_high_score.source_order = 2;

    let mut tier_0_mid_score_early_source = QueryResult::from(TestSearchItem {
        id: "tier_0_score_50_source_0".to_string(),
        priority_tier: 0,
        score: 50.0,
    });
    tier_0_mid_score_early_source.source_order = 0;

    let mut tier_0_mid_score_late_source = QueryResult::from(TestSearchItem {
        id: "tier_0_score_50_source_1".to_string(),
        priority_tier: 0,
        score: 50.0,
    });
    tier_0_mid_score_late_source.source_order = 1;

    let mut tier_1_highest_score = QueryResult::from(TestSearchItem {
        id: "tier_1_score_999_source_0".to_string(),
        priority_tier: 1,
        score: 999.0,
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
                        score: 100.0,
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

        // When the async source finishes later, its results are placed at the low-priority edge
        // without reordering the already-visible sync results.
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
                vec!["late_async", "sync"]
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
