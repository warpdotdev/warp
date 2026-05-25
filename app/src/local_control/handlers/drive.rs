use ::local_control::protocol::{
    DriveCreateParams, DriveDeleteParams, DriveInsertParams, DriveMutationResult,
    DriveObjectSummary, DriveObjectType as ControlDriveObjectType, DriveRunParams, DriveTarget,
    DriveUpdateParams,
};
use ::local_control::{ActionKind, ControlError, ErrorCode, RequestEnvelope};
use warpui::{ModelContext, SingletonEntity};

use crate::auth::AuthStateProvider;
use crate::cloud_object::model::generic_string_model::GenericStringObjectId;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{
    CloudObject, GenericStringObjectFormat, JsonObjectType, ObjectType, Owner,
};
use crate::env_vars::{CloudEnvVarCollection, CloudEnvVarCollectionModel, EnvVarCollection};
use crate::local_control::LocalControlBridge;
use crate::notebooks::{CloudNotebook, CloudNotebookModel, NotebookId};
use crate::server::ids::{ClientId, SyncId};
use crate::workflows::workflow::Workflow;
use crate::workflows::{CloudWorkflow, CloudWorkflowModel, WorkflowId};

pub(crate) fn create_drive_object(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_drive_target(&request.target, request.action.kind)?;
    let params = request.action.params_as::<DriveCreateParams>()?;
    if params.name.is_empty() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "drive.create requires a non-empty Drive object name",
        ));
    }
    let client_id = ClientId::new();
    let sync_id = SyncId::ClientId(client_id);
    let owner = authenticated_user_owner(ctx)?;
    CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| match params.object_type {
        ControlDriveObjectType::Workflow | ControlDriveObjectType::Prompt => {
            let workflow =
                workflow_from_drive_content(params.object_type, &params.name, params.content)?;
            cloud_model.create_object(
                sync_id,
                CloudWorkflow::new_local(CloudWorkflowModel::new(workflow), owner, None, client_id),
                ctx,
            );
            Ok(())
        }
        ControlDriveObjectType::Notebook => {
            let notebook = notebook_from_drive_content(&params.name, params.content, None)?;
            cloud_model.create_object(
                sync_id,
                CloudNotebook::new_local(notebook, owner, None, client_id),
                ctx,
            );
            Ok(())
        }
        ControlDriveObjectType::Environment => {
            let env_vars = env_vars_from_drive_content(&params.name, params.content)?;
            cloud_model.create_object(
                sync_id,
                CloudEnvVarCollection::new_local(
                    CloudEnvVarCollectionModel::new(env_vars),
                    owner,
                    None,
                    client_id,
                ),
                ctx,
            );
            Ok(())
        }
    })?;
    let cloud_model = CloudModel::as_ref(ctx);
    let object = cloud_model.get_by_uid(&sync_id.uid()).ok_or_else(|| {
        ControlError::new(
            ErrorCode::Internal,
            "drive.create could not resolve the created Drive object",
        )
    })?;
    drive_mutation_result(object, params.object_type)
}

pub(crate) fn update_drive_object(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_drive_target(&request.target, request.action.kind)?;
    let params = request.action.params_as::<DriveUpdateParams>()?;
    validate_drive_request_id(&params.id, request.action.kind)?;
    validate_drive_target_matches_params(
        &request.target,
        params.object_type,
        &params.id,
        request.action.kind,
    )?;
    let (sync_id, existing_notebook) = {
        let cloud_model = CloudModel::as_ref(ctx);
        let object = drive_object_for_mutation(
            cloud_model,
            params.object_type,
            &params.id,
            request.action.kind,
        )?;
        (
            object.sync_id(),
            object
                .as_any()
                .downcast_ref::<CloudNotebook>()
                .map(|notebook| notebook.model().clone()),
        )
    };
    CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| match params.object_type {
        ControlDriveObjectType::Workflow | ControlDriveObjectType::Prompt => {
            let workflow =
                workflow_from_drive_content(params.object_type, "", params.content.clone())?;
            cloud_model.update_object_from_edit::<WorkflowId, CloudWorkflowModel>(
                CloudWorkflowModel::new(workflow),
                sync_id,
                ctx,
            );
            Ok(())
        }
        ControlDriveObjectType::Notebook => {
            let notebook =
                notebook_from_drive_content("", params.content.clone(), existing_notebook)?;
            cloud_model
                .update_object_from_edit::<NotebookId, CloudNotebookModel>(notebook, sync_id, ctx);
            Ok(())
        }
        ControlDriveObjectType::Environment => {
            let env_vars = env_vars_from_drive_content("", params.content.clone())?;
            cloud_model
                .update_object_from_edit::<GenericStringObjectId, CloudEnvVarCollectionModel>(
                    CloudEnvVarCollectionModel::new(env_vars),
                    sync_id,
                    ctx,
                );
            Ok(())
        }
    })?;
    let cloud_model = CloudModel::as_ref(ctx);
    let object = drive_object_for_mutation(
        cloud_model,
        params.object_type,
        &params.id,
        request.action.kind,
    )?;
    drive_mutation_result(object, params.object_type)
}

