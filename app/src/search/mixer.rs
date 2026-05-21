use super::data_source::{Query, QueryResult};
use crate::debounce::debounce;
use crate::search::QueryFilter;
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::TelemetryEvent;
use async_channel::Sender;
use async_trait::async_trait;
use futures_util::stream::AbortHandle;
use itertools::Itertools;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use warpui::r#async::Timer;
use warpui::{Action, AppContext, Entity, ModelContext};

/// Maximum time to wait for matching data sources to return results before showing
/// partial results.
///
/// This is a UX tradeoff: waiting briefly reduces flicker in UIs that mix sync and async
/// sources (e.g. command palette file search), but we still want to show something quickly
/// if an async source is slow.
const INITIAL_RESULTS_TIMEOUT: Duration = Duration::from_millis(500);

#[cfg(not(target_family = "wasm"))]
pub(crate) type BoxFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

#[cfg(target_family = "wasm")]
pub(crate) type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + 'a>>;

#[derive(Debug, Clone, Default)]
pub enum DedupeStrategy {
    #[default]
    AllowDuplicates,
    HighestScore,
}

/// Deduplicate the results list based on provided keys, if any, and keep the highest score,
/// while preserving the original order of the kept items.
pub fn dedupe_score<T: Action + Clone>(original: Vec<QueryResult<T>>) -> Vec<QueryResult<T>> {
    let mut deduped_results: Vec<(Option<String>, &QueryResult<T>)> = Vec::new();

    for result in original.iter() {
        let mut needs_insert = true;
        let new_key = result.dedup_key();

        if new_key.is_some() {
            for (existing_key, existing_result) in deduped_results.iter_mut() {
                // Note: at this point, new_key must be Some(str)
                if new_key == *existing_key {
                    // This does not need to be inserted - either replace or discard
                    needs_insert = false;
                    if result.score() > existing_result.score() {
                        *existing_result = result;
                    }
                    break;
                }
            }
        }

        if needs_insert {
            deduped_results.push((new_key, result));
        }
    }

    deduped_results
        .into_iter()
        .map(|(_, r)| r.clone())
        .collect()
}

/// A structure that combines results from various data sources to produce a
/// single, ordered, heterogeneous list of search results.
#[derive(Default)]
pub struct SearchMixer<T: Action + Clone> {
    /// The set of sources to be used to run a query against.
    sources: HashMap<DataSourceId, RegisteredDataSource<T>>,

    /// The latest set of search results produced by the latest `query`.
    results: Vec<QueryResult<T>>,

    /// The latest query that was used to search against, if any.
    query: Option<Query>,

    /// The set of sources that have finished running for the latest query.
    finished_sources: HashSet<DataSourceId>,

    /// The strategy for deduplication
    dedupe_strategy: DedupeStrategy,

    /// Monotonically increasing counter incremented on each `run_query`. Used to discard stale
    /// async callbacks and timeout callbacks whose futures completed before the abort took effect.
    query_generation: u64,

    /// Results buffered for the current query that haven't been committed to results yet.
    /// `Some(vec)` means we're actively buffering (old results remain visible).
    /// `None` means results have been committed; late-arriving results go directly to `results`.
    pending_results: Option<Vec<QueryResult<T>>>,

    /// Tracks whether the current query has emitted its initial set of visible results yet.
    initial_results_emitted: bool,
}

impl<T: Action + Clone> Entity for SearchMixer<T> {
    type Event = SearchMixerEvent;
}

/// A unique identifier for a DataSource.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
struct DataSourceId(usize);
impl DataSourceId {
    /// Constructs a new globally-unique entity ID.
    #[allow(clippy::new_without_default)]
    pub fn new() -> DataSourceId {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let raw = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        DataSourceId(raw)
    }
}

pub enum SearchMixerEvent {
    ResultsChanged,
}

pub struct AddAsyncSourceOptions {
    pub debounce_interval: Option<Duration>,
    /// Whether to run this source when the query text is empty
    /// (i.e. the user hasn't typed anything yet).
    pub run_in_zero_state: bool,
    pub run_when_unfiltered: bool,
}

impl<T: Action + Clone> SearchMixer<T> {
    pub fn new() -> Self {
        Self {
            sources: HashMap::new(),
            finished_sources: HashSet::new(),
            results: vec![],
            query: None,
            dedupe_strategy: DedupeStrategy::AllowDuplicates,
            query_generation: 0,
            pending_results: None,
            initial_results_emitted: false,
        }
    }

