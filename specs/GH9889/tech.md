# Technical spec: Per-message timestamps in Agent Mode (GH-9889)

This spec is the implementation companion to `product.md`. It picks
the data-source path, the UI integration site, the timer strategy,
and the testable invariants.

## What already exists

> **Correction (review #10128):** earlier drafts of this spec named
> `start_ts` / `completed_ts` fields on `AIAgentExchange` at
> `conversation.rs:3218-3219`. Those line numbers are unrelated
> serialized-block fields; the actual exchange struct lives in
> `app/src/ai/agent/mod.rs:2835` and uses different field names.
> The correct facts are below.

- `AIAgentExchange` is defined at
  [app/src/ai/agent/mod.rs:2835](app/src/ai/agent/mod.rs) with fields:
  - `pub start_time: DateTime<Local>` (line 2849, always present —
    set when the input is sent)
  - `pub finish_time: Option<DateTime<Local>>` (line 2852 — populated
    when the exchange's output finishes streaming)
  - `pub time_to_first_token_ms: Option<i64>` (line 2855 — TTFT for
    the exchange; relevant to the duration display, see B3)
- Helpers on `Conversation` recompute these from message timestamps
  when needed (used during restoration / late-binding):
  - `Conversation::start_time_from_exchange_messages` derives the
    start time from the latest input's `AIAgentContext::CurrentTime`
    ([conversation.rs:556](app/src/ai/agent/conversation.rs)).
  - `Conversation::finish_time_from_exchange_messages` derives the
    finish time from the latest message timestamp on the exchange
    ([conversation.rs:536](app/src/ai/agent/conversation.rs)).
  - The recompute call sites at conversation.rs:1776, 1894, 1961
    write back into `exchange.start_time` / `exchange.finish_time`
    on restoration paths.
- `Conversation::start_ts` ([conversation.rs:920](app/src/ai/agent/conversation.rs))
  exposes the conversation-wide start by reading the earliest
  `exchange.start_time`.
- `chrono::Local` is the existing convention for derived times.
- Conversation restoration replays the underlying message timestamps
  through the helpers above, so restored conversations populate the
  same `start_time` / `finish_time` fields as live ones (A6 is
  automatic if the view re-derives on render).

## What does not exist

- No view-layer surface for these timestamps. Every consumer today
  uses them for ordering, status, or export — never for display
  next to a message bubble.
- No relative-time formatter. The codebase uses
  `Local.timestamp_opt(...)` for raw `DateTime<Local>` and then
  defers to whatever the consumer needs.
- No 1Hz tick infrastructure for live duration counters in the
  agent view. The block-status header has its own independent
  refresh path; we should not graft onto that.

## Data path: derive at render time, not store on the view model

The agent view renders from the `Conversation` model. Add a pure
helper (no new state) that reads the existing exchange fields:

```rust
// In a new module: app/src/ai/blocklist/block/view_impl/exchange_times.rs

pub struct ExchangeTimes {
    pub submitted_at: DateTime<Local>,           // exchange.start_time (always present)
    pub completed_at: Option<DateTime<Local>>,   // exchange.finish_time; None == in progress
    pub cancelled_at: Option<DateTime<Local>>,   // None unless cancelled
}

pub fn exchange_times(
    conversation: &Conversation,
    exchange: &AIAgentExchange,
) -> ExchangeTimes { ... }
```

The helper reads `exchange.start_time` and `exchange.finish_time`
directly. Cancellation is detected via the exchange's
`output_status: AIAgentOutputStatus` (mod.rs:2843) — when the status
matches the cancelled discriminant, treat `finish_time` as the
cancellation time and surface it in `cancelled_at`. No fallback
recomputation is needed in the render path; the conversation model
already keeps `start_time` / `finish_time` consistent on restore via
the recompute call sites at conversation.rs:1776/1894/1961.

Why a fresh helper module instead of methods on `Conversation`:
- `Conversation` is already large.
- The helper has zero callers outside the agent view; co-locating
  it next to the consumer reduces blast radius.

## UI integration site

> **Correction (review #10128):** earlier drafts pointed this section at
> `agent_view/agent_message_bar.rs`. That file renders the bottom
> input/status bar, not the conversation prompt/response bubbles.
> The correct render sites are below.

Per product.md §risk 1 recommendation: **inline in the existing
message-bubble metadata row**.

The conversation's per-exchange UI is rendered by the block view-impls
under [`app/src/ai/blocklist/block/view_impl/`](app/src/ai/blocklist/block/view_impl/):

- **Prompt bubble (user input):**
  [`view_impl/query.rs::render_query`](app/src/ai/blocklist/block/view_impl/query.rs)
  is the entry point.

  > **Correction (re-review #10128):** earlier drafts claimed the
  > prompt path already receives an `AIAgentExchange` (or its id).
  > It does not. `render_query` takes primitive parameters
  > (`query: &str`, `user_display_name: &str`, attachments,
  > redaction state, etc.) via the `Props` struct above
  > `maybe_render`. The submitted-at timestamp must be plumbed
  > through.

  Plumbing: extend the `Props` struct in `view_impl/query.rs` with
  a `submitted_at: DateTime<Local>` field. Add the same field to
  the `Props` builder at the call site in `view_impl/mod.rs` (or
  wherever the props are constructed). Source the timestamp from
  the surrounding block's exchange the same way the `query: &str`
  is sourced today — the parent block has `AIAgentExchangeId`
  context and can read `exchange.start_time` from the model. This
  is one new field on each of two structs and one new call-site
  argument; no new layer.

  Inside `render_query`, append a `TimestampLabel` widget bound to
  `ExchangeTimes::submitted_at` next to the existing query metadata
  (avatar, attachments). The shared text helper
  `view_impl/common.rs::render_query_text` lays out the user's
  prompt text and is unchanged.

- **Response bubble (agent output):**
  [`view_impl/output.rs`](app/src/ai/blocklist/block/view_impl/output.rs)
  renders the streaming/finished response. The Props/render entry
  point in this file already has access to the per-exchange model
  (used to drive streaming state), so plumbing `ExchangeTimes` here
  is a one-field addition to the same Props pattern (no new layer).
  Append a `TimestampLabel` bound to `ExchangeTimes::completed_at`
  and a `DurationLabel` bound to `(submitted_at, completed_at)`. If
  `completed_at` is `None`, render a live `ProgressDurationLabel`
  bound to `submitted_at` plus the shared 1Hz tick instead.

The bottom-bar `agent_view/agent_message_bar.rs` is not modified.

Three new widgets:
- `TimestampLabel` — relative-or-absolute renderer, refreshes via
  the shared 30s tick.
- `DurationLabel` — `submitted_at → completed_at` formatter.
- `ProgressDurationLabel` — live "running for Xs" counter, refreshes
  via the shared 1Hz tick.

All three live in `app/src/ai/blocklist/block/view_impl/timestamp_widgets.rs`
(co-located with the consumers).

## Timer strategy

Per product.md §risk 2 recommendation: **single shared 1Hz timer,
gated to visible exchanges with relative-format or progress labels**.

> **Correction (re-review #10128):** the previous "gated to visible
> in-progress exchanges" wording would have stopped 30s relative-
> timestamp refreshes after every exchange completed, breaking
> B2/A8's auto-promote contract for completed labels. The corrected
> gate is: tick while ANY visible label needs refresh — that's
> in-progress (1Hz needed) OR completed-with-relative-label (30s
> needed). When all visible labels are absolute-format (>=24h ago,
> rendered as "HH:MM" or "YYYY-MM-DD HH:MM"), the timer pauses
> until visibility, scroll, or new exchange brings a relative-
> format or in-progress label back into view.

- Add a `TimestampTickService` registered on the agent view's owning
  context.
- It holds a `Vec<Weak<RefCell<dyn Tick>>>` of subscribers
  (the `ProgressDurationLabel` instances and the 30s `TimestampLabel`
  refresh).
- Two tick frequencies: 1Hz for progress labels, 30s for relative
  labels. Use a single underlying timer firing at 1Hz and
  internally rate-limit the 30s consumers.
- Recompute the "needs ticking" predicate on each:
  `AgentViewControllerEvent::EnteredAgentView`,
  `AgentViewControllerEvent::ExitedAgentView`, scroll event, and
  exchange-state transition. These are concrete model events from
  `app/src/ai/blocklist/agent_view/controller.rs`; existing views
  already subscribe to them with `ctx.subscribe_to_model(...)` (see
  `agent_message_bar.rs` and `block/status_bar.rs`).
- Start/stop the underlying `SpawnedFutureHandle` with the same
  `ViewContext::spawn` + `Timer::after(Duration::from_secs(1))`
  pattern used by `BlocklistAIStatusBar::start_last_read_timer` in
  `app/src/ai/blocklist/block/status_bar.rs`. No new WarpUI view
  lifecycle hook is required.
- The "visible" predicate is:
  `ActiveAgentViewsModel::is_conversation_open(conversation_id, ctx)`
  AND at least one visible timestamp widget for that conversation
  needs refresh. `is_conversation_open` is already the repo's
  concrete "expanded agent view in some pane" check.
- Stops ticking when `ExitedAgentView` makes the conversation no
  longer open, or when all visible labels are absolute-format and no
  progress labels remain. Resumes on `EnteredAgentView`, scroll, or
  a new exchange-state transition that makes a refresh-needed label
  visible again.

Why not per-widget timers:
- A long conversation can have 50+ visible exchanges. 50 separate
  per-widget timers would dwarf the cost of one shared timer.
- A shared timer makes "pause when hidden" a one-line toggle
  instead of a per-widget cleanup dance.

## Settings entry

Add a new boolean setting:

```rust
// In app/src/settings/ai.rs near the existing voice_input_toggle_key

settings::macros::implement_setting!(
    show_message_timestamps: bool,
    AISettings,
    SupportedPlatforms::DESKTOP,
    SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "agents.show_message_timestamps",
    description: "Show submitted-at and completion timestamps next to each Agent Mode exchange.",
    default_value: true,
);
```

> **Correction (re-review #10128):** the previous draft cited
> `SyncToCloud::Always`, which doesn't exist. The actual variants
> in `app/src/settings/cloud_preferences.rs` are `Never`,
> `Globally(RespectUserSyncSetting)`, and `PerPlatform(...)`. The
> right value here is `Globally(RespectUserSyncSetting::Yes)` —
> the same value used by every UI preference in
> `app/src/settings/code.rs:19/29/40/48` (the four flagship UI
> settings). It syncs across devices when the user has cloud sync
> on and is local-only when they have it off.

The agent view reads this setting on render; when false, none of
the three widgets are added to the bubble layout (so there is no
layout cost in the off case — A5 invariant).

## Format auto-promotion

`TimestampLabel` re-renders on every 30s tick AND on visibility
regain. A pure function `format_relative_or_absolute(...)` implements
B2.

> **Correction (re-review #10128):** the previous draft proposed
> using `chrono::Locale` and `format_localized`, but the workspace
> chrono dependency in `Cargo.toml` is `chrono = { version =
> "0.4.38", features = ["serde"] }` — the `unstable-locales` feature
> needed by `format_localized` is **not enabled** and would be a
> non-trivial addition (it's still gated behind chrono's
> `unstable-` flag). The corrected design below avoids
> `format_localized` entirely.

```rust
struct ClockFormat {
    /// True if the user's locale uses 24-hour time, false for 12-hour.
    twenty_four_hour: bool,
}

fn format_relative_or_absolute(
    ts: DateTime<Local>,
    now: DateTime<Local>,
    fmt: ClockFormat,
) -> String {
    let delta = now.signed_duration_since(ts);
    let time_fmt = if fmt.twenty_four_hour { "%H:%M" } else { "%-l:%M %p" };
    match delta {
        d if d < ChronoDuration::minutes(1) => "just now".into(),
        d if d < ChronoDuration::hours(1) => format!("{}m ago", d.num_minutes()),
        d if d < ChronoDuration::days(1) => ts.format(time_fmt).to_string(),
        d if d < ChronoDuration::days(7) => {
            // English weekday + locale-preferred 12/24h time. chrono's
            // default `%a` returns English short-form ("Mon", "Tue"); we
            // intentionally do not localize the day name.
            ts.format(&format!("%a {time_fmt}")).to_string()
        }
        _ => ts.format("%Y-%m-%d %H:%M").to_string(), // ISO-style for >=7d
    }
}
```

V1 weekday names are **English-only** because that's what default
chrono provides without a feature-flag bump. The 12h-vs-24h preference
is read from the OS — that's the more impactful localization knob,
and it works without `unstable-locales`. Localized weekday names
(German "Mo./Di./Mi.", etc.) are an explicit V2 follow-up that
requires either:
1. Enabling `chrono`'s `unstable-locales` feature workspace-wide
   (changes the dependency contract; needs maintainer sign-off), OR
2. A small lookup table for short weekday names in the languages
   Warp's UI strings already cover.

`ClockFormat` is sourced from the OS once at startup and cached.
Determination logic:

- **macOS:** `NSLocale.currentLocale` reports the 24h preference via
  `localizedString(for: .timeFormat)` parsing.
- **Windows:** read `LOCALE_ITIME` via the existing `windows-rs`
  bindings used elsewhere in Warp.
- **Linux:** consult `LC_TIME`/`LANG` for the locale; map common
  locales to 24h-vs-12h via a small lookup (`en_US` → 12h, others →
  24h is a reasonable default).
- **Fallback:** if any source is unavailable, default to 24h
  (unambiguous).

The formatter's behavior is exhaustively tested across both
`twenty_four_hour: true` and `false` (T8 and T9 in the test plan
are duplicated for each clock setting).

The leading-zero variants `%-l` (POSIX) vs `%#l` (Windows) are
handled by a small `cfg!(windows)`-gated helper that picks the
correct format string.

## Tooltip

Hover tooltip uses an existing tooltip widget; pass it the ISO 8601
string `ts.format("%Y-%m-%d %H:%M:%S %:z").to_string()`.

## Missing-timestamp fallback

> **Correction (re-review #10128):** the previous draft kept a
> "render dash if `finish_time: None` on a finished exchange" path,
> but the model's recompute call sites at
> [conversation.rs:1777](app/src/ai/agent/conversation.rs),
> [1895](app/src/ai/agent/conversation.rs), and
> [1962](app/src/ai/agent/conversation.rs) all do
> `finish_time_from_exchange_messages(...).unwrap_or_else(Local::now)`
> — i.e., the model **synthesizes** `Local::now()` when the
> derivation fails. So `exchange.finish_time` is never `None` on a
> finished exchange in practice; the dash branch never fires. The
> corrected behavior below makes that explicit.

`exchange.start_time` is `DateTime<Local>` (not `Option<>`), so
`submitted_at` is always present.

`exchange.finish_time` is `Option<DateTime<Local>>`, but per the
recompute paths above, it is:
- `None` exactly while the exchange is in progress.
- `Some(...)` once the exchange transitions out of in-progress —
  including `Some(Local::now())` synthesized by the model for
  restored conversations whose stored timestamps were lost. That
  synthesized value is treated as the model's completion time in V1;
  the view does not try to detect or reinterpret it.

This means the only real branch is **in-progress vs not**. The
render path:
- In-progress (`completed_at: None`): render
  `ProgressDurationLabel` ("running for Xs"). No dash.
- Not in-progress (`completed_at: Some(t)`): render `t` directly via
  `TimestampLabel` and `DurationLabel`. No dash.

The dash glyph is only used in two diagnostic-only cases that should
not happen in production:
1. **Defensive guard:** if `output_status` reports finished but
   `finish_time` is somehow `None` (state machine bug), render "—"
   and emit `log::warn!(exchange_id = ?id, "model invariant: finished
   exchange has finish_time: None")`. Tracked via a
   `HashSet<AIAgentExchangeId>` on the `TimestampTickService` so the
   warn fires at most once per exchange.
2. **Explicit unset start_time at construction:** can't happen with
   the current schema (the field is non-`Option<>`), but if a future
   refactor changes that, the same defensive pattern applies.

Both are debug guard rails — they cost nothing in the happy path
and protect against silent rendering of garbage if the model
invariants ever drift.

## Cancellation handling (B8)

> **Correction (re-review #10128):** the previous draft had two
> separate slots — `TimestampLabel` (showing `completed_at`) and
> `DurationLabel` (showing the duration, swapped to "cancelled at
> HH:MM • Xs" when cancelled). That double-displays the
> completion time once the cancellation slot also renders the time.
> Resolved by routing the entire response-bubble metadata through
> a single composite renderer.

The `ExchangeTimes` struct exposes
`cancelled_at: Option<DateTime<Local>>`. The response bubble
renders **one** of three mutually-exclusive shapes per the
exchange state:

- **In progress** (`completed_at: None`, `cancelled_at: None`):
  `ProgressDurationLabel` only. No timestamp slot.
- **Cancelled** (`cancelled_at: Some(t)`): a single composite
  label "cancelled at HH:MM • Xs". `TimestampLabel` is NOT
  rendered separately — the composite owns both the time and
  the duration so they cannot duplicate.
- **Finished** (`completed_at: Some(t)`, `cancelled_at: None`):
  `TimestampLabel` shows the completion time; `DurationLabel`
  shows the duration. Two slots, no overlap with the cancelled
  shape.

This makes the three shapes structurally distinct rather than
"swap a string in one slot and hope the other slot is hidden,"
removing the duplication path.

## Test plan

### Unit tests (`app/src/ai/blocklist/block/view_impl/exchange_times_test.rs` — new)

- T1: `exchange_times` returns `submitted_at == exchange.start_time`.
- T2: `exchange_times` returns `completed_at == exchange.finish_time`
  for a finished exchange.
- T3: In-progress exchange returns `completed_at: None`.
- T4: Cancelled exchange returns `cancelled_at: Some(...)`.
- T5: Exchange with `output_status == finished` AND `finish_time:
  None` returns `completed_at: None` (the inconsistent-state path).

### Unit tests (`app/src/ai/blocklist/block/view_impl/timestamp_widgets_test.rs` — new)

Tests use a fixed `now` (e.g. 2026-05-05 15:47:23) and exercise both
`twenty_four_hour: true` and `twenty_four_hour: false`:

- T6: `format_relative_or_absolute(now - 30s, now, _)` returns
  `"just now"` regardless of locale.
- T7: `format_relative_or_absolute(now - 5min, now, _)` returns
  `"5m ago"` regardless of locale.
- T8: `format_relative_or_absolute(now - 3h, now, fmt_12h)` returns
  `"12:47 PM"`.
- T8b: Same input with `fmt_24h` returns `"12:47"`.
- T9: `format_relative_or_absolute(now - 3d, now, fmt_12h)` returns
  `"Sun 12:47 PM"`.
- T9b: Same input with `fmt_24h` returns `"Sun 12:47"` (English
  weekday name in both — V1 does not localize weekdays; see the
  format-auto-promotion section's V2 follow-up note).
- T10: `format_relative_or_absolute(now - 30d, now, _)` returns
  `"2026-04-05 15:47"` regardless of clock format (ISO-style for
  >=7d bucket; deliberately format-independent for unambiguous
  timestamps).
- T11: Duration formatter cases (B3): "<1s", "Xs", "Xm Ys", "Xh Ym".

### Integration tests (`app/src/integration_testing/agent_mode/timestamps_test.rs` — new)

- IT1: Submit a prompt; the user bubble immediately shows
  "just now" next to it.
- IT2: While the agent responds, the response bubble shows
  "running for Xs" with X incrementing once per second.
- IT3: On agent completion, the counter is replaced with
  "HH:MM • Xs".
- IT4: Hover the timestamp; tooltip shows the ISO 8601 form.
- IT5: With `agents.show_message_timestamps = false`, none of the
  widgets render and the bubble layout is identical to the current
  build (snapshot test).
- IT6: Restore a yaml conversation with messages dated yesterday;
  exchanges show "Mon 3:47 PM" not "just now."
- IT7: An exchange with `output_status = Finished` and
  `finish_time: None` (forced via test fixture, since the model's
  recompute paths normally synthesize `Local::now()`) renders "—"
  in the duration slot and emits a single `log::warn!`. This is
  the defensive-guard path from B7; it does NOT fire on normal
  restored conversations whose timestamps were synthesized.
- IT8: After 5 minutes (advance the clock in test), a "just now"
  exchange auto-promotes to "5m ago" without user action.
- IT9: Cancel a running exchange; the response bubble shows the
  single composite label "cancelled at HH:MM • Xs" and does not
  render a separate completed timestamp.

## Files touched

- `app/src/settings/ai.rs` — new `show_message_timestamps` setting.
- `app/src/ai/blocklist/block/view_impl/exchange_times.rs` (new) —
  data-derivation helper reading `exchange.start_time`,
  `exchange.finish_time`, and `exchange.output_status`.
- `app/src/ai/blocklist/block/view_impl/timestamp_widgets.rs` (new)
  — three label widgets and the `format_relative_or_absolute`
  formatter with `ClockFormat`.
- `app/src/ai/blocklist/block/view_impl/timestamp_tick_service.rs`
  (new) — single shared 1Hz timer that drives both 1Hz and 30s
  consumers.
- `app/src/ai/blocklist/block/view_impl/query.rs` — append
  `TimestampLabel` to the user-prompt bubble's metadata row inside
  `render_query`.
- `app/src/ai/blocklist/block/view_impl/output.rs` — append
  `TimestampLabel` + `DurationLabel` (or `ProgressDurationLabel`
  for in-progress) to the response bubble's metadata row.
- `app/src/ai/blocklist/block/view_impl/exchange_times_test.rs` (new)
  — T1–T5.
- `app/src/ai/blocklist/block/view_impl/timestamp_widgets_test.rs`
  (new) — T6, T7, T8, T8b, T9, T9b, T10, T11.
- `app/src/integration_testing/agent_mode/timestamps_test.rs` (new)
  — IT1–IT9.

This spec does NOT modify
`app/src/ai/blocklist/agent_view/agent_message_bar.rs` — that file
renders the bottom input bar and is unrelated to the per-exchange
prompt/response rendering this spec targets.

## Out-of-scope follow-ups

- Per-token / per-tool-call timestamps.
- Cumulative agent CPU time / credits per exchange (overlaps
  #10000, #10052).
- Time-zone selection (use `Local`). Note: 24h vs 12h preference is IN
  scope per B2 and the formatter section — read from OS at startup,
  no user-facing setting in V1.
- Exporting timestamps to the conversation yaml view.
- CLI agent (third-party) conversation timestamps — same
  `AIAgentExchange` model could be reused but the CLI agent surface
  has its own bubble layout.

## Open questions for maintainer review

1. Default: `true` (opt-out) or `false` (opt-in)? Spec
   recommends `true`; this is the most reversible decision.
2. Inside-bubble (recommended) vs. gutter rendering. Defers to
   design.
3. Tick service lifetime: registered on the conversation view or
   on a higher-level singleton? Per-conversation if conversations
   can render in multiple places; singleton if not.
4. ISO 8601 tooltip vs. a localized "Tuesday, May 4, 2026 at
   3:47:23 PM CDT" form. ISO is unambiguous; localized is friendlier.
