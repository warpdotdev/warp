use crate::ai::active_agent_views_model::ConversationOrTaskId;
use crate::ai::agent_conversations_model::{
    AgentConversationsModel, AgentConversationsModelEvent, AgentManagementFilters, ArtifactFilter,
    ConversationOrTask, CreatedOnFilter, CreatorFilter, OwnerFilter, SessionStatus, SourceFilter,
    StatusFilter,
};
use fuzzy_match::match_indices_case_insensitive;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

pub struct ConversationListViewModelEvent;

#[derive(Clone, Debug)]
pub struct ConversationEntry {
    pub id: ConversationOrTaskId,
    pub highlight_indices: Vec<usize>,
}

pub struct ConversationListViewModel {
    conversations_model: ModelHandle<AgentConversationsModel>,
    cached_conversation_or_task_ids: Vec<ConversationOrTaskId>,
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
                | AgentConversationsModelEvent::TasksUpdated
                | AgentConversationsModelEvent::TaskManuallyOpened => {
                    me.refresh_cached_items(ctx);
                }
                // Status changes don't affect the set of IDs (status is read
                // at render time via get_item_by_id); just signal a re-render.
                AgentConversationsModelEvent::ConversationUpdated => {
                    ctx.emit(ConversationListViewModelEvent);
                }
                // Artifact updates don't affect the conversation list
                AgentConversationsModelEvent::ConversationArtifactsUpdated { .. } => {}
            }
        });

        let mut model = Self {
            conversations_model,
            cached_conversation_or_task_ids: Vec::new(),
            filtered_items: Vec::new(),
            search_query: String::new(),
        };
        model.refresh_cached_items(ctx);
        model
    }

    /// Rebuilds the cached list of IDs from the current task/conversation set.
    ///
    /// The cache stores only `ConversationOrTaskId`s; per-item fields like
    /// status, title, and last-updated are read fresh at render time via
    /// `get_item_by_id`. Callers should therefore avoid invoking this on
    /// events that only mutate per-item state (e.g. `ConversationUpdated`);
    /// emitting `ConversationListViewModelEvent` is sufficient there.
    fn refresh_cached_items(&mut self, ctx: &mut ModelContext<Self>) {
        let model = self.conversations_model.as_ref(ctx);
        self.cached_conversation_or_task_ids = model
            .get_tasks_and_conversations(
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
            // Expired and Unavailable ambient agent sessions can't be opened, so we filter them out.
            // Regular conversations have None session_status
            .filter(|item| {
                item.get_session_status()
                    .is_none_or(|status| status == SessionStatus::Available)
            })
            // Only show user-initiated sources (Slack, Linear, Interactive) or tasks that have
            // been manually opened from the management page.
            .filter(|item| {
                let is_user_initiated = item.source().is_some_and(|s| s.is_user_initiated());
                let is_manually_opened = match item {
                    ConversationOrTask::Task(task) => model.is_task_manually_opened(&task.task_id),
                    ConversationOrTask::Conversation(_) => false,
                };
                is_user_initiated || is_manually_opened
            })
            .map(|item| match item {
                ConversationOrTask::Task(task) => ConversationOrTaskId::TaskId(task.task_id),
                ConversationOrTask::Conversation(conv) => {
                    ConversationOrTaskId::ConversationId(conv.nav_data.id)
                }
            })
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
                .cached_conversation_or_task_ids
                .iter()
                .map(|id| ConversationEntry {
                    id: *id,
                    highlight_indices: vec![],
                })
                .collect();
        } else {
            let mut matched_items: Vec<(i64, ConversationEntry)> = self
                .cached_conversation_or_task_ids
                .iter()
                .filter_map(|id| {
                    let item = match id {
                        ConversationOrTaskId::TaskId(task_id) => {
                            conversations_model.get_task(task_id)
                        }
                        ConversationOrTaskId::ConversationId(conv_id) => {
                            conversations_model.get_conversation(conv_id)
                        }
                    }?;

                    match_indices_case_insensitive(&item.title(ctx), &search_query).map(|result| {
                        (
                            result.score,
                            ConversationEntry {
                                id: *id,
                                highlight_indices: result.matched_indices,
                            },
                        )
                    })
                })
                .collect();

            matched_items.sort_by(|a, b| b.0.cmp(&a.0));
            self.filtered_items = matched_items.into_iter().map(|(_, item)| item).collect();
        }
    }

    /// Returns the total number of conversations in the model before any filtering is applied.
    pub fn unfiltered_item_count(&self) -> usize {
        self.cached_conversation_or_task_ids.len()
    }

    /// Returns the filtered items with their highlight indices.
    pub fn filtered_items(&self) -> &[ConversationEntry] {
        &self.filtered_items
    }

    /// Look up a conversation or task by ID.
    pub fn get_item_by_id<'a>(
        &self,
        id: &ConversationOrTaskId,
        ctx: &'a AppContext,
    ) -> Option<ConversationOrTask<'a>> {
        let model = self.conversations_model.as_ref(ctx);
        match id {
            ConversationOrTaskId::TaskId(task_id) => model.get_task(task_id),
            ConversationOrTaskId::ConversationId(conv_id) => model.get_conversation(conv_id),
        }
    }

    pub fn current_ids(&self) -> impl Iterator<Item = &ConversationOrTaskId> {
        self.filtered_items.iter().map(|item| &item.id)
    }
}
