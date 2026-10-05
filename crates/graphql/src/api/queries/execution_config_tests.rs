use cynic::QueryBuilder;

use super::{ExecutionConfig, ExecutionConfigInput, ExecutionConfigVariables};
use crate::request_context::{ClientContext, OsContext, RequestContext};

#[test]
fn query_binds_both_ids_without_serializing_a_workload_token() {
    let operation = ExecutionConfig::build(ExecutionConfigVariables {
        input: ExecutionConfigInput {
            task_id: "task".into(),
            execution_id: "execution".into(),
        },
        request_context: RequestContext {
            client_context: ClientContext { version: None },
            os_context: OsContext {
                category: None,
                linux_kernel_version: None,
                name: None,
                version: None,
            },
        },
    });
    let variables = serde_json::to_string(&operation.variables).unwrap();
    assert!(variables.contains("\"taskId\":\"task\""));
    assert!(variables.contains("\"executionId\":\"execution\""));
    assert!(!variables.contains("workloadToken"));
    for field in [
        "mcpServersJson",
        "factorySkillDirs",
        "preserveOrigin",
        "cloneFrom",
        "sessionSharingAcls",
        "snapshotDisabled",
    ] {
        assert!(operation.query.contains(field), "missing {field}");
    }
    assert!(!operation.query.contains("removeRepositoryOrigins"));
    assert!(!operation.query.contains("snapshotUploadTimeout"));
    assert!(!operation.query.contains("snapshotScriptTimeout"));
}
