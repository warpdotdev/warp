mod glibc;

pub use glibc::{GlibcVersion, RemoteLibc};

use std::time::Duration;

use anyhow::anyhow;
use warp_core::channel::{Channel, ChannelState};
pub const REMOTE_SERVER_ARTIFACT_VERSION_UNPINNED: &str = "unversioned";

/// State machine for the remote server install → launch → initialize flow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteServerSetupState {
    /// Checking if the binary exists on remote.
    Checking,
    /// Downloading and installing the binary for the first time on this host.
    Installing { progress_percent: Option<u8> },
    /// Replacing an existing install with a differently-versioned binary.
    /// Rendered as "Updating..." in the UI so the user understands this
    /// isn't a fresh install.
    Updating,
    /// Binary is launched, waiting for InitializeResponse.
    Initializing,
    /// Handshake complete. Ready.
    Ready,
    /// Something failed. Fall back to ControlMaster.
    Failed { error: String },
    /// Preinstall check classified the host as incompatible with the
    /// prebuilt remote-server binary. The controller treats this as a
    /// clean fall-back to the legacy ControlMaster-backed SSH flow,
    /// distinct from `Failed` (which is rendered as a real error).
    Unsupported { reason: UnsupportedReason },
}

impl RemoteServerSetupState {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::Unsupported { .. })
    }

    pub fn is_terminal(&self) -> bool {
        self.is_ready() || self.is_failed() || self.is_unsupported()
    }

    pub fn is_in_progress(&self) -> bool {
        matches!(
            self,
            Self::Checking | Self::Installing { .. } | Self::Updating | Self::Initializing
        )
    }

    pub fn is_connecting(&self) -> bool {
        matches!(
            self,
            Self::Installing { .. } | Self::Updating | Self::Initializing
        )
    }
}

impl From<&crate::transport::Error> for RemoteServerSetupState {
    fn from(error: &crate::transport::Error) -> Self {
        if let Some(reason) = UnsupportedReason::from_transport_error(error) {
            Self::Unsupported { reason }
        } else {
            Self::Failed {
                error: error.to_string(),
            }
        }
    }
}

/// Outcome of [`crate::transport::RemoteTransport::run_preinstall_check`].
///
/// The script runs over the existing SSH socket before any install UI
/// surfaces and reports whether the host can run the prebuilt
/// remote-server binary. The Rust side is intentionally a thin parser
/// over the script's structured stdout (see `preinstall_check.sh`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreinstallCheckResult {
    pub status: PreinstallStatus,
    pub libc: RemoteLibc,
    /// Verbatim, trimmed script stdout. Forwarded to telemetry for
    /// diagnosing `Unknown` outcomes on exotic distros.
    pub raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreinstallStatus {
    Supported,
    Unsupported {
        reason: UnsupportedReason,
    },
    /// Probe ran but couldn't classify the host. Treated as supported
    /// (fail open) by [`PreinstallCheckResult::is_supported`] so we keep
    /// today's install-and-try behavior on hosts where the probe is
    /// unreliable.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnsupportedReason {
    GlibcTooOld {
        detected: GlibcVersion,
        required: GlibcVersion,
    },
    NonGlibc {
        name: String,
    },
    UnsupportedOs {
        os: String,
    },
    UnsupportedArch {
        arch: String,
    },
}

impl UnsupportedReason {
    pub fn from_transport_error(error: &crate::transport::Error) -> Option<Self> {
        match error {
            crate::transport::Error::UnsupportedOs { os } => {
                Some(Self::UnsupportedOs { os: os.clone() })
            }
            crate::transport::Error::UnsupportedArch { arch } => {
                Some(Self::UnsupportedArch { arch: arch.clone() })
            }
            crate::transport::Error::TimedOut
            | crate::transport::Error::ScriptFailed { .. }
            | crate::transport::Error::Other(_) => None,
        }
    }

    pub fn as_telemetry_reason(&self) -> &'static str {
        match self {
            Self::GlibcTooOld { .. } => "glibc_too_old",
            Self::NonGlibc { .. } => "non_glibc",
            Self::UnsupportedOs { .. } => "unsupported_os",
            Self::UnsupportedArch { .. } => "unsupported_arch",
        }
    }
}

