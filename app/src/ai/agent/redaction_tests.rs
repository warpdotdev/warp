use regex::Regex;
use serial_test::serial;

use super::*;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentActionId, AIAgentActionResult, GrepResult};
use crate::terminal::model::secrets;

fn action_result_input(context: Arc<[AIAgentContext]>) -> AIAgentInput {
    AIAgentInput::ActionResult {
        result: AIAgentActionResult {
            id: AIAgentActionId::from("action-id".to_string()),
            task_id: TaskId::new("task-id".to_string()),
            result: AIAgentActionResultType::Grep(GrepResult::Cancelled),
        },
        context,
    }
}

fn context_arc() -> Arc<[AIAgentContext]> {
    Arc::from(vec![AIAgentContext::SelectedText("SECRET123".to_string())])
}

fn selected_text(input: &AIAgentInput) -> &str {
    let AIAgentInput::ActionResult { context, .. } = input else {
        panic!("expected ActionResult input");
    };
    let AIAgentContext::SelectedText(text) = &context[0] else {
        panic!("expected SelectedText context");
    };
    text
}

/// A batch of inputs sharing one context arc (as `send_query` produces for a batch of completed
/// action results) must still have the shared secret redacted in every input.
#[test]
#[serial]
fn test_redact_inputs_redacts_a_context_shared_across_inputs() {
    secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("SECRET123").expect("valid regex")],
        std::iter::empty(),
    );

    let shared_context = context_arc();
    let mut inputs = vec![
        action_result_input(shared_context.clone()),
        action_result_input(shared_context.clone()),
        action_result_input(shared_context),
    ];

    redact_inputs(&mut inputs);

    for input in &inputs {
        assert_eq!(selected_text(input), "*********");
    }
}

/// Inputs that do not share a context must each have their own secret redacted independently.
#[test]
#[serial]
fn test_redact_inputs_redacts_independent_contexts() {
    secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("SECRET123").expect("valid regex")],
        std::iter::empty(),
    );

    let mut inputs = vec![
        action_result_input(context_arc()),
        action_result_input(context_arc()),
    ];

    redact_inputs(&mut inputs);

    for input in &inputs {
        assert_eq!(selected_text(input), "*********");
    }
}
