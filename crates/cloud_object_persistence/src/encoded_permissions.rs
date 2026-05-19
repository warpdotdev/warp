//! Supporting helpers for persisting cloud-object permissions to SQLite.

use anyhow::anyhow;
use cloud_objects::{
    auth::UserUid,
    cloud_object::{CloudLinkSharing, CloudObjectGuest, ServerObjectContainer},
    drive::sharing::{SharingAccessLevel, Subject, TeamKind, UserKind},
    ids::ServerId,
};
use serde::{Deserialize, Serialize};

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

/// Database representation of an object guest.
#[derive(Serialize, Deserialize)]
struct PersistedGuest {
    subject: PersistedSubject,
    access_level: SharingAccessLevel,
    source: Option<ServerObjectContainer>,
}

/// Database representation of a guest subject.
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

    /// Convert a [`Subject`] into a guest subject type.
    pub fn try_from_subject(subject: &Subject) -> anyhow::Result<Self> {
        match subject {
            Subject::User(user_kind) => match user_kind {
                UserKind::Account(user_uid) => Ok(PersistedSubject::User {
                    firebase_uid: user_uid.to_string(),
                }),
                UserKind::SharedSessionParticipant(_) => {
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
                    Err(anyhow!("Session-sharing teams not supported"))
                }
            },
            Subject::AnyoneWithLink(_) => Err(anyhow!("Anyone with the link not supported")),
        }
    }
}

#[cfg(test)]
mod tests {
    use cloud_objects::{
        cloud_object::{CloudObjectGuest, ServerObjectContainer},
        drive::sharing::{LinkSharingSubjectType, SharingAccessLevel, Subject, TeamKind, UserKind},
        ids::ServerId,
    };
    use lazy_static::lazy_static;
    use session_sharing_protocol::common::{InputReplicaId, ProfileData};

    use super::{decode_guests, encode_guests};

    #[test]
    fn test_roundtrip_guests() {
        let guests = vec![
            CloudObjectGuest {
                subject: Subject::User(UserKind::Account(cloud_objects::UserUid::new(
                    "firebase_uid",
                ))),
                access_level: SharingAccessLevel::Edit,
                source: None,
            },
            CloudObjectGuest {
                subject: Subject::PendingUser {
                    email: Some("pending@warp.dev".to_string()),
                },
                access_level: SharingAccessLevel::View,
                source: Some(ServerObjectContainer::Folder {
                    folder_uid: ServerId::from_string_lossy("1234567890123456789012"),
                }),
            },
            CloudObjectGuest {
                subject: Subject::Team(TeamKind::Team {
                    team_uid: ServerId::from_string_lossy("abcdefghijklmnopqrstuv"),
                }),
                access_level: SharingAccessLevel::Edit,
                source: None,
            },
        ];

        let encoded = encode_guests(&guests).expect("encode should succeed");
        let decoded = decode_guests(&encoded).expect("decode should succeed");

        assert_eq!(guests, decoded);
    }

    lazy_static! {
        /// By construction, [`CloudObjectGuest`] only accepts `'static`-lifetime [`Subject`]s.
        ///
        /// In most cases, this would prevent persisting a shared session subject, but this test
        /// works around it for completeness.
        static ref PROFILE_DATA: ProfileData = ProfileData {
            firebase_uid: "2YP93GScglXJMdEr2Id12dI7HCG3".to_string(),
            display_name: "Some User".to_string(),
            photo_url: Some("http://example.com/some-image".to_string()),
            email: Some("user@warp.dev".to_string()),
            input_replica_id: InputReplicaId::from("some-id".to_string()),
        };
    }

    #[test]
    fn test_fail_unsupported_subjects() {
        let result = encode_guests(&[CloudObjectGuest {
            subject: Subject::AnyoneWithLink(LinkSharingSubjectType::Anyone),
            access_level: SharingAccessLevel::View,
            source: None,
        }]);
        assert!(result.is_err());

        let result = encode_guests(&[CloudObjectGuest {
            subject: Subject::User(UserKind::SharedSessionParticipant(PROFILE_DATA.clone())),
            access_level: SharingAccessLevel::View,
            source: None,
        }]);
        assert!(result.is_err());
    }
}
