use crate::auth::AuthStateProvider;
use ::local_control::protocol::{
    DriveInspectParams, DriveInspectResult, DriveListParams, DriveListResult, DriveMutationAudit,
    DriveMutationResult, DriveObjectCreateParams, DriveObjectId, DriveObjectInsertParams,
    DriveObjectSummary, DriveObjectTarget, DriveObjectType, DriveObjectUpdateParams,
    PermissionCategory, TargetSelector, WindowTarget,
};
use ::local_control::{ActionKind, ControlError, ErrorCode};
use serde_json::json;
use warpui::{ModelContext, SingletonEntity, TypedActionView, ViewHandle, WindowId};

use crate::cloud_object::{
    model::persistence::CloudModel, CloudObject, GenericStringObjectFormat, JsonObjectType,
    ObjectType, Owner,
};
use crate::drive::folders::{CloudFolder, CloudFolderModel, FolderId};
use crate::drive::items::WarpDriveItemId;
use crate::drive::CloudObjectTypeAndId;
use crate::env_vars::manager::EnvVarCollectionSource;
use crate::env_vars::{CloudEnvVarCollection, CloudEnvVarCollectionModel, EnvVarCollection};
use crate::local_control::resolver::require_active_window_id_for_action;
use crate::local_control::LocalControlBridge;
use crate::notebooks::{CloudNotebook, CloudNotebookModel, NotebookId};
use crate::server::cloud_objects::update_manager::{InitiatedBy, UpdateManager};
use crate::server::ids::{ClientId, SyncId};
use crate::server::telemetry::SharingDialogSource;
use crate::workflows::workflow::Workflow;
use crate::workflows::CloudWorkflow;
use crate::workspace::{Workspace, WorkspaceAction};
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
            .then_with(|| left.id.cmp(&right.id))
    });
    serde_json::to_value(DriveListResult { objects }).map_err(json_response_error)
}

pub(crate) fn drive_inspect(
    target: &TargetSelector,
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_drive_target(target, ActionKind::DriveInspect)?;
    let params = action.params_as::<DriveInspectParams>()?;
    let object = CloudModel::as_ref(ctx)
        .get_by_uid(&params.id)
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::StaleTarget,
                "drive.inspect could not resolve the requested Drive object id",
            )
        })?;
    drive_object_get_result(object)
        .and_then(|result| serde_json::to_value(result).map_err(json_response_error))
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

/// Validates that an open-style Drive action only targets the active window or
/// an opaque window id. Tab/pane/session selectors are rejected because Drive
/// open actions operate on app-wide Warp Drive state, not pane state.
fn validate_drive_open_target(
    target: &TargetSelector,
    action: ActionKind,
) -> Result<(), ControlError> {
    if target.tab.is_some() || target.pane.is_some() || target.session.is_some() {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} does not accept tab, pane, or session selectors",
                action.as_str()
            ),
        ));
    }
    if matches!(
        target.window.as_ref(),
        Some(WindowTarget::Index { .. } | WindowTarget::Title { .. })
    ) {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} only supports active and opaque window id selectors",
                action.as_str()
            ),
        ));
    }
    Ok(())
}

pub(crate) fn drive_open(
    target: &TargetSelector,
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_drive_open_target(target, ActionKind::DriveOpen)?;
    let params = action.params_as::<DriveInspectParams>()?;
    let object_type_and_id =
        resolve_cloud_object_type_and_id(&params.id, ActionKind::DriveOpen, ctx)?;
    let window_id = select_window_for_drive_open(ActionKind::DriveOpen, target, ctx)?;
    let workspace = workspace_for_window(ActionKind::DriveOpen, window_id, ctx)?;
    workspace.update(ctx, |workspace, view_ctx| {
        workspace.handle_action(
            &WorkspaceAction::ViewObjectInWarpDrive(WarpDriveItemId::Object(object_type_and_id)),
            view_ctx,
        );
    });
    ctx.windows().show_window_and_focus_app(window_id);
    Ok(json!({
        "action": ActionKind::DriveOpen.as_str(),
        "opened": true,
        "id": params.id,
        "object_type": cloud_object_type_label(object_type_and_id),
        "window_id": window_id.to_string(),
    }))
}

