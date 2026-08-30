use cynic::QueryBuilder;

use super::{
    TaskGitCredentials, TaskGitCredentialsInput, TaskGitCredentialsLegacy,
    TaskGitCredentialsLegacyInput, TaskGitCredentialsLegacyVariables, TaskGitCredentialsVariables,
};
use crate::request_context::{ClientContext, OsContext, RequestContext};

fn request_context() -> RequestContext {
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
fn current_query_selects_partial_refresh_fields() {
    let operation = TaskGitCredentials::build(TaskGitCredentialsVariables {
        input: TaskGitCredentialsInput {
            task_id: cynic::Id::new("task"),
            workload_token: "token".to_string(),
            accepts_partial_refresh: Some(true),
        },
        request_context: request_context(),
    });

    assert!(operation.query.contains("failedHosts"));
    assert!(operation.query.contains("credentials"));
}

#[test]
fn bootstrap_query_still_selects_failed_hosts() {
    let operation = TaskGitCredentials::build(TaskGitCredentialsVariables {
        input: TaskGitCredentialsInput {
            task_id: cynic::Id::new("task"),
            workload_token: "token".to_string(),
            accepts_partial_refresh: Some(false),
        },
        request_context: request_context(),
    });

    assert!(operation.query.contains("failedHosts"));
}

#[test]
fn legacy_query_omits_partial_refresh_fields() {
    let operation = TaskGitCredentialsLegacy::build(TaskGitCredentialsLegacyVariables {
        input: TaskGitCredentialsLegacyInput {
            task_id: cynic::Id::new("task"),
            workload_token: "token".to_string(),
        },
        request_context: request_context(),
    });

    assert!(!operation.query.contains("acceptsPartialRefresh"));
    assert!(!operation.query.contains("failedHosts"));
    assert!(operation.query.contains("credentials"));
}