impl PreinstallCheckResult {
    pub fn unsupported(reason: UnsupportedReason) -> Self {
        Self {
            status: PreinstallStatus::Unsupported { reason },
            libc: RemoteLibc::Unknown,
            raw: String::new(),
        }
    }
    /// Whether the host is supported. Both `Supported` and `Unknown`
    /// return true — only positive detection of an incompatible libc
    /// triggers the silent fall-back.
    pub fn is_supported(&self) -> bool {
        match self.status {
            PreinstallStatus::Supported | PreinstallStatus::Unknown => true,
            PreinstallStatus::Unsupported { .. } => false,
        }
    }

    /// Parses the structured `key=value` stdout emitted by
    /// `preinstall_check.sh`. Tolerates unknown keys and lines without
    /// `=` (forward-compatibility): future versions of the script can
    /// add new keys without coordinating a client release.
    pub fn parse(stdout: &str) -> Self {
        let mut status_str: Option<&str> = None;
        let mut reason_str: Option<&str> = None;
        let mut libc_family: Option<&str> = None;
        let mut libc_version: Option<&str> = None;
        let mut required_glibc: Option<&str> = None;

        for line in stdout.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "status" => status_str = Some(value.trim()),
                "reason" => reason_str = Some(value.trim()),
                "libc_family" => libc_family = Some(value.trim()),
                "libc_version" => libc_version = Some(value.trim()),
                "required_glibc" => required_glibc = Some(value.trim()),
                _ => {} // ignore unknown keys
            }
        }

        let libc = glibc::parse_libc(libc_family, libc_version);
        let status = parse_status(status_str, reason_str, &libc, required_glibc);

        Self {
            status,
            libc,
            raw: stdout.trim().to_string(),
        }
    }
}

fn parse_status(
    status: Option<&str>,
    reason: Option<&str>,
    libc: &RemoteLibc,
    required_glibc: Option<&str>,
) -> PreinstallStatus {
    match status {
        Some("supported") => PreinstallStatus::Supported,
        Some("unsupported") => match reason {
            Some("glibc_too_old") => {
                let detected = match libc {
                    RemoteLibc::Glibc(v) => Some(*v),
                    _ => None,
                };
                let required = required_glibc.and_then(GlibcVersion::parse);
                match (detected, required) {
                    (Some(detected), Some(required)) => PreinstallStatus::Unsupported {
                        reason: UnsupportedReason::GlibcTooOld { detected, required },
                    },
                    // The script said `unsupported` + `glibc_too_old` but we
                    // can't recover the numbers — fail open rather than
                    // surface a malformed reason.
                    _ => PreinstallStatus::Unknown,
                }
            }
            Some("non_glibc") => {
                let name = match libc {
                    RemoteLibc::NonGlibc { name } => name.clone(),
                    _ => "unknown".to_string(),
                };
                PreinstallStatus::Unsupported {
                    reason: UnsupportedReason::NonGlibc { name },
                }
            }
            _ => PreinstallStatus::Unknown,
        },
        // status=unknown, missing, or anything else → fail open.
        _ => PreinstallStatus::Unknown,
    }
}

/// The bundled preinstall check script. Loaded as a string so the SSH
/// transport can pipe it through the existing ControlMaster socket via
/// [`crate::ssh::run_ssh_script`].
///
/// The script is intentionally self-contained — the supported-glibc
/// floor is hardcoded inside the script (see `preinstall_check.sh`)
/// rather than templated from Rust.
pub const PREINSTALL_CHECK_SCRIPT: &str = include_str!("preinstall_check.sh");

/// Detected remote platform from `uname -sm` output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemotePlatform {
    pub os: RemoteOs,
    pub arch: RemoteArch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteOs {
    Linux,
    MacOs,
}

impl RemoteOs {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::MacOs => "macos",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteArch {
    X86_64,
    Aarch64,
}

impl RemoteArch {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }
}