pub(crate) fn drive_notebook_open(
    target: &TargetSelector,
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_drive_open_target(target, ActionKind::DriveNotebookOpen)?;
    let params = action.params_as::<DriveInspectParams>()?;
    let sync_id = resolve_typed_drive_sync_id(
        &params.id,
        ObjectType::Notebook,
        ActionKind::DriveNotebookOpen,
        ctx,
    )?;
    let window_id = select_window_for_drive_open(ActionKind::DriveNotebookOpen, target, ctx)?;
    let workspace = workspace_for_window(ActionKind::DriveNotebookOpen, window_id, ctx)?;
    workspace.update(ctx, |workspace, view_ctx| {
        workspace.handle_action(&WorkspaceAction::OpenNotebook { id: sync_id }, view_ctx);
    });
    ctx.windows().show_window_and_focus_app(window_id);
    Ok(json!({
        "action": ActionKind::DriveNotebookOpen.as_str(),
        "opened": true,
        "id": params.id,
        "window_id": window_id.to_string(),
    }))
}

pub(crate) fn drive_env_var_collection_open(
    target: &TargetSelector,
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_drive_open_target(target, ActionKind::DriveEnvVarCollectionOpen)?;
    let params = action.params_as::<DriveInspectParams>()?;
    let env_var_collection_type = ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
        JsonObjectType::EnvVarCollection,
    ));
    let sync_id = resolve_typed_drive_sync_id(
        &params.id,
        env_var_collection_type,
        ActionKind::DriveEnvVarCollectionOpen,
        ctx,
    )?;
    let window_id =
        select_window_for_drive_open(ActionKind::DriveEnvVarCollectionOpen, target, ctx)?;
    let workspace = workspace_for_window(ActionKind::DriveEnvVarCollectionOpen, window_id, ctx)?;
    workspace.update(ctx, |workspace, view_ctx| {
        workspace.open_env_var_collection(
            &EnvVarCollectionSource::Existing(sync_id),
            false,
            view_ctx,
        );
    });
    ctx.windows().show_window_and_focus_app(window_id);
    Ok(json!({
        "action": ActionKind::DriveEnvVarCollectionOpen.as_str(),
        "opened": true,
        "id": params.id,
        "window_id": window_id.to_string(),
    }))
}

pub(crate) fn drive_object_share_open(
    target: &TargetSelector,
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_drive_open_target(target, ActionKind::DriveObjectShareOpen)?;
    let params = action.params_as::<DriveInspectParams>()?;
    let object_type_and_id =
        resolve_cloud_object_type_and_id(&params.id, ActionKind::DriveObjectShareOpen, ctx)?;
    let window_id = select_window_for_drive_open(ActionKind::DriveObjectShareOpen, target, ctx)?;
    let workspace = workspace_for_window(ActionKind::DriveObjectShareOpen, window_id, ctx)?;
    workspace.update(ctx, |workspace, view_ctx| {
        workspace.handle_action(
            &WorkspaceAction::OpenObjectSharingSettings {
                object_id: object_type_and_id,
                source: SharingDialogSource::DriveIndex,
            },
            view_ctx,
        );
    });
    ctx.windows().show_window_and_focus_app(window_id);
    Ok(json!({
        "action": ActionKind::DriveObjectShareOpen.as_str(),
        "opened": true,
        "id": params.id,
        "object_type": cloud_object_type_label(object_type_and_id),
        "window_id": window_id.to_string(),
    }))
}

