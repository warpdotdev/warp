use super::{warp_ai_entitlement_card, WarpAiEntitlementCard};

#[test]
fn test_warp_ai_entitlement_card_choice() {
    // A plan with base credits shows the meter card.
    assert_eq!(
        warp_ai_entitlement_card(true, false),
        WarpAiEntitlementCard::BaseCredits,
    );

    // The gated Free plan shows the no-Warp-AI card instead of a 0-of-N meter.
    assert_eq!(
        warp_ai_entitlement_card(false, true),
        WarpAiEntitlementCard::FreePlanNoAi,
    );

    // The gated state wins even if a stale cached limit is still non-zero (e.g. the
    // FREE_PLAN_NO_AI denial arrived before the usage refresh): never render a
    // credit meter for a plan that includes no Warp AI.
    assert_eq!(
        warp_ai_entitlement_card(true, true),
        WarpAiEntitlementCard::FreePlanNoAi,
    );

    // No base credits and not gated (e.g. limits not loaded yet): no card.
    assert_eq!(
        warp_ai_entitlement_card(false, false),
        WarpAiEntitlementCard::None,
    );
}