/// Parse `uname -sm` output into a `RemotePlatform`.
///
/// The expected format is `<os> <arch>`, e.g. `Linux x86_64` or `Darwin arm64`.
/// Takes the last line to skip any shell initialization output.
pub fn parse_uname_output(
    output: &str,
) -> std::result::Result<RemotePlatform, crate::transport::Error> {
    use crate::transport::Error;

    let line = output
        .lines()
        .last()
        .ok_or_else(|| Error::Other(anyhow!("empty uname output")))
        .map(str::trim)?;

    let mut parts = line.split_whitespace();
    let os_str = parts
        .next()
        .ok_or_else(|| Error::Other(anyhow!("missing OS in uname output: {line}")))?;
    let arch_str = parts
        .next()
        .ok_or_else(|| Error::Other(anyhow!("missing arch in uname output: {line}")))?;

    let os = match os_str {
        "Linux" => RemoteOs::Linux,
        "Darwin" => RemoteOs::MacOs,
        other => {
            return Err(Error::UnsupportedOs {
                os: other.to_string(),
            })
        }
    };

    let arch = match arch_str {
        "x86_64" | "amd64" => RemoteArch::X86_64,
        "aarch64" | "arm64" => RemoteArch::Aarch64,
        other => {
            return Err(Error::UnsupportedArch {
                arch: other.to_string(),
            })
        }
    };

    Ok(RemotePlatform { os, arch })
}

/// Returns the remote directory where the binary is installed, keyed by channel.
///
/// - stable:      `~/.warp/remote-server`
/// - preview:     `~/.warp-preview/remote-server`
/// - dev:         `~/.warp-dev/remote-server`
/// - local:       `~/.warp-local/remote-server`
/// - integration: `~/.warp-dev/remote-server`
/// - warp-oss:    `~/.warp-oss/remote-server`
pub fn remote_server_dir() -> String {
    let warp_dir = match ChannelState::channel() {
        Channel::Stable => ".warp",
        Channel::Preview => ".warp-preview",
        Channel::Dev | Channel::Integration => ".warp-dev",
        Channel::Local => ".warp-local",
        Channel::Oss => {
            // TODO(alokedesai): need to figure out how remote server works with warp-oss
            // For now, return what Dev returns.
            ".warp-dev"
        }
    };
    format!("~/{warp_dir}/remote-server")
}