    /// Set the deduplication strategy for the mixer
    pub fn set_dedupe_strategy(&mut self, strategy: DedupeStrategy) {
        self.dedupe_strategy = strategy;
    }

    /// Resets the mixer's state.
    pub fn reset(&mut self, ctx: &mut ModelContext<Self>) {
        self.abort_in_flight_async_queries();
        self.query_generation = self.query_generation.wrapping_add(1);

        self.sources.clear();
        self.finished_sources.clear();
        self.results.clear();
        self.pending_results = None;
        self.query.take();
        self.initial_results_emitted = false;
        ctx.emit(SearchMixerEvent::ResultsChanged);
    }

    /// Abort the current in-flight query to avoid stale searches
    /// continuing and passing back results when they are no longer wanted.
    fn abort_in_flight_async_queries(&mut self) {
        for registered_source in self.sources.values_mut() {
            if let DataSource::AsyncDataSource {
                latest_run_abort_handle,
                ..
            } = &mut registered_source.source
            {
                if let Some(abort_handle) = latest_run_abort_handle.take() {
                    abort_handle.abort();
                }
            }
        }
    }

    /// Resets the mixer's results. Use the all-encompassing [`reset`] API
    /// to clear _all_ of the mixer's state.
    pub fn reset_results(&mut self, ctx: &mut ModelContext<Self>) {
        self.abort_in_flight_async_queries();
        self.query_generation = self.query_generation.wrapping_add(1);

        self.results.clear();
        self.pending_results = None;
        self.query.take();
        self.initial_results_emitted = false;
        ctx.emit(SearchMixerEvent::ResultsChanged);
    }

    /// Adds a [`SyncDataSource`] to produce results when the mixer is queried. Query results will
    /// be produced from this source if there are no filters provided or if one of the filters
    /// within a [`Query`] is equal to this filter.
    pub fn add_sync_source(
        &mut self,
        source: impl SyncDataSource<Action = T>,
        filters: impl Into<HashSet<QueryFilter>>,
    ) {
        self.sources.insert(
            DataSourceId::new(),
            RegisteredDataSource::new(
                DataSource::SyncDataSource {
                    source: Arc::new(source),
                },
                filters.into(),
            ),
        );
    }

    /// Adds an [`AsyncDataSource`] to produce results when the mixer is queried.
    /// The results will be produced asynchronously and the mixer will notify its
    /// subscribers whenever the result set changes.
    ///
    /// A debounce interval can be provided to only query the data source in a debounced fashion.
    ///
    /// By default, async sources only run when the query's filters explicitly match. Set
    /// `run_when_unfiltered` to `true` so the source also runs when `query.filters` is empty.
    /// Only enable this when the source's work is cheap (e.g. local fuzzy matching) — expensive
    /// operations like network requests should not run on every unfiltered keystroke.
    pub fn add_async_source(
        &mut self,
        source: impl AsyncDataSource<Action = T>,
        filters: impl Into<HashSet<QueryFilter>>,
        options: AddAsyncSourceOptions,
        ctx: &mut ModelContext<Self>,
    ) {
        let source = Arc::new(source);
        let data_source_id = DataSourceId::new();
        let debounce_tx = options.debounce_interval.map(|interval| {
            self.start_debounce_stream_for_data_source(data_source_id, interval, ctx)
        });

        self.sources.insert(
            data_source_id,
            RegisteredDataSource::new(
                DataSource::AsyncDataSource {
                    source,
                    debounce_tx,
                    latest_run_abort_handle: None,
                    run_in_zero_state: options.run_in_zero_state,
                    run_when_unfiltered: options.run_when_unfiltered,
                },
                filters.into(),
            ),
        );
    }

    pub fn current_query(&self) -> Option<&Query> {
        self.query.as_ref()
    }

