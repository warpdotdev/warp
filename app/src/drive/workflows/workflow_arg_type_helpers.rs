use std::collections::HashMap;

use warp_errors::report_error;
use warpui::{AppContext, SingletonEntity, ViewHandle};

use super::enum_creation_dialog::{EnumCreationDialog, WorkflowEnumData};
use super::workflow_arg_selector::WorkflowArgSelector;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObjectEventEntrypoint, Owner};
use crate::editor::EditorView;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::SyncId;
use crate::workflows::workflow::{Argument, ArgumentType};
use crate::workflows::workflow_enum::WorkflowEnum;

#[derive(Debug, Clone)]
pub struct ArgumentEditorRowIndex(pub usize);

/// Trait for getting a `WorkflowArgSelector` from a component.
/// Used to make helper functions generic, working for both the
/// `WorkflowView` and `WorkflowModal` components.
pub trait ArgumentTypeEditor {
    fn arg_type_editor(&self) -> &ViewHandle<WorkflowArgSelector>;
}

impl ArgumentTypeEditor for super::modal::ArgumentEditorRow {
    fn arg_type_editor(&self) -> &ViewHandle<WorkflowArgSelector> {
        &self.typed_default_value_editor
    }
}

/// Get all workflow enums in the space, filtering to only show the shared ones
pub fn load_workflow_enums_with_owner<V>(
    owner: Owner,
    ctx: &mut warpui::ViewContext<V>,
) -> HashMap<SyncId, WorkflowEnumData>
where
    V: warpui::View,
{
    let cloud_model = CloudModel::as_ref(ctx);
    cloud_model
        .workflow_enums_with_owner(owner, ctx)
        .filter(|workflow_enum| workflow_enum.model().string_model.is_shared)
        .map(|workflow_enum| {
            let enum_id = workflow_enum.id;
            let enum_data = WorkflowEnumData {
                name: workflow_enum.model().string_model.name.clone(),
                id: enum_id,
                is_shared: workflow_enum.model().string_model.is_shared,
                revision_ts: workflow_enum.metadata.revision.clone(),
                new_data: None,
            };
            (enum_id, enum_data)
        })
        .collect()
}

/// Helper function used to load an argument into the ArgSelector component on initialization
/// Used by both `WorkflowModal` and `WorkflowView`
pub fn load_argument_into_selector(
    selector: &mut WorkflowArgSelector,
    argument: &Argument,
    all_workflow_enums: &mut HashMap<SyncId, WorkflowEnumData>,
    ctx: &mut warpui::ViewContext<WorkflowArgSelector>,
) {
    let selected_type = argument.arg_type.clone().into();
    selector.set_selected_type(selected_type, ctx);

    if let ArgumentType::Enum { enum_id } = argument.arg_type {
        // If we have the enum in the global list, add it to the menu
        // Otherwise, get the enum data from memory and make a new entry in the list for it
        if let Some(enum_data) = all_workflow_enums.get(&enum_id) {
            selector.insert_enum_into_menu(enum_id, enum_data.name.clone(), ctx);
        } else {
            // Grab the revision_ts, enum name, and shared status from the cloud model
            let cloud_model = CloudModel::as_ref(ctx);
            let workflow_enum_model = cloud_model.get_workflow_enum(&enum_id);
            let revision_ts = workflow_enum_model.and_then(|model| model.metadata.revision.clone());
            let enum_data = workflow_enum_model.map(|workflow_enum| {
                let workflow_enum = &workflow_enum.model().string_model;
                (workflow_enum.name.clone(), workflow_enum.is_shared)
            });

            // If we found an enum in memory, add the enum to the global list
            if let Some((enum_name, is_shared)) = enum_data {
                all_workflow_enums.insert(
                    enum_id,
                    WorkflowEnumData {
                        id: enum_id,
                        name: enum_name.clone(),
                        is_shared,
                        revision_ts,
                        new_data: None,
                    },
                );

                selector.insert_enum_into_menu(enum_id, enum_name, ctx);
            }
        }

        // Set the selected enum for the selector
        selector.set_selected_enum_with_base_enum(Some(enum_id), ctx);
    } else {
        selector.clear_data();
    }

    let text = match &argument.default_value {
        Some(default_value) => default_value.as_str(),
        None => "",
    };
    selector.set_editor_text(text, ctx);
}

/// Helper function used to create an argument given the workflow argument selector and text editor
/// Used by both `WorkflowModal` and `WorkflowView`
pub fn extract_typed_argument_from_selector(
    argument: &Argument,
    description: Option<String>,
    type_selector: &WorkflowArgSelector,
    text_editor: &EditorView,
    app: &AppContext,
) -> Argument {
    let id = type_selector.get_selected_enum();

    // If we have arg type data with an enum ID, use that as our type, otherwise text.
    let (arg_type, default_value) = match id {
        Some(enum_id) => (
            ArgumentType::Enum { enum_id },
            None, // we haven't implemented default value for enums
        ),
        None => (
            ArgumentType::Text,
            match text_editor.is_empty(app) {
                true => None,
                false => Some(text_editor.buffer_text(app)),
            },
        ),
    };

    Argument {
        name: argument.name.clone(),
        description,
        default_value,
        arg_type,
    }
}

