use std::cmp::Ordering;
use std::collections::HashSet;

use chrono::TimeZone;
use warpui::{AppContext, EntityId, SingletonEntity, WindowId};

use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::blocklist::history_model::{AIConversationMetadata, BlocklistAIHistoryModel};
use crate::terminal::view::blocklist_filter;
use crate::undo_close::UndoCloseStack;
use crate::workspace::{PaneViewLocator, WorkspaceRegistry};

/// Result from matching a conversation.
/// terminal_view_id and window_id are optional because, when we add restored conversations,
/// these conversations will not have associated windows or terminal views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationNavigationData {
    pub id: AIConversationId,
    pub title: String,
    pub initial_query: Option<String>,
    pub last_updated: chrono::DateTime<chrono::Local>,
    pub terminal_view_id: Option<EntityId>,
    pub window_id: Option<WindowId>,
    pub pane_view_locator: Option<PaneViewLocator>,
    pub initial_working_directory: Option<String>,
    pub latest_working_directory: Option<String>,
    pub is_selected: bool,
    pub is_in_active_pane: bool,
    /// The conversation is hidden on the undo stack if its parent view (either the tab or the split pane)
    /// has been recently closed, and this closure can still be undone. We should still show the conversation
    /// as historical even though the pane group still "exists", as the pane group is still hidden to the user.
    pub is_closed: bool,
    /// The server-generated conversation token, used to reference this conversation in context tags.
    pub server_conversation_token: Option<ServerConversationToken>,
}

impl PartialOrd for ConversationNavigationData {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ConversationNavigationData {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.is_historical(), other.is_historical()) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            _ => match (self.is_in_active_pane, other.is_in_active_pane) {
                (false, true) => Ordering::Less,
                (true, false) => Ordering::Greater,
                _ => match (self.is_selected, other.is_selected) {
                    (false, true) => Ordering::Less,
                    (true, false) => Ordering::Greater,
                    _ => self.last_updated.cmp(&other.last_updated),
                },
            },
        }
    }
}

impl ConversationNavigationData {
    #[allow(clippy::too_many_arguments)]
    pub fn from_ai_conversation(
        conversation: &AIConversation,
        terminal_view_id: Option<EntityId>,
        window_id: Option<WindowId>,
        pane_view_locator: Option<PaneViewLocator>,
        initial_working_directory: Option<String>,
        is_selected: bool,
        is_in_active_pane: bool,
        is_closed: bool,
    ) -> Self {
        let initial_query = conversation.initial_query();
        let title = conversation
            .title()
            .unwrap_or_else(|| "Untitled conversation".to_string());
        let last_updated = conversation
            .latest_exchange()
            .map(|exchange| exchange.start_time)
            .unwrap_or_else(chrono::Local::now);

        Self {
            id: conversation.id(),
            title,
            initial_query,
            last_updated,
            terminal_view_id,
            window_id,
            pane_view_locator,
            initial_working_directory,
            latest_working_directory: conversation.current_working_directory(),
            is_selected,
            is_in_active_pane,
            is_closed,
            server_conversation_token: conversation.server_conversation_token().cloned(),
        }
    }

    pub fn from_historical_conversation_metadata(metadata: &AIConversationMetadata) -> Self {
        Self {
            id: metadata.id,
            title: metadata.title.clone(),
            initial_query: Some(metadata.initial_query.clone()),
            last_updated: chrono::Local.from_utc_datetime(&metadata.last_modified_at),
            terminal_view_id: None,
            window_id: None,
            pane_view_locator: None,
            initial_working_directory: metadata.initial_working_directory.clone(),
            latest_working_directory: None,
            is_selected: false,
            is_in_active_pane: false,
            is_closed: false,
            server_conversation_token: metadata.server_conversation_token.clone(),
        }
    }

