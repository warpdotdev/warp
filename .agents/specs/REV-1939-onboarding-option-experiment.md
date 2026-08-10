# Spec: Onboarding "Choose how to start" option-count experiment — Warp client (REV-1939)

This is the `warpdotdev/warp` half of a multi-repository feature. The sibling
`warpdotdev/warp-server` change owns the experiment definition, user bucketing,
GraphQL enum values, eligibility, and traffic configuration. This client change
owns consuming that assignment, rendering the correct onboarding arm, and
emitting arm-qualified client telemetry. The server behavior is an input
contract; no server internals are specified here.

Linear: <https://linear.app/warpdotdev/issue/REV-1939/onboarding-choose-how-to-start-ab-experiment-2-option-control-vs-3>
Originating thread: <https://warpdev.slack.com/archives/C0BDQDW8V5E/p1785959175591619>

This spec was re-derived against the onboarding code as it exists on the current
`master` base: the `OfferVariant::ChooseHowToStart` slide now folds the plan and
the one-time credit packs into a single combined "Use Warp with AI" card (a
subscribe button plus inline credit-pack tiles). That combined card is the
experiment treatment; the control suppresses the pack UI.

## Product

Run a server-assigned A/B experiment on the account-first, post-authentication
`free_standard` "Choose how to start" slide, to measure which arm converts
better to paid usage.

Behavior (testable invariants):
1. Only the account-first, post-auth `OfferVariant::ChooseHowToStart` path for
   `free_standard` users is affected. Paid users, `free_icp`
   `OfferVariant::HeadStart`, legacy onboarding, and every non-offer surface
   behave exactly as before.
2. Control (and unassigned) users see the historical two-option layout even when
   purchasable packs are loaded: a plain "Use Warp with AI" card (label
   "Use Warp with AI", the pre-credit-pack description) plus "Set up AI later".
   No subscribe button, credit-pack tiles, divider, or pack-selection state is
   rendered. "Get Warping" on the primary opens the existing `/upgrade` flow.
3. Experiment users with one or more purchasable packs see the current combined
   "Use Warp with AI" card (subscribe button + inline pack tiles) plus
   "Set up AI later" — today's behavior unchanged.
4. Experiment users with no available packs (pricing not loaded, purchase policy
   disallows, empty server list) fall back to the same historical two-option
   layout as control, but remain telemetry-assigned to `experiment`; lack of
   packs is never rewritten as control.
5. Unassigned is the default when neither arm is present, and both-arms
   (malformed) state fails closed to unassigned. The arm is snapshotted just
   before the offer is shown and frozen for that exposure; a later server
   refresh does not change the visible arm mid-exposure.
6. Existing stable identifiers are unchanged (`choose_how_to_start`,
   `use_warp_with_ai`, `buy_ai_credits`, `set_up_later`). The onboarding
   monetization funnel events for this offer additionally carry
   `experiment_arm: "control" | "experiment" | "unassigned"`:
   `onboarding_slide_viewed`, `onboarding_action`, `onboarding_upgrade_started`,
   `onboarding_upgrade_completed`, and `onboarding_completed`. Events outside the
   offer omit the key rather than emitting `null`.

## Tech

- Add `OnboardingChooseHowToStartControl` / `OnboardingChooseHowToStartExperiment`
  to the client GraphQL `Experiment` enum and the checked-in schema snapshot,
  mapping to `ONBOARDING_CHOOSE_HOW_TO_START_CONTROL` and
  `ONBOARDING_CHOOSE_HOW_TO_START_EXPERIMENT`.
- Add matching `ServerExperiment` arms (no-op `on_added_to`, queried directly)
  and a `ServerExperiments::choose_how_to_start_experiment_arm()` resolver
  (control-only → Control, experiment-only → Experiment, neither/both →
  Unassigned).
- Add a public `ChooseHowToStartExperimentArm` (Unassigned default / Control /
  Experiment) to the onboarding crate with `telemetry_value()` and
  `shows_credit_packs()`; store it on `OnboardingStateModel` with an idempotent
  setter and an `offer_experiment_arm()` telemetry helper. Forward the
  setter/getter on `AgentOnboardingView`.
- `RootView` snapshots the arm from `ServerExperiments` immediately before each
  `show_post_auth_offer`.
- `OfferSlide::credit_packs`/`shows_credit_packs` return packs only for the
  experiment arm with a non-empty list; `render_options` renders the combined
  card only when packs are shown, otherwise the plain historical primary card.
- Thread `experiment_arm` into the five onboarding funnel events above.

## Validation
- `cargo nextest run -p onboarding -p warp_graphql -p warp` (arm resolution,
  control/unassigned/experiment rendering, telemetry payloads).
- `./script/format --check` and the documented clippy invocations.
- Computer-use visual proof of control, experiment (with packs), and
  unassigned/experiment-without-packs states, attached to the task and PR body.

## Out of scope
- Server experiment definition, bucketing, eligibility, traffic, analytics.
- Any change to the `/upgrade` page, pack pricing/premiums, checkout, or
  purchase-policy logic.
- `OfferVariant::HeadStart`, paid-user onboarding, or legacy onboarding.
- New copy, card components, or a visual redesign; both arms reuse the existing
  offer-slide component and copy.