/// Given arg type data, a space, and a ViewContext, saves the data represented by arg type data to the cloud.
pub fn save_enum<V>(
    enum_data: &WorkflowEnumData,
    owner: Option<Owner>,
    ctx: &mut warpui::ViewContext<V>,
) where
    V: warpui::View,
{
    let Some(variants) = enum_data.new_data.clone() else {
        return;
    };

    let workflow_enum = WorkflowEnum {
        name: enum_data.name.clone(),
        is_shared: true,
        variants,
    };

    // Depending on the type of ID, create or update the relevant objects.
    match enum_data.id {
        SyncId::ClientId(client_id) => {
            if let Some(owner) = owner {
                UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                    update_manager.create_workflow_enum(
                        workflow_enum,
                        owner,
                        client_id,
                        CloudObjectEventEntrypoint::Unknown,
                        true,
                        ctx,
                    );
                });
            }
        }
        SyncId::ServerId(_) => {
            // We will issue enum update requests here
            UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                update_manager.update_workflow_enum(
                    workflow_enum,
                    enum_data.id,
                    enum_data.revision_ts.clone(),
                    ctx,
                );
            })
        }
    }
}

/// Create a new enum after closing the enum dialog
pub fn create_enum<V, T>(
    enum_data: &WorkflowEnumData,
    all_workflow_enums: &mut HashMap<SyncId, WorkflowEnumData>,
    arguments_rows: &[T],
    pending_argument_editor_row: &mut Option<ArgumentEditorRowIndex>,
    ctx: &mut warpui::ViewContext<V>,
) where
    T: ArgumentTypeEditor,
    V: warpui::View,
{
    let enum_id = enum_data.id;
    let enum_name = enum_data.name.clone();

    // Add the data to the global list of enums
    all_workflow_enums.insert(enum_id, enum_data.clone());

    // Add the new enum to each argument row's list
    if enum_data.is_shared {
        arguments_rows.iter().for_each(|row| {
            row.arg_type_editor().update(ctx, |editor, ctx| {
                editor.insert_enum_into_menu(enum_id, enum_name.clone(), ctx);
            })
        });
    }

    // Update the relevant row with the new index of the selected enum
    if let Some(ArgumentEditorRowIndex(index)) = pending_argument_editor_row {
        arguments_rows[*index]
            .arg_type_editor()
            .update(ctx, |selector, ctx| {
                // Insert into the menu, which we might not have done earlier if the enum is not shared
                if !enum_data.is_shared {
                    selector.insert_enum_into_menu(enum_id, enum_name.clone(), ctx);
                }
                selector.set_selected_enum(Some(enum_id), ctx);
            });
    }
}

/// Edit an enum after closing the enum dialog
pub fn edit_enum<V, T>(
    enum_data: &WorkflowEnumData,
    did_visibility_change: bool,
    all_workflow_enums: &mut HashMap<SyncId, WorkflowEnumData>,
    arguments_rows: &[T],
    pending_argument_editor_row: &mut Option<ArgumentEditorRowIndex>,
    ctx: &mut warpui::ViewContext<V>,
) where
    T: ArgumentTypeEditor,
    V: warpui::View,
{
    let enum_id = enum_data.id;
    let enum_name = &enum_data.name;

    // Replace this item in the global enum map
    all_workflow_enums.insert(enum_data.id, enum_data.clone());

    // Update the enum to each argument row's list, in case its name was updated, if it is shared
    if enum_data.is_shared {
        arguments_rows.iter().for_each(|row| {
            row.arg_type_editor().update(ctx, |editor, ctx| {
                editor.insert_enum_into_menu(enum_id, enum_name.clone(), ctx);
            })
        });
    }
    // Otherwise, remove the enum from the dropdown list for every row if it is newly "unshared"
    else if !enum_data.is_shared && did_visibility_change {
        arguments_rows.iter().for_each(|row| {
            row.arg_type_editor().update(ctx, |editor, ctx| {
                editor.remove_enum_from_menu(&enum_id, ctx);
            })
        });
    }

    // Update the relevant row with the selected index of their enum
    if let Some(ArgumentEditorRowIndex(index)) = pending_argument_editor_row {
        arguments_rows[*index]
            .arg_type_editor()
            .update(ctx, |selector, ctx| {
                // Insert into the menu, which we might have undone earlier if the enum is unshared
                if !enum_data.is_shared {
                    selector.insert_enum_into_menu(enum_id, enum_name.clone(), ctx);
                }
            });
    }
}

/// Load in an enum to the enum dialog.
/// Returns a boolean, `true` if we want to show the enum dialog
pub fn load_enum<V>(
    id: &SyncId,
    all_workflow_enums: &HashMap<SyncId, WorkflowEnumData>,
    enum_creation_dialog: &ViewHandle<EnumCreationDialog>,
    ctx: &mut warpui::ViewContext<V>,
) -> bool
where
    V: warpui::View,
{
    match all_workflow_enums.get(id) {
        // If we have local variants for this enum, pass them in
        Some(WorkflowEnumData {
            name,
            is_shared,
            new_data: Some(new_data),
            ..
        }) => {
            enum_creation_dialog.update(ctx, |dialog, ctx| {
                dialog.load_from_data(name, *id, *is_shared, new_data, ctx);
            });
            true
        }
        // We don't have the variants for this enum
        Some(WorkflowEnumData { .. }) => {
            enum_creation_dialog.update(ctx, |dialog, ctx| {
                dialog.load_from_cloud_model(*id, ctx);
            });
            true
        }
        _ => {
            report_error!("Attempting to select an enum that cannot be found");
            false
        }
    }
}
