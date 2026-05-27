use crate::localization;
use warpui::AppContext;

use super::{CloudObject, GenericStringObjectFormat, JsonObjectType, ObjectType};
use crate::server::cloud_objects::update_manager::{
    InitiatedBy, ObjectOperation, OperationSuccessType,
};

pub struct CloudObjectToastMessage;

fn text(app: &AppContext, key: &str) -> String {
    localization::text_for_app(app, key)
}

fn replace_object(app: &AppContext, key: &str, object: &str) -> String {
    text(app, key).replace("{object}", object)
}

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
            (
                _,
                ObjectOperation::Create {
                    initiated_by: InitiatedBy::User,
                },
                OperationSuccessType::Success,
            ) => {
                let containing_object_name = object.containing_object_name(app);
                Some(
                    text(app, "cloud_object.toast.saved_to")
                        .replace("{object}", &object_name)
                        .replace("{location}", &containing_object_name),
                )
            }
            // notebooks intentionally do not have an update message, as they are updated
            // as the user types and so toasts would be VERY noisy
            (ObjectType::Notebook, ObjectOperation::Update, OperationSuccessType::Success) => None,
            (_, ObjectOperation::Update, OperationSuccessType::Success) => Some(replace_object(
                app,
                "cloud_object.toast.updated",
                &object_name,
            )),
            (_, ObjectOperation::MoveToFolder, OperationSuccessType::Success)
            | (_, ObjectOperation::MoveToDrive, OperationSuccessType::Success) => {
                let containing_object_name = object.containing_object_name(app);
                Some(
                    text(app, "cloud_object.toast.moved_to")
                        .replace("{object}", &object_name)
                        .replace("{location}", &containing_object_name),
                )
            }
            (_, ObjectOperation::Trash, OperationSuccessType::Success) => Some(replace_object(
                app,
                "cloud_object.toast.trashed",
                &object_name,
            )),
            (_, ObjectOperation::Untrash, OperationSuccessType::Success) => Some(replace_object(
                app,
                "cloud_object.toast.restored",
                &object_name,
            )),
            (_, ObjectOperation::Leave, OperationSuccessType::Success) => {
                Some(replace_object(app, "cloud_object.toast.left", &object_name))
            }
            (
                _,
                ObjectOperation::Create {
                    initiated_by: InitiatedBy::User,
                },
                OperationSuccessType::Failure,
            ) => Some(replace_object(
                app,
                "cloud_object.toast.failed_create",
                &object_name_lowercase,
            )),
            (
                _,
                ObjectOperation::Create {
                    initiated_by: InitiatedBy::User,
                },
                OperationSuccessType::Denied(message),
            ) => Some(message.to_string()),
            (_, ObjectOperation::Update, OperationSuccessType::Failure) => Some(replace_object(
                app,
                "cloud_object.toast.failed_update",
                &object_name_lowercase,
            )),
            (_, ObjectOperation::MoveToFolder, OperationSuccessType::Failure)
            | (_, ObjectOperation::MoveToDrive, OperationSuccessType::Failure) => {
                Some(replace_object(
                    app,
                    "cloud_object.toast.failed_move",
                    &object_name_lowercase,
                ))
            }
            (_, ObjectOperation::Trash, OperationSuccessType::Failure) => Some(replace_object(
                app,
                "cloud_object.toast.failed_trash",
                &object_name_lowercase,
            )),
            (_, ObjectOperation::Untrash, OperationSuccessType::Failure) => Some(replace_object(
                app,
                "cloud_object.toast.failed_restore",
                &object_name_lowercase,
            )),
            // We should only show deletion failure toasts for user-initiated deletions.
            (
                _,
                ObjectOperation::Delete {
                    initiated_by: InitiatedBy::User,
                },
                OperationSuccessType::Failure,
            ) => Some(replace_object(
                app,
                "cloud_object.toast.failed_delete",
                &object_name_lowercase,
            )),
            (_, ObjectOperation::Leave, OperationSuccessType::Failure) => Some(replace_object(
                app,
                "cloud_object.toast.failed_leave",
                &object_name,
            )),
            (ObjectType::Workflow, ObjectOperation::Update, OperationSuccessType::Rejection) => {
                Some(text(app, "cloud_object.toast.rejection.workflow"))
            }
            (
                ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                    JsonObjectType::EnvVarCollection,
                )),
                ObjectOperation::Update,
                OperationSuccessType::Rejection,
            ) => Some(text(app, "cloud_object.toast.rejection.env_vars")),
            (
                ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                    JsonObjectType::AIFact,
                )),
                ObjectOperation::Update,
                OperationSuccessType::Rejection,
            ) => Some(text(app, "cloud_object.toast.rejection.rule")),
            (_, ObjectOperation::TakeEditAccess, OperationSuccessType::Failure) => {
                Some(replace_object(
                    app,
                    "cloud_object.toast.failed_start_editing",
                    &object_name_lowercase,
                ))
            }
            (_, ObjectOperation::UpdatePermissions, OperationSuccessType::Success) => {
                Some(replace_object(
                    app,
                    "cloud_object.toast.updated_permissions",
                    &object_name_lowercase,
                ))
            }
            (_, ObjectOperation::UpdatePermissions, OperationSuccessType::Failure) => {
                Some(replace_object(
                    app,
                    "cloud_object.toast.failed_update_permissions",
                    &object_name_lowercase,
                ))
            }
            _ => None,
        }
    }

    pub fn toast_deletion_confirm_message(
        num_objects: i32,
        operation: &ObjectOperation,
        success_type: &OperationSuccessType,
        app: &AppContext,
    ) -> Option<String> {
        let count_objects_message = if num_objects == 1 {
            text(app, "cloud_object.toast.object_count.singular")
        } else {
            text(app, "cloud_object.toast.object_count.plural")
                .replace("{count}", &num_objects.to_string())
        };
        match (operation, success_type) {
            // We should only show deletion failure toasts for user-initiated deletions.
            (
                ObjectOperation::Delete {
                    initiated_by: InitiatedBy::User,
                },
                OperationSuccessType::Success,
            ) => Some(
                text(app, "cloud_object.toast.deleted_forever")
                    .replace("{count_objects}", &count_objects_message),
            ),
            (ObjectOperation::EmptyTrash, OperationSuccessType::Success) => Some(
                text(app, "cloud_object.toast.trash_emptied")
                    .replace("{count_objects}", &count_objects_message),
            ),
            (ObjectOperation::EmptyTrash, OperationSuccessType::Failure) => {
                Some(text(app, "cloud_object.toast.failed_empty_trash"))
            }
            (ObjectOperation::EmptyTrash, OperationSuccessType::Rejection) => {
                Some(text(app, "cloud_object.toast.no_objects_to_empty"))
            }
            _ => None,
        }
    }
}
