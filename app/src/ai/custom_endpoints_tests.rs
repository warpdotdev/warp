use std::cell::RefCell;
use std::collections::HashMap;

use ai::api_keys::{ApiKeyManager, CustomEndpointPersistenceMode};
use warpui_core::App;

use super::{CustomEndpointDefinitionsCoordinator as _, CustomEndpointSource};
use crate::settings::AISettings;
use crate::settings::manager::SettingsManager;
use crate::user_config::WarpConfig;

const CUSTOM_ENDPOINT_API_KEYS_STORAGE_KEY: &str = "CustomEndpointApiKeys";

/// A faithful in-memory `SecureStorage` double that actually round-trips
/// values, unlike `register_noop` (which discards writes).
#[derive(Default)]
struct FakeSecureStorage {
    values: RefCell<HashMap<String, String>>,
}

impl warpui_extras::secure_storage::SecureStorage for FakeSecureStorage {
    fn write_value(
        &self,
        key: &str,
        value: &str,
    ) -> Result<(), warpui_extras::secure_storage::Error> {
        self.values
            .borrow_mut()
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn read_value(&self, key: &str) -> Result<String, warpui_extras::secure_storage::Error> {
        self.values
            .borrow()
            .get(key)
            .cloned()
            .ok_or(warpui_extras::secure_storage::Error::NotFound)
    }

    fn remove_value(&self, key: &str) -> Result<(), warpui_extras::secure_storage::Error> {
        self.values.borrow_mut().remove(key);
        Ok(())
    }
}

fn register_test_settings(app: &mut App) {
    app.update(crate::settings::init_and_register_user_preferences);
    app.add_singleton_model(|_| SettingsManager::default());
    app.add_singleton_model(WarpConfig::mock);
    app.update(|ctx| {
        ctx.add_singleton_model(|_| -> warpui_extras::secure_storage::Model {
            Box::new(FakeSecureStorage::default())
        });
    });
    app.update(AISettings::register_and_subscribe_to_events);
}

/// Seeds a leftover `CustomEndpointApiKeys` entry, simulating a key stored by
/// a previous run before the user deleted its endpoint definition.
fn seed_leftover_key(app: &mut App) {
    app.update(|ctx| {
        use warpui_extras::secure_storage::AppContextExt as _;
        ctx.secure_storage()
            .write_value(
                CUSTOM_ENDPOINT_API_KEYS_STORAGE_KEY,
                r#"{"Acme":"sk-acme"}"#,
            )
            .expect("seeding the fake secure storage should succeed");
    });
}

fn stored_custom_endpoint_keys(app: &App) -> String {
    app.read(|ctx| {
        use warpui_extras::secure_storage::AppContextExt as _;
        ctx.secure_storage()
            .read_value(CUSTOM_ENDPOINT_API_KEYS_STORAGE_KEY)
            .unwrap_or_default()
    })
}

// ── APP-5380 review finding #2 ──────────────────────────────────

#[test]
fn restart_after_deleting_final_definition_prunes_the_orphaned_key() {
    App::test((), |mut app| async move {
        register_test_settings(&mut app);
        seed_leftover_key(&mut app);

        // `custom_endpoints` is absent (its natural, never-explicitly-set
        // default) -- the user deleted their last endpoint and restarted --
        // but the settings file otherwise parsed successfully this launch.
        let manager = app.add_singleton_model(|ctx| {
            let mut manager = ApiKeyManager::new(CustomEndpointPersistenceMode::Split, ctx);
            manager.subscribe_to_custom_endpoint_definitions(
                CustomEndpointSource::SettingsCollection,
                true,
                ctx,
            );
            manager
        });

        manager.read(&app, |manager, _| {
            assert!(!manager.custom_endpoint_key_is_connected("Acme"));
        });
        assert!(
            !stored_custom_endpoint_keys(&app).contains("Acme"),
            "a successful parse with an absent custom_endpoints must prune the orphaned key"
        );
    });
}

#[test]
fn startup_toml_parse_failure_does_not_prune_stored_keys() {
    App::test((), |mut app| async move {
        register_test_settings(&mut app);
        seed_leftover_key(&mut app);

        // Same absent `custom_endpoints`, but this time the file failed to
        // parse at all this launch: `AISettings` is on a cached/default
        // snapshot that doesn't reflect the user's actual file, so
        // reconciliation must be skipped rather than treating "absent" as
        // authoritative and wrongly orphan-cleaning every stored key.
        let manager = app.add_singleton_model(|ctx| {
            let mut manager = ApiKeyManager::new(CustomEndpointPersistenceMode::Split, ctx);
            manager.subscribe_to_custom_endpoint_definitions(
                CustomEndpointSource::SettingsCollection,
                false,
                ctx,
            );
            manager
        });

        // With `custom_endpoints` itself unset (the whole file failed to
        // parse, so there is no trustworthy definition to join against
        // either), the effective projection has nothing to connect — the
        // important guarantee is that the *stored* secret survives on disk,
        // ready to reconnect once the user fixes the file and restarts.
        manager.read(&app, |manager, _| {
            assert!(!manager.custom_endpoint_key_is_connected("Acme"));
        });
        assert!(
            stored_custom_endpoint_keys(&app).contains("Acme"),
            "a full-file parse failure must not orphan-clean stored keys"
        );
    });
}
