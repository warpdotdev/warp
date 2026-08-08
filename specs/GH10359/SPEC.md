# Spec: Bulk delete chat history (GH-10359)

## Summary

Add multi-select and bulk-delete affordances to the conversation
list, so users can clean up chat history in batches instead of
deleting conversations one row at a time. Includes a 5-second undo
window after each bulk delete.

## Problem

Today, deleting chat conversations is a per-row action. Users with
months of accumulated chat history have no efficient way to clean
up dozens of conversations at once. Issue #10359 asks for standard
multi-select + bulk-delete, with an undo affordance to soften the
risk of accidental destructive actions.

## Goals

- Enter a multi-select mode in the conversation list.
- Select via per-row checkbox, Cmd/Ctrl-click, Shift-click, or
  "Select all visible" / "Select all".
- Delete the entire selection with a single confirmation.
- Provide a 5-second undo affordance after deletion. Within the
  window, deletions are reversible; after, they are permanent.
- Keep keyboard navigation parity with mouse interactions.

## Non-Goals

- Not a chat-export feature.
- Not a chat-archive workflow (archive vs delete is a separate
  decision out of scope here).
- Not a server-side admin or workspace-wide bulk-deletion tool.
- Not a search-then-delete flow; selection is driven from the
  visible conversation list.
- Not changing single-row delete semantics. Single-row delete
  continues to work exactly as it does today.

## Behavior Contract

### B1. Entering selection mode

- A "Select" button appears in the conversation list header.
  Clicking it enters selection mode and renders a checkbox column
  on each conversation row.
- Right-clicking any conversation row exposes a "Select multiple"
  menu item that also enters selection mode and pre-selects the
  right-clicked row.
- While in selection mode, the header replaces the "Select"
  button with selection-mode controls (see B2 and B3).

### B2. Selection actions

