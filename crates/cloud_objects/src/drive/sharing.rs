use std::str::FromStr;

use serde::{Deserialize, Serialize};
use session_sharing_protocol::common::{ProfileData as SessionSharingProfileData, Role};
use warp_graphql::object_permissions::AccessLevel;

use crate::{auth::UserUid, cloud_object::Owner, ids::ServerId};

#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SharingAccessLevel {
    View,
    Edit,
    Full,
}

impl SharingAccessLevel {
    pub fn label(&self) -> &'static str {
        match self {
            SharingAccessLevel::View => "Can view",
            SharingAccessLevel::Edit => "Can edit",
            SharingAccessLevel::Full => "Full access",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            SharingAccessLevel::View => "view",
            SharingAccessLevel::Edit => "edit",
            SharingAccessLevel::Full => "access",
        }
    }

    /// Whether or not this access level implies the `Trash` action.
    pub fn can_trash(self) -> bool {
        self >= SharingAccessLevel::Edit
    }

    /// Whether or not this access level implies the `DeletePermanently` action.
    pub fn can_delete(self) -> bool {
        self >= SharingAccessLevel::Full
    }

    /// Whether or not this access level implies the `ChangeOwner` action.
    pub fn can_move_drive(self) -> bool {
        self >= SharingAccessLevel::Full
    }

    /// Whether or not this access level implies the `EditAccess` action.
    pub fn can_edit_access(self) -> bool {
        self >= SharingAccessLevel::Full
    }

    /// Convert this access level to a serializable value, which can be parsed by [`FromStr`].
    pub fn to_serializable_value(self) -> &'static str {
        match self {
            SharingAccessLevel::View => "VIEW",
            SharingAccessLevel::Edit => "EDIT",
            SharingAccessLevel::Full => "FULL",
        }
    }
}

impl FromStr for SharingAccessLevel {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "VIEW" => Ok(Self::View),
            "EDIT" => Ok(Self::Edit),
            "FULL" => Ok(Self::Full),
            _ => Err(anyhow::anyhow!("unknown access level {value}")),
        }
    }
}

impl From<AccessLevel> for SharingAccessLevel {
    fn from(server_access: AccessLevel) -> Self {
        match server_access {
            AccessLevel::Viewer => Self::View,
            AccessLevel::Editor => Self::Edit,
            AccessLevel::Full => Self::Full,
        }
    }
}

impl From<SharingAccessLevel> for AccessLevel {
    fn from(val: SharingAccessLevel) -> Self {
        match val {
            SharingAccessLevel::View => AccessLevel::Viewer,
            SharingAccessLevel::Edit => AccessLevel::Editor,
            SharingAccessLevel::Full => AccessLevel::Full,
        }
    }
}

impl From<Role> for SharingAccessLevel {
    fn from(role: Role) -> Self {
        match role {
            Role::Reader => Self::View,
            Role::Executor => Self::Edit,
            Role::Full => Self::Full,
        }
    }
}

impl From<SharingAccessLevel> for Role {
    fn from(access_level: SharingAccessLevel) -> Self {
        match access_level {
            SharingAccessLevel::View => Self::Reader,
            SharingAccessLevel::Edit | SharingAccessLevel::Full => Self::Executor,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LinkSharingSubjectType {
    None,
    Anyone,
}
/// A `Subject` is someone with access to a shared object, like its owner or a directly-added
/// guest.
#[derive(Debug, Clone, PartialEq)]
pub enum Subject {
    User(UserKind),
    #[allow(dead_code)]
    PendingUser {
        email: Option<String>,
    },
    Team(TeamKind),
    AnyoneWithLink(LinkSharingSubjectType),
}

/// A kind of user. In all cases, there is an underlying Warp account, but it's represented
/// differently in certain cases.
#[derive(Debug, Clone)]
pub enum UserKind {
    /// A Warp user account, tracked in the [`UserProfiles`] model.
    Account(UserUid),
    /// A session-sharing participant.
    // TODO(CLD-2283): Remove this once we have Firebase UIDs for shared session participants.
    SharedSessionParticipant(SessionSharingProfileData),
}

/// A kind of team. Team permission updates are propagated differently for
/// shared sessions, so we need to store different info in certain cases.
#[derive(Debug, Clone, PartialEq)]
pub enum TeamKind {
    Team {
        team_uid: ServerId,
    },
    /// The team of the shared session sharer.
    SharedSessionTeam {
        team_uid: ServerId,
        name: String,
    },
}

impl TeamKind {
    /// Gets the team UID.
    pub fn team_uid(&self) -> ServerId {
        match self {
            TeamKind::Team { team_uid } => *team_uid,
            TeamKind::SharedSessionTeam { team_uid, .. } => *team_uid,
        }
    }
}

impl Subject {
    /// Convert an [`Owner`] into the closest [`Subject`] type.
    pub fn from_owner(owner: Owner) -> Self {
        match owner {
            Owner::User { user_uid } => Subject::User(UserKind::Account(user_uid)),
            Owner::Team { team_uid } => Subject::Team(TeamKind::Team { team_uid }),
        }
    }

    /// Gets the user UID for this subject, if it has one.
    pub fn user_uid(&self) -> Option<UserUid> {
        match self {
            Subject::User(user_kind) => match user_kind {
                UserKind::Account(user_uid) => Some(*user_uid),
                UserKind::SharedSessionParticipant(profile_data) => {
                    Some(UserUid::new(profile_data.firebase_uid.as_str()))
                }
            },
            Subject::PendingUser { .. } => None,
            Subject::Team(_) => None,
            Subject::AnyoneWithLink(_) => None,
        }
    }

    /// Checks if this subject refers to a given Firebase user directly.
    pub fn is_user(&self, other_uid: UserUid) -> bool {
        match self {
            Subject::User(UserKind::Account(user_uid)) => *user_uid == other_uid,
            Subject::User(UserKind::SharedSessionParticipant(profile_data)) => {
                profile_data.firebase_uid.as_str() == other_uid.as_str()
            }
            _ => false,
        }
    }

    /// Gets the team UID for this subject, if it has one.
    pub fn team_uid(&self) -> Option<ServerId> {
        match self {
            Subject::Team(team_kind) => Some(team_kind.team_uid()),
            _ => None,
        }
    }
}

impl PartialEq for UserKind {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Account(self_uid), Self::Account(other_uid)) => self_uid == other_uid,
            // Shared session participant data does not implement `PartialEq`. We only compare
            // `UserKind`s in tests, so support isn't yet needed.
            _ => false,
        }
    }
}