    /// Runs a query against the registered data sources using the provided Query configuration.
    /// On completion, the mixer emits an event to subscribers to indicate the result set has changed.
    ///
    /// Old results remain visible while new results are buffered. The visible result set is
    /// replaced atomically once all sources finish, or after [`INITIAL_RESULTS_TIMEOUT`] elapses.
    /// Late-arriving async results are appended without reordering existing results.
    pub fn run_query(&mut self, query: Query, ctx: &mut ModelContext<Self>) {
        self.pending_results = Some(Vec::new());
        self.finished_sources.clear();
        self.query = Some(query.clone());
        self.query_generation = self.query_generation.wrapping_add(1);
        self.initial_results_emitted = false;
        let query = &query;

        // We want to run the queries in the order that the data sources were added.
        let data_source_ids_to_run = self.ordered_data_source_ids_for_query(query).collect_vec();
        for id in data_source_ids_to_run {
            self.run_query_internal(id, false, ctx);
        }

        // Sync sources (and skipped async sources) will have already finished
        // inside the loop. If everything is done, commit immediately.
        if self.pending_results.is_some() {
            if !self.is_loading() {
                self.commit_pending_results_for_current_query(ctx);
            } else {
                let query_generation = self.query_generation;
                let _ = ctx.spawn(
                    async move { Timer::after(INITIAL_RESULTS_TIMEOUT).await },
                    move |mixer, _, ctx| {
                        mixer.commit_pending_results_after_timeout(query_generation, ctx);
                    },
                );
            }
        }
    }

    pub fn results(&self) -> &Vec<QueryResult<T>> {
        &self.results
    }

    pub fn are_results_empty(&self) -> bool {
        self.results.is_empty()
    }