- Per-row checkbox toggles the row's selection.
- Header controls (precise semantics):
  - **"Select all visible"** — selects every row that is at least
    PARTIALLY inside the conversation list's scrollable viewport
    rectangle at the moment the action is invoked. The selection
    is computed from the conversation-list MODEL using the
    viewport's scroll offset and item height — it is NOT driven by
    the virtualization overscan window or the materialized DOM,
    both of which are implementation details that can vary by
    overscan factor or platform. Specifically:
    - The set is `{ row | row.bottom > viewport.top AND
      row.top < viewport.bottom }`, evaluated against the model
      (not the DOM).
    - Overscan-only rows (rendered for scroll smoothing but
      outside the visible rectangle) are NOT included.
    - Rows that scroll into view AFTER the action are NOT
      auto-selected.
    - The set is deterministic for a given (scroll offset, item
      list, viewport size) tuple regardless of virtualization
      configuration.
    This is the **safe default**.
  - **"Select all"** — selects every conversation matching the
    current filter/search query, **including non-materialized
    rows** that the virtualization layer has not rendered.

    The action is a **two-step server round-trip**:

    1. **Count step.** The client issues
       `GET /api/conversations:countMatching?query=<filter>`
       (or omits the `query` param when there is no active filter)
       which returns `{ count: <int> }`. If the count exceeds
       **100**, the UI shows a confirmation chip
       (`Select all 247? [Confirm] [Cancel]`) before proceeding.
       Cancel aborts the flow without any further calls; Confirm
       continues to step 2.
    2. **ID materialization step.** The client issues
       `GET /api/conversations:listIds?query=<filter>` which
       streams back the full, ordered list of matching
       conversation ids
       (`{ ids: ["id1", "id2", ...], cursor?: "..." }`,
       paginated with a cursor when the count exceeds the
       server's per-response cap of **2,000 ids**). The client
       concatenates pages until `cursor` is absent, producing the
       authoritative selection-id set. This set is what is fed
       to the B3 delete confirmation and the
       `:batchDelete` endpoint.

    The selection chip displays the count from step 1 while step
    2 is in flight (e.g. `247 selected · fetching ids…`); the
    Delete button is **disabled** until step 2 completes, so the
    client never sends a `:batchDelete` request without a fully
    materialized id list. If step 2 fails (network error or
    partial cursor failure after retries), the selection is
    aborted with an error chip and the user is asked to retry.

    With no active filter, "Select all" applies to all
    conversations; with an active filter, it applies to the
    filtered set only (e.g. `Select all 47 matching "foo"?`).
    The 5,000-id hard cap from "Bulk-delete safety limits"
    applies to the materialized id set: if step 1's count
    exceeds 5,000, the confirmation chip is replaced by a
    blocking error chip per that section, and step 2 is never
    issued.
  - **"Clear selection"** — clears the selection set without
    exiting selection mode.
- Selection set tracks rows by **conversation ID**. The selection
  persists across filter/search changes — previously-selected
  conversations remain selected even if they are no longer visible
  due to a tightened filter. When this happens, the header chip
  surfaces both totals, e.g. `12 selected · 8 not currently
  visible`.
- Mouse modifiers:
  - Cmd-click (macOS) / Ctrl-click (Win/Linux) on a row toggles
    that single row's selection.
  - Shift-click on a row extends the selection from the most
    recently selected anchor row to the clicked row, inclusive.
- Selected count is shown live in the header, e.g.
  `12 selected`.

### B3. Delete action

- Header includes a "Delete selected (N)" button.
  - Disabled when N == 0.
  - **Skip-confirmation is NOT supported for bulk delete in V1.**
    Every bulk-delete operation prompts, regardless of any
    existing single-row "skip confirmations" preference. The
    blast radius of a bulk delete is materially larger than a
    single row, so V1 intentionally does not honor a skip-confirm
    preference here. Single-row delete continues to honor its
    existing preference unchanged.
- Confirmation dialog copy:
  > Delete N conversations? You have 5 seconds to undo.
  >
  > [Cancel] [Delete]
- After confirmation, all N conversations are removed from the
  list immediately with a fade-out transition (optimistic UI).
- **Reconciling optimistic removal with per-id failure.** The
  optimistic removal is reversed for any id that the server
  reports as `"error"` or `"forbidden"` (and never tombstoned —
  see Storage / API contract). `"not_found"` is treated as a
  **client-visible success** (the row is removed from the list and
  remains removed) but is **NOT** added to the undo tombstone
  set — there is nothing on the server to restore, so including
  it in the tombstone set would cause Undo to silently fail for
  that id. The reconciliation is:
  1. On confirmation, all N rows are removed from the rendered
     list and added to a **provisional client-side removal set**
     (not yet the canonical undo tombstone set).
  2. The client issues the batched server request(s).
  3. As each batch response arrives:
     - ids returned with status `"deleted"` are moved from the
       provisional set into the **canonical undo tombstone set**
       (eligible for Undo). The original list position is
       remembered so Undo can re-insert correctly.
     - ids returned with status `"not_found"` are removed from
       the provisional set and are **NOT** added to the undo
       tombstone set. They stay removed from the visible list
       permanently. Undo will not attempt to restore them
       (per B4-undo-coverage below). Rationale: the conversation
       does not exist on the server — there is no state to
       restore — so silently swallowing them in undo would
       mislead the user about what Undo can do. They are
       therefore not "undoable" and the user is informed via the
       snackbar count, which reflects only the `"deleted"`
       outcomes (see B4-snackbar-count below).
     - ids returned with status `"error"` or `"forbidden"` are
       RE-INSERTED into the list at their original positions,
       removed from the provisional set, and rendered with an
       inline error indicator and Retry affordance. They are NOT
       added to the undo tombstone set.
  4. The header chip and snackbar count are updated to reflect
     the actual number of conversations in the **undo tombstone
     set** (i.e., only `"deleted"` outcomes). `"not_found"`
     outcomes are reflected in the snackbar via a secondary
     informational sub-line (see B4-snackbar-count) so the user
     understands why the count may be lower than the original
     selection. If the tombstone set is empty after reconciliation
     (every id was `"error"`, `"forbidden"`, or `"not_found"`),
     the snackbar is dismissed and either an error chip surfaces
     (for `"error"` / `"forbidden"`) or an informational chip
     surfaces (for an all-`"not_found"` batch:
     `N conversations already gone — list refreshed`).

### B4. Undo

- After deletion, a snackbar appears at the bottom of the
  conversation list:
  > Deleted N conversations. **Undo**
- The snackbar persists for 5 seconds.
- **Undo-window start for multi-batch deletes.** When the
  selection exceeds the 500-id batch cap and the client splits
  into multiple sequential batches, the 5-second undo timer
  starts when the **FINAL batch's response is received** (or its
  retry budget is exhausted), NOT at confirmation time and NOT
  per-batch. The snackbar appears with a progress indicator
  during multi-batch processing
  (`Deleting 1,200 conversations… (batch 2 of 3)`); it does NOT
  show "Undo" until all batches have settled. This guarantees:
  - The user never has the timer expire on later batches before
    earlier batches return.
  - "Undo" within the 5-second window restores the entire
    selection (subject to per-id reconciliation in B3), not a
    partial subset.
  - For a single-batch delete (≤500 ids) this rule reduces to
    "timer starts when the single batch response is received",
    matching the simple case.
