use std::collections::HashSet;

use chrono::{Local, TimeZone};

use super::{
    MAX_RESTORED_AI_EXCHANGES_PER_CONVERSATION, collect_bounded_exchanges_for_restoration,
    most_recent_exchanges,
};
use crate::ai::agent::conversation::AIConversation;
use crate::ai::agent::{AIAgentExchange, AIAgentExchangeId, AIAgentOutputStatus};
use crate::ai::llms::LLMId;
use crate::terminal::view::load_ai_conversation::RestoredAIConversation;

fn exchange_at(seconds: i64) -> AIAgentExchange {
    AIAgentExchange {
        id: AIAgentExchangeId::new(),
        input: vec![],
        output_status: AIAgentOutputStatus::Streaming { output: None },
        added_message_ids: HashSet::new(),
        start_time: Local.timestamp_opt(seconds, 0).unwrap(),
        finish_time: None,
        time_to_first_token_ms: None,
        working_directory: None,
        model_id: LLMId::from("test-model"),
        request_cost: None,
        coding_model_id: LLMId::from("test-coding-model"),
        cli_agent_model_id: LLMId::from("test-cli-agent-model"),
        computer_use_model_id: LLMId::from("test-computer-use-model"),
        response_initiator: None,
    }
}

#[test]
fn most_recent_exchanges_keeps_everything_under_the_cap() {
    let exchanges: Vec<AIAgentExchange> = (0..3).map(exchange_at).collect();
    let refs: Vec<&AIAgentExchange> = exchanges.iter().collect();

    let retained = most_recent_exchanges(refs, 5);

    let retained_ids: Vec<_> = retained.iter().map(|e| e.id).collect();
    assert_eq!(
        retained_ids,
        vec![exchanges[0].id, exchanges[1].id, exchanges[2].id]
    );
}

#[test]
fn most_recent_exchanges_keeps_exactly_the_most_recent_by_start_time() {
    // Deliberately out of chronological order to assert the function sorts by `start_time`
    // rather than trusting the input order.
    let exchanges: Vec<AIAgentExchange> = vec![
        exchange_at(30),
        exchange_at(10),
        exchange_at(50),
        exchange_at(20),
        exchange_at(40),
    ];
    let refs: Vec<&AIAgentExchange> = exchanges.iter().collect();

    let retained = most_recent_exchanges(refs, 2);

    // Only the two most recent by start_time (40, 50) survive, oldest-first.
    let retained_ids: Vec<_> = retained.iter().map(|e| e.id).collect();
    assert_eq!(retained_ids, vec![exchanges[4].id, exchanges[2].id]);
}

#[test]
fn most_recent_exchanges_at_exact_cap_keeps_all() {
    let exchanges: Vec<AIAgentExchange> = (0..4).map(exchange_at).collect();
    let refs: Vec<&AIAgentExchange> = exchanges.iter().collect();

    let retained = most_recent_exchanges(refs, 4);

    assert_eq!(retained.len(), 4);
}

#[test]
fn most_recent_exchanges_with_zero_cap_drops_everything() {
    let exchanges: Vec<AIAgentExchange> = (0..3).map(exchange_at).collect();
    let refs: Vec<&AIAgentExchange> = exchanges.iter().collect();

    let retained = most_recent_exchanges(refs, 0);

    assert!(retained.is_empty());
}

#[test]
fn most_recent_exchanges_with_empty_input_returns_empty() {
    let retained = most_recent_exchanges(vec![], 10);

    assert!(retained.is_empty());
}

fn conversation_with_exchanges_at(seconds: impl IntoIterator<Item = i64>) -> AIConversation {
    let mut conversation = AIConversation::new(false, false);
    for seconds in seconds {
        conversation.append_root_exchange_for_test(exchange_at(seconds));
    }
    conversation
}

#[test]
fn collect_bounded_exchanges_truncates_each_conversation_before_cloning_and_merges_sorted() {
    // One conversation under the cap, one comfortably over it.
    let under_cap = conversation_with_exchanges_at([100, 200]);
    let over_cap_seconds: Vec<i64> =
        (0..MAX_RESTORED_AI_EXCHANGES_PER_CONVERSATION as i64 + 2).collect();
    let over_cap = conversation_with_exchanges_at(over_cap_seconds.clone());

    let expected_retained_from_over_cap: Vec<AIAgentExchangeId> = over_cap
        .all_exchanges()
        .into_iter()
        .filter(|exchange| {
            exchange.start_time.timestamp()
                >= over_cap_seconds.len() as i64 - MAX_RESTORED_AI_EXCHANGES_PER_CONVERSATION as i64
        })
        .map(|exchange| exchange.id)
        .collect();
    assert_eq!(
        expected_retained_from_over_cap.len(),
        MAX_RESTORED_AI_EXCHANGES_PER_CONVERSATION
    );

    let restored = vec![
        RestoredAIConversation::new(under_cap.clone()),
        RestoredAIConversation::new(over_cap),
    ];

    let collected = collect_bounded_exchanges_for_restoration(&restored);

    // The under-cap conversation keeps all of its exchanges; the over-cap one is truncated to
    // the cap. Total count reflects what actually got cloned, not just what came out the other
    // end of block-index assignment.
    assert_eq!(
        collected.len(),
        2 + MAX_RESTORED_AI_EXCHANGES_PER_CONVERSATION
    );

    let under_cap_ids: HashSet<AIAgentExchangeId> = under_cap
        .all_exchanges()
        .into_iter()
        .map(|exchange| exchange.id)
        .collect();
    let over_cap_retained_ids: HashSet<AIAgentExchangeId> =
        expected_retained_from_over_cap.into_iter().collect();

    for (exchange, _) in &collected {
        assert!(
            under_cap_ids.contains(&exchange.id) || over_cap_retained_ids.contains(&exchange.id),
            "exchange {:?} should either belong to the under-cap conversation or be one of the \
             most recent retained exchanges from the over-cap conversation",
            exchange.id
        );
    }

    // The combined result is sorted by start_time across conversations, regardless of which
    // conversation each exchange came from.
    let start_times: Vec<_> = collected
        .iter()
        .map(|(exchange, _)| exchange.start_time)
        .collect();
    let mut sorted_start_times = start_times.clone();
    sorted_start_times.sort();
    assert_eq!(start_times, sorted_start_times);
}