pub(crate) fn delete_drive_object(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_drive_target(&request.target, request.action.kind)?;
    let params = request.action.params_as::<DriveDeleteParams>()?;
    validate_drive_request_id(&params.id, request.action.kind)?;
    validate_drive_target_matches_params(
        &request.target,
        params.object_type,
        &params.id,
        request.action.kind,
    )?;
    let (sync_id, summary) = {
        let cloud_model = CloudModel::as_ref(ctx);
        let object = drive_object_for_mutation(
            cloud_model,
            params.object_type,
            &params.id,
            request.action.kind,
        )?;
        let summary = drive_object_summary(object).ok_or_else(|| {
            ControlError::new(
                ErrorCode::UnsupportedAction,
                "drive.delete does not support this Drive object type",
            )
        })?;
        (object.sync_id(), summary)
    };
    CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
        cloud_model.delete_object(sync_id, ctx);
    });
    to_drive_data(DriveMutationResult {
        object: summary,
        execution_id: None,
    })
}

pub(crate) fn execute_drive_action_with_policy(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_drive_target(&request.target, request.action.kind)?;
    match request.action.kind {
        ActionKind::DriveRun => {
            let params = request.action.params_as::<DriveRunParams>()?;
            validate_drive_request_id(&params.id, request.action.kind)?;
            validate_drive_target_matches_params(
                &request.target,
                params.object_type,
                &params.id,
                request.action.kind,
            )?;
            if params.object_type != ControlDriveObjectType::Workflow {
                return Err(ControlError::new(
                    ErrorCode::UnsupportedAction,
                    "drive.run only supports workflow objects",
                ));
            }
            ensure_drive_execution_policy_approved(request.action.kind)?;
            let cloud_model = CloudModel::as_ref(ctx);
            let object = drive_object_for_mutation(
                cloud_model,
                params.object_type,
                &params.id,
                request.action.kind,
            )?;
            drive_mutation_result(object, params.object_type)
        }
        ActionKind::DriveInsert => {
            let params = request.action.params_as::<DriveInsertParams>()?;
            validate_drive_request_id(&params.id, request.action.kind)?;
            validate_drive_target_matches_params(
                &request.target,
                params.object_type,
                &params.id,
                request.action.kind,
            )?;
            if params.object_type != ControlDriveObjectType::Notebook {
                return Err(ControlError::new(
                    ErrorCode::UnsupportedAction,
                    "drive.insert only supports notebook objects",
                ));
            }
            ensure_drive_execution_policy_approved(request.action.kind)?;
            let cloud_model = CloudModel::as_ref(ctx);
            let object = drive_object_for_mutation(
                cloud_model,
                params.object_type,
                &params.id,
                request.action.kind,
            )?;
            drive_mutation_result(object, params.object_type)
        }
        action => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!("{} is not a Drive execution action", action.as_str()),
        )),
    }
}

fn authenticated_user_owner(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Owner, ControlError> {
    let auth_state = AuthStateProvider::as_ref(ctx).get();
    if auth_state.is_anonymous_or_logged_out() {
        return Err(ControlError::new(
            ErrorCode::AuthenticatedUserUnavailable,
            "this action requires a logged-in Warp user",
        ));
    }
    auth_state
        .user_id()
        .map(|user_uid| Owner::User { user_uid })
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::AuthenticatedUserUnavailable,
                "this action requires a logged-in Warp user",
            )
        })
}

#[derive(serde::Deserialize)]
struct NotebookDriveContent {
    title: Option<String>,
    data: Option<String>,
}

