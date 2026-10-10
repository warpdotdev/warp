use cynic::QueryBuilder;

use super::{ExecutionBootstrap, ExecutionBootstrapVariables};
use crate::api::queries::task_attachments::TaskInput;
use crate::api::queries::task_secrets::TaskSecretsInput;
use crate::request_context::{ClientContext, OsContext, RequestContext};

#[test]
fn bootstrap_fetches_all_task_data_in_one_operation() {
    for supports_deferred_repositories in [false, true] {
        let operation = ExecutionBootstrap::build(ExecutionBootstrapVariables {
            execution_id: "execution".into(),
            supports_deferred_repositories,
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
        assert!(query.contains("supportsDeferredRepositories: $supportsDeferredRepositories"));
        assert_eq!(query.matches("taskSecrets(").count(), 1);
        assert_eq!(query.matches("task(").count(), 1);
        for field in [
            "mcpServersJson",
            "teamId",
            "factorySkillDirs",
            "preserveOrigin",
            "cloneFrom",
            "sessionSharingAcls",
            "snapshotDisabled",
            "awsSessionToken",
            "downloadUrl",
            "mimeType",
            "deferredRepositories",
        ] {
            assert!(query.contains(field), "missing {field}");
        }
        let variables = serde_json::to_value(&operation.variables).unwrap();
        assert_eq!(variables["executionId"], "execution");
        assert_eq!(
            variables["supportsDeferredRepositories"],
            supports_deferred_repositories
        );
        assert_eq!(variables["secretsInput"]["taskId"], "task");
        assert_eq!(variables["secretsInput"]["workloadToken"], "token");
        assert_eq!(variables["taskInput"]["taskId"], "task");
        assert!(!query.contains("executionConfig(input:"));
    }
}
