use ::local_control::protocol::{
    DriveInspectParams, DriveInspectResult, DriveListParams, DriveListResult, DriveMutationAudit,
    DriveMutationResult, DriveObjectCreateParams, DriveObjectId, DriveObjectInsertParams,
    DriveObjectSummary, DriveObjectType as ControlDriveObjectType, DriveObjectUpdateParams,
    TargetSelector,
};
use ::local_control::{ActionKind, ControlError, ErrorCode, RequestEnvelope};
use serde_json::json;
use warpui::{ModelContext, SingletonEntity};

use crate::auth::AuthStateProvider;
use crate::cloud_object::model::generic_string_model::GenericStringObjectId;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{
    CloudObject, GenericStringObjectFormat, JsonObjectType, ObjectType, Owner, Space,
};
use crate::drive::folders::{CloudFolder, CloudFolderModel, FolderId};
use crate::drive::CloudObjectTypeAndId;
use crate::env_vars::{CloudEnvVarCollection, CloudEnvVarCollectionModel, EnvVarCollection};
use crate::local_control::LocalControlBridge;
use crate::notebooks::{CloudNotebook, CloudNotebookModel, NotebookId};
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::{ClientId, SyncId};
use crate::workflows::workflow::Workflow;
use crate::workflows::{CloudWorkflow, CloudWorkflowModel, WorkflowId};
use crate::workspaces::user_workspaces::UserWorkspaces;

pub(crate) fn drive_list(
    target: &TargetSelector,
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_drive_target(target, ActionKind::DriveList)?;
    let params = action.params_as::<DriveListParams>()?;
    let mut objects = CloudModel::as_ref(ctx)
        .cloud_objects()
        .filter_map(|object| drive_object_summary(object.as_ref()))
        .filter(|summary| {
            params
                .object_type
                .is_none_or(|object_type| summary.object_type == object_type)
        })
        .collect::<Vec<_>>();
    objects.sort_by(|left, right| {
        drive_object_type_rank(left.object_type)
            .cmp(&drive_object_type_rank(right.object_type))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.0.cmp(&right.id.0))
    });
    to_drive_data(DriveListResult { objects })
}

pub(crate) fn drive_inspect(
    target: &TargetSelector,
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_drive_target(target, ActionKind::DriveInspect)?;
    let params = action.params_as::<DriveInspectParams>()?;
    let object = CloudModel::as_ref(ctx)
        .get_by_uid(&params.id.0)
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::StaleTarget,
                "drive.inspect could not resolve the requested Drive object id",
            )
        })?;
    drive_object_get_result(object).and_then(to_drive_data)
}

pub(crate) fn validate_drive_target(
    target: &TargetSelector,
    action: ActionKind,
) -> Result<(), ControlError> {
    if target.window.is_some()
        || target.tab.is_some()
        || target.pane.is_some()
        || target.session.is_some()
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} does not accept window, tab, pane, or session selectors",
                action.as_str()
            ),
        ));
    }
    Ok(())
}

