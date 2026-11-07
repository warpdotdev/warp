use super::*;

#[test]
fn scale_capacity_limits_do_not_offer_a_build_downgrade() {
    assert!(!CloudAgentCapacityModal::can_upgrade(
        CustomerType::Scale,
        CloudAgentCapacityModalVariant::ConcurrentLimit,
    ));
    assert!(!CloudAgentCapacityModal::should_show_cta(
        CustomerType::Scale,
        CloudAgentCapacityModalVariant::ConcurrentLimit,
    ));
    assert!(!CloudAgentCapacityModal::can_upgrade(
        CustomerType::Scale,
        CloudAgentCapacityModalVariant::OutOfCredits,
    ));
    assert!(CloudAgentCapacityModal::should_show_cta(
        CustomerType::Scale,
        CloudAgentCapacityModalVariant::OutOfCredits,
    ));
    assert!(CloudAgentCapacityModal::can_upgrade(
        CustomerType::Build,
        CloudAgentCapacityModalVariant::ConcurrentLimit,
    ));
}
