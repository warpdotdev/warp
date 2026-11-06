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
    let pool = GetIncludedUsagePools::build(GetIncludedUsagePoolsVariables {
        request_context: context(),
    });
    for field in [
        "includedUsagePool",
        "monthlyCredits",
        "usedCredits",
        "monthlyCents",
        "grantedCents",
        "remainingCents",
        "expiresAt",
    ] {
        assert!(pool.query.contains(field), "{field}");
    }
    let usage = GetRequestLimitInfo::build(GetRequestLimitInfoVariables {
        request_context: context(),
    });
    let metadata = GetWorkspacesMetadataForUser::build(GetWorkspacesMetadataForUserVariables {
        request_context: context(),
    });
    assert!(!usage.query.contains("includedUsagePool"));
    assert!(!metadata.query.contains("includedUsagePool"));
}

#[test]
fn pool_response_accepts_null_and_fractional_balances() {
    let response: GetIncludedUsagePools = serde_json::from_value(serde_json::json!({
        "user": {
            "__typename": "UserOutput",
            "user": {
                "workspaces": [
                    {"uid": "without-pool", "includedUsagePool": null},
                    {"uid": "with-pool", "includedUsagePool": {
                        "monthlyCredits": 1234.75,
                        "usedCredits": 1000.125,
                        "monthlyCents": 10000,
                        "grantedCents": 10000.0,
                        "remainingCents": 1234.56,
                        "expiresAt": null
                    }}
                ]
            }
        }
    }))
    .unwrap();
    let UserResult::UserOutput(output) = response.user else {
        panic!("Expected user")
    };
    assert!(output.user.workspaces[0].included_usage_pool.is_none());
    let pool = output.user.workspaces[1]
        .included_usage_pool
        .as_ref()
        .unwrap();
    assert_eq!(pool.monthly_credits, 1234.75);
    assert_eq!(pool.used_credits, 1000.125);
    assert_eq!(
        output.user.workspaces[1]
            .included_usage_pool
            .as_ref()
            .unwrap()
            .remaining_cents,
        1234.56,
    );
}
