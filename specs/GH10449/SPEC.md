# Spec: AI credit usage in the bottom status line (GH-10449)

## Summary

Add an opt-in status-bar segment that shows the user's AI credit
consumption for the current billing period (e.g., `89/2500`),
mirroring the existing "context remaining" segment so users can
monitor remaining credits at a glance without opening Settings.

## Problem

Today, users can only see how many AI credits they have used by
opening Settings → Billing & Usage. For users on metered plans
who run agent turns frequently, this round-trip is slow and
discourages cost awareness mid-session. Issue #10449 asks for a
small, always-visible segment in the bottom status line that
shows `<used>/<allowance>`, directly analogous to the existing
"96% context remaining" segment.

## Goals

- Provide an opt-in status-bar segment showing
  `<used>/<allowance>` for the current billing period.
- Click on the segment navigates to Settings → Billing & Usage.
- Segment updates within ≤2s of an agent-turn completion event.
- Visual state escalates by usage threshold (neutral → yellow →
  red) so high-spend users notice without opening Settings.
- Hidden by default; user opts in from Appearance → Status Bar.

## Non-Goals

- Not a real-time billing telemetry feed.
- Not a notification system (no toasts on threshold crossings).
- Not a quota-block enforcer; the segment never gates command
  execution.
- Not a replacement for the Billing & Usage pane; it summarizes,
  it does not duplicate the full breakdown.
- Not a per-agent or per-task cost breakdown; it only shows the
  scope-level totals already returned by the billing API.
- No segment-side telemetry beyond click events. The segment
  MUST NOT emit usage totals, scope names, workspace identifiers,
  plan tier, or allowance reset times to telemetry. All
  usage-shape telemetry remains owned by the Billing & Usage pane.

## Behavior Contract

### B1. Display format

- Format: `<used>/<allowance>` rendered as plain integers, e.g.
  `89/2500`. No thousands separators in V1 to keep the segment
  compact at narrow window widths.
- Threshold colors:
  - `<80%` used → default status-bar foreground.
  - `≥80%` and `<95%` used → yellow / warning foreground (uses
    the existing status-bar warning token).
  - `≥95%` used → red / danger foreground (uses the existing
    status-bar danger token).
- "Unlimited" plan: see B1a.ii (plan-overlay) row for unlimited.
- Missing-data display: see B1a.i. The data-state table is the
  single source of truth and supersedes any older wording about
  `—/—` indeterminate rendering.

### B1a. Display state matrix (single source of truth)

This table is THE canonical map from data state to render. Any
prior wording elsewhere in this spec that conflicts with this
table is superseded by the table. Every state below reuses
already-defined status-bar tokens — the segment never invents
new visual variants.

#### B1a.i Data-state table (data fetch lifecycle)

These rows cover what the segment shows for ANY combination of
fetch lifecycle and missing-field state. Plan/account states
(unlimited, enterprise, delinquent, restricted, add-ons) are
overlaid in the next subsection.

| Data state | Display | Color tokens | Tooltip |
|---|---|---|---|
| Loading (initial fetch, no cached value) | `…/—` | Neutral default fg | "Fetching usage…" |
| Loading (initial fetch, cached value present) | last cached `<used>/<allowance>` with subtle dim | Dim default fg | "Refreshing… last updated <Xs> ago" |
| Success — both `used` and `allowance` known | `<used>/<allowance>` | <80% default · 80–94% warning (`status_bar_warning_fg`) · ≥95% danger (`status_bar_danger_fg`) | "<scope> · <period> · resets <reset_date>" |
| Success — `allowance` missing, `used` known | `<used>/—` | Neutral default fg; thresholds NOT applied | "Allowance unknown" |
| Success — `used` missing, `allowance` known | `—/<allowance>` | Neutral default fg; thresholds NOT applied | "Usage unknown" |
| Success — both `used` and `allowance` missing | Segment hidden | n/a | n/a |
| Stale (≥30s since last successful fetch, retrying) | last known `<used>/<allowance>` plus a single dot indicator (e.g. `89/2500 ·`) | Dim default fg | "Stale: <Xs> since last update" |
| Failed (≥5 min consecutive failures) | Segment hidden | n/a | n/a |
| Unauthorized (no billing-view permission for current scope) | Segment hidden | n/a | n/a |