pub(crate) fn drive_object_create(
    target: &TargetSelector,
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_drive_mutation_target(target, ActionKind::DriveObjectCreate)?;
    let params = action.params_as::<DriveObjectCreateParams>()?;
    let content = drive_content_value(
        params.content.as_deref(),
        params.content_file.as_deref(),
        ActionKind::DriveObjectCreate,
    )?;
    let name = object_name_from_content(params.object_type, content.as_ref());
    let subject = authenticated_user_subject(ctx)?;
    let owner = authenticated_user_owner(ctx)?;
    let client_id = ClientId::new();
    let sync_id = SyncId::ClientId(client_id);
    let object_type = params.object_type;
    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| match object_type {
        DriveObjectType::Workflow | DriveObjectType::Prompt => {
            let workflow =
                workflow_from_drive_content(object_type, &name, required_content(content)?)?;
            update_manager.create_workflow(
                workflow,
                owner,
                None,
                client_id,
                Default::default(),
                true,
                ctx,
            );
            Ok(())
        }
        DriveObjectType::Notebook => {
            let notebook = notebook_from_drive_content(&name, optional_content(content), None)?;
            update_manager.create_notebook(
                client_id,
                owner,
                None,
                notebook,
                Default::default(),
                true,
                ctx,
            );
            Ok(())
        }
        DriveObjectType::EnvVarCollection => {
            let env_vars = env_vars_from_drive_content(&name, required_content(content)?)?;
            update_manager.create_env_var_collection(
                client_id,
                owner,
                None,
                CloudEnvVarCollectionModel::new(env_vars),
                Default::default(),
                true,
                ctx,
            );
            Ok(())
        }
        DriveObjectType::Folder => {
            update_manager.create_folder(
                name,
                owner,
                client_id,
                None,
                true,
                InitiatedBy::User,
                ctx,
            );
            Ok(())
        }
        DriveObjectType::AiFact
        | DriveObjectType::AiRule
        | DriveObjectType::McpServer
        | DriveObjectType::McpServerCollection
        | DriveObjectType::Space
        | DriveObjectType::Trash => Err(unsupported_mutation_type_error(
            ActionKind::DriveObjectCreate,
            object_type,
        )),
    })?;
    let object = CloudModel::as_ref(ctx)
        .get_by_uid(&sync_id.uid())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::Internal,
                "drive.object.create could not resolve the created Drive object",
            )
        })?;
    drive_mutation_result(object, object_type, ActionKind::DriveObjectCreate, subject)
}

pub(crate) fn drive_object_update(
    target: &TargetSelector,
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_drive_mutation_target(target, ActionKind::DriveObjectUpdate)?;
    let params = action.params_as::<DriveObjectUpdateParams>()?;
    let content = drive_content_value(
        params.content.as_deref(),
        params.content_file.as_deref(),
        ActionKind::DriveObjectUpdate,
    )?;
    let id = params.id.0;
    validate_drive_request_id(&id, ActionKind::DriveObjectUpdate)?;
    validate_drive_target_matches_params(target, &id, ActionKind::DriveObjectUpdate)?;
    let subject = authenticated_user_subject(ctx)?;
    let (sync_id, object_type, existing_notebook) = {
        let object =
            drive_object_for_mutation(CloudModel::as_ref(ctx), &id, ActionKind::DriveObjectUpdate)?;
        (
            object.sync_id(),
            control_drive_object_type(object).ok_or_else(drive_unsupported_type_error)?,
            object
                .as_any()
                .downcast_ref::<CloudNotebook>()
                .map(|notebook| notebook.model().clone()),
        )
    };
    let content = required_content(content)?;
    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| match object_type {
        DriveObjectType::Workflow | DriveObjectType::Prompt => {
            let workflow = workflow_from_drive_content(object_type, "", content)?;
            update_manager.update_workflow(workflow, sync_id, None, ctx);
            Ok(())
        }
        DriveObjectType::Notebook => {
            let notebook = notebook_from_drive_content("", content, existing_notebook)?;
            update_manager
                .update_object::<NotebookId, CloudNotebookModel>(notebook, sync_id, None, ctx);
            Ok(())
        }
        DriveObjectType::EnvVarCollection => {
            let env_vars = env_vars_from_drive_content("", content)?;
            update_manager.update_env_var_collection(env_vars, sync_id, None, ctx);
            Ok(())
        }
        DriveObjectType::Folder => {
            let name = folder_name_from_content(&content)?;
            update_manager.update_object::<FolderId, CloudFolderModel>(
                CloudFolderModel::new(&name, false),
                sync_id,
                None,
                ctx,
            );
            Ok(())
        }
        DriveObjectType::AiFact
        | DriveObjectType::AiRule
        | DriveObjectType::McpServer
        | DriveObjectType::McpServerCollection
        | DriveObjectType::Space
        | DriveObjectType::Trash => Err(unsupported_mutation_type_error(
            ActionKind::DriveObjectUpdate,
            object_type,
        )),
    })?;
    let object =
        drive_object_for_mutation(CloudModel::as_ref(ctx), &id, ActionKind::DriveObjectUpdate)?;
    drive_mutation_result(object, object_type, ActionKind::DriveObjectUpdate, subject)
}

