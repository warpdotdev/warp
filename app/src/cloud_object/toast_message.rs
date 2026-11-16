use warpui::AppContext;

use super::{CloudObject, GenericStringObjectFormat, JsonObjectType, ObjectType};
use crate::server::cloud_objects::update_manager::{
    InitiatedBy, ObjectOperation, OperationSuccessType,
};

pub struct CloudObjectToastMessage;

impl CloudObjectToastMessage {
    pub fn toast_message(
        object: &dyn CloudObject,
        operation: &ObjectOperation,
        success_type: &OperationSuccessType,
        app: &AppContext,
    ) -> Option<String> {
        let object_name = object.model_type_name().to_owned();
        let object_name_lowercase = object_name.to_ascii_lowercase();

        match (object.object_type(), operation, success_type) {
            // We should only show toasts for creates initiated by the user, not by the system
            (_, ObjectOperation::Create { initiated_by: InitiatedBy::User }, OperationSuccessType::Success) => {
                let containing_object_name = object.containing_object_name(app);
                Some(format!("{object_name} saved to {containing_object_name}"))
            }
            // notebooks intentionally do not have an update message, as they are updated
            // as the user types and so toasts would be VERY noisy
            (
                ObjectType::Notebook,
                ObjectOperation::Update,
                OperationSuccessType::Success,
            ) => None,
            (_, ObjectOperation::Update, OperationSuccessType::Success) => {
                Some(format!("{object_name} updated"))
            }
            (_, ObjectOperation::MoveToFolder, OperationSuccessType::Success) | (_, ObjectOperation::MoveToDrive, OperationSuccessType::Success) => {
                let containing_object_name = object.containing_object_name(app);
                Some(format!("{object_name} moved to {containing_object_name}"))
            }
            (_, ObjectOperation::Trash, OperationSuccessType::Success) => {
                Some(format!("{object_name} trashed"))
            }
            (_, ObjectOperation::Untrash, OperationSuccessType::Success) => {
                Some(format!("{object_name} restored"))
            }
            (_, ObjectOperation::Leave, OperationSuccessType::Success) => {
                Some(format!("Left {object_name}"))
            }
            (_, ObjectOperation::Create { initiated_by: InitiatedBy::User }, OperationSuccessType::Failure) => {
                Some(format!("Failed to create {object_name_lowercase}"))
            }
            (_, ObjectOperation::Create { initiated_by: InitiatedBy::User }, OperationSuccessType::Denied(message)) => {
                Some(message.to_string())
            }
            (_, ObjectOperation::Update, OperationSuccessType::Failure) => {
                Some(format!("Failed to update {object_name_lowercase}"))
            }
            (_, ObjectOperation::MoveToFolder, OperationSuccessType::Failure) | (_, ObjectOperation::MoveToDrive, OperationSuccessType::Failure) => {
                Some(format!("Failed to move {object_name_lowercase}"))
            }
            (_, ObjectOperation::Trash, OperationSuccessType::Failure) => {
                Some(format!("Failed to trash {object_name_lowercase}"))
            }
            (_, ObjectOperation::Untrash, OperationSuccessType::Failure) => {
                Some(format!("Failed to restore {object_name_lowercase}"))
            }
            // We should only show deletion failure toasts for user-initiated deletions.
            (_, ObjectOperation::Delete { initiated_by: InitiatedBy::User }, OperationSuccessType::Failure) => {
                Some(format!("Failed to delete {object_name_lowercase}"))
            }
            (_, ObjectOperation::Leave, OperationSuccessType::Failure) => {
                Some(format!("Failed to leave {object_name}"))
            }
            (
                ObjectType::Workflow,
                ObjectOperation::Update,
                OperationSuccessType::Rejection,
            ) => {
                Some("This workflow could not be saved because changes were made while you were editing.".to_string())
            }
            (
                ObjectType::GenericStringObject(GenericStringObjectFormat::Json(JsonObjectType::EnvVarCollection)),
                ObjectOperation::Update,
                OperationSuccessType::Rejection,
            ) => {
                Some("Environment variables could not be saved because changes were made while you were editing.".to_string())
            }
            (
                ObjectType::GenericStringObject(GenericStringObjectFormat::Json(JsonObjectType::AIFact)),
                ObjectOperation::Update,
                OperationSuccessType::Rejection,
            ) => {
                Some("Rule could not be saved because changes were made while you were editing.".to_string())
            }
            (_, ObjectOperation::TakeEditAccess, OperationSuccessType::Failure) => {
                Some(format!("Failed to start editing {object_name_lowercase}"))
            }
            (_, ObjectOperation::UpdatePermissions, OperationSuccessType::Success) => {
                Some(format!("Successfully updated permissions for {object_name_lowercase}"))
            }
            (_, ObjectOperation::UpdatePermissions, OperationSuccessType::Failure) => {
                Some(format!("Failed to update permissions for {object_name_lowercase}"))
            }
            _ => None,
        }
    }

    pub fn toast_deletion_confirm_message(
        num_objects: i32,
        operation: &ObjectOperation,
        success_type: &OperationSuccessType,
    ) -> Option<String> {
        let count_objects_message = match num_objects {
            1 => "1 object".to_string(),
            n => {
                format!("{n} objects")
            }
        };
        match (operation, success_type) {
            // We should only show deletion failure toasts for user-initiated deletions.
            (
                ObjectOperation::Delete {
                    initiated_by: InitiatedBy::User,
                },
                OperationSuccessType::Success,
            ) => Some(format!("{count_objects_message} deleted forever")),
            (ObjectOperation::EmptyTrash, OperationSuccessType::Success) => Some(format!(
                "Trash emptied: {count_objects_message} deleted forever"
            )),
            (ObjectOperation::EmptyTrash, OperationSuccessType::Failure) => {
                Some("Failed to empty trash".to_string())
            }
            (ObjectOperation::EmptyTrash, OperationSuccessType::Rejection) => {
                Some("No objects in trash to empty".to_string())
            }
            _ => None,
        }
    }
}
