use super::*;

fn snapshot(buffer_text: &str, cursor_byte_offset: usize) -> TuiCompletionInputSnapshot {
    TuiCompletionInputSnapshot {
        buffer_text: buffer_text.to_owned(),
        cursor_byte_offset,
    }
}

#[test]
fn completion_requests_reject_every_stale_snapshot_dimension() {
    let input = snapshot("git che", 7);
    let request = CompletionRequestSnapshot {
        input: input.clone(),
        session_id: SessionId::from(42),
        current_working_directory: "/repo".to_owned(),
        generation: 7,
    };
    assert!(completion_request_is_current(
        &request,
        7,
        Some(&input),
        Some(SessionId::from(42)),
        Some("/repo"),
        false,
    ));

    let changed_input = snapshot("git checkout", 12);
    for is_current in [
        completion_request_is_current(
            &request,
            8,
            Some(&input),
            Some(SessionId::from(42)),
            Some("/repo"),
            false,
        ),
        completion_request_is_current(
            &request,
            7,
            Some(&changed_input),
            Some(SessionId::from(42)),
            Some("/repo"),
            false,
        ),
        completion_request_is_current(
            &request,
            7,
            Some(&input),
            Some(SessionId::from(43)),
            Some("/repo"),
            false,
        ),
        completion_request_is_current(
            &request,
            7,
            Some(&input),
            Some(SessionId::from(42)),
            Some("/other"),
            false,
        ),
        completion_request_is_current(
            &request,
            7,
            Some(&input),
            Some(SessionId::from(42)),
            Some("/repo"),
            true,
        ),
    ] {
        assert!(!is_current);
    }
}