Notes:
- "Both missing" and "Failed" hide the segment entirely. Hide is
  preferred over showing `—/—` to avoid misleading screenshots.
- The 30s stale window in B2 marks the segment as stale; the
  ≥5 min window in this table is the upper bound that flips it to
  "Failed" and hides it. Resume display once a fetch succeeds.
- Rows above replace any prior wording around indeterminate
  display, hide-on-failure timing, or `—/—` rendering elsewhere
  in this spec; if older wording disagrees, this table wins.

#### B1a.ii Plan / account state overlay

Layered on top of the data-state table above. When the data is
"Success", the plan/account state determines the SHAPE of the
displayed value and whether thresholds apply.

| Plan / account state | Segment glyph / text | Color tokens | Tooltip | Click action |
|---|---|---|---|---|
| Unlimited plan (`is_unlimited == true`) | `∞` glyph (or "Unlimited" if locale lacks the glyph) | Default fg; thresholds NOT applied | "Unlimited plan" — **no USD figure, no monthly spend, no invoice amount, no credit-burn rate**. The unlimited tooltip is intentionally degenerate; any financial figure belongs to the sibling USD-spend spec (#10224), not this chip. See "Unlimited tooltip — financial-data exclusion" below for the full prohibition. | Open Settings → Billing & Usage |
| Usage-based / metered (`is_unlimited == false`, no add-ons) | `<used>/<allowance>` (integers) | Per data-state table thresholds | "<scope> · Monthly allowance · resets <date>" | Open Settings → Billing & Usage |
| Add-on credits on top of base allowance (one or more add-ons returned) | `<used>/<base>+<addon_total>` (e.g. `89/2500+750`) | Thresholds computed against `(base + addon_total)` | See B1a.iii | Open Settings → Billing & Usage |
| Enterprise plan with contract limit | `<used>/<allowance>` | Same thresholds as metered | "<scope> · Enterprise · resets <date>" | Open Settings → Billing & Usage |
| Enterprise plan, no contract numeric limit | Plan name only ("Enterprise"); no numeric usage | Default fg; thresholds NOT applied | "Enterprise plan · usage tracked centrally" | Open Settings → Billing & Usage |
| Delinquent (account past due) | `!` glyph | Danger fg | "Account past due" | Open Settings → Billing & Usage → Payment |
| Restricted (admin-disabled) | Lock glyph | Default fg | "Restricted by admin" | Open Settings → Billing & Usage (informational) |
| Signed-out / no billing context | Segment hidden | n/a | n/a | n/a |

##### Unlimited tooltip — financial-data exclusion

This chip is scoped to **credit-count display only**. Even when
`is_unlimited == true` causes the credit count to be absent, the
chip MUST NOT compensate by surfacing a USD / financial figure.
Concretely, the Unlimited-plan tooltip:

- MUST render the literal string "Unlimited plan" (or its
  localized equivalent) and nothing else.
- MUST NOT include a current-period USD spend, a monthly USD
  amount, an invoice total, a credit-burn rate, a per-day cost,
  a projected monthly cost, an estimated-overage figure, or any
  other dollar/currency amount — regardless of whether the
  upstream billing source exposes such a value.
- MUST NOT include the count of agent turns, tokens, or any
  other surrogate that could be reverse-engineered into a
  financial figure.
- MUST NOT vary by plan tier name; "Unlimited Pro", "Unlimited
  Team", and any other unlimited variant all render the same
  degenerate "Unlimited plan" tooltip.
- Justification: the chip lives in always-visible chrome (visible
  in screenshares, screenshots, screen recordings, and screen-
  sharing during pair work). Surfacing a monthly USD figure here
  would create an always-visible financial-data exposure outside
  the user's control. Any USD display belongs to the sibling
  USD-spend spec (#10224), which owns its own opt-in surface,
  threshold logic, and exposure model.
- Click action remains "Open Settings → Billing & Usage" so users
  who want to see USD figures find them in Settings.
- This exclusion is asserted in tests T1 and T9 (unlimited row
  coverage) and in T12 (no financial fields in any segment-owned
  payload).

#### B1a.iii Multiple add-on pools

When the billing source returns multiple add-on credit pools
attached to the current scope (verified field:
`bonus_grants: Vec<BonusGrant>` on `AIRequestUsageModel`, see
`app/src/ai/request_usage_model.rs:175`), V1 renders:

- Display: `<used>/<base>+<addon_total>`, where `addon_total` is
  the **sum of every UNEXPIRED add-on grant** at render time. If
  some grants have expired, expired grants are excluded from the
  sum.
- Threshold computation: against `(base + addon_total)`. If
  `addon_total` is 0 (all expired), the segment falls back to
  the base-only `<used>/<allowance>` render.
- Tooltip lists EACH add-on on its own line, in the form
  `Add-on <display_name>: <amount> (expires <expires_at>)`.
  Example:
  ```
  Acme Workspace · Base 2500 (resets Dec 1)
  Add-on Boost: 500 (expires Dec 15)
  Add-on Top-up: 250 (expires Jan 2)
  ```
- Sorting in the tooltip: ascending by `expires_at`, oldest
  expiry first (so the soonest-to-vanish add-on is at the top).
- **Redaction rule for add-on tooltip rows.** Because the tooltip
  is rendered in always-visible chrome (visible in screenshares,
  screenshots, and screen recordings), each add-on row is
  constrained to the SAME "no raw identifier" rule applied to
  workspace identifiers in B3a:
  - `<display_name>`: ONLY the human-readable add-on display name
    already shown in Settings → Billing & Usage (the
    `BonusGrant::display_name` field). Raw add-on / grant
    identifiers (`bonus_grant_id`, internal grant slug, billing
    line-item id, SKU, invoice id, customer id) MUST NOT appear.
  - `<amount>`: integer credit count only. No USD figure, no
    invoice amount, no purchase price.
  - `<expires_at>`: a coarse user-facing date (locale-formatted
    `YYYY-MM-DD` or its locale equivalent). NO timestamps, NO
    timezones, NO time-of-day, NO ISO durations.
  - If a grant has NO `display_name` (older grants where the
    field is missing), the row falls back to the literal string
    "Add-on credit" — never the grant id. If even the amount is
    missing for a grant, that grant row is OMITTED rather than
    rendered with placeholder data.
  - The same redaction applies whether the tooltip is shown via
    hover, keyboard focus, or assistive-tech accessibility tree.
  - These tooltip rows are display-only and are NEVER copied into
    telemetry payloads (see Telemetry).
- The "+addon" display only renders when at least one add-on
  with `expires_at > now` is present. If no add-ons are surfaced,
  the segment falls back to the metered render.

### B2. Refresh & caching (subscription model)

- **Verified owner: `AIRequestUsageModel`.** The existing singleton
  model `AIRequestUsageModel` (defined in
  `app/src/ai/request_usage_model.rs`, registered as a
  `SingletonEntity` via `ctx.add_singleton_model(...)` in
  `app/src/lib.rs:1251`) is the ONE owner of usage and bonus-grant
  state for the process. It already:
  - Owns `request_limit_info: RequestLimitInfo` and
    `bonus_grants: Vec<BonusGrant>`.
  - Owns `last_update_time: Option<Instant>`.
  - Caches usage to private user preferences via
    `cache_request_limit_info` for next-launch hydration.
  - Spawns the GraphQL fetch via
    `AIRequestUsageModel::refresh_request_usage_async`.
  - Emits `AIRequestUsageModelEvent::RequestUsageUpdated` to
    subscribers.
- **Status-bar segment is a passive observer of fetches.** The
  chip's relationship to network refresh is strictly observer:
  - It MAY subscribe to `AIRequestUsageModel`'s
    `RequestUsageUpdated` events to drive re-renders (this is a
    model-event subscription, not a network fetch).
  - It MUST NOT call `refresh_request_usage_async`, MUST NOT spawn
    its own polling loop, and MUST NOT open its own GraphQL
    client.
  - It MUST NOT subscribe to or trigger billing GraphQL fetches
    directly. The existing call sites that already drive
    `refresh_request_usage_async` (auth state changes, agent-turn
    completion handler in the conversation/Billing pane path,
    explicit settings refresh) remain the ONLY refresh triggers.
  - Reads happen via
    `AIRequestUsageModel::as_ref(ctx).request_limit()` (used
    elsewhere in `app/src/terminal/input.rs:12788`).
  - Subscription pattern mirrors the existing warpui call sites
    (verified): `ctx.subscribe_to_model(&AIRequestUsageModel::handle(ctx),
    |me, event, ctx| { ... })` — see
    `app/src/terminal/buy_credits_banner.rs:75` and
    `app/src/workspace/view/free_tier_limit_hit_modal.rs:68`.
  In short: the segment subscribes to MODEL EVENTS for redraw, not
  to AGENT-TURN events for refetch. The agent-turn → refetch path
  is owned upstream by `AIRequestUsageModel` and its existing
  callers; the segment never adds a new refresh trigger.
- **Single-fetch invariant.** Because `AIRequestUsageModel` is a
  warpui `SingletonEntity`, the per-process single-instance
  invariant is structural, not advisory: spawning N status-bar
  chips (in N windows / panes) attaches N subscribers to the SAME
  singleton — there is no additional fetch loop per chip.
- Cache TTL: the chip re-renders within ≤2s of the most-recent
  `RequestUsageUpdated` event published by `AIRequestUsageModel`.
  The agent-turn-completion → `refresh_request_usage_async` →
  `RequestUsageUpdated` chain is owned ENTIRELY upstream by
  `AIRequestUsageModel` and its existing callers (the conversation
  list / Billing pane path). The segment does NOT subscribe to
  agent-turn completion itself, does NOT subscribe to billing
  fetches directly, and does NOT add a new refresh trigger; it
  only listens to `RequestUsageUpdated` for redraw.
- Background refresh cadence is OWNED by `AIRequestUsageModel`
  and the call sites that already invoke
  `refresh_request_usage_async` (auth state changes, post agent
  turn, manual settings refresh). The status-bar chip does NOT
  add a new refresh trigger.
- **Stale and failed state — two distinct signals.** `last_update_time`
  alone is insufficient to distinguish "stale because no fetch has
  been attempted recently" from "stale because every recent fetch
  attempt has failed". This spec therefore requires TWO signals on
  `AIRequestUsageModel`, both upstream-owned:
  1. `last_update_time: Option<Instant>` — the timestamp of the
     most recent **successful** fetch (already present, see
     `app/src/ai/request_usage_model.rs`).
  2. `last_fetch_failed_at: Option<Instant>` — the timestamp of the
     most recent **failed** fetch attempt. Set whenever
     `refresh_request_usage_async` resolves to an error; cleared
     whenever it resolves to success. If this field does not yet
     exist on `AIRequestUsageModel`, this spec authorizes adding
     it on the upstream model (the same upstream that already owns
     `last_update_time`); the field is read by the segment, never
     written by it.
  The segment then computes its display row deterministically:
  - **Stale row (B1a.i).** Render when
    `now - last_update_time > 30s` AND
    `last_fetch_failed_at` is None OR
    `last_fetch_failed_at < now - 5min`. Intuition: data is older
    than 30s but the chip has no positive evidence of repeated
    recent failure — show the stale dot and the last-known value.
  - **Failed row (B1a.i, segment hidden).** Render when
    `last_fetch_failed_at` is Some AND
    `now - last_fetch_failed_at <= 5min` AND
    (`last_update_time` is None OR
     `last_update_time < last_fetch_failed_at - 5min`). Intuition:
    at least one failure has occurred and no successful fetch has
    landed for at least 5 minutes — hide.
  - **Recovery.** A successful `RequestUsageUpdated` event clears
    `last_fetch_failed_at`, advances `last_update_time`, and the
    segment leaves both Stale and Failed states on the next render
    tick.
  Both signals are computed from upstream model state; the segment
  itself maintains NO timers, NO failure counters, and NO retry
  loops of its own. The "consecutive failures" language in earlier
  drafts is replaced by the timestamp-pair test above, which is
  what the implementation actually evaluates.

### B3. Scope (workspace selector)

- The segment follows the current workspace-billing-context
  selector (the same selector that drives the Billing pane).
- **Refresh ownership on workspace-scope change.** When the user
  switches workspace scope (e.g. "Personal" → "Acme Workspace"),
  the segment itself does NOT trigger a fetch. The
  workspace-billing-context resolver is the existing call site
  that invokes `AIRequestUsageModel::refresh_request_usage_async`
  on scope change (one of the "auth state changes / existing
  refresh triggers" enumerated in B2). The segment listens for
  the subsequent `RequestUsageUpdated` event and re-renders
  within ≤2s of that event, matching the per-process single-fetch
  invariant. If `AIRequestUsageModel`'s current implementation
  does not already refresh on workspace-billing-context change,
  this spec authorizes adding that refresh call inside the
  existing workspace-billing-context resolver — NOT inside the
  segment. The segment remains the strict observer defined in
  B2, with no fetch / no polling / no GraphQL client of its own,
  even on scope change. This is the same passive-observer rule
  that applies to agent-turn completion in B2; B3 is consistent
  with B2.
- During the brief window between the scope change firing and
  the next `RequestUsageUpdated` event landing, the segment
  hides (or shows the Loading row of B1a.i if a cached value
  for the new scope is available) rather than flashing the
  previous scope's numbers (see B3a authorization & redaction).
- Tooltip shows scope name + allowance period, e.g.:
  `Acme Workspace · Monthly allowance · resets Dec 1`.

### B3a. Authorization & redaction

- Authorization: the data fetch path uses the SAME Billing &
  Usage authorization checks as the Billing pane. If the user
  lacks billing-view permission for the active workspace (e.g.
  member-without-billing-access on a team workspace), the segment
  hides entirely — there is no degraded "see used but not
  allowance" mode. The segment never bypasses the pane's authz.
- Workspace-scope changes: workspace-scope transitions go through
  the existing workspace-billing-context resolver. The segment
  never resolves workspace identity on its own. If the resolver
  reports the active workspace is not authorized for billing
  read, the segment hides without flashing stale numbers from a
  previous scope.
- Redaction at the chrome layer: because the segment is in
  always-visible chrome (and visible in screenshares,
  screenshots, screen recordings), the tooltip MUST NOT reveal
  workspace identifiers (workspace UUID, slug, billing customer
  id) that the user does not already see in the workspace
  selector. The tooltip uses the same human-readable workspace
  name the selector shows, never raw identifiers.
- No raw allowance/usage values are sent to telemetry from the
  segment (see Telemetry below).

### B4. Click action

- Single click on the segment: opens Settings → Billing & Usage,
  scoped to the workspace shown in the segment.
- Right-click: opens a small context menu with "Hide credits in
  status bar" (toggles `status_bar.show_ai_credits` to `false`)
  and "Open Billing & Usage".
- Keyboard: focusable via the existing status-bar tab order;
  Enter activates the click action.

### B5. Visibility & opt-in

- Default state: segment is hidden
  (`status_bar.show_ai_credits = false`).
- Opt-in surface: Settings → Appearance → Status Bar, alongside
  any existing status-bar visibility toggles.
- When the user has no billing context at all (e.g., signed-out
  state), the segment stays hidden even if the toggle is on, and
  the toggle subtitle reads "Sign in to see credit usage."

## Settings / API surface

- New setting:
  - Key: `status_bar.show_ai_credits`
  - Type: `bool`
  - Default: `false` (opt-in)
  - Sync: `SyncToCloud::Globally(RespectUserSyncSetting::Yes)`,
    consistent with other Appearance and `status_bar.show_*`
    visibility toggles. The toggle's `bool` value is the ONLY
    field synced — see "Sync & privacy" below.
  - TOML namespace: `[status_bar]` table,
    `show_ai_credits = false`.
- **Sync & privacy (synced-setting justification):**
  - Why synced: this setting represents a user preference for
    chrome density (whether the credits chip is shown in the
    bottom status bar). It pattern-matches every other
    `status_bar.show_*` visibility toggle, which are all synced
    today. A user enabling the chip on their laptop expects it
    to also appear on their desktop; sync delivers that.
  - What is actually synced: ONLY the `bool` value of
    `status_bar.show_ai_credits`. NO usage value, NO allowance,
    NO scope name, NO workspace identifier, NO plan tier, NO
    reset date, NO add-on data is ever synced via this setting.
    The setting is a pure UI-visibility toggle; all billing data
    flows through `AIRequestUsageModel`'s existing fetch and
    cache, not through cloud-synced settings storage.
  - Per-device override: users who want the chip on one device
    but not another use the existing per-device override
    mechanism that already covers every other
    `status_bar.show_*` setting (the same `RespectUserSyncSetting`
    family, which honors a user's per-device decision to opt out
    of settings sync globally). No segment-specific override
    is added.
- No new billing API endpoint. Reuses the existing usage fetch
  consumed by the Billing & Usage pane via `AIRequestUsageModel`.
- No new color tokens. Reuses existing status-bar `warning` and
  `danger` foreground tokens.

## Acceptance Criteria

- A1. With `status_bar.show_ai_credits = true` and a metered plan,
  the segment renders `<used>/<allowance>` matching the values
  shown in Settings → Billing & Usage.
- A2. Threshold colors match B1: <80% default, 80–94% yellow,
  ≥95% red. Verified at the boundary values.
- A3. Clicking the segment opens Settings → Billing & Usage.
- A4. Switching workspace billing scope updates the segment to
  the new scope's used/allowance within 2s.
- A5. After ≥30s without a successful update the segment switches
  to the Stale row of B1a.i (last-known value with stale dot
  indicator). After ≥5 min of consecutive failed/missing updates
  the segment hides per the Failed row. A subsequent successful
  fetch restores the live render.
- A6. With `plan == "unlimited"`, the segment renders `∞` and
  ignores threshold colors.
- A7. With `status_bar.show_ai_credits = false`, the segment is
  not rendered and incurs no extra fetch.
- A8. Right-click → "Hide credits in status bar" flips the
  setting to `false` and the segment disappears immediately.
- A9. Display-state matrix coverage: each row of B1a renders the
  exact glyph/text and color token specified, including
  delinquent, restricted, missing-allowance, enterprise-no-limit,
  and add-on credit cases.
- A10. Single shared upstream: with N status-bar segments visible
  simultaneously across windows/tabs (N ≥ 2), there is exactly
  ONE `AIRequestUsageModel` instance (the existing
  `SingletonEntity`) and the underlying GraphQL fetch cadence is
  unchanged from N=1. Each segment registers an
  `RequestUsageUpdated` model-event subscription on that singleton
  for its own redraw; these are model-event listeners, not
  network fetches.
- A11. Authorization: a user lacking billing-view permission for
  the active workspace has the segment hidden, even with
  `status_bar.show_ai_credits = true`. Switching to a workspace
  the user can view billing for makes the segment appear without
  ever flashing the previous workspace's numbers.
- A12. Telemetry payload contains only `{ event:
  "status_bar.segment_clicked", segment: "ai_credits" }`. No
  used / allowance / scope name / workspace id / plan tier
  fields are emitted from segment-owned code paths.

## Implementation Pointers

These pointers reference verified paths in the current codebase.
Where no module exists yet, the pointer is marked `(new module)`.

- Usage state owner (existing, verified):
  - `app/src/ai/request_usage_model.rs` — `AIRequestUsageModel`
    is the singleton owner of `request_limit_info`,
    `bonus_grants`, `last_update_time`, GraphQL fetch via
    `refresh_request_usage_async`, and emits
    `AIRequestUsageModelEvent::RequestUsageUpdated`.
  - `app/src/lib.rs:1251` — singleton registration site:
    `ctx.add_singleton_model(|ctx| AIRequestUsageModel::new(ai_client, ctx))`.
  - Existing observer call sites the segment will mirror:
    - `app/src/terminal/buy_credits_banner.rs:75` — pattern for
      `ctx.subscribe_to_model(&AIRequestUsageModel::handle(ctx), ...)`.
    - `app/src/workspace/view/free_tier_limit_hit_modal.rs:68` —
      identical subscription pattern.
    - `app/src/terminal/input.rs:12788` — read-only access via
      `AIRequestUsageModel::as_ref(ctx).request_limit()`.
- Billing & Usage pane (existing):
  - `app/src/settings_view/billing_and_usage_page.rs`,
    `app/src/settings_view/billing_and_usage/usage_history_model.rs`
    — `UsageHistoryModel` powers usage-history table only;
    `AIRequestUsageModel` powers limit/used display. The
    status-bar chip consumes ONLY `AIRequestUsageModel`; the
    `UsageHistoryModel` is unaffected.
- Status-bar segment (new):
  - The current "context remaining" footer chip lives in
    `app/src/ai/blocklist/agent_view/agent_input_footer/mod.rs`
    (`update_context_window_button` at the agent input footer)
    and `chips.rs` next to it. The new credits segment can be
    added in a sibling module, e.g.
    `app/src/ai/blocklist/agent_view/agent_input_footer/ai_credits_chip.rs`
    `(new module)`, exposed via the existing chip selection
    pipeline used by `AgentToolbarItemKind`.
  - Existing block-level status bar:
    `app/src/ai/blocklist/block/status_bar.rs` (block-scoped, not
    the workspace footer — reference for status-bar styling
    tokens but not the integration site).
- Settings:
  - `app/src/settings/mod.rs` — register the new
    `status_bar.show_ai_credits` key here following the pattern
    used by sibling files (`accessibility.rs`, `block_visibility.rs`).
  - `app/src/settings_view/appearance_page.rs` — add the toggle
    row, with subtitle covering the signed-out / no-billing-context
    fallback.
- Agent-turn completion event channel: reuse the same channel the
  Billing pane and the conversation list already consume to drive
  the ≤2s refresh debounce. No new event channel.
- Workspace billing-context resolver: reuse the existing resolver
  that the Billing pane uses for workspace-scoped fetches; do not
  add a segment-local resolver.

If during implementation any of these paths have moved, update
the pointer in this spec rather than introducing a parallel
module to keep the no-extra-pipeline invariant intact.

## Tests

- T1. Segment formatting: numeric values, threshold colors at
  79/80/94/95/100 percent, unlimited rendering, indeterminate
  rendering. **Unlimited row sub-assertion:** with
  `is_unlimited == true`, the tooltip string equals "Unlimited
  plan" exactly (or its localized equivalent) and contains NO
  `$`, NO digit followed by a currency code, NO "USD", NO
  "month", NO "spend", and NO "$/day"-shaped substring.
- T2. Fetch debounce: rapid agent-turn completions coalesce into
  a single fetch within 2s. The segment itself MUST NOT call
  `refresh_request_usage_async` during this test — only the
  pre-existing upstream callers do.
- T3. Stale-on-error: fetch failure retains last value and
  surfaces stale indicator in tooltip. Asserted by setting
  `last_fetch_failed_at = now` on `AIRequestUsageModel` (with
  `last_update_time` still in the recent past, ≤30s ago); the
  segment renders the Stale row of B1a.i with the cached value
  intact.
- T4. **Stale-then-hidden timing — timestamp-pair driven.**
  Drive the segment state by manipulating
  `AIRequestUsageModel`'s `last_update_time` and
  `last_fetch_failed_at` fields (the segment owns no timer):
  - Set `last_update_time = now - 31s`,
    `last_fetch_failed_at = None` → assert Stale row renders.
  - Set `last_update_time = now - 6min`,
    `last_fetch_failed_at = now - 30s` → assert Failed row
    renders (segment hidden) per the B2 timestamp-pair test.
  - Fire one successful `RequestUsageUpdated` that updates
    `last_update_time = now` and clears `last_fetch_failed_at`
    → assert live render restored within one tick.
  The test MUST NOT rely on a per-segment failure counter; the
  segment exposes none.
- T5. Click routing: click navigates to Settings → Billing &
  Usage scoped to the current workspace.
- T6. Workspace-scope switch updates the segment within 2s of
  the next `RequestUsageUpdated` event firing (segment does NOT
  initiate the fetch — the workspace-billing-context resolver
  does, per B3). The tooltip shows the new scope name. During
  the in-between window the segment hides or renders the Loading
  row, never flashing the previous scope's numbers.
- T7. Toggle off (`show_ai_credits = false`) prevents rendering
  AND prevents `RequestUsageUpdated` subscription registration
  from segment-owned code (no new model-event listener attached
  when opted out). The segment never opens a billing fetch, with
  or without the toggle on — that is asserted in T10.
- T8. Right-click context menu hides the segment and persists the
  setting flip.
- T9. Display-state matrix: each row of B1a renders the exact
  glyph/text and color token specified — covers unlimited,
  metered, add-on, enterprise-with-limit, enterprise-no-limit,
  delinquent, restricted, missing-allowance, indeterminate, and
  signed-out. Unlimited-row sub-assertion is the same as T1's
  "no USD / no spend / no currency / no time-rate" prohibition.
- T10. **Single fetch, N model-event listeners.** The
  per-process single-fetch invariant in B2 is asserted directly,
  matching the model-event observer pattern defined elsewhere
  in this spec (NOT a ref-counted "billing subscription"). The
  test asserts:
  - Spawning N≥2 segments registers exactly N
    `RequestUsageUpdated` model-event listeners on the singleton
    `AIRequestUsageModel`. These are observer listeners, not
    fetch handles. There is no shared "subscription handle"
    being ref-counted on or off.
  - The underlying GraphQL fetch counter (the call count of
    `refresh_request_usage_async`) is unchanged from the N=1
    baseline regardless of how many segments exist. The chip's
    presence MUST NOT add any fetches.
  - Closing all but one segment leaves the singleton's fetch
    cadence and `RequestUsageUpdated` emission cadence
    unchanged. Closing the last segment removes the final
    `RequestUsageUpdated` listener but DOES NOT dispose any
    fetch — `AIRequestUsageModel` keeps running for the rest of
    the app (Billing pane, buy-credits banner, etc.).
  - There is no segment-owned billing-subscription object to
    dispose. The earlier "shared billing subscription
    ref-counted to last segment removed" language is replaced
    by this observer-only assertion.
- T11. Authorization gating: a user without billing-view
  permission for the active workspace never sees the segment,
  even with `status_bar.show_ai_credits = true`. Switching from a
  permitted workspace to a non-permitted one hides the segment
  without flashing previous-scope numbers.
- T12. Telemetry shape: assert click telemetry payload is exactly
  `{ event: "status_bar.segment_clicked", segment: "ai_credits" }`
  — no `used`, `allowance`, `scope_name`, `workspace_id`,
  `plan_tier`, `reset_at`, `usd_amount`, `monthly_spend`, or any
  currency / financial field.

## Resolved questions (V1 decisions)

The items below were Open Questions in round 1; each is now a V1
decision and is referenced by behavior or acceptance criteria.
This section exists for reviewer traceability only — the decision
is the spec.

- **R1. Unlimited plans, daily-spend USD.** V1: render `∞` only
  (covered by B1a.ii Unlimited row); do NOT mix in USD daily-spend
  here. Revisit when the sibling USD-spend spec (#10224) ships.
- **R2. Default visibility.** V1: opt-in for everyone, including
  metered-plan users (covered by B5 and the `false` default in
  Settings / API surface). Promote via a one-time tooltip on the
  Billing & Usage pane.
- **R3. Unlimited glyph vs. word.** V1: glyph in the chip, word
  ("Unlimited") in the tooltip and as the locale fallback when
  the glyph is unavailable (B1a.ii Unlimited row).
- **R4. Add-on threshold computation.** V1: thresholds are
  computed against `(base + addon_total)`, with `addon_total`
  being the sum of unexpired grants only (covered by B1a.iii and
  used by A9). This matches what the Billing & Usage pane treats
  as "available".

## Open Questions

(None remain that are referenced by acceptance criteria. New
items added in future revisions belong here only if they are
NOT yet referenced by acceptance.)

## Telemetry

- **Segment-emitted events (closed set).** The status-bar segment
  emits EXACTLY ONE event from segment-owned code:
  - `{ event: "status_bar.segment_clicked", segment: "ai_credits" }`
  - No additional fields. No usage totals (`used`, `allowance`,
    `remaining_pct`), no scope names, no workspace identifiers
    (workspace id, slug, billing customer id), no plan tier, no
    reset timestamps. No add-on display names, amounts, or
    expirations.
- **Fetch instrumentation is upstream-owned and out of scope for
  this spec.** The billing GraphQL fetch lives on
  `AIRequestUsageModel` and its existing callers. That code path
  already emits its own telemetry today, OWNS its event shape, and
  is NOT modified by this spec:
  - The segment does NOT add a new `surface` field to the existing
    fetch instrumentation.
  - The segment does NOT contribute a `"status_bar"` surface value
    to upstream fetch events.
  - The existing fetch event remains exactly as it is in the
    current codebase. If a future change wants to attribute fetches
    to a calling surface, that is a separate spec.
  This eliminates the prior conflict between "exactly one click
  event" and "the segment's attached observer contributes a
  surface value" — there is no observer-side fetch instrumentation.
- **Explicit non-goal:** no segment-side telemetry beyond the
  click event. Any usage-shape telemetry (allowance crossing
  thresholds, plan-tier distributions, reset timing, add-on
  inventory) remains owned by the Billing & Usage pane and its
  existing telemetry surface.
