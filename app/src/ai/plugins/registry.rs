//! The active plugin set and the discovery kill switch.
//!
//! The registry is deliberately free of `AppContext`, watchers, and MCP plumbing: turning
//! discovery off has to do two separate jobs — stop discovering, and tear down what is already
//! live — and the ordering between them is the part most likely to be subtly wrong. Keeping it
//! here means that ordering is directly testable rather than only reachable through the toggle.
use std::collections::BTreeMap;

use ai::plugins::{
    ActivePluginSet, PluginComponentId, PluginDiagnostic, PluginDiagnosticCode, PluginPackage,
    PluginSkillComponent, split_qualified_name,
};

/// Why plugin discovery is on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginDiscoveryPolicy {
    /// An interactive client, following the user's `Agent Plugin discovery` preference.
    InteractivePreference,
    /// A Factory runtime. Plugin discovery is part of the applied Factory definition, so a
    /// requester's or service account's personal preference is never consulted.
    RequiredByFactory,
}

impl PluginDiscoveryPolicy {
    /// Resolves the effective enablement, given the interactive preference.
    pub fn is_enabled(self, interactive_preference: bool) -> bool {
        match self {
            PluginDiscoveryPolicy::InteractivePreference => interactive_preference,
            PluginDiscoveryPolicy::RequiredByFactory => true,
        }
    }
}

/// One effect an enabled-to-disabled transition produces, in the order it must happen.
///
/// The registry stops answering lookups before it emits any of these, so nothing can register a
/// new component while the teardown is in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginTeardownStep {
    /// Stop filesystem watchers and invalidate outstanding parse generations.
    StopWatchers,
    /// Publish an empty plugin-skill generation, removing plugin skills from the model catalog
    /// and the explicit invocation resolver.
    WithdrawSkills,
    /// Cancel in-flight plugin MCP tool calls with the discovery-disabled diagnostic.
    CancelInFlightMcpCalls,
    /// Stop and unregister every plugin-provenance MCP installation.
    UnregisterMcpInstallations { components: Vec<PluginComponentId> },
}

/// What a change to the discovery state requires of the rest of the client.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginStateTransition {
    /// Ordered teardown steps for an enabled-to-disabled transition.
    pub teardown: Vec<PluginTeardownStep>,
    /// Whether a complete fresh rescan is required, as on a disabled-to-enabled transition.
    pub rescan: bool,
}

impl PluginStateTransition {
    pub fn is_noop(&self) -> bool {
        self.teardown.is_empty() && !self.rescan
    }
}

/// The active plugin packages, plus everything needed to reject work while discovery is off.
#[derive(Debug)]
pub struct PluginRegistry {
    enabled: bool,
    /// Monotonic parse generation. A scan result tagged with a superseded generation is dropped,
    /// so a slow parse cannot overwrite newer state — including the empty state that disabling
    /// discovery just published.
    generation: u64,
    active: BTreeMap<String, PluginPackage>,
    diagnostics: Vec<PluginDiagnostic>,
}

