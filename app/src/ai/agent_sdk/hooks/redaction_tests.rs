use super::*;

#[test]
fn oz_hooks_redaction_removes_known_secrets_and_credentials_before_truncation() {
    let redactor = HookRedactor::new(["known-secret".to_string()]);
    let input = "known-secret Authorization: Bearer top-secret api_key=another-secret";

    let output = redactor.redact_text(input, 128);

    assert!(!output.value.contains("known-secret"));
    assert!(!output.value.contains("top-secret"));
    assert!(!output.value.contains("another-secret"));
}

#[test]
fn oz_hooks_redaction_truncates_only_at_utf8_boundaries_with_metadata() {
    let output = HookRedactor::new([]).redact_text("abc😀def", 5);

    assert_eq!(output.value, "abc");
    assert_eq!(
        output.truncation,
        Some(TruncationMetadata {
            truncated: true,
            original_bytes: 10,
            included_bytes: 3,
        })
    );
}

#[test]
fn oz_hooks_redaction_represents_omitted_file_content_structurally() {
    let value =
        RedactedValue::object([("content", RedactedValue::redacted("file_content", 18_432))]);
    let serialized = serde_json::to_value(value).unwrap();

    assert_eq!(serialized["content"]["redacted"], true);
    assert_eq!(serialized["content"]["reason"], "file_content");
    assert_eq!(serialized["content"]["byte_count"], 18_432);
    assert!(!contains_prohibited_payload_key(&serialized));
}
