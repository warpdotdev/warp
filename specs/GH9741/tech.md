# Show Per-Member Add-On Credit Usage Breakdown in Team Billing — Tech Spec
Product spec: `specs/GH9741/product.md`
GitHub issue: https://github.com/warpdotdev/warp/issues/9741

## Problem
The Billing and usage page can show team-wide add-on credit balance and purchase settings, and it can show per-member base request usage. It cannot currently attribute shared add-on credit consumption to individual team members because the existing client data model does not query or store per-member add-on usage. The implementation needs to add a data source for billing-cycle add-on usage entries, aggregate those entries by member, and render the result for team admins without changing billing behavior.

The key research finding for the issue comment is: this data is not already passed to the billing page today. The checked-in GraphQL schema does expose `Workspace.billingCycleUsageHistory` and `UsageEntry` fields that look suitable for per-subject credit usage, but no current client query, Rust GraphQL fragment, workspace model, or Billing and usage renderer consumes that field. If that schema field is populated with `BONUS_GRANT`/add-on credit entries for team admins, the feature is primarily a client query/model/UI change. If it is not populated or not authorized with the needed per-user subjects, a server resolver change is required before the client can ship the breakdown.

## Relevant code
- `app/src/settings_view/billing_and_usage_page.rs:201` — `BillingAndUsagePageView` owns the selected tab, usage-history model, sorting state, add-on modal state, and refresh behavior.
- `app/src/settings_view/billing_and_usage_page.rs:696` — `on_page_selected` refreshes workspace metadata, request usage, usage history, and add-on settings when the page opens.
- `app/src/settings_view/billing_and_usage_page.rs (1608-2050)` — `render_addon_credits_panel` renders shared add-on balance, purchase controls, monthly spend limit, auto reload, and purchased-this-month summary.
- `app/src/settings_view/billing_and_usage_page.rs (2466-3150)` — `UsageWidget::render` builds the Usage section, computes admin permissions, renders the add-on credits panel, and renders existing per-member base usage rows.
- `app/src/settings_view/billing_and_usage_page.rs:3298` — `sort_user_items_in_place` is the existing display-name/request sorting helper and pattern for adding add-on usage sorting.
- `app/src/settings_view/billing_and_usage/usage_history_model.rs:11` — current Usage History tab model fetches current-user conversation usage only; it is not a team billing-cycle summary.
- `app/src/settings_view/billing_and_usage/usage_history_entry.rs:72` — current usage-history entries render conversation-level credits spent for the current user.
- `app/src/workspaces/workspace.rs:34` — `Workspace` stores `members`, `billing_metadata`, `bonus_grants_purchased_this_month`, `settings`, and `total_requests_used_since_last_refresh`; it has no per-member add-on usage field.
- `app/src/workspaces/workspace.rs:187` — `WorkspaceMemberUsageInfo` stores base request usage and limits only.
- `app/src/workspaces/gql_convert.rs (123-140)` — converts GraphQL `WorkspaceMemberUsageInfo` into the app model; there is no add-on field to convert.
- `app/src/workspaces/gql_convert.rs (871-924)` — converts GraphQL `Workspace` into the app `Workspace`, including `bonus_grants_info.spending_info` for purchased-this-month totals but not usage attribution.
- `crates/graphql/src/api/workspace.rs:6` — generated GraphQL `Workspace` fragment currently queries `bonus_grants_info`, `members`, `billing_metadata`, settings, and total request usage; it does not query `billing_cycle_usage_history`.
- `crates/graphql/src/api/queries/get_workspaces_metadata_for_user.rs:142` — `GetWorkspacesMetadataForUser` is the metadata query used by `TeamClient::workspaces_metadata`.
- `crates/graphql/src/api/queries/get_request_limit_info.rs:1` — current request-limit query fetches the current user's limits and workspace bonus-grant balances, not team member add-on consumption.
- `app/src/server/server_api/team.rs:146` — `TeamClient::workspaces_metadata` fetches and converts workspace metadata.
- `crates/warp_graphql_schema/api/schema.graphql:468` — `BillingCycleUsageHistory` and `BillingCycleUsageSummary` exist in the schema.
- `crates/warp_graphql_schema/api/schema.graphql:3795` — `UsageEntry` includes `creditsUsed`, `costCents`, `costType`, `subjectDisplayName`, `subjectType`, `subjectUid`, `usageBucket`, and `usageSource`.
- `crates/warp_graphql_schema/api/schema.graphql:4026` — `Workspace.billingCycleUsageHistory` is available in the schema but unused in client code.