impl PluginRegistry {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            generation: 0,
            active: BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Starts a new scan and returns the generation its result must be tagged with.
    pub fn begin_scan(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    /// The current generation, which is the only one [`apply_scan`](Self::apply_scan) accepts.
    pub fn current_generation(&self) -> u64 {
        self.generation
    }

    /// Adopts a scan result, if it is still current and discovery is still on.
    ///
    /// Returns whether the active set changed.
    pub fn apply_scan(&mut self, generation: u64, resolved: ActivePluginSet) -> bool {
        if !self.enabled || generation != self.generation {
            return false;
        }
        self.diagnostics = resolved.all_diagnostics();
        self.active = resolved.active;
        true
    }

    /// Applies a new effective enablement and reports what the rest of the client must do.
    ///
    /// Disabling drops the active set before returning, so any lookup racing the teardown
    /// already fails with the discovery-disabled diagnostic rather than resolving a component
    /// that is about to be stopped.
    pub fn set_enabled(&mut self, enabled: bool) -> PluginStateTransition {
        if enabled == self.enabled {
            return PluginStateTransition::default();
        }
        self.enabled = enabled;

        if enabled {
            // Never revive a stale snapshot: a fresh scan is the only way back.
            self.generation += 1;
            return PluginStateTransition {
                teardown: Vec::new(),
                rescan: true,
            };
        }

        let components = self.active_mcp_component_ids();
        // Invalidate every outstanding parse so a scan that started while discovery was on
        // cannot land after the teardown.
        self.generation += 1;
        self.active.clear();
        self.diagnostics.clear();

        PluginStateTransition {
            teardown: vec![
                PluginTeardownStep::StopWatchers,
                PluginTeardownStep::WithdrawSkills,
                PluginTeardownStep::CancelInFlightMcpCalls,
                PluginTeardownStep::UnregisterMcpInstallations { components },
            ],
            rescan: false,
        }
    }

    /// Every active plugin skill, in a stable order.
    ///
    /// Empty while discovery is off, which is what the model catalog and the explicit invocation
    /// resolver observe.
    pub fn active_skills(&self) -> Vec<&PluginSkillComponent> {
        if !self.enabled {
            return Vec::new();
        }
        self.active
            .values()
            .flat_map(|package| package.skills.iter())
            .collect()
    }

    /// The component ids of every active plugin MCP server, in a stable order.
    pub fn active_mcp_component_ids(&self) -> Vec<PluginComponentId> {
        self.active
            .values()
            .flat_map(|package| {
                package
                    .mcp_servers
                    .iter()
                    .map(|server| package.mcp_component_id(&server.name))
            })
            .collect()
    }

    pub fn active_packages(&self) -> impl Iterator<Item = &PluginPackage> {
        self.active.values()
    }

    pub fn diagnostics(&self) -> &[PluginDiagnostic] {
        &self.diagnostics
    }

    /// Resolves a skill name that may or may not be plugin-qualified.
    ///
    /// `flat_names` are the names of the non-plugin skills currently in scope. An unqualified
    /// name resolves only when exactly one active skill — flat or plugin — has it, so a plugin
    /// can never silently take over a name a flat skill already answers to.
    pub fn resolve_skill(
        &self,
        name: &str,
        flat_names: &[String],
    ) -> Result<&PluginSkillComponent, PluginDiagnostic> {
        if !self.enabled {
            return Err(PluginDiagnostic::new(
                PluginDiagnosticCode::DiscoveryDisabled,
                format!(
                    "'{name}' is provided by an Agent Plugin, and Agent Plugin discovery is \
                     turned off"
                ),
            ));
        }

        if let Some((plugin, component)) = split_qualified_name(name) {
            return self
                .active
                .get(plugin)
                .and_then(|package| {
                    package
                        .skills
                        .iter()
                        .find(|skill| skill.id.local_name == component)
                })
                .ok_or_else(|| {
                    PluginDiagnostic::new(
                        PluginDiagnosticCode::SkillInvalid,
                        format!("no active plugin skill named '{name}'"),
                    )
                    .with_plugin(plugin)
                    .with_component(component)
                });
        }

        let matches: Vec<&PluginSkillComponent> = self
            .active_skills()
            .into_iter()
            .filter(|skill| skill.id.local_name == name)
            .collect();
        let flat_matches = flat_names.iter().filter(|flat| *flat == name).count();

        match (matches.len(), flat_matches) {
            (1, 0) => Ok(matches[0]),
            (0, _) => Err(PluginDiagnostic::new(
                PluginDiagnosticCode::SkillInvalid,
                format!("no active plugin skill named '{name}'"),
            )),
            _ => {
                let mut candidates: Vec<String> =
                    matches.iter().map(|skill| skill.qualified_name()).collect();
                if flat_matches > 0 {
                    candidates.push(name.to_owned());
                }
                Err(PluginDiagnostic::new(
                    PluginDiagnosticCode::ComponentAmbiguous,
                    format!(
                        "'{name}' is ambiguous; use one of: {}",
                        candidates.join(", ")
                    ),
                ))
            }
        }
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
