//! Background auto-updater for the headless `warp-tui` front-end.
//!
//! Follows the "native installer" model used by peer CLIs (e.g. Claude Code):
//! the installer (`warp-server/download/tui_install.sh`) lays installs out as
//!
//! ```text
//! <root>/                      # ~/.warp/tui by default
//!   versions/<version>/        # binary + resources/ per installed version
//!   current                    # symlink to the active versions/<version>
//! ~/.local/bin/warp-tui        # symlink to current/warp-tui-<channel>
//! ```
//!
//! and this module keeps that layout fresh: it polls on the same cadence as
//! the GUI autoupdater (each poll is a single lightweight `/client_version`
//! request), downloads newer builds from the server's `/download/tui`
//! endpoint, stages them into `versions/<version>`, and atomically retargets
//! the `current` symlink. The running session is never touched — the new
//! version is picked up on the next launch. In particular, the updater never
//! deletes the version directory the current process is executing from
//! (removing a running binary breaks child-process spawning).
//!
//! Background updates only run for managed installs (i.e. when the running
//! executable resolves into a `versions/` directory), so `cargo run` builds
//! and legacy flat installs are unaffected. Users can opt out with the
//! file-backed `general.autoupdate_enabled` setting or the
//! `WARP_TUI_DISABLE_AUTOUPDATE` environment variable; re-running the
//! install script remains available as a manual escape hatch.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context as _, Result};
use channel_versions::{ChannelVersions, ParsedVersion};
use futures::{StreamExt as _, TryStreamExt as _};
use warp::settings::TuiAutoupdateSettings;
use warp_core::channel::{Channel, ChannelState};
use warp_core::send_telemetry_from_ctx;
use warpui::r#async::Timer;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::telemetry::TuiAutoupdateTelemetryEvent;

/// Setting this environment variable (to any value) disables background
/// auto-updates for a single launch, regardless of the
/// `general.autoupdate_enabled` setting.
const DISABLE_ENV_VAR: &str = "WARP_TUI_DISABLE_AUTOUPDATE";

/// Name of the directory holding per-version installs under the install root.
const VERSIONS_DIR_NAME: &str = "versions";

/// Name of the symlink under the install root pointing at the active version.
const CURRENT_LINK_NAME: &str = "current";

/// Lock file under the install root serializing installs across concurrent
/// TUI processes.
const LOCK_FILE_NAME: &str = ".update.lock";

/// How often to check for updates. Mirrors the GUI autoupdater's poll
/// interval (`AutoupdateState::AUTOUPDATE_POLL`); each check is a single
/// lightweight `/client_version` request unless a new version actually needs
/// downloading.
const CHECK_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// A lock file held for longer than this is considered abandoned (e.g. a
/// crashed updater) and is broken.
const STALE_LOCK_AGE: Duration = Duration::from_secs(60 * 60);

/// Timeout for the (small) channel-versions fetch.
const FETCH_VERSIONS_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for downloading the TUI tarball itself.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// The managed, versioned install layout the running binary belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
struct InstallLayout {
    /// Root of the versioned install (e.g. `~/.warp/tui`).
    root: PathBuf,
    /// `<root>/versions`.
    versions_dir: PathBuf,
    /// `<root>/current`, the symlink to the active version directory.
    current_link: PathBuf,
    /// The version directory the running binary is executing from.
    running_version_dir: PathBuf,
    /// The channel-suffixed binary name (e.g. `warp-tui-dev`).
    binary_name: String,
}

impl InstallLayout {
    /// Detects the managed install layout from the running executable.
    /// Returns `None` when the binary isn't inside a `versions/<version>/`
    /// directory (e.g. `cargo run` builds or legacy flat installs).
    fn detect() -> Option<Self> {
        let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
        Self::from_canonical_exe_path(&exe)
    }

