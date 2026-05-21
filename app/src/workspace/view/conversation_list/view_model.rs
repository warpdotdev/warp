use crate::ai::agent_conversations_model::{
    AgentConversationEntry, AgentConversationEntryId, AgentConversationsModel,
    AgentConversationsModelEvent, AgentManagementFilters, ArtifactFilter, CreatedOnFilter,
    CreatorFilter, OwnerFilter, SourceFilter, StatusFilter,
};
use fuzzy_match::match_indices_case_insensitive;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

pub struct ConversationListViewModelEvent;

#[derive(Clone, Debug)]
pub struct ConversationEntry {
    pub id: AgentConversationEntryId,
    pub highlight_indices: Vec<usize>,
}

pub struct ConversationListViewModel {
    conversations_model: ModelHandle<AgentConversationsModel>,
    cached_entry_ids: Vec<AgentConversationEntryId>,
    filtered_items: Vec<ConversationEntry>,
    search_query: String,
}

impl Entity for ConversationListViewModel {
    type Event = ConversationListViewModelEvent;
}

impl ConversationListViewModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let conversations_model = AgentConversationsModel::handle(ctx);

        ctx.subscribe_to_model(&conversations_model, |me, event, ctx| {
            match event {
                // These events change the set of items in the list, so we need
                // to rebuild the cached ID list.
                AgentConversationsModelEvent::ConversationsLoaded
                | AgentConversationsModelEvent::NewTasksReceived
                | AgentConversationsModelEvent::TasksUpdated => {
                    me.refresh_cached_items(ctx);
                }
                // Status changes don't affect the set of IDs (status is read
                // at render time via get_item_by_id); just signal a re-render.
                AgentConversationsModelEvent::ConversationUpdated { .. } => {
                    ctx.emit(ConversationListViewModelEvent);
                }
                // Artifact updates don't affect the conversation list
                AgentConversationsModelEvent::ConversationArtifactsUpdated { .. } => {}
            }
        });

        let mut model = Self {
            conversations_model,
            cached_entry_ids: Vec::new(),
            filtered_items: Vec::new(),
            search_query: String::new(),
        };
        model.refresh_cached_items(ctx);
        model
    }

    /// Rebuilds the cached list of IDs from the current task/conversation set.
    ///
    /// The cache stores only `AgentConversationEntryId`s; per-item fields like
    /// status, title, and last-updated are read fresh at render time via
    /// `get_item_by_id`. Callers should therefore avoid invoking this on
    /// events that only mutate per-item state (e.g. `ConversationUpdated`);
    /// emitting `ConversationListViewModelEvent` is sufficient there.
    fn refresh_cached_items(&mut self, ctx: &mut ModelContext<Self>) {
        let model = self.conversations_model.as_ref(ctx);
        self.cached_entry_ids = model
            .get_entries(
                &AgentManagementFilters {
                    owners: OwnerFilter::PersonalOnly,
                    status: StatusFilter::All,
                    source: SourceFilter::All,
                    created_on: CreatedOnFilter::All,
                    creator: CreatorFilter::All,
                    artifact: ArtifactFilter::All,
                    environment: Default::default(),
                    harness: Default::default(),
                },
                ctx,
            )
            .into_iter()
            .filter(|entry| entry.capabilities.can_open)
            .map(|entry| entry.id)
            .collect();

        self.apply_search_filter(ctx);
        ctx.emit(ConversationListViewModelEvent);
    }

    pub fn set_search_query(&mut self, query: String, ctx: &mut ModelContext<Self>) {
        if query == self.search_query {
            return;
        }

        self.search_query = query;
        self.apply_search_filter(ctx);
        ctx.emit(ConversationListViewModelEvent);
    }

    fn apply_search_filter(&mut self, ctx: &mut ModelContext<Self>) {
        let search_query = self.search_query.trim().to_lowercase();
        let conversations_model = self.conversations_model.as_ref(ctx);

        if search_query.is_empty() {
            self.filtered_items = self
                .cached_entry_ids
                .iter()
                .map(|id| ConversationEntry {
                    id: *id,
                    highlight_indices: vec![],
                })
                .collect();
        } else {
            let mut matched_items: Vec<(i64, ConversationEntry)> = self
                .cached_entry_ids
                .iter()
                .filter_map(|id| {
                    let item = conversations_model.get_entry_by_id(id, ctx)?;

                    match_indices_case_insensitive(&item.display.title, &search_query).map(
                        |result| {
                            (
                                result.score,
                                ConversationEntry {
                                    id: *id,
                                    highlight_indices: result.matched_indices,
                                },
                            )
                        },
                    )
                })
                .collect();

            matched_items.sort_by(|a, b| b.0.cmp(&a.0));
            self.filtered_items = matched_items.into_iter().map(|(_, item)| item).collect();
        }
    }

    /// Returns the total number of conversations in the model before any filtering is applied.
    pub fn unfiltered_item_count(&self) -> usize {
        self.cached_entry_ids.len()
    }

    /// Returns the filtered items with their highlight indices.
    pub fn filtered_items(&self) -> &[ConversationEntry] {
        &self.filtered_items
    }

    /// Look up a normalized conversation entry by ID.
    pub fn get_item_by_id(
        &self,
        id: &AgentConversationEntryId,
        ctx: &AppContext,
    ) -> Option<AgentConversationEntry> {
        let model = self.conversations_model.as_ref(ctx);
        model.get_entry_by_id(id, ctx)
    }

    pub fn current_ids(&self) -> impl Iterator<Item = &AgentConversationEntryId> {
        self.filtered_items.iter().map(|item| &item.id)
    }
}
