# Conversation Message Timestamps — Tech Spec

Product spec: `specs/conversation-message-timestamps/PRODUCT.md`

Research commit: `0a7d5380e4a2cf39428ccfd5c367fc7dda39dd48`

## Context

An Agent Mode turn is represented by one `AIAgentExchange`. Its `start_time` already records when
the user input was sent, so user-query timestamps require no server, GraphQL, or persistence
changes.

- [`app/src/ai/agent/mod.rs (3325-3375)`](https://github.com/warpdotdev/warp/blob/0a7d5380e4a2cf39428ccfd5c367fc7dda39dd48/app/src/ai/agent/mod.rs#L3325-L3375)
  defines `AIAgentExchange::start_time` as the time the input was sent.
- [`app/src/ai/agent/conversation.rs (2040-2095)`](https://github.com/warpdotdev/warp/blob/0a7d5380e4a2cf39428ccfd5c367fc7dda39dd48/app/src/ai/agent/conversation.rs#L2040-L2095)
  creates live exchanges with the request start time.
- [`app/src/ai/agent/api/convert_conversation.rs (1881-1945)`](https://github.com/warpdotdev/warp/blob/0a7d5380e4a2cf39428ccfd5c367fc7dda39dd48/app/src/ai/agent/api/convert_conversation.rs#L1881-L1945)
  reconstructs restored start times from the input's `CurrentTime` context or persisted message
  timestamps. Its final fallback is `DateTime::default()`.

The GUI already has both interaction points shown in the Figma design:

- [`app/src/ai/blocklist/block/view_impl.rs (906-1050)`](https://github.com/warpdotdev/warp/blob/0a7d5380e4a2cf39428ccfd5c367fc7dda39dd48/app/src/ai/blocklist/block/view_impl.rs#L906-L1050)
  resolves the query participant, renders the query, and places the Agent View overflow button on
  the query row.
- [`app/src/ai/blocklist/block/view_impl/query.rs (38-151)`](https://github.com/warpdotdev/warp/blob/0a7d5380e4a2cf39428ccfd5c367fc7dda39dd48/app/src/ai/blocklist/block/view_impl/query.rs#L38-L151)
  owns the query row and calls `render_user_avatar`.
- [`app/src/ai/blocklist/block/view_impl/common.rs (3549-3576)`](https://github.com/warpdotdev/warp/blob/0a7d5380e4a2cf39428ccfd5c367fc7dda39dd48/app/src/ai/blocklist/block/view_impl/common.rs#L3549-L3576)
  renders the current avatar without a tooltip.
- [`app/src/ai/blocklist/block/view_impl/header.rs (206-249)`](https://github.com/warpdotdev/warp/blob/0a7d5380e4a2cf39428ccfd5c367fc7dda39dd48/app/src/ai/blocklist/block/view_impl/header.rs#L206-L249)
  renders the reusable three-dot control.

The existing overflow menu and clipboard path are block-scoped:

- [`app/src/terminal/view/context_menu.rs (14-163)`](https://github.com/warpdotdev/warp/blob/0a7d5380e4a2cf39428ccfd5c367fc7dda39dd48/app/src/terminal/view/context_menu.rs#L14-L163)
  builds the AI-block copy group and its ordering.
- [`app/src/terminal/view/context_menu.rs (383-479)`](https://github.com/warpdotdev/warp/blob/0a7d5380e4a2cf39428ccfd5c367fc7dda39dd48/app/src/terminal/view/context_menu.rs#L383-L479)
  opens the overflow menu for an exchange.
- [`app/src/terminal/view.rs (25128-25326)`](https://github.com/warpdotdev/warp/blob/0a7d5380e4a2cf39428ccfd5c367fc7dda39dd48/app/src/terminal/view.rs#L25128-L25326)
  routes context-menu copy actions back to the matching `AIBlock`.
- [`app/src/ai/blocklist/block.rs (426-500)`](https://github.com/warpdotdev/warp/blob/0a7d5380e4a2cf39428ccfd5c367fc7dda39dd48/app/src/ai/blocklist/block.rs#L426-L500)
  owns persistent mouse-state handles. The avatar tooltip handle must live here rather than being
  created during rendering.

## Proposed changes

### 1. Expose a trustworthy query timestamp through the block model

Extend `AIBlockModel` with a `query_sent_at` accessor returning `Option<DateTime<Local>>`.
`AIBlockModelImpl` reads `start_time` from its exchange and returns `None` when it equals
`DateTime::<Local>::default()`.

The trait boundary keeps live, restored, forked, imported, and fake block models decoupled from the
view. Give the trait method a default `None` implementation so test/import models do not need data
they cannot provide.

Do not substitute conversation creation metadata or `Local::now()` in the UI. Existing conversation
processing remains the only source of the recorded timestamp.

### 2. Add one shared timestamp formatter

Add `format_message_timestamp(DateTime<Local>) -> String` to
`app/src/util/time_format.rs`. It returns the clipboard value required by Product Behavior 2 and 6
using `%-m/%-d at %-I:%M %p`.

Both tooltip and clipboard code call this helper:

- Tooltip: `format!("Message sent {}", format_message_timestamp(timestamp))`
- Clipboard: `format_message_timestamp(timestamp)`

Keeping one formatter prevents the display and copied value from drifting.

### 3. Add the query-avatar tooltip

Add a persistent `query_timestamp_tooltip_handle` to `AIBlockStateHandles`. Pass the valid query
timestamp and handle through `query::Props`.

Keep `render_user_avatar` generic. In `query.rs`, wrap its existing avatar element with
`Appearance::ui_builder().tool_tip_on_element` or the overlay equivalent. When the timestamp is
absent, render the avatar unchanged and preserve its current spacing, transcript-navigation ring,
shared-session participant styling, and selection behavior.

### 4. Add the copy action to the existing overflow menu

Add `ContextMenuAction::CopyAIBlockTimestamp { ai_block_view_id }` and
`AIBlockAction::CopyTimestamp`. Follow the existing `CopyAIBlockQuery` routing pattern:

1. The menu builder finds the matching `AIBlock` and checks `query_sent_at`.
2. If present, insert "Copy timestamp" after optional "Copy command" and "Copy git branch" and
   before the separator above "Save as prompt".
3. Selecting the item routes to the matching `AIBlock`.
4. `AIBlockAction::CopyTimestamp` resolves `query_sent_at` again, formats it with
   `format_message_timestamp`, and writes plain text through the existing clipboard API.

Re-resolving at selection time avoids embedding wall-clock data in actions. If the timestamp is no
longer available, the action is a no-op. When the timestamp is absent during menu construction, do
not insert the item; all existing labels and ordering remain unchanged.

The same menu builder serves both the explicit three-dot button and AI-block right-click menu, so
both entry points receive consistent behavior without duplicated menu code.

### 5. Keep responses, restoration, and APIs unchanged

Do not add response icons, response overflow controls, response finish-time accessors, or
query/response target enums. Agent-response timestamps are deferred.

Do not add fields to conversation metadata, GraphQL, protobuf messages, or SQLite. Loaded exchanges
already reconstruct the required query start time. The explicit default-time check prevents ancient
or malformed restored conversations from displaying an epoch value.

Orchestration transcript rows and inter-agent message avatars remain unchanged. An orchestrated
conversation receives this feature only on its normal user-query rows.

## End-to-end flow

1. Conversation processing creates or restores an `AIAgentExchange` with `start_time`.
2. `AIBlockModelImpl::query_sent_at` rejects the default sentinel and exposes a valid local time.
3. `AIBlock` passes that time and a persistent mouse-state handle to the query row.
4. Hover formats the timestamp for the avatar tooltip.
5. The existing overflow menu conditionally adds "Copy timestamp".
6. Selecting it resolves, formats, and writes the query timestamp to the clipboard.

## Testing and validation

### Unit tests

- Add focused formatter cases in `app/src/util/time_format_tests.rs` for single-digit and
  double-digit month/day/hour, zero-padded minutes, midnight, and noon. These cover Product
  Behavior 2, 6, and 11.
- Add focused timestamp-eligibility coverage at the block-model/helper boundary: a normal start
  time is available and the default/epoch sentinel is rejected. This covers Product Behavior 1
  and 9.
- If menu ordering can be extracted into a small pure helper without increasing production
  complexity, test insertion after conditional command/branch items and omission when unavailable.
  Otherwise verify ordering through the GUI; do not add a brittle full-menu snapshot.

### Static checks

- Run `./script/format`.
- Run the repository clippy command used by `./script/presubmit`.
- Run the targeted unit-test packages containing the formatter and block-model tests.

### GUI verification

After a successful build, verify the running app with computer use:

1. Hover a user-query avatar and confirm the exact tooltip format.
2. Open the three-dot menu and confirm "Copy timestamp" is after conditional command/branch copy
   items and before "Save as prompt".
3. Select "Copy timestamp" and confirm the clipboard contains only the friendly timestamp.
4. Restore a historical conversation and repeat the tooltip and copy checks.
5. Confirm a query with an unavailable/default timestamp has no tooltip or menu item.
6. Confirm shared-session participant avatars, transcript navigation rings, text selection, narrow
   panes, and existing menu actions remain unchanged.
7. Confirm agent responses and orchestration transcript avatars have no new timestamp affordances.

Capture screenshots of the query tooltip and query menu. Run the repository's cloud UI verification
flow after local checks because this is a user-facing client change.

## Risks and mitigations

- **Invalid restored timestamps:** restoration can default a missing start time. Central validity
  checks omit the feature instead of exposing an epoch date.
- **Tooltip state resets:** inline mouse-state handles lose hover state across renders. Store the
  new handle in `AIBlockStateHandles`.
- **Menu regressions:** the copy group has several conditional items. Insert immediately before its
  existing separator and verify both command/branch-present and absent cases.
- **Timezone ambiguity:** use the stored `DateTime<Local>` directly and one formatter everywhere;
  do not introduce UTC conversion in the view layer.

## Parallelization

Parallel implementation agents are not recommended. The remaining change is small and the model
accessor, query rendering, context-menu action, and tests touch tightly coupled `AIBlock` interfaces.
Implement sequentially in the current
`/Users/vkodithala/Desktop/warp/warp.varoon-conversation-timestamps` worktree on
`varoon/conversation-timestamps`, then run cloud GUI verification after local validation passes.
