use super::*;

#[test]
fn secret_value_debug_is_redacted() {
    let secret = SecretValue::new("hunter2-super-secret");
    let debug = format!("{secret:?}");
    assert_eq!(debug, "SecretValue(***redacted***)");
    assert!(!debug.contains("hunter2"));
}

#[test]
fn secret_value_redacted_inside_containing_struct() {
    #[derive(Debug)]
    #[allow(dead_code)]
    struct Holder {
        name: String,
        secret: SecretValue,
    }
    let holder = Holder {
        name: "db".to_owned(),
        secret: SecretValue::new("p@ssw0rd"),
    };
    let debug = format!("{holder:?}");
    assert!(debug.contains("db"));
    assert!(!debug.contains("p@ssw0rd"));
    assert!(debug.contains("***redacted***"));
}

#[test]
fn secret_value_exposes_plaintext_only_on_request() {
    let secret = SecretValue::new("abc123");
    assert_eq!(secret.expose_secret(), "abc123");
    assert!(!secret.is_empty());
    assert!(SecretValue::new("").is_empty());
}

#[test]
fn in_memory_store_round_trips() {
    let store = InMemorySecretStore::new();
    assert!(store.is_empty());

    store.write_value("k1", "v1").unwrap();
    assert!(store.contains_key("k1"));
    assert_eq!(store.read_value("k1").unwrap(), "v1");
    assert_eq!(store.len(), 1);

    store.remove_value("k1").unwrap();
    assert!(!store.contains_key("k1"));
    assert!(matches!(
        store.read_value("k1"),
        Err(SecretStoreError::NotFound)
    ));
}

#[test]
fn in_memory_store_remove_missing_is_ok() {
    let store = InMemorySecretStore::new();
    assert!(store.remove_value("missing").is_ok());
}

#[test]
fn owner_only_fallback_defaults_to_write_value() {
    let store = InMemorySecretStore::new();
    store
        .write_value_with_owner_only_fallback("k", "v")
        .unwrap();
    assert_eq!(store.read_value("k").unwrap(), "v");
}

#[test]
fn unavailable_store_disables_writes_and_reads() {
    let store = UnavailableSecretStore;
    assert!(matches!(
        store.write_value("k", "v"),
        Err(SecretStoreError::Unavailable)
    ));
    assert!(matches!(
        store.read_value("k"),
        Err(SecretStoreError::NotFound)
    ));
    // Removal is a no-op success since nothing is persisted.
    assert!(store.remove_value("k").is_ok());
}