- Clicking "Undo" within the 5-second window restores all
  deleted conversations to their original positions in the list,
  with their prior unread / pinned / starred state intact.
- After 5 seconds, the snackbar dismisses and the deletions
  become permanent (the underlying tombstones are committed and
  cannot be recovered through this flow).
- If the user triggers another bulk delete while a snackbar is
  active, the previous deletion is committed immediately
  (tombstones flushed) and a new snackbar is shown for the new
  batch. Undo only ever applies to the most recent batch.

#### B4-snackbar-count. Snackbar count reflects undoable rows only

The primary "Deleted N conversations" count in the snackbar is
the size of the canonical undo tombstone set — i.e., the number
of ids the server returned with status `"deleted"`. It does NOT
include `"not_found"`, `"error"`, or `"forbidden"` outcomes.

When the original selection contained ids that resolved to
`"not_found"`, the snackbar adds a single non-actionable
sub-line below the main message:

> Deleted N conversations. **Undo**
> M conversations were already gone (not undoable).

`M` is the count of `"not_found"` outcomes. The sub-line is
omitted when `M == 0`. The same `role="alert"` /
`aria-live="assertive"` announcement (B4a) is extended:

> Deleted N conversations. M already gone, not undoable. Undo
> available for 5 seconds. Press Tab to focus undo.

`"error"` / `"forbidden"` counts are NOT surfaced in this
sub-line because those rows have been re-inserted into the list
with their own inline error indicators (see B3
reconciliation) — they are visibly still present and need no
further snackbar copy.

#### B4-undo-coverage. What Undo restores

Undo restores **only** ids in the canonical undo tombstone set —
the ids the server returned as `"deleted"`. Undo:

- Sends `POST /api/conversations:batchRestore` with the
  tombstoned ids only.
- Never includes `"not_found"` ids in the restore request body.
  If a `"not_found"` id were sent, the server would not be able
  to restore it (the conversation does not exist), so the client
  pre-filters them out at submission time. This makes the undo
  operation deterministic from the server's perspective: every
  id in a `:batchRestore` body MUST be a tombstone the server
  recorded during the corresponding `:batchDelete`.
- Treats restore failures (`"error"` / `"forbidden"` returned by
  the server during restore) per existing reconciliation rules —
  the user is informed and the still-deleted rows remain
  removed.

This invariant is tested by `T_undo_excludes_not_found`.

### B4a. Undo accessibility

The 5-second undo window must be reachable without a mouse and
must work with reduced-motion and assistive-technology settings:

- The snackbar's "Undo" button is keyboard-reachable: pressing
  Tab while the snackbar is visible focuses the Undo button. Esc
  returns focus to the prior context (typically the conversation
  list) without invoking Undo.
- Screen readers announce the snackbar on appearance:
  > Deleted N conversations. Undo available for 5 seconds. Press
  > Tab to focus undo.
- The snackbar element uses `role="alert"` and
  `aria-live="assertive"` so the announcement is not deferred.
- For users with `prefers-reduced-motion`, the snackbar appears
  and dismisses without slide / fade animations; the 5-second
  visibility window is unchanged.
- Cmd+Z (macOS) / Ctrl+Z (Win/Linux) while focus is anywhere
  inside the conversation list also triggers Undo, provided the
  invocation occurs within the 5-second window. After the
  window, the shortcut is a no-op for this flow.

### B5. Active-conversation refocus

- If any of the deleted conversations was the currently active
  conversation, focus moves to the next-most-recent remaining
  conversation in the list.
- If the list becomes empty, the active-conversation pane shows
  the existing empty state used elsewhere.

### B6. Keyboard

