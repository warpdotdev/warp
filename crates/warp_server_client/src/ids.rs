use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// Convert ID enums into and from a hashed UUID.
pub trait HashableId: Sized + Send + Sync {
    fn to_hash(&self) -> String;
    fn from_hash(hash: &str) -> Option<Self>;
}

/// Local object id categories kept for backwards-compatible SQLite ids.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ObjectIdType {
    Notebook,
    Workflow,
    Folder,
    GenericStringObject,
}

impl ObjectIdType {
    /// Returns the legacy prefix used for object ids stored in SQLite.
    pub fn sqlite_prefix(&self) -> &'static str {
        match self {
            ObjectIdType::Notebook => "Notebook",
            ObjectIdType::Workflow => "Workflow",
            ObjectIdType::Folder => "Folder",
            ObjectIdType::GenericStringObject => "GenericStringObject",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(description = "A client-generated unique identifier.")]
pub struct ClientId(Uuid);

impl HashableId for ClientId {
    fn to_hash(&self) -> String {
        self.to_string()
    }

    fn from_hash(hash: &str) -> Option<ClientId> {
        hash.strip_prefix("Client-")
            .and_then(|s| Uuid::parse_str(s).ok())
            .map(ClientId)
    }
}

impl ClientId {
    pub fn new() -> ClientId {
        Self(Uuid::new_v4())
    }

    pub fn sqlite_hash(&self) -> String {
        self.to_string()
    }
}

impl Default for ClientId {
    fn default() -> Self {
        ClientId::new()
    }
}

impl fmt::Display for ClientId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Client-{}", self.0)
    }
}

impl From<String> for ClientId {
    fn from(s: String) -> Self {
        ClientId::from_hash(&s).unwrap_or_default()
    }
}

/// ID of an object in the sync queue.
#[derive(Clone, Debug, Hash, PartialEq, Eq, schemars::JsonSchema)]
#[schemars(description = "Identifier for a synced object.")]
pub enum SyncId {
    /// Item has not been sync-ed yet. Using a client-created UUID.
    #[schemars(
        description = "A locally-generated identifier for an object not yet synced to the server."
    )]
    ClientId(ClientId),
    /// Legacy persisted object ID from the removed cloud sync model.
    #[schemars(description = "A legacy persisted object identifier.")]
    LegacyObjectId(ObjectUid),
}

impl SyncId {
    pub fn uid(&self) -> ObjectUid {
        match self {
            Self::ClientId(id) => id.to_string(),
            Self::LegacyObjectId(id) => id.clone(),
        }
    }

    pub fn sqlite_uid_hash(&self, object_id_type: ObjectIdType) -> String {
        match self {
            SyncId::ClientId(id) => id.sqlite_hash(),
            SyncId::LegacyObjectId(id) => {
                format!("{}-{}", object_id_type.sqlite_prefix(), id)
            }
        }
    }

    pub fn into_client(self) -> Option<ClientId> {
        match self {
            Self::LegacyObjectId(_) => None,
            Self::ClientId(id) => Some(id),
        }
    }
}

impl settings_value::SettingsValue for SyncId {}

impl fmt::Display for SyncId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::LegacyObjectId(id) => id.fmt(f),
            Self::ClientId(id) => id.fmt(f),
        }
    }
}

impl From<String> for SyncId {
    fn from(id: String) -> SyncId {
        if let Some(client_id) = ClientId::from_hash(&id) {
            SyncId::ClientId(client_id)
        } else {
            SyncId::LegacyObjectId(id)
        }
    }
}

/// Custom serialize function for SyncIds.
impl Serialize for SyncId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            SyncId::LegacyObjectId(id) => id.serialize(serializer),
            SyncId::ClientId(client_id) => client_id.to_hash().serialize(serializer),
        }
    }
}

/// Custom deserialize function for SyncIds.
impl<'de> Deserialize<'de> for SyncId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;

        // Client IDs are prefixed with `Client-`. Other strings are legacy object IDs
        // from the removed server sync model and should not be length-normalized.
        if let Some(hashed) = ClientId::from_hash(s.as_str()) {
            Ok(SyncId::ClientId(hashed))
        } else {
            Ok(SyncId::LegacyObjectId(s))
        }
    }
}

/// For server IDs, this is the value that is stored
/// in the database. For client IDs, it is of the form "Client-{id}".
/// Used to index into cloud model and in most object read, write, and metadata
/// mutation server APIs.
pub type ObjectUid = String;

/// Corresponds to what is stored for a given object id within the local sqlite
/// database. Needed for backwards compatibility of the sqlite db following a refactor
/// that stripped the object type away from SyncID.
///
/// Of the format {sqlite_prefix}-{uid}.
///
/// Other than sqlite model events, this id is used for embedded objects within notebooks.
pub type HashedSqliteId = String;

/// UID for API keys.
pub type ApiKeyUid = String;

/// Removes the prefix from sqlite IDs to extract the UIDs. Should not be used unless there
/// is not other way to cleanly do the conversion, i.e., when we don't know the ID type.
#[allow(clippy::result_unit_err)]
pub fn parse_sqlite_id_to_uid(hashed_sqlite_id: HashedSqliteId) -> Result<ObjectUid, ()> {
    let Some(uid) = hashed_sqlite_id.split("-").last() else {
        return Err(());
    };

    Ok(uid.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{ClientId, SyncId};

    #[test]
    fn legacy_sync_id_deserializes_without_fixed_length_requirement() {
        let deserialized: SyncId =
            serde_json::from_str("\"legacy-workflow-id\"").expect("sync id should deserialize");

        assert_eq!(
            deserialized,
            SyncId::LegacyObjectId(String::from("legacy-workflow-id"))
        );
    }

    #[test]
    fn client_sync_id_still_round_trips_as_a_client_prefixed_string() {
        let id = SyncId::ClientId(ClientId::new());
        let serialized = serde_json::to_string(&id).expect("sync id should serialize");
        let deserialized: SyncId =
            serde_json::from_str(&serialized).expect("sync id should deserialize");

        assert_eq!(deserialized, id);
    }
}
