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
            (
                _,
                ObjectOperation::Create {
                    initiated_by: InitiatedBy::User,
                },
                OperationSuccessType::Success,
            ) => {
                let containing_object_name = object.containing_object_name(app);
                Some(
                    i18n::t("cloud_object.toast.saved_to")
                        .replace("{object_name}", &object_name)
                        .replace("{containing_object_name}", &containing_object_name),
                )
            }
            // notebooks intentionally do not have an update message, as they are updated
            // as the user types and so toasts would be VERY noisy
            (ObjectType::Notebook, ObjectOperation::Update, OperationSuccessType::Success) => None,
            (_, ObjectOperation::Update, OperationSuccessType::Success) => {
                Some(i18n::t("cloud_object.toast.updated").replace("{object_name}", &object_name))
            }
            (_, ObjectOperation::MoveToFolder, OperationSuccessType::Success)
            | (_, ObjectOperation::MoveToDrive, OperationSuccessType::Success) => {
                let containing_object_name = object.containing_object_name(app);
                Some(
                    i18n::t("cloud_object.toast.moved_to")
                        .replace("{object_name}", &object_name)
                        .replace("{containing_object_name}", &containing_object_name),
                )
            }
            (_, ObjectOperation::Trash, OperationSuccessType::Success) => {
                Some(i18n::t("cloud_object.toast.trashed").replace("{object_name}", &object_name))
            }
            (_, ObjectOperation::Untrash, OperationSuccessType::Success) => {
                Some(i18n::t("cloud_object.toast.restored").replace("{object_name}", &object_name))
            }
            (_, ObjectOperation::Leave, OperationSuccessType::Success) => {
                Some(i18n::t("cloud_object.toast.left").replace("{object_name}", &object_name))
            }
            (
                _,
                ObjectOperation::Create {
                    initiated_by: InitiatedBy::User,
                },
                OperationSuccessType::Failure,
            ) => Some(
                i18n::t("cloud_object.toast.failed_create")
                    .replace("{object_name_lowercase}", &object_name_lowercase),
            ),
            (
                _,
                ObjectOperation::Create {
                    initiated_by: InitiatedBy::User,
                },
                OperationSuccessType::Denied(message),
            ) => Some(message.to_string()),
            (_, ObjectOperation::Update, OperationSuccessType::Failure) => Some(
                i18n::t("cloud_object.toast.failed_update")
                    .replace("{object_name_lowercase}", &object_name_lowercase),
            ),
            (_, ObjectOperation::MoveToFolder, OperationSuccessType::Failure)
            | (_, ObjectOperation::MoveToDrive, OperationSuccessType::Failure) => Some(
                i18n::t("cloud_object.toast.failed_move")
                    .replace("{object_name_lowercase}", &object_name_lowercase),
            ),
            (_, ObjectOperation::Trash, OperationSuccessType::Failure) => Some(
                i18n::t("cloud_object.toast.failed_trash")
                    .replace("{object_name_lowercase}", &object_name_lowercase),
            ),
            (_, ObjectOperation::Untrash, OperationSuccessType::Failure) => Some(
                i18n::t("cloud_object.toast.failed_restore")
                    .replace("{object_name_lowercase}", &object_name_lowercase),
            ),
            // We should only show deletion failure toasts for user-initiated deletions.
            (
                _,
                ObjectOperation::Delete {
                    initiated_by: InitiatedBy::User,
                },
                OperationSuccessType::Failure,
            ) => Some(
                i18n::t("cloud_object.toast.failed_delete")
                    .replace("{object_name_lowercase}", &object_name_lowercase),
            ),
            (_, ObjectOperation::Leave, OperationSuccessType::Failure) => Some(
                i18n::t("cloud_object.toast.failed_leave").replace("{object_name}", &object_name),
            ),
            (ObjectType::Workflow, ObjectOperation::Update, OperationSuccessType::Rejection) => {
                Some(i18n::t("cloud_object.toast.workflow_conflict"))
            }
            (
                ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                    JsonObjectType::EnvVarCollection,
                )),
                ObjectOperation::Update,
                OperationSuccessType::Rejection,
            ) => Some(i18n::t("cloud_object.toast.env_vars_conflict")),
            (
                ObjectType::GenericStringObject(GenericStringObjectFormat::Json(
                    JsonObjectType::AIFact,
                )),
                ObjectOperation::Update,
                OperationSuccessType::Rejection,
            ) => Some(i18n::t("cloud_object.toast.rule_conflict")),
            (_, ObjectOperation::TakeEditAccess, OperationSuccessType::Failure) => Some(
                i18n::t("cloud_object.toast.failed_start_editing")
                    .replace("{object_name_lowercase}", &object_name_lowercase),
            ),
            (_, ObjectOperation::UpdatePermissions, OperationSuccessType::Success) => Some(
                i18n::t("cloud_object.toast.permissions_updated")
                    .replace("{object_name_lowercase}", &object_name_lowercase),
            ),
            (_, ObjectOperation::UpdatePermissions, OperationSuccessType::Failure) => Some(
                i18n::t("cloud_object.toast.permissions_update_failed")
                    .replace("{object_name_lowercase}", &object_name_lowercase),
            ),
            _ => None,
        }
    }

    pub fn toast_deletion_confirm_message(
        num_objects: i32,
        operation: &ObjectOperation,
        success_type: &OperationSuccessType,
    ) -> Option<String> {
        let count_objects_message = match num_objects {
            1 => i18n::t("cloud_object.toast.object_count_one"),
            n => i18n::t("cloud_object.toast.object_count_many").replace("{count}", &n.to_string()),
        };
        match (operation, success_type) {
            // We should only show deletion failure toasts for user-initiated deletions.
            (
                ObjectOperation::Delete {
                    initiated_by: InitiatedBy::User,
                },
                OperationSuccessType::Success,
            ) => Some(
                i18n::t("cloud_object.toast.deleted_forever")
                    .replace("{count_objects_message}", &count_objects_message),
            ),
            (ObjectOperation::EmptyTrash, OperationSuccessType::Success) => Some(
                i18n::t("cloud_object.toast.trash_emptied")
                    .replace("{count_objects_message}", &count_objects_message),
            ),
            (ObjectOperation::EmptyTrash, OperationSuccessType::Failure) => {
                Some(i18n::t("cloud_object.toast.empty_trash_failed"))
            }
            (ObjectOperation::EmptyTrash, OperationSuccessType::Rejection) => {
                Some(i18n::t("cloud_object.toast.no_objects_to_empty"))
            }
            _ => None,
        }
    }
}