pub(crate) fn drive_object_delete(
    target: &TargetSelector,
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_drive_mutation_target(target, ActionKind::DriveObjectDelete)?;
    let params = action.params_as::<DriveObjectIdParams>()?;
    let id = params.id.0;
    validate_drive_request_id(&id, ActionKind::DriveObjectDelete)?;
    validate_drive_target_matches_params(target, &id, ActionKind::DriveObjectDelete)?;
    let subject = authenticated_user_subject(ctx)?;
    let (type_and_id, summary) = {
        let object =
            drive_object_for_mutation(CloudModel::as_ref(ctx), &id, ActionKind::DriveObjectDelete)?;
        let summary = drive_object_summary(object).ok_or_else(drive_unsupported_type_error)?;
        (object.cloud_object_type_and_id(), summary)
    };
    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
        update_manager.delete_object_with_initiated_by(type_and_id, InitiatedBy::User, ctx);
    });
    to_drive_data(DriveMutationResult {
        object: summary,
        audit: Some(audit(ActionKind::DriveObjectDelete, subject)),
    })
}

pub(crate) fn drive_object_insert(
    target: &TargetSelector,
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_drive_mutation_target(target, ActionKind::DriveObjectInsert)?;
    let params = action.params_as::<DriveObjectInsertParams>()?;
    let id = params.id.0;
    validate_drive_request_id(&id, ActionKind::DriveObjectInsert)?;
    validate_drive_target_matches_params(target, &id, ActionKind::DriveObjectInsert)?;
    let subject = authenticated_user_subject(ctx)?;
    let sync_id = {
        let object =
            drive_object_for_mutation(CloudModel::as_ref(ctx), &id, ActionKind::DriveObjectInsert)?;
        let summary = drive_object_summary(object).ok_or_else(drive_unsupported_type_error)?;
        if summary.object_type != DriveObjectType::Notebook {
            return Err(ControlError::new(
                ErrorCode::UnsupportedAction,
                "drive.object.insert currently supports notebook objects only",
            ));
        }
        object.sync_id()
    };
    let insertion_target = params.target.unwrap_or_else(|| target.clone());
    validate_drive_insert_target(&insertion_target)?;
    let window_id =
        select_window_for_drive_open(ActionKind::DriveObjectInsert, &insertion_target, ctx)?;
    let workspace = workspace_for_window(ActionKind::DriveObjectInsert, window_id, ctx)?;
    workspace.update(ctx, |workspace, view_ctx| {
        workspace.handle_action(&WorkspaceAction::OpenNotebook { id: sync_id }, view_ctx);
    });
    ctx.windows().show_window_and_focus_app(window_id);
    let object =
        drive_object_for_mutation(CloudModel::as_ref(ctx), &id, ActionKind::DriveObjectInsert)?;
    drive_mutation_result(
        object,
        DriveObjectType::Notebook,
        ActionKind::DriveObjectInsert,
        subject,
    )
}

pub(crate) fn drive_object_share_to_team(
    target: &TargetSelector,
    action: &::local_control::Action,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<serde_json::Value, ControlError> {
    validate_drive_mutation_target(target, ActionKind::DriveObjectShareToTeam)?;
    let params = action.params_as::<DriveObjectIdParams>()?;
    let id = params.id.0;
    validate_drive_request_id(&id, ActionKind::DriveObjectShareToTeam)?;
    validate_drive_target_matches_params(target, &id, ActionKind::DriveObjectShareToTeam)?;
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
        let object = drive_object_for_mutation(
            CloudModel::as_ref(ctx),
            &id,
            ActionKind::DriveObjectShareToTeam,
        )?;
        if !matches!(object.permissions().owner, Owner::User { .. }) {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                "drive.object.share_to_team only supports personal Drive objects",
            ));
        }
        if object.sync_id().into_server().is_none() {
            return Err(ControlError::new(
                ErrorCode::TargetStateConflict,
                "drive.object.share_to_team requires a server-backed Drive object",
            ));
        }
        let summary = drive_object_summary(object).ok_or_else(drive_unsupported_type_error)?;
        (object.cloud_object_type_and_id(), summary)
    };
    UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
        update_manager.move_object_to_location(
            type_and_id,
            crate::cloud_object::CloudObjectLocation::Space(crate::cloud_object::Space::Team {
                team_uid,
            }),
            ctx,
        );
    });
    to_drive_data(DriveMutationResult {
        object: summary,
        audit: Some(audit(ActionKind::DriveObjectShareToTeam, subject)),
    })
}

