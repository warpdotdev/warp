//! Supporting types for persisting cloud objects to SQLite.

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use crate::auth::UserUid;
use crate::cloud_object::{CloudLinkSharing, CloudObjectGuest, ServerObjectContainer};
use crate::drive::sharing::{SharingAccessLevel, Subject, TeamKind, UserKind};
use crate::ids::ServerId;

/// Decode a link-sharing setting.
pub fn decode_link_sharing(
    encoded_access_level: &str,
    encoded_source: Option<&[u8]>,
) -> anyhow::Result<CloudLinkSharing> {
    let access_level = encoded_access_level.parse()?;
    let source = encoded_source.map(bincode::deserialize).transpose()?;
    Ok(CloudLinkSharing {
        access_level,
        source,
    })
}

/// Encode a link-sharing setting.
pub fn encode_link_sharing(
    link_sharing: &CloudLinkSharing,
) -> anyhow::Result<(&'static str, Option<Vec<u8>>)> {
    let source = link_sharing
        .source
        .as_ref()
        .map(bincode::serialize)
        .transpose()?;
    Ok((link_sharing.access_level.to_serializable_value(), source))
}

/// Deserialize encoded object guests.
pub fn decode_guests(encoded_guests: &[u8]) -> anyhow::Result<Vec<CloudObjectGuest>> {
    let persisted_guests = bincode::deserialize::<Vec<PersistedGuest>>(encoded_guests)?;
    Ok(persisted_guests
        .into_iter()
        .map(PersistedGuest::into_cloud_object_guest)
        .collect())
}

/// Encode object guests for persistence.
pub fn encode_guests(guests: &[CloudObjectGuest]) -> anyhow::Result<Vec<u8>> {
    let persisted_guests = guests
        .iter()
        .map(PersistedGuest::try_from_cloud_object_guest)
        .collect::<anyhow::Result<Vec<PersistedGuest>>>()?;
    Ok(bincode::serialize(&persisted_guests)?)
}

/// Database representation of an object guest. These are [`bincode`]-serialized to support storing
/// an arbitrarily-long guest list.
#[derive(Serialize, Deserialize)]
struct PersistedGuest {
    subject: PersistedSubject,
    access_level: SharingAccessLevel,
    source: Option<ServerObjectContainer>,
}

/// Database representation of a guest subject. This is restricted compared to the [`Subject`] type
/// since not all subjects are persisted.
#[derive(Serialize, Deserialize)]
enum PersistedSubject {
    User { firebase_uid: String },
    PendingUser { email: Option<String> },
    Team { team_uid: ServerId },
}

impl PersistedGuest {
    pub fn into_cloud_object_guest(self) -> CloudObjectGuest {
        CloudObjectGuest {
            subject: self.subject.into_subject(),
            access_level: self.access_level,
            source: self.source,
        }
    }

    pub fn try_from_cloud_object_guest(guest: &CloudObjectGuest) -> anyhow::Result<Self> {
        Ok(PersistedGuest {
            subject: PersistedSubject::try_from_subject(&guest.subject)?,
            access_level: guest.access_level,
            source: guest.source,
        })
    }
}

impl PersistedSubject {
    pub fn into_subject(self) -> Subject {
        match self {
            PersistedSubject::User { firebase_uid } => {
                Subject::User(UserKind::Account(UserUid::new(&firebase_uid)))
            }
            PersistedSubject::PendingUser { email } => Subject::PendingUser { email },
            PersistedSubject::Team { team_uid } => Subject::Team(TeamKind::Team { team_uid }),
        }
    }

    /// Convert a [`Subject`] into a guest subject type. This is only supported for subjects that
    /// may be direct object guests.
    pub fn try_from_subject(subject: &Subject) -> anyhow::Result<Self> {
        match subject {
            Subject::User(user_kind) => match user_kind {
                UserKind::Account(user_uid) => Ok(PersistedSubject::User {
                    firebase_uid: user_uid.to_string(),
                }),
                UserKind::SharedSessionParticipant(_) => {
                    // Shared sessions are transient, so we don't persist their ACLs to SQLite.
                    Err(anyhow!("Session-sharing participants not supported"))
                }
            },
            Subject::PendingUser { email } => Ok(PersistedSubject::PendingUser {
                email: email.clone(),
            }),
            Subject::Team(team_kind) => match team_kind {
                TeamKind::Team { team_uid } => Ok(PersistedSubject::Team {
                    team_uid: *team_uid,
                }),
                TeamKind::SharedSessionTeam { .. } => {
                    // Shared sessions are transient, so we don't persist their ACLs to SQLite.
                    Err(anyhow!("Session-sharing teams not supported"))
                }
            },
            // Link sharing is persisted separately in the schema.
            Subject::AnyoneWithLink(_) => Err(anyhow!("Anyone with the link not supported")),
        }
    }
}
