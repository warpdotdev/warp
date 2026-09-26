use super::{
    FORBIDDEN_FALLBACK_USER_MESSAGE, ForbiddenBody, MULTI_AGENT_REQUEST_SIZE_LIMIT_BYTES,
    classify_forbidden_body, encoded_len_exceeds_request_size_limit,
    user_message_for_forbidden_body,
};

#[test]
fn size_limit_is_strictly_greater_than_fifty_million_bytes() {
    assert!(!encoded_len_exceeds_request_size_limit(
        MULTI_AGENT_REQUEST_SIZE_LIMIT_BYTES
    ));
    assert!(encoded_len_exceeds_request_size_limit(
        MULTI_AGENT_REQUEST_SIZE_LIMIT_BYTES + 1
    ));
    assert!(!encoded_len_exceeds_request_size_limit(0));
}

#[test]
fn structured_json_403_preserves_error_message() {
    assert_eq!(
        classify_forbidden_body(r#"{"error": "model not allowed on this plan"}"#),
        ForbiddenBody::StructuredMessage("model not allowed on this plan".to_owned())
    );
}

#[test]
fn structured_json_403_trims_error_message() {
    assert_eq!(
        classify_forbidden_body("  {\"error\": \"  blocked by policy  \"}  "),
        ForbiddenBody::StructuredMessage("blocked by policy".to_owned())
    );
}

#[test]
fn html_403_is_opaque_and_uses_hedged_copy() {
    let html = "<!DOCTYPE html><html><head><title>403 Forbidden</title></head>\
         <body>Access denied</body></html>";

    assert_eq!(classify_forbidden_body(html), ForbiddenBody::Opaque);

    let message = user_message_for_forbidden_body(html);
    assert_eq!(message, FORBIDDEN_FALLBACK_USER_MESSAGE);
    assert!(!message.contains('<'));
    assert!(!message.to_ascii_lowercase().contains("html"));
    assert!(!message.contains("50MB"));
}

#[test]
fn empty_and_non_json_403_bodies_are_opaque() {
    assert_eq!(classify_forbidden_body(""), ForbiddenBody::Opaque);
    assert_eq!(classify_forbidden_body("   "), ForbiddenBody::Opaque);
    assert_eq!(classify_forbidden_body("forbidden"), ForbiddenBody::Opaque);
    assert_eq!(classify_forbidden_body("[]"), ForbiddenBody::Opaque);
    assert_eq!(
        classify_forbidden_body(r#"{"message": "no error field"}"#),
        ForbiddenBody::Opaque
    );
    assert_eq!(
        classify_forbidden_body(r#"{"error": ""}"#),
        ForbiddenBody::Opaque
    );
    assert_eq!(
        classify_forbidden_body(r#"{"error": 403}"#),
        ForbiddenBody::Opaque
    );
}

#[test]
fn json_error_that_looks_like_html_is_opaque() {
    assert_eq!(
        classify_forbidden_body(r#"{"error": "<html>denied</html>"}"#),
        ForbiddenBody::Opaque
    );
}