- In selection mode:
  - Up / Down arrows move the focus row without changing
    selection.
  - Space toggles selection on the focused row.
  - Shift+Up / Shift+Down extend the selection by one row.
  - Cmd/Ctrl+A selects all visible (matches B2 "Select all
    visible").
  - Cmd/Ctrl+Shift+A selects all (matches B2 "Select all").
  - Delete / Backspace triggers the same flow as the
    "Delete selected (N)" button (with the B3 confirmation).
  - Esc exits selection mode and clears the selection.

### B7. State persistence

- Selection mode and the current selection set are **not**
  persisted across app restarts. Re-launching the app returns the
  conversation list to normal (non-selection) mode with no rows
  selected.
- The 5-second undo window is **not** persisted across app
  restarts. If the user quits Warp during the 5s window, the
  pending deletion is committed on shutdown.

## Settings / API surface

- No new user-facing settings.
- Existing single-row delete-confirmation preference (if any) is
  respected for single-row deletes; bulk delete intentionally
  does NOT honor a skip-confirm preference in V1 (see B3).
- Storage layer: `app/src/storage/conversations.rs` gains a
  batch-delete API and a tombstone window so the in-memory state
  can restore the batch within the 5s undo window without a full
  reload.

### Storage / API contract (bulk delete)

Bulk delete is implemented as a **single batch endpoint** rather
than a client-side fan-out of per-row deletes. This bounds the
failure surface and gives the server a single point of accounting.

- Endpoint: `POST /api/conversations:batchDelete`
  - Method: **`POST`** with a JSON body. We deliberately do NOT
    use `DELETE` here. Many HTTP intermediaries (CDNs, proxies,
    some load balancers) and several client HTTP libraries either
    drop or refuse to forward bodies on `DELETE` requests; the
    behavior is implementation-defined per RFC 9110 §9.3.5. To
    avoid that interop landmine for a feature whose blast radius
    is destructive bulk deletion, we use a `POST` to a
    `:batchDelete` action sub-resource (the same convention used
    by Google Cloud APIs and recommended by AIP-165). Single-row
    delete continues to use `DELETE /api/conversations/{id}`
    unchanged — only the batch endpoint is `POST`.
  - Body: `{ ids: ["...", "...", ...] }`
  - Response: `{ results: [{ id, status, error_message? }, ...] }`
    where `status` is one of **`"deleted" | "unavailable" |
    "error"`** (see "Information disclosure" below for the
    rationale for collapsing the previous `"not_found"` and
    `"forbidden"` statuses into `"unavailable"`).
    `error_message` is present only when `status == "error"` and
    describes a transient backend failure suitable for client
    Retry. `error_message` is NEVER present on `"unavailable"`
    outcomes, so the client cannot distinguish the three
    underlying causes (truly missing / unauthorized / hard-
    deleted) from the wire format alone.

- **Atomicity**: best-effort, **per-id**. Partial failure does
  not roll back the successful deletions. Each id outcome is
  reported individually so the client can surface granular
  feedback.

- **Per-batch limit**: maximum **500 ids per batch**. The client
  splits a larger selection into sequential 500-id batches and
  surfaces aggregate progress in the snackbar / header chip
  (e.g. `Deleting 1,200 conversations… (batch 2 of 3)`).

- **Failure handling**:
  - Successful per-id deletions (`"deleted"`) enter the undo
    tombstone set and can be restored within the 5s window.
  - Per-id `"unavailable"` outcomes are NOT tombstoned and stay
    removed from the visible list (see B3 reconciliation and
    "Information disclosure" below). They are counted in the
    snackbar's "already gone" sub-line (B4-snackbar-count).
  - Per-id `"error"` outcomes are NOT tombstoned; the row is
    re-inserted into the conversation list with an inline error
    indicator and a Retry affordance. (The previous draft's
    `"forbidden"` branch is now folded into `"unavailable"` and
    no longer surfaces inline retry — see "Information
    disclosure".)
  - On a network-level error mid-batch (no usable response),
    retry the failed batch up to **3 times** with exponential
    backoff (250ms, 500ms, 1s). After exhaustion, surface a
    user-visible error in the snackbar with a "Retry" affordance.

- **Undo endpoint**: `POST /api/conversations:batchRestore`
  - Body: `{ ids: ["...", "...", ...] }`
  - The server retains tombstone state for at least the 5-second
    client undo window plus a server-side grace period sufficient
    to cover clock skew and request latency.

### Authentication & per-conversation authorization

The destructive batch-delete and batch-restore endpoints are
authenticated and authorized BY CONVERSATION, not just by user
session. The contract is:

- **Authentication.** Both `POST /api/conversations:batchDelete`
  and `POST /api/conversations:batchRestore` require a valid Warp
  user session token in the `Authorization` header, identical to
  the existing single-row `DELETE /api/conversations/{id}`
  endpoint. Unauthenticated requests return `401 Unauthorized`
  with NO per-id `results` body — the request is rejected at the
  edge before any id is processed.
- **Per-conversation authorization (delete).** For each id in the
  request body, the server independently verifies that the
  authenticated user has delete permission on that conversation:
  - The conversation's owning user must equal the authenticated
    user, OR the authenticated user must hold an explicit
    workspace role granting delete-conversation permission for
    the conversation's workspace.
  - Workspace-scoped conversations require the same workspace
    membership and role check the server already enforces on
    single-row delete.
  - Authorization is checked AFTER the id is parsed but BEFORE
    any tombstone/state mutation for that id.
  - An id the user does not own and does not have workspace
    permission for returns **`status: "unavailable"`** for that
    id (collapsed with the truly-nonexistent case — see
    "Information disclosure" below). NO `error_message` field is
    returned for `"unavailable"` outcomes. The other ids in the
    same batch are processed normally — one unauthorized id does
    not poison the batch.
- **Per-conversation authorization (restore).** Restore uses the
  STRICTER check:
  - The id must be present in the server-side tombstone set
    associated with the SAME authenticated user that issued the
    delete. Restoring a tombstone created by a different user is
    rejected with **`status: "unavailable"`** per id (the
    response cannot tell the caller whether the tombstone exists
    for another user vs does not exist at all).
  - Tombstones are scoped to `(user_id, conversation_id)` on the
    server. There is no cross-user undo path.
  - Restore can only resurrect conversations the user could
    delete in the first place; workspace permission changes
    between delete and restore that REVOKE delete permission also
    revoke restore (returned as **`"unavailable"`**).
- **Information disclosure (revised — addresses round-N
  concern).** The earlier wording in this section described
  `"forbidden"` and `"not_found"` as universally distinct in the
  response. That is **incorrect** because it would let an
  authenticated caller distinguish *existing-but-unauthorized*
  conversation ids from *truly nonexistent* ids via probing, in
  violation of the stated no-existence-leak requirement. The
  authoritative rule for V1 is:

  The server collapses `"forbidden"` and `"not_found"` into a
  **single response status** for the bulk-delete endpoint:
  **`"unavailable"`**. The server returns `"unavailable"`
  whenever any of the following is true for the requested id:

  1. The conversation does not exist anywhere on the server.
  2. The conversation exists but is owned by a different user
     and is not in any workspace the caller has delete
     permission on.
  3. The conversation exists but has been hard-deleted past the
     server-side tombstone window.

  None of these three cases can be distinguished by the client
  from the response alone. The server is responsible for logging
  the underlying reason internally (per "Logging" below) so
  operators retain visibility, but the wire format exposes only
  `"unavailable"`.

  The response schema therefore becomes
  `{ results: [{ id, status, error_message? }, ...] }` where
  `status ∈ { "deleted" | "unavailable" | "error" }`.
  `error_message` is present only when `status == "error"` and
  describes the transient failure (e.g. "temporary backend
  failure — please retry"). `error_message` is NEVER present on
  `"unavailable"` outcomes.

  **Client treatment of `"unavailable"`.** The client treats
  `"unavailable"` the same way it treated `"not_found"` in the
  earlier draft:

  - The optimistic removal stays — the row remains removed from
    the visible list.
  - The id is NOT added to the undo tombstone set (there is no
    server state to restore, regardless of which of the three
    underlying reasons applied).
  - The id is counted into the snackbar sub-line as part of
    "M conversations were already gone (not undoable)" (see
    B4-snackbar-count).
  - The id is NOT re-inserted into the list and NOT given a
    Retry affordance, because retrying with the same id would
    yield the same `"unavailable"` response.

  **Restore endpoint mirrors the same rule.**
  `:batchRestore` returns `status ∈ { "restored" |
  "unavailable" | "error" }` with the same collapsed semantics:
  a missing-tombstone, a cross-user-tombstone, and a
  permission-revoked-after-delete all collapse to
  `"unavailable"`, indistinguishable on the wire.

  **Where the existence-leak boundary actually lies.** A user
  CAN trivially distinguish "this id is one of mine" from
  "this id resolves to `unavailable`" — they get `"deleted"`
  versus `"unavailable"`. That is by design; the user is
  authorized to know about their own conversations. They
  CANNOT distinguish "this id exists, owned by someone else"
  from "this id was never assigned by the server" from "this
  id was hard-deleted years ago" — all three return
  `"unavailable"`. This satisfies the stated no-existence-leak
  requirement.

  **UI copy and outward-facing semantics**. The previous draft's
  client-facing terminology used the word "forbidden" in the
  Retry-error inline indicator copy. That copy is removed —
  there is no longer a `"forbidden"` status surfaced to the
  user. The inline-error indicator and Retry affordance now
  apply ONLY to `"error"` (transient backend failure) outcomes.
- **Logging.** Every per-id `"unavailable"` outcome is logged
  server-side with the authenticated user id, the requested
  conversation id, and the underlying reason
  (`not_found_globally` / `forbidden_other_user` /
  `forbidden_workspace` / `hard_deleted_past_window`); the client
  never sees those details. Server operators retain full
  visibility for abuse-detection and rate-limit
  decisions, while the wire format does not leak existence to the
  caller.
- **Rate limit interaction.** The `429 Too Many Requests` rate
  limit defined in "Bulk-delete safety limits" applies AFTER
  authentication; an unauthenticated request gets `401`, not
  `429`. The rate limit is keyed by authenticated user id, not by
  IP, to prevent shared-network users from cross-rate-limiting
  each other.

### Bulk-delete safety limits

- **Hard cap**: a single bulk-delete operation may target at most
  **5,000 conversations**. Selecting more than this and pressing
  Delete shows an error chip:
  > Maximum 5,000 conversations per delete — narrow your
  > selection.
- **Server-side rate limit**: at most **10 batch-delete
  requests per minute per user**. Exceeding this returns
  `429 Too Many Requests` and the client surfaces a transient
  error with retry-after guidance.

## Acceptance Criteria

- A1. Clicking "Select" enters selection mode; clicking again
  (now labeled "Cancel" or equivalent) exits and clears the
  selection.
- A2. Selecting via checkbox, Cmd-click, and Shift-click all
  produce the expected selection set; the live header count is
  accurate.
- A3. "Delete selected (N)" always prompts the bulk-delete
  confirmation. The per-row "skip confirmations" preference is
  not honored for bulk delete in V1.
- A4. Within 5s, clicking Undo restores all deleted conversations
  to their original positions and state.
- A5. Deleting the active conversation as part of a bulk batch
  refocuses to the next-most-recent remaining conversation.
- A6. Keyboard: Space toggles, Shift+Arrow extends, Cmd/Ctrl+A
  selects visible, Cmd/Ctrl+Shift+A selects all, Esc exits.
- A7. Restarting the app while in selection mode returns the
  list to normal mode with no selection.
- A8. Quitting during a 5s undo window commits the deletion on
  next launch (no zombie undo snackbar).
- A9. Triggering a second bulk delete during an active undo
  window commits the previous batch immediately and starts a new
  5s window for the new batch.

## Implementation Pointers

- `app/src/conversation_list/*.rs` — selection-mode state, row
  rendering with checkbox column.
- `app/src/conversation_list/header.rs` (new or extended) —
  selection toolbar with "Select all visible", "Select all",
  "Clear selection", "Delete selected (N)", and live count.
- `app/src/conversation_list/snackbar.rs` (new) — 5s undo
  snackbar component, reusable for future similar flows.
- `app/src/storage/conversations.rs` — batch-delete API,
  tombstone window, commit-on-timeout / commit-on-shutdown /
  commit-on-next-batch logic.
- `app/src/keybindings/conversation_list.rs` — keyboard map for
  selection mode.

## Tests

- T1. Selection-mode toggle: enter from header button, enter
  from row context menu, exit via Esc, exit via header.
- T2. Mixed selection paths: checkbox + Cmd-click + Shift-click
  produce the same selection set the user would expect from
  standard list semantics.
- T3. Bulk delete shows the confirmation dialog even when
  single-row skip-confirmation preference is on.
- T4. Undo within 5s restores all deleted conversations,
  including their original ordering and state (unread / pinned /
  active).
- T5. Active-conversation refocus on bulk delete picks the next
  most-recent remaining conversation; empty-state shown when
  list is fully drained.
- T6. Full keyboard navigation: every interaction in B6 is
  covered with a deterministic test.
- T7. Selection cleared on app restart (no persisted selection).
- T8. Quitting during 5s undo commits the deletion on next
  launch; the restored conversations do NOT reappear.
- T9. Back-to-back bulk deletes: second delete commits the
  first; undo only restores the second batch.
- T_select_all_visible_overscan_invariant. With virtualization
  configured to render 5 overscan rows above and below the
  viewport: scroll so that rows 100..120 are visible inside the
  viewport rectangle (rows 95..99 and 121..125 are materialized
  but outside the visible rectangle). Invoke "Select all visible"
  → exactly rows 100..120 are selected. Re-run with overscan
  set to 50 in the same scroll position → still exactly rows
  100..120 are selected. The result is independent of overscan.
- T_select_all_visible_no_post_scroll_capture. After "Select all
  visible" returns, scroll the list. Newly-visible rows are NOT
  added to the selection set.
- T_optimistic_reconcile_error_transient. Bulk delete 5 ids;
  server returns `"error"` for one id, `"deleted"` for the other
  4. The errored id is re-inserted at its original list position
  with an inline error indicator and a Retry affordance; the
  snackbar count reads "Deleted 4 conversations". No
  `"already gone"` sub-line appears.
- T_optimistic_reconcile_unavailable. Bulk delete 5 ids; server
  returns `"unavailable"` for one id, `"deleted"` for the other
  4. The unavailable id stays REMOVED (no re-insert, no Retry
  affordance) and is NOT added to the undo tombstone set; the
  snackbar reads "Deleted 4 conversations" with a sub-line
  "1 conversation was already gone (not undoable)".
- T_optimistic_reconcile_all_failed. Bulk delete 3 ids; server
  returns `"error"` for all 3. Snackbar is dismissed (count ↓ 0)
  and an error chip surfaces. No tombstones remain.
- T_optimistic_reconcile_all_unavailable. Bulk delete 3 ids;
  server returns `"unavailable"` for all 3. Snackbar is dismissed
  and an informational chip surfaces:
  `3 conversations already gone — list refreshed`. No tombstones
  remain; the 3 rows stay removed; no Retry affordance is shown.
- T_undo_timer_starts_after_final_batch. Bulk delete 1,200 ids
  → splits into 3 batches. Batch 1 returns at t=200ms, batch 2
  at t=400ms, batch 3 at t=900ms. Snackbar displays
  "Undo" only at t=900ms. Timer-to-commit fires at t=5,900ms,
  not earlier. Undo at t=2,000ms restores the full 1,200-id
  batch (subject to per-id reconciliation). Undo at t=6,500ms
  is a no-op.
- T_select_all_over_100_warning. With 247 conversations matching
  filter, "Select all" first issues `:countMatching` (returns
  247) and surfaces `Select all 247? [Confirm] [Cancel]` chip.
  Cancel → no selection, no `:listIds` request issued. Confirm →
  client issues `:listIds`, materializes 247 ids, header chip
  reads "247 selected" once the list is materialized.
- T_select_all_two_step_id_materialization. With 3,500
  conversations matching filter, "Select all" → `:countMatching`
  returns 3,500 → confirmation chip shown → Confirm → client
  issues `:listIds` and receives two paginated pages (2,000 ids
  with a cursor, then 1,500 ids with no cursor). Assert the
  Delete button is disabled until all 3,500 ids are
  materialized, then enabled. Triggering Delete sends a
  `:batchDelete` with the full 3,500 ids split across 7 batches
  of 500.
- T_select_all_listids_failure_aborts. "Select all" with 200
  matching conversations: `:countMatching` returns 200, user
  Confirms, but `:listIds` returns network error on every retry.
  Assert no selection is recorded, an error chip is shown
  ("Couldn't load conversation list — try again"), and no
  `:batchDelete` is ever issued.
- T_batch_endpoint_post_method. Inspect the network request for
  bulk delete: method is `POST`, path is
  `/api/conversations:batchDelete`, body is
  `{ "ids": [...] }`. No `DELETE` request is issued during bulk
  delete (single-row delete continues to use `DELETE` and is
  unaffected).
- T_auth_unauthenticated_401. Issue a batchDelete without a
  session token → response is `401 Unauthorized` with NO
  `results` body, NO state mutation, NO tombstone created.
- T_auth_per_id_unavailable_isolated. Authenticated user A
  deletes a batch of 3 ids: id1 (own), id2 (owned by user B, not
  in any shared workspace), id3 (own). Response: id1 →
  `"deleted"`, id2 → `"unavailable"`, id3 → `"deleted"`. id3 is
  NOT poisoned by id2's failure. The snackbar reads "Deleted 2
  conversations" with sub-line "1 conversation was already gone".
- T_auth_restore_other_user_unavailable. User A deletes id1.
  Within the 5s window, user B (different session) attempts
  `batchRestore { ids: [id1] }` → response status
  `"unavailable"`. id1 is NOT restored on user B's side; user A's
  tombstone is untouched and user A can still Undo successfully.
- T_auth_no_existence_leak. Authenticated user A requests
  batchDelete for id_x where id_x exists but belongs to user B
  (no shared workspace). Response is **`"unavailable"`**.
  Authenticated user A requests batchDelete for id_y that does
  NOT exist anywhere → response is also **`"unavailable"`**.
  Authenticated user A requests batchDelete for id_z that was
  hard-deleted past the server tombstone window → also
  **`"unavailable"`**. The client CANNOT distinguish these three
  cases from the wire response (no `error_message`, no
  differentiating status code, identical envelope).
- T_unavailable_no_error_message. For any `"unavailable"` outcome
  in `:batchDelete` or `:batchRestore`, assert the response
  object contains exactly `{ id, status: "unavailable" }` and
  NO `error_message` field — JSON schema validation rejects
  any extra fields.
- T_undo_excludes_not_found. Bulk delete 4 ids: 3 succeed
  (`"deleted"`), 1 is `"unavailable"`. Within the 5s window,
  click Undo. Inspect the `:batchRestore` request body — it
  contains exactly the 3 tombstoned ids; the `"unavailable"` id
  is NOT in the body. The 3 conversations are restored to their
  original positions; the `"unavailable"` id remains removed.
- T_status_no_forbidden_no_not_found_on_wire. Across every test
  that exercises the bulk-delete / restore endpoints, assert
  that no response `status` field ever takes the values
  `"forbidden"` or `"not_found"`. The only valid statuses on the
  wire are `"deleted" | "unavailable" | "error"` for delete and
  `"restored" | "unavailable" | "error"` for restore.
- T_rate_limit_keyed_by_user. User A and user B share the same
  source IP. User A issues 10 batch deletes in 60s → 11th
  returns `429`. User B's first batch in the same window is
  NOT rate-limited (rate limit is per authenticated user id,
  not per IP).

## Open Questions

- Q1. Should bulk-delete be gated behind a feature flag for the
  first preview rollout? Proposal: **yes**, gate to the Preview
  channel for one cycle, then promote to Stable.
- Q2. Should the snackbar duration be user-configurable? V1
  proposal: no — 5s is a fixed default matching macOS Finder's
  recover semantics. Revisit if accessibility feedback demands a
  longer window.
- Q3. ~~Should "Select all" warn before selecting >N (e.g., 500)
  conversations?~~ **Resolved**: V1 keeps the >100 confirmation
  chip on "Select all" defined in B2 (`Select all 247?
  [Confirm] [Cancel]`). The earlier "no warning" wording in this
  question was inconsistent with B2 and is superseded — B2 is the
  authoritative contract. The B3 delete confirmation always
  shows the count as a second guardrail, but it does not replace
  the >100 selection-time confirmation.
- Q4. ~~Does the storage layer need a server-side bulk-delete
  endpoint, or is per-row deletion fanned-out client-side
  acceptable for V1?~~ **Resolved**: V1 uses a single batch
  endpoint (`POST /api/conversations:batchDelete`) with explicit
  per-id results, a 500-id batch cap, exponential-backoff retry,
  and tombstone-based undo. See **Storage / API contract** above
  for the full request/response shape and the rationale for using
  `POST` instead of `DELETE` (HTTP intermediaries inconsistently
  forward bodies on `DELETE`). Client-side fan-out is rejected
  because it leaves partial-failure semantics undefined and makes
  server-side rate limiting and accounting unreliable.

## Telemetry

- No new event. Extend the existing `conversation.delete` event
  with two new fields:
  - `count: u32` — number of conversations in the deletion
    (1 for single-row delete, N for bulk delete).
  - `bulk: bool` — `true` when the deletion came from the
    bulk-delete flow, `false` for single-row.
- An additional `undone: bool` field on the same event records
  whether the user invoked Undo within the 5s window. Events for
  bulk deletions fire after the 5s window closes (or on
  early-commit due to next-batch / shutdown), so the field is
  always known at emit time.
