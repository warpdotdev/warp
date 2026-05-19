use std::sync::Arc;

use warp_core::report_error;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::auth::AuthStateProvider;
use crate::server::server_api::{auth::AuthClient, ServerApiProvider};
use warp_graphql::scalars::Time;

const PAGE_SIZE: i32 = 20;

pub struct UsageHistoryModel {
    auth_client: Arc<dyn AuthClient>,
    entries: Vec<warp_graphql::queries::get_conversation_usage::ConversationUsage>,
    is_loading: bool,
    // Whether the server indicated that there may be more entries to load.
    has_more_entries: bool,
}

impl Entity for UsageHistoryModel {
    type Event = ();
}

impl SingletonEntity for UsageHistoryModel {}

impl UsageHistoryModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let auth_client = ServerApiProvider::as_ref(ctx).get_auth_client();
        Self {
            auth_client,
            entries: Vec::new(),
            is_loading: false,
            has_more_entries: true,
        }
    }

    pub fn entries(&self) -> &[warp_graphql::queries::get_conversation_usage::ConversationUsage] {
        &self.entries
    }

    pub fn is_loading(&self) -> bool {
        self.is_loading
    }

    pub fn has_more_entries(&self) -> bool {
        self.has_more_entries
    }

    /// Fetches conversation usage over the past 30 days.
    /// If some usage has already been loaded, this fetches the same number of entries.
    /// If no usage has been loaded, this fetches PAGE_SIZE entries.
    pub fn refresh_usage_history_async(&mut self, ctx: &mut ModelContext<Self>) {
        if self.is_loading || !AuthStateProvider::as_ref(ctx).get().is_logged_in() {
            return;
        }

        // If the user has already loaded some number of entries,
        // we should load that same number of items on refresh so that the list doesn't shrink
        // every time the page is refreshed.
        let num_items_to_fetch = if self.entries.is_empty() {
            PAGE_SIZE
        } else {
            self.entries.len() as i32
        };

        // Reset pagination state and clear any existing entries.
        self.entries.clear();
        self.has_more_entries = true;

        self.fetch_next_page(num_items_to_fetch, None, ctx);
    }

    /// Fetches the next page of conversation usage entries, appending them to the existing list.
    pub fn load_more_usage_history_async(&mut self, ctx: &mut ModelContext<Self>) {
        if self.is_loading || !self.has_more_entries {
            return;
        }

        let last_updated_end_timestamp: Option<Time> =
            self.entries.last().map(|entry| entry.last_updated);
        if last_updated_end_timestamp.is_none() {
            return;
        }

        self.fetch_next_page(PAGE_SIZE, last_updated_end_timestamp, ctx);
    }

    /// Fetches the next page of conversation usage entries, appending them to the existing list.
    /// last_updated_end_timestamp is the timestamp of the last entry in the existing list,
    /// and is used to paginate the results and only return entries that we don't already have.
    fn fetch_next_page(
        &mut self,
        limit: i32,
        last_updated_end_timestamp: Option<Time>,
        ctx: &mut ModelContext<Self>,
    ) {
        // If no time stamp is provided for pagination, we can assume that this is the first page of results.
        let is_initial_load = last_updated_end_timestamp.is_none();
        let auth_client = self.auth_client.clone();

        if is_initial_load {
            self.is_loading = true;
            ctx.notify();
        }

        ctx.spawn(
            async move {
                auth_client
                    .get_conversation_usage_history(
                        Some(30),
                        Some(limit),
                        last_updated_end_timestamp,
                    )
                    .await
            },
            move |me, result, ctx| {
                me.is_loading = false;
                match result {
                    Ok(entries) => {
                        let fetched_count = entries.len() as i32;

                        // If we received fewer than requested, assume there are no more entries.
                        me.has_more_entries = fetched_count == limit;

                        if !is_initial_load {
                            me.entries.extend(entries);
                        } else {
                            me.entries = entries;
                        }
                    }
                    Err(e) => {
                        report_error!(e.context("Failed to fetch conversation usage"));
                    }
                }
                ctx.notify();
            },
        );
    }
}
