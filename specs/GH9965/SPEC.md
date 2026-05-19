# Spec: Persistent 7-day USD spend status bar (GH-9965)

## Problem

AI spend visibility is split across two surfaces today: a per-turn
credit chip on the latest agent response, and the
Settings → Billing & Usage dashboard (several clicks away from a
normal terminal/agent workflow). Users have no glanceable signal
for "where am I in my budget over the last week?"

## Goal

Add an opt-in persistent indicator at the bottom-right of the Warp
window showing the user's running 7-day rolling USD spend on Agent
Mode. Setting controls visibility; default off (don't push spend in
users' faces unless they want it).

User-facing label: **"7-day spend"** (rolling 7-day window). All UI
strings in the spec use "7-day" rather than "weekly" to make the
window unambiguous and avoid implying a calendar week boundary.

## Billing scope

The indicator reflects the active billing context for the current
window/workspace.

- For users with a single billing context (personal only, or a single
  team/workspace), the indicator shows that context's spend with no
  scope label necessary in the chip itself.
- For users with multiple contexts (e.g., personal + one or more
  team/workspace billing accounts), the indicator follows the
  workspace selector — the same selector that drives which workspace
  the agent runs against.
- **Scope-change behavior (security-relevant — see "Cross-scope leak
  prevention" below):** the moment the workspace selector changes
  scope, the chip's *displayed* dollar value is cleared synchronously
  in the same UI commit that updates the workspace selector itself,
  before the next paint. The chip immediately enters a "loading"
  state (no dollar amount visible — see B6.1 below for the rendered
  placeholder), and an immediate scoped refresh is dispatched. The
  60 s refresh tick is NOT relied upon to clear the previous value.
  This guarantees no dollar amount from the previous scope is ever
  rendered while the workspace selector reads the new scope.
- The hover tooltip always names the active scope explicitly
  (e.g., "Personal" or "Acme Workspace") so users with multiple
  contexts can confirm what the dollar figure represents. During the
  loading state the tooltip's scope name is the **new** scope (never
  the old one).
- When no billing context is resolvable (signed out, workspace
  switching mid-flight, or BYOK without credit accounting), the
  indicator hides entirely rather than displaying a zero or
  ambiguous value.

### Scope-change state taxonomy (resolves the B6/B6.1 conflict)

The reviewer flagged that "hide immediately on workspace switching"
(B6 / A3a / "Hide-on-unresolved-scope") contradicts "show loading
placeholder during scope change" (B6.1). Both rules are correct but
they address different states. This subsection pins the taxonomy so
the conflict is resolved explicitly:

| State | Trigger | Active scope identifier | Chip behavior | Authoritative rule |
|---|---|---|---|---|
| **S-resolved-old** | Steady state on scope A | `Some(A)` | "$X · 7-day spend" for A | B2 / A2 |
| **S-changing** | User selects scope B; commit in flight | `Some(B)` (committed synchronously) | Loading placeholder "— · 7-day spend"; tooltip names B | B6.1 |
| **S-fetched-new** | First successful fetch for B lands | `Some(B)` | "$Y · 7-day spend" for B | B2 / A2 |
| **S-unresolved** | No billing scope (signed out, BYOK-no-credit, scope resolver returned `None`, or scope-resolution error) | `None` | Hidden (slot empty) | B6 / A3a / "Hide-on-unresolved-scope" |
| **S-prolonged-failure** | ≥5 min consecutive failed fetches for the active scope | `Some(B)` | Hidden | B6 / A3c |

**The distinguishing rule.** The scope identifier is binary: it is
either `Some(scope_id)` or `None`. The chip hides if and only if
the active scope identifier is `None` (S-unresolved) or the
prolonged-failure threshold has fired (S-prolonged-failure).
**Workspace switching is split into two sub-states:**

- The instant the user clicks a different workspace in the selector
  and the new scope identifier `B` is computed: the active scope
  identifier transitions atomically from `A` to `B` (never to
  `None`). The chip enters S-changing with the loading placeholder
  per B6.1.
- If, during the switch, the scope resolver cannot produce a
  scope identifier for the new selection at all (e.g., the
  workspace exists in the selector but has no billing context
  configured, or the API call to resolve the scope fails), the
  active scope identifier becomes `None` and the chip enters
  S-unresolved (hidden) per B6 / A3a.

The phrase "workspace switching mid-flight" in B6 and the
"Hide-on-unresolved-scope" section is hereby clarified to mean
**only** the case where the scope identifier is `None`. The case
where the scope identifier is `Some(B)` but no value for B has yet
been fetched is S-changing and uses the loading placeholder, not
hiding. The same clarification applies to A3a; the row below
restates A3a's precondition in those terms.

This taxonomy is asserted by `T_scope_state_table` (Test plan)
which drives each state from a synthesized fixture and asserts the
matching chip rendering.

### Cross-scope leak prevention

Billing spend and workspace scope names are sensitive account data:
showing one billing scope's dollar value under another scope's
workspace label is a privacy regression even for a few seconds. This
is treated as a security-relevant requirement of V1, not a polish
follow-up.

The implementation rules below are mandatory:

1. **Synchronous clear.** Scope changes invalidate the cached value
   *before* the workspace-selector mutation is committed to the UI
   tree. The chip never renders a frame in which the workspace
   selector reads "Acme" while the chip's dollar amount still
   reflects "Personal."
2. **No display-from-previous-scope.** The cache is keyed by scope
   identifier (e.g., `BillingScopeId`). A cache lookup that returns
   a value belonging to a different scope than the active one is
   treated as a cache miss; the chip enters the loading state rather
   than rendering the mismatched value.
3. **Immediate scoped refresh.** A dedicated fetch for the new scope
   is dispatched in the same code path that triggers the synchronous
   clear, not on the next 60 s tick. Network failures fall through
   to the failure-retry timer ("Refresh vs failure-retry timers"
   below); the chip stays in the loading state until the new scope's
   value lands or the 5-minute hide threshold elapses.
4. **In-flight fetch cancellation.** Any in-flight fetch for a
   previous scope is cancelled at scope-change time so a late
   response from the old scope cannot land in the chip. If
   cancellation is best-effort, the response handler MUST verify the
   scope identifier on the response matches the active scope and
   discard the response otherwise.
5. **Tooltip parity.** The tooltip's scope label and dollar value
   are read from the same scope-keyed cache entry. The tooltip can
   never show a scope name that disagrees with the dollar value
   above it.

These rules together close the round-1 stale-cross-scope-display
gap.

## Behavior contract

- B1. New setting
  `agents.show_weekly_spend_status: bool` (default `false`,
  `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`). The setting
  key keeps the existing `weekly` token for backwards-compat with
  any in-progress drafts; user-facing copy uses "7-day spend".
- B2. When ON, a status-bar item shows "$XX.XX · 7-day spend" in the
  right-hand status-bar group, immediately to the left of the
  context-remaining chip. See "Status-bar placement" below for the
  precise slot, ordering, and width-constrained collapse rules.
- B3. The indicator updates on every agent turn finish AND on a
  60-second refresh timer. Failure handling uses a separate
  exponential-backoff retry timer; the 60 s cadence governs only the
  success path. See "Refresh vs failure-retry timers" below.
- B4. Hover tooltip shows: active scope name, rolling 7-day total
  (USD with 2 decimal places), current month total, the caption
  "rolling 7 days", and a "View details →" link that opens
  Settings → Billing & Usage.
- B5. Click on the indicator opens Settings → Billing & Usage
  (one-click bypass of the menu drill-down).
- B6. When billing data is unavailable, the indicator is hidden —
  not stuck on $0.00 which would be misleading. The hide thresholds
  are split: scope-unresolved hides immediately; ≥30 s without a
  successful fetch surfaces a "stale" tooltip annotation but keeps
  the chip visible; ≥5 minutes of continuous failure hides the chip
  entirely. See "Refresh vs failure-retry timers" below.
- B6.1. **Loading-state rendering during scope change.** Between the
  synchronous clear (Cross-scope leak prevention rule 1) and the
  arrival of the new scope's first successful fetch, the chip
  renders the placeholder "— · 7-day spend" (em-dash, no dollar
  amount). The placeholder is taken to occupy the same width as the
  full label so the right-hand cluster does not reflow. If the
  loading state persists past 5 minutes (rule 3 above + the 5-minute
  hide threshold), the chip hides per B6's prolonged-failure rule.
- B7. Default OFF: the issue's request is opt-in; users who don't
  want to see spend reminders aren't forced to. The setting is
  one click in Settings → Agents → Display.

## Data source

- **Source of truth.** The indicator reads from the same
  Billing & Usage backend response that already powers the
  Settings → Billing & Usage pane. **No new endpoint** is added for
  V1, but V1 depends on an **additive** field on the existing
  response.
- **Required field: `usage_rolling_7d_usd`.** The indicator requires a
  `usage_rolling_7d_usd: f64` (or equivalent fixed-point USD) field on
  the existing `BillingUsage` response. This is the only field that
  delivers the rolling 7-day window directly; today's response carries
  the current-month total and per-day buckets but does not expose a
  precomputed rolling 7-day USD figure with the freshness guarantees
  this indicator needs (≤2s post-turn).
  - Computing 7-day rolling on the client from per-day buckets is
    rejected as the V1 plan because (a) per-day buckets settle
    asynchronously after a turn and would not satisfy A4, and (b) it
    pushes monetary aggregation into the client where rounding rules
    must match the server-side dashboard exactly.
  - Backend additive change: extend the existing `BillingUsage`
    response with `usage_rolling_7d_usd`. Same endpoint, same auth,
    same scope resolution; one additional field. **This is not a new
    endpoint** and is the explicit "no new endpoint" interpretation
    for this spec.
  - Until the backend confirms the additive field can ship in the V1
    window, this dependency is tracked in **Open questions** below
    and blocks V1.
- **Refresh cadence.** The client cache is invalidated on every
  agent-turn-completion event and on the 60-second timer. The post-
  turn refresh is required to land in the indicator within ≤2s of
  the turn-completion event (B3 / A4). Refresh and failure-retry are
  separate timers (see "Refresh vs failure-retry timers" below); the
  60-second timer governs only the success path.
- **Cache TTL.** Cached value is considered fresh for ≤2s following
  a turn completion. Outside that window the cache follows the 60s
  timer.
- **Hide-on-unresolved-scope.** When the active billing scope is
  unresolved (signed out, workspace switching, BYOK without credit
  accounting), the indicator hides immediately — no stale value is
  shown across scope changes.

### Refresh vs failure-retry timers

The 60-second refresh cadence and the failure-handling behavior are
governed by **two independent timers**. Treating them as one (as the
round-1 draft did) caused a contradiction: a 30-second hide-on-failure
rule cannot coexist with a 60-second refresh cadence — the indicator
would hide before its next legitimate refresh attempt.

- **Refresh timer (success path).** Period 60 s, plus an immediate
  refresh on every `agent_turn_completed` event. Each tick fetches
  from the existing Billing & Usage subscription. On success, the
  cached value updates, the failure counter resets to zero, and the
  failure-retry timer is cancelled.
- **Failure-retry timer.** On a refresh failure, the refresh timer
  pauses and the failure-retry timer takes over with exponential
  backoff: **5 s, 10 s, 20 s, 40 s, 80 s (cap)**. Each retry
  attempts the same fetch. On success, the failure-retry timer is
  cancelled, the refresh timer resumes its 60 s cadence, and the
  failure counter is cleared.
- **Stale tooltip threshold (≥30 s since last success).** While the
  failure-retry timer is active and ≥30 s have elapsed since the
  most recent successful fetch, the tooltip adds a "stale"
  annotation: "Updated Xs ago — couldn't refresh." The chip itself
  does not change appearance; only the tooltip surfaces the
  staleness so the status-bar layout does not shift. **The chip
  remains visible at this stage** — it shows the last-known value
  with the stale annotation in the tooltip.
- **Hide-on-prolonged-failure (≥5 min consecutive failures).** If the
  failure-retry timer has been active continuously for 5 minutes
  without a successful fetch, the indicator hides entirely. It
  re-appears on the first successful fetch, at which point the
  refresh timer resumes its 60 s cadence.
- **Reconciliation note.** Round-1 specified "hide after 30 seconds
  of consecutive failures." Round 2 separates the two thresholds:
  ≥30 s → tooltip "stale" annotation, chip remains visible; ≥5 min
  → hide. The 30 s value is preserved as the *staleness marker*,
  not the *hide threshold*, which removes the conflict with the 60 s
  refresh cadence.

## Acceptance criteria

- A1. Setting OFF: no status-bar entry, layout pixel-equivalent.
- A2. Setting ON, billing data available: shows
  "$XX.XX · 7-day spend" with hover tooltip and click-through.
- A3a. Setting ON, **active scope identifier is `None`** (signed
  out, BYOK without credit accounting, scope resolver returned
  `None` for the active workspace, or scope-resolution error during
  a switch): status-bar slot is empty immediately. This is the
  S-unresolved state from the "Scope-change state taxonomy"
  subsection. The case where the active scope identifier is
  `Some(B)` but the value for B has not yet been fetched (the
  S-changing state during a successful workspace switch) is
  covered by A3a-loading below, not by this hide rule.
- A3a-loading. Setting ON, **active scope identifier is `Some(B)`
  but no successful fetch for B yet**: chip renders the loading
  placeholder "— · 7-day spend" per B6.1 with the tooltip naming
  B. This is the S-changing state. Persisting past 5 minutes
  promotes to A3c (hide on prolonged failure).
- A3b. Setting ON, ≥30 s since most recent successful fetch (transient
  failures continuing): chip remains visible with last-known value;
  tooltip shows "Updated Xs ago — couldn't refresh".
- A3c. Setting ON, ≥5 minutes of continuous failed fetches: status-
  bar slot is empty; reappears on first successful fetch.
- A4. Number updates within ≤2 seconds of an agent turn finishing.
- A5. Status-bar placement: the segment is positioned in the right-
  hand status-bar group, in the order specified in "Status-bar
  placement" below. Window-resize tests confirm the documented
  collapse priority.

## Status-bar placement

The 7-day-spend segment lives in the **right-hand status-bar slot
group** — the same group that already hosts the context-remaining
indicator (the "96% context" chip) and other right-aligned chips. It
is NOT a new slot group and does not introduce a new region in the
status-bar layout.

### Slot ordering (left → right within the right-hand group)

```
[ 7-day spend ] [ context % ] [ debug / dev indicators ] [ menu chips ]
```

The 7-day-spend chip sits immediately to the **left** of the
context-remaining chip. Rationale: spend is the lower-frequency, more
human-readable indicator and reads first when the eye scans the
right-hand cluster; the context indicator updates per token and is
treated as a more transient, machine-state signal that anchors the
group.

### Collision and collapse priority

When window width is constrained, status-bar segments collapse in
priority order. Lower-priority segments collapse to icon-only first,
then hide; higher-priority segments stay full until they cannot fit.
The 7-day-spend chip is treated as **medium priority** in the
existing scheme:

1. **Highest priority (never collapse first).** Context-remaining,
   active-error chip.
2. **Medium priority — collapses to "$XX.XX" without the
   "· 7-day spend" caption when width is tight.** 7-day spend.
3. **Medium priority — hides under further width constraint.**
   7-day spend (after the icon-only collapse).
4. **Lowest priority (collapse first).** Debug / dev / experimental
   chips.

Concrete behavior at three breakpoints:

| Available right-cluster width | 7-day-spend rendering |
|-------------------------------|------------------------|
| Wide (default)                | "$XX.XX · 7-day spend" with full caption |
| Narrow                        | "$XX.XX" only, caption dropped |
| Very narrow (sub-threshold)   | Hidden; reappears when width returns |

The chip never wraps onto a second line; the status bar is always a
single row. Width thresholds are inherited from the existing
status-bar collapse logic — no new threshold values are introduced.

## Privacy / telemetry

- The dollar value lives only in the local UI. No new telemetry
  events are emitted by enabling/disabling this indicator beyond
  the existing `setting_changed` event with the boolean value.
- The indicator is read-only — it cannot send the value anywhere.

## Test plan

- T1. Setting round-trips through TOML.
- T2. With billing data fixture and setting ON, the
  rendered status-bar tree contains the spend label
  "$XX.XX · 7-day spend".
- T3. With billing data fixture and setting OFF, the slot is empty
  (snapshot test).
- T4. Click handler dispatches the
  `OpenSettingsPage(BillingAndUsage)` action.
- T5a. Hidden-on-unresolved-scope: indicator disappears immediately
  when the scope is unresolved (signed-out fixture; workspace
  switching mid-flight; BYOK without credit accounting).
- T5b. Stale-tooltip-after-30s: with a fault-injecting fake client
  failing every fetch after one initial success, the chip remains
  visible at t+35 s with the tooltip carrying the "stale"
  annotation. (Asserts the 30 s threshold marks staleness, not
  hiding.)
- T5c. Hidden-after-5min: with the same fault-injecting client, the
  chip is hidden by t+5 min 5 s, and reappears within ≤2 s of the
  next successful fetch.
- T5d. Backoff schedule — distinguishes **interval-between-attempts**
  from absolute timestamps.

  Let `t_fail_1` be the wall-clock time the first refresh failure
  is observed. The failure-retry timer schedules subsequent attempts
  by **interval** (delay since the previous attempt finished), not
  by absolute time relative to `t_fail_1`. The intervals are the
  exponential backoff sequence from "Refresh vs failure-retry
  timers": **5 s, 10 s, 20 s, 40 s, 80 s, 80 s, ... (capped at
  80 s)**.

  The test asserts the **interval** between each pair of
  consecutive attempts:

  | Attempt # | Interval since previous attempt | Cumulative wall-clock from `t_fail_1` (informational only) |
  |---|---|---|
  | retry 1 | 5 s after `t_fail_1` | `t_fail_1 + 5 s` |
  | retry 2 | 10 s after retry 1 | `t_fail_1 + 15 s` |
  | retry 3 | 20 s after retry 2 | `t_fail_1 + 35 s` |
  | retry 4 | 40 s after retry 3 | `t_fail_1 + 75 s` |
  | retry 5 | 80 s after retry 4 | `t_fail_1 + 155 s` |
  | retry 6+ | 80 s after the previous attempt (capped) | `+80 s` each |

  Assertions:
  1. `interval(retry_n, retry_{n+1})` matches the sequence above
     (±50 ms scheduler tolerance, asserted against a virtual /
     mocked clock so the test does not actually sleep).
  2. On recovery (any retry succeeds), the failure-retry timer is
     cancelled and the refresh timer resumes its **60 s interval**
     from the success timestamp — NOT from `t_fail_1` or any
     earlier wall-clock anchor.
  3. The "cumulative wall-clock" column is informational only and
     is NOT used as an absolute-timestamp assertion. Wall-clock
     drift across retries (e.g., a slow scheduler tick) must not
     fail the test; only the consecutive-interval assertion may
     fail it.

  Rationale: an earlier wording confused intervals with absolute
  timestamps, which would make the test brittle under realistic
  scheduler jitter. The interval-based contract is what the
  implementation actually owns; absolute timestamps are derived
  data.