fn drive_object_get_result(object: &dyn CloudObject) -> Result<DriveInspectResult, ControlError> {
    let summary = drive_object_summary(object).ok_or_else(|| {
        ControlError::new(
            ErrorCode::UnsupportedAction,
            "drive.inspect does not support this Drive object type",
        )
    })?;
    Ok(DriveInspectResult {
        object: summary,
        content: drive_object_content(object)?,
    })
}
pub(crate) fn create_drive_object(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let params = request.action.params_as::<DriveObjectCreateParams>()?;
    validate_no_content_file(params.content_file.as_ref(), request.action.kind)?;
    validate_supported_object_type(params.object_type, request.action.kind)?;
    let name = required_name(params.name.as_deref(), &params.content, request.action.kind)?;
    let subject = authenticated_user_subject(ctx)?;
    let owner = authenticated_user_owner(ctx)?;
    let content = content_value(params.content.as_deref());
    let client_id = ClientId::new();
    let sync_id = SyncId::ClientId(client_id);
    CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| match params.object_type {
        ControlDriveObjectType::Workflow | ControlDriveObjectType::Prompt => {
            let workflow = workflow_from_drive_content(params.object_type, &name, content)?;
            cloud_model.create_object(
                sync_id,
                CloudWorkflow::new_local(CloudWorkflowModel::new(workflow), owner, None, client_id),
                ctx,
            );
            Ok(())
        }
        ControlDriveObjectType::Notebook => {
            let notebook = notebook_from_drive_content(&name, content, None)?;
            cloud_model.create_object(
                sync_id,
                CloudNotebook::new_local(notebook, owner, None, client_id),
                ctx,
            );
            Ok(())
        }
        ControlDriveObjectType::EnvVarCollection => {
            let env_vars = env_vars_from_drive_content(&name, content)?;
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
        ControlDriveObjectType::Folder => {
            cloud_model.create_object(
                sync_id,
                CloudFolder::new_local(CloudFolderModel::new(&name, false), owner, None, client_id),
                ctx,
            );
            Ok(())
        }
        _ => Err(unsupported_object_type(
            params.object_type,
            request.action.kind,
        )),
    })?;
    let cloud_model = CloudModel::as_ref(ctx);
    let object = cloud_model.get_by_uid(&sync_id.uid()).ok_or_else(|| {
        ControlError::new(
            ErrorCode::Internal,
            "drive.object.create could not resolve the created Drive object",
        )
    })?;
    drive_mutation_result(object, params.object_type, request.action.kind, subject)
}

pub(crate) fn update_drive_object(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let params = request.action.params_as::<DriveObjectUpdateParams>()?;
    validate_drive_request_id(&params.id, request.action.kind)?;
    validate_no_content_file(params.content_file.as_ref(), request.action.kind)?;
    let subject = authenticated_user_subject(ctx)?;
    let content = required_content(params.content.as_deref(), request.action.kind)?;
    let (sync_id, object_type, existing_notebook) = {
        let cloud_model = CloudModel::as_ref(ctx);
        let object = drive_object_for_mutation(cloud_model, &params.id, request.action.kind)?;
        (
            object.sync_id(),
            control_drive_object_type(object).ok_or_else(|| {
                ControlError::new(
                    ErrorCode::UnsupportedAction,
                    "drive.object.update does not support this Drive object type",
                )
            })?,
            object
                .as_any()
                .downcast_ref::<CloudNotebook>()
                .map(|notebook| notebook.model().clone()),
        )
    };
    CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| match object_type {
        ControlDriveObjectType::Workflow | ControlDriveObjectType::Prompt => {
            let workflow = workflow_from_drive_content(object_type, "", content.clone())?;
            cloud_model.update_object_from_edit::<WorkflowId, CloudWorkflowModel>(
                CloudWorkflowModel::new(workflow),
                sync_id,
                ctx,
            );
            Ok(())
        }
        ControlDriveObjectType::Notebook => {
            let notebook = notebook_from_drive_content("", content.clone(), existing_notebook)?;
            cloud_model
                .update_object_from_edit::<NotebookId, CloudNotebookModel>(notebook, sync_id, ctx);
            Ok(())
        }
        ControlDriveObjectType::EnvVarCollection => {
            let env_vars = env_vars_from_drive_content("", content.clone())?;
            cloud_model
                .update_object_from_edit::<GenericStringObjectId, CloudEnvVarCollectionModel>(
                    CloudEnvVarCollectionModel::new(env_vars),
                    sync_id,
                    ctx,
                );
            Ok(())
        }
        ControlDriveObjectType::Folder => {
            let name = folder_name_from_content(&content, request.action.kind)?;
            cloud_model.update_object_from_edit::<FolderId, CloudFolderModel>(
                CloudFolderModel::new(&name, false),
                sync_id,
                ctx,
            );
            Ok(())
        }
        _ => Err(unsupported_object_type(object_type, request.action.kind)),
    })?;
    let cloud_model = CloudModel::as_ref(ctx);
    let object = drive_object_for_mutation(cloud_model, &params.id, request.action.kind)?;
    drive_mutation_result(object, object_type, request.action.kind, subject)
}

