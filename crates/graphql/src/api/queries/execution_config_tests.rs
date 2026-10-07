use cynic::QueryBuilder;

use super::{
    ExecutionBootstrap, ExecutionBootstrapVariables, ExecutionConfig, ExecutionConfigInput,
    ExecutionConfigVariables,
};
use crate::api::queries::task_attachments::TaskInput;
use crate::api::queries::task_secrets::TaskSecretsInput;
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

#[test]
fn bootstrap_fetches_all_task_data_in_one_operation() {
    let operation = ExecutionBootstrap::build(ExecutionBootstrapVariables {
        config_input: ExecutionConfigInput {
            task_id: "task".into(),
            execution_id: "execution".into(),
        },
        secrets_input: TaskSecretsInput {
            task_id: "task".into(),
            workload_token: "token".to_owned(),
        },
        task_input: TaskInput {
            task_id: "task".into(),
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
    let query = operation.query;
    assert_eq!(query.matches("executionConfig(").count(), 1);
    assert_eq!(query.matches("taskSecrets(").count(), 1);
    assert_eq!(query.matches("task(").count(), 1);
    for field in [
        "mcpServersJson",
        "awsSessionToken",
        "downloadUrl",
        "mimeType",
    ] {
        assert!(query.contains(field), "missing {field}");
    }
    let variables = serde_json::to_value(&operation.variables).unwrap();
    assert_eq!(variables["configInput"]["taskId"], "task");
    assert_eq!(variables["configInput"]["executionId"], "execution");
    assert_eq!(variables["secretsInput"]["taskId"], "task");
    assert_eq!(variables["secretsInput"]["workloadToken"], "token");
    assert_eq!(variables["taskInput"]["taskId"], "task");
    assert!(
        !variables["configInput"]
            .to_string()
            .contains("workloadToken")
    );
}
