use cynic::QueryBuilder;

use super::*;
use crate::queries::get_request_limit_info::{GetRequestLimitInfo, GetRequestLimitInfoVariables};
use crate::queries::get_workspaces_metadata_for_user::{
    GetWorkspacesMetadataForUser, GetWorkspacesMetadataForUserVariables,
};
use crate::request_context::{ClientContext, OsContext};

fn context() -> RequestContext {
    RequestContext {
        client_context: ClientContext { version: None },
        os_context: OsContext {
            category: None,
            linux_kernel_version: None,
            name: None,
            version: None,
        },
    }
}

#[test]
fn pool_query_is_separate_from_core_usage_and_metadata_queries() {
    let pool = GetIncludedUsageInfo::build(GetIncludedUsageInfoVariables {
        request_context: context(),
    });
    for field in [
        "includedUsageInfo",
        "includedUsageCredits",
        "usageCreditsUsedSinceLastRefresh",
        "includedUsageCents",
        "usageCentsUsedSinceLastRefresh",
        "nextRefreshTime",
    ] {
        assert!(pool.query.contains(field), "{field}");
    }
    let usage = GetRequestLimitInfo::build(GetRequestLimitInfoVariables {
        request_context: context(),
    });
    let metadata = GetWorkspacesMetadataForUser::build(GetWorkspacesMetadataForUserVariables {
        request_context: context(),
    });
    assert!(!usage.query.contains("includedUsageInfo"));
    assert!(!metadata.query.contains("includedUsageInfo"));
}

#[test]
fn pool_response_accepts_null_and_fractional_balances() {
    let response: GetIncludedUsageInfo = serde_json::from_value(serde_json::json!({
        "user": {
            "__typename": "UserOutput",
            "user": {
                "workspaces": [
                    {"uid": "without-pool", "includedUsageInfo": null},
                    {"uid": "with-pool", "includedUsageInfo": {
                        "includedUsageCredits": 1234.75,
                        "usageCreditsUsedSinceLastRefresh": 1000.125,
                        "includedUsageCents": 10000.125,
                        "usageCentsUsedSinceLastRefresh": 8765.565,
                        "nextRefreshTime": "2026-12-06T00:00:00Z"
                    }}
                ]
            }
        }
    }))
    .unwrap();
    let UserResult::UserOutput(output) = response.user else {
        panic!("Expected user")
    };
    assert!(output.user.workspaces[0].included_usage_info.is_none());
    let pool = output.user.workspaces[1]
        .included_usage_info
        .as_ref()
        .unwrap();
    assert_eq!(pool.included_usage_credits, 1234.75);
    assert_eq!(pool.included_usage_cents, 10000.125);
    assert_eq!(pool.usage_credits_used_since_last_refresh, 1000.125);
    assert_eq!(
        output.user.workspaces[1]
            .included_usage_info
            .as_ref()
            .unwrap()
            .usage_cents_used_since_last_refresh,
        8765.565,
    );
}
