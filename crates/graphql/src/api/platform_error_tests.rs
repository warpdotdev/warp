use super::*;

fn response_with_metadata(
    metadata: Vec<(&str, &str)>,
    debug: Option<&str>,
) -> PlatformErrorInfoResponse {
    PlatformErrorInfoResponse {
        code: PlatformErrorCode::ResourceUnavailable,
        retryable: true,
        metadata: metadata
            .into_iter()
            .map(|(key, value)| PlatformErrorMetadataResponse {
                key: key.to_string(),
                value: value.to_string(),
            })
            .collect(),
        debug: debug.map(str::to_string),
    }
}

#[test]
fn response_decodes_and_input_encodes_platform_error_info() {
    let response = response_with_metadata(
        vec![("resource", "installation"), ("provider", "github")],
        Some("request-id=example"),
    );

    let info = PlatformErrorInfo::from(response);
    let input = PlatformErrorInput::from(info.clone());

    assert_eq!(info.code, PlatformErrorCode::ResourceUnavailable);
    assert!(info.retryable);
    assert_eq!(info.metadata["provider"], "github");
    assert_eq!(info.metadata["resource"], "installation");
    assert_eq!(info.debug.as_deref(), Some("request-id=example"));
    assert_eq!(input.code, info.code);
    assert_eq!(input.retryable, info.retryable);
    assert_eq!(input.metadata.len(), 2);
    assert_eq!(input.metadata[0].key, "provider");
    assert_eq!(input.metadata[0].value, "github");
    assert_eq!(input.metadata[1].key, "resource");
    assert_eq!(input.metadata[1].value, "installation");
    assert_eq!(input.debug, info.debug);
}

#[test]
fn duplicate_metadata_keys_decode_with_the_last_value() {
    let response =
        response_with_metadata(vec![("provider", "github"), ("provider", "gitlab")], None);

    let info = PlatformErrorInfo::from(response);
    let input = PlatformErrorInput::from(info.clone());

    assert_eq!(
        info.metadata,
        BTreeMap::from([("provider".to_string(), "gitlab".to_string())])
    );
    assert_eq!(input.metadata.len(), 1);
    assert_eq!(input.metadata[0].key, "provider");
    assert_eq!(input.metadata[0].value, "gitlab");
}

#[test]
fn optional_debug_is_preserved_when_absent() {
    let info = PlatformErrorInfo::from(response_with_metadata(Vec::new(), None));
    let input = PlatformErrorInput::from(info.clone());

    assert_eq!(info.debug, None);
    assert_eq!(input.debug, None);
}

#[test]
fn without_debug_preserves_structured_fields() {
    let info = PlatformErrorInfo::from(response_with_metadata(
        vec![("provider", "github")],
        Some("request-id=example"),
    ));

    let redacted = info.without_debug();

    assert_eq!(redacted.code, PlatformErrorCode::ResourceUnavailable);
    assert!(redacted.retryable);
    assert_eq!(redacted.metadata["provider"], "github");
    assert_eq!(redacted.debug, None);
}