pub(crate) fn delete_drive_object(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let id = drive_object_id_params(request)?;
    validate_drive_request_id(&id, request.action.kind)?;
    let subject = authenticated_user_subject(ctx)?;
    let (sync_id, summary) = {
        let cloud_model = CloudModel::as_ref(ctx);
        let object = drive_object_for_mutation(cloud_model, &id, request.action.kind)?;
        let summary = drive_object_summary(object).ok_or_else(|| {
            ControlError::new(
                ErrorCode::UnsupportedAction,
                "drive.object.delete does not support this Drive object type",
            )
        })?;
        (object.sync_id(), summary)
    };
    CloudModel::handle(ctx).update(ctx, |cloud_model, ctx| {
        cloud_model.delete_object(sync_id, ctx);
    });
    to_drive_data(DriveMutationResult {
        object: summary,
        audit: Some(audit(request.action.kind, subject)),
    })
}

pub(crate) fn insert_drive_object(
    request: &RequestEnvelope,
) -> Result<serde_json::Value, ControlError> {
    let params = request.action.params_as::<DriveObjectInsertParams>()?;
    validate_drive_request_id(&params.id, request.action.kind)?;
    Err(ControlError::new(
        ErrorCode::ExecutionContextNotAllowed,
        "drive.object.insert requires an insertion target policy hook that is not available in this shard",
    ))
}

pub(crate) fn share_drive_object_to_team(
    request: &RequestEnvelope,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    let id = drive_object_id_params(request)?;
    validate_drive_request_id(&id, request.action.kind)?;
    let subject = authenticated_user_subject(ctx)?;
    let team_uid = UserWorkspaces::as_ref(ctx)
        .current_team_uid()
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                "drive.object.share_to_team requires a current Warp team",
            )
        })?;
    let (type_and_id, summary) = {
        let cloud_model = CloudModel::as_ref(ctx);
        let object = drive_object_for_mutation(cloud_model, &id, request.action.kind)?;
        if !matches!(object.permissions().owner, Owner::User { .. }) {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                "drive.object.share_to_team only supports personal Drive objects",
            ));
        }
        let Some(server_id) = object.sync_id().into_server() else {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                "drive.object.share_to_team requires a server-backed Drive object",
            ));
        };
        let summary = drive_object_summary(object).ok_or_else(|| {
            ControlError::new(
                ErrorCode::UnsupportedAction,
                "drive.object.share_to_team does not support this Drive object type",
            )
        })?;
        (
            CloudObjectTypeAndId::from_id_and_type(
                SyncId::ServerId(server_id),
                object.object_type(),
            ),
            summary,
        )
    };
    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
        update_manager.move_object_to_location(
            type_and_id,
            crate::cloud_object::CloudObjectLocation::Space(Space::Team { team_uid }),
            ctx,
        );
    });
    to_drive_data(DriveMutationResult {
        object: summary,
        audit: Some(audit(request.action.kind, subject)),
    })
}

fn drive_object_id_params(request: &RequestEnvelope) -> Result<DriveObjectId, ControlError> {
    #[derive(serde::Deserialize)]
    struct Params {
        id: DriveObjectId,
    }

    request.action.params_as::<Params>().map(|params| params.id)
}

fn authenticated_user_subject(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<String, ControlError> {
    let auth_state = AuthStateProvider::as_ref(ctx).get();
    if auth_state.is_anonymous_or_logged_out() {
        return Err(ControlError::new(
            ErrorCode::AuthenticatedUserUnavailable,
            "this action requires a logged-in Warp user",
        ));
    }
    auth_state
        .user_id()
        .map(|uid| uid.as_string())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::AuthenticatedUserUnavailable,
                "this action requires a logged-in Warp user",
            )
        })
}

