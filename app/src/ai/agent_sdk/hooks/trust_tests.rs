use std::fs;

use sha2::{Digest as _, Sha256};

use super::*;

fn trust_key(root: &Path, config_path: &Path) -> HookTrustKey {
    HookTrustKey {
        git_root: fs::canonicalize(root).unwrap(),
        config_path: fs::canonicalize(config_path).unwrap(),
        definition_hash: hex::encode(Sha256::digest(fs::read(config_path).unwrap())),
    }
}

#[test]
fn oz_hooks_persistent_trust_round_trips_and_revokes_exact_definition() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let config_dir = project.join(".warp");
    let config_path = config_dir.join("hooks.json");
    let store_path = temp.path().join("trust.json");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(&config_path, b"trusted definition").unwrap();
    let key = trust_key(&project, &config_path);
    let store = PersistentHookTrustStore::load(store_path.clone()).unwrap();

    store.trust(key.clone()).unwrap();
    assert!(store.is_trusted(&key));
    assert!(
        PersistentHookTrustStore::load(store_path.clone())
            .unwrap()
            .is_trusted(&key)
    );

    fs::write(&config_path, b"trusted definition\n").unwrap();
    let changed_key = trust_key(&project, &config_path);
    assert!(
        !PersistentHookTrustStore::load(store_path.clone())
            .unwrap()
            .is_trusted(&changed_key)
    );

    store.revoke(&key).unwrap();
    assert!(
        !PersistentHookTrustStore::load(store_path)
            .unwrap()
            .is_trusted(&key)
    );
}

#[test]
fn oz_hooks_persistent_trust_rejects_unknown_schema_fields_and_invalid_hashes() {
    let temp = tempfile::tempdir().unwrap();
    let store_path = temp.path().join("trust.json");
    let cases = [
        serde_json::json!({
            "schema_version": TRUST_STORE_SCHEMA_VERSION,
            "records": [],
            "unknown": true
        }),
        serde_json::json!({
            "schema_version": "future",
            "records": []
        }),
        serde_json::json!({
            "schema_version": TRUST_STORE_SCHEMA_VERSION,
            "records": [{
                "git_root": "/project",
                "config_path": "/project/.warp/hooks.json",
                "definition_hash": "ABC"
            }]
        }),
    ];

    for contents in cases {
        fs::write(&store_path, serde_json::to_vec(&contents).unwrap()).unwrap();
        assert!(PersistentHookTrustStore::load(store_path.clone()).is_err());
    }
}

#[test]
fn oz_hooks_identifies_only_the_host_trust_store_path() {
    assert!(is_hook_trust_store_path(Path::new(
        "/home/user/.warp/oz-hook-trust.json"
    )));
    assert!(!is_hook_trust_store_path(Path::new(
        "/project/.warp/hooks.json"
    )));
    assert!(!is_hook_trust_store_path(Path::new(
        "/project/oz-hook-trust.json"
    )));
}
