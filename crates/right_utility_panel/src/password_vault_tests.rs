use super::*;
use crate::model::RightUtilityPanelData;
use crate::secret::{
    InMemorySecretStore, SecretResult, SecretStore, SecretStoreError, SecretValue,
    UnavailableSecretStore,
};

const SECRET: &str = "correct-horse-battery-staple";

fn new_password(title: &str, secret: &str) -> NewPassword {
    let mut input = NewPassword::new(title.to_owned(), SecretValue::new(secret));
    input.username = "alice".to_owned();
    input.url = "https://example.com".to_owned();
    input
}

#[test]
fn create_writes_secret_and_records_metadata() {
    let vault = PasswordVault::new(InMemorySecretStore::new());
    let mut data = RightUtilityPanelData::new();

    let id = vault.create(&mut data, new_password("DB", SECRET)).unwrap();

    assert_eq!(data.passwords.len(), 1);
    let meta = data.password(id).unwrap();
    assert_eq!(meta.title, "DB");
    // The secret lives in the store under the metadata's key.
    assert!(vault.store().contains_key(&meta.secret_storage_key));
    assert_eq!(vault.store().len(), 1);
}

#[test]
fn secret_key_is_derived_from_uuid_not_title() {
    let vault = PasswordVault::new(InMemorySecretStore::new());
    let mut data = RightUtilityPanelData::new();
    let id = vault
        .create(&mut data, new_password("My Bank Login", SECRET))
        .unwrap();
    let meta = data.password(id).unwrap();
    assert!(meta.secret_storage_key.contains(&id.to_string()));
    assert!(!meta.secret_storage_key.contains("My Bank Login"));
}

#[test]
fn metadata_serialization_never_contains_secret() {
    let vault = PasswordVault::new(InMemorySecretStore::new());
    let mut data = RightUtilityPanelData::new();
    vault.create(&mut data, new_password("DB", SECRET)).unwrap();

    let json = serde_json::to_string(&data).unwrap();
    assert!(!json.contains(SECRET), "serialized data leaked the secret");
    // Non-secret metadata is present.
    assert!(json.contains("DB"));
    assert!(json.contains("secret_storage_key"));
}

#[test]
fn reveal_returns_stored_secret() {
    let vault = PasswordVault::new(InMemorySecretStore::new());
    let mut data = RightUtilityPanelData::new();
    let id = vault.create(&mut data, new_password("DB", SECRET)).unwrap();

    let revealed = vault.reveal(data.password(id).unwrap()).unwrap();
    assert_eq!(revealed.expose_secret(), SECRET);
}

#[test]
fn update_secret_changes_stored_value() {
    let vault = PasswordVault::new(InMemorySecretStore::new());
    let mut data = RightUtilityPanelData::new();
    let id = vault.create(&mut data, new_password("DB", SECRET)).unwrap();
    let created_updated_at = data.password(id).unwrap().updated_at;

    vault
        .update_secret(&mut data, id, SecretValue::new("new-secret"))
        .unwrap();

    let revealed = vault.reveal(data.password(id).unwrap()).unwrap();
    assert_eq!(revealed.expose_secret(), "new-secret");
    assert!(data.password(id).unwrap().updated_at >= created_updated_at);
}

#[test]
fn delete_removes_secret_and_metadata() {
    let vault = PasswordVault::new(InMemorySecretStore::new());
    let mut data = RightUtilityPanelData::new();
    let id = vault.create(&mut data, new_password("DB", SECRET)).unwrap();

    let removed = vault.delete(&mut data, id).unwrap();
    assert!(removed.is_some());
    assert!(data.passwords.is_empty());
    assert!(vault.store().is_empty());
}

#[test]
fn delete_missing_id_is_ok_none() {
    let vault = PasswordVault::new(InMemorySecretStore::new());
    let mut data = RightUtilityPanelData::new();
    let result = vault.delete(&mut data, uuid::Uuid::new_v4()).unwrap();
    assert!(result.is_none());
}

#[test]
fn create_with_unavailable_storage_adds_no_metadata() {
    let vault = PasswordVault::new(UnavailableSecretStore);
    let mut data = RightUtilityPanelData::new();

    let result = vault.create(&mut data, new_password("DB", SECRET));
    assert!(matches!(result, Err(SecretStoreError::Unavailable)));
    // No dangling metadata that points at a missing secret.
    assert!(data.passwords.is_empty());
}

/// A store whose `remove_value` always fails, to exercise the delete-failure
/// path.
#[derive(Default)]
struct RemoveFailingStore {
    inner: InMemorySecretStore,
}

impl SecretStore for RemoveFailingStore {
    fn write_value(&self, key: &str, value: &str) -> SecretResult<()> {
        self.inner.write_value(key, value)
    }

    fn read_value(&self, key: &str) -> SecretResult<String> {
        self.inner.read_value(key)
    }

    fn remove_value(&self, _key: &str) -> SecretResult<()> {
        Err(SecretStoreError::Backend("remove failed".to_owned()))
    }
}

#[test]
fn delete_failure_retains_metadata_and_surfaces_error() {
    let vault = PasswordVault::new(RemoveFailingStore::default());
    let mut data = RightUtilityPanelData::new();
    let id = vault.create(&mut data, new_password("DB", SECRET)).unwrap();

    let result = vault.delete(&mut data, id);
    assert!(matches!(result, Err(SecretStoreError::Backend(_))));
    // We never pretend the secret was removed: metadata stays.
    assert_eq!(data.passwords.len(), 1);
}
