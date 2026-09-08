use std::sync::Arc;

use warpui::{Action, AppContext};

use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::{AsyncDataSource, BoxFuture, DataSourceRunErrorWrapper};

type SnapshotFn<S> = dyn Fn(&Query, &AppContext) -> S + Send + Sync;

type MatchFn<S, A> = dyn Fn(S) -> BoxFuture<'static, Result<Vec<QueryResult<A>>, DataSourceRunErrorWrapper>>
    + Send
    + Sync;

/// This is a basic wrapper on top of the AsyncDataSource that separates sourcing into two steps:
/// 1. `snapshot_fn` — this is called on the main thread to capture an owned snapshot of the data
///    needed for matching (we need app context to do this, so it has to be synchronous).
///    Avoid deep-cloning large datasets here and prefer cloning shared handles (e.g. `Arc` collections)
///    captured by the data source.
/// 2. `match_fn` — called async with the snapshot to perform the expensive fuzzy matching and produce query results.
///
/// This split lets data sources that depend on `AppContext` (e.g. reading model state) run
/// their expensive work without blocking the UI.
pub struct AsyncSnapshotDataSource<S, A>
where
    S: Send + 'static,
    A: Action + Clone,
{
    snapshot_fn: Arc<SnapshotFn<S>>,
    match_fn: Arc<MatchFn<S, A>>,
}

impl<S, A> AsyncSnapshotDataSource<S, A>
where
    S: Send + 'static,
    A: Action + Clone,
{
    pub fn new(
        snapshot_fn: impl Fn(&Query, &AppContext) -> S + Send + Sync + 'static,
        match_fn: impl Fn(
            S,
        )
            -> BoxFuture<'static, Result<Vec<QueryResult<A>>, DataSourceRunErrorWrapper>>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            snapshot_fn: Arc::new(snapshot_fn),
            match_fn: Arc::new(match_fn),
        }
    }
}

impl<S, A> AsyncDataSource for AsyncSnapshotDataSource<S, A>
where
    S: Send + 'static,
    A: Action + Clone,
{
    type Action = A;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> BoxFuture<'static, Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper>> {
        let snapshot = (self.snapshot_fn)(query, app);
        let match_fn = self.match_fn.clone();
        (match_fn)(snapshot)
    }
}
