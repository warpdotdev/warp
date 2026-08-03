use super::SecureStorage;
use crate::secure_storage::{Error, SecureStorage as _};

#[test]
fn read_value_returns_not_found() {
    let storage = SecureStorage;

    assert!(matches!(storage.read_value("key"), Err(Error::NotFound)));
}

#[test]
fn write_value_is_discarded() {
    let storage = SecureStorage;

    storage.write_value("key", "value").expect("write succeeds");

    assert!(matches!(storage.read_value("key"), Err(Error::NotFound)));
}

#[test]
fn remove_value_succeeds() {
    let storage = SecureStorage;

    storage.remove_value("key").expect("remove succeeds");
}