fn workflow_from_drive_content(
    object_type: ControlDriveObjectType,
    fallback_name: &str,
    content: serde_json::Value,
) -> Result<Workflow, ControlError> {
    if let Ok(mut workflow) = serde_json::from_value::<Workflow>(content.clone()) {
        if workflow_kind_matches(object_type, &workflow) {
            if !fallback_name.is_empty() {
                workflow.set_name(fallback_name);
            }
            return Ok(workflow);
        }
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            "Drive workflow content does not match the requested object type",
        ));
    }
    match object_type {
        ControlDriveObjectType::Workflow => {
            let command = content.get("command").and_then(serde_json::Value::as_str);
            let command = command.ok_or_else(|| {
                ControlError::new(
                    ErrorCode::InvalidParams,
                    "drive.create/update workflow content requires a command string or typed workflow object",
                )
            })?;
            Ok(Workflow::new(fallback_name, command))
        }
        ControlDriveObjectType::Prompt => {
            let query = content.get("query").and_then(serde_json::Value::as_str);
            let query = query.ok_or_else(|| {
                ControlError::new(
                    ErrorCode::InvalidParams,
                    "drive.create/update prompt content requires a query string or typed workflow object",
                )
            })?;
            Ok(Workflow::AgentMode {
                name: fallback_name.to_owned(),
                query: query.to_owned(),
                description: None,
                arguments: Vec::new(),
            })
        }
        _ => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "workflow content is only valid for workflow and prompt Drive object types",
        )),
    }
}

fn workflow_kind_matches(object_type: ControlDriveObjectType, workflow: &Workflow) -> bool {
    match object_type {
        ControlDriveObjectType::Workflow => workflow.is_command_workflow(),
        ControlDriveObjectType::Prompt => workflow.is_agent_mode_workflow(),
        _ => false,
    }
}

fn notebook_from_drive_content(
    fallback_title: &str,
    content: serde_json::Value,
    existing: Option<CloudNotebookModel>,
) -> Result<CloudNotebookModel, ControlError> {
    if let Some(data) = content.as_str() {
        return Ok(CloudNotebookModel {
            title: non_empty_string(fallback_title)
                .or_else(|| existing.as_ref().map(|notebook| notebook.title.clone()))
                .unwrap_or_default(),
            data: data.to_owned(),
            ai_document_id: existing
                .as_ref()
                .and_then(|notebook| notebook.ai_document_id),
            conversation_id: existing.and_then(|notebook| notebook.conversation_id),
        });
    }
    let typed = serde_json::from_value::<NotebookDriveContent>(content).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            "drive.create/update notebook content requires a string or typed notebook object",
            err.to_string(),
        )
    })?;
    Ok(CloudNotebookModel {
        title: typed
            .title
            .or_else(|| non_empty_string(fallback_title))
            .or_else(|| existing.as_ref().map(|notebook| notebook.title.clone()))
            .unwrap_or_default(),
        data: typed
            .data
            .or_else(|| existing.as_ref().map(|notebook| notebook.data.clone()))
            .unwrap_or_default(),
        ai_document_id: existing
            .as_ref()
            .and_then(|notebook| notebook.ai_document_id),
        conversation_id: existing.and_then(|notebook| notebook.conversation_id),
    })
}

fn env_vars_from_drive_content(
    fallback_title: &str,
    content: serde_json::Value,
) -> Result<EnvVarCollection, ControlError> {
    let mut env_vars = serde_json::from_value::<EnvVarCollection>(content).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            "drive.create/update environment content requires a typed environment-variable collection",
            err.to_string(),
        )
    })?;
    if env_vars.title.as_ref().is_none_or(String::is_empty) {
        env_vars.title = non_empty_string(fallback_title);
    }
    Ok(env_vars)
}

fn non_empty_string(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

pub(crate) fn validate_drive_request_id(id: &str, action: ActionKind) -> Result<(), ControlError> {
    if id.is_empty() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{} requires a non-empty Drive object id", action.as_str()),
        ));
    }
    Ok(())
}

pub(crate) fn validate_drive_target_matches_params(
    target: &::local_control::protocol::TargetSelector,
    object_type: ControlDriveObjectType,
    id: &str,
    action: ActionKind,
) -> Result<(), ControlError> {
    if let Some(DriveTarget::Id {
        object_type: target_type,
        id: target_id,
    }) = target.drive.as_ref()
    {
        if *target_type != object_type || target_id.0 != id {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                format!(
                    "{} target selector does not match the requested Drive object",
                    action.as_str()
                ),
            ));
        }
    }
    Ok(())
}

