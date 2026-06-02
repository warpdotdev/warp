use enum_iterator::Sequence;
use instant::Instant;
use uuid::Uuid;
use warpui::EntityId;

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::artifacts::Artifact;
use crate::terminal::CLIAgent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NotificationId(Uuid);

impl NotificationId {
    fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationCategory {
    /// The agent has stopped (i.e. successfully completed or was cancelled)
    Complete,
    /// The agent needs user action (i.e. blocked on some permission request or idle prompt)
    Request,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Sequence)]
pub enum NotificationFilter {
    All,
    Unread,
    Errors,
}

impl NotificationFilter {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            NotificationFilter::All => "All tabs",
            NotificationFilter::Unread => "Unread",
            NotificationFilter::Errors => "Errors",
        }
    }
}

/// Identifies the agent that produced a notification, including whether the run was
/// ambient (cloud) or local. The `is_ambient` flag drives the cloud-lobe rendering in
/// [`render_agent_avatar`].
#[derive(Debug, Clone, Copy)]
#[allow(clippy::upper_case_acronyms)]
pub enum NotificationSourceAgent {
    Oz { is_ambient: bool },
    CLI { agent: CLIAgent, is_ambient: bool },
}

impl NotificationSourceAgent {
    pub fn is_ambient(&self) -> bool {
        match self {
            NotificationSourceAgent::Oz { is_ambient }
            | NotificationSourceAgent::CLI { is_ambient, .. } => *is_ambient,
        }
    }
}

/// Identifies the conversation or session a notification belongs to.
/// Used for de-duplication (replacing stale notifications on update)
/// and cleanup (removing notifications when the source closes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationOrigin {
    Conversation(AIConversationId),
    /// CLI sessions are keyed by terminal view because we only track one session per pane.
    CLISession(EntityId),
}

#[derive(Debug, Clone)]
pub struct NotificationItem {
    pub id: NotificationId,
    pub origin: NotificationOrigin,
    pub title: String,
    pub message: String,
    pub category: NotificationCategory,
    pub agent: NotificationSourceAgent,
    /// Whether the user has already seen this notification
    /// (either because they clicked into it or because it was emitted for a conversation
    /// that they've since navigated to).
    pub is_read: bool,
    pub created_at: Instant,
    pub terminal_view_id: EntityId,
    pub artifacts: Vec<Artifact>,
    /// The git branch associated with this notification's conversation/session.
    /// When present, the notification renders in "rich" layout with a branch header row.
    /// When absent, it falls back to the "simple" layout.
    pub branch: Option<String>,
}

impl NotificationItem {
    /// Marks this notification as read. Returns true if it was previously unread.
    fn mark_as_read(&mut self) -> bool {
        if self.is_read {
            return false;
        }
        self.is_read = true;
        true
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        title: String,
        message: String,
        category: NotificationCategory,
        agent: NotificationSourceAgent,
        origin: NotificationOrigin,
        is_read: bool,
        terminal_view_id: EntityId,
        artifacts: Vec<Artifact>,
        branch: Option<String>,
    ) -> Self {
        Self {
            id: NotificationId::new(),
            origin,
            title,
            message,
            category,
            agent,
            is_read,
            created_at: Instant::now(),
            terminal_view_id,
            artifacts,
            branch,
        }
    }
}

#[derive(Debug, Default)]
pub struct NotificationItems {
    items: Vec<NotificationItem>,
}

impl NotificationItems {
    /// Push a notification items into the mailbox list
    /// (deleting older notifications if we've exceeded the max list size).
    pub(crate) fn push(&mut self, item: NotificationItem) {
        self.remove_by_origin(item.origin);
        self.items.insert(0, item);
        self.items.truncate(100);
    }

    pub(crate) fn remove_by_origin(&mut self, key: NotificationOrigin) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.origin != key);
        self.items.len() != before
    }

    pub(crate) fn items_filtered(
        &self,
        filter: NotificationFilter,
    ) -> impl Iterator<Item = &NotificationItem> {
        self.items.iter().filter(move |item| match filter {
            NotificationFilter::All => true,
            NotificationFilter::Unread => !item.is_read,
            NotificationFilter::Errors => item.category == NotificationCategory::Error,
        })
    }

    pub(crate) fn filtered_count(&self, filter: NotificationFilter) -> usize {
        self.items_filtered(filter).count()
    }

    /// Returns the filters that should be shown as tabs.
    /// "All" is always included; other filters are included only when they have at least one item.
    pub(crate) fn visible_filters(&self) -> Vec<NotificationFilter> {
        enum_iterator::all::<NotificationFilter>()
            .filter(|f| *f == NotificationFilter::All || self.filtered_count(*f) > 0)
            .collect()
    }

    pub(crate) fn get_by_id(&self, id: NotificationId) -> Option<&NotificationItem> {
        self.items.iter().find(|item| item.id == id)
    }

    /// Marks all notifications from the given terminal view as read.
    /// Returns true if any were changed.
    pub(crate) fn mark_all_terminal_view_items_as_read(
        &mut self,
        terminal_view_id: EntityId,
    ) -> bool {
        let mut any_changed = false;
        for item in &mut self.items {
            if item.terminal_view_id == terminal_view_id {
                any_changed |= item.mark_as_read();
            }
        }
        any_changed
    }

    pub(crate) fn mark_item_read(&mut self, id: NotificationId) -> bool {
        self.items
            .iter_mut()
            .find(|item| item.id == id)
            .is_some_and(|item| item.mark_as_read())
    }

    pub(crate) fn mark_all_items_read(&mut self) -> bool {
        let mut any_changed = false;
        for item in &mut self.items {
            any_changed |= item.mark_as_read();
        }
        any_changed
    }

    pub(crate) fn has_unread_for_terminal_view(&self, terminal_view_id: EntityId) -> bool {
        self.items
            .iter()
            .any(|item| item.terminal_view_id == terminal_view_id && !item.is_read)
    }
}

#[cfg(test)]
#[path = "item_tests.rs"]
mod tests;