- T6. Refresh timing: a simulated agent-turn-completion event
  causes the indicator to reflect the new value within 2 seconds.
- T7. Tooltip totals (full B4 coverage): tooltip renders ALL of:
  (a) the active scope name (e.g., "Personal" or "Acme
  Workspace");
  (b) the rolling 7-day total formatted as exact USD with 2
  decimal places;
  (c) the **current-month total** formatted as exact USD with 2
  decimal places, sourced from the existing `BillingUsage`
  `usage_current_month_usd` field (no client-side aggregation);
  (d) the "rolling 7 days" caption;
  (e) a "View details →" link that, when activated by click or by
  keyboard activation (Enter / Space when focused), dispatches the
  same `OpenSettingsPage(BillingAndUsage)` action as T4. Asserted
  with both a click-handler unit test on the link and an
  end-to-end snapshot of the tooltip DOM verifying all 5 fields
  are present.
- T8. Stale-on-error single-failure: a single failed fetch following
  a successful one keeps the chip's value visible (within the 30 s
  staleness window the tooltip does not yet add the "stale"
  annotation; the annotation appears only past 30 s).
- T_scope_state_table. Drive each of the five rows in the
  "Scope-change state taxonomy" table (S-resolved-old, S-changing,
  S-fetched-new, S-unresolved, S-prolonged-failure) from a
  synthesized fixture and assert the chip rendering matches the
  table:
  - S-resolved-old: scope `A`, cached value `$12.34` → chip shows
    `"$12.34 · 7-day spend"` with tooltip naming `A`.
  - S-changing: switch from `A` to `B`, no fetch yet for `B` → chip
    shows placeholder `"— · 7-day spend"` with tooltip naming `B`.
    Asserts NO frame contains `$12.34` together with `B`'s
    tooltip label (the security-critical no-leak rule).
  - S-fetched-new: first successful fetch for `B` lands with value
    `$5.67` → chip transitions to `"$5.67 · 7-day spend"` with
    tooltip naming `B`.
  - S-unresolved: scope resolver returns `None` (signed-out
    fixture, BYOK-no-credit fixture, scope-resolution error
    fixture — three sub-cases) → chip is hidden (slot empty).
  - S-prolonged-failure: scope `B` with 5 min of continuous failed
    fetches → chip is hidden; first success after that re-shows
    the chip within ≤2 s.