/// Returns a short, deterministic directory name for a remote-server
/// identity key, used for the daemon socket and PID file paths.
///
/// Hashes the key to 8 hex chars so the socket path stays within the
/// `sun_path` limit across all channels.
pub fn remote_server_identity_dir_name(identity_key: &str) -> String {
    use std::hash::{Hash, Hasher};

    if identity_key.is_empty() {
        return "empty".to_string();
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    identity_key.hash(&mut hasher);
    format!("{:016x}", hasher.finish())[..8].to_string()
}

/// Percent-encodes an identity key for use in filesystem paths.
///
/// Keeps ASCII alphanumeric characters plus `-` and `_`; percent-encodes
/// all other bytes.  Used by [`remote_server_daemon_data_dir`] for
/// persistent data that must not collide across identities.
fn percent_encode_identity_key(identity_key: &str) -> String {
    if identity_key.is_empty() {
        return "empty".to_string();
    }

    let mut encoded = String::with_capacity(identity_key.len());
    for byte in identity_key.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// Returns the identity-scoped remote directory used for the daemon socket
/// and PID file.  Uses the hashed identity dir name so the full socket
/// path fits within `sun_path`.
pub fn remote_server_daemon_dir(identity_key: &str) -> String {
    format!(
        "{}/{}",
        remote_server_dir(),
        remote_server_identity_dir_name(identity_key)
    )
}

/// Returns the identity-scoped remote directory used for daemon-owned
/// per-user data files (e.g. SQLite databases).
///
/// Uses the full percent-encoded identity key (not the hash) so that
/// persistent data is never shared between distinct identities due to
/// a hash collision.  The `sun_path` limit does not apply here because
/// this path is only used for regular file I/O, not Unix sockets.
pub fn remote_server_daemon_data_dir(identity_key: &str) -> String {
    format!(
        "{}/{}/data",
        remote_server_dir(),
        percent_encode_identity_key(identity_key)
    )
}

/// Returns a short, deterministic 8-hex-char hash of the app version string.
///
/// Used to version-discriminate daemon socket and PID files without
/// embedding the full version string in the filename, which would push
/// the Unix domain socket path over the `sun_path` limit (107 bytes on
/// Linux, 103 on macOS) for users with moderately long identity keys or
/// home directory paths.
pub fn version_hash() -> Option<String> {
    use std::hash::{Hash, Hasher};

    let version = ChannelState::app_version()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    version.hash(&mut hasher);
    Some(format!("{:016x}", hasher.finish())[..8].to_string())
}

/// Returns the daemon socket filename, versioned with a short hash when
/// a release tag is baked in.
///
/// - With `GIT_RELEASE_TAG`:    `server-{hash8}.sock`  (e.g. `server-a1b2c3d4.sock`)
/// - Without (plain cargo run): `server.sock`
pub fn daemon_socket_name() -> String {
    match version_hash() {
        Some(hash) => format!("server-{hash}.sock"),
        None => "server.sock".to_string(),
    }
}

/// Returns the daemon PID filename, versioned with a short hash when a
/// release tag is baked in.
///
/// - With `GIT_RELEASE_TAG`:    `server-{hash8}.pid`
/// - Without (plain cargo run): `server.pid`
pub fn daemon_pid_name() -> String {
    match version_hash() {
        Some(hash) => format!("server-{hash}.pid"),
        None => "server.pid".to_string(),
    }
}

/// Returns the binary name, keyed by channel.
///
/// Matches the CLI command names: `oz` (stable), `oz-preview`, `oz-dev`.
pub fn binary_name() -> &'static str {
    ChannelState::channel().cli_command_name()
}

/// Returns the full remote binary path for the current channel and client
/// version.
///
/// The path-versioning rule is keyed strictly off [`Channel`]:
///
/// - [`Channel::Local`] and [`Channel::Oss`] always use the bare
///   `{binary_name}` path. For `Local` this is the slot
///   `script/deploy_remote_server` writes to; `Oss` is treated the
///   same way because it has no release-pinned CDN artifact and is
///   expected to be deployed/managed locally.
/// - Every other channel always uses `{binary_name}-{version}`, where
///   `version` is the baked-in `GIT_RELEASE_TAG` when present and falls
///   back to `CARGO_PKG_VERSION` otherwise. The fallback keeps the path
///   deterministic for misconfigured `cargo run --bin {dev,preview,...}`
///   builds; the resulting `&version=...` query is expected to 404 against
///   `/download/cli` and surface a clean `SetupFailed` rather than silently
///   writing to a path that doesn't follow the rule.
pub fn remote_server_binary() -> String {
    let dir = remote_server_dir();
    let name = binary_name();
    match ChannelState::channel() {
        Channel::Local | Channel::Oss => format!("{dir}/{name}"),
        Channel::Stable | Channel::Preview | Channel::Dev | Channel::Integration => {
            format!("{dir}/{name}-{}", pinned_version())
        }
    }
}

/// Returns the shell command to verify the remote server binary is
/// installed and functional by running it with `--version`.
///
/// Exits 0 when the binary is present, executable, and can parse its
/// own arguments. A missing binary produces exit 127 (command not
/// found) or 126 (not executable), and a corrupted binary will fail
/// with a non-zero exit of its own.
pub fn binary_check_command() -> String {
    format!("{} --version", remote_server_binary())
}

/// Returns the version string used to pin remote-server installs on
/// channels that take the versioned path (i.e. everything except
/// [`Channel::Local`] and [`Channel::Oss`]). Prefers the baked-in
/// `GIT_RELEASE_TAG` from [`ChannelState::app_version`]; falls back to
/// `CARGO_PKG_VERSION` so the path / install URL is deterministic even on
/// dev `cargo run` builds without a release tag. The `CARGO_PKG_VERSION`
/// fallback is not expected to map to a real `/download/cli` artifact —
/// it exists to produce a clean install-time failure rather than silently
/// fall through to the unversioned (Local/Oss-only) path.
fn pinned_version() -> &'static str {
    ChannelState::app_version().unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// Returns the version key used to identify remote-server download artifacts.
///
/// This must match the versioning used by [`download_tarball_url`] and
/// [`install_script`], so versioned download URLs do not reuse stale tarballs
/// from a previous client version.
pub fn remote_server_artifact_version() -> &'static str {
    match ChannelState::channel() {
        Channel::Local | Channel::Oss => REMOTE_SERVER_ARTIFACT_VERSION_UNPINNED,
        Channel::Stable | Channel::Preview | Channel::Dev | Channel::Integration => {
            pinned_version()
        }
    }
}

