use crate::cloud_object::{Owner, Space};

mod embedded_fuzzy_match;
mod notebooks;
pub mod searcher;
pub mod view;
mod workflows;

/// Tests if an object owned by `object_owner` is accessible to all users with permissions to
/// `embedding_space`.
fn is_embed_accessible(embedding_space: Space, object_owner: Owner) -> bool {
    match (embedding_space, object_owner) {
        // If embedding in a personal object, _all_ objects accessible to the client are visible.
        (Space::Personal, _) => true,
        // TODO: Revisit the UX here, as the user doesn't know who else can see the object.
        (Space::Shared, _) => false,
        (
            Space::Team {
                team_uid: notebook_team_uid,
                ..
            },
            Owner::Team {
                team_uid: workflow_team_uid,
                ..
            },
        ) => notebook_team_uid == workflow_team_uid,
        // Private objects will not be accessible to all members of a team.
        (Space::Team { .. }, Owner::User { .. }) => false,
    }
}
