use ai::api_keys::{
    ApiKeyManager, CustomEndpointDefinition, CustomEndpointDefinitions, CustomEndpointId,
    CustomEndpointModel, CustomEndpointParams,
};
use settings::Setting;
use uuid::Uuid;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::LaunchMode;
use crate::global_resource_handles::GlobalResourceHandlesProvider;
use crate::settings::cloud_preferences_syncer::{
    CloudPreferencesSyncer, CloudPreferencesSyncerEvent,
};
use crate::settings::{AISettings, AISettingsChangedEvent, SettingsFileError};
use crate::user_config::{WarpConfig, WarpConfigUpdateEvent};

const CUSTOM_ENDPOINTS_TOML_KEY: &str = "custom_endpoints";

struct CustomEndpointSettingsModel {
    imports_legacy_endpoints: bool,
    settings_invalid: bool,
}

impl CustomEndpointSettingsModel {
    fn new(launch_mode: &LaunchMode, ctx: &mut ModelContext<Self>) -> Self {
        let settings_invalid = GlobalResourceHandlesProvider::as_ref(ctx)
            .get()
            .settings_file_error
            .as_ref()
            .is_some_and(settings_error_affects_custom_endpoints);
        ctx.subscribe_to_model(&AISettings::handle(ctx), |model, _, event, ctx| {
            if matches!(event, AISettingsChangedEvent::CustomEndpoints { .. }) {
                model.sync_or_migrate(ctx);
            }
        });
        ctx.subscribe_to_model(
            &WarpConfig::handle(ctx),
            |model, _, event, ctx| match event {
                WarpConfigUpdateEvent::SettingsErrors(error) => {
                    model.settings_invalid = settings_error_affects_custom_endpoints(error);
                    model.sync_or_migrate(ctx);
                }
                WarpConfigUpdateEvent::SettingsErrorsCleared => {
                    model.settings_invalid = false;
                    model.sync_or_migrate(ctx);
                }
                WarpConfigUpdateEvent::Themes
                | WarpConfigUpdateEvent::LocalUserWorkflows
                | WarpConfigUpdateEvent::LaunchConfigs
                | WarpConfigUpdateEvent::TabConfigs
                | WarpConfigUpdateEvent::TabConfigErrors(_)
                | WarpConfigUpdateEvent::ModelConfigs
                | WarpConfigUpdateEvent::ModelConfigErrors(_)
                | WarpConfigUpdateEvent::Settings => {}
            },
        );
        ctx.subscribe_to_model(
            &CloudPreferencesSyncer::handle(ctx),
            |model, _, event, ctx| {
                if matches!(event, CloudPreferencesSyncerEvent::InitialLoadCompleted) {
                    model.sync_or_migrate(ctx);
                }
            },
        );
        let mut model = Self {
            imports_legacy_endpoints: matches!(
                launch_mode,
                LaunchMode::App { .. } | LaunchMode::Test { .. }
            ),
            settings_invalid,
        };
        model.sync_or_migrate(ctx);
        model
    }

    fn sync_or_migrate(&mut self, ctx: &mut ModelContext<Self>) {
        if self.settings_invalid {
            ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                manager.invalidate_custom_endpoint_definitions(ctx);
            });
            return;
        }
        let setting = &AISettings::as_ref(ctx).custom_endpoints;
        if setting.is_value_explicitly_set() || !self.imports_legacy_endpoints {
            set_active_definitions(setting.value().clone(), ctx);
            return;
        }
        if !CloudPreferencesSyncer::as_ref(ctx).has_completed_initial_load() {
            return;
        }
        let legacy = ApiKeyManager::as_ref(ctx).keys().custom_endpoints.clone();
        if legacy.is_empty() {
            set_active_definitions(CustomEndpointDefinitions::default(), ctx);
            return;
        }
        let Ok((definitions, keys)) = CustomEndpointDefinitions::from_legacy(&legacy) else {
            log::warn!("Could not migrate invalid legacy custom endpoint definitions");
            set_active_definitions(CustomEndpointDefinitions::default(), ctx);
            return;
        };
        let keys_result = ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.persist_custom_endpoint_keys(keys, ctx)
        });
        if let Err(error) = keys_result {
            log::warn!("Could not migrate custom endpoint keys: {error:#}");
            return;
        }
        if let Err(error) = write_definitions(definitions.clone(), ctx) {
            log::warn!("Could not migrate custom endpoint definitions: {error:#}");
            return;
        }
        set_active_definitions(definitions, ctx);
    }
}

