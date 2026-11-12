use super::*;

#[test]
fn custom_endpoint_overrides_model_and_authentication() {
    let secrets = HashMap::from([
        (
            "WARP_CLAUDE_BYOE_MODEL".into(),
            ManagedSecretValue::RawValue {
                value: "my-deployment".into(),
            },
        ),
        (
            "ANTHROPIC_BASE_URL".into(),
            ManagedSecretValue::RawValue {
                value: "https://endpoint.example".into(),
            },
        ),
        (
            "ANTHROPIC_API_KEY".into(),
            ManagedSecretValue::RawValue {
                value: "endpoint-key".into(),
            },
        ),
    ]);
    let mut config = Some(HarnessModelConfig {
        model_id: Uuid::new_v4().to_string(),
        reasoning_level: None,
    });
    let env = resolve(Harness::Claude, &secrets, &mut config).unwrap();
    assert_eq!(config.unwrap().model_id, "my-deployment");
    for name in [
        "ANTHROPIC_MODEL",
        "ANTHROPIC_CUSTOM_MODEL_OPTION",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    ] {
        assert_eq!(
            env.get(&OsString::from(name)),
            Some(&OsString::from("my-deployment"))
        );
    }
    assert_eq!(
        env.get(&OsString::from("ANTHROPIC_API_KEY")),
        Some(&OsString::from("endpoint-key"))
    );
    assert_eq!(
        env.get(&OsString::from("ANTHROPIC_BASE_URL")),
        Some(&OsString::from("https://endpoint.example"))
    );
    for name in [
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
    ] {
        assert_eq!(env.get(&OsString::from(name)), Some(&OsString::new()));
    }
}

#[test]
fn missing_endpoint_credentials_fail_closed() {
    let secrets = HashMap::from([(
        "WARP_CLAUDE_BYOE_MODEL".into(),
        ManagedSecretValue::RawValue {
            value: "deployment".into(),
        },
    )]);
    let mut config = Some(HarnessModelConfig {
        model_id: Uuid::new_v4().to_string(),
        reasoning_level: None,
    });
    assert!(resolve(Harness::Claude, &secrets, &mut config).is_err());
}

#[test]
fn ordinary_claude_selection_does_not_change() {
    let mut config = Some(HarnessModelConfig {
        model_id: "sonnet".into(),
        reasoning_level: None,
    });
    assert!(
        resolve(Harness::Claude, &HashMap::new(), &mut config)
            .unwrap()
            .is_empty()
    );
    assert_eq!(config.unwrap().model_id, "sonnet");
}

#[test]
fn custom_selection_without_runtime_credentials_fails_closed() {
    let mut config = Some(HarnessModelConfig {
        model_id: Uuid::new_v4().to_string(),
        reasoning_level: None,
    });
    assert!(resolve(Harness::Claude, &HashMap::new(), &mut config).is_err());
}
