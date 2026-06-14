# Show Per-Member Add-On Credit Usage Breakdown in Team Billing — Product Spec
GitHub issue: https://github.com/warpdotdev/warp/issues/9741
Figma: none provided

## Summary
Add a team-admin-visible breakdown of add-on credit usage to Settings → Billing and usage so team managers can see which members are consuming the shared add-on credit pool during the current billing cycle. The billing page already shows team-level add-on credit balance, purchasing, auto-reload, and per-member base AI credit usage; this feature makes shared add-on consumption attributable to individual members in the same workflow.

## Problem
Build and Business teams can purchase shared add-on credits that are consumed after a member exhausts their personal/base quota. Today the Billing and usage screen shows aggregate team add-on credit state, including remaining shared balance and credits purchased this month, but it does not show which team members have consumed those shared credits. Admins therefore cannot identify heavy users, explain team spend, or manage add-on credit purchasing and monthly spend limits with enough context.

The existing per-member Usage list can be mistaken for this answer because it shows each member's total AI usage against their personal quota, but it does not distinguish base-plan usage from add-on credit usage. The new experience must make that distinction explicit.

## Goals
- Show team admins a per-member add-on credit consumption breakdown for the current billing cycle.
- Make the relationship between the shared add-on pool and individual member usage clear: the team owns the add-on balance, while individual members consume from it after their base quota is exhausted.
- Keep the current team-level add-on credit balance, purchased-this-month, monthly spend limit, auto-reload, and manual purchase flows intact.
- Use the same member identity conventions as the existing Usage list: display name when available, email as fallback.
- Provide understandable empty, loading, refresh, and unavailable states so admins can distinguish "no add-on usage yet" from "usage data could not be loaded."
- Avoid exposing team-wide member usage data to users who are not team admins.

## Non-goals
- Changing how credits are metered, billed, refunded, or deducted.
- Changing the add-on credit purchase, auto-reload, monthly spend limit, or Stripe billing portal flows.
- Showing per-conversation or per-request add-on attribution in this view. The Usage History tab can remain conversation-level for the current user.
- Showing dollar-cost allocation per member as the primary behavior. If cost is available it may be shown as secondary metadata, but the required breakdown is in credits.
- Exporting the breakdown to CSV or external reporting tools.
- Backfilling historical add-on usage before the backend data source supports it.
- Showing this team-wide breakdown to non-admin team members.

## Figma / design references
No Figma mock was provided. The implementation should follow the existing Billing and usage card, row, typography, sorting, and loading-state patterns unless design provides a mock before implementation.

## User experience
1. Team admins on plans that can purchase or consume shared add-on credits see an add-on usage breakdown in Settings → Billing and usage → Overview.
   - The breakdown appears near the existing Add-on credits card because it explains consumption of the same shared balance.
   - The existing Add-on credits card still shows the current shared balance and purchasing controls first.
   - The existing Usage section still shows base credit usage and limits for members.

2. The breakdown is scoped to the current billing cycle by default.
   - The visible heading or helper text makes the period clear, for example "Add-on usage this billing cycle."
   - If the backend supplies the billing-cycle reset or period-end timestamp, the UI shows the same reset date style used elsewhere on the page.
   - The numbers refresh when the admin opens the page or presses the existing refresh control.

3. Each member row shows:
   - member display name, falling back to email when display name is unavailable or blank
   - add-on credits consumed in the current billing cycle
   - the member's share of total team add-on credits consumed in the current billing cycle when total consumed is greater than zero
   - an optional secondary email label when the display name is present and the row needs disambiguation

4. A team total row is shown when at least one member has add-on usage in the current billing cycle.
   - The total equals the sum of all visible member add-on credits.
   - The total is labeled as shared add-on credits consumed, not remaining balance.
   - The total is visually distinct from the existing base-usage "Team total" row so admins do not confuse the two metrics.

5. Rows with zero add-on usage are included by default for current team members so admins can see that a member has not consumed the shared pool.
   - Zero-usage rows sort after non-zero rows when sorting by add-on usage descending.
   - If the team is large enough that showing every zero row would make the card unwieldy, the implementation may collapse zero-usage rows behind a "Show all members" affordance, but the initial state must still make it clear how many members have zero add-on usage.

