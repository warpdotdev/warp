# Spec: Free-plan ad-hoc credit purchases in the Warp desktop client

Linear issue: [REV-1792](https://linear.app/warpdotdev/issue/REV-1792/allow-free-plan-users-to-purchase-ad-hoc-credits-10percent-markup)

## Product

### Summary

Logged-in Free-plan users can purchase the existing add-on credit packs from the Warp desktop client. Free-plan pricing includes a server-authored 10% markup, shown as base price plus markup plus total price, alongside an upgrade-to-save action. Paid-plan pricing and existing pack denominations remain unchanged.

This spec covers only the `warpdotdev/warp` desktop client. Billing, Stripe, team creation, and the GraphQL contract are owned by the paired `warpdotdev/warp-server` spec and implementation.

### Goals

- Make server-eligible Free users able to buy one-time add-on credit packs from desktop purchase surfaces.
- Show the exact server-authored base price, Free-plan markup, and total before purchase.
- Surface the same choice during onboarding without creating a parallel onboarding flow.
- Preserve paid-plan purchases, volume discounts, spending limits, auto-reload behavior, and user-facing errors.

### Non-goals

- Renaming credits to usage or converting credits to true API cost.
- Changing the credit ratio, included usage, or existing credit balances.
- Adding variable purchase amounts or new pack denominations.
- Enabling Free-plan auto-reload.
- Tracking historical “you could have saved $X,” refunding prior markup after upgrade, or storing markup consumption.
- Adding enterprise, US-hosted-inference, per-model, or promotional markup.
- Adding a credits-versus-dollars display setting.
- Implementing server billing, Stripe, or team-creation behavior in this repository.

### Key design choices

1. The desktop client renders prices supplied by the server and never calculates or rounds the 10% markup locally.
2. Existing access-choice slides are extended instead of adding a new onboarding step: `OfferSlide` for account-first onboarding and `AiAccessSlide` for the fallback Warp Agent path.
3. Free one-time purchase eligibility comes only from `PurchaseAddOnCreditsPolicy.enabled`; markup visibility comes from a nonzero server-authored `markupUsdCents`. Free auto-reload remains hidden and disabled.
4. The current Settings → Billing & Usage v2 card remains the canonical full purchase surface. The out-of-credits banner and onboarding reuse the same pack data and price semantics rather than introducing separate pricing logic.

### Behavior

1. **Eligibility is server-driven.** A logged-in user sees an actionable one-time purchase experience only when the current purchase context has `PurchaseAddOnCreditsPolicy.enabled == true` and at least one server-provided pack. A missing policy, `enabled == false`, or no purchase context must not be treated as eligible.
2. **The existing packs are preserved.** Every desktop surface lists the server-provided `credits` denominations without adding, removing, or synthesizing pack amounts.
3. **Free pricing is explicit.** When a selected pack has `markupUsdCents > 0`, the user sees all three values before confirming:
   - base price from `basePriceUsdCents`;
   - a clearly labeled “Free plan markup” amount from `markupUsdCents`;
   - total charged from `totalPriceUsdCents`.
4. **The server is authoritative.** The client formats integer cents as currency but never computes 10%, derives the markup as `total - base`, or rewrites any server amount. The purchase mutation identifies the selected pack by `credits`; it does not submit a client-computed price. `teamUid` is optional so the server can create the standard personal workspace for a teamless Free user.
5. **The upgrade incentive is contextual.** A marked-up pack includes an upgrade-to-save action that reuses the existing upgrade flow. The action may state that upgrading avoids the current pack’s displayed markup, but it must not imply historical savings or refunds. Final placement and copy must follow approved design mocks.
6. **Paid pricing does not change.** A paid-plan pack has `markupUsdCents == 0` and `totalPriceUsdCents == basePriceUsdCents`; the client shows the existing single-price treatment without a Free-plan markup row or upgrade-to-save action.
7. **All desktop purchase surfaces agree.** Settings → Billing & Usage v2, the out-of-credits banner, and onboarding show the same selected denomination and server-authored total. Existing volume-discount badges continue to use base pack pricing so the Free markup does not alter the advertised pack discount.
8. **Spending-limit prechecks use the charge total.** Any desktop precheck that compares a selected purchase against a monthly spend limit uses `totalPriceUsdCents`, not the base price. The server remains authoritative and may still reject a purchase.
9. **Free auto-reload stays out of scope.** Enabling one-time purchases for a Free tier must not expose, enable, or mutate auto-reload controls. Existing auto-reload UI and behavior remain unchanged for eligible paid plans.
10. **Account-first onboarding uses the existing post-auth offer.** For a user classified as `FreeIcp` or `FreeStandard`, `OfferSlide` adds “Buy credits as needed” alongside the existing subscription and set-up-later choices. Selecting it reveals the approved pack selector and price breakdown on that slide; it does not insert a new `OnboardingStep`.
11. **Fallback onboarding uses the existing AI-access choice.** When `AccountFirstOnboarding` is disabled, a logged-in `OnboardingAuthState::FreeUser` on the Warp Agent path can select a credits option from `AiAccessSlide`. Logged-out users do not see an actionable purchase control.
12. **Onboarding is not purchase-gated.** Users can still upgrade or continue with the existing set-up-later path. A missing or still-loading purchase context does not block onboarding completion.
13. **Purchases are single-submit.** While the mutation or a returned Stripe Checkout is pending, the initiating button is disabled, its state is visibly loading, pack selection cannot trigger a second mutation, and other existing navigation semantics remain deterministic.
14. **Free purchase uses Stripe Checkout.** For a logged-in Free user, `PurchaseAddonCreditsCheckoutRequired` opens its server-provided `url` in the default browser and retains the returned `teamUid` as the purchase target for refreshes. The desktop shows “Complete purchase in your browser” and does not report success from the redirect alone.
15. **Browser return is reconciled before success.** When Warp becomes active after Checkout, it refreshes workspace/billing metadata and AI credit usage for the returned team. It shows success and advances/dismisses only after refreshed server state confirms the selected credits were granted. If Checkout was canceled or the grant is not yet visible, the surface remains available and must not claim success; bounded refresh/retry behavior follows the approved design.
16. **Paid success remains immediate.** Existing paid plans continue to receive `PurchaseAddonCreditsOutput` and retain the current immediate metadata refresh, success confirmation, and Settings/banner dismissal behavior.
17. **Errors are actionable and retryable.** A server `UserFacingError`, including a Checkout-creation failure, is shown without replacing it with a generic client message. The user stays on the purchase surface, the pending state clears, and the user can retry, upgrade, or continue onboarding without purchasing.
18. **Unavailable data is not fabricated.** While policy or pricing is loading, the surface shows an approved loading treatment or keeps purchase disabled. Empty, missing, negative, or internally inconsistent price data is not rendered as `$0.00`; the purchase action remains unavailable and a safe error is surfaced.
19. **Design approval precedes UI implementation.** The Pricing Transparency plan identifies the new ad-hoc purchase UI as still needing design. Implementation must not invent the three-option onboarding layout, price-breakdown hierarchy, CTA copy, browser-pending/canceled state, loading state, or error placement. Approved mocks are required before the user-facing portion is considered complete.
20. **Accessibility and input parity are retained.** New choices and actions have keyboard navigation, Enter activation, visible focus/selection state, pointer interaction, and readable labels consistent with the existing onboarding cards and purchase controls.

## Tech

### Context

References are pinned to `warpdotdev/warp` commit `b04a0c2a480c30befcf48d847541651d1bbe6af6`.

- `crates/graphql/src/api/billing.rs:98-112` defines `Tier.purchase_add_on_credits_policy`; `crates/graphql/src/api/billing.rs:227-231` currently models that policy as only `enabled`.
- `crates/graphql/src/api/billing.rs:280-296` defines `AddonCreditsOption` with only `credits` and `price_usd_cents`; all current desktop prices flow through this type.
- `crates/graphql/src/api/mutations/purchase_addon_credits.rs:6-40` binds the team-scoped mutation and returns either success or a typed user-facing error.
- `app/src/pricing/mod.rs:7-67` stores the latest `PricingInfo` and exposes add-on pack options to desktop surfaces.
- `app/src/settings_view/billing_and_usage_page_v2.rs:1049-1167` gates the v2 purchase card on the tier policy, derives the selected price, and prechecks the monthly limit. Free workspaces already dispatch to this v2 page (`app/src/settings_view/billing_and_usage_dispatch_tests.rs:26-33`).
- `app/src/settings_view/billing_and_usage_page_v2.rs:1510-1628` renders the selected pack and one-time purchase action.
- `app/src/terminal/buy_credits_banner.rs:293-370` builds the pack dropdown, and `app/src/terminal/buy_credits_banner.rs:488-690` renders and submits the out-of-credits purchase.
- `app/src/ai/request_usage_model.rs:584-657` uses the tier policy to decide whether the out-of-credits banner is eligible.
- `app/src/workspaces/user_workspaces.rs:1373-1418` and `app/src/server/server_api/workspace.rs:136-190` execute a purchase and propagate workspace refreshes or user-facing failures.
- `crates/onboarding/src/model.rs:113-124`, `crates/onboarding/src/model.rs:878-1009`, and `crates/onboarding/src/model.rs:1100-1185` define the finite onboarding steps, routing, and progress indicators.
- `crates/onboarding/src/slides/offer_slide.rs:31-124` models the account-first Free-user offer variants; `crates/onboarding/src/slides/ai_access_slide.rs:43-61` models the fallback Warp Agent access choices.
- `crates/onboarding/src/agent_onboarding_view.rs:69-96`, `crates/onboarding/src/agent_onboarding_view.rs:238-283`, and `crates/onboarding/src/agent_onboarding_view.rs:642-800` own slide handles, event forwarding, rendering, and keyboard dispatch.
- `app/src/root_view.rs:2788-2800` completes account-first onboarding after an existing Free-user offer selection, making the root view the correct boundary for app-owned purchase state and mutations.

### Cross-repository server contract

The paired `warpdotdev/warp-server` spec owns the contract and billing implementation. The desktop implementation depends on the following GraphQL semantics:

- Each `AddonCreditsOption` exposes:
  - `credits`;
  - `basePriceUsdCents`;
  - `markupUsdCents`;
  - `totalPriceUsdCents`.
- For Free-plan callers, `markupUsdCents` is the server-calculated 10% purchase markup and `totalPriceUsdCents` is the exact Stripe charge total.
- For paid-plan callers, `markupUsdCents == 0` and `totalPriceUsdCents == basePriceUsdCents`.
- `PurchaseAddOnCreditsPolicy.enabled` remains the entitlement gate.
- `PurchaseAddonCreditsInput.teamUid` is optional. When omitted for a logged-in teamless Free user, the server idempotently creates the standard non-discoverable team/workspace and returns the resolved team identity.
- `PurchaseAddonCreditsResult` adds `PurchaseAddonCreditsCheckoutRequired { url, teamUid, responseContext }`. Free purchases return this variant and complete through the server-authored one-time Stripe Checkout URL. Paid purchases keep the existing immediate `PurchaseAddonCreditsOutput`.
- The desktop opens only the returned Checkout `url`, retains the returned `teamUid` for refresh/reconciliation, and never constructs a Stripe URL.
- Checkout line items contain the server-authored base price plus 10% markup. The desktop does not submit line-item or price data.
- Checkout creation or billing failures return a stable `UserFacingError`; the client displays that message and does not infer Stripe readiness.
- Integer-cent amounts already embody the server’s rounding rule. The desktop does not independently round or validate the 10% relationship.

If the server contract changes materially, update this spec and the generated schema binding together before implementation proceeds.

### Design alternatives

- **Onboarding placement**
  - **Selected: extend `OfferSlide` and `AiAccessSlide`.** These slides already own the decision between subscription and continuing without a plan, already have upgrade routing and telemetry, and know whether the user is logged-in Free. An inline pack choice keeps the purchase in the context where the user chooses AI access.
  - **Rejected: add a new `OnboardingStep`.** A dedicated slide would require duplicated forward/back/progress routing across account-first and fallback variants, would be shown outside the existing access decision, and would add a mandatory-looking step to an optional purchase.
  - **Rejected: route onboarding directly to Settings.** This technically exposes purchasing but breaks the onboarding flow, loses onboarding-specific loading/error/completion behavior, and does not meet the requirement that the choice surfaces during onboarding.
- **Price calculation**
  - **Selected: server-authored base, markup, and total fields.** This keeps Stripe charging, rounding, UI, receipts, and spending-limit checks aligned.
  - **Rejected: calculate 10% in Rust.** Independent client rounding can disagree with the charge, and old clients would encode billing policy.
  - **Rejected: derive markup from total minus base.** This hides the semantic contract and makes validation of malformed responses ambiguous.
- **UI reuse**
  - **Selected: share semantic pricing state, not a single renderer.** `PricingInfoModel` and a small pure presentation helper in `app/src/pricing/` provide consistent selected-pack semantics to Settings and the terminal banner. The app root maps the same server fields into an onboarding-owned DTO because the `onboarding` crate cannot depend on the app crate.
  - **Rejected: move all purchase rendering into `onboarding`.** Settings and terminal surfaces have different layout and ownership constraints, and the onboarding crate must remain reusable without app dependencies.
- **Eligibility signal**
  - **Selected: policy for purchase eligibility, markup amount for marked-up presentation.** This follows the existing entitlement pattern and avoids client plan-name branching.
  - **Rejected: key behavior directly on `CustomerType::Free`.** That duplicates rollout policy in the client and can expose UI before the server can fulfill it.
- **Free auto-reload**
  - **Selected: hide Free auto-reload while leaving paid behavior intact.** This keeps the ticket to one-time packs and prevents the broader M4 behavior from leaking in through the existing shared policy.
  - **Rejected: let `PurchaseAddOnCreditsPolicy.enabled` implicitly enable auto-reload.** The policy currently gates both nearby paths in client code, but Free auto-reload is explicitly outside this milestone.

### Proposed changes

#### GraphQL and pricing state

1. Update the checked-in GraphQL schema and `AddonCreditsOption` query fragment to consume `basePriceUsdCents`, `markupUsdCents`, and `totalPriceUsdCents`. Remove desktop reads of the ambiguous `priceUsdCents` only after the paired server schema has landed; coordinate branch ordering so generated queries compile against the server contract.
2. Add pure helpers in `app/src/pricing/` for:
   - selected pack lookup;
   - formatted base, markup, and total values;
   - `has_markup`;
   - base-price rate used by existing volume-discount calculations;
   - total price used by spend-limit prechecks.
   The helper must not calculate a markup amount.
3. Keep `credits` as the purchase identifier, make `teamUid` optional in the generated input, and add `PurchaseAddonCreditsCheckoutRequired` to the result binding. Treat unknown future union variants as a safe error, not immediate success.

#### Settings → Billing & Usage v2

1. Extend `AddonCreditsPurchaseState` with separate base/markup/total presentation values and an explicit one-time-purchase eligibility state.
2. Let policy-enabled Free users enter `AddonCreditsPanelState::Purchase` even when they are not part of a multi-person paid team. Preserve permission checks for shared paid teams.
3. Render the approved markup breakdown and upgrade-to-save action only for `has_markup`.
4. Use `totalPriceUsdCents` for monthly-limit prechecks and keep the server error authoritative.
5. Hide auto-reload controls for marked-up Free purchases. Do not mutate existing paid auto-reload settings or copy.
6. Leave the legacy Billing & Usage page unchanged for paid legacy plans unless the server contract removes the old price field globally; if schema compatibility requires touching it, preserve its zero-markup single-price behavior.

#### Out-of-credits banner

1. Continue gating the banner through `PurchaseAddOnCreditsPolicy.enabled`.
2. Update dropdown labels and the selected purchase summary to use the server-authored total and expose the approved Free markup treatment without crowding or truncating the responsive banner.
3. Use the selected total for monthly-limit checks.
4. Add the upgrade-to-save action for marked-up options. For Checkout-required results, keep the banner visible in a browser-pending state until refreshed server state confirms the grant; do not dismiss on redirect. Keep existing paid-plan discount badges, purchase telemetry, immediate-success dismissal, auto-reload experiments, and error handling unchanged; Free users must not see the auto-reload experiment controls.

#### Onboarding

1. Add an onboarding-owned credit-pack DTO containing `credits`, base price cents, markup cents, and total cents, plus a purchase-context state (`loading`, `eligible`, `ineligible`, `error`). The DTO contains server values; it does not derive markup.
2. Extend `AgentOnboardingView` with methods for the app root to update purchase context and mutation/Checkout/reconciliation state, and with events for selecting a pack, requesting purchase, requesting upgrade, and completing after a confirmed grant.
3. In account-first onboarding, extend `OfferSlide` for both Free variants with a third “Buy credits as needed” choice. Keep Subscription recommended and preserve Set up later.
4. In fallback onboarding, extend `AiAccessChoice` and `AiAccessSlide` with the credits choice only for logged-in Free users with eligible purchase context.
5. Keep purchase selection inline on the existing slide; do not add an `OnboardingStep`, change progress-dot counts, or alter unrelated path routing.
6. Route the purchase event through `AgentOnboardingView` to `RootView`, where `UserWorkspaces` and app-owned pricing/workspace state already live. Do not make the `onboarding` crate depend on the app crate or GraphQL client.
7. When a Free purchase returns Checkout-required, open the returned URL, store the returned team and selected pack in transient onboarding state, and show the browser-pending treatment. On app activation, `RootView` refreshes that team’s workspace metadata and AI credit usage. Only a server-confirmed grant clears pending state, shows confirmation, and invokes the existing completion transition. Cancellation/no confirmed grant returns to a retryable choice without advancing.
8. Add canonical onboarding telemetry for choice shown, pack selected, purchase started, purchase succeeded, purchase failed, and upgrade-to-save selected. Do not include prices, team identifiers, email addresses, or raw error text in telemetry.

#### Error and stale-state handling

1. Change the workspace client result from “refreshed metadata or error” to an explicit outcome that distinguishes immediate paid success from Free Checkout-required. Preserve typed `UserFacingError` handling; remove the current generic network wrapper where it would discard a server-provided Checkout-creation message.
2. When pricing or policy refreshes while a surface is open, preserve the selected denomination by `credits` when it still exists. Otherwise select the first valid server option and re-render before enabling purchase.
3. If policy becomes disabled, the selected option disappears, or returned amounts are invalid while a purchase is not pending, disable purchase and move to the safe ineligible/error state.
4. Do not cancel an in-flight mutation or Checkout because metadata refreshed. On app activation, reconcile the returned team against fresh server state; ignore duplicate activation/completion events and never infer success solely because the browser was opened.

### Open questions resolved

- **Desktop onboarding placement:** modify `OfferSlide` and `AiAccessSlide`; do not create a new slide or `OnboardingStep`.
- **Markup display:** show server-authored base price, “Free plan markup,” and total for the selected pack.
- **Client/server calculation boundary:** the server calculates and rounds all amounts; desktop only formats them.
- **“You could have saved $X” history:** out of scope. The CTA may reference the current pack’s displayed markup only.
- **Markup timing:** markup is represented at purchase, not during credit consumption.
- **Teamless users:** desktop omits `teamUid`; the server idempotently creates the standard non-discoverable workspace and returns the resolved `teamUid` in Checkout-required.
- **Free purchase mechanism:** Free users complete a server-created one-time Stripe Checkout in the browser; paid users retain immediate purchase.
- **Checkout completion:** browser launch is not success. Desktop refreshes the returned team on app activation and advances only after server state confirms the credit grant.
- **Free auto-reload:** out of scope and hidden even though existing client code places auto-reload beside one-time purchase.
- **Onboarding purchase completion:** successful purchase completes the access-choice portion of onboarding; purchasing remains optional.
- **Design mocks:** required before implementing the user-facing layout and copy. This is an explicit UI dependency, not permission to invent the surface.
- **Rollout:** no client plan-name check is introduced. Server policy controls eligibility, while existing onboarding feature flags continue to select account-first versus fallback placement.

### Risks and mitigations

- **Money displayed differs from money charged.** Mitigate by rendering only server-authored integer-cent fields and submitting only the pack identifier.
- **Free eligibility accidentally enables auto-reload.** Mitigate with an explicit Free/marked-up presentation branch and tests that auto-reload controls remain absent.
- **One surface shows stale or base-only pricing.** Mitigate with shared pricing helpers and cross-surface tests using the same pack fixture.
- **Monthly-limit UI undercounts the markup.** Mitigate by making all prechecks use `totalPriceUsdCents` and testing the boundary.
- **Onboarding state-machine regressions.** Mitigate by keeping the existing steps and adding reversible-path, loading, error, success, and duplicate-submit tests.
- **Teamless users lack a `teamUid`.** Mitigate by omitting it, accepting only the server-returned resolved team, and testing idempotent repeated attempts.
- **Browser return is ambiguous or delayed.** Mitigate by retaining a transient pending purchase, performing bounded server refreshes on activation, showing a retryable incomplete/canceled state, and never equating redirect with success.
- **Schema rollout ordering breaks older clients or the build.** Coordinate with the server PR, preserve compatibility fields during rollout if required, and compile the generated query fragments against the landed schema.
- **Design is missing.** UI implementation and visual verification remain incomplete until approved mocks are available; schema/state work may proceed independently.

## Validation and verification criteria

Every criterion must pass before the implementation PR is marked ready.

1. **GraphQL contract compiles.** A focused `warp_graphql` test or compile fixture confirms that `AddonCreditsOption` requests `credits`, `basePriceUsdCents`, `markupUsdCents`, and `totalPriceUsdCents`; `teamUid` is optional; and the purchase union handles immediate success, `PurchaseAddonCreditsCheckoutRequired`, `UserFacingError`, and unknown variants safely. Run `cargo nextest run -p warp_graphql` and a build of the affected packages. Verifies Behaviors 2–4 and 14–18.
2. **Pricing helpers use server values verbatim.** Add unit tests with a Free fixture `(base=100, markup=10, total=110)` and a paid fixture `(base=100, markup=0, total=100)`. Assert formatted values, `has_markup`, base-rate discount input, and spend-limit total. Include an inconsistent fixture to prove purchase is disabled rather than recomputed. Verifies Behaviors 3–8 and 18.
3. **Free Settings v2 is eligible.** Add `billing_and_usage_page_v2_tests.rs` coverage proving a policy-enabled marked-up Free workspace reaches the purchase state, shows base/markup/total plus upgrade action, and does not show auto-reload controls. A policy-disabled Free workspace remains ineligible. Verifies Behaviors 1, 3, 5, and 9.
4. **Paid Settings v2 is unchanged.** The same state tests prove a paid zero-markup option renders the existing single-price treatment with no markup row or upgrade-to-save action, while existing paid auto-reload and discount state remains available. Verifies Behaviors 6, 7, and 9.
5. **Spending-limit boundaries include markup.** Unit tests cover `already_spent + total == limit` (allowed under current semantics) and `already_spent + total > limit` (disabled), including a case where base alone would fit but marked-up total would not. Verifies Behavior 8.
6. **The out-of-credits banner matches Settings.** Add focused banner/presentation tests with the same fixtures, confirming denomination, total, markup treatment, upgrade action, responsive state, and Free auto-reload suppression. A Checkout-required result opens only the returned URL and keeps the banner in browser-pending state; immediate paid success retains current dismissal. Existing budget-exceeded, dismissal, and paid auto-reload experiment tests continue to pass. Verifies Behaviors 3, 5–9, and 13–17.
7. **Account-first onboarding exposes an optional purchase choice.** Extend `offer_slide_tests.rs` and `model_tests.rs` to prove both `FreeIcp` and `FreeStandard` variants show Subscription, Buy credits as needed, and Set up later when eligible; no new step is inserted; Back, keyboard selection, setup-later, and progress behavior remain unchanged. Verifies Behaviors 10, 12, 19, and 20.
8. **Fallback onboarding is correctly gated.** Tests prove `AiAccessSlide` shows the credits choice only for a logged-in eligible `FreeUser`; it remains absent or disabled for `LoggedOut`, `PayingUser`, disabled policy, loading-without-data, and empty-pack states. Verifies Behaviors 1, 11, 12, and 18.
9. **Checkout submit and return are deterministic.** View/root tests prove one selected `credits` value is emitted, `teamUid` is omitted for a teamless user, a second submit is blocked while the mutation or browser Checkout is pending, and Checkout-required stores the server-returned `teamUid` and opens exactly its URL. Simulated app activation refreshes that team and usage: a confirmed grant completes/dismisses once, while canceled, delayed, and duplicate-activation cases never report false success and return to the approved retryable state. Verifies Behaviors 4 and 13–16.
10. **User-facing purchase errors survive end to end.** Against the paired server branch or a deterministic GraphQL mock, trigger a Checkout-creation/billing `UserFacingError` from Settings, the banner, and onboarding. Each surface displays the server message, stays usable, and allows retry/upgrade/continue; no generic “Failed to purchase” replacement appears. Verifies Behavior 17 and the cross-repo contract.
11. **Pack refresh handles stale selection.** Tests update pricing while each surface is open and prove selection is retained by `credits` when possible, falls back safely when removed, and disables purchase for empty or invalid data. Verifies Behavior 18.
12. **Telemetry is safe and complete.** Telemetry tests assert the onboarding choice, Checkout-started, grant-confirmed, canceled/incomplete, purchase-failed, and upgrade events while confirming payloads omit price values, Checkout URLs, raw errors, email, and team identifiers. Existing onboarding telemetry tests continue to pass. Verifies Behaviors 10–17.
13. **Focused Rust tests pass.** Run at minimum:
    - `cargo nextest run -p warp_graphql`
    - `cargo nextest run -p onboarding`
    - `cargo nextest run -p warp billing_and_usage`
    - `cargo nextest run -p warp buy_credits`
    - `cargo nextest run -p warp onboarding`
    Exact filters may be adjusted to match the final test names, but all touched modules and their direct dependents must run.
14. **Repository checks pass.** Run `./script/format --check`, the clippy command used by `./script/presubmit`, and a build of the affected `warp_graphql`, `onboarding`, and `warp` packages. Because this change crosses shared pricing, onboarding, Settings, and terminal surfaces, run the full documented `./script/presubmit` unless the implementation PR’s current CI provides an equivalent full-suite backstop explicitly accepted by the repository owner.
15. **Running UI is visually verified with computer use.** Using a build connected to the paired server/test fixture, capture and attach screenshot proof for:
    - Free Settings v2 with a selected pack’s base, 10% markup, total, and upgrade-to-save action;
    - paid Settings v2 showing no markup or Free upgrade action;
    - the Free out-of-credits banner in wide and constrained layouts;
    - account-first and fallback onboarding purchase choices;
    - mutation loading, “Complete purchase in your browser,” canceled/incomplete return, confirmed success, and Checkout-creation error states.
    Validate the screenshots against Behaviors 3, 5–7, and 9–20. Media is attached to the task and PR, not committed.
16. **Design dependency is closed.** The PR links approved mocks or records explicit design approval for the final layout, copy, responsive behavior, loading, browser-pending/canceled, error, focus, and success states. Missing design approval is a blocker for completing the UI criteria.
17. **Cross-repo end-to-end purchase passes.** With both repository branches running, a logged-in Free user without a team selects an existing pack, sees the exact server amounts, submits without `teamUid`, receives `PurchaseAddonCreditsCheckoutRequired`, completes the returned Stripe Checkout, returns to Warp, and only then sees a confirmed refreshed credit balance and success state. Canceling Checkout produces no success and allows retry. A paid user receives immediate success without markup or browser redirect. Stripe charge/test records match `totalPriceUsdCents`. This verifies Behaviors 1–18.
