use std::fs;
use std::path::{Path, PathBuf, absolute};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::Deserialize;
use typed_path::{WindowsComponent, WindowsPath};
use warp_cli::agent::Harness;
use warp_util::path::file_exists_and_is_executable;

use super::super::super::AgentDriverError;
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
    pub(super) launch: AcpLaunchSpec,
    pub(super) auth_check_command: Option<String>,
    pub(super) runtime_error_patterns: Vec<String>,
    pub(super) is_sidecar: bool,
}

pub(super) fn resolve(harness: Harness) -> Result<HarnessManifest, AgentDriverError> {
    let override_dir = std::env::var_os(SIDECAR_ENV).map(PathBuf::from);
    discover(harness, override_dir.as_deref(), Path::new("/mnt")).map_err(|error| {
        AgentDriverError::HarnessSetupFailed {
            harness: harness.to_string(),
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
    let mount_path = mounts_dir
        .join(format!("{harness}-cli-sidecar"))
        .join(MANIFEST_FILE);
    let path = if let Some(dir) = override_dir {
        dir.join(MANIFEST_FILE)
    } else if mount_path
        .try_exists()
        .with_context(|| format!("Cannot inspect {}", mount_path.display()))?
    {
        mount_path
    } else {
        log::info!("Using built-in ACP defaults for {harness}");
        return default_manifest(harness)
            .with_context(|| format!("The {harness} harness cannot be driven over ACP."));
    };
    let dir = absolute(path.parent().context("Manifest has no parent directory")?)?;
    let mut manifest = read_manifest(&path, harness)?;
    manifest.acp.program = resolve_sidecar_program(&dir, &manifest.acp.program)
        .with_context(|| format!("Invalid ACP program in {}", path.display()))?;
    log::info!("Selected ACP manifest for {harness}: {}", path.display());
    Ok(HarnessManifest {
        launch: manifest.acp,
        auth_check_command: manifest.auth.check_command,
        runtime_error_patterns: manifest.auth.runtime_error_patterns,
        is_sidecar: true,
    })
}

/// Launch and auth details for a harness with no sidecar: the ACP adapter is resolved on
/// `PATH`, and the auth knowledge comes from the terminal transport for the same harness.
fn default_manifest(harness: Harness) -> Option<HarnessManifest> {
    let (program, args, pty): (&str, &[&str], Option<&dyn ThirdPartyHarness>) = match harness {
        Harness::Claude => ("claude-code-acp", &[], Some(&ClaudeHarness)),
        Harness::Codex => ("codex-acp", &[], Some(&CodexHarness)),
        Harness::Gemini => ("gemini", &["--experimental-acp"], Some(&GeminiHarness)),
        Harness::Pi => ("pi-acp", &[], None),
        Harness::Oz | Harness::OpenCode | Harness::Unknown => return None,
    };
    Some(HarnessManifest {
        launch: AcpLaunchSpec {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        },
        auth_check_command: pty.and_then(ThirdPartyHarness::auth_check_command),
        runtime_error_patterns: pty
            .map(ThirdPartyHarness::runtime_error_patterns_for_run)
            .unwrap_or_default(),
        is_sidecar: false,
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
