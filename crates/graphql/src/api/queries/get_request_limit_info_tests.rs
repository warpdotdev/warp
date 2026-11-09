use super::*;

#[test]
fn workspace_usage_accepts_null_and_fractional_allowances() {
    let workspace = |allowance| {
        serde_json::json!({
            "uid": "workspace-1",
            "bonusGrantsInfo": { "grants": [], "spendingInfo": null },
            "includedUsageInfo": allowance,
        })
    };
    let missing: WorkspaceInfo =
        serde_json::from_value(workspace(serde_json::Value::Null)).unwrap();
    assert!(missing.included_usage_info.is_none());
    let present: WorkspaceInfo = serde_json::from_value(workspace(serde_json::json!({
        "includedUsageCredits": 1234.75,
        "usageCreditsUsedSinceLastRefresh": 250.125,
        "includedUsageCents": 10000.125,
        "usageCentsUsedSinceLastRefresh": 8765.565,
        "nextRefreshTime": "2026-12-06T00:00:00Z"
    })))
    .unwrap();
    let allowance = present.included_usage_info.unwrap();
    assert_eq!(allowance.included_usage_credits, 1234.75);
    assert_eq!(allowance.usage_credits_used_since_last_refresh, 250.125);
    assert_eq!(allowance.included_usage_cents, 10000.125);
    assert_eq!(allowance.usage_cents_used_since_last_refresh, 8765.565);
}