#[derive(serde::Deserialize)]
struct DriveObjectIdParams {
    id: DriveObjectId,
}

fn drive_object_summary(object: &dyn CloudObject) -> Option<DriveObjectSummary> {
    Some(DriveObjectSummary {
        object_type: control_drive_object_type(object)?,
        id: object.uid(),
        name: object.display_name(),
    })
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

fn control_drive_object_type(object: &dyn CloudObject) -> Option<DriveObjectType> {
    match object.object_type() {
        ObjectType::Workflow => {
            let workflow = object.as_any().downcast_ref::<CloudWorkflow>()?;
            if workflow.model().data.is_agent_mode_workflow() {
                Some(DriveObjectType::Prompt)
            } else {
                Some(DriveObjectType::Workflow)
            }
        }
        ObjectType::Notebook => Some(DriveObjectType::Notebook),
        ObjectType::Folder => Some(DriveObjectType::Folder),
        ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
            JsonObjectType::EnvVarCollection,
        )) => Some(DriveObjectType::EnvVarCollection),
        ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
            JsonObjectType::AIFact,
        )) => Some(DriveObjectType::AiFact),
        ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
            JsonObjectType::MCPServer | JsonObjectType::TemplatableMCPServer,
        )) => Some(DriveObjectType::McpServer),
        _ => None,
    }
}

fn drive_object_content(object: &dyn CloudObject) -> Result<serde_json::Value, ControlError> {
    match control_drive_object_type(object).ok_or_else(drive_unsupported_type_error)? {
        DriveObjectType::Workflow | DriveObjectType::Prompt => object
            .as_any()
            .downcast_ref::<CloudWorkflow>()
            .ok_or_else(drive_type_mismatch_error)
            .and_then(|workflow| {
                serde_json::to_value(&workflow.model().data).map_err(json_response_error)
            }),
        DriveObjectType::Notebook => {
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
        DriveObjectType::EnvVarCollection => object
            .as_any()
            .downcast_ref::<CloudEnvVarCollection>()
            .ok_or_else(drive_type_mismatch_error)
            .and_then(|env_var_collection| {
                serde_json::to_value(&env_var_collection.model().string_model)
                    .map_err(json_response_error)
            }),
        DriveObjectType::Folder => {
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
        DriveObjectType::AiFact
        | DriveObjectType::McpServer
        | DriveObjectType::McpServerCollection
        | DriveObjectType::AiRule
        | DriveObjectType::Space
        | DriveObjectType::Trash => Err(drive_unsupported_type_error()),
    }
}

fn drive_object_type_rank(object_type: DriveObjectType) -> u8 {
    match object_type {
        DriveObjectType::Workflow => 0,
        DriveObjectType::Prompt => 1,
        DriveObjectType::Notebook => 2,
        DriveObjectType::EnvVarCollection => 3,
        DriveObjectType::Folder => 4,
        DriveObjectType::AiFact => 5,
        DriveObjectType::McpServer => 6,
        DriveObjectType::McpServerCollection => 7,
        DriveObjectType::AiRule => 8,
        DriveObjectType::Space => 9,
        DriveObjectType::Trash => 10,
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

fn validate_drive_mutation_target(
    target: &TargetSelector,
    action: ActionKind,
) -> Result<(), ControlError> {
    if target.tab.is_some()
        || target.pane.is_some()
        || target.session.is_some()
        || target.block.is_some()
        || target.file.is_some()
        || target.project.is_some()
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} does not accept tab, pane, session, block, file, or project selectors",
                action.as_str()
            ),
        ));
    }
    match (action, target.drive_object.as_ref()) {
        (ActionKind::DriveObjectCreate, None) => Ok(()),
        (ActionKind::DriveObjectCreate, Some(_)) => Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "drive.object.create uses explicit parameters and does not accept Drive object selectors",
        )),
        (
            ActionKind::DriveObjectUpdate
            | ActionKind::DriveObjectDelete
            | ActionKind::DriveObjectInsert
            | ActionKind::DriveObjectShareToTeam,
            Some(DriveObjectTarget::Id { id }),
        ) => {
            if id.0.trim().is_empty() {
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
        (
            ActionKind::DriveObjectUpdate
            | ActionKind::DriveObjectDelete
            | ActionKind::DriveObjectInsert
            | ActionKind::DriveObjectShareToTeam,
            Some(DriveObjectTarget::Lookup { .. }),
        ) => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!("{} does not support Drive lookup selectors", action.as_str()),
        )),
        (
            ActionKind::DriveObjectUpdate
            | ActionKind::DriveObjectDelete
            | ActionKind::DriveObjectInsert
            | ActionKind::DriveObjectShareToTeam,
            None,
        ) => Ok(()),
        _ => Ok(()),
    }
}

