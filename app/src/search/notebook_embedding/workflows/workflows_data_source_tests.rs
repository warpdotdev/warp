use crate::cloud_object::{Owner, Space};
use crate::search::notebook_embedding::is_embed_accessible;
use crate::server::ids::ServerId;

#[test]
fn test_embed_in_personal_object() {
    assert!(is_embed_accessible(
        Space::Personal,
        Owner::mock_current_user()
    ));
    assert!(is_embed_accessible(
        Space::Personal,
        Owner::Team {
            team_uid: ServerId::from(123),
        }
    ));
}

#[test]
fn test_embed_in_team_object() {
    // Private objects are not team-visible.
    assert!(!is_embed_accessible(
        Space::Team {
            team_uid: ServerId::from(123)
        },
        Owner::mock_current_user()
    ));
    // Objects in another team are not visible.
    assert!(!is_embed_accessible(
        Space::Team {
            team_uid: ServerId::from(123)
        },
        Owner::Team {
            team_uid: ServerId::from(456)
        }
    ));
    // Objects from the same team are visible.
    assert!(is_embed_accessible(
        Space::Team {
            team_uid: ServerId::from(123),
        },
        Owner::Team {
            team_uid: ServerId::from(123),
        }
    ));
}
