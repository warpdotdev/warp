# Conversation Message Timestamps

## Summary

Users can inspect when an individual user query was sent by hovering the query's avatar. The same
timestamp can be copied from that query's overflow menu without making timestamps permanently
visible in the conversation.

## Figma

Figma:
https://www.figma.com/design/AsF5uAM6L5tUmc11vm9YSi/Agent-orchestration?node-id=5077-128600&m=dev

The first two frames define the user-query hover tooltip and the placement of the
"Copy timestamp" action.

## Goals and non-goals

- Users can view and copy timestamps for user queries in Agent Mode conversations.
- Timestamps remain hidden until requested through hover or the overflow menu.
- The behavior is consistent in live, resumed, historical, shared, and orchestrated
  conversations when a trustworthy timestamp is available.
- This release does not add timestamps to agent responses, tool calls, action cards, reasoning
  sections, or inter-agent transcript entries.
- This feature does not add a setting for timestamp visibility or change conversation-level
  creation and update times.
- This feature does not promise a server-authoritative send time or backfill missing timestamps
  in old conversations.

## Behavior

1. A user query has a message timestamp representing when the user submitted that query.

2. Hovering the avatar beside a user query with a trustworthy timestamp shows a tooltip with:
   `Message sent M/D at h:mm AM/PM`.
   - Month, day, and hour do not have leading zeroes.
   - Minutes always use two digits.
   - The time uses a 12-hour clock with `AM` or `PM`.
   - The year, seconds, and timezone are not shown.
   - The timestamp is presented in local time.

3. Moving the pointer away from the avatar dismisses the tooltip according to the existing Warp
   tooltip behavior. Showing or dismissing the tooltip does not move focus, select the message,
   or alter the conversation.

4. Opening the overflow menu for a user query with a trustworthy timestamp includes a
   "Copy timestamp" action.

5. "Copy timestamp" appears after conditional query-specific copy actions such as "Copy command"
   and "Copy git branch", and before the divider above "Save as prompt". Existing menu items retain
   their relative order.

6. Selecting "Copy timestamp" writes the friendly timestamp shown inside the tooltip to the
   clipboard, without the `Message sent` prefix. For example, a tooltip reading
   `Message sent 8/10 at 9:22 AM` copies `8/10 at 9:22 AM`.

7. Selecting "Copy timestamp" follows the existing success and failure feedback behavior for copy
   actions in the same menu. This feature does not introduce a timestamp-specific confirmation
   state.

8. Live, resumed, historical, shared, and orchestrated conversations expose the same behavior when
   the relevant query has a trustworthy timestamp. Whether the conversation is being viewed by its
   creator or another participant does not change the displayed value.

9. If a user query has no trustworthy timestamp, Warp omits both its timestamp tooltip and its
   "Copy timestamp" action. Warp must not substitute the conversation creation time, display an
   epoch/default date, or show an unknown placeholder.

10. User queries without timestamps remain otherwise unchanged. Their avatar, content, overflow
    menu, selection behavior, and adjacent message spacing match the behavior they had before this
    feature.

11. The timestamp is calculated once for the query represented in the conversation. Opening the
    same tooltip or menu repeatedly produces the same value; elapsed time does not turn it into
    relative text such as "5 minutes ago".

12. Opening one query's timestamp tooltip or menu does not expose timestamps on other messages and
    does not close, modify, or reorder any conversation content.

13. The "Copy timestamp" action participates in the overflow menu's existing keyboard navigation
    and activation behavior. Adding the action must not prevent users from reaching or invoking
    neighboring menu items.
