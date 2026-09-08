//! Shared display metadata for the compute a cloud run executes on.
//!
//! Any UI surface that shows a run's platform to the user — the conversation
//! details panel, the `runner` CLI listing, etc. — should source its labels and
//! icon from here so the surfaces cannot drift.

use std::collections::HashMap;

use warp_graphql::queries::get_runners::{
    Runner, RunnerArch, RunnerConfig, RunnerMacOsVersion, RunnerOs,
};

use crate::ui_components::icons::Icon;

/// User-visible label for a runner's operating system.
pub fn os_display(os: RunnerOs) -> &'static str {
    match os {
        RunnerOs::Linux => "Linux",
        RunnerOs::Macos => "macOS",
    }
}

/// User-visible label for a runner's CPU architecture.
pub fn arch_display(arch: RunnerArch) -> &'static str {
    match arch {
        RunnerArch::X8664 => "x86-64",
        RunnerArch::Aarch64 => "aarch64",
    }
}

/// User-visible label for a macOS runner's OS version.
pub fn macos_version_display(version: RunnerMacOsVersion) -> &'static str {
    match version {
        RunnerMacOsVersion::Macos14 => "macOS 14",
        RunnerMacOsVersion::Macos15 => "macOS 15",
        RunnerMacOsVersion::Macos26 => "macOS 26",
        RunnerMacOsVersion::Macos27 => "macOS 27",
    }
}

/// Leading icon for a runner's operating system.
pub fn icon_for(os: RunnerOs) -> Icon {
    match os {
        RunnerOs::Linux => Icon::Linux,
        RunnerOs::Macos => Icon::Apple,
    }
}

/// The display-ready subset of a runner's compute configuration.
///
/// Owned rather than borrowed from the GraphQL types so views can cache it
/// across renders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnerPlatform {
    pub os: RunnerOs,
    pub arch: RunnerArch,
    pub macos_version: Option<RunnerMacOsVersion>,
    pub name: String,
    /// vCPUs the runner pins, when it pins an instance shape.
    pub vcpus: Option<i32>,
    /// Memory in GB the runner pins, when it pins an instance shape.
    pub memory_gb: Option<i32>,
}

impl RunnerPlatform {
    pub fn from_config(config: &RunnerConfig) -> Self {
        Self {
            os: config.os,
            arch: config.arch,
            macos_version: config.mac.as_ref().and_then(|mac| mac.version),
            name: config.name.clone(),
            vcpus: config.instance_shape.as_ref().map(|shape| shape.vcpus),
            memory_gb: config.instance_shape.as_ref().map(|shape| shape.memory_gb),
        }
    }
}

/// Builds the runner lookup a view caches after fetching runners.
pub fn platforms_by_uid(runners: &[Runner]) -> HashMap<String, RunnerPlatform> {
    runners
        .iter()
        .map(|runner| {
            (
                runner.uid.inner().to_string(),
                RunnerPlatform::from_config(&runner.config),
            )
        })
        .collect()
}

/// The compute a run executes on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunPlatform {
    /// A stored runner supplies the run's compute.
    Runner(RunnerPlatform),
    /// Nothing references a runner, so the run gets Warp's default compute.
    /// This mirrors the server synthesizing x86-64 Linux from an environment's
    /// inline image.
    Default,
}

impl RunPlatform {
    pub fn icon(&self) -> Icon {
        match self {
            RunPlatform::Runner(platform) => icon_for(platform.os),
            RunPlatform::Default => Icon::Linux,
        }
    }

    /// One-line summary: OS, architecture, and whatever else the runner pins.
    pub fn summary(&self) -> String {
        let platform = match self {
            RunPlatform::Runner(platform) => platform,
            RunPlatform::Default => {
                return format!(
                    "{} · {}",
                    os_display(RunnerOs::Linux),
                    arch_display(RunnerArch::X8664)
                );
            }
        };

        let os_label = platform
            .macos_version
            .map(macos_version_display)
            .unwrap_or_else(|| os_display(platform.os));

        let mut parts = vec![
            os_label.to_string(),
            arch_display(platform.arch).to_string(),
        ];
        if !platform.name.is_empty() {
            parts.push(platform.name.clone());
        }
        if let Some(vcpus) = platform.vcpus {
            parts.push(format!("{vcpus} vCPU"));
        }
        if let Some(memory_gb) = platform.memory_gb {
            parts.push(format!("{memory_gb} GB"));
        }
        parts.join(" · ")
    }
}

/// Resolves the compute a run executes on, by the same precedence the server
/// uses: the runner the run names, then its environment's default runner, then
/// Warp's default compute.
///
/// Returns `None` when a runner is referenced but absent from `platforms` —
/// the runner may be inaccessible, deleted, or still loading, and guessing a
/// platform there would misreport where the run runs.
pub fn resolve_run_platform(
    runner_uid: Option<&str>,
    environment_default_runner_uid: Option<&str>,
    platforms: &HashMap<String, RunnerPlatform>,
) -> Option<RunPlatform> {
    let referenced = runner_uid
        .map(str::trim)
        .filter(|uid| !uid.is_empty())
        .or_else(|| {
            environment_default_runner_uid
                .map(str::trim)
                .filter(|uid| !uid.is_empty())
        });

    match referenced {
        Some(uid) => platforms.get(uid).cloned().map(RunPlatform::Runner),
        None => Some(RunPlatform::Default),
    }
}

#[cfg(test)]
#[path = "runner_display_tests.rs"]
mod tests;
