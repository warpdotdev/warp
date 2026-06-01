use super::{
    AmbientAgentEnvironment, AwsProviderConfig, BaseImage, GcpProviderConfig, GithubRepo,
    ProvidersConfig,
};

#[test]
fn deserialize_legacy_environment_without_providers() {
    let json = serde_json::json!({
        "name": "my-env",
        "github_repos": [{"owner": "warpdotdev", "repo": "warp"}],
        "docker_image": "ubuntu:latest",
        "setup_commands": ["echo hello"]
    });

    let env: AmbientAgentEnvironment = serde_json::from_value(json).unwrap();
    assert_eq!(env.name, "my-env");
    assert_eq!(env.providers, ProvidersConfig::default());
    assert_eq!(env.github_repos.len(), 1);
    assert_eq!(
        env.base_image,
        BaseImage::DockerImage("ubuntu:latest".into())
    );
    assert_eq!(env.setup_commands, vec!["echo hello"]);
}

#[test]
fn deserialize_with_aws_provider() {
    let json = serde_json::json!({
        "name": "aws-env",
        "github_repos": [],
        "docker_image": "node:18",
        "providers": {
            "aws": {
                "role_arn": "arn:aws:iam::123456789012:role/my-role"
            }
        }
    });

    let env: AmbientAgentEnvironment = serde_json::from_value(json).unwrap();
    assert_eq!(env.name, "aws-env");
    let providers = env.providers;
    assert_eq!(providers.gcp, None);
    let aws = providers.aws.unwrap();
    assert_eq!(aws.role_arn, "arn:aws:iam::123456789012:role/my-role");
}

#[test]
fn deserialize_with_gcp_provider() {
    let json = serde_json::json!({
        "name": "gcp-env",
        "github_repos": [],
        "docker_image": "node:18",
        "providers": {
            "gcp": {
                "project_number": "123456",
                "workload_identity_federation_pool_id": "pool-1",
                "workload_identity_federation_provider_id": "provider-1"
            }
        }
    });

    let env: AmbientAgentEnvironment = serde_json::from_value(json).unwrap();
    let gcp = env.providers.gcp.unwrap();
    assert_eq!(gcp.project_number, "123456");
    assert_eq!(gcp.workload_identity_federation_pool_id, "pool-1");
    assert_eq!(gcp.workload_identity_federation_provider_id, "provider-1");
    assert_eq!(gcp.service_account_email, None);
}

#[test]
fn deserialize_with_gcp_provider_service_account() {
    let json = serde_json::json!({
        "name": "gcp-sa-env",
        "github_repos": [],
        "docker_image": "node:18",
        "providers": {
            "gcp": {
                "project_number": "123456",
                "workload_identity_federation_pool_id": "pool-1",
                "workload_identity_federation_provider_id": "provider-1",
                "service_account_email": "sa@project.iam.gserviceaccount.com"
            }
        }
    });

    let env: AmbientAgentEnvironment = serde_json::from_value(json).unwrap();
    let gcp = env.providers.gcp.unwrap();
    assert_eq!(gcp.project_number, "123456");
    assert_eq!(
        gcp.service_account_email.as_deref(),
        Some("sa@project.iam.gserviceaccount.com")
    );
}

#[test]
fn deserialize_with_both_providers() {
    let json = serde_json::json!({
        "name": "both-env",
        "github_repos": [],
        "docker_image": "node:18",
        "providers": {
            "gcp": {
                "project_number": "123456",
                "workload_identity_federation_pool_id": "pool-1",
                "workload_identity_federation_provider_id": "provider-1"
            },
            "aws": {
                "role_arn": "arn:aws:iam::123456789012:role/my-role"
            }
        }
    });

    let env: AmbientAgentEnvironment = serde_json::from_value(json).unwrap();
    let providers = env.providers;
    assert!(providers.gcp.is_some());
    assert!(providers.aws.is_some());
}

#[test]
fn serialize_with_providers_none_omits_field() {
    let env = AmbientAgentEnvironment::new(
        "test-env".into(),
        None,
        vec![],
        "ubuntu:latest".into(),
        vec![],
    );

    let json = serde_json::to_value(&env).unwrap();
    assert!(!json.as_object().unwrap().contains_key("providers"));
}

#[test]
fn serialize_with_providers_includes_field() {
    let mut env = AmbientAgentEnvironment::new(
        "test-env".into(),
        None,
        vec![],
        "ubuntu:latest".into(),
        vec![],
    );
    env.providers = ProvidersConfig {
        gcp: None,
        aws: Some(AwsProviderConfig {
            role_arn: "arn:aws:iam::123456789012:role/test".into(),
        }),
    };

    let json = serde_json::to_value(&env).unwrap();
    let providers = json.get("providers").unwrap();
    assert!(providers.get("aws").is_some());
    assert!(providers.get("gcp").is_none());
}

#[test]
fn roundtrip_serde_with_providers() {
    let mut env = AmbientAgentEnvironment::new(
        "rt-env".into(),
        Some("desc".into()),
        vec![GithubRepo::new("owner".into(), "repo".into())],
        "alpine:latest".into(),
        vec!["make build".into()],
    );
    env.providers = ProvidersConfig {
        gcp: Some(GcpProviderConfig {
            project_number: "999".into(),
            workload_identity_federation_pool_id: "p".into(),
            workload_identity_federation_provider_id: "pr".into(),
            service_account_email: Some("sa@proj.iam.gserviceaccount.com".into()),
        }),
        aws: Some(AwsProviderConfig {
            role_arn: "arn:aws:iam::1:role/r".into(),
        }),
    };

    let serialized = serde_json::to_string(&env).unwrap();
    let deserialized: AmbientAgentEnvironment = serde_json::from_str(&serialized).unwrap();
    assert_eq!(env, deserialized);
}