fn validate_drive_insert_target(target: &TargetSelector) -> Result<(), ControlError> {
    if target.drive_object.is_some()
        || target.block.is_some()
        || target.file.is_some()
        || target.project.is_some()
    {
        return Err(ControlError::new(
            ErrorCode::InvalidSelector,
            "drive.object.insert insertion target only accepts window selectors",
        ));
    }
    validate_drive_open_target(target, ActionKind::DriveObjectInsert)
}

fn validate_drive_target_matches_params(
    target: &TargetSelector,
    id: &str,
    action: ActionKind,
) -> Result<(), ControlError> {
    if let Some(DriveObjectTarget::Id { id: target_id }) = target.drive_object.as_ref() {
        if target_id.0 != id {
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

fn validate_drive_request_id(id: &str, action: ActionKind) -> Result<(), ControlError> {
    if id.trim().is_empty() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{} requires a non-empty Drive object id", action.as_str()),
        ));
    }
    Ok(())
}

fn drive_content_value(
    content: Option<&str>,
    content_file: Option<&str>,
    action: ActionKind,
) -> Result<Option<serde_json::Value>, ControlError> {
    match (content, content_file) {
        (Some(_), Some(_)) => Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!(
                "{} accepts either content or content_file, not both",
                action.as_str()
            ),
        )),
        (Some(content), None) => Ok(Some(parse_drive_content(content, action)?)),
        (None, Some(path)) => {
            if path.trim().is_empty() {
                return Err(ControlError::new(
                    ErrorCode::InvalidParams,
                    format!("{} content_file must be non-empty", action.as_str()),
                ));
            }
            let content = std::fs::read_to_string(path).map_err(|err| {
                ControlError::with_details(
                    ErrorCode::InvalidParams,
                    format!("{} could not read content_file", action.as_str()),
                    err.to_string(),
                )
            })?;
            Ok(Some(parse_drive_content(&content, action)?))
        }
        (None, None) => Ok(None),
    }
}

fn parse_drive_content(
    content: &str,
    action: ActionKind,
) -> Result<serde_json::Value, ControlError> {
    if content.trim().is_empty() {
        return Ok(serde_json::Value::String(String::new()));
    }
    serde_json::from_str(content)
        .or_else(|_| Ok(serde_json::Value::String(content.to_owned())))
        .map_err(|err: serde_json::Error| {
            ControlError::with_details(
                ErrorCode::InvalidParams,
                format!("{} content is not valid", action.as_str()),
                err.to_string(),
            )
        })
}

fn required_content(content: Option<serde_json::Value>) -> Result<serde_json::Value, ControlError> {
    content.ok_or_else(|| {
        ControlError::new(
            ErrorCode::InvalidParams,
            "this Drive object mutation requires content or content_file",
        )
    })
}

fn optional_content(content: Option<serde_json::Value>) -> serde_json::Value {
    content.unwrap_or_else(|| serde_json::Value::Object(Default::default()))
}