fn authenticated_user_owner(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<Owner, ControlError> {
    let auth_state = AuthStateProvider::as_ref(ctx).get();
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
            let command = content
                .as_str()
                .or_else(|| content.get("command").and_then(serde_json::Value::as_str))
                .ok_or_else(|| {
                    ControlError::new(
                        ErrorCode::InvalidParams,
                        "drive.object.create/update workflow content requires a command string or typed workflow object",
                    )
                })?;
            Ok(Workflow::new(fallback_name, command))
        }
        ControlDriveObjectType::Prompt => {
            let query = content
                .as_str()
                .or_else(|| content.get("query").and_then(serde_json::Value::as_str))
                .ok_or_else(|| {
                    ControlError::new(
                        ErrorCode::InvalidParams,
                        "drive.object.create/update prompt content requires a query string or typed workflow object",
                    )
                })?;
            Ok(Workflow::AgentMode {
                name: fallback_name.to_owned(),
                query: query.to_owned(),
                description: None,
                arguments: Vec::new(),
            })
        }
        _ => Err(unsupported_object_type(
            object_type,
            ActionKind::DriveObjectCreate,
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
    if content.is_null() {
        return Ok(CloudNotebookModel {
            title: non_empty_string(fallback_title)
                .or_else(|| existing.as_ref().map(|notebook| notebook.title.clone()))
                .unwrap_or_default(),
            data: existing
                .as_ref()
                .map(|notebook| notebook.data.clone())
                .unwrap_or_default(),
            ai_document_id: existing
                .as_ref()
                .and_then(|notebook| notebook.ai_document_id),
            conversation_id: existing.and_then(|notebook| notebook.conversation_id),
        });
    }
    let typed = serde_json::from_value::<NotebookDriveContent>(content).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            "drive.object.create/update notebook content requires a string or typed notebook object",
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
    let mut env_vars = if content.is_null() {
        EnvVarCollection::default()
    } else {
        serde_json::from_value::<EnvVarCollection>(content).map_err(|err| {
            ControlError::with_details(
                ErrorCode::InvalidParams,
                "drive.object.create/update env-var-collection content requires a typed environment-variable collection",
                err.to_string(),
            )
        })?
    };
    if env_vars.title.as_ref().is_none_or(String::is_empty) {
        env_vars.title = non_empty_string(fallback_title);
    }
    Ok(env_vars)
}

fn content_value(content: Option<&str>) -> serde_json::Value {
    content
        .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok())
        .or_else(|| content.map(|content| serde_json::Value::String(content.to_owned())))
        .unwrap_or(serde_json::Value::Null)
}

fn required_content(
    content: Option<&str>,
    action: ActionKind,
) -> Result<serde_json::Value, ControlError> {
    let Some(content) = content else {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{} requires inline Drive object content", action.as_str()),
        ));
    };
    Ok(content_value(Some(content)))
}

fn non_empty_string(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn required_name(
    name: Option<&str>,
    content: &Option<String>,
    action: ActionKind,
) -> Result<String, ControlError> {
    let name = name
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .or_else(|| object_name_from_content(content.as_deref()));
    name.ok_or_else(|| {
        ControlError::new(
            ErrorCode::InvalidParams,
            format!("{} requires a non-empty Drive object name", action.as_str()),
        )
    })
}

fn object_name_from_content(content: Option<&str>) -> Option<String> {
    let value =
        content.and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok())?;
    value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .or_else(|| value.get("title").and_then(serde_json::Value::as_str))
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn folder_name_from_content(
    content: &serde_json::Value,
    action: ActionKind,
) -> Result<String, ControlError> {
    content
        .as_str()
        .or_else(|| content.get("name").and_then(serde_json::Value::as_str))
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::InvalidParams,
                format!(
                    "{} folder content requires a non-empty name",
                    action.as_str()
                ),
            )
        })
}

fn validate_no_content_file(
    content_file: Option<&String>,
    action: ActionKind,
) -> Result<(), ControlError> {
    if content_file.is_some() {
        return Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!(
                "{} does not support local file content inputs",
                action.as_str()
            ),
        ));
    }
    Ok(())
}

