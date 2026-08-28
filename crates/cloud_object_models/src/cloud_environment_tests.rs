use super::{
    AmbientAgentEnvironment, AwsProviderConfig, BaseImage, CodeForge, EnvironmentSecretRef,
    GcpProviderConfig, GithubRepo, ProvidersConfig, SourceRepo,
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
    assert_eq!(env.code_forge, None);
    assert_eq!(env.source_repos, None);
    assert_eq!(
        env.effective_repos(),
        vec![SourceRepo::new(
            CodeForge::GitHub,
            "warpdotdev".into(),
            "warp".into()
        )]
    );
    assert_eq!(
        env.base_image,
        Some(BaseImage::DockerImage("ubuntu:latest".into()))
    );
    assert_eq!(env.setup_commands, vec!["echo hello"]);
}

#[test]
fn deserialize_environment_without_docker_image() {
    // Environments that don't pin a base image must still deserialize; the
    // server may omit the docker image entirely.
    let json = serde_json::json!({
        "name": "no-image-env",
        "github_repos": [{"owner": "warpdotdev", "repo": "warp"}],
        "setup_commands": ["echo hello"]
    });

    let env: AmbientAgentEnvironment = serde_json::from_value(json).unwrap();
    assert_eq!(env.name, "no-image-env");
    assert_eq!(env.base_image, None);
    assert_eq!(env.base_image_display(), "");
    assert_eq!(env.setup_commands, vec!["echo hello"]);
}

#[test]
fn serialize_environment_without_docker_image_omits_field() {
    let env = AmbientAgentEnvironment {
        name: "no-image-env".into(),
        description: None,
        code_forge: None,
        github_repos: vec![],
        source_repos: None,
        base_image: None,
        setup_commands: vec![],
        providers: ProvidersConfig::default(),
        secrets: None,
        default_runner_uid: None,
    };

    let json = serde_json::to_value(&env).unwrap();
    assert!(!json.as_object().unwrap().contains_key("docker_image"));
}

#[test]
fn roundtrip_serde_without_docker_image() {
    let env = AmbientAgentEnvironment {
        name: "no-image-rt".into(),
        description: None,
        code_forge: None,
        github_repos: vec![GithubRepo::new("owner".into(), "repo".into())],
        source_repos: None,
        base_image: None,
        setup_commands: vec!["make build".into()],
        providers: ProvidersConfig::default(),
        secrets: None,
        default_runner_uid: None,
    };

    let serialized = serde_json::to_string(&env).unwrap();
    let deserialized: AmbientAgentEnvironment = serde_json::from_str(&serialized).unwrap();
    assert_eq!(env, deserialized);
}

#[test]
fn source_repo_checkout_ref_round_trips_and_is_optional() {
    // Present: the field serializes under the `checkout_ref` JSON tag and
    // survives a round trip.
    let pinned = SourceRepo::new(CodeForge::GitHub, "warpdotdev".into(), "warp".into())
        .with_checkout_ref(Some("abc123".into()));
    let json = serde_json::to_value(&pinned).unwrap();
    assert_eq!(json.get("checkout_ref").unwrap(), "abc123");
    assert_eq!(serde_json::from_value::<SourceRepo>(json).unwrap(), pinned);

    // Absent: the field is omitted from the wire form and defaults to None on
    // deserialize, so existing payloads without it keep working.
    let unpinned = SourceRepo::new(CodeForge::GitHub, "warpdotdev".into(), "warp".into());
    let json = serde_json::to_value(&unpinned).unwrap();
    assert!(!json.as_object().unwrap().contains_key("checkout_ref"));
    let parsed: SourceRepo =
        serde_json::from_value(serde_json::json!({"owner": "warpdotdev", "repo": "warp"})).unwrap();
    assert_eq!(parsed.checkout_ref, None);
}

#[test]
fn deserialize_repo_less_environment_resolves_to_none_forge() {
    let json = serde_json::json!({
        "name": "repo-less-env",
        "code_forge": "NONE",
        "github_repos": [],
        "source_repos": [],
        "setup_commands": ["echo hello"]
    });

    let env: AmbientAgentEnvironment = serde_json::from_value(json).unwrap();

    assert_eq!(env.effective_code_forge(), CodeForge::None);
    assert!(env.effective_repos().is_empty());
}

#[test]
fn deserialize_environment_with_unrecognized_forge_still_succeeds() {
    // A forge value this client build doesn't know about yet (e.g. the
    // server introduces a new one before this client updates) must not fail
    // deserialization of the whole environment.
    let json = serde_json::json!({
        "name": "future-forge-env",
        "code_forge": "BITBUCKET",
        "github_repos": [],
        "setup_commands": ["echo hello"]
    });

    let env: AmbientAgentEnvironment = serde_json::from_value(json).unwrap();

    assert_eq!(env.effective_code_forge(), CodeForge::Unknown);
}

