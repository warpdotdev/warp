use std::fs;
use std::path::{Path, PathBuf, absolute};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::Deserialize;
use typed_path::{WindowsComponent, WindowsPath};
use warp_cli::agent::Harness;
use warp_util::path::file_exists_and_is_executable;

use super::super::super::{AgentDriverError, HarnessSetupStage};
use super::super::ThirdPartyHarness;
use super::super::claude_code::ClaudeHarness;
use super::super::codex::CodexHarness;
use super::super::gemini::GeminiHarness;
use super::launch::AcpLaunchSpec;

const SIDECAR_ENV: &str = "WARP_HARNESS_SIDECAR";
const MANIFEST_FILE: &str = "warp-harness.json";

/// Unknown top-level sections are ignored so a sidecar can add ones for other transports
/// without breaking clients that only know about ACP; the sections consumed here are strict.
#[derive(Debug, Deserialize)]
struct Manifest {
    schema_version: u32,
    harness: String,
    acp: AcpLaunchSpec,
    #[serde(default)]
    auth: Auth,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Auth {
    check_command: Option<String>,
    #[serde(default)]
    runtime_error_patterns: Vec<String>,
}

/// How to launch and check a harness over ACP: either what its sidecar's `warp-harness.json`
/// declares, with the program resolved under the sidecar, or the built-in defaults used when
/// there is no sidecar and the adapter is expected on `PATH`. Anything that depends on the
/// sidecar's own layout is the launched program's business, not the client's.
pub(super) struct HarnessManifest {
    /// The preferred program and its arguments. The bridge launches the first of
    /// [`Self::program_candidates`] it finds on the terminal session's `PATH`, which may be a
    /// different name; this one is for logs and messages.
    pub(super) launch: AcpLaunchSpec,
    /// The built-in default this manifest came from; `None` for a sidecar, whose program was
    /// verified when the manifest was read.
    pub(super) default: Option<DefaultAdapter>,
    /// For a built-in default: whether the harness's sidecar directory was mounted without a
    /// manifest, which means the worker mounted the terminal CLI image for an ACP run. Always
    /// `false` for a sidecar.
    pub(super) sidecar_without_manifest: bool,
    pub(super) auth_check_command: Option<String>,
    pub(super) runtime_error_patterns: Vec<String>,
}

impl HarnessManifest {
    /// Program names in preference order. A sidecar has exactly one, already absolute.
    pub(super) fn program_candidates(&self) -> Vec<String> {
        match &self.default {
            Some(adapter) => adapter.programs().map(str::to_owned).collect(),
            None => vec![self.launch.program.clone()],
        }
    }
}

/// The built-in launch for a harness with no sidecar. The adapter is expected on `PATH` under
/// one of its names, which differ between where it came from: the bin an npm package installs
/// versus the launcher an agent package ships.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DefaultAdapter {
    /// The name tried first.
    pub(super) preferred: &'static str,
    /// Other names the adapter may be installed under, in preference order.
    pub(super) alternates: &'static [&'static str],
    args: &'static [&'static str],
    /// The npm package that provides the adapter, for install guidance.
    pub(super) package: Option<&'static str>,
    /// A CLI the adapter drives, which has to be installed as well.
    pub(super) wraps_cli: Option<&'static str>,
}

impl DefaultAdapter {
    /// Every name the adapter may be installed under, preferred first.
    pub(super) fn programs(&self) -> impl Iterator<Item = &'static str> + '_ {
        std::iter::once(self.preferred).chain(self.alternates.iter().copied())
    }