## Current state
The Overview tab refreshes three pieces of data when selected:

1. workspace metadata through `TeamUpdateManager::refresh_workspace_metadata`, which ultimately calls `TeamClient::workspaces_metadata`;
2. current-user request and bonus-grant balances through `AIRequestUsageModel::refresh_request_usage_async`;
3. current-user conversation usage history through `UsageHistoryModel::refresh_usage_history_async`.

The add-on credit card renders from two data sources:

- `AIRequestUsageModel::total_workspace_bonus_credits_remaining(workspace.uid)` for the current shared add-on balance.
- `Workspace.bonus_grants_purchased_this_month` for current-month purchased credits and spend.

The per-member Usage list renders from `Workspace.members[].usage_info.requests_used_since_last_refresh` and `request_limit`. Those fields represent base request usage against each member's quota; they do not distinguish whether any usage was paid for by shared add-on credits.

The schema contains a better-shaped reporting model: `Workspace.billingCycleUsageHistory` returns billing-cycle summaries containing `UsageEntry` records with a subject, usage bucket, source, cost type, credits used, and cost. That is the only checked-in client-visible API shape that appears capable of answering "which subject consumed how many add-on credits?" However, because no current Rust query fragment includes that field, the data is not already passed to the billing screen.

## Proposed changes
### 1. Add a dedicated team add-on usage query
Add a new GraphQL query module under `crates/graphql/src/api/queries`, for example `get_workspace_addon_credit_usage_breakdown.rs`, rather than expanding `GetWorkspacesMetadataForUser`.

The query should fetch the current workspace's `billingCycleUsageHistory` with the fields needed to aggregate the product spec:

- `periodStart`
- `periodEnd`
- `entries.costType`
- `entries.creditsUsed`
- `entries.costCents`
- `entries.subjectType`
- `entries.subjectUid`
- `entries.subjectDisplayName`
- `entries.usageBucket`
- `entries.usageSource`

Prefer a dedicated query because this data is used only on the Billing and usage page and may be heavier than workspace metadata. It also gives the UI an independent loading/error state so a failure to load usage attribution does not block workspace switching or add-on purchasing controls.

Open backend dependency: confirm that `billingCycleUsageHistory` returns team-scoped entries to team admins and either omits or rejects the data for non-admin users. If the existing resolver is current-user-only, excludes add-on credit cost types, or lacks per-user `subjectUid`/`subjectDisplayName`, the server must be updated before the client UI can show the requested breakdown.

### 2. Define a client model for add-on usage breakdown
Add a small model near the existing billing-and-usage models, for example `app/src/settings_view/billing_and_usage/addon_usage_breakdown_model.rs`.

Suggested model state:

- `period_start: Option<Time>`
- `period_end: Option<Time>`
- `rows: Vec<AddonUsageBreakdownRow>`
- `team_total_credits_used: i32`
- `is_loading: bool`
- `last_error: Option<String>` or an enum suitable for rendering a generic unavailable state

Suggested row shape:

- `subject_key`: stable key from subject type and UID when available
- `subject_type`: user, service account, team, or unknown
- `user_uid: Option<UserUid>` for member matching
- `display_name: String`
- `email: Option<String>` when resolvable from current workspace members
- `credits_used: i32`
- `cost_cents: Option<i32>`
- `is_current_member: bool`

Aggregation rules:

- Include only entries that represent shared add-on credit consumption. Expected filters are `costType == BONUS_GRANT` for purchased/shared add-on credits and possibly `AMBIENT_BONUS_GRANT` only if product decides ambient-only grants belong in this breakdown. Do not include `BASE_LIMIT` or `PAYG` entries in the add-on breakdown.
- Include AI-credit buckets by default. Exclude unrelated buckets unless product explicitly wants compute, voice, or suggested-code-diff add-on consumption included under the same heading.
- Group user entries by `subjectUid` when present. Fall back to subject display name only for historical/deleted users with no UID.
- Join active user rows against `Workspace.members` and `UserProfiles` to reuse the current display-name/email fallback behavior.
- Add zero rows for active workspace members who have no matching add-on entries.
- Preserve non-user entries as grouped rows so row totals reconcile with the backend total.

