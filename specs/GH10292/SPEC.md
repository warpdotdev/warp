# Start Timestamp on Collapsed Agent Sub-Blocks (GH-10292)

> Note: setting was renamed mid-spec from the earlier draft
> `agent.reasoning_phase_timestamp_format` to `agent.subblock_timestamp_format`
> for accuracy across affected sub-block types (reasoning, tool calls, plan steps).

## Summary

During Agent conversations, reasoning/thinking phases (and other agent sub-blocks
such as tool calls and plan steps) collapse into a single expandable header line
that currently carries no time information. This spec adds a start timestamp to
each collapsed sub-block header (e.g. `▸ Thinking… (2m ago)` or
`▸ Thinking… (11:42:07)`), and shows start time + duration when the block is
expanded. Format is configurable per user preference and locale-aware.

## Problem

- Long agent runs accumulate many collapsed reasoning / tool-call / plan-step
  rows. Users cannot tell when each phase started without expanding it, which
  makes it hard to:
  - Correlate a phase with a terminal action they remember triggering.
  - Judge whether a still-running phase has been pending too long.
  - Audit a transcript after the fact (e.g. "what was the model doing at 11:42?").
- The conversation already records start events for these sub-blocks server-side;
  they just are not surfaced in the collapsed UI.
- Per-message timestamps (PR #10128) cover the message-level granularity but do
  not extend into the sub-blocks within a single agent message.

## Goals

- Collapsed reasoning / tool-call / plan-step rows show a start timestamp inline
  in the header.
- Expanded headers show both an absolute start time and a live elapsed counter
  (in-progress) or final duration (completed) — the elapsed value updates on
  the existing 1 Hz ticker, never as a separate `running…` placeholder.
- Format respects the user's locale and existing 12h/24h convention.
- A user-facing setting selects between absolute, relative, automatic, or off.
- A second user-facing setting (`agent.subblock_timestamp_show_in_expanded`)
  controls whether the timestamp affordance appears in the expanded view at
  all; the format setting always governs the format itself.
- Coverage extends consistently across all collapsible "agent sub-blocks"
  (reasoning, tool calls, plan steps).

## Non-Goals

- Not per-token / per-chunk timing within a phase.
- Not editing or reordering phases in the conversation.
- Not new server-side analytics on per-phase durations beyond what already
  exists.
- Not redesigning the collapsed-row visual treatment beyond adding the
  timestamp affordance.
- Not changing how message-level timestamps render (those continue to come
  from #10128).

## Behavior Contract

### B1. Affected sub-blocks

The following collapsible blocks within an agent message are in scope:

- Reasoning / thinking phases.
- Tool-call blocks.
- Plan-step entries.

#### B1.1. Plan-step row definition (single source of truth)

A "plan-step row" is a single rendered row in the **agent transcript's
TODO list** — the UI rendered by
`app/src/ai/blocklist/block/view_impl/todos.rs` (NOT
`app/src/ai/blocklist/prompt/plan_and_todo_list.rs`, which renders the
prompt/context chip and is out of scope for this feature). Each
plan-step row corresponds to ONE `AIAgentTodo` entry in the
`AIAgentTodoList` for a given message. The timestamp affordance is
attached to these transcript rows; the prompt-chip view is unchanged
by this spec. Each plan-step row carries the SAME timestamp
affordance as a reasoning or tool-call sub-block:

- **Identity.** A plan-step row is identified by its
  `(message_id, AIAgentTodoId)` pair. `AIAgentTodoId` already exists
  in `app/src/ai/agent/mod.rs` (around line 1444); `message_id` comes
  from the enclosing agent message. The pair is stable across
  re-renders.
- **Start time — derived, not event-driven.** The agent orchestrator
  does NOT today emit per-step `PlanStepStarted` events on the
  conversation event stream
  (`app/src/ai/blocklist/orchestration_events.rs`). Earlier drafts of
  this spec assumed such events existed and named them
  `PlanStepStarted` / `PlanStepCompleted` / `PlanStepPaused` /
  `PlanStepResumed`; those events do NOT exist in the current
  contract and this spec does NOT add them. Instead, the start
  timestamp for a plan-step row is **derived client-side** from the
  observable state changes that DO exist:
  - The agent emits `TodoOperation::UpdateTodos { todos }` and
    `TodoOperation::MarkAsCompleted { completed_todos }` (see
    `app/src/ai/agent/mod.rs:1495`). Each todo has a derived status
    of type `TodoStatus` (`Pending`, `InProgress`, `Completed`,
    `Cancelled`, `Stopped`) — see
    `app/src/ai/agent/conversation.rs:80`.
  - The first time the client observes a given `AIAgentTodoId`
    transition to `TodoStatus::InProgress` (or, if no `InProgress`
    transition is ever observed because the agent reports the row as
    already `Completed`, the first observation of any terminal status
    on that id), the client records the local wall-clock time
    (`SystemTime::now()`) as that row's `started_at`. The
    `(AIAgentTodoId → started_at)` map lives in the view-model
    container that already holds `AIAgentTodoList` state and is reset
    when the enclosing message is cleared.
  - This is a **client-side observation timestamp**, not an
    authoritative server timestamp. V1 accepts this trade-off; a
    follow-up may add an authoritative `started_at` field to the
    persisted todo if accuracy across reconnect/replay becomes a
    requirement (tracked in Open Questions).
- **Completion time — derived, not event-driven.** The row's elapsed
  counter freezes and is replaced by the final duration when the
  client observes the todo transition to any terminal status
  (`Completed`, `Cancelled`, or `Stopped`). The completion timestamp
  is `SystemTime::now()` at the moment of that observation. All
  three terminal statuses freeze the counter identically; the row
  label may render status-specific styling, but the timestamp
  affordance is identical across the three.
- **Pending / never-started rows.** A plan-step row that the client
  has only ever observed in `TodoStatus::Pending` (never
  `InProgress` and never any terminal status) has NO recorded
  `started_at` and therefore shows NO timestamp affordance — no
  collapsed timestamp, no expanded `Started …` line. This is the
  ONLY kind of plan-step row in scope that does not carry the
  affordance, and it is intentional: there is no time to render.
- **Editing / re-ordering.** If the agent's `UpdateTodos` operation
  replaces a row's `AIAgentTodoId` (e.g. plan is rewritten), the row
  is treated as new — its `started_at` is recorded the next time it
  is observed as `InProgress`. The retired id's recorded
  `started_at` is dropped. If the same `AIAgentTodoId` is reused
  with a fresh transition `Pending → InProgress`, the existing
  `started_at` is retained (the row was paused, not deleted).
- **Pause-and-resume.** V1 does NOT model cumulative
  active-vs-paused time. The elapsed counter is the wall-clock
  delta between the recorded `started_at` and "now" (or the
  recorded terminal-status time), regardless of whether the row
  briefly returned to `Pending`. This is a deliberate
  simplification consistent with the absence of pause/resume events
  in the current event stream. A future revision MAY add a
  pause-aware computation; until then, the simpler
  `now − started_at` is the contract and is verified by tests.

Any future "agent sub-block" type that exposes an observable status
transition (analogous to `TodoStatus`) should adopt the same
treatment. If a future revision introduces explicit `PlanStepStarted`
/ `PlanStepCompleted` events on the conversation event stream, those
SHOULD replace the client-side derivation — but doing so is out of
scope for this spec and gated on a separate orchestrator change.

### B2. Collapsed-row format

```
▸ <kind>… (<time>)
```

`<time>` is the start timestamp of the sub-block. Default formatting rule
(`auto`):

- RELATIVE if the start was within the last 60 minutes
  (e.g. `just now`, `34s ago`, `2m ago`).
- ABSOLUTE if older
  (rendered per the canonical absolute-time format defined in B2.1).

The `(<time>)` token is rendered as a secondary, lower-contrast label so it
does not visually compete with the kind label.

### B2.1. Canonical absolute-time format

Every absolute timestamp shown by this feature — collapsed row, expanded header,
or `aria-label` — uses the SAME canonical form. Seconds are ALWAYS included so
collapsed and expanded rows are visually consistent.

- **24-hour mode** (`time.use_24h = true`):
  `HH:MM:SS` — zero-padded; locale-independent for the time portion.
  Example: `11:42:07`.
- **12-hour mode** (`time.use_24h = false`):
  `h:MM:SS AM/PM` — hour NOT zero-padded; minutes/seconds zero-padded;
  AM/PM marker per locale (e.g. `AM`/`PM` in en-US; `a.m.`/`p.m.` in
  some locales). Example: `11:42:07 AM`.
- **Older than 24 hours** (in any mode that resolves to absolute):
  prefix with a short date in **ISO-8601 `YYYY-MM-DD`** form,
  joined to the time portion by a single space. V1 deliberately
  uses ISO-8601 — NOT a locale-aware short date — so the rendered
  form is unambiguous in transcripts, logs, and screenshots
  regardless of the user's locale. Example (24h):
  `2026-05-08 11:42:07`. Example (12h): `2026-05-08 11:42:07 AM`.
  This rule supersedes earlier wording that called the date
  "locale-appropriate"; locale governs ONLY the AM/PM marker, not
  the date format. A locale-aware short date may be revisited in
  V1.5.
- The locale governs the AM/PM marker only; the digit/colon
  ordering of the time portion AND the ISO date prefix are fixed
  for parity. The locale is read implicitly inside `format_absolute`
  via the app's existing `chrono::Local` context (the same source
  `time_format.rs` already uses) — the helper does NOT take a
  locale parameter, and there is no per-call locale override in V1.
  This keeps the "locale-aware" surface area small enough that the
  helper signature `format_absolute(started_at, now, prefer_24h)`
  in Implementation Pointers fully describes the input set.

### B2.2. Forced `relative` mode beyond 60 minutes

When the user explicitly sets the format to `"relative"` (NOT `"auto"`), the
relative form must extend past the 60-minute window:

| Age range | Format | Example |
| --- | --- | --- |
| < 5 s | `just now` | `just now` |
| 5 s – < 60 s | `<N>s ago` | `34s ago` |
| 60 s – < 60 min | `<N>m ago` | `2m ago`, `59m ago` |
| 60 min – < 120 min | `<N>m ago` (rounded down) | `60m ago`, `119m ago` |
| 2 h – < 48 h | `<N>h ago` (rounded down) | `2h ago`, `47h ago` |
| 48 h – < 7 d | `<N>d ago` (rounded down) | `2d ago`, `6d ago` |
| ≥ 7 d | `<N>w ago` (rounded down) | `1w ago`, `12w ago` |

All ranges round DOWN to the unit shown (floor division). This applies ONLY
when the user has explicitly chosen `"relative"`. The default `"auto"` mode
continues to switch to absolute past the 60-minute mark per B2.

### B3. Expanded-row header

When the block is expanded, the header shows both an absolute start time and an
elapsed counter / final duration. The same field is reused across the in-progress
and completed states — there is NO separate `running…` placeholder.

- In-progress phase: `Started 11:42:07 · <elapsed>`, where `<elapsed>` is
  the COMPACT in-progress label produced by
  `format_elapsed_label` (Implementation Pointers). Examples while ticking
  at 1 Hz: `1s elapsed`, `2s elapsed`, `3s elapsed`, `4s elapsed`, …
  `12m elapsed`, `1h elapsed`. The output format is the compact contract in
  `format_compact_duration` + the literal suffix `" elapsed"`; it does NOT
  use the verbose `human_readable_precise_duration` form (e.g.
  `"3.14 sec"`).
- Completed phase: `Started 11:42:07 · <duration>` where `<duration>` is
  the COMPACT final duration produced by `format_compact_duration` —
  examples: `4.3s` (sub-10-second precise), `12s`, `2m`, `1h`. At the
  moment the completion observation arrives (per B1.1 derivation for
  plan-step rows; per existing per-phase events for reasoning / tool
  calls), the elapsed value is replaced by the final compact duration
  string and stops updating. This compact form is the exclusive expanded-
  row duration vocabulary used by this feature; `time_format.rs`'s
  verbose helpers are NOT used here.

The expanded view always displays a numeric elapsed/duration value; it never
shows a static `running…` label. The 1 Hz ticker described in B4 is the only
clock driving in-progress updates — no per-row timer.

Absolute time in the expanded header is always rendered using B2.1's canonical
form, regardless of B2's relative/absolute choice for the collapsed row.

### B3.1. Visibility of the expanded-row timestamp

The `agent.subblock_timestamp_show_in_expanded` setting (Settings / API surface)
controls whether the expanded-view header carries the timestamp affordance at
all. Its precedence is defined in B6.

### B4. Live-update cadence

The cadences below are stated as **periods** (one tick every N
seconds), not as Hz frequencies. The "5 Hz subscriber" / "5 Hz" wording
that appeared elsewhere in earlier drafts was incorrect and is
superseded by this section.

- **Relative-timestamp ticker** — fires once every **5 seconds**
  (period = 5 s, frequency = 0.2 Hz). Drives re-render of every visible
  collapsed row whose timestamp is rendered in relative form.
- **In-progress elapsed-counter ticker** — fires once every **1 second**
  (period = 1 s, frequency = 1 Hz). Drives the elapsed counter on the
  single most-recent in-progress phase (B3) and the most-recent
  in-progress relative timestamp.
- Absolute timestamps do not refresh.
- A single coalesced ticker pair (one 5 s + one 1 s) drives all visible
  rows for an entire conversation list; per-row timers are not
  permitted (see Implementation Pointers).
- Implementation Pointers and Tests in this spec MUST refer to these
  cadences as `5 s` and `1 s` respectively. The earlier `5 Hz` /
  `1 Hz + 5 Hz` phrasing is wrong; the correct phrasing is
  "1 s ticker + 5 s ticker" (or "1 Hz + 0.2 Hz" if expressed in
  frequency).

### B5. Locale & 12h/24h

- Use the existing app locale.
- Use the `time.use_24h` setting when present.
- If `time.use_24h` is unset, fall back to the OS preference.

### B6. Setting

`agent.subblock_timestamp_format` controls collapsed-row formatting:

- `"off"` — no timestamp on collapsed or expanded rows.
- `"absolute"` — always absolute time per B2.1.
- `"relative"` — always relative time per B2.2 (extends past 60 min).
- `"auto"` (default) — relative ≤ 60 min, absolute beyond per B2.1.

### B6.1. Precedence: format vs. show_in_expanded

The two settings are orthogonal but interact:

- `agent.subblock_timestamp_format` controls the FORMAT used wherever a
  timestamp is rendered (off / absolute / relative / auto).
- `agent.subblock_timestamp_show_in_expanded` controls VISIBILITY in the
  EXPANDED view ONLY. It has no effect on the collapsed row.

The full **4 × 2 matrix** (four `format` values × two
`show_in_expanded` values; the format-`"off"` row collapses both
`show_in_expanded` cases to identical "hidden" output, but the matrix
is still 4 × 2 in shape):

| `format`     | `show_in_expanded` | Collapsed row timestamp | Expanded header timestamp |
| ------------ | ------------------ | ----------------------- | ------------------------- |
| `"off"`      | `true`  *or* `false` | hidden                  | hidden (the format-`off` rule wins; `show_in_expanded` has no effect) |
| `"absolute"` | `true`             | absolute, per B2.1      | `Started <absolute> · <elapsed>/duration` per B3 |
| `"absolute"` | `false`            | absolute, per B2.1      | hidden (no `Started …` line and no elapsed/duration affordance in the expanded header) |
| `"relative"` | `true`             | relative, per B2.2      | `Started <absolute> · <elapsed>/duration` per B3 (expanded form ALWAYS uses canonical absolute, per B3) |
| `"relative"` | `false`            | relative, per B2.2      | hidden |
| `"auto"`     | `true`             | auto (rel ≤ 60 min, else abs) | `Started <absolute> · <elapsed>/duration` per B3 |
| `"auto"`     | `false`            | auto (rel ≤ 60 min, else abs) | hidden |

When the expanded header timestamp is hidden because of `show_in_expanded =
false`, the entire `Started … · <elapsed>/duration` affordance is suppressed —
NOT just the start time. This is intentional so the expanded view is fully
free of timing chrome when the user opts out.

The format `"off"` rule is the global override: when format is `"off"`,
`show_in_expanded` is ignored and the affordance is hidden in BOTH views.

### B7. Accessibility

- The collapsed row's `aria-label` must include a human-readable timestamp
  description, e.g. `"Thinking phase started 2 minutes ago"` or
  `"Thinking phase started at 11:42:07"`.
- The expanded header's accessible name must include start time plus the
  elapsed counter (in-progress) or final duration (completed) — wording
  matches the visible text and updates on the 1 Hz ticker.
- Hover tooltip (see Open Questions) must not be the only carrier of timing
  information.

## Settings / API surface

| Setting | Type | Default | Notes |
| --- | --- | --- | --- |
| `agent.subblock_timestamp_format` | enum `"off" \| "absolute" \| "relative" \| "auto"` | `"auto"` | Drives FORMAT for reasoning, tool-call, and plan-step sub-blocks. `"off"` is the global override — hides timestamps in BOTH collapsed and expanded views. |
| `agent.subblock_timestamp_show_in_expanded` | bool | `true` | Controls VISIBILITY in the EXPANDED view ONLY. When `false`, the entire `Started … · <elapsed>/duration` affordance is suppressed in the expanded header (collapsed row is unaffected). Has NO effect when `format = "off"`. Full precedence in B6.1. |

UI placement: Settings → Agents → "Agent sub-block timestamps":

- Radio group bound to `agent.subblock_timestamp_format`.
- Checkbox bound to `agent.subblock_timestamp_show_in_expanded`.

The label "Agent sub-block timestamps" reflects the broader scope of this
feature — it covers reasoning phases, tool-call blocks, and plan-step blocks,
not just reasoning. The earlier draft used the narrower
`agent.reasoning_phase_timestamp_format` / "Reasoning phase timestamps" naming;
the renamed form is the canonical version going forward.

No new public API. The values flow through the existing settings store.

## Acceptance Criteria

- A1. With format `"auto"`, a collapsed reasoning row whose start time is
  within the last 60 minutes shows a relative timestamp; older than 60 minutes
  shows an absolute timestamp.
- A2. Expanding a completed phase shows `Started <absolute> · <duration>`
  (e.g. `Started 11:42:07 · 4.3s`). Expanding an in-progress phase shows
  `Started <absolute> · <elapsed> elapsed`, where `<elapsed>` advances on the
  existing 1 Hz ticker (e.g. `1s elapsed`, `2s elapsed`, …) and is replaced by
  the final duration string the moment the completion event arrives.
- A2.1. The expanded view never displays a static `running…` label. The
  elapsed counter is the only in-progress affordance.
- A3. Setting format `"off"` removes timestamps from collapsed and expanded
  rows in BOTH views, regardless of `subblock_timestamp_show_in_expanded`.
- A4. Setting `"absolute"` forces absolute on collapsed rows; `"relative"`
  forces relative.
- A5. 12h vs 24h rendering follows `time.use_24h` (with OS fallback when unset).
- A6. Tool-call blocks and plan-step blocks get the same treatment as reasoning
  phases (collapsed timestamp, expanded start + elapsed/duration per A2).
- A7. Relative timestamps refresh on a 5-second cadence; the most recent
  in-progress phase refreshes on a 1-second cadence and drives the elapsed
  counter described in A2.
- A8. The collapsed row's `aria-label` carries an accurate timing description
  matching the visible format.
- A_show_in_expanded_off_format_on. With format `!= "off"` and
  `subblock_timestamp_show_in_expanded = false`, the collapsed row shows the
  timestamp per its format setting, and the expanded header shows NO
  `Started …` line and NO elapsed/duration affordance.
- A_show_in_expanded_on. With format `!= "off"` and
  `subblock_timestamp_show_in_expanded = true`, both views show timestamps
  per B3.
- A_show_in_expanded_format_off_overrides. With format `"off"`, the value of
  `subblock_timestamp_show_in_expanded` (true OR false) has NO effect:
  timestamps remain hidden in BOTH views.

## Implementation Pointers

> Paths verified against the worktree at spec time. Modules that don't yet
> exist are marked `(new module)` so reviewers can distinguish net-new files
> from edits to existing files.

- Agent block output renderer (collapsed/expanded reasoning header lives here
  today; "Thinking" label and `thinking_display_mode` are wired in this file):
  `app/src/ai/blocklist/block/view_impl/output.rs`.
- Agent block view container:
  `app/src/ai/blocklist/agent_view/agent_view_block.rs`.
- Settings (existing `ThinkingDisplayMode` enum is the closest precedent for
  the new `subblock_timestamp_format` enum and lives alongside other
  AI/agent settings): `app/src/settings/ai.rs`.
- Settings UI placement (Agents page — radio group + checkbox land here):
  `app/src/settings_view/ai_page.rs`.
- Settings migration / initialization (mirror the
  `KeepThinkingExpanded → ThinkingDisplayMode` migration pattern when adding
  the new enum, see lines around 121–155): `app/src/settings/initializer.rs`.
- Conversation event stream / per-phase start & completion events
  (timestamps surfaced here come from this stream, not a new timing source):
  `app/src/ai/blocklist/orchestration_events.rs` and
  `app/src/ai/agent/conversation_yaml.rs`.
- `(new module)` Time-formatting helper:
  `app/src/util/relative_time.rs`. The codebase already has
  `app/src/util/time_format.rs` whose existing helpers
  (`format_approx_duration_from_now`, `human_readable_precise_duration`,
  `human_readable_approx_duration`) produce **verbose** forms like
  `"3.14 sec"`, `"5 min"`, `"2 hours"`, `"2 days ago"`, `"just now"`.
  Those verbose forms are NOT used by this feature directly: the
  collapsed-row, expanded-row, and aria-label strings in B2 / B2.2 / B3
  are COMPACT (`34s ago`, `2m ago`, `2h ago`, `4.3s`, `1s elapsed`).
  The new `relative_time.rs` module is therefore dedicated to this
  feature's compact forms and MUST be the SINGLE shared helper used by
  reasoning, tool-call, and plan-step rendering — no duplicated
  formatters. It is intentionally separate from `time_format.rs`;
  earlier drafts implied the existing `human_readable_precise_duration`
  could be reused directly, which is wrong (its output does not match
  the compact contract in B2.2 / B3 — e.g. `human_readable_precise_duration`
  returns `"4.3 sec"`, whereas this feature renders `4.3s`). Module
  exposes the following exact signatures and output contracts:

  - `fn format_relative_auto(now: SystemTime, started_at: SystemTime) -> RelativeOrAbsolute`
    — implements B2's auto rule. Returns the
    `RelativeOrAbsolute::Relative(String)` variant with one of the
    compact forms below when `now - started_at < 60 min`; returns
    `RelativeOrAbsolute::Absolute` (a marker — the caller then invokes
    `format_absolute`) when `now - started_at >= 60 min`.
    The compact relative forms returned (exactly these strings, no
    pluralization, no spaces between number and unit):
    `"just now"` (< 5 s); `"<N>s ago"` (5 s – < 60 s); `"<N>m ago"`
    (60 s – < 60 min).
  - `fn format_relative_extended(now: SystemTime, started_at: SystemTime) -> String`
    — implements B2.2. Returns the compact forms `"<N>m ago"`,
    `"<N>h ago"`, `"<N>d ago"`, `"<N>w ago"` per the B2.2 range table.
    Floor division, no pluralization, no spaces between number and
    unit. Examples: `"60m ago"`, `"119m ago"`, `"2h ago"`,
    `"47h ago"`, `"2d ago"`, `"1w ago"`, `"12w ago"`.
  - `fn format_absolute(started_at: SystemTime, now: SystemTime, prefer_24h: bool) -> String`
    — implements B2.1. The clock is INJECTED via `now` so the
    date-prefix branch (`now - started_at > 24 h`) is fully
    deterministic and unit-testable without depending on the system
    clock. Behavior:
    - When `now - started_at <= 24 h`: time-only form per B2.1
      (`"11:42:07"` or `"11:42:07 AM"`).
    - When `now - started_at > 24 h`: date-prefixed form
      (`"2026-05-08 11:42:07"` or `"2026-05-08 11:42:07 AM"`).
    - Locale governs ONLY the AM/PM marker in 12-hour mode (per the
      app's existing chrono locale handling, the same source
      `time_format.rs` uses for `DateTime<Local>`); the digit/colon
      ordering of the time portion and the ISO date prefix are fixed
      regardless of locale. This is the entire surface area of
      "locale-aware" behavior in V1 — earlier wording implying the
      helper takes a locale parameter is superseded; the only
      locale-sensitive output is the AM/PM marker, which is obtained
      from the active `chrono::Local` formatting context inside the
      helper, NOT a separate parameter.
  - `fn format_compact_duration(duration: Duration) -> String`
    — implements the expanded-row compact duration shown in B3 (e.g.
    `"4.3s"`, `"1s"`, `"2s"`, `"3s"`, `"12m"`, `"1h"`). Output
    contract: durations < 60 s render as `"<N>s"` for integer seconds
    or `"<N.M>s"` (one decimal) for sub-10-second precise final
    durations like `4.3s`; durations 60 s – < 60 min render as
    `"<N>m"`; durations ≥ 60 min render as `"<N>h"`. No spaces, no
    pluralization, no "sec"/"min"/"hours" expansions — this is the
    explicit divergence from `human_readable_precise_duration` and
    the reason this helper exists.
  - `fn format_elapsed_label(duration: Duration) -> String`
    — wraps `format_compact_duration` for the in-progress label per
    B3: appends `" elapsed"` (e.g. `"1s elapsed"`, `"2s elapsed"`,
    `"12m elapsed"`). The 1 s ticker invokes THIS helper on every
    tick; it does NOT invoke `human_readable_precise_duration`
    (earlier drafts said it did — that was wrong, since the verbose
    output `"3.14 sec"` does not match the compact `"3s elapsed"`
    contract).
  - `(internal)` `fn format_date_prefix(date: SystemTime) -> String`
    used by `format_absolute` only. V1 always returns the ISO-8601
    short date `YYYY-MM-DD` (locale-independent). The locale parameter
    is intentionally NOT taken — V1 does not use a locale-aware short
    date per B2.1. A locale-aware variant may be added in V1.5 behind
    a separate setting; until then, ISO-8601 is the single rendered
    form.
- A SINGLE shared helper rule: all sub-block renderers (reasoning,
  tool-call, plan-step) call into `relative_time.rs` for the
  collapsed-row compact form (`format_relative_auto` /
  `format_relative_extended`), the absolute time
  (`format_absolute`), AND the expanded-row in-progress / completed
  duration (`format_compact_duration` /
  `format_elapsed_label`). No renderer hand-rolls its own time string,
  and no renderer reaches into `time_format.rs`'s verbose helpers for
  the affordance defined by this spec. The existing
  `time_format.rs` helpers remain in use ONLY for their pre-existing
  call sites (message-level relative times, etc.) and are unchanged
  by this spec.
- `(new module or co-located in agent_view)` Coalesced ticker for
  live-update cadence — a single PAIR of subscribers per conversation
  list: ONE with a 1-second period (the "1 s ticker") and ONE with a
  5-second period (the "5 s ticker"). Frequencies in Hz: 1 Hz and
  0.2 Hz respectively. Earlier wording referring to a "5 Hz
  subscriber" was wrong; that is 5 fires per second, not the desired
  cadence. Suggested location:
  `app/src/ai/blocklist/agent_view/timestamp_ticker.rs`. The 1 s
  ticker drives BOTH the most-recent in-progress relative timestamp
  AND the expanded-view elapsed counter (B3). The 5 s ticker drives
  re-render of every other visible relative timestamp. No per-row
  timers.
- Tool-call and plan-step rendering: tool calls flow through the same agent
  output renderer above (`view_impl/output.rs`). The transcript todo / plan-
  step UI is rendered in
  **`app/src/ai/blocklist/block/view_impl/todos.rs`** — this is the file
  that draws the per-row TODO entries inside an agent message block and is
  where the per-row timestamp affordance is wired. The prompt/context chip
  rendered by `app/src/ai/blocklist/prompt/plan_and_todo_list.rs` is OUT
  OF SCOPE for this feature and is NOT modified. Earlier drafts of this
  spec pointed at `plan_and_todo_list.rs`; that pointer was wrong (it
  targets the prompt chip, not the transcript todo rows) and is superseded
  by this entry. The renderer in `todos.rs` reads the resolved
  `subblock_timestamp_format` via the existing settings context.
- Plan-step start time is **derived client-side** from `TodoStatus`
  transitions observed on `TodoOperation::UpdateTodos` /
  `MarkAsCompleted` (see B1.1). The view-model container holding
  `AIAgentTodoList` gains a sibling `HashMap<AIAgentTodoId, SystemTime>`
  (call it `todo_started_at`) and a `HashMap<AIAgentTodoId, SystemTime>`
  for terminal-status observation time (`todo_finished_at`). Both maps
  are populated by an observer registered against the existing
  `TodoOperation` stream; no new event types are added to
  `orchestration_events.rs`. The observer runs alongside the existing
  todos-update consumer in `todos.rs`.
- Reasoning-phase and tool-call sub-blocks continue to read their start
  times from the existing per-phase start / completion events on the
  conversation event stream (`orchestration_events.rs` and
  `conversation_yaml.rs`). Do not introduce a parallel timing source for
  those sub-block types.

## Tests

- T1. `format_relative_auto` returns `just now` < 5 s, `Ns ago` < 60 s,
  `Nm ago` < 60 min.
- T2. `format_relative_auto` signals fallthrough to `format_absolute` past 60
  minutes; `auto` adapter renders the canonical absolute form per B2.1.
- T3. Expanded view shows the final duration string (e.g. `4.3s`) once a phase
  emits a completion event, replacing the prior elapsed counter at the exact
  tick the completion arrives.
- T4. In-progress phase shows the elapsed counter produced by
  `format_elapsed_label` (`1s elapsed`, `2s elapsed`, …) and the value
  advances on each 1 Hz tick of the ticker. Test asserts the text never
  reads `running…` — the elapsed counter is the only in-progress
  affordance. Test also asserts the rendered string matches the compact
  `format_compact_duration` contract (`"3s elapsed"`, NOT
  `"3.00 sec elapsed"`); the verbose `human_readable_precise_duration`
  output is explicitly disallowed in this code path.
- T4.1. Plan-step row derived start time (per B1.1). Feed a synthetic
  `TodoOperation::UpdateTodos` stream into the view-model with a single
  todo transitioning `Pending → InProgress → Completed`. Assert: (a)
  `todo_started_at[id]` is recorded the moment `InProgress` is
  observed; (b) `todo_finished_at[id]` is recorded the moment a
  terminal status is observed; (c) the elapsed counter renders only
  while `InProgress`; (d) a todo that is observed only in `Pending`
  has neither a collapsed-row timestamp nor an expanded `Started …`
  line. Test also verifies the spec does NOT depend on any
  `PlanStepStarted` / `PlanStepCompleted` event types — those are not
  added to `orchestration_events.rs`.
- T5. Setting format `"off"` removes the timestamp DOM/aria-label entirely
  from BOTH collapsed and expanded views, regardless of `show_in_expanded`.
- T6. Setting `"absolute"` and `"relative"` force the corresponding format
  regardless of age. Forced `"relative"` exercises B2.2 ranges:
  `60m ago`, `2h ago`, `2d ago`, `1w ago`.
- T7. Absolute form rendering per B2.1 with `format_absolute(start, now,
  prefer_24h)`: `prefer_24h = true` renders `11:42:07`; `prefer_24h = false`
  renders `11:42:07 AM`. With `now - start > 24 h` the helper prefixes the
  date (V1 fallback `2026-05-08`, e.g. `2026-05-08 11:42:07` /
  `2026-05-08 11:42:07 AM`). The clock-injection contract is exercised by
  passing two synthetic `now` values across the 24 h boundary in the same test
  to confirm no system-clock dependency.
- T7.1. The shared-helper rule is enforced: a single `format_absolute` call
  site is used by reasoning, tool-call, and plan-step rendering. Test asserts
  identical output for the same `(start, now, prefer_24h)` across all three
  call sites.
- T8. Tool-call block and plan-step block receive identical treatment per
  B1–B3.
- T9. Stepping the mock clock by 5 s causes all visible relative-formatted
  rows to re-render with the new value.
- T10. `aria-label` audit for collapsed and expanded rows matches the visible
  text, including the canonical absolute form when applicable. Expanded-view
  aria-label MUST include the elapsed counter while in-progress (e.g. "Started
  at 11:42:07, 4 seconds elapsed") and the final duration once complete.
- T11. Timer coalescing: rendering 50 sub-block rows registers exactly one
  1-second ticker subscriber and one 5-second ticker subscriber against
  the coalesced ticker (NOT a 5 Hz / 5-fires-per-second subscriber —
  the second cadence is one fire every 5 seconds, i.e. 0.2 Hz).
- T_show_in_expanded_matrix. The 4 × 2 matrix in B6.1 is exercised end-to-end
  per cell: for each combination of `format ∈ {"off", "absolute", "relative",
  "auto"}` × `show_in_expanded ∈ {true, false}` (8 cells total), the test
  asserts the exact collapsed-row and expanded-header rendering described in
  B6.1's table. This includes the format-`"off"` override (both
  `show_in_expanded` values yield identical hidden output).

## Open Questions

- Should hovering a collapsed row show a tooltip with the OTHER format (e.g.
  hover relative-shown row to see absolute time)? Suggested answer: yes for
  V1 — purely additive, supports both quick scanning and precise audit. Tooltip
  must echo, not replace, the timing info already exposed via `aria-label`.
- Should the expanded view also support a "show full ISO timestamp" affordance
  (right-click / context menu), for users exporting transcripts?

## Telemetry

No new events. The existing per-phase start/complete events already carry the
timing data this spec surfaces. If usage signals are required later, add a
single `agent.subblock_timestamp.format_changed` event tied to the
setting toggle (out of scope for V1).
