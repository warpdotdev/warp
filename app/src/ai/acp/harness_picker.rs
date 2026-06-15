use std::collections::HashMap;

use settings::Setting;
use warp_cli::agent::Harness;
use warp_core::report_if_error;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::{models, registry, telemetry};
use crate::ai::cloud_agent_settings::CloudAgentSettings;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LocalAcpHarnessModelEvent {
    SelectionChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LocalAcpModelDiscoveryStatus {
    Idle,
    Loading,
    Loaded,
    Failed(String),
}

pub(crate) struct LocalAcpHarnessModel {
    selected_harness: Harness,
    selected_model_id: Option<String>,
    discovered_models: HashMap<Harness, Vec<models::LocalAcpModelInfo>>,
    model_discovery_status: HashMap<Harness, LocalAcpModelDiscoveryStatus>,
}

impl LocalAcpHarnessModel {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        let settings = CloudAgentSettings::as_ref(ctx);
        let selected_harness = settings
            .last_selected_harness
            .value()
            .as_deref()
            .and_then(Harness::from_config_name)
            .filter(|harness| registry::is_local_acp_harness(*harness))
            .unwrap_or(Harness::Claude);
        let selected_model_id = saved_model_id_for_harness(settings, selected_harness);

        Self {
            selected_harness,
            selected_model_id,
            discovered_models: HashMap::new(),
            model_discovery_status: HashMap::new(),
        }
    }

    pub(crate) fn selected_harness(&self) -> Harness {
        self.selected_harness
    }

    pub(crate) fn selected_model_id(&self) -> Option<&str> {
        self.selected_model_id.as_deref()
    }

    pub(crate) fn selected_model_id_owned(&self) -> Option<String> {
        self.selected_model_id.clone()
    }

    pub(crate) fn selected_model_label(&self) -> String {
        self.selected_model_id
            .clone()
            .unwrap_or_else(|| "Default".to_string())
    }

    pub(crate) fn models_for_harness(&self, harness: Harness) -> Vec<models::LocalAcpModelInfo> {
        self.discovered_models
            .get(&harness)
            .filter(|models| !models.is_empty())
            .cloned()
            .unwrap_or_else(|| models::default_models_for_harness(harness))
    }

    pub(crate) fn model_discovery_status(&self, harness: Harness) -> LocalAcpModelDiscoveryStatus {
        self.model_discovery_status
            .get(&harness)
            .cloned()
            .unwrap_or(LocalAcpModelDiscoveryStatus::Idle)
    }

    pub(crate) fn select_harness(&mut self, harness: Harness, ctx: &mut ModelContext<Self>) {
        if self.selected_harness == harness {
            return;
        }
        self.selected_harness = harness;
        self.selected_model_id =
            saved_model_id_for_harness(CloudAgentSettings::as_ref(ctx), harness);
        CloudAgentSettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings
                .last_selected_harness
                .set_value(Some(harness.config_name().to_string()), ctx));
        });
        telemetry::record_local_acp_harness_selected(self.selected_harness);
        self.ensure_models_discovered(harness, ctx);
        ctx.emit(LocalAcpHarnessModelEvent::SelectionChanged);
    }

    pub(crate) fn select_model_id(
        &mut self,
        model_id: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.selected_model_id == model_id {
            return;
        }
        self.selected_model_id = model_id.clone();
        let harness = self.selected_harness;
        CloudAgentSettings::handle(ctx).update(ctx, |settings, ctx| {
            settings.persist_harness_model_selection(
                harness,
                model_id.as_deref().unwrap_or(""),
                None,
                ctx,
            );
        });
        ctx.emit(LocalAcpHarnessModelEvent::SelectionChanged);
    }

    pub(crate) fn ensure_all_models_discovered(&mut self, ctx: &mut ModelContext<Self>) {
        for spec in registry::agent_specs() {
            self.ensure_models_discovered(spec.harness, ctx);
        }
    }

    pub(crate) fn ensure_models_discovered(&mut self, harness: Harness, ctx: &mut ModelContext<Self>) {
        match self.model_discovery_status(harness) {
            LocalAcpModelDiscoveryStatus::Loading | LocalAcpModelDiscoveryStatus::Loaded => return,
            LocalAcpModelDiscoveryStatus::Failed(_) if harness != self.selected_harness => return,
            LocalAcpModelDiscoveryStatus::Idle | LocalAcpModelDiscoveryStatus::Failed(_) => {}
        }

        self.begin_model_discovery(harness, ctx);
        ctx.spawn(
            async move { models::discover_models_for_harness(harness).await },
            move |model, result, ctx| match result {
                Ok(discovered_models) => {
                    model.set_discovered_models(harness, discovered_models, ctx);
                }
                Err(error) => {
                    log::debug!("Failed to discover ACP models for {harness}: {error:#}");
                    model.set_model_discovery_failed(harness, error.to_string(), ctx);
                }
            },
        );
    }

    pub(crate) fn begin_model_discovery(&mut self, harness: Harness, ctx: &mut ModelContext<Self>) {
        if self.model_discovery_status.get(&harness) == Some(&LocalAcpModelDiscoveryStatus::Loading)
        {
            return;
        }
        self.model_discovery_status
            .insert(harness, LocalAcpModelDiscoveryStatus::Loading);
        if self.selected_harness == harness {
            ctx.emit(LocalAcpHarnessModelEvent::SelectionChanged);
        }
    }

    pub(crate) fn set_discovered_models(
        &mut self,
        harness: Harness,
        discovered_models: Vec<models::LocalAcpModelInfo>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.model_discovery_status
            .insert(harness, LocalAcpModelDiscoveryStatus::Loaded);
        self.discovered_models.insert(harness, discovered_models);
        if self.selected_harness == harness {
            ctx.emit(LocalAcpHarnessModelEvent::SelectionChanged);
        }
    }

    pub(crate) fn set_model_discovery_failed(
        &mut self,
        harness: Harness,
        error: String,
        ctx: &mut ModelContext<Self>,
    ) {
        self.model_discovery_status
            .insert(harness, LocalAcpModelDiscoveryStatus::Failed(error));
        if self.selected_harness == harness {
            ctx.emit(LocalAcpHarnessModelEvent::SelectionChanged);
        }
    }
}

fn saved_model_id_for_harness(
    settings: &CloudAgentSettings,
    harness: Harness,
) -> Option<String> {
    settings
        .last_selected_harness_model
        .value()
        .get(harness.config_name())
        .map(|selection| selection.model_id.clone())
        .filter(|model_id: &String| !model_id.is_empty())
}

impl Entity for LocalAcpHarnessModel {
    type Event = LocalAcpHarnessModelEvent;
}

impl SingletonEntity for LocalAcpHarnessModel {}