fn object_name_from_content(
    object_type: DriveObjectType,
    content: Option<&serde_json::Value>,
) -> String {
    content
        .and_then(|value| {
            value
                .get("name")
                .or_else(|| value.get("title"))
                .and_then(serde_json::Value::as_str)
        })
        .filter(|name| !name.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| match object_type {
            DriveObjectType::Workflow => "Untitled Workflow".to_owned(),
            DriveObjectType::Prompt => "Untitled Prompt".to_owned(),
            DriveObjectType::Notebook => "Untitled Notebook".to_owned(),
            DriveObjectType::EnvVarCollection => "Untitled Environment".to_owned(),
            DriveObjectType::Folder => "Untitled Folder".to_owned(),
            DriveObjectType::AiFact
            | DriveObjectType::AiRule
            | DriveObjectType::McpServer
            | DriveObjectType::McpServerCollection
            | DriveObjectType::Space
            | DriveObjectType::Trash => "Untitled Drive Object".to_owned(),
        })
}

#[derive(serde::Deserialize)]
struct NotebookDriveContent {
    title: Option<String>,
    data: Option<String>,
}

fn workflow_from_drive_content(
    object_type: DriveObjectType,
    fallback_name: &str,
    content: serde_json::Value,
) -> Result<Workflow, ControlError> {
    if let Ok(mut workflow) = serde_json::from_value::<Workflow>(content.clone()) {
        if workflow_kind_matches(object_type, &workflow) {
            if !fallback_name.trim().is_empty() {
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
        DriveObjectType::Workflow => {
            let command = content.get("command").and_then(serde_json::Value::as_str);
            let command = command.or_else(|| content.as_str()).ok_or_else(|| {
                ControlError::new(
                    ErrorCode::InvalidParams,
                    "drive.object.create/update workflow content requires a command string or typed workflow object",
                )
            })?;
            Ok(Workflow::new(fallback_name, command))
        }
        DriveObjectType::Prompt => {
            let query = content.get("query").and_then(serde_json::Value::as_str);
            let query = query.or_else(|| content.as_str()).ok_or_else(|| {
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
        _ => Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            "workflow content is only valid for workflow and prompt Drive object types",
        )),
    }
}

fn workflow_kind_matches(object_type: DriveObjectType, workflow: &Workflow) -> bool {
    match object_type {
        DriveObjectType::Workflow => workflow.is_command_workflow(),
        DriveObjectType::Prompt => workflow.is_agent_mode_workflow(),
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
    let mut env_vars = serde_json::from_value::<EnvVarCollection>(content).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            "drive.object.create/update env-var-collection content requires a typed environment-variable collection",
            err.to_string(),
        )
    })?;
    if env_vars.title.as_ref().is_none_or(String::is_empty) {
        env_vars.title = non_empty_string(fallback_title);
    }
    Ok(env_vars)
}

fn folder_name_from_content(content: &serde_json::Value) -> Result<String, ControlError> {
    content
        .get("name")
        .and_then(serde_json::Value::as_str)
        .or_else(|| content.as_str())
        .filter(|name| !name.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::InvalidParams,
                "drive.object.update folder content requires a non-empty name string",
            )
        })
}

fn non_empty_string(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_owned())
}

fn drive_object_for_mutation<'a>(
    cloud_model: &'a CloudModel,
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
    drive_object_summary(object).ok_or_else(|| {
        ControlError::new(
            ErrorCode::UnsupportedAction,
            format!(
                "{} does not support this Drive object type",
                action.as_str()
            ),
        )
    })?;
    Ok(object)
}

fn unsupported_mutation_type_error(
    action: ActionKind,
    object_type: DriveObjectType,
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

fn drive_mutation_result(
    object: &dyn CloudObject,
    object_type: DriveObjectType,
    action: ActionKind,
    subject: String,
) -> Result<serde_json::Value, ControlError> {
    let summary = drive_object_summary(object).ok_or_else(drive_unsupported_type_error)?;
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

fn authenticated_user_subject(
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<String, ControlError> {
    AuthStateProvider::as_ref(ctx)
        .get()
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
    AuthStateProvider::as_ref(ctx)
        .get()
        .user_id()
        .map(|user_uid| Owner::User { user_uid })
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::AuthenticatedUserUnavailable,
                "this action requires a logged-in Warp user",
            )
        })
}

