use warp_multi_agent_api as api;

use super::*;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{AIAgentActionId, AIAgentActionResult};

const REASON: &str = "production branches are off limits";

#[test]
fn a_hook_denial_is_failed_but_not_cancelled() {
    let result = denied_result(REASON);
    assert_eq!(
        result,
        AIAgentActionResultType::DeniedByHook {
            reason: REASON.to_owned(),
        }
    );
    assert!(result.is_failed());
    assert!(!result.is_cancelled());
}

#[test]
fn a_hook_denials_reason_reaches_the_wire() {
    let result = AIAgentActionResult {
        id: AIAgentActionId::from("call_1".to_owned()),
        task_id: TaskId::new("task".to_owned()),
        result: denied_result(REASON),
    };
    let wire = api::request::input::user_inputs::user_input::Input::try_from(result)
        .expect("the result converts to the wire");
    assert!(format!("{wire:?}").contains(REASON));
}