6. Sorting behavior:
   - The breakdown supports sorting by member name and add-on credits used.
   - The default sort is add-on credits descending, with the current user pinned first only if the existing Usage list keeps that invariant for the same list pattern. If pinning the current user would hide the highest consumer, the row must still make the usage ranking understandable.
   - Ties are broken by display name, case-insensitively.

7. Empty state:
   - When the team has shared add-on credits enabled or available but no member has consumed any add-on credits in the current billing cycle, the card shows a short empty message such as "No add-on credits used this billing cycle."
   - The empty state must not imply that the team has no add-on balance or cannot buy add-on credits; it only describes consumption.

8. Unavailable state:
   - If the plan can purchase or consume add-on credits but per-member usage cannot be loaded, admins see a non-blocking unavailable message in the breakdown area and can still use existing billing controls.
   - The message should not expose raw backend errors. It should say that per-member add-on usage is temporarily unavailable and suggest refreshing or trying again later.

9. Loading state:
   - The breakdown shows a lightweight loading state while the data is being fetched.
   - Existing add-on balance and purchase controls remain visible if their data is already available.

10. Permission behavior:
    - Team admins and owners can see the full team breakdown.
    - Non-admin members do not see the team-wide breakdown. They continue seeing their own usage and existing non-admin billing guidance.
    - If a non-admin member has personally consumed add-on credits, this feature does not add a new personal-only breakdown in the billing page.

11. Plan behavior:
    - Build and Business teams with add-on credit purchasing enabled show the breakdown.
    - Teams that can upgrade to a plan with add-on credits but cannot currently purchase them do not show an empty per-member breakdown; they continue seeing the existing upgrade guidance.
    - Enterprise PAYG teams keep the existing limited reporting callout unless the backend explicitly supports equivalent member-level add-on credit attribution for that plan. This feature should not replace enterprise admin-panel reporting.

12. Privacy and identity behavior:
    - Member names and emails shown in the breakdown follow the same visibility rules as the existing team member usage list.
    - Removed members who consumed credits during the current cycle should not appear as active team members unless the backend returns historical subject display names for them. If historical users are returned, they are grouped under a clearly labeled former/deleted member row rather than being silently dropped from the total.
    - Service-account or team-level usage entries, if returned by the backend, are grouped under non-user rows such as "Service accounts" or "Team-level usage" so the visible member total plus non-member rows still matches the team total.

13. Number formatting:
    - Credit counts use the same comma-separated formatting as the rest of the Billing and usage page.
    - Fractional credits, if returned by the backend, are rounded consistently with existing credit display helpers. The UI must not round in a way that makes row totals visibly disagree with the team total.
    - Percent share is hidden when total consumed is zero.

## Success criteria
- A team admin on a Build or Business team can open Settings → Billing and usage and identify how many shared add-on credits each team member consumed in the current billing cycle.
- The admin can distinguish base usage from add-on usage without reading implementation-specific terminology.
- Existing add-on credit purchase, auto-reload, monthly spend limit, and balance behavior is unchanged.
- Non-admin team members cannot see a full-team per-member add-on breakdown.
- The screen handles no-usage, loading, backend-unavailable, and mixed active/former-member data without misleading totals.
- The total add-on credits consumed shown in the breakdown matches the sum of row values and reconciles with the backend-provided billing-cycle usage data.

## Validation
- Unit-test the row aggregation and sorting rules with active users, zero-usage members, former users, service-account/team-level entries, ties, and missing display names.
- Unit-test permission gating so only team admins/owners receive or render the full-team breakdown.
- Add view/model tests for empty, loading, error, and populated states.
- Manually validate with a Build or Business test team where at least two members have consumed shared add-on credits and one member has not.
- Manually validate that non-admin members on the same team do not see the full breakdown.
- Manually validate that the existing add-on balance, purchase, auto-reload, monthly spend limit, and base Usage list still render correctly.

## Open questions
- Should the breakdown live inside the existing Add-on credits card, immediately below it as a separate card, or inside a new tab if design provides a higher-fidelity layout?
- Should zero-usage members always be shown, or collapsed after a threshold for larger teams?
- Should the default sort prioritize highest add-on usage or preserve the current-user-pinned pattern from the existing Usage list?
- Should cost in dollars be shown as secondary metadata when the backend returns `costCents`, or should the first version stay credits-only?
- If the backend returns historical usage for users who left the team during the billing cycle, what exact label should the UI use for those rows?
