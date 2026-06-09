use super::*;

#[test]
fn warping_verb_session_key_uses_same_stream_for_exchanges_from_same_output() {
    let response_stream_id = ResponseStreamId::new_for_test();
    let first_exchange_id = AIAgentExchangeId::new();
    let second_exchange_id = AIAgentExchangeId::new();
    let mut exchange_ids = HashSet::new();
    exchange_ids.insert(first_exchange_id);
    exchange_ids.insert(second_exchange_id);

    assert_eq!(
        warping_verb_session_key_for_exchange(
            Some(first_exchange_id),
            Some(&response_stream_id),
            &exchange_ids,
        ),
        warping_verb_session_key_for_exchange(
            Some(second_exchange_id),
            Some(&response_stream_id),
            &exchange_ids,
        )
    );
}

#[test]
fn warping_verb_session_key_falls_back_to_exchange_for_untracked_exchange() {
    let response_stream_id = ResponseStreamId::new_for_test();
    let tracked_exchange_id = AIAgentExchangeId::new();
    let untracked_exchange_id = AIAgentExchangeId::new();
    let mut exchange_ids = HashSet::new();
    exchange_ids.insert(tracked_exchange_id);

    assert_eq!(
        warping_verb_session_key_for_exchange(
            Some(untracked_exchange_id),
            Some(&response_stream_id),
            &exchange_ids,
        ),
        format!("exchange:{untracked_exchange_id}")
    );
}