    /// Builds the layout from an already-canonicalized executable path of the
    /// shape `<root>/versions/<version>/<binary_name>`.
    fn from_canonical_exe_path(exe: &Path) -> Option<Self> {
        let binary_name = exe.file_name()?.to_str()?.to_owned();
        let running_version_dir = exe.parent()?.to_path_buf();
        let versions_dir = running_version_dir.parent()?.to_path_buf();
        if versions_dir.file_name()? != VERSIONS_DIR_NAME {
            return None;
        }
        let root = versions_dir.parent()?.to_path_buf();
        Some(Self {
            current_link: root.join(CURRENT_LINK_NAME),
            root,
            versions_dir,
            running_version_dir,
            binary_name,
        })
    }
}

/// The result of a single update check.
#[derive(Debug)]
enum UpdateOutcome {
    /// Skipped: another process is installing an update right now.
    Locked,
    /// The running build is already the channel's latest version.
    UpToDate { version: String },
    /// A newer version was already staged by a previous check and `current`
    /// points at it; nothing to do until the next launch.
    PendingRestart { version: String },
    /// A newer version was staged and `current` now points at it. It takes
    /// effect on the next launch.
    Installed { version: String },
}

impl UpdateOutcome {
    /// Stable identifier for this kind of outcome, used for telemetry and
    /// for detecting transitions between consecutive checks.
    fn kind(&self) -> &'static str {
        match self {
            UpdateOutcome::Locked => "locked",
            UpdateOutcome::UpToDate { .. } => "up_to_date",
            UpdateOutcome::PendingRestart { .. } => "pending_restart",
            UpdateOutcome::Installed { .. } => "installed",
        }
    }

    /// The version associated with this outcome, if any.
    fn version(&self) -> Option<&str> {
        match self {
            UpdateOutcome::Locked => None,
            UpdateOutcome::UpToDate { version }
            | UpdateOutcome::PendingRestart { version }
            | UpdateOutcome::Installed { version } => Some(version),
        }
    }
}

/// User-visible status of the background updater, shown next to the version
/// in the transcript zero state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TuiAutoupdateStatus {
    /// Nothing to show: updates are disabled for this process, or no check
    /// has produced a stable result yet (e.g. the first check failed).
    Idle,
    /// Fetching the latest version for this channel.
    Checking,
    /// Downloading and staging a newer version.
    Updating,
    /// The running build is the channel's latest version.
    UpToDate,
    /// A newer version is staged and takes effect on the next launch.
    PendingRestart,
}

/// Events emitted by [`TuiAutoupdater`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TuiAutoupdaterEvent {
    /// [`TuiAutoupdater::status`] changed.
    StatusChanged,
}

/// Whether this process runs the background update loop.
#[derive(Clone, Debug)]
enum AutoupdateEligibility {
    /// Eligible: a release build running from the managed versioned install
    /// layout, without the env opt-out.
    Enabled(InstallLayout),
    /// Background updates are disabled for this process.
    Disabled {
        /// Why updates are disabled, for logging/debugging.
        reason: &'static str,
    },
}

impl AutoupdateEligibility {
    /// Determines whether this process should run the background update loop.
    ///
    /// The `general.autoupdate_enabled` setting is read once here, at startup;
    /// toggling it takes effect on the next launch.
    fn determine(ctx: &AppContext) -> Self {
        if std::env::var_os(DISABLE_ENV_VAR).is_some() {
            return Self::Disabled {
                reason: "opted out via the WARP_TUI_DISABLE_AUTOUPDATE environment variable",
            };
        }
        if !*TuiAutoupdateSettings::as_ref(ctx).autoupdate_enabled {
            return Self::Disabled {
                reason: "opted out via the general.autoupdate_enabled setting",
            };
        }
        if ChannelState::app_version().is_none() {
            return Self::Disabled {
                reason: "no release version tag baked into this build",
            };
        }
        if download_os().is_none() {
            return Self::Disabled {
                reason: "no TUI release artifacts exist for this platform",
            };
        }
        match InstallLayout::detect() {
            Some(layout) => Self::Enabled(layout),
            None => Self::Disabled {
                reason: "not running from a managed install",
            },
        }
    }
}

