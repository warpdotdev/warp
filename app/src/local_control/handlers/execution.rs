//! Execution-underlying handlers for local-control actions.
use ::local_control::protocol::{ActionParams, LocalControlAuditRecord, WorkflowRunParams};
use ::local_control::{
    ActionKind, ControlError, ErrorCode, PermissionCategory, RequestEnvelope,
};

pub(crate) fn run_input(
    request: &RequestEnvelope,
    authenticated_user_subject: &str,
) -> Result<serde_json::Value, ControlError> {
    let ActionParams::Text { text } = request.action.params_as::<ActionParams>()? else {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "input.run requires text parameters",
        ));
    };
    if text.trim().is_empty() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "input.run requires non-empty text",
        ));
    }
    fail_closed_without_execution_policy(request.action.kind, authenticated_user_subject)
}

pub(crate) fn run_drive_workflow(
    request: &RequestEnvelope,
    authenticated_user_subject: &str,
) -> Result<serde_json::Value, ControlError> {
    let ActionParams::WorkflowRun(params) = request.action.params_as::<ActionParams>()? else {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "drive.workflow.run requires workflow run parameters",
        ));
    };
    validate_workflow_run_params(&params)?;
    fail_closed_without_execution_policy(request.action.kind, authenticated_user_subject)
}

pub(crate) fn execution_audit_record(
    action: ActionKind,
    authenticated_user_subject: &str,
) -> LocalControlAuditRecord {
    LocalControlAuditRecord {
        action: action.as_str().to_owned(),
        target_scope: action.metadata().target_scope,
        permission_category: PermissionCategory::MutateUnderlyingData,
        authenticated_user_subject: authenticated_user_subject.to_owned(),
    }
}

fn validate_workflow_run_params(params: &WorkflowRunParams) -> Result<(), ControlError> {
    if params.id.0.trim().is_empty() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "drive.workflow.run requires a non-empty workflow id",
        ));
    }
    for arg in &params.args {
        if excluded_submission_argument_name(&arg.name) {
            return Err(ControlError::new(
                ErrorCode::UnsupportedAction,
                "drive.workflow.run does not accept accepted-command or agent-prompt submissions",
            ));
        }
    }
    Ok(())
}

fn excluded_submission_argument_name(name: &str) -> bool {
    matches!(
        name,
        "accepted_command"
            | "accepted-command"
            | "agent_prompt"
            | "agent-prompt"
            | "internal_dispatch"
            | "internal-dispatch"
    )
}

fn fail_closed_without_execution_policy(
    action: ActionKind,
    authenticated_user_subject: &str,
) -> Result<serde_json::Value, ControlError> {
    let audit = execution_audit_record(action, authenticated_user_subject);
    let details = serde_json::to_string(&audit).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to encode local-control execution audit record",
            err.to_string(),
        )
    })?;
    Err(ControlError::with_details(
        ErrorCode::ExecutionContextNotAllowed,
        format!(
            "{} requires an explicit execution approval policy, but no approval is available",
            action.as_str()
        ),
        details,
    ))
}