- T9a. Multi-scope synchronous clear (security-critical): switching
  the workspace selector from "Personal" ($12.34) to "Acme
  Workspace" results in the chip showing the loading placeholder
  "— · 7-day spend" *before the next paint*. Asserted by capturing
  the rendered tree on the same UI commit that mutates the
  workspace selector and checking that no frame ever contains
  "$12.34" together with the "Acme Workspace" tooltip label.
- T9b. Cross-scope cache miss: a cached value tagged for scope
  `personal` is treated as a miss when the active scope is
  `acme-workspace` — the chip enters the loading state rather than
  rendering the cached personal value under the Acme label.
- T9c. Late-response discard: a fetch issued for `personal` that
  resolves *after* the user has switched to `acme-workspace` is
  dropped on arrival; the chip's value does not change. Asserted
  with a fault-injecting fake client that delays the personal
  response.
- T9d. Immediate scoped refresh: scope change dispatches a fresh
  fetch for the new scope without waiting for the 60 s tick. Once
  the new scope's response arrives, the chip transitions from
  "— · 7-day spend" to the new scope's USD value.
- T10. Status-bar placement: snapshot the right-hand status-bar
  group with the chip ON; assert the 7-day-spend chip is rendered
  immediately to the left of the context-remaining chip.
- T11. Width-constrained collapse: at the "narrow" breakpoint the
  chip renders as "$XX.XX" without the "· 7-day spend" caption; at
  the "very narrow" breakpoint the chip is hidden; at "wide" the
  full label is rendered.
