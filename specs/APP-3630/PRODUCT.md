# Artifact Notifications — Product Spec

## Problem

When an agent creates artifacts (plans, PRs, screenshots) during a conversation, the user has no way to know this happened without navigating into the conversation. The existing notifications only fire on status changes (completed, blocked, error).

## Desired Behavior

When a conversation reaches a terminal state (success, cancelled, error) and artifacts were added during the most recent turn, the completion notification (both toast and mailbox) should include an artifact row beneath the standard notification content. The artifact row uses the same interactive chip style as the management view (plan name, branch name, PR link, screenshot count).

## Scope

- **Trigger:** The existing completion notification (`UpdatedConversationStatus` reaching a terminal state). No separate artifact notification.
- **Which artifacts:** All artifacts added since the last terminal-state notification (i.e. accumulated across all turns of the current response).
- **Interactivity:** The artifact chips are interactive (same buttons used in the management view). Clicking the notification itself navigates to the conversation.
- **Surfaces:** Both the toast popup and the notification mailbox.
- **Feature flag:** Gated behind `hoa_notifications` (the existing flag for the notification system).