### 3. Add a server client method
Add a method to the appropriate client trait, likely `WorkspaceClient` in `app/src/server/server_api/workspace.rs` or `TeamClient` in `app/src/server/server_api/team.rs`, depending on whether the query is workspace-scoped or team-scoped in GraphQL.

Recommended signature:

```rust path=null start=null
async fn get_addon_credit_usage_breakdown(
    &self,
    workspace_uid: WorkspaceUid,
) -> Result<WorkspaceAddonCreditUsageBreakdown>;
```

If the GraphQL field is only available from the currently selected user's workspace list and does not take a workspace UID argument, the method can still accept the UID and filter the returned workspace client-side. Return an authorization/unavailable error when the current user cannot access the field so the UI can render the generic unavailable state.

Add mocks for tests alongside the existing `MockWorkspaceClient`/`MockTeamClient` patterns.

### 4. Wire refresh and permissions into `BillingAndUsagePageView`
Instantiate the new model in `BillingAndUsagePageView::new` next to `UsageHistoryModel`. On page selection and refresh:

- call the breakdown model refresh only when there is a current workspace and current team;
- call it only when the current user has admin permissions;
- call it only when the plan can purchase/consume add-on credits or when the workspace has add-on usage history to display;
- refresh it alongside `TeamUpdateManager::refresh_workspace_metadata` and `AIRequestUsageModel::refresh_request_usage_async`.

Do not fetch the full-team breakdown for non-admin users. Client gating is not a security boundary, so the backend must also enforce this, but avoiding the request keeps behavior and telemetry cleaner.

### 5. Render the breakdown near the add-on credits card
Add rendering helpers in `billing_and_usage_page.rs` or a sibling file if the page becomes too large:

- `render_addon_usage_breakdown_card`
- `render_addon_usage_breakdown_row`
- `render_addon_usage_breakdown_empty_state`
- `render_addon_usage_breakdown_unavailable_state`

Place the card directly under the existing `render_addon_credits_panel` output in `UsageWidget::render` when the current user is an admin and add-on credits are relevant for the current plan.

Reuse existing UI patterns:

- `render_ai_usage_limit_row` is a useful row-layout reference, but do not overload its `Divisor` semantics because add-on usage has no per-member quota.
- `UserSortingCriteria` and `sort_user_items_in_place` can be generalized or mirrored for add-on rows.
- `thousands::Separable` is already imported for comma-separated credit counts.
- Use the existing theme, surface, border, and loading placeholder patterns from the add-on credits and usage-history UI.

The card should not replace the existing Usage list. The existing list answers "how much of each member's personal/base quota has been used"; the new card answers "how much shared add-on credit each member consumed."

### 6. Sorting and totals
Introduce an add-on-specific sort state if the breakdown has its own sort menu, or extend the current sorting menu carefully if product wants one sort control for both lists.

Recommended first implementation:

- default sort by add-on credits descending;
- support name ascending/descending and add-on credits ascending/descending;
- tie-break by display name;
- include zero rows after non-zero rows for the default sort;
- compute the visible total from the rows after aggregation, not from a separate UI-only counter.

If backend returns a total that differs from the row sum due to hidden entries or rounding, prefer preserving reconciliation by adding non-user/unknown rows rather than silently dropping those credits.

### 7. Feature flag and rollout
Use an existing billing/Build-plan rollout mechanism if one exists; otherwise add a new feature flag for the UI surface. The data model can be introduced safely behind the page/admin/plan gates, but the rendered card should be releasable independently in case backend data needs staged rollout.

Telemetry should be minimal:

- card loaded successfully
- card load failed
- sort option selected

Do not log member names, emails, UIDs, or raw usage amounts in telemetry.