/// Singleton driving the background update loop for the TUI session.
///
/// Always registered — even when this process isn't eligible for background
/// updates — so other callsites can safely access the singleton. The polling
/// loop only runs when [`Self::eligibility`] is
/// [`AutoupdateEligibility::Enabled`].
pub(crate) struct TuiAutoupdater {
    /// Whether (and where) this process runs background updates.
    eligibility: AutoupdateEligibility,
    /// The user-visible status of the update loop.
    status: TuiAutoupdateStatus,
    /// The outcome kind last reported to telemetry. Consecutive checks
    /// usually resolve to the same outcome (e.g. `up_to_date` on every
    /// poll), so only transitions are reported.
    last_reported_outcome: Option<&'static str>,
}

impl Entity for TuiAutoupdater {
    type Event = TuiAutoupdaterEvent;
}

impl SingletonEntity for TuiAutoupdater {}

impl TuiAutoupdater {
    /// Registers the singleton and starts the background update loop when
    /// this process is eligible (see [`AutoupdateEligibility::determine`]).
    pub(crate) fn register(ctx: &mut AppContext) {
        let eligibility = AutoupdateEligibility::determine(ctx);
        ctx.add_singleton_model(move |_| TuiAutoupdater {
            eligibility,
            status: TuiAutoupdateStatus::Idle,
            last_reported_outcome: None,
        });
        TuiAutoupdater::handle(ctx).update(ctx, |me, ctx| match me.eligibility.clone() {
            AutoupdateEligibility::Enabled(layout) => me.check_now(layout, ctx),
            AutoupdateEligibility::Disabled { reason } => {
                log::info!("TUI autoupdate disabled: {reason}");
            }
        });
    }

    /// The user-visible status of the update loop, for the zero state.
    pub(crate) fn status(&self) -> TuiAutoupdateStatus {
        self.status
    }

    /// Updates the status, emitting [`TuiAutoupdaterEvent::StatusChanged`]
    /// only on actual transitions.
    fn set_status(&mut self, status: TuiAutoupdateStatus, ctx: &mut ModelContext<Self>) {
        if self.status == status {
            return;
        }
        self.status = status;
        ctx.emit(TuiAutoupdaterEvent::StatusChanged);
    }

    /// Runs one background update check, then schedules the next one after
    /// [`CHECK_INTERVAL`]. The pass runs in two phases so the zero state can
    /// show progress: a lightweight version check, then — only when a newer
    /// version needs staging — the download/install phase.
    fn check_now(&mut self, layout: InstallLayout, ctx: &mut ModelContext<Self>) {
        // Where the status settles when this pass fails or is skipped: the
        // previous pass's stable status, never the transient `Checking`.
        let fallback_status = self.status;
        self.set_status(TuiAutoupdateStatus::Checking, ctx);
        let check_layout = layout.clone();
        ctx.spawn(
            async move { check_for_update(check_layout).await },
            move |me, decision, ctx| match decision {
                Ok(CheckDecision::Settled(outcome)) => {
                    me.finish_check(Ok(outcome), fallback_status, layout, ctx);
                }
                Ok(CheckDecision::NeedsInstall {
                    latest_version,
                    already_staged,
                }) => {
                    me.set_status(TuiAutoupdateStatus::Updating, ctx);
                    let install_layout = layout.clone();
                    ctx.spawn(
                        async move {
                            install_update(install_layout, latest_version, already_staged).await
                        },
                        move |me, result, ctx| {
                            me.finish_check(result, fallback_status, layout, ctx);
                        },
                    );
                }
                Err(error) => me.finish_check(Err(error), fallback_status, layout, ctx),
            },
        );
    }