    pub fn id(&self) -> AIConversationId {
        self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn last_updated(&self) -> chrono::DateTime<chrono::Local> {
        self.last_updated
    }

    pub fn pane_view_locator(&self) -> Option<PaneViewLocator> {
        self.pane_view_locator
    }

    pub fn is_in_active_pane(&self) -> bool {
        self.is_in_active_pane
    }

    pub fn window_id(&self) -> Option<WindowId> {
        self.window_id
    }

    // A conversation is historical if it does not have an open terminal view associated with it
    pub fn is_historical(&self) -> bool {
        self.terminal_view_id.is_none() || self.is_closed
    }

    pub fn all_conversations(app: &AppContext) -> Vec<ConversationNavigationData> {
        let history_model = BlocklistAIHistoryModel::as_ref(app);

        // Iterate through all registered workspaces and collect conversations from terminal views
        let mut all_conversations = Vec::new();
        let mut all_conversation_ids = HashSet::new();

        let mut open_terminal_views: HashSet<EntityId> = HashSet::new();

        let active_window_id = app.windows().active_window();
        let registry = WorkspaceRegistry::as_ref(app);
        for (window_id, workspace_handle) in registry.all_workspaces(app) {
            let workspace = workspace_handle.as_ref(app);
            let active_tab_pane_group_id = workspace.active_tab_pane_group().id();

            for pane_group_handle in workspace.tab_views() {
                // Use try_as_ref to avoid panicking if the pane group is currently
                // being mutated (e.g., during a split operation that creates a new
                // terminal view which calls this function).
                let Some(pane_group) = pane_group_handle.try_as_ref(app) else {
                    continue;
                };
                for pane_id in pane_group.terminal_pane_ids() {
                    let Some(terminal_view) = pane_group.terminal_view_from_pane_id(pane_id, app)
                    else {
                        continue;
                    };

                    let Some(terminal_view_ref) = terminal_view.try_as_ref(app) else {
                        continue;
                    };

                    let is_closed = pane_group.is_pane_hidden_for_close(pane_id)
                        || UndoCloseStack::as_ref(app)
                            .is_pane_group_tab_in_stack(pane_group_handle.id());

                    let terminal_view_id = terminal_view.id();
                    open_terminal_views.insert(terminal_view_id);

                    // Skip conversation transcript viewers, as they are stored elsewhere
                    // and should not be presented as regular user conversations.
                    if history_model
                        .is_terminal_surface_conversation_transcript_viewer(terminal_view_id)
                    {
                        continue;
                    }

                    // Get the context model to determine selected conversation for this terminal view
                    let selected_conversation_id = terminal_view_ref
                        .ai_context_model()
                        .as_ref(app)
                        .selected_conversation_id(app);

                    // Get all continuable conversations for this terminal view
                    for conversation in
                        history_model.all_live_conversations_for_terminal_surface(terminal_view_id)
                    {
                        if !all_conversation_ids.contains(&conversation.id()) {
                            if conversation.should_exclude_from_navigation() {
                                // Track the ID so the historical loop below doesn't re-add it.
                                all_conversation_ids.insert(conversation.id());
                                continue;
                            }

                            let is_selected =
                                !is_closed && Some(conversation.id()) == selected_conversation_id;

                            if !is_selected
                                && !blocklist_filter::conversation_would_render_in_blocklist(
                                    conversation,
                                )
                            {
                                continue;
                            }

                            let pane_view_locator = PaneViewLocator {
                                pane_group_id: pane_group_handle.id(),
                                pane_id,
                            };

                            all_conversations.push(
                                ConversationNavigationData::from_ai_conversation(
                                    conversation,
                                    Some(terminal_view_id),
                                    Some(window_id),
                                    Some(pane_view_locator),
                                    conversation
                                        .initial_working_directory()
                                        .or_else(|| terminal_view.as_ref(app).pwd()),
                                    is_selected,
                                    // Check if the conversation is in the active pane, tab, and window
                                    // to determine if its pane is currently focused.
                                    Some(window_id) == active_window_id
                                        && pane_group_handle.id() == active_tab_pane_group_id
                                        && pane_group.focused_pane_id(app) == pane_id,
                                    is_closed,
                                ),
                            );
                            all_conversation_ids.insert(conversation.id());
                        }
                    }
                }
            }
        }

        // Get conversations from terminal views that were open at the start of the session,
        // but have since been closed.
        history_model
            .all_live_conversations()
            .iter()
            .for_each(|(terminal_id, conversation)| {
                if conversation.should_exclude_from_navigation()
                    || history_model
                        .is_terminal_surface_conversation_transcript_viewer(*terminal_id)
                    || !blocklist_filter::conversation_would_render_in_blocklist(conversation)
                {
                    // Track the ID so the historical loop below doesn't re-add it.
                    all_conversation_ids.insert(conversation.id());
                    return;
                }

                if !open_terminal_views.contains(terminal_id)
                    && !all_conversation_ids.contains(&conversation.id())
                {
                    all_conversation_ids.insert(conversation.id());
                    all_conversations.push(ConversationNavigationData::from_ai_conversation(
                        conversation,
                        None,
                        None,
                        None,
                        conversation.initial_working_directory(),
                        false,
                        false,
                        false,
                    ));
                }
            });

        // Get conversations that have been cleared from the terminal view
        history_model
            .all_cleared_conversations()
            .iter()
            .for_each(|(terminal_id, conversation)| {
                if conversation.should_exclude_from_navigation()
                    || history_model
                        .is_terminal_surface_conversation_transcript_viewer(*terminal_id)
                    || !blocklist_filter::conversation_would_render_in_blocklist(conversation)
                {
                    // Track the ID so the historical loop below doesn't re-add it.
                    all_conversation_ids.insert(conversation.id());
                    return;
                }

                if !all_conversation_ids.contains(&conversation.id()) {
                    all_conversation_ids.insert(conversation.id());
                    all_conversations.push(ConversationNavigationData::from_ai_conversation(
                        conversation,
                        None,
                        None,
                        None,
                        conversation.initial_working_directory(),
                        false,
                        false,
                        false,
                    ));
                }
            });

        let historical_conversations = Self::historical_conversations(app);

        for conversation in historical_conversations {
            let conversation_id = conversation.id();
            if !all_conversation_ids.contains(&conversation_id) {
                all_conversations.push(conversation);
                all_conversation_ids.insert(conversation_id);
            }
        }

        all_conversations.sort();
        all_conversations
    }

    pub fn historical_conversations(app: &AppContext) -> Vec<ConversationNavigationData> {
        let history_model = BlocklistAIHistoryModel::as_ref(app);

        let mut conversations = Vec::new();
        let mut conversation_ids = HashSet::new();

        history_model
            .get_local_conversations_metadata()
            .for_each(|metadata| {
                let conversation =
                    ConversationNavigationData::from_historical_conversation_metadata(metadata);
                if !conversation_ids.contains(&conversation.id()) {
                    conversation_ids.insert(conversation.id());
                    conversations.push(conversation);
                }
            });

        conversations.sort();
        conversations
    }
}
