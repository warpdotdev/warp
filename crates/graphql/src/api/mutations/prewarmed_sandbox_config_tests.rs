use cynic::{GraphQlResponse, MutationBuilder};
use serde_json::json;

use super::*;
use crate::client::{RequestOptions, build_graphql_request, get_request_context};
use crate::queries::execution_config::{CodeForge, ExecutionRepositoryRefType};
use crate::queries::task_secrets::ManagedSecretValue;
use crate::request_context::{ClientContext, OsContext};

#[test]
fn serializes_configuration_mutation_without_workload_token_input() {
    let operation = PrewarmedSandboxConfig::build(PrewarmedSandboxConfigVariables {
        input: PrewarmedSandboxConfigInput {
            sandbox_id: "sandbox".into(),
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
    assert!(
        operation
            .query
            .starts_with("mutation PrewarmedSandboxConfig(")
    );
    assert!(
        operation
            .query
            .contains("prewarmedSandboxConfig(input: $input, requestContext: $requestContext)")
    );
    assert!(
        operation
            .query
            .contains("... on PrewarmedSandboxConfiguration")
    );
    assert!(operation.query.contains("... on UserFacingError"));
    for field in [
        "sandboxId",
        "fingerprint",
        "spec",
        "harness",
        "repositories",
        "forge",
        "owner",
        "name",
        "ref",
        "type",
        "value",
        "cloneFrom",
        "codeForge",
        "repo",
        "preserveOrigin",
        "deferredRepositories",
        "setupCommands",
        "mcpServersJson",
        "skills",
        "factorySkillDirs",
        "computerUseEnabled",
        "providers",
        "gcp",
        "projectNumber",
        "poolId",
        "providerId",
        "serviceAccountEmail",
        "aws",
        "roleArn",
        "environmentVariables",
        "gitCredentials",
        "host",
        "username",
        "token",
        "email",
        "managedSecrets",
        "... on ManagedSecretRawValue",
        "... on ManagedSecretAnthropicApiKeyValue",
        "apiKey",
        "expiresAt",
    ] {
        assert!(operation.query.contains(field), "missing {field}");
    }
    assert!(!operation.query.contains("workload-token"));
    let request = serde_json::to_value(operation).unwrap();
    assert_eq!(
        request["variables"]["input"],
        json!({"sandboxId": "sandbox"})
    );
    assert!(request["variables"]["requestContext"].is_object());
}

#[test]
fn deserializes_configuration_response_with_shared_repository_types() {
    let response: GraphQlResponse<PrewarmedSandboxConfig> =
        serde_json::from_str(include_str!("fixtures/prewarmed_sandbox_config.json")).unwrap();
    assert!(response.errors.is_none());
    let PrewarmedSandboxConfigResult::PrewarmedSandboxConfiguration(config) =
        response.data.unwrap().prewarmed_sandbox_config
    else {
        panic!("expected a configuration");
    };
    assert_eq!(
        config.sandbox_id.inner(),
        "11111111-2222-3333-4444-555555555555"
    );
    assert_eq!(config.fingerprint, "abc123");
    let repository = &config.spec.repositories[0];
    assert_eq!(repository.forge, CodeForge::GitHub);
    assert_eq!(
        repository.ref_.as_ref().unwrap().type_,
        ExecutionRepositoryRefType::Branch
    );
    assert_eq!(repository.ref_.as_ref().unwrap().value, "main");
    assert_eq!(repository.clone_from.as_ref().unwrap().repo, "warp-fork");
    assert!(repository.preserve_origin);
    assert_eq!(config.spec.deferred_repositories[0].repo, "warp-server");
    let providers = config.spec.providers.as_ref().unwrap();
    assert_eq!(providers.gcp.as_ref().unwrap().pool_id, "oz-pool");
    assert_eq!(
        providers.aws.as_ref().unwrap().role_arn,
        "arn:aws:iam::123456789012:role/OzAgent"
    );
    assert!(serde_json::from_str::<serde_json::Value>(&config.spec.mcp_servers_json).is_ok());
    assert_eq!(
        config.environment_variables[0].name,
        "WARP_FACTORY_REPO_DIR"
    );
    assert_eq!(config.environment_variables[0].value, "factory");
    assert_eq!(config.git_credentials[0].token, "ghs_secret");
    assert_eq!(config.managed_secrets[0].name, "API_TOKEN");
    let ManagedSecretValue::ManagedSecretRawValue(raw) = &config.managed_secrets[0].value else {
        panic!("expected a raw value secret");
    };
    assert_eq!(raw.value, "value");
    assert_eq!(config.managed_secrets[1].name, "CLAUDE_AUTH");
    let ManagedSecretValue::ManagedSecretAnthropicApiKeyValue(api_key) =
        &config.managed_secrets[1].value
    else {
        panic!("expected an Anthropic API key secret");
    };
    assert_eq!(api_key.api_key, "sk-ant-key");
    assert_eq!(
        config.expires_at.utc().to_rfc3339(),
        "2026-01-01T00:00:00+00:00"
    );
}

#[test]
fn deserializes_error_and_unknown_result_variants() {
    let error: PrewarmedSandboxConfig = serde_json::from_value(json!({
        "prewarmedSandboxConfig": {
            "__typename": "UserFacingError",
            "error": { "__typename": "UnknownError", "message": "unavailable" },
            "responseContext": { "serverVersion": null }
        }
    }))
    .unwrap();
    assert!(matches!(
        error.prewarmed_sandbox_config,
        PrewarmedSandboxConfigResult::UserFacingError(_)
    ));

    let unknown: PrewarmedSandboxConfig = serde_json::from_value(json!({
        "prewarmedSandboxConfig": { "__typename": "FutureConfig" }
    }))
    .unwrap();
    assert!(matches!(
        unknown.prewarmed_sandbox_config,
        PrewarmedSandboxConfigResult::Unknown
    ));
}

#[test]
fn rejects_invalid_workload_token_headers() {
    let client = http_client::Client::new_for_test();
    for header in [
        "X-Warp-Ambient-Workload-Token",
        "x-warp-ambient-workload-token",
    ] {
        let operation = PrewarmedSandboxConfig::build(PrewarmedSandboxConfigVariables {
            input: PrewarmedSandboxConfigInput {
                sandbox_id: "sandbox".into(),
            },
            request_context: get_request_context(),
        });
        let result = build_graphql_request(
            &client,
            operation,
            RequestOptions {
                headers: [(header.into(), "invalid\nworkload-token".into())].into(),
                ..Default::default()
            },
        );
        assert!(result.is_err());
    }
}
