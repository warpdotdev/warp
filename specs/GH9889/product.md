# Product spec: Per-message timestamps in Agent Mode (GH-9889)

## Problem

Agent Mode shows the conversation as a sequence of prompts and
responses with no temporal information. Users cannot tell:

- When a specific prompt was submitted.
- When the agent finished responding to it.
- How long the agent took.
- How recent any given exchange is — particularly relevant when
  resuming a long-running conversation, comparing two attempts, or
  reviewing what an autonomous agent did overnight.

The data already exists at the model layer: `AIAgentExchange` derives
both start time (from `AIAgentContext::CurrentTime` on the user input)
and finish time (from the latest message timestamp on the exchange).
`Conversation::start_ts()` exposes the conversation-wide start. Only
the view layer omits this information.

## Goal

Every prompt-and-response pair in Agent Mode shows when the prompt
was submitted and how long the agent took to respond, in a form that
is glanceable, non-disruptive to the conversation flow, and accurate
across long-running and restored conversations.

## Non-goals (V1)

- **Per-token or per-tool-call timestamps.** The granularity is the
  exchange (one user prompt + the agent response that follows it),
  not individual streamed tokens or individual tool invocations.
- **Server time vs. client time reconciliation.** Use the timestamps
  Warp already records. If the user's clock skews mid-conversation,
  the displayed times skew with it. (Existing exchange timestamp
  derivation has the same property.)
