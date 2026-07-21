use super::OfferVariant;

#[test]
fn head_start_copy_and_telemetry_names_match_spec() {
    let variant = OfferVariant::HeadStart;

    assert_eq!(variant.title(), "You've got a head start");
    assert_eq!(variant.subtitle(), "Your account comes with some free AI");
    assert_eq!(variant.primary_label(), "Get more AI");
    assert_eq!(variant.slide_name(), "head_start");
    assert_eq!(variant.account_class(), "free_icp");
    assert_eq!(variant.primary_action(), "get_more_ai");
}

#[test]
fn choose_how_to_start_copy_and_telemetry_names_match_spec() {
    let variant = OfferVariant::ChooseHowToStart;

    assert_eq!(variant.title(), "Choose how to start");
    assert_eq!(
        variant.subtitle(),
        "Warp's agent requires a plan. Pick how you want to start"
    );
    assert_eq!(variant.primary_label(), "Use Warp with AI");
    assert_eq!(variant.slide_name(), "choose_how_to_start");
    assert_eq!(variant.account_class(), "free_standard");
    assert_eq!(variant.primary_action(), "use_warp_with_ai");
}
