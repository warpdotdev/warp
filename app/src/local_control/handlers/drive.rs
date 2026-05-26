use ::local_control::protocol::{
    DriveInspectParams, DriveInspectResult, DriveListParams, DriveListResult, DriveObjectSummary,
    DriveObjectType, TargetSelector,
};
use ::local_control::{ActionKind, ControlError, ErrorCode};
use serde_json::json;
use warpui::{ModelContext, SingletonEntity};

use crate::cloud_object::{
    model::persistence::CloudModel, CloudObject, GenericStringObjectFormat, JsonObjectType,
    ObjectType,
};
use crate::drive::folders::CloudFolder;
use crate::env_vars::CloudEnvVarCollection;
use crate::local_control::LocalControlBridge;
use crate::notebooks::CloudNotebook;
use crate::workflows::CloudWorkflow;

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
        DriveObjectType::Space => 7,
        DriveObjectType::Trash => 8,
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