## End-to-end flow
1. A team admin opens Settings → Billing and usage.
2. `BillingAndUsagePageView::on_page_selected` refreshes workspace metadata, current request usage, usage history, and the new add-on usage breakdown.
3. The new model calls the GraphQL query for the current workspace billing-cycle usage history.
4. The model filters entries to shared add-on credit usage, groups by subject, joins active users to workspace members and user profiles, inserts zero rows for active members, and computes totals.
5. `UsageWidget::render` renders the existing Add-on credits card, then the new breakdown card.
6. The admin can sort rows and refresh the data without affecting purchase, auto-reload, or spend-limit controls.
7. A non-admin member opens the same page and does not trigger the full-team query or render the card.

## Risks and mitigations
### Risk: existing schema field is insufficient
`Workspace.billingCycleUsageHistory` exists in the schema, but the client repository does not prove that the resolver returns per-member add-on credit entries for team admins.

Mitigation: make backend verification the first implementation step. If the field is insufficient, extend the server resolver or add a dedicated backend field before building the UI. Keep the client model isolated so only the query adapter changes when the server shape is finalized.

### Risk: confusing base usage with add-on usage
The current Usage list already shows per-member request usage and limits. Adding another per-member card can confuse admins if labels are too similar.

Mitigation: label the new card and rows explicitly as add-on/shared credits consumed this billing cycle. Keep the existing base Usage list unchanged and avoid using `request_limit` divisors in add-on rows.

### Risk: non-admin exposure of team usage
The data is billing-sensitive because it attributes team spend to members.

Mitigation: gate client fetch/render on `Team::has_admin_permissions`, and require server-side authorization for the query. Add tests proving the client does not request or render the model for non-admin users.

### Risk: totals do not reconcile
Deleted users, service accounts, team-level usage, or rounding can make member-only row sums differ from backend totals.

Mitigation: preserve non-user and unknown-subject rows rather than dropping them. Use integer backend credit counts when possible. If fractional credits are introduced, centralize formatting and total calculation in the model.

### Risk: metadata query becomes too heavy
Adding billing-cycle history to `GetWorkspacesMetadataForUser` would make every workspace refresh heavier, even when the billing page is closed.

Mitigation: use a dedicated query/model that only runs on the Billing and usage page for admins.

## Testing and validation
### Unit tests
- Aggregation filters include add-on/bonus-grant usage and exclude base-limit and PAYG entries.
- Aggregation groups multiple entries for the same user and sums credits/cost.
- Active members with no add-on usage receive zero rows.
- Former/deleted users and non-user subjects are preserved in separate rows.
- Display name fallback uses user profile display name, then email, then backend subject display name.
- Sorting handles credits descending/ascending, display name ascending/descending, ties, and zero rows.
- Permission gating prevents non-admin refresh/render.

### Model and UI tests
- Loading state renders while the model is fetching.
- Empty state renders when all active members have zero add-on usage.
- Unavailable state renders when the query fails without blocking the existing add-on controls.
- Populated state renders team total and rows with formatted credits and percentage share.
- Refresh action updates the model without resetting unrelated billing page state.

### GraphQL/client tests
- New query compiles against `crates/warp_graphql_schema/api/schema.graphql`.
- Conversion from GraphQL usage entries to app model handles nullable `subjectUid` and `subjectDisplayName`.
- Mock client tests cover authorization/unavailable errors.

### Manual validation
- On a Build or Business test team, consume base quota for at least two members and then consume shared add-on credits. Confirm the admin sees both members' add-on usage and the correct team total.
- Confirm a member with no add-on usage appears as zero or under the collapsed zero-member affordance.
- Confirm a non-admin team member does not see the breakdown.
- Confirm existing add-on balance, purchase, auto-reload, monthly spend limit, delinquency warning, and enterprise limited-reporting callout behavior is unchanged.

## Follow-ups and open technical questions
- Confirm whether `AMBIENT_BONUS_GRANT` should be included in this add-on breakdown or kept separate from purchased add-on credits.
- Confirm whether `billingCycleUsageHistory` supports selecting a specific workspace when a user belongs to multiple workspaces, or whether the client must filter the returned workspace list.
- Confirm whether the backend can return historical subject display names for removed members.
- Decide whether to keep the model local to the settings page or promote it if future admin/reporting surfaces need the same data.
