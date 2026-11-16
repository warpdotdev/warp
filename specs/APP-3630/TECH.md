# Artifact Row in Completion Notifications — Tech Spec

Product spec: `specs/APP-3630/PRODUCT.md`

## Current State

**Notification data:** `NotificationItem` (notifications/item.rs:62) has `title`, `message`, `category`, `agent`, `origin`, `is_read`, `created_at`, `terminal_view_id`. No artifact data.

**Notification creation:** `AgentNotificationsModel` (agent_management_model.rs) listens for `UpdatedConversationStatus` and creates notifications in `handle_history_event_for_mailbox`. It does not listen for `UpdatedConversationArtifacts`.

**Artifact event:** `UpdatedConversationArtifacts` (history_model.rs:2003) is emitted by `AIConversation::add_artifact` and `update_plan_notebook_uid`. It carries `terminal_view_id` and `conversation_id` but not the artifact itself.

**Rendering:** `render_notification_item_content` (item_rendering.rs:21) renders avatar + title + message. Takes `&NotificationItem` + `&Appearance`, no view context.

**Artifact buttons pattern:** The management view (agent_management/view.rs:1106-1113) creates `ViewHandle<ArtifactButtonsRow>`, subscribes to events, and stores the handle in `CardState`.

## Changes

### 1. Carry the artifact in `UpdatedConversationArtifacts`

Add `artifact: Artifact` field to the `UpdatedConversationArtifacts` event (history_model.rs:2003). Clone the artifact at both emit sites:
- `add_artifact` (conversation.rs:1010): clone before pushing.
- `update_plan_notebook_uid` (conversation.rs:1025): clone after mutating.

### 2. Accumulate artifacts in `AgentNotificationsModel`

Add `pending_artifacts: HashMap<AIConversationId, Vec<Artifact>>` to `AgentNotificationsModel`.
- On `UpdatedConversationArtifacts`: append the artifact to `pending_artifacts[conversation_id]`.
- On `InProgress`: do **not** clear `pending_artifacts` — artifacts accumulate across turns.
- On terminal state (Success/Cancelled/Error): drain `pending_artifacts[conversation_id]` and pass to `add_notification`.
- On conversation deletion/removal: clean up `pending_artifacts`.
- CLI agent notifications: empty vec.

### 3. Add `artifacts: Vec<Artifact>` to `NotificationItem`

Thread through `NotificationItem::new` and `add_notification`.

### 4. Store `ArtifactButtonsRow` views in toast and mailbox

Both `NotificationToastItem` and `NotificationMailboxView` store an `Option<ViewHandle<ArtifactButtonsRow>>` per notification. Create on add, subscribe to `ArtifactButtonsRowEvent`, handle actions (same as agent_management/view.rs:1116-1163).

### 5. Thread artifact view into rendering

Add `Option<&ViewHandle<ArtifactButtonsRow>>` param to `render_notification_item_content`. Render `ChildView` below message text when present. Update call sites in toast and mailbox.

## Parallelism

Step 1 must go first. Steps 2-3 and steps 4-5 can be done in parallel.