#[test]
fn none_and_unknown_forges_have_no_clonable_host() {
    // Neither identifies a real host; a caller falling back to GitHub's host
    // for either would authenticate against the wrong one.
    assert_eq!(CodeForge::None.host(), "");
    assert_eq!(CodeForge::Unknown.host(), "");
}

#[test]
fn deserialize_gitlab_environment_uses_authoritative_source_repos() {
    let json = serde_json::json!({
        "name": "gitlab-env",
        "code_forge": "GITLAB",
        "github_repos": [{"owner": "legacy-mirror", "repo": "ignored"}],
        "source_repos": [{
            "owner": "platform/backend",
            "repo": "api"
        }],
        "docker_image": "ubuntu:latest"
    });

    let env: AmbientAgentEnvironment = serde_json::from_value(json).unwrap();

    assert_eq!(env.effective_code_forge(), CodeForge::GitLab);
    assert_eq!(env.source_repos.as_ref().unwrap()[0].code_forge, None);
    assert_eq!(
        env.effective_repos(),
        vec![SourceRepo::new(
            CodeForge::GitLab,
            "platform/backend".into(),
            "api".into()
        )]
    );
    assert_eq!(
        env.effective_repos()[0].https_clone_url(),
        "https://gitlab.com/platform/backend/api.git"
    );
}

#[test]
fn present_empty_source_repos_override_legacy_mirror() {
    let json = serde_json::json!({
        "name": "empty-env",
        "code_forge": "GITLAB",
        "github_repos": [{"owner": "legacy-mirror", "repo": "ignored"}],
        "source_repos": [],
        "docker_image": "ubuntu:latest"
    });

    let env: AmbientAgentEnvironment = serde_json::from_value(json).unwrap();

    assert!(env.effective_repos().is_empty());
}

#[test]
fn legacy_environment_serialization_omits_provider_neutral_fields() {
    let env = AmbientAgentEnvironment::new(
        "legacy-env".into(),
        None,
        vec![GithubRepo::new("warpdotdev".into(), "warp".into())],
        "ubuntu:latest".into(),
        vec![],
    );

    let json = serde_json::to_value(&env).unwrap();

    assert!(!json.as_object().unwrap().contains_key("code_forge"));
    assert!(!json.as_object().unwrap().contains_key("source_repos"));
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

#[test]
fn deserialize_legacy_environment_without_secrets() {
    let json = serde_json::json!({
        "name": "no-secrets-env",
        "github_repos": [],
        "docker_image": "ubuntu:latest"
    });

    let env: AmbientAgentEnvironment = serde_json::from_value(json).unwrap();
    assert_eq!(env.secrets, None);
}

#[test]
fn deserialize_with_empty_secrets() {
    let json = serde_json::json!({
        "name": "empty-secrets-env",
        "github_repos": [],
        "docker_image": "ubuntu:latest",
        "secrets": []
    });

    let env: AmbientAgentEnvironment = serde_json::from_value(json).unwrap();
    assert_eq!(env.secrets, Some(vec![]));
}

#[test]
fn deserialize_with_specific_secrets() {
    let json = serde_json::json!({
        "name": "secrets-env",
        "github_repos": [],
        "docker_image": "ubuntu:latest",
        "secrets": [
            {"name": "GH_TOKEN"},
            {"name": "NPM_TOKEN"}
        ]
    });

    let env: AmbientAgentEnvironment = serde_json::from_value(json).unwrap();
    let secrets = env.secrets.unwrap();
    assert_eq!(secrets.len(), 2);
    assert_eq!(secrets[0].name, "GH_TOKEN");
    assert_eq!(secrets[1].name, "NPM_TOKEN");
}

#[test]
fn serialize_with_secrets_none_omits_field() {
    let env = AmbientAgentEnvironment::new(
        "test-env".into(),
        None,
        vec![],
        "ubuntu:latest".into(),
        vec![],
    );

    let json = serde_json::to_value(&env).unwrap();
    assert!(!json.as_object().unwrap().contains_key("secrets"));
}

#[test]
fn serialize_with_empty_secrets_includes_field() {
    let mut env = AmbientAgentEnvironment::new(
        "test-env".into(),
        None,
        vec![],
        "ubuntu:latest".into(),
        vec![],
    );
    env.secrets = Some(vec![]);

    let json = serde_json::to_value(&env).unwrap();
    let secrets = json.get("secrets").unwrap();
    assert!(secrets.as_array().unwrap().is_empty());
}

#[test]
fn roundtrip_serde_with_secrets() {
    let mut env = AmbientAgentEnvironment::new(
        "secrets-rt".into(),
        None,
        vec![],
        "ubuntu:latest".into(),
        vec![],
    );
    env.secrets = Some(vec![
        EnvironmentSecretRef {
            name: "MY_SECRET".into(),
        },
        EnvironmentSecretRef {
            name: "OTHER_SECRET".into(),
        },
    ]);

    let serialized = serde_json::to_string(&env).unwrap();
    let deserialized: AmbientAgentEnvironment = serde_json::from_str(&serialized).unwrap();
    assert_eq!(env, deserialized);
}