- T12. Field dependency: a fixture response missing
  `usage_rolling_7d_usd` causes the chip to hide (treated as
  unavailable) and surfaces a single warn-level log entry — does
  not crash, does not synthesize a client-side value.

  **Redaction contract for the warn log (security-relevant).** The
  warn-level log entry is the only log emitted by the indicator on
  the missing-field path. Its payload is restricted to the
  following non-sensitive fields, and the test asserts both
  (a) the presence of these fields and (b) the **absence** of
  every banned field:

  Permitted fields in the log entry:

  - A static event identifier (e.g., `"billing_usage_field_missing"`).
  - A static missing-field name (the literal string
    `"usage_rolling_7d_usd"`) — this is a code constant, not user
    data.
  - A scope-kind enum tag, with only the constant values
    `"personal"`, `"workspace"`, or `"unknown"`. NEVER the workspace
    name, workspace ID, or any other scope-identifying string.
  - A boolean `chip_hidden: true`.

  Banned fields (explicitly asserted as absent by `T_log_redaction`):

  1. Any raw billing payload bytes, JSON snippets, or fragments of
     the response body.
  2. Any dollar amount, in any field (current month, 7-day, per-day
     buckets, or otherwise) — even from sibling fields that happen
     to be populated.
  3. Any scope name string (e.g., the literal `"Acme Workspace"`),
     scope display name, organization name, or workspace slug.
  4. Any scope identifier (`BillingScopeId`, workspace UUID, team
     ID) other than the three-value `scope_kind` enum above.
  5. Any user identifier (user ID, email, account ID) or auth
     token / session token fragment.
  6. Any free-form `message` string built from the response (the
     log uses a static format string only).

  **Assertion mechanism for `T_log_redaction`:** the test installs a
  capturing log subscriber, runs the missing-field path against a
  response fixture populated with sentinel values (workspace name
  `"SENTINEL_WS"`, dollar amount `99999.99`, user email
  `"sentinel@example.com"`, scope id `"SENTINEL_SCOPE_ID"`), then
  asserts:

  1. Exactly one warn-level entry is emitted.
  2. The entry's serialized form (string-formatted plus structured
     fields) contains the permitted fields above.
  3. The entry's serialized form contains NONE of the sentinel
     values — confirming the redaction is structural (the code
     never reaches for those values) rather than just regex-based.

  Rationale: billing data is account-sensitive, and a warn-level log
  on a server-trace pipeline is a realistic exfiltration vector if
  the log message naively interpolates response fields. The
  structural redaction above (static format string + small
  enum-typed scope kind) keeps the log useful for debugging without
  ever forwarding raw billing payloads, dollar amounts, or scope
  names to logs.

## Open questions

- **OQ1. Backend `usage_rolling_7d_usd` field.** V1 depends on the
  Billing & Usage backend exposing a `usage_rolling_7d_usd` field
  on the existing response (additive, no new endpoint). Until
  backend confirms this can ship in the V1 window, V1 cannot ship.
  Tracked as a hard dependency. If backend pushes back, fall-back
  options (in order of preference):
  1. Server-side rolling calculation gated behind a feature flag
     so the client can begin opt-in adoption while the backend
     stabilises.
  2. Client-side rolling computation from per-day buckets, accepting
     the relaxed A4 (drop "≤2 s after turn completion" to
     "≤2 s after the next per-day bucket settles") — explicitly
     called out as a regression from V1 goals.

## Out of scope (V1)

- Daily / monthly toggles. V1 is 7-day rolling only — the most
  asked-for window in the issue thread.
- Visual warning thresholds ("80% of budget"). The user's budget
  is set elsewhere; surfacing that intersection is V2.
- Real-time SSE updates instead of the 60s poll + per-turn refresh.
- A new dedicated billing endpoint optimised for the indicator.
  V1 reuses the existing Billing & Usage endpoint.
