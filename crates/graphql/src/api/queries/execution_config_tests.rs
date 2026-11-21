use super::{ExecutionBootstrap, ExecutionBootstrapVariables};
use crate::api::queries::task_attachments::TaskInput;
use crate::api::queries::task_secrets::TaskSecretsInput;
use crate::request_context::{ClientContext, OsContext, RequestContext};

#[test]
fn bootstrap_fetches_all_task_data_in_one_operation() {
    for supports_deferred_repositories in [false, true] {
        let operation = ExecutionBootstrap::build_with_capabilities(ExecutionBootstrapVariables {
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
        })
        .unwrap();
        let query = operation.query;
        assert_eq!(query.matches("executionConfig(").count(), 1);
        assert!(query.contains("supportsDeferredRepositories: $supportsDeferredRepositories"));
        assert_eq!(query.matches("taskSecrets(").count(), 1);
        assert_eq!(query.matches("task(").count(), 1);
        for field in [
            "mcpServersJson",
            "teamId",
            "factorySkillDirs",
            "factoryBranch",
            "sourceSha",
            "preserveOrigin",
            "cloneFrom",
            "sessionSharingAcls",
            "snapshotDisabled",
            "awsSessionToken",
            "downloadUrl",
            "mimeType",
        ] {
            assert!(query.contains(field), "missing {field}");
        }
        assert_eq!(
            query.contains("expectedSha"),
            supports_deferred_repositories
        );
        assert_eq!(
            query.contains("deferredRepositories"),
            supports_deferred_repositories
        );
        assert!(query.contains("supportsAgentHooks: true"));
        for field in ["agentHookSources", "onAsk", "strict"] {
            assert!(query.contains(field), "missing {field}");
        }
        assert!(!query.contains("@skip"));
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

fn configuration_json() -> serde_json::Value {
    serde_json::json!({
        "taskId": "task",
        "executionId": "execution",
        "conversationId": null,
        "parentRunId": null,
        "teamId": null,
        "harness": "OZ",
        "modelId": null,
        "reasoningLevel": null,
        "profileId": null,
        "mcpServersJson": "{}",
        "skills": [],
        "factorySkillDirs": [],
        "factoryBranch": null,
        "computerUseEnabled": false,
        "computerUseModelId": null,
        "inferenceProviders": null,
        "repositories": [],
        "deferredRepositories": [],
        "setupCommands": [],
        "providers": null,
        "sessionSharingAcls": [],
        "skipInitialTurn": false,
        "idleOnCompleteSeconds": null,
        "idleOnFailSeconds": null,
        "snapshotDisabled": false,
        "agentHookSources": []
    })
}

/// A pre-warmed assignment carries every field the server knows, including ones added after this
/// client was built, so a field the fragment does not select must not fail decoding.
#[test]
fn a_field_this_client_does_not_select_is_ignored() {
    let mut json = configuration_json();
    json["addedByANewerServer"] = serde_json::json!("value");
    serde_json::from_value::<super::ExecutionConfiguration>(json).expect("decodes");
}

#[test]
fn hook_sources_decode_and_an_unknown_value_is_kept_as_unknown() {
    let mut json = configuration_json();
    json["agentHookSources"] = serde_json::json!([
        {
            "format": "CURSOR",
            "repository": { "forge": "GITHUB", "owner": "acme", "name": "api" },
            "path": ".cursor",
            "level": "AGENT",
            "strict": true,
            "onAsk": "FAIL_RUN"
        },
        {
            "format": "CLAUDE_CODE",
            "repository": null,
            "path": "/etc/hooks",
            "level": "SOMETHING_NEW",
            "strict": false,
            "onAsk": "ESCALATE"
        }
    ]);
    let config: super::ExecutionConfiguration = serde_json::from_value(json).expect("decodes");
    let sources = config.agent_hook_sources;
    assert_eq!(sources[0].format, super::AgentHookFormat::Cursor);
    assert_eq!(sources[0].level, super::AgentHookLevel::Agent);
    assert_eq!(sources[0].on_ask, super::AgentHookOnAsk::FailRun);
    assert_eq!(
        sources[0].repository.as_ref().map(|repo| repo.forge),
        Some(super::CodeForge::GitHub)
    );
    assert_eq!(sources[1].format, super::AgentHookFormat::Unknown);
    assert_eq!(sources[1].level, super::AgentHookLevel::Unknown);
    assert_eq!(sources[1].on_ask, super::AgentHookOnAsk::Unknown);
    assert!(sources[1].repository.is_none());
}
