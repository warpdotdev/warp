use super::*;
use crate::workspaces::workspace::{BillingMetadata, CustomerType};

fn workspace_with_customer_type(customer_type: CustomerType) -> Workspace {
    Workspace {
        uid: "workspace_uid123456789".to_string().into(),
        name: "test".to_string(),
        stripe_customer_id: None,
        teams: vec![],
        open_teams: vec![],
        billing_metadata: BillingMetadata {
            customer_type,
            ..Default::default()
        },
        bonus_grants_purchased_this_month: Default::default(),
        billing_cycle_usage: None,
        has_billing_history: false,
        settings: Default::default(),
        feature_model_choice: Default::default(),
        invite_link_domain_restrictions: vec![],
        pending_email_invites: vec![],
        is_eligible_for_discovery: false,
        members: vec![],
        total_requests_used_since_last_refresh: 0,
    }
}

#[test]
fn scale_routes_to_v2_without_personal_plan_upgrade_offers() {
    let workspace = workspace_with_customer_type(CustomerType::Scale);
    assert!(BillingAndUsageDispatchView::workspace_uses_v2(Some(
        &workspace
    )));
    assert!(workspace.billing_metadata.is_on_scale_plan());
    assert!(!workspace.billing_metadata.can_upgrade_to_build_plan());
    assert!(!workspace.billing_metadata.can_upgrade_to_build_max_plan());
    let build = workspace_with_customer_type(CustomerType::Build);
    assert!(!build.billing_metadata.is_on_scale_plan());
}

#[test]
fn uses_v2_when_user_has_no_workspace() {
    assert!(BillingAndUsageDispatchView::workspace_uses_v2(None));
}

#[test]
fn uses_v2_for_free_workspace() {
    let workspace = workspace_with_customer_type(CustomerType::Free);

    assert!(BillingAndUsageDispatchView::workspace_uses_v2(Some(
        &workspace
    )));
}

#[test]
fn does_not_use_v2_for_legacy_paid_workspace() {
    let workspace = workspace_with_customer_type(CustomerType::Prosumer);

    assert!(!BillingAndUsageDispatchView::workspace_uses_v2(Some(
        &workspace
    )));
}
