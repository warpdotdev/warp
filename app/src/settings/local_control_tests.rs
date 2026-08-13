use std::collections::HashMap;
use std::sync::Mutex;

use settings::{PrivatePreferences, PublicPreferences, Setting as _, SettingsManager, SyncToCloud};
use warp_core::channel::{Channel, ChannelState};
use warpui::SingletonEntity as _;
use warpui_extras::secure_storage::{self, AppContextExt as _};
use warpui_extras::user_preferences;

use super::{
    LocalControlMode, LocalControlModeSetting, LocalControlSettings, default_mode_for_channel,
};

#[derive(Default)]
struct InMemorySecureStorage {
    values: Mutex<HashMap<String, String>>,
}

impl secure_storage::SecureStorage for InMemorySecureStorage {
    fn write_value(&self, key: &str, value: &str) -> Result<(), secure_storage::Error> {
        match self.values.lock() {
            Ok(mut values) => {
                values.insert(key.to_owned(), value.to_owned());
                Ok(())
            }
            Err(err) => Err(secure_storage::Error::Unknown(anyhow::anyhow!(
                err.to_string()
            ))),
        }
    }

    fn read_value(&self, key: &str) -> Result<String, secure_storage::Error> {
        match self.values.lock() {
            Ok(values) => values
                .get(key)
                .cloned()
                .ok_or(secure_storage::Error::NotFound),
            Err(err) => Err(secure_storage::Error::Unknown(anyhow::anyhow!(
                err.to_string()
            ))),
        }
    }

    fn remove_value(&self, key: &str) -> Result<(), secure_storage::Error> {
        match self.values.lock() {
            Ok(mut values) => {
                values.remove(key);
                Ok(())
            }
            Err(err) => Err(secure_storage::Error::Unknown(anyhow::anyhow!(
                err.to_string()
            ))),
        }
    }
}

fn default_settings() -> LocalControlSettings {
    LocalControlSettings {
        local_control_mode: LocalControlModeSetting::new(None),
    }
}

#[test]
fn default_mode_is_enabled_only_on_dogfood_channels() {
    assert_eq!(
        default_mode_for_channel(Channel::Dev),
        LocalControlMode::Enabled
    );
    assert_eq!(
        default_mode_for_channel(Channel::Local),
        LocalControlMode::Enabled
    );
    for channel in [
        Channel::Stable,
        Channel::Preview,
        Channel::Oss,
        Channel::Integration,
    ] {
        assert_eq!(
            default_mode_for_channel(channel),
            LocalControlMode::Disabled,
            "{channel} must require explicit opt-in"
        );
    }
}

#[test]
fn unset_mode_follows_channel_default() {
    let settings = default_settings();

    assert_eq!(LocalControlMode::default(), LocalControlMode::Disabled);
    assert_eq!(
        settings.mode(),
        default_mode_for_channel(ChannelState::channel())
    );
}

#[test]
fn mode_is_persisted_to_secure_storage() {
    warpui::App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| {
                PublicPreferences::new(
                    Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
                )
            });
            ctx.add_singleton_model(|_| {
                PrivatePreferences::new(
                    Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
                )
            });
            ctx.add_singleton_model(|_| SettingsManager::default());
            ctx.add_singleton_model(|_| -> secure_storage::Model {
                Box::<InMemorySecureStorage>::default()
            });
            LocalControlSettings::register(ctx);
        });

        app.update(|ctx| {
            LocalControlSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .local_control_mode
                    .set_value(LocalControlMode::Enabled, ctx)
            })
        })
        .expect("setting update should succeed");

        app.read(|ctx| {
            let stored = ctx
                .secure_storage()
                .read_value(LocalControlModeSetting::storage_key())
                .expect("local-control mode should be stored securely");
            let mode = serde_json::from_str::<LocalControlMode>(&stored)
                .expect("stored local-control mode should deserialize");
            assert_eq!(mode, LocalControlMode::Enabled);

            let private_value = LocalControlModeSetting::preferences_for_setting(ctx)
                .read_value(LocalControlModeSetting::storage_key())
                .expect("private preferences should be readable");
            assert!(private_value.is_none());
        });
    });
}

#[test]
fn mode_does_not_migrate_from_private_preferences() {
    warpui::App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| {
                PublicPreferences::new(
                    Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
                )
            });
            ctx.add_singleton_model(|_| {
                PrivatePreferences::new(
                    Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
                )
            });
            ctx.add_singleton_model(|_| SettingsManager::default());
            ctx.add_singleton_model(|_| -> secure_storage::Model {
                Box::<InMemorySecureStorage>::default()
            });
            LocalControlModeSetting::preferences_for_setting(ctx)
                .write_value(
                    LocalControlModeSetting::storage_key(),
                    serde_json::to_string(&LocalControlMode::Enabled).expect("mode serializes"),
                )
                .expect("private preference is writable");
            LocalControlSettings::register(ctx);
        });

        app.read(|ctx| {
            assert_eq!(
                LocalControlSettings::as_ref(ctx).mode(),
                default_mode_for_channel(ChannelState::channel())
            );
            let private_value = LocalControlModeSetting::preferences_for_setting(ctx)
                .read_value(LocalControlModeSetting::storage_key())
                .expect("private preference is readable");
            assert!(private_value.is_some());
        });
    });
}
#[test]
fn mode_is_private_and_never_cloud_synced() {
    assert_eq!(LocalControlModeSetting::sync_to_cloud(), SyncToCloud::Never);
    assert!(LocalControlModeSetting::is_private());
}

#[test]
fn cloud_sync_cannot_disable_local_control() {
    warpui::App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| {
                PublicPreferences::new(
                    Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
                )
            });
            ctx.add_singleton_model(|_| {
                PrivatePreferences::new(
                    Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
                )
            });
            ctx.add_singleton_model(|_| SettingsManager::default());
            ctx.add_singleton_model(|_| -> secure_storage::Model {
                Box::<InMemorySecureStorage>::default()
            });
            LocalControlSettings::register(ctx);
        });

        app.update(|ctx| {
            LocalControlSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .local_control_mode
                    .set_value(LocalControlMode::Enabled, ctx)
            })
        })
        .expect("local control should enable");

        app.update(|ctx| {
            LocalControlSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .local_control_mode
                    .set_value_from_cloud_sync(LocalControlMode::Disabled, ctx)
            })
        })
        .expect("cloud sync update should be ignored without error");

        app.read(|ctx| {
            let settings = LocalControlSettings::as_ref(ctx);
            assert_eq!(settings.mode(), LocalControlMode::Enabled);
            let stored = ctx
                .secure_storage()
                .read_value(LocalControlModeSetting::storage_key())
                .expect("explicitly enabled mode should remain stored securely");
            let mode = serde_json::from_str::<LocalControlMode>(&stored)
                .expect("stored local-control mode should deserialize");
            assert_eq!(mode, LocalControlMode::Enabled);
        });
    });
}
