use cloud_objects::auth::UserUid;
use cloud_objects::cloud_object::{CloudObjectGuest, ServerObjectContainer};
use cloud_objects::drive::sharing::{
    LinkSharingSubjectType, SharingAccessLevel, Subject, TeamKind, UserKind,
};
use cloud_objects::ids::ServerId;
use lazy_static::lazy_static;
use session_sharing_protocol::common::{InputReplicaId, ProfileData};

#[test]
fn test_roundtrip_guests() {
    let guests = vec![
        CloudObjectGuest {
            subject: Subject::User(UserKind::Account(UserUid::new("firebase_uid"))),
            access_level: SharingAccessLevel::Edit,
            source: None,
        },
        CloudObjectGuest {
            subject: Subject::PendingUser {
                email: Some("pending@warp.dev".to_string()),
            },
            access_level: SharingAccessLevel::View,
            source: Some(ServerObjectContainer::Folder {
                folder_uid: 123.into(),
            }),
        },
        CloudObjectGuest {
            subject: Subject::Team(TeamKind::Team {
                team_uid: ServerId::from(99),
            }),
            access_level: SharingAccessLevel::Edit,
            source: None,
        },
    ];

    let encoded = super::encode_guests(&guests).expect("encode should succeed");
    let decoded = super::decode_guests(&encoded).expect("decode should succeed");

    assert_eq!(guests, decoded);
}

lazy_static! {
    /// By construction, [`CloudObjectGuest`] only accepts `'static`-lifetime [`Subject`]s.
    ///
    /// In most cases, this would prevent persisting a shared session subject, but we work around
    /// it here for completeness;
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
    let result = super::encode_guests(&[CloudObjectGuest {
        subject: Subject::AnyoneWithLink(LinkSharingSubjectType::Anyone),
        access_level: SharingAccessLevel::View,
        source: None,
    }]);
    assert!(result.is_err());

    let result = super::encode_guests(&[CloudObjectGuest {
        subject: Subject::User(UserKind::SharedSessionParticipant(PROFILE_DATA.clone())),
        access_level: SharingAccessLevel::View,
        source: None,
    }]);
    assert!(result.is_err());
}