fn validate_drive_request_id(id: &DriveObjectId, action: ActionKind) -> Result<(), ControlError> {
    if id.0.is_empty() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{} requires a non-empty Drive object id", action.as_str()),
        ));
    }
    Ok(())
}

fn validate_supported_object_type(
    object_type: ControlDriveObjectType,
    action: ActionKind,
) -> Result<(), ControlError> {
    match object_type {
        ControlDriveObjectType::Workflow
        | ControlDriveObjectType::Notebook
        | ControlDriveObjectType::EnvVarCollection
        | ControlDriveObjectType::Prompt
        | ControlDriveObjectType::Folder => Ok(()),
        _ => Err(unsupported_object_type(object_type, action)),
    }
}

fn unsupported_object_type(
    object_type: ControlDriveObjectType,
    action: ActionKind,
) -> ControlError {
    ControlError::new(
        ErrorCode::UnsupportedAction,
        format!(
            "{} does not support Drive object type {:?}",
            action.as_str(),
            object_type
        ),
    )
}

fn drive_object_for_mutation<'a>(
    cloud_model: &'a CloudModel,
    id: &DriveObjectId,
    action: ActionKind,
) -> Result<&'a dyn CloudObject, ControlError> {
    let object = cloud_model.get_by_uid(&id.0).ok_or_else(|| {
        ControlError::new(
            ErrorCode::StaleTarget,
            format!(
                "{} could not resolve the requested Drive object id",
                action.as_str()
            ),
        )
    })?;
    validate_supported_object_type(
        control_drive_object_type(object).ok_or_else(|| {
            ControlError::new(
                ErrorCode::UnsupportedAction,
                format!(
                    "{} does not support this Drive object type",
                    action.as_str()
                ),
            )
        })?,
        action,
    )?;
    Ok(object)
}

fn drive_mutation_result(
    object: &dyn CloudObject,
    object_type: ControlDriveObjectType,
    action: ActionKind,
    subject: String,
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
        audit: Some(audit(action, subject)),
    })
}

fn drive_object_summary(object: &dyn CloudObject) -> Option<DriveObjectSummary> {
    Some(DriveObjectSummary {
        object_type: control_drive_object_type(object)?,
        id: DriveObjectId(object.uid()),
        name: object.display_name(),
    })
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
        ObjectType::Folder => Some(ControlDriveObjectType::Folder),
        ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
            JsonObjectType::EnvVarCollection,
        )) => Some(ControlDriveObjectType::EnvVarCollection),
        ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
            JsonObjectType::AIFact,
        )) => Some(ControlDriveObjectType::AiFact),
        ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
            JsonObjectType::MCPServer | JsonObjectType::TemplatableMCPServer,
        )) => Some(ControlDriveObjectType::McpServer),
        _ => None,
    }
}

fn drive_object_content(object: &dyn CloudObject) -> Result<serde_json::Value, ControlError> {
    match control_drive_object_type(object).ok_or_else(drive_unsupported_type_error)? {
        ControlDriveObjectType::Workflow | ControlDriveObjectType::Prompt => object
            .as_any()
            .downcast_ref::<CloudWorkflow>()
            .ok_or_else(drive_type_mismatch_error)
            .and_then(|workflow| {
                serde_json::to_value(&workflow.model().data).map_err(json_response_error)
            }),
        ControlDriveObjectType::Notebook => {
            let notebook = object
                .as_any()
                .downcast_ref::<CloudNotebook>()
                .ok_or_else(drive_type_mismatch_error)?;
            Ok(json!({
                "title": notebook.model().title.clone(),
                "data": notebook.model().data.clone(),
                "ai_document_id": notebook.model().ai_document_id.as_ref().map(|id| id.to_string()),
                "conversation_id": notebook.model().conversation_id.clone(),
            }))
        }
        ControlDriveObjectType::EnvVarCollection => object
            .as_any()
            .downcast_ref::<CloudEnvVarCollection>()
            .ok_or_else(drive_type_mismatch_error)
            .and_then(|env_var_collection| {
                serde_json::to_value(&env_var_collection.model().string_model)
                    .map_err(json_response_error)
            }),
        ControlDriveObjectType::Folder => {
            let folder = object
                .as_any()
                .downcast_ref::<CloudFolder>()
                .ok_or_else(drive_type_mismatch_error)?;
            Ok(json!({
                "name": folder.model().name.clone(),
                "is_open": folder.model().is_open,
                "is_warp_pack": folder.model().is_warp_pack,
            }))
        }
        ControlDriveObjectType::AiFact
        | ControlDriveObjectType::AiRule
        | ControlDriveObjectType::McpServer
        | ControlDriveObjectType::McpServerCollection
        | ControlDriveObjectType::Space
        | ControlDriveObjectType::Trash => Err(drive_unsupported_type_error()),
    }
}