    /// Returns all the filters that are currently registered.
    pub fn registered_filters(&self) -> impl Iterator<Item = QueryFilter> + '_ {
        self.sources
            .values()
            .flat_map(|source| source.filters.clone())
    }

    /// Returns the query filter for the first data source that hasn't completed.
    pub fn loading_query_filters(&self) -> Option<HashSet<QueryFilter>> {
        if self.initial_results_emitted {
            return None;
        }
        let query = self.query.as_ref()?;
        self.ordered_data_source_ids_for_query(query)
            .find(|id| !self.finished_sources.contains(id))
            .and_then(|id| self.sources.get(&id))
            .map(|data_source| data_source.filters.clone())
    }

    /// Returns true iff there is at least one loading data source.
    /// Helper that computes over `loading_query_filter`.
    pub fn is_loading(&self) -> bool {
        self.loading_query_filters().is_some()
    }

    /// Returns the first error found from running the data sources against the query, if any.
    pub fn first_data_source_error(
        &self,
    ) -> Option<(HashSet<QueryFilter>, &DataSourceRunErrorWrapper)> {
        let query = self.query.as_ref()?;
        self.ordered_data_source_ids_for_query(query)
            .find_map(|id| {
                self.sources
                    .get(&id)
                    .and_then(|s| Some(s.filters.clone()).zip(s.latest_run_error.as_ref()))
            })
    }

    /// Returns an ordered list of data source IDs in the order that the corresponding
    /// data sources were registered in.
    /// We could use a map that respects insertion order but that will likely be
    // overkill since the number of data sources is usually minute.
    fn ordered_data_source_ids_for_query<'a>(
        &'a self,
        query: &'a Query,
    ) -> impl Iterator<Item = DataSourceId> + 'a {
        self.sources
            .keys()
            .sorted()
            .filter(|id| {
                self.sources
                    .get(id)
                    .is_some_and(|registered_source| registered_source.matches_query(query))
            })
            .copied()
    }

    /// Runs the query for the [`DataSource`] identified by the provided `data_source_id`.
    /// If `skip_debounce` is true, then the query is started immediately even if queries
    /// against the data source are meant to be debounced.
    fn run_query_internal(
        &mut self,
        data_source_id: DataSourceId,
        skip_debounce: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(registered_source) = self.sources.get_mut(&data_source_id) else {
            return;
        };
        let Some(query) = self.query.clone() else {
            return;
        };

        // Clear the latest run error, if any, because we're about to run a new query.
        registered_source.latest_run_error = None;

        match &mut registered_source.source {
            DataSource::SyncDataSource { source } => {
                let new_results = source.run_query(&query, ctx);
                self.add_new_results(data_source_id, new_results, ctx);
            }
            DataSource::AsyncDataSource {
                source,
                debounce_tx,
                latest_run_abort_handle,
                run_in_zero_state,
                run_when_unfiltered: _,
            } => {
                // Abort any existing run before starting a new one.
                // This is necessary to do even if we end up debouncing
                // because there might already be a running query that's taking long.
                if let Some(abort_handle) = latest_run_abort_handle.take() {
                    abort_handle.abort();
                }

                // Only run async sources in the zero state if the async source indicated it should run in the
                // zero state when registered. It can be costly to run async sources on blank queries so we don't
                // do this by default.
                if query.text.is_empty() && !*run_in_zero_state {
                    self.mark_source_as_finished(data_source_id);
                    if self.pending_results.is_some() && !self.is_loading() {
                        self.commit_pending_results_for_current_query(ctx);
                    }
                    return;
                }

                // Check if we should just be debouncing the query rather than running it right now.
                if let Some(debounce_tx) = debounce_tx {
                    if !skip_debounce {
                        let _ = debounce_tx.try_send(DataSourceDebounceArg {});
                        return;
                    }
                }

                // If we get here, then we should run the query against the data source right now.
                let query_generation = self.query_generation;
                let source = source.clone();
                let filters = registered_source.filters.to_owned();
                let new_abort_handle = ctx.spawn(
                    source.run_query(&query, ctx),
                    move |mixer, new_results, ctx| {
                        // Discard results from a previous query whose future completed before
                        // the abort took effect.
                        if mixer.query_generation != query_generation {
                            source.on_query_finished(ctx);
                            return;
                        }
                        let error_payload =
                            new_results.as_ref().err().map(|e| e.telemetry_payload());
                        send_telemetry_from_ctx!(
                            TelemetryEvent::CommandSearchAsyncQueryCompleted {
                                filters,
                                error_payload,
                            },
                            ctx
                        );
                        mixer.add_new_results(data_source_id, new_results, ctx);
                        source.on_query_finished(ctx);
                    },
                );
                *latest_run_abort_handle = Some(new_abort_handle.abort_handle());
            }
        }
    }

    fn add_new_results(
        &mut self,
        data_source_id: DataSourceId,
        new_results: Result<Vec<QueryResult<T>>, DataSourceRunErrorWrapper>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.finished_sources.contains(&data_source_id) {
            log::warn!(
                "Ignoring duplicate results for source {data_source_id:?} that was already marked finished"
            );
            return;
        }
        self.mark_source_as_finished(data_source_id);

        match new_results {
            Ok(results) => {
                let results_with_order = results
                    .into_iter()
                    .map(|mut result| {
                        result.source_order = data_source_id.0;
                        result
                    })
                    .collect_vec();

                if let Some(pending) = &mut self.pending_results {
                    pending.extend(results_with_order);
                    if !self.is_loading() {
                        self.commit_pending_results_for_current_query(ctx);
                    }
                } else if self.initial_results_emitted {
                    let mut late_results = results_with_order;
                    late_results.sort_by_key(|r| (r.priority_tier(), r.score(), r.source_order));

                    self.results.extend(late_results);

                    if matches!(self.dedupe_strategy, DedupeStrategy::HighestScore) {
                        self.results = dedupe_score(std::mem::take(&mut self.results));
                    }

                    ctx.emit(SearchMixerEvent::ResultsChanged);
                } else {
                    self.results.extend(results_with_order);
                    self.sort_and_dedupe_results();
                    ctx.emit(SearchMixerEvent::ResultsChanged);
                }
            }
            Err(e) => {
                if let Some(source) = self.sources.get_mut(&data_source_id) {
                    source.latest_run_error = Some(e);
                }

                if self.pending_results.is_some() && !self.is_loading() {
                    self.commit_pending_results_for_current_query(ctx);
                } else if self.pending_results.is_none() {
                    ctx.emit(SearchMixerEvent::ResultsChanged);
                }
            }
        }
    }

    /// Commits buffered results from the current query, replacing the visible result set.
    /// After this, any late-arriving results are added directly to `results`.
    fn commit_pending_results(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(pending) = self.pending_results.take() else {
            return;
        };
        self.results = pending;
        self.sort_and_dedupe_results();
        ctx.emit(SearchMixerEvent::ResultsChanged);
    }

    fn commit_pending_results_for_current_query(&mut self, ctx: &mut ModelContext<Self>) {
        self.initial_results_emitted = true;
        self.commit_pending_results(ctx);
    }

    /// Sort by (priority_tier, score, source_order) so that equal-scored results
    /// from earlier-registered sources appear first, regardless of async completion order.
    fn sort_and_dedupe_results(&mut self) {
        self.results
            .sort_by_key(|r| (r.priority_tier(), r.score(), r.source_order));
        if matches!(self.dedupe_strategy, DedupeStrategy::HighestScore) {
            self.results = dedupe_score(std::mem::take(&mut self.results));
        }
    }

    fn mark_source_as_finished(&mut self, data_source_id: DataSourceId) {
        self.finished_sources.insert(data_source_id);
    }

    fn commit_pending_results_after_timeout(
        &mut self,
        query_generation: u64,
        ctx: &mut ModelContext<Self>,
    ) {
        if query_generation != self.query_generation || self.pending_results.is_none() {
            return;
        }
        self.commit_pending_results_for_current_query(ctx);
    }

    fn start_debounce_stream_for_data_source(
        &mut self,
        data_source_id: DataSourceId,
        interval: Duration,
        ctx: &mut ModelContext<Self>,
    ) -> Sender<DataSourceDebounceArg> {
        let (debounce_tx, debounce_rx) = async_channel::unbounded();
        let _ = ctx.spawn_stream_local(
            debounce(interval, debounce_rx),
            move |mixer, _, ctx| {
                mixer.run_query_internal(data_source_id, true, ctx);
            },
            |_, _| {},
        );
        debounce_tx
    }
}