impl Entity for CustomEndpointSettingsModel {
    type Event = ();
}

impl SingletonEntity for CustomEndpointSettingsModel {}

pub(crate) fn init(launch_mode: &LaunchMode, ctx: &mut AppContext) {
    ctx.add_singleton_model(|ctx| CustomEndpointSettingsModel::new(launch_mode, ctx));
}

pub(crate) fn add(params: CustomEndpointParams, ctx: &mut AppContext) -> anyhow::Result<usize> {
    let mut definitions = AISettings::as_ref(ctx).custom_endpoints.value().clone();
    let id = CustomEndpointId::generated();
    let key = params.api_key.clone();
    definitions.insert(id.clone(), definition_from_params(params))?;
    ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
        manager.persist_custom_endpoint_key(id.clone(), Some(key), ctx)
    })?;
    if let Err(error) = write_definitions(definitions.clone(), ctx) {
        let _ = ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.persist_custom_endpoint_key(id, None, ctx)
        });
        return Err(error);
    }
    set_active_definitions(definitions, ctx);
    Ok(AISettings::as_ref(ctx).custom_endpoints.value().len() - 1)
}

pub(crate) fn save(
    index: usize,
    params: CustomEndpointParams,
    ctx: &mut AppContext,
) -> anyhow::Result<()> {
    let mut definitions = AISettings::as_ref(ctx).custom_endpoints.value().clone();
    let id = definitions
        .id_at(index)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("custom endpoint index is out of bounds"))?;
    definitions.insert(id.clone(), definition_from_params(params.clone()))?;
    let old_key = ApiKeyManager::as_ref(ctx)
        .custom_endpoint_key(&id)
        .map(ToOwned::to_owned);
    ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
        manager.persist_custom_endpoint_key(id.clone(), Some(params.api_key), ctx)
    })?;
    if let Err(error) = write_definitions(definitions.clone(), ctx) {
        let _ = ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.persist_custom_endpoint_key(id, old_key, ctx)
        });
        return Err(error);
    }
    set_active_definitions(definitions, ctx);
    Ok(())
}

pub(crate) fn remove(index: usize, ctx: &mut AppContext) -> anyhow::Result<()> {
    let mut definitions = AISettings::as_ref(ctx).custom_endpoints.value().clone();
    let id = definitions
        .id_at(index)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("custom endpoint index is out of bounds"))?;
    definitions.remove(&id);
    write_definitions(definitions.clone(), ctx)?;
    set_active_definitions(definitions, ctx);
    let _ = ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
        manager.persist_custom_endpoint_key(id, None, ctx)
    });
    Ok(())
}

fn definition_from_params(params: CustomEndpointParams) -> CustomEndpointDefinition {
    CustomEndpointDefinition {
        name: params.name,
        base_url: params.url,
        schema: params.schema,
        models: params
            .models
            .into_iter()
            .map(|(name, alias, config_key)| CustomEndpointModel {
                name,
                alias,
                config_key: config_key
                    .filter(|key| !key.trim().is_empty())
                    .unwrap_or_else(|| Uuid::new_v4().to_string()),
            })
            .collect(),
    }
}

fn write_definitions(
    definitions: CustomEndpointDefinitions,
    ctx: &mut AppContext,
) -> anyhow::Result<()> {
    AISettings::handle(ctx).update(ctx, |settings, ctx| {
        settings.custom_endpoints.set_value(definitions, ctx)
    })
}

fn set_active_definitions(definitions: CustomEndpointDefinitions, ctx: &mut AppContext) {
    ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
        manager.set_custom_endpoint_definitions(definitions, ctx);
    });
}

fn settings_error_affects_custom_endpoints(error: &SettingsFileError) -> bool {
    match error {
        SettingsFileError::FileParseFailed(_) => true,
        SettingsFileError::InvalidSettings(keys) => {
            keys.iter().any(|key| key == CUSTOM_ENDPOINTS_TOML_KEY)
        }
    }
}

#[cfg(test)]
#[path = "custom_endpoints_tests.rs"]
mod tests;