fn drive_object_type_rank(object_type: ControlDriveObjectType) -> u8 {
    match object_type {
        ControlDriveObjectType::Workflow => 0,
        ControlDriveObjectType::Prompt => 1,
        ControlDriveObjectType::Notebook => 2,
        ControlDriveObjectType::EnvVarCollection => 3,
        ControlDriveObjectType::Folder => 4,
        ControlDriveObjectType::AiFact => 5,
        ControlDriveObjectType::AiRule => 6,
        ControlDriveObjectType::McpServer => 7,
        ControlDriveObjectType::McpServerCollection => 8,
        ControlDriveObjectType::Space => 9,
        ControlDriveObjectType::Trash => 10,
    }
}

fn drive_type_mismatch_error() -> ControlError {
    ControlError::new(
        ErrorCode::TargetStateConflict,
        "drive.inspect Drive object type does not match the resolved object",
    )
}

fn drive_unsupported_type_error() -> ControlError {
    ControlError::new(
        ErrorCode::UnsupportedAction,
        "drive.inspect content reads are not supported for this Drive object type",
    )
}

fn json_response_error(error: serde_json::Error) -> ControlError {
    ControlError::with_details(
        ErrorCode::Internal,
        "failed to encode local-control Drive response",
        error.to_string(),
    )
}

fn audit(action: ActionKind, authenticated_user_subject: String) -> DriveMutationAudit {
    DriveMutationAudit {
        action: action.as_str().to_owned(),
        authenticated_user_subject,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_create_rejects_local_content_file_inputs() {
        let err = validate_no_content_file(
            Some(&"/tmp/notebook.md".to_owned()),
            ActionKind::DriveObjectCreate,
        )
        .expect_err("content_file is out of scope");
        assert_eq!(err.code, ErrorCode::UnsupportedAction);
    }

    #[test]
    fn drive_create_requires_supported_object_type() {
        let err = validate_supported_object_type(
            ControlDriveObjectType::McpServer,
            ActionKind::DriveObjectCreate,
        )
        .expect_err("MCP server CRUD is out of scope");
        assert_eq!(err.code, ErrorCode::UnsupportedAction);
    }

    #[test]
    fn drive_create_accepts_folder_object_type() {
        validate_supported_object_type(
            ControlDriveObjectType::Folder,
            ActionKind::DriveObjectCreate,
        )
        .expect("folder objects are supported by the catalog shard");
    }

    #[test]
    fn drive_mutations_require_non_empty_ids() {
        let err =
            validate_drive_request_id(&DriveObjectId(String::new()), ActionKind::DriveObjectUpdate)
                .expect_err("empty Drive object IDs are rejected");
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn drive_create_extracts_names_from_typed_content() {
        let name = required_name(
            None,
            &Some(r#"{"title":"Release notes","data":"ship"}"#.to_owned()),
            ActionKind::DriveObjectCreate,
        )
        .expect("title supplies Drive object name");
        assert_eq!(name, "Release notes");
    }
}
