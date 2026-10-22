//! Derives editable keybindings from the user's saved launch configurations and tab configs.
//!
//! Every loaded config registers one binding — `launch_config:open:<name>` /
//! `tab_config:open:<file stem>`, the same identifiers the `warp://launch` and
//! `warp://tab_config` URIs accept — with no default keystroke. Users assign keys in
//! Settings → Keyboard Shortcuts or `keybindings.yaml` like any other action. Bindings are
//! re-registered whenever the configs reload from disk; custom triggers assigned before a
//! binding registers (configs load asynchronously after `keybindings.yaml` is applied) still
//! take effect because the keymap remembers triggers by name.

use std::collections::HashSet;

use warp_core::context_flag::ContextFlag;
use warpui::keymap::{BindingDescription, EditableBinding};
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use crate::features::FeatureFlag;
use crate::launch_configs::launch_config::LaunchConfig;
use crate::settings_view::keybindings::{KeybindingChangedEvent, KeybindingChangedNotifier};
use crate::tab_configs::TabConfig;
use crate::user_config::{WarpConfig, WarpConfigUpdateEvent};
use crate::util::bindings::BindingGroup;
use crate::workspace::WorkspaceAction;

/// Name prefix for bindings that open a launch configuration.
pub const LAUNCH_CONFIG_BINDING_PREFIX: &str = "launch_config:open:";
/// Name prefix for bindings that open a tab config.
pub const TAB_CONFIG_BINDING_PREFIX: &str = "tab_config:open:";

pub fn init(app: &mut AppContext) {
    app.add_singleton_model(ConfigKeybindings::new);
}

/// Singleton that keeps the keymap's config-derived bindings in sync with [`WarpConfig`].
pub struct ConfigKeybindings {}

impl Entity for ConfigKeybindings {
    type Event = ();
}

impl SingletonEntity for ConfigKeybindings {}

impl ConfigKeybindings {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&WarpConfig::handle(ctx), Self::handle_config_event);
        Self::register_bindings(ctx);
        Self {}
    }

    fn handle_config_event(
        &mut self,
        _: ModelHandle<WarpConfig>,
        event: &WarpConfigUpdateEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        if matches!(
            event,
            WarpConfigUpdateEvent::LaunchConfigs | WarpConfigUpdateEvent::TabConfigs
        ) {
            Self::register_bindings(ctx);
        }
    }

    fn register_bindings(ctx: &mut ModelContext<Self>) {
        let (launch_bindings, tab_bindings) = if FeatureFlag::ConfigKeybindings.is_enabled() {
            let warp_config_handle = WarpConfig::handle(ctx);
            let warp_config = warp_config_handle.as_ref(ctx);
            (
                launch_config_bindings(warp_config.launch_configs()),
                tab_config_bindings(warp_config.tab_configs()),
            )
        } else {
            (Vec::new(), Vec::new())
        };

        ctx.replace_editable_bindings_with_prefix(LAUNCH_CONFIG_BINDING_PREFIX, launch_bindings);
        ctx.replace_editable_bindings_with_prefix(TAB_CONFIG_BINDING_PREFIX, tab_bindings);

        KeybindingChangedNotifier::handle(ctx).update(ctx, |_me, ctx| {
            ctx.emit(KeybindingChangedEvent::BindingsReloaded);
        });
    }
}

/// Builds one binding per launch configuration, keyed by the config's `name` — the identifier
/// `warp://launch/<name>` accepts. Duplicate names (case-insensitive) keep the first loaded
/// config, matching URI resolution.
fn launch_config_bindings(configs: &[LaunchConfig]) -> Vec<EditableBinding> {
    use warpui::keymap::macros::*;

    let mut seen = HashSet::new();
    configs
        .iter()
        .filter(|config| {
            let is_new = seen.insert(config.name.to_lowercase());
            if !is_new {
                log::warn!(
                    "duplicate launch configuration name '{}'; only the first gets a keybinding",
                    config.name
                );
            }
            is_new
        })
        .map(|config| {
            EditableBinding::new(
                format!("{LAUNCH_CONFIG_BINDING_PREFIX}{}", config.name),
                BindingDescription::new_preserve_case(format!(
                    "Open Launch Configuration \"{}\"",
                    config.name
                )),
                WorkspaceAction::OpenLaunchConfigNamed {
                    name: config.name.clone(),
                },
            )
            .with_context_predicate(id!("Workspace") & !id!("Workspace_PaneDragging"))
            .with_group(BindingGroup::LaunchConfigurations.as_str())
            .with_enabled(|| ContextFlag::LaunchConfigurations.is_enabled())
        })
        .collect()
}

/// Builds one binding per tab config, keyed by the config file's stem — the identifier
/// `warp://tab_config/<stem>` accepts. Configs without a UTF-8 file stem are skipped;
/// duplicate stems (case-insensitive) keep the first loaded config.
fn tab_config_bindings(configs: &[TabConfig]) -> Vec<EditableBinding> {
    use warpui::keymap::macros::*;

    let mut seen = HashSet::new();
    configs
        .iter()
        .filter_map(|config| {
            let Some(stem) = config.file_stem() else {
                log::warn!(
                    "tab config '{}' has no UTF-8 file stem; skipping keybinding",
                    config.name
                );
                return None;
            };
            if !seen.insert(stem.to_lowercase()) {
                log::warn!(
                    "duplicate tab config file stem '{stem}'; only the first gets a keybinding"
                );
                return None;
            }
            Some(
                EditableBinding::new(
                    format!("{TAB_CONFIG_BINDING_PREFIX}{stem}"),
                    BindingDescription::new_preserve_case(format!(
                        "Open Tab Config \"{}\"",
                        config.name
                    )),
                    WorkspaceAction::OpenTabConfigNamed {
                        file_stem: stem.to_string(),
                    },
                )
                .with_context_predicate(id!("Workspace") & !id!("Workspace_PaneDragging"))
                .with_group(BindingGroup::TabConfigs.as_str())
                .with_enabled(|| FeatureFlag::TabConfigs.is_enabled()),
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "config_keybindings_tests.rs"]
mod tests;
