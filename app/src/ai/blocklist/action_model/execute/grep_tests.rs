use super::*;
use crate::terminal::model::secrets::regexes::FIREBASE_AUTH_DOMAIN;
use crate::terminal::shell::ShellType;

#[test]
fn test_create_redacted_grep_error_event() {
    crate::terminal::model::set_user_and_enterprise_secret_regexes(
        [&regex::Regex::new(FIREBASE_AUTH_DOMAIN).expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    // Create input with a known secret pattern (Firebase domain)
    let queries = vec![
        "normal query".to_string(),
        "query with 127.0.0.1 secret".to_string(),
    ];
    let path = "path/to/file/with/127.0.0.1/secret".to_string();
    let shell_type = Some(ShellType::Bash);
    let working_directory = Some("/users/test/127.0.0.1".to_string());
    let absolute_path =
        "/absolute/path/with/127.0.0.1/secret".to_string();
    let error = GrepError::new("Error message".to_string())
        .with_command("grep 127.0.0.1".to_string())
        .with_output("Output with 127.0.0.1".to_string());

    // Call the function with the test inputs
    let event = create_redacted_grep_error_event(
        true,
        None,
        queries.clone(),
        path.clone(),
        shell_type,
        working_directory.clone(),
        absolute_path.clone(),
        error,
    );

    // Verify the telemetry event has redacted secrets
    if let TelemetryEvent::GrepToolFailed {
        queries: Some(redacted_queries),
        path: Some(redacted_path),
        shell_type: _,
        working_directory: Some(redacted_working_directory),
        absolute_path: Some(redacted_absolute_path),
        command: Some(redacted_command),
        output: Some(redacted_output),
        error: _,
        server_output_id: _,
    } = event
    {
        // Verify secrets are redacted from all relevant fields
        assert_eq!(redacted_queries.len(), 2);
        assert_eq!(redacted_queries[0], "normal query");
        assert!(!redacted_queries[1].contains("127.0.0.1"));
        assert!(redacted_queries[1].contains("*****"));

        assert!(!redacted_path.contains("127.0.0.1"));
        assert!(redacted_path.contains("*****"));

        assert!(!redacted_working_directory.contains("127.0.0.1"));
        assert!(redacted_working_directory.contains("*****"));

        assert!(!redacted_absolute_path.contains("127.0.0.1"));
        assert!(redacted_absolute_path.contains("*****"));

        assert!(!redacted_command.contains("127.0.0.1"));
        assert!(redacted_command.contains("*****"));

        assert!(!redacted_output.contains("127.0.0.1"));
        assert!(redacted_output.contains("*****"));
    } else {
        panic!("Expected GrepToolFailed event");
    }
}