/// The install script template, loaded from a standalone `.sh` file for
/// readability. Placeholders like `{download_base_url}` are substituted by
/// [`install_script`].
const INSTALL_SCRIPT_TEMPLATE: &str = include_str!("install_remote_server.sh");

/// Returns the install script that downloads and installs the CLI binary
/// at the current client version.
///
/// The script detects the remote architecture via `uname -m`, downloads
/// the correct Oz CLI tarball from the download URL, and installs it at
/// the path returned by [`remote_server_binary`] so repeat invocations
/// are idempotent. The `version_query` / `version_suffix` substitutions
/// follow the same rule as [`remote_server_binary`]: empty on
/// [`Channel::Local`] and [`Channel::Oss`] (so the install lands at
/// the unversioned path used by `script/deploy_remote_server`); pinned to
/// `&version={v}` / `-{v}` on every other channel, where `v` falls back
/// to `CARGO_PKG_VERSION` when no release tag is baked in.
pub fn install_script(staging_tarball_path: Option<&str>) -> String {
    let (vq, version_suffix) = match ChannelState::channel() {
        Channel::Local | Channel::Oss => (String::new(), String::new()),
        Channel::Stable | Channel::Preview | Channel::Dev | Channel::Integration => {
            let v = pinned_version();
            (format!("&version={v}"), format!("-{v}"))
        }
    };
    INSTALL_SCRIPT_TEMPLATE
        .replace("{download_base_url}", &download_url())
        .replace("{channel}", download_channel())
        .replace("{install_dir}", &remote_server_dir())
        .replace("{binary_name}", binary_name())
        .replace("{version_query}", &vq)
        .replace("{version_suffix}", &version_suffix)
        .replace(
            "{no_http_client_exit_code}",
            &NO_HTTP_CLIENT_EXIT_CODE.to_string(),
        )
        .replace("{staging_tarball_path}", staging_tarball_path.unwrap_or(""))
}

/// Construct the download URL from the server root URL.
///
/// For example, given `https://app.warp.dev`, returns
/// `https://app.warp.dev/download/cli`.
fn download_url() -> String {
    let base = ChannelState::server_root_url();
    let base = base.trim_end_matches('/');
    format!("{base}/download/cli")
}

/// Maps the client's [`Channel`] to the server's download channel parameter.
///
/// The server recognises `"stable"`, `"preview"`, and `"dev"`.  Local and
/// Integration builds map to `"dev"` so they fetch dogfood artifacts.
fn download_channel() -> &'static str {
    match ChannelState::channel() {
        Channel::Stable => "stable",
        Channel::Preview => "preview",
        Channel::Dev | Channel::Local | Channel::Integration => "dev",
        Channel::Oss => {
            // TODO(alokedesai): need to figure out how remote server works with warp-oss
            // For now, return what Dev returns.
            "dev"
        }
    }
}

/// Returns the version query string for the download URL (e.g.
/// `"&version=v0.2026.01.01"` on release channels, empty on Local/Oss).
fn version_query() -> String {
    match ChannelState::channel() {
        Channel::Local | Channel::Oss => String::new(),
        Channel::Stable | Channel::Preview | Channel::Dev | Channel::Integration => {
            format!("&version={}", pinned_version())
        }
    }
}

/// Returns the full download URL for the remote server tarball,
/// parameterized by the remote platform. Used by the SCP upload
/// fallback to download the same artifact the shell script would fetch.
pub fn download_tarball_url(platform: &RemotePlatform) -> String {
    format!(
        "{}?package=tar&os={}&arch={}&channel={}{}",
        download_url(),
        platform.os.as_str(),
        platform.arch.as_str(),
        download_channel(),
        version_query(),
    )
}

/// Exit code the install script uses when neither curl nor wget is
/// available on the remote host. The Rust side matches on this to
/// trigger the SCP upload fallback.
pub const NO_HTTP_CLIENT_EXIT_CODE: i32 = 3;

/// Timeout for the binary existence check.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// Timeout for the install script (curl/wget path).
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(180);

/// Timeout for the SCP upload fallback path (local download + SCP +
/// extraction). Higher than [`INSTALL_TIMEOUT`] because SCP transfers
/// the tarball over the user's SSH link, which is typically slower than
/// the remote host's direct internet connection.
pub const SCP_INSTALL_TIMEOUT: Duration = Duration::from_secs(240);

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
