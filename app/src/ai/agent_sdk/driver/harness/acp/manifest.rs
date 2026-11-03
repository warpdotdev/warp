use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf, absolute};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::Deserialize;
use typed_path::{WindowsComponent, WindowsPath};
use warp_cli::agent::Harness;
use warp_util::path::{file_exists_and_is_executable, resolve_executable_in_path};

use super::super::super::AgentDriverError;
use super::super::ThirdPartyHarness;
use super::super::claude_code::ClaudeHarness;
use super::super::codex::CodexHarness;
use super::super::gemini::GeminiHarness;
use super::launch::AcpLaunchSpec;

const SIDECAR_ENV: &str = "WARP_HARNESS_SIDECAR";
const MANIFEST_FILE: &str = "warp-harness.json";

/// Program names the no-sidecar Claude default accepts, in preference order: the bin the
/// upstream npm package installs, then the launcher name the macOS and Windows agent packages
/// put on `PATH`. (The Linux sidecar hides the difference behind its `warp-acp` wrapper.)
pub(super) const CLAUDE_DEFAULT_PROGRAMS: [&str; 2] = ["claude-agent-acp", "claude-code-acp"];
/// The npm package that provides [`CLAUDE_DEFAULT_PROGRAMS`]'s first entry.
pub(super) const CLAUDE_ADAPTER_PACKAGE: &str = "@agentclientprotocol/claude-agent-acp";

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
    let path_env = std::env::var_os("PATH").unwrap_or_default();
    let (program, args, pty): (String, &[&str], Option<&dyn ThirdPartyHarness>) = match harness {
        Harness::Claude => (
            select_default_program(&CLAUDE_DEFAULT_PROGRAMS, &path_env).to_owned(),
            &[],
            Some(&ClaudeHarness),
        ),
        Harness::Codex => ("codex-acp".to_owned(), &[], Some(&CodexHarness)),
        Harness::Gemini => (
            "gemini".to_owned(),
            &["--experimental-acp"],
            Some(&GeminiHarness),
        ),
        Harness::Pi => ("pi-acp".to_owned(), &[], None),
        Harness::Oz | Harness::OpenCode | Harness::Unknown => return None,
    };
    Some(HarnessManifest {
        launch: AcpLaunchSpec {
            program,
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        },
        auth_check_command: pty.and_then(ThirdPartyHarness::auth_check_command),
        runtime_error_patterns: pty
            .map(ThirdPartyHarness::runtime_error_patterns_for_run)
            .unwrap_or_default(),
        is_sidecar: false,
    })
}

/// Picks the first of `candidates` that resolves on `path_env`, so the bridge launches the
/// name that actually exists. When none does, the preferred name is kept: the install check
/// then reports it as missing with [`missing_default_program_reason`].
fn select_default_program<'a>(candidates: &[&'a str], path_env: &OsStr) -> &'a str {
    candidates
        .iter()
        .copied()
        .find(|candidate| resolve_executable_in_path(candidate, path_env).is_some())
        .unwrap_or(candidates[0])
}

/// Why a harness with no sidecar cannot start: its ACP adapter is not on `PATH`. The
/// adapter is launched through the terminal session's shell, so it has to be visible to
/// that shell as well as to this process. `None` when the generic missing-CLI message is
/// enough for `harness`.
pub(super) fn missing_default_program_reason(
    harness: Harness,
    install_docs_url: Option<&str>,
) -> Option<String> {
    match harness {
        Harness::Claude => {
            let [preferred, fallback] = CLAUDE_DEFAULT_PROGRAMS;
            let mut reason = format!(
                "No Claude ACP adapter found: neither '{preferred}' nor '{fallback}' is on PATH. \
                 Install it with `npm install -g {CLAUDE_ADAPTER_PACKAGE}` and make sure it is \
                 on PATH for both the oz process and the terminal session's shell, which \
                 launches it."
            );
            if let Some(url) = install_docs_url {
                reason.push_str(&format!(" Claude Code itself must be installed too: {url}"));
            }
            Some(reason)
        }
        Harness::Codex
        | Harness::Gemini
        | Harness::Pi
        | Harness::Oz
        | Harness::OpenCode
        | Harness::Unknown => None,
    }
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