    /// Logs and reports the final result of an update pass, settles the
    /// user-visible status, and schedules the next check.
    fn finish_check(
        &mut self,
        result: Result<UpdateOutcome>,
        fallback_status: TuiAutoupdateStatus,
        layout: InstallLayout,
        ctx: &mut ModelContext<Self>,
    ) {
        match &result {
            Ok(outcome) => log::info!("TUI autoupdate check finished: {outcome:?}"),
            // Fail quietly and let the next poll retry; transient
            // network errors (e.g. waking from sleep) are common here.
            Err(error) => log::warn!("TUI autoupdate check failed: {error:#}"),
        }
        self.report_outcome(&result, ctx);
        let status = match &result {
            Ok(UpdateOutcome::UpToDate { .. }) => TuiAutoupdateStatus::UpToDate,
            Ok(UpdateOutcome::PendingRestart { .. } | UpdateOutcome::Installed { .. }) => {
                TuiAutoupdateStatus::PendingRestart
            }
            // Skipped/failed checks aren't surfaced; settle back on the
            // previous stable status and let the next poll retry.
            Ok(UpdateOutcome::Locked) | Err(_) => fallback_status,
        };
        // Once an update is staged, only a restart clears it: never downgrade
        // from `PendingRestart` (e.g. on a server-side version rollback).
        let status = if fallback_status == TuiAutoupdateStatus::PendingRestart {
            TuiAutoupdateStatus::PendingRestart
        } else {
            status
        };
        self.set_status(status, ctx);
        ctx.spawn(
            async { Timer::after(CHECK_INTERVAL).await },
            move |me, _, ctx| me.check_now(layout, ctx),
        );
    }

    /// Sends a telemetry event when the outcome kind changed since the last
    /// check, so the frequent poll doesn't emit repeated `up_to_date` (or
    /// repeated-failure) events.
    fn report_outcome(&mut self, result: &Result<UpdateOutcome>, ctx: &mut ModelContext<Self>) {
        let kind = match result {
            Ok(outcome) => outcome.kind(),
            Err(_) => "failed",
        };
        if self.last_reported_outcome == Some(kind) {
            return;
        }
        self.last_reported_outcome = Some(kind);

        let event = match result {
            Ok(outcome) => TuiAutoupdateTelemetryEvent::CheckCompleted {
                outcome: kind,
                version: outcome.version().map(ToOwned::to_owned),
            },
            Err(error) => TuiAutoupdateTelemetryEvent::CheckFailed {
                error: format!("{error:#}"),
            },
        };
        send_telemetry_from_ctx!(event, ctx);
    }
}

/// The result of the lightweight check phase of an update pass.
#[derive(Debug)]
enum CheckDecision {
    /// Nothing to install; the pass is complete with this outcome.
    Settled(UpdateOutcome),
    /// A newer version needs the install phase ([`install_update`]).
    NeedsInstall {
        latest_version: String,
        /// A previous check already staged this version's directory; only
        /// the `current` symlink still needs retargeting.
        already_staged: bool,
    },
}

/// Performs the check phase of an update pass: a single lightweight
/// `/client_version` request plus local filesystem checks, deciding whether
/// the (heavier) install phase is needed.
async fn check_for_update(layout: InstallLayout) -> Result<CheckDecision> {
    let current_version =
        ChannelState::app_version().context("no release version tag baked into this build")?;

    let client = http_client::Client::new();
    let latest_version = fetch_latest_version(&client).await?;

    // Version strings become directory names below; reject anything that
    // doesn't parse as a Warp version outright.
    let latest_parsed = ParsedVersion::try_from(latest_version.as_str())
        .with_context(|| format!("invalid latest version {latest_version:?}"))?;
    if latest_version.contains(['/', '\\']) {
        bail!("invalid latest version {latest_version:?}");
    }

    // Only ever move strictly forward. If the server reports an older (or
    // equal) version — e.g. a rollback — keep the running build; users can
    // reinstall a pinned version via the install script.
    let current_parsed = ParsedVersion::try_from(current_version)
        .with_context(|| format!("invalid current version {current_version:?}"))?;
    if latest_parsed <= current_parsed {
        return Ok(CheckDecision::Settled(UpdateOutcome::UpToDate {
            version: current_version.to_owned(),
        }));
    }

    let version_dir = layout.versions_dir.join(&latest_version);
    if version_dir == layout.running_version_dir {
        bail!("refusing to overwrite the running version directory {version_dir:?}");
    }

    // If a previous check already staged this version and pointed `current`
    // at it, there is nothing left to do until the user restarts. Like the
    // staging validation, don't treat a symlinked binary as staged.
    let already_staged = fs::symlink_metadata(version_dir.join(&layout.binary_name))
        .is_ok_and(|metadata| metadata.file_type().is_file());
    if already_staged && current_points_at(&layout, &latest_version) {
        return Ok(CheckDecision::Settled(UpdateOutcome::PendingRestart {
            version: latest_version,
        }));
    }

    Ok(CheckDecision::NeedsInstall {
        latest_version,
        already_staged,
    })
}