- **Wall-clock duration vs. cumulative agent CPU time.** Display
  wall-clock only. CPU/credit accounting belongs to a different
  surface (#10000 token tracking, #10052 credit reconciliation).
- **Editing or backdating timestamps.** Read-only display.
- **Time-zone selection.** Use `Local` (matches existing
  `Local.timestamp_opt(...)` derivation in conversation.rs:549).
  *Time-zone* selection is out of scope. **12h-vs-24h preference**
  is in scope (see B2) and is read from the OS — that is not a
  user-facing setting in V1, just an OS preference read at startup.
- **Showing timestamps in the conversation export / yaml.** Out of
  scope — the export already preserves the underlying timestamps,
  this spec is about the in-app view.
- **Adding timestamps to CLI agent (third-party) conversations.**
  V1 is Warp's first-party Agent Mode only. CLI agent harness
  conversations have their own surface and a follow-up issue.

## Behavior contract (V1)

### B1 — Every visible exchange shows two timestamps

For each visible `AIAgentExchange`, the view shows:
- A **submitted-at** timestamp aligned with the user prompt
  bubble.
- A **completed-at** timestamp aligned with the agent response
  bubble.

Hidden exchanges (per `Conversation::is_exchange_hidden`) do not get
timestamps surfaced. In-progress exchanges (no completion timestamp
yet) show the submitted-at and an active "running for Xs" duration
in place of the completed-at, updated at most once per second.

### B2 — Display format: relative for recent, absolute for older

- **<1 minute ago:** "just now"
- **<1 hour ago:** "Xm ago" (e.g. "5m ago")
- **<24 hours ago:** "HH:MM" in the user's locale's preferred
  format (e.g. "3:47 PM" or "15:47")
- **<7 days ago:** "Day HH:MM" (e.g. "Mon 3:47 PM")
- **>=7 days ago:** "YYYY-MM-DD HH:MM"

The format auto-promotes as time passes (a "just now" entry becomes
"5m ago" five minutes later) without requiring the user to
re-render the conversation. Updates fire at most every 30 seconds
to avoid layout churn.

### B3 — Duration display

The duration between submitted-at and completed-at is shown next to
the completed-at:
- **<1 second:** "<1s"
- **<60 seconds:** "Xs"
- **<60 minutes:** "Xm Ys" (drop the seconds if X >= 10)
- **>=60 minutes:** "Xh Ym"

For in-progress exchanges, the duration is "running for Xs" with
the seconds counter live.

### B4 — Hover for absolute time

Hovering any timestamp shows a tooltip with the full absolute
local time, ISO 8601 form: "2026-05-04 15:47:23 -05:00". This is
the disambiguator for users who need exact timing (debugging,
support tickets, regression diffs).

### B5 — Hidden by default behind a setting; default ON

> **Correction (re-review #10128):** the previous draft cited a
> non-existent `SyncToCloud::Always`. The actual variants in
> `app/src/settings/cloud_preferences.rs` are `Never`,
> `Globally(RespectUserSyncSetting)`, and `PerPlatform(...)`. The
> right value here is
> `SyncToCloud::Globally(RespectUserSyncSetting::Yes)` — sync
> across the user's devices, but respect their global cloud-sync
> opt-out.

A new setting `agents.show_message_timestamps` (boolean, default
`true`,
`SyncToCloud::Globally(RespectUserSyncSetting::Yes)`) gates the
entire feature. When `false`, the view layer is identical to today
(no timestamp glyphs, no tooltip hooks, no extra layout). Existing
keyboard shortcuts and focus handlers are unchanged in either state.

The default is `true` because the feature is opt-out (the issue is
about a missing affordance) but power users who want a denser view
can disable it.

### B6 — Restored conversations show timestamps from history

A conversation restored from yaml or from session restoration shows
the same timestamps as it did before the restoration. The
restoration path already preserves message timestamps; the view
layer must re-derive submitted-at and completed-at on restore.

### B7 — Missing-timestamp fallback (defensive guard only)

> **Correction (re-review #10128):** the previous draft promised
> the dash for restored conversations whose timestamps were lost.
> The model already synthesizes `Local::now()` at
> [conversation.rs:1777/1895/1962](app/src/ai/agent/conversation.rs)
> in those exact paths via
> `finish_time_from_exchange_messages(...).unwrap_or_else(Local::now)`,
> so a finished restored exchange never has a `None` finish_time.
> Promising the dash there contradicted the model. Resolved below.

In normal operation **the dash never renders**:
- In-progress exchanges show `ProgressDurationLabel` ("running for
  Xs"), not a dash.
- Finished exchanges always have `finish_time: Some(...)` after the
  recompute paths, so they show the timestamp.
- Restored conversations with lost stored timestamps get a
  synthesized `Local::now()` from the recompute paths, which means
  on the FIRST render after restore they appear as "just now" and
  promote out of that bucket on the 30s tick.

The dash is reserved for two **defensive guards** that only fire
on a model invariant violation:
1. `output_status` reports finished but `finish_time` is somehow
   `None` (state-machine bug, should not happen with the current
   schema).
2. A future schema change makes `start_time` optional and a
   `None` slips through.

In either case, render "—" and emit `log::warn!(exchange_id =
?id, ...)` once per affected exchange so the team can spot the
violation in production. These cost nothing in the happy path.

### B8 — In-progress exchange handling

While an exchange is running:
- Submitted-at is rendered immediately on prompt submission (not
  waiting for the first response chunk).
- The "running for Xs" counter ticks once per second.
- On completion, the counter freezes and is replaced with the
  completed-at timestamp + final duration.

Cancellation: if the user cancels the exchange before completion,
display "cancelled at HH:MM" + the duration spent before
cancellation.

## Acceptance criteria

A1. With `agents.show_message_timestamps = true`, sending a prompt
    immediately shows submitted-at "just now" next to the prompt
    bubble.

A2. While the agent is responding, a "running for Xs" counter
    appears next to the response bubble, ticking once per second.

A3. On agent completion, the counter is replaced with the
    completed-at timestamp + final duration.

A4. Hovering any timestamp shows a tooltip with the full ISO 8601
    local time.

A5. With `agents.show_message_timestamps = false`, no timestamps,
    tooltips, or duration counters appear, and the view is
    pixel-equivalent to the current build.

A6. Restoring a conversation from yaml (with messages timestamped
    yesterday) shows the timestamps as "Mon 3:47 PM" (per B2 step
    4) — not "just now," not the load time.

A7. **Defensive guard:** an exchange that violates the model
    invariant (`output_status` reports finished but `finish_time`
    is `None`) shows "—" in the duration slot and emits a single
    `log::warn!` per exchange. In normal operation this never
    fires — the model's recompute paths synthesize `Local::now()`
    so finished exchanges always have `Some(finish_time)`.

A8. The display format auto-promotes from "just now" → "Xm ago" →
    "HH:MM" without requiring a click or re-open of the
    conversation panel.

## Risks and decisions for tech.md

1. **Where to render in the agent view.** Two candidate sites:
   (a) Inside each message bubble's metadata row, alongside any
       existing model/branch/permission badges.
   (b) Outside the bubble, in a thin gutter aligned to the bubble's
       top edge.
   The TECH spec must pick one, with attention to existing density
   and to vertical layout impact. Recommendation: (a) for V1,
   leveraging the existing badge row to avoid a new layout slot.

2. **Tick frequency.** A live "Xs" counter that updates every
   second on every visible in-progress exchange in a long
   conversation could cost render time. The TECH spec must define:
   - Single shared 1Hz timer for all live exchanges (preferred), OR
   - Per-exchange timers (simpler, more wasteful).
   And it must define how the timer is paused when the conversation
   panel is hidden.

3. **Format auto-promotion mechanic.** "just now" must become
   "5m ago" after 5 minutes without user action. The TECH spec
   must define:
   - A 30-second tick for refresh of all visible relative-time
     labels (B2's max 30s update rule), OR
   - Render-on-scroll only.
   Recommendation: the 30s tick, gated to "while panel is visible
   AND any visible timestamp is in a relative-format bucket" so we
   don't waste cycles on conversations whose timestamps are all
   absolute.

4. **Setting default.** Default `true` (opt-out) per B5. Confirm
   with maintainers before implementation; this is the most
   reversible decision.

5. **Telemetry.** Add only the existing `setting_changed` event when
   the user toggles the new setting, so the team can see the opt-out
   rate. No timestamp-display telemetry is added.

   **Privacy guardrails (security review #10128):**
   - The `setting_changed` payload is `{ setting: "agents.show_message_timestamps",
     new_value: bool }`. No timestamp values, no exchange/conversation
     IDs, no message content, no clock format / locale info.
   - The event respects the existing global telemetry opt-out — if
     the user has disabled product analytics, no event fires regardless
     of the setting toggle.
   - The setting itself follows
     `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`, so the
     boolean value is synced as part of normal settings sync IF the
     user has cloud sync enabled — and is held local-only otherwise.
     This is the same privacy semantic as other UX preferences in
     `app/src/settings/code.rs`; no new data class.
   - No client-side ticker / counter values are ever transmitted.
   - The feature does not introduce any new server-side telemetry
     channels.

## Reporter-supplied detail (preserved)

The reporter explicitly cited "long agent tasks" and "reference past
conversations" as the motivating workflows. They suggested
"5:47 AM" or "2 min ago" as example formats — both are subsumed by
B2 (absolute for older, relative for recent). The issue carries the
`needs-mocks` label; this spec deliberately does not pin pixel
positions or visual treatment, leaving that to the design pass.