fn audit(action: ActionKind, authenticated_user_subject: String) -> DriveMutationAudit {
    DriveMutationAudit {
        action: action.as_str().to_owned(),
        authenticated_user_subject,
        permission_category: PermissionCategory::MutateUnderlyingData,
    }
}

fn to_drive_data<T: serde::Serialize>(data: T) -> Result<serde_json::Value, ControlError> {
    serde_json::to_value(data).map_err(json_response_error)
}

fn resolve_cloud_object_type_and_id(
    id: &str,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<CloudObjectTypeAndId, ControlError> {
    if id.trim().is_empty() {
        return Err(ControlError::new(
            ErrorCode::InvalidParams,
            format!("{} requires a non-empty Drive object id", action.as_str()),
        ));
    }
    let owned_id = id.to_owned();
    let object = CloudModel::as_ref(ctx)
        .get_by_uid(&owned_id)
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::StaleTarget,
                format!(
                    "{} could not resolve the requested Drive object id",
                    action.as_str()
                ),
            )
        })?;
    Ok(object.cloud_object_type_and_id())
}

fn resolve_typed_drive_sync_id(
    id: &str,
    expected: ObjectType,
    action: ActionKind,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<SyncId, ControlError> {
    let object_type_and_id = resolve_cloud_object_type_and_id(id, action, ctx)?;
    if object_type_and_id.object_type() != expected {
        return Err(ControlError::new(
            ErrorCode::TargetStateConflict,
            format!(
                "{} can only open Drive objects of type {}",
                action.as_str(),
                expected
            ),
        ));
    }
    Ok(object_type_and_id.sync_id())
}

fn select_window_for_drive_open(
    action: ActionKind,
    target: &TargetSelector,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<WindowId, ControlError> {
    match target.window.as_ref() {
        None | Some(WindowTarget::Active) => {
            require_active_window_id_for_action(ctx.windows().active_window(), action)
        }
        Some(WindowTarget::Id { id }) => ctx
            .window_ids()
            .find(|window_id| window_id.to_string() == id.0)
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::StaleTarget,
                    format!("{} cannot resolve the requested window id", action.as_str()),
                )
            }),
        Some(WindowTarget::Index { .. } | WindowTarget::Title { .. }) => Err(ControlError::new(
            ErrorCode::InvalidSelector,
            format!(
                "{} only supports active and opaque window id selectors",
                action.as_str()
            ),
        )),
    }
}

fn workspace_for_window(
    action: ActionKind,
    window_id: WindowId,
    ctx: &mut ModelContext<LocalControlBridge>,
) -> Result<ViewHandle<Workspace>, ControlError> {
    ctx.views_of_type::<Workspace>(window_id)
        .and_then(|workspaces| workspaces.into_iter().next())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::MissingTarget,
                format!(
                    "{} requires a workspace in the target window",
                    action.as_str()
                ),
            )
        })
}

fn cloud_object_type_label(object_type_and_id: CloudObjectTypeAndId) -> &'static str {
    match object_type_and_id {
        CloudObjectTypeAndId::Notebook(_) => "notebook",
        CloudObjectTypeAndId::Workflow(_) => "workflow",
        CloudObjectTypeAndId::Folder(_) => "folder",
        CloudObjectTypeAndId::GenericStringObject {
            object_type: GenericStringObjectFormat::Json(JsonObjectType::EnvVarCollection),
            ..
        } => "env_var_collection",
        CloudObjectTypeAndId::GenericStringObject {
            object_type: GenericStringObjectFormat::Json(JsonObjectType::AIFact),
            ..
        } => "ai_fact",
        CloudObjectTypeAndId::GenericStringObject {
            object_type:
                GenericStringObjectFormat::Json(
                    JsonObjectType::MCPServer | JsonObjectType::TemplatableMCPServer,
                ),
            ..
        } => "mcp_server",
        CloudObjectTypeAndId::GenericStringObject { .. } => "generic_string_object",
    }
}