/// Performs the install phase of an update pass: downloads and stages
/// `latest_version` (unless already staged) and retargets `current` at it.
async fn install_update(
    layout: InstallLayout,
    latest_version: String,
    already_staged: bool,
) -> Result<UpdateOutcome> {
    // Serialize installs across concurrent TUI processes.
    let Some(_lock) = InstallLock::acquire(&layout.root)? else {
        return Ok(UpdateOutcome::Locked);
    };

    if !already_staged {
        let client = http_client::Client::new();
        let version_dir = layout.versions_dir.join(&latest_version);
        download_and_stage(&layout, &client, &latest_version, &version_dir).await?;
    }

    point_current_at(&layout, &latest_version)?;
    prune_old_versions(&layout, &latest_version).await;

    Ok(UpdateOutcome::Installed {
        version: latest_version,
    })
}

/// Whether the `current` symlink points at `versions/<version>`.
fn current_points_at(layout: &InstallLayout, version: &str) -> bool {
    fs::read_link(&layout.current_link).is_ok_and(|target| {
        target
            .file_name()
            .is_some_and(|name| name == std::ffi::OsStr::new(version))
    })
}

/// Fetches the latest version for this channel: from the Warp server's
/// `/client_version` endpoint, falling back to the channel-versions JSON in
/// GCP storage (mirroring the GUI autoupdater's fallback).
async fn fetch_latest_version(client: &http_client::Client) -> Result<String> {
    let server_url = format!(
        "{}/client_version?include_changelogs=false",
        ChannelState::server_root_url().trim_end_matches('/')
    );
    let from_server: Result<ChannelVersions> = async {
        let response = client
            .get(server_url.as_str())
            .timeout(FETCH_VERSIONS_TIMEOUT)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }
    .await;

    let versions = match from_server {
        Ok(versions) => versions,
        Err(error) => {
            let releases_base_url = ChannelState::releases_base_url();
            if releases_base_url.is_empty() {
                return Err(error.context("failed to fetch channel versions from the Warp server"));
            }
            log::warn!(
                "Failed to fetch channel versions from the Warp server ({error:#}); \
                 falling back to GCP JSON storage"
            );
            // The nonce busts any CDN/browser-style caching of the JSON file.
            let url = format!(
                "{}/channel_versions.json?r={}",
                releases_base_url.trim_end_matches('/'),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            );
            let response = client
                .get(url.as_str())
                .timeout(FETCH_VERSIONS_TIMEOUT)
                .send()
                .await?
                .error_for_status()?;
            response.json().await?
        }
    };

    latest_version_for_channel(&versions)
}

/// Picks this channel's latest version out of the channel-versions payload.
fn latest_version_for_channel(versions: &ChannelVersions) -> Result<String> {
    let channel_version = match ChannelState::channel() {
        Channel::Dev => &versions.dev,
        Channel::Preview => &versions.preview,
        Channel::Stable => &versions.stable,
        channel @ (Channel::Local | Channel::Oss | Channel::Integration) => {
            bail!("no TUI release artifacts exist for the {channel} channel")
        }
    };
    Ok(channel_version.version_info().version)
}

/// The server's `channel` query parameter for the current channel. Only
/// channels accepted by [`latest_version_for_channel`] reach this.
fn download_channel() -> &'static str {
    match ChannelState::channel() {
        Channel::Preview => "preview",
        Channel::Stable => "stable",
        Channel::Dev | Channel::Local | Channel::Oss | Channel::Integration => "dev",
    }
}

/// The server's `os` query parameter for this build's platform, or `None` on
/// platforms that can never have TUI release artifacts. Deriving this from
/// the build target (instead of hard-coding macOS) guarantees e.g. a Linux
/// build can never download and stage a macOS artifact; on platforms without
/// artifacts, [`AutoupdateEligibility::determine`] disables updates entirely.
fn download_os() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("macos")
    } else if cfg!(target_os = "linux") {
        Some("linux")
    } else {
        None
    }
}

