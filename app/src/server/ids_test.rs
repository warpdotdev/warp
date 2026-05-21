use warp_server_client::ids::{ClientId, SyncId};

#[test]
pub fn test_client_sync_id_serialization() {
    let id: SyncId = SyncId::ClientId(ClientId::new());
    let serialized = serde_json::to_string(&id).expect("failed to serialize");
    assert_eq!(serialized, format!("\"{}\"", id.uid()));
    let deserialized: SyncId =
        serde_json::from_str(serialized.as_str()).expect("failed to deserialize");
    assert_eq!(id, deserialized);
}

#[test]
pub fn test_legacy_sync_id_uid_serialization() {
    let id = SyncId::LegacyObjectId(String::from("Ymgrzu0nh2HwDNeYEtXF1x"));
    let serialized = serde_json::to_string(&id).expect("failed to serialize");
    assert_eq!(
        serialized,
        format!("\"{}\"", String::from("Ymgrzu0nh2HwDNeYEtXF1x"))
    );
    let deserialized: SyncId =
        serde_json::from_str(serialized.as_str()).expect("failed to deserialize");
    assert_eq!(id, deserialized);
}

#[test]
pub fn test_non_22_character_legacy_sync_id_deserialization_does_not_panic() {
    let deserialized: SyncId =
        serde_json::from_str("\"legacy-workflow-id\"").expect("failed to deserialize");
    assert_eq!(
        deserialized,
        SyncId::LegacyObjectId(String::from("legacy-workflow-id"))
    );
}
