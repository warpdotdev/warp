use super::parse_ambient_task_id;

#[test]
fn parse_ambient_task_id_accepts_valid_ids() {
    let task_id =
        parse_ambient_task_id("550e8400-e29b-41d4-a716-446655440000", "Invalid run ID").unwrap();

    assert_eq!(task_id.to_string(), "550e8400-e29b-41d4-a716-446655440000");
}

#[test]
fn parse_ambient_task_id_preserves_error_prefix() {
    let err = parse_ambient_task_id("not-a-run-id", "Invalid run ID").unwrap_err();

    assert!(err.to_string().contains("Invalid run ID 'not-a-run-id'"));
}
