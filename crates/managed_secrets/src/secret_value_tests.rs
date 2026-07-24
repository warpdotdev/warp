use crate::secret_value::ManagedSecretValue;

/// Test to ensure that `raw_value` secrets are serialized in the format that the server expects.
#[test]
fn test_serialize_raw_value() {
    let secret = ManagedSecretValue::RawValue {
        value: "secret".to_string(),
    };
    let serialized = serde_json::to_string(&secret).expect("failed to serialize");
    assert_eq!(serialized, "{\"value\":\"secret\"}");
}

/// Test to ensure that the [`ManagedSecretValue`] debug representation does not leak the secret value.
#[test]
fn test_debug_representation_no_secrets() {
    let secret = ManagedSecretValue::RawValue {
        value: "secret".to_string(),
    };
    let debug_representation = format!("{:?}", secret);
    assert!(
        !debug_representation.contains("secret"),
        "debug representation contains secret value: {debug_representation}"
    );
}

/// Test to ensure that `anthropic_api_key` secrets are serialized in the format that the server expects.
#[test]
fn test_serialize_anthropic_api_key() {
    let secret = ManagedSecretValue::AnthropicApiKey {
        api_key: "sk-ant-test-key".to_string(),
        base_url: None,
    };
    let serialized = serde_json::to_string(&secret).expect("failed to serialize");
    assert_eq!(serialized, "{\"api_key\":\"sk-ant-test-key\"}");
}

/// Test to ensure that the [`ManagedSecretValue::AnthropicApiKey`] debug representation does not leak the API key.
#[test]
fn test_debug_representation_no_secrets_anthropic_api_key() {
    let secret = ManagedSecretValue::AnthropicApiKey {
        api_key: "sk-ant-secret-key".to_string(),
        base_url: None,
    };
    let debug_representation = format!("{:?}", secret);
    assert!(
        !debug_representation.contains("sk-ant-secret-key"),
        "debug representation contains secret value: {debug_representation}"
    );
}

/// Test to ensure that `anthropic_bedrock_api_key` secrets are serialized in the format that the server expects.
#[test]
fn test_serialize_anthropic_bedrock_api_key() {
    let secret = ManagedSecretValue::AnthropicBedrockApiKey {
        aws_bearer_token_bedrock: "test-token".to_string(),
        aws_region: "us-east-1".to_string(),
    };
    let serialized = serde_json::to_string(&secret).expect("failed to serialize");
    assert_eq!(
        serialized,
        "{\"aws_bearer_token_bedrock\":\"test-token\",\"aws_region\":\"us-east-1\"}"
    );
}

/// Test to ensure that `anthropic_bedrock_access_key` secrets are serialized in the format that the server expects.
#[test]
fn test_serialize_anthropic_bedrock_access_key() {
    let secret = ManagedSecretValue::AnthropicBedrockAccessKey {
        aws_access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
        aws_secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
        aws_session_token: Some("FwoGZXIvYXdzEBY".to_string()),
        aws_region: "us-east-1".to_string(),
    };
    let serialized = serde_json::to_string(&secret).expect("failed to serialize");
    assert_eq!(
        serialized,
        "{\"aws_access_key_id\":\"AKIAIOSFODNN7EXAMPLE\",\"aws_secret_access_key\":\"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\",\"aws_session_token\":\"FwoGZXIvYXdzEBY\",\"aws_region\":\"us-east-1\"}"
    );
}

/// Test to ensure that an `anthropic_bedrock_access_key` secret with no session
/// token (i.e. persistent IAM credentials) omits the `aws_session_token` field
/// from the JSON payload sent to the server.
#[test]
fn test_serialize_anthropic_bedrock_access_key_without_session_token() {
    let secret = ManagedSecretValue::AnthropicBedrockAccessKey {
        aws_access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
        aws_secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
        aws_session_token: None,
        aws_region: "us-east-1".to_string(),
    };
    let serialized = serde_json::to_string(&secret).expect("failed to serialize");
    assert_eq!(
        serialized,
        "{\"aws_access_key_id\":\"AKIAIOSFODNN7EXAMPLE\",\"aws_secret_access_key\":\"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\",\"aws_region\":\"us-east-1\"}"
    );
    assert!(
        !serialized.contains("aws_session_token"),
        "aws_session_token must not appear in serialized JSON when None: {serialized}"
    );
}

/// Test that the constructor helper correctly passes an optional session token through.
#[test]
fn test_anthropic_bedrock_access_key_constructor_optional_session_token() {
    let with_token = ManagedSecretValue::anthropic_bedrock_access_key(
        "AKID",
        "secret",
        Some("token".to_string()),
        "us-east-1",
    );
    match with_token {
        ManagedSecretValue::AnthropicBedrockAccessKey {
            aws_session_token, ..
        } => assert_eq!(aws_session_token.as_deref(), Some("token")),
        _ => panic!("unexpected variant"),
    }

    let without_token =
        ManagedSecretValue::anthropic_bedrock_access_key("AKID", "secret", None, "us-east-1");
    match without_token {
        ManagedSecretValue::AnthropicBedrockAccessKey {
            aws_session_token, ..
        } => assert!(aws_session_token.is_none()),
        _ => panic!("unexpected variant"),
    }
}

/// Test to ensure that the [`ManagedSecretValue::AnthropicBedrockAccessKey`] debug representation does not leak secrets.
#[test]
fn test_debug_representation_no_secrets_anthropic_bedrock_access_key() {
    let secret = ManagedSecretValue::AnthropicBedrockAccessKey {
        aws_access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
        aws_secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
        aws_session_token: Some("FwoGZXIvYXdzEBY".to_string()),
        aws_region: "us-west-2".to_string(),
    };
    let debug_representation = format!("{:?}", secret);
    assert!(
        !debug_representation.contains("AKIAIOSFODNN7EXAMPLE"),
        "debug representation contains aws_access_key_id: {debug_representation}"
    );
    assert!(
        !debug_representation.contains("wJalrXUtnFEMI"),
        "debug representation contains aws_secret_access_key: {debug_representation}"
    );
    assert!(
        !debug_representation.contains("FwoGZXIvYXdzEBY"),
        "debug representation contains aws_session_token: {debug_representation}"
    );
    assert!(
        !debug_representation.contains("us-west-2"),
        "debug representation contains aws_region: {debug_representation}"
    );
}

/// Test to ensure that the [`ManagedSecretValue::AnthropicBedrockApiKey`] debug representation does not leak secrets.
#[test]
fn test_debug_representation_no_secrets_anthropic_bedrock_api_key() {
    let secret = ManagedSecretValue::AnthropicBedrockApiKey {
        aws_bearer_token_bedrock: "secret-token".to_string(),
        aws_region: "us-west-2".to_string(),
    };
    let debug_representation = format!("{:?}", secret);
    assert!(
        !debug_representation.contains("secret-token"),
        "debug representation contains secret value: {debug_representation}"
    );
    assert!(
        !debug_representation.contains("us-west-2"),
        "debug representation contains aws_region: {debug_representation}"
    );
}