pub(crate) fn drive_object_for_mutation<'a>(
    cloud_model: &'a CloudModel,
    object_type: ControlDriveObjectType,
    id: &str,
    action: ActionKind,
) -> Result<&'a dyn CloudObject, ControlError> {
    let object = cloud_model.get_by_uid(&id.to_owned()).ok_or_else(|| {
        ControlError::new(
            ErrorCode::StaleTarget,
            format!(
                "{} could not resolve the requested Drive object id",
                action.as_str()
            ),
        )
    })?;
    let summary = drive_object_summary(object).ok_or_else(|| {
        ControlError::new(
            ErrorCode::UnsupportedAction,
            format!(
                "{} does not support this Drive object type",
                action.as_str()
            ),
        )
    })?;
    if summary.object_type != object_type {
        return Err(ControlError::new(
            ErrorCode::TargetStateConflict,
            format!(
                "{} Drive object type does not match the requested type",
                action.as_str()
            ),
        ));
    }
    Ok(object)
}

pub(crate) fn drive_mutation_result(
    object: &dyn CloudObject,
    object_type: ControlDriveObjectType,
) -> Result<serde_json::Value, ControlError> {
    let summary = drive_object_summary(object).ok_or_else(|| {
        ControlError::new(
            ErrorCode::UnsupportedAction,
            "Drive mutation does not support this Drive object type",
        )
    })?;
    if summary.object_type != object_type {
        return Err(ControlError::new(
            ErrorCode::TargetStateConflict,
            "Drive object type does not match the requested type",
        ));
    }
    to_drive_data(DriveMutationResult {
        object: summary,
        execution_id: None,
    })
}

fn ensure_drive_execution_policy_approved(action: ActionKind) -> Result<(), ControlError> {
    Err(ControlError::new(
        ErrorCode::ExecutionContextNotAllowed,
        format!(
            "{} requires an explicit approval policy hook, but no approval is available",
            action.as_str()
        ),
    ))
}

pub(crate) fn drive_object_summary(object: &dyn CloudObject) -> Option<DriveObjectSummary> {
    Some(DriveObjectSummary {
        object_type: control_drive_object_type(object)?,
        id: object.uid(),
        name: object.display_name(),
    })
}

pub(crate) fn validate_drive_target(
    target: &::local_control::protocol::TargetSelector,
    action: ActionKind,
) -> Result<(), ControlError> {
    if target.window.is_some()
        || target.tab.is_some()
        || target.pane.is_some()
        || target.session.is_some()
        || target.file.is_some()
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} does not accept window, tab, pane, session, or file selectors",
                action.as_str()
            ),
        ));
    }
    match (action, target.drive.as_ref()) {
        (
            ActionKind::DriveCreate
            | ActionKind::DriveUpdate
            | ActionKind::DriveDelete
            | ActionKind::DriveRun
            | ActionKind::DriveInsert,
            None,
        ) => Ok(()),
        (
            ActionKind::DriveUpdate
            | ActionKind::DriveDelete
            | ActionKind::DriveRun
            | ActionKind::DriveInsert,
            Some(DriveTarget::Id { id, .. }),
        ) => {
            if id.0.is_empty() {
                return Err(ControlError::new(
                    ErrorCode::InvalidSelector,
                    format!(
                        "{} requires a non-empty Drive object id selector",
                        action.as_str()
                    ),
                ));
            }
            Ok(())
        }
        (_, Some(DriveTarget::Name { .. })) => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!("{} does not support Drive name selectors", action.as_str()),
        )),
        (_, Some(DriveTarget::Id { .. })) => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!(
                "{} does not support Drive id selectors in this context",
                action.as_str()
            ),
        )),
        (_, None) => Ok(()),
    }
}

fn control_drive_object_type(object: &dyn CloudObject) -> Option<ControlDriveObjectType> {
    match object.object_type() {
        ObjectType::Workflow => {
            let workflow = object.as_any().downcast_ref::<CloudWorkflow>()?;
            if workflow.model().data.is_agent_mode_workflow() {
                Some(ControlDriveObjectType::Prompt)
            } else {
                Some(ControlDriveObjectType::Workflow)
            }
        }
        ObjectType::Notebook => Some(ControlDriveObjectType::Notebook),
        ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
            JsonObjectType::EnvVarCollection,
        )) => Some(ControlDriveObjectType::Environment),
        _ => None,
    }
}

fn to_drive_data<T: serde::Serialize>(data: T) -> Result<serde_json::Value, ControlError> {
    serde_json::to_value(data).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to encode local-control Drive response",
            err.to_string(),
        )
    })
}
