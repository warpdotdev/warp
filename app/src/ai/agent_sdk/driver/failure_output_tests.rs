use super::{FAILURE_OUTPUT_MAX_BYTES, prepare_failure_output};

const TRUNCATION_MARKER: &str = "\n… output truncated …\n";

#[test]
fn failure_output_is_trimmed() {
    assert_eq!(
        prepare_failure_output(" \n Harness startup failed \n ", TRUNCATION_MARKER),
        "Harness startup failed"
    );
}

#[test]
fn failure_output_redacts_secrets() {
    let secret = "AKIAIOSFODNN7EXAMPLE";

    let output = prepare_failure_output(
        &format!("Harness failed with credential {secret}"),
        TRUNCATION_MARKER,
    );

    assert!(!output.contains(secret));
    assert_eq!(
        output,
        format!(
            "Harness failed with credential {}",
            "*".repeat(secret.len())
        )
    );
}

#[test]
fn failure_output_truncation_retains_start_and_end() {
    let output = format!("START{}END", "x".repeat(FAILURE_OUTPUT_MAX_BYTES));

    let truncated = prepare_failure_output(&output, TRUNCATION_MARKER);

    assert!(truncated.len() <= FAILURE_OUTPUT_MAX_BYTES);
    assert!(truncated.starts_with("START"));
    assert!(truncated.ends_with("END"));
    assert!(truncated.contains(TRUNCATION_MARKER));
}

#[test]
fn failure_output_truncation_preserves_unicode_boundaries() {
    let output = format!("START-世{}界-END", "🙂".repeat(FAILURE_OUTPUT_MAX_BYTES));

    let truncated = prepare_failure_output(&output, TRUNCATION_MARKER);

    assert!(truncated.len() <= FAILURE_OUTPUT_MAX_BYTES);
    assert!(truncated.starts_with("START-世"));
    assert!(truncated.ends_with("界-END"));
    assert!(truncated.contains(TRUNCATION_MARKER));
}