    /// Why the harness cannot start when none of [`Self::programs`] is installed. The adapter
    /// is launched through the terminal session's shell, so it has to be visible to that shell
    /// as well as to this process.
    pub(super) fn missing_reason(&self, install_docs_url: Option<&str>) -> String {
        let mut reason = if self.alternates.is_empty() {
            format!("ACP adapter '{}' is not on PATH.", self.preferred)
        } else {
            let names = self
                .programs()
                .map(|program| format!("'{program}'"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("No ACP adapter found: none of {names} is on PATH.")
        };
        if let Some(package) = self.package {
            reason.push_str(&format!(" Install it with `npm install -g {package}`."));
        }
        reason.push_str(
            " Make sure it is on PATH for both the oz process and the terminal session's \
             shell, which launches it.",
        );
        match (self.wraps_cli, install_docs_url) {
            (Some(cli), Some(url)) => {
                reason.push_str(&format!(" The '{cli}' CLI must be installed too: {url}"));
            }
            (None, Some(url)) => reason.push_str(&format!(" Install docs: {url}")),
            (_, None) => {}
        }
        reason
    }
}

pub(super) fn resolve(harness: Harness) -> Result<HarnessManifest, AgentDriverError> {
    let override_dir = std::env::var_os(SIDECAR_ENV).map(PathBuf::from);
    discover(harness, override_dir.as_deref(), Path::new("/mnt")).map_err(|error| {
        AgentDriverError::HarnessSetupFailed {
            harness: harness.to_string(),
            stage: Some(HarnessSetupStage::Manifest),
            reason: format!("{error:#}"),
        }
    })
}

/// Selects the sidecar manifest named by `override_dir`, else the one on the harness's mount,
/// else the built-in defaults. An override that is set but unusable is an error rather than a
/// fall-through.
fn discover(
    harness: Harness,
    override_dir: Option<&Path>,
    mounts_dir: &Path,
) -> Result<HarnessManifest> {
    let mount_dir = mounts_dir.join(format!("{harness}-cli-sidecar"));
    let mount_path = mount_dir.join(MANIFEST_FILE);
    let path = if let Some(dir) = override_dir {
        dir.join(MANIFEST_FILE)
    } else if mount_path
        .try_exists()
        .with_context(|| format!("Cannot inspect {}", mount_path.display()))?
    {
        mount_path
    } else {
        let sidecar_without_manifest = mount_dir.is_dir();
        if sidecar_without_manifest {
            log::info!(
                "{} has no ACP manifest; using built-in ACP defaults for {harness}",
                mount_dir.display()
            );
        } else {
            log::info!("Using built-in ACP defaults for {harness}");
        }
        return default_manifest(harness)
            .map(|manifest| HarnessManifest {
                sidecar_without_manifest,
                ..manifest
            })
            .with_context(|| format!("The {harness} harness cannot be driven over ACP."));
    };
    let dir = absolute(path.parent().context("Manifest has no parent directory")?)?;
    let mut manifest = read_manifest(&path, harness)?;
    manifest.acp.program = resolve_sidecar_program(&dir, &manifest.acp.program)
        .with_context(|| format!("Invalid ACP program in {}", path.display()))?;
    log::info!("Selected ACP manifest for {harness}: {}", path.display());
    Ok(HarnessManifest {
        launch: manifest.acp,
        default: None,
        sidecar_without_manifest: false,
        auth_check_command: manifest.auth.check_command,
        runtime_error_patterns: manifest.auth.runtime_error_patterns,
    })
}

/// Launch and auth details for a harness with no sidecar: the ACP adapter is resolved on
/// `PATH`, and the auth knowledge comes from the terminal transport for the same harness.
fn default_manifest(harness: Harness) -> Option<HarnessManifest> {
    let (adapter, pty) = default_adapter(harness)?;
    Some(HarnessManifest {
        launch: AcpLaunchSpec {
            program: adapter.preferred.to_owned(),
            args: adapter.args.iter().map(|arg| (*arg).to_owned()).collect(),
        },
        default: Some(adapter),
        sidecar_without_manifest: false,
        auth_check_command: pty.and_then(ThirdPartyHarness::auth_check_command),
        runtime_error_patterns: pty
            .map(ThirdPartyHarness::runtime_error_patterns_for_run)
            .unwrap_or_default(),
    })
}

/// The built-in adapter for `harness`, with the terminal transport it borrows auth checks
/// from; `None` for a harness that cannot be driven over ACP.
fn default_adapter(
    harness: Harness,
) -> Option<(DefaultAdapter, Option<&'static dyn ThirdPartyHarness>)> {
    const NONE: DefaultAdapter = DefaultAdapter {
        preferred: "",
        alternates: &[],
        args: &[],
        package: None,
        wraps_cli: None,
    };
    Some(match harness {
        // The launcher the macOS and Windows agent packages put on `PATH` pins the adapter
        // version, so it wins over a globally installed copy of the upstream npm package. (The
        // Linux sidecar hides the difference behind its `warp-acp` wrapper.)
        Harness::Claude => (
            DefaultAdapter {
                preferred: "claude-code-acp",
                alternates: &["claude-agent-acp"],
                package: Some("@agentclientprotocol/claude-agent-acp"),
                wraps_cli: Some("claude"),
                ..NONE
            },
            Some(&ClaudeHarness),
        ),
        Harness::Codex => (
            DefaultAdapter {
                preferred: "codex-acp",
                ..NONE
            },
            Some(&CodexHarness),
        ),
        Harness::Gemini => (
            DefaultAdapter {
                preferred: "gemini",
                args: &["--experimental-acp"],
                ..NONE
            },
            Some(&GeminiHarness),
        ),
        Harness::Pi => (
            DefaultAdapter {
                preferred: "pi-acp",
                ..NONE
            },
            None,
        ),
        Harness::Oz | Harness::OpenCode | Harness::Unknown => return None,
    })
}

fn read_manifest(path: &Path, harness: Harness) -> Result<Manifest> {
    let bytes =
        fs::read(path).with_context(|| format!("Cannot read manifest {}", path.display()))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .with_context(|| format!("Invalid manifest {}", path.display()))?;
    if manifest.schema_version != 1 {
        bail!(
            "Manifest {} uses unsupported schema_version {}; expected 1",
            path.display(),
            manifest.schema_version
        );
    }
    if manifest.harness != harness.to_string() {
        bail!(
            "Manifest {} declares harness {:?}; expected {harness}",
            path.display(),
            manifest.harness
        );
    }
    Ok(manifest)
}

fn resolve_sidecar_program(dir: &Path, program: &str) -> Result<String> {
    let mut components = WindowsPath::new(program).components();
    if program.contains(['/', '\\'])
        || !matches!(components.next(), Some(WindowsComponent::Normal(_)))
        || components.next().is_some()
    {
        bail!("Sidecar ACP program must be a bare filename, got {program:?}");
    }
    let path = dir.join("usr/local/bin").join(program);
    let metadata = fs::metadata(&path).with_context(|| {
        format!(
            "Sidecar ACP executable {} is missing or unreadable",
            path.display()
        )
    })?;
    if !metadata.is_file() {
        bail!("Sidecar ACP executable {} is not a file", path.display());
    }
    if !file_exists_and_is_executable(&path) {
        bail!(
            "Sidecar ACP executable {} is not executable",
            path.display()
        );
    }
    path.into_os_string()
        .into_string()
        .map_err(|_| anyhow!("Sidecar ACP executable path is not valid UTF-8"))
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