/// Downloads and stages `version` into `version_dir`: stream the tarball into
/// a staging directory next to `versions/`, extract and validate it, then
/// atomically rename it into place. The staging directory lives on the same
/// filesystem so the final move is a cheap rename, and it is cleaned up on
/// any failure.
async fn download_and_stage(
    layout: &InstallLayout,
    client: &http_client::Client,
    version: &str,
    version_dir: &Path,
) -> Result<()> {
    let os = download_os().context("no TUI release artifacts exist for this platform")?;
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    let url = format!(
        "{}/download/tui?os={os}&arch={arch}&channel={}&version={version}",
        ChannelState::server_root_url().trim_end_matches('/'),
        download_channel(),
    );

    async_fs::create_dir_all(&layout.versions_dir)
        .await
        .with_context(|| format!("failed to create {:?}", layout.versions_dir))?;
    let staging_dir = layout
        .versions_dir
        .join(format!(".staging-{version}-{}", std::process::id()));
    let _cleanup = RemoveDirOnDrop(staging_dir.clone());
    // Clear any leftovers from a previous crashed attempt by this same pid.
    let _ = async_fs::remove_dir_all(&staging_dir).await;
    async_fs::create_dir_all(&staging_dir)
        .await
        .with_context(|| format!("failed to create staging dir {staging_dir:?}"))?;

    // Stream the tarball straight to disk instead of buffering it in memory
    // (the artifact is tens of MBs), mirroring the GUI's DMG download.
    log::info!("TUI autoupdate: downloading version {version}");
    let response = client
        .get(url.as_str())
        .timeout(DOWNLOAD_TIMEOUT)
        .send()
        .await
        .context("failed to download the TUI update")?
        .error_for_status()
        .context("failed to download the TUI update")?;
    let tarball_path = staging_dir.join("warp-tui.tar.gz");
    let mut tarball = async_fs::File::create(&tarball_path)
        .await
        .with_context(|| format!("failed to create {tarball_path:?}"))?;
    futures_lite::io::copy(
        response
            .bytes_stream()
            .map_err(std::io::Error::other)
            .into_async_read(),
        &mut tarball,
    )
    .await
    .context("failed to download the TUI update")?;
    tarball.sync_data().await?;
    drop(tarball);

    // Extract the tarball (binary + sibling resources/ tree) into a payload
    // directory, using the system tar like the install script does.
    let payload_dir = staging_dir.join("payload");
    async_fs::create_dir_all(&payload_dir).await?;
    let output = command::r#async::Command::new("tar")
        .arg("xzf")
        .arg(&tarball_path)
        .arg("-C")
        .arg(&payload_dir)
        .output()
        .await
        .context("failed to run tar to extract the TUI update")?;
    if !output.status.success() {
        bail!(
            "failed to extract the TUI update: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    // Validate the payload before touching the install, using symlink_metadata
    // so a crafted archive can't satisfy these checks with symlinks pointing
    // outside the staged payload: the binary and resources/ must be a regular
    // file and a real directory, not symlinks.
    let binary_path = payload_dir.join(&layout.binary_name);
    let binary_is_regular_file = async_fs::symlink_metadata(&binary_path)
        .await
        .is_ok_and(|metadata| metadata.file_type().is_file());
    if !binary_is_regular_file {
        bail!(
            "downloaded TUI archive did not contain expected binary {:?} as a regular file",
            layout.binary_name
        );
    }
    let resources_is_regular_dir = async_fs::symlink_metadata(payload_dir.join("resources"))
        .await
        .is_ok_and(|metadata| metadata.file_type().is_dir());
    if !resources_is_regular_dir {
        bail!("downloaded TUI archive did not contain the expected resources/ directory");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        async_fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))
            .await
            .context("failed to mark the TUI binary as executable")?;
    }

    // Standalone binaries can't have a notarization ticket stapled, so clear
    // any Gatekeeper quarantine attribute to avoid a first-run prompt.
    #[cfg(target_os = "macos")]
    {
        let _ = command::r#async::Command::new("xattr")
            .arg("-dr")
            .arg("com.apple.quarantine")
            .arg(&payload_dir)
            .output()
            .await;
    }

    // Move the validated payload into place. `version_dir` can only already
    // exist as a partial install (the validity check above failed for it), so
    // replacing it is safe — and it's never the running version's directory.
    let _ = async_fs::remove_dir_all(version_dir).await;
    async_fs::rename(&payload_dir, version_dir)
        .await
        .with_context(|| format!("failed to move the staged TUI update into {version_dir:?}"))?;

    Ok(())
}