/// A trait representing a set of data that can be queried for search results synchronously.
pub trait SyncDataSource: 'static {
    /// The action that is dispatched when a result produced by this data source is
    /// accepted.
    type Action: Action + Clone;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper>;
}

/// A trait representing a set of data that can be queried for search results asynchronously.
#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
pub trait AsyncDataSource: 'static + Send + Sync {
    /// The action that is dispatched when a result produced by this data source is
    /// accepted.
    type Action: Action + Clone;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> BoxFuture<'static, Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper>>;

    /// Function that should be run in the callback after `run_query` finishes.
    fn on_query_finished(&self, _ctx: &mut AppContext) {}
}

/// Helper type alias for a DataSourceRunError.
pub type DataSourceRunErrorWrapper = Box<dyn DataSourceRunError>;

pub trait DataSourceRunError: 'static + Send + Sync + std::fmt::Debug {
    fn user_facing_error(&self) -> String;
    fn telemetry_payload(&self) -> serde_json::Value;
    fn as_any(&self) -> &dyn Any;
}

struct DataSourceDebounceArg {}

enum DataSource<T: Action + Clone> {
    SyncDataSource {
        source: Arc<dyn SyncDataSource<Action = T>>,
    },
    AsyncDataSource {
        latest_run_abort_handle: Option<AbortHandle>,
        source: Arc<dyn AsyncDataSource<Action = T>>,
        debounce_tx: Option<Sender<DataSourceDebounceArg>>,
        run_in_zero_state: bool,
        run_when_unfiltered: bool,
    },
}

/// A registered [`DataSource`] for a [`SearchMixer`].
struct RegisteredDataSource<T: Action + Clone> {
    source: DataSource<T>,

    /// Corresponding filter for this data source.
    filters: HashSet<QueryFilter>,

    /// The error produced by this data source during its last run.
    latest_run_error: Option<DataSourceRunErrorWrapper>,
}

impl<T: Action + Clone> RegisteredDataSource<T> {
    /// Sync sources always run when the query has no filters. Async sources only run on
    /// unfiltered queries when `run_when_unfiltered` is set, to avoid running expensive
    /// operations (e.g. network requests) on every keystroke.
    fn matches_query(&self, query: &Query) -> bool {
        match &self.source {
            DataSource::SyncDataSource { .. } => {
                query.filters.is_empty() || query.filters.intersection(&self.filters).count() > 0
            }
            DataSource::AsyncDataSource {
                run_when_unfiltered,
                ..
            } => {
                (*run_when_unfiltered && query.filters.is_empty())
                    || query.filters.intersection(&self.filters).count() > 0
            }
        }
    }
}

impl<T: Action + Clone> RegisteredDataSource<T> {
    fn new(source: DataSource<T>, filters: HashSet<QueryFilter>) -> Self {
        Self {
            source,
            filters,
            latest_run_error: None,
        }
    }
}

#[cfg(test)]
#[path = "mixer_tests.rs"]
mod mixer_test;
