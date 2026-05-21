use warp_graphql::queries::get_updated_cloud_objects::UpdatedObjectInput;

use super::RevisionAndLastEditor;

/// Result of attempting to update a cloud object.
#[derive(Debug)]
pub enum UpdateCloudObjectResult<T> {
    /// The update was successful and the object now has the specified revision.
    Success {
        revision_and_editor: RevisionAndLastEditor,
    },
    /// The update was rejected because the update was not sent from the current revision in
    /// storage. The object and revision in storage are returned.
    Rejected { object: T },
}

/// Helper struct that contains all the info needed to fetch changed objects from the server.
#[derive(Default)]
pub struct ObjectsToUpdate {
    pub notebooks: Vec<UpdatedObjectInput>,
    pub workflows: Vec<UpdatedObjectInput>,
    pub folders: Vec<UpdatedObjectInput>,
    pub generic_string_objects: Vec<UpdatedObjectInput>,
}

impl Clone for ObjectsToUpdate {
    fn clone(&self) -> Self {
        Self {
            notebooks: self
                .notebooks
                .iter()
                .map(copy_updated_object_input)
                .collect(),
            workflows: self
                .workflows
                .iter()
                .map(copy_updated_object_input)
                .collect(),
            folders: self.folders.iter().map(copy_updated_object_input).collect(),
            generic_string_objects: self
                .generic_string_objects
                .iter()
                .map(copy_updated_object_input)
                .collect(),
        }
    }
}

fn copy_updated_object_input(input: &UpdatedObjectInput) -> UpdatedObjectInput {
    UpdatedObjectInput {
        uid: input.uid.clone(),
        actions_ts: input.actions_ts,
        metadata_ts: input.metadata_ts,
        permissions_ts: input.permissions_ts,
        revision_ts: input.revision_ts,
    }
}
