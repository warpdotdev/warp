use crate::ai::agent::AIAgentActionResultType;

/// The result an action finishes with when a hook denies it before it runs.
pub(crate) fn denied_result(reason: &str) -> AIAgentActionResultType {
    AIAgentActionResultType::DeniedByHook {
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
#[path = "denied_result_tests.rs"]
mod tests;
