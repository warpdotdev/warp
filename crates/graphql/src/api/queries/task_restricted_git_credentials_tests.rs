use cynic::QueryBuilder;

use super::*;
use crate::request_context::{ClientContext, OsContext};

#[test]
fn provider_selection_is_optional_without_changing_the_restricted_query() {
    for providers in [
        None,
        Some(vec![
            TaskGitCredentialProvider::Gitlab,
            TaskGitCredentialProvider::AzureDevops,
        ]),
    ] {
        let operation =
            RestrictedTaskGitCredentials::build(RestrictedTaskGitCredentialsVariables {
                input: RestrictedTaskGitCredentialsInput {
                    task_id: cynic::Id::new("task"),
                    workload_token: "test-workload".into(),
                    accepts_partial_refresh: Some(true),
                    use_factory_repositories: Some(true),
                    github_credential_contract_version: Some(1),
                    providers: providers.clone(),
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
        let variables = serde_json::to_value(&operation.variables).unwrap();
        if providers.is_some() {
            assert_eq!(
                variables["input"]["providers"],
                serde_json::json!(["GITLAB", "AZURE_DEVOPS"])
            );
        } else {
            assert!(variables["input"].get("providers").is_none());
        }
        assert_eq!(variables["input"]["githubCredentialContractVersion"], 1);
        assert_eq!(variables["input"]["useFactoryRepositories"], true);
        for field in [
            "failedHosts",
            "githubCredentialMode",
            "expiresAt",
            "repositoryScope",
        ] {
            assert!(operation.query.contains(field));
        }
        assert!(!operation.query.contains("TaskGitCredentialProvider"));
    }
}