/// Atomically points the `current` symlink at `versions/<version>` by staging
/// a new symlink and renaming it over the old one. `rename(2)` replaces the
/// destination link itself, so `current` never dangles mid-swap. These are
/// metadata-only operations, so plain (sync) fs calls are fine here.
#[cfg(unix)]
fn point_current_at(layout: &InstallLayout, version: &str) -> Result<()> {
    let staged_link = layout.root.join(".current.new");
    let _ = fs::remove_file(&staged_link);
    std::os::unix::fs::symlink(Path::new(VERSIONS_DIR_NAME).join(version), &staged_link)
        .context("failed to stage the new `current` symlink")?;
    fs::rename(&staged_link, &layout.current_link)
        .context("failed to retarget the `current` symlink")
}

#[cfg(not(unix))]
fn point_current_at(_layout: &InstallLayout, _version: &str) -> Result<()> {
    bail!("TUI auto-update is only supported on unix platforms")
}

/// Removes stale version directories, keeping the newly installed version,
/// the version the running binary is executing from, and whatever `current`
/// points at. Deleting a running version's directory would break its child
/// process spawning, so this errs on the side of keeping things.
async fn prune_old_versions(layout: &InstallLayout, new_version: &str) {
    let current_target = fs::read_link(&layout.current_link)
        .ok()
        .and_then(|target| target.file_name().map(|name| name.to_owned()));
    let running_version = layout
        .running_version_dir
        .file_name()
        .map(ToOwned::to_owned);

    let Ok(mut entries) = async_fs::read_dir(&layout.versions_dir).await else {
        return;
    };
    while let Some(Ok(entry)) = entries.next().await {
        let name = entry.file_name();
        // Skip staging dirs / lock leftovers, the new install, the running
        // version, and the current target.
        if name.to_string_lossy().starts_with('.')
            || name == *new_version
            || Some(&name) == running_version.as_ref()
            || Some(&name) == current_target.as_ref()
        {
            continue;
        }
        if entry
            .file_type()
            .await
            .is_ok_and(|file_type| file_type.is_dir())
        {
            log::info!("TUI autoupdate: pruning old version {name:?}");
            if let Err(error) = async_fs::remove_dir_all(entry.path()).await {
                log::warn!("TUI autoupdate: failed to prune {name:?}: {error}");
            }
        }
    }
}

/// An exclusive advisory lock over the install, backed by an `O_EXCL`-style
/// lock file under the install root. Removed on drop; locks older than
/// [`STALE_LOCK_AGE`] are assumed abandoned and broken.
struct InstallLock {
    path: PathBuf,
}

impl InstallLock {
    /// Attempts to take the lock. Returns `Ok(None)` when another live
    /// process holds it.
    fn acquire(root: &Path) -> Result<Option<Self>> {
        let path = root.join(LOCK_FILE_NAME);
        for attempt in 0..2 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Some(Self { path })),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let is_stale = fs::metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > STALE_LOCK_AGE);
                    if !is_stale || attempt > 0 {
                        return Ok(None);
                    }
                    log::warn!("TUI autoupdate: breaking stale install lock at {path:?}");
                    let _ = fs::remove_file(&path);
                }
                Err(error) => {
                    return Err(error).context(format!("failed to create lock file {path:?}"))
                }
            }
        }
        Ok(None)
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Removes a directory tree when dropped. Used to clean up the staging
/// directory on both success and failure.
struct RemoveDirOnDrop(PathBuf);

impl Drop for RemoveDirOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
#[path = "autoupdate_tests.rs"]
mod tests;
