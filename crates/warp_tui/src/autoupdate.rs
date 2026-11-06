//! Background auto-updater for the headless `warp-tui` front-end.
//!
//! Follows the "native installer" model used by peer CLIs (e.g. Claude Code):
//! the installer (`warp-server/download/tui_install.sh`) lays installs out as
//!
//! ```text
//! <root>/                      # ~/.warp/tui by default
//!   versions/<version>/        # binary + resources/ per installed version
//!   current                    # active version symlink (Unix) or text pointer (Windows)
//! <bin-dir>/warp[-<channel>]   # stable symlink (Unix) or launcher (Windows)
//! ```
//!
//! and this module keeps that layout fresh: it polls on the same cadence as
//! the GUI autoupdater (each poll is a single lightweight `/client_version`
//! request). Unix stages release archives directly; Windows downloads and
//! executes the same signed Inno installer used for initial installation.
//! Both paths activate immutable versions without touching the running
//! session. Managed processes hold shared per-version leases for their
//! lifetime, and cleanup only removes inactive versions whose lease can be
//! locked exclusively.
//!
//! Native background updates only run for managed installs (i.e. when the
//! running executable resolves into a `versions/` directory). Recognized
//! Homebrew installs check for newer versions but leave installation to
//! Homebrew, while `cargo run` builds and legacy flat installs are unaffected.
//! Users can opt out with the file-backed `general.autoupdate_enabled` setting
//! or the `WARP_TUI_DISABLE_AUTOUPDATE` environment variable; re-running the
//! install script remains available as a manual escape hatch.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use channel_versions::{ChannelVersions, ParsedVersion};
use futures::TryStreamExt as _;
use warp::settings::TuiAutoupdateSettings;
use warp_core::channel::{Channel, ChannelState};
use warp_core::{safe_warn, send_telemetry_from_ctx};
use warpui::r#async::Timer;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::telemetry::TuiAutoupdateTelemetryEvent;

/// Setting this environment variable (to any value) disables background
/// auto-updates for a single launch, regardless of the
/// `general.autoupdate_enabled` setting.
const DISABLE_ENV_VAR: &str = "WARP_TUI_DISABLE_AUTOUPDATE";
/// The stable cask token shared by the Warp-managed tap and the planned
/// official Homebrew package.
const HOMEBREW_CASK_TOKEN: &str = "warp-agent-cli";

/// User-facing status for Homebrew installations with a newer cask.
pub(crate) const HOMEBREW_UPDATE_STATUS: &str =
    "update available — run brew upgrade --cask warp-agent-cli";

/// Name of the directory holding per-version installs under the install root.
const VERSIONS_DIR_NAME: &str = "versions";

/// Name of the platform-specific pointer to the active version.
const CURRENT_POINTER_NAME: &str = "current";
/// Name of the Windows text pointer to the prior valid version.
#[cfg(windows)]
const PREVIOUS_POINTER_NAME: &str = "previous";
/// Directory under the install root holding stable per-version lease files.
const VERSION_LEASES_DIR_NAME: &str = "version-leases";

/// Atomic directory lock shared by the Unix Rust and shell installers.
#[cfg(unix)]
const LOCK_FILE_NAME: &str = ".update.lock";

/// Debug metadata written inside [`LOCK_FILE_NAME`].
#[cfg(unix)]
const LOCK_OWNER_FILE_NAME: &str = "owner";

/// How often to check for updates. Mirrors the GUI autoupdater's poll
/// interval (`AutoupdateState::AUTOUPDATE_POLL`); each check is a single
/// lightweight `/client_version` request unless a new version actually needs
/// downloading.
const CHECK_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// A lock file held for longer than this is considered abandoned (e.g. a
/// crashed updater) and is broken.
#[cfg(unix)]
const STALE_LOCK_AGE: Duration = Duration::from_secs(60 * 60);

/// Timeout for the (small) channel-versions fetch.
const FETCH_VERSIONS_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for downloading a TUI release artifact.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Disambiguates generated staging paths and install-lock owner tokens when
/// their process ID and timestamp components happen to match.
static NEXT_UNIQUE_ID: AtomicU64 = AtomicU64::new(0);
const MAX_STAGING_DIR_ATTEMPTS: usize = 100;

/// The managed, versioned install layout the running binary belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
struct InstallLayout {
    /// Root of the versioned install (e.g. `~/.warp/tui`).
    root: PathBuf,
    /// `<root>/versions`.
    versions_dir: PathBuf,
    /// `<root>/current`, the platform-specific active-version pointer.
    current_pointer: PathBuf,
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
        let exe = std::env::current_exe().ok()?;
        // Fall back to the kernel-reported path if canonicalization loses a
        // startup race with GC. Its `versions/<version>` shape still proves
        // this is managed, so lease validation must fail closed.
        let canonical_exe = exe.canonicalize().unwrap_or(exe);
        Self::from_canonical_exe_path(&canonical_exe)
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
            current_pointer: root.join(CURRENT_POINTER_NAME),
            root,
            versions_dir,
            running_version_dir,
            binary_name,
        })
    }
}

/// The installation owner inferred from the canonical running executable.
#[derive(Clone, Debug, PartialEq, Eq)]
enum InstallMethod {
    /// Installed by Warp's native versioned installer.
    Native(InstallLayout),
    /// Installed from the `warp-agent-cli` Homebrew cask.
    Homebrew,
    /// A development build, legacy flat install, or unknown package manager.
    Unmanaged,
}

impl InstallMethod {
    fn detect() -> Self {
        let Ok(exe) = std::env::current_exe() else {
            return Self::Unmanaged;
        };
        let canonical_exe = exe.canonicalize().unwrap_or(exe);
        Self::from_canonical_exe_path(&canonical_exe)
    }

    fn from_canonical_exe_path(exe: &Path) -> Self {
        if let Some(layout) = InstallLayout::from_canonical_exe_path(exe) {
            return Self::Native(layout);
        }
        if is_homebrew_cask_exe_path(exe) {
            return Self::Homebrew;
        }
        Self::Unmanaged
    }
}

fn is_homebrew_cask_exe_path(exe: &Path) -> bool {
    if exe.file_name() != Some(OsStr::new("warp-tui-stable")) {
        return false;
    }
    let components = exe
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(component) => Some(component),
            std::path::Component::Prefix(_)
            | std::path::Component::RootDir
            | std::path::Component::CurDir
            | std::path::Component::ParentDir => None,
        })
        .collect::<Vec<_>>();
    components.windows(2).any(|components| {
        components[0] == OsStr::new("Caskroom") && components[1] == OsStr::new(HOMEBREW_CASK_TOKEN)
    })
}

fn download_url(version: &str) -> Result<String> {
    let os = download_os().context("no TUI release artifacts exist for this platform")?;
    let arch = download_arch();
    Ok(format!(
        "{}{}?os={os}&arch={arch}&version={version}",
        ChannelState::server_root_url().trim_end_matches('/'),
        download_endpoint(ChannelState::channel()),
    ))
}

#[cfg(unix)]
fn read_current_pointer(layout: &InstallLayout) -> Result<OsString> {
    fs::read_link(&layout.current_pointer)
        .with_context(|| {
            format!(
                "failed to read current TUI symlink {:?}",
                layout.current_pointer
            )
        })?
        .file_name()
        .map(ToOwned::to_owned)
        .context("current TUI symlink has no version component")
}

#[cfg(windows)]
fn read_current_pointer(layout: &InstallLayout) -> Result<OsString> {
    let version = fs::read_to_string(&layout.current_pointer)
        .with_context(|| {
            format!(
                "failed to read current TUI version pointer {:?}",
                layout.current_pointer
            )
        })?
        .trim()
        .to_owned();
    if !is_safe_version_component(&version) {
        bail!("current TUI version pointer contains an invalid version {version:?}");
    }
    Ok(version.into())
}

#[cfg(windows)]
fn previous_pointer_version(layout: &InstallLayout) -> Option<OsString> {
    let version = fs::read_to_string(layout.root.join(PREVIOUS_POINTER_NAME)).ok()?;
    let version = version.trim();
    is_safe_version_component(version).then(|| version.into())
}

#[cfg(not(windows))]
fn previous_pointer_version(_layout: &InstallLayout) -> Option<OsString> {
    None
}

#[cfg(not(any(unix, windows)))]
fn read_current_pointer(_layout: &InstallLayout) -> Result<OsString> {
    bail!("TUI auto-update is not supported on this platform")
}

fn version_lease_path(root: &Path, version: &OsStr) -> PathBuf {
    let mut lease_name = version.to_os_string();
    lease_name.push(".lock");
    root.join(VERSION_LEASES_DIR_NAME).join(lease_name)
}

fn open_version_lease(root: &Path, version: &OsStr) -> Result<fs::File> {
    let lease_dir = root.join(VERSION_LEASES_DIR_NAME);
    fs::create_dir_all(&lease_dir)
        .with_context(|| format!("failed to create TUI version lease directory {lease_dir:?}"))?;
    let lease_path = version_lease_path(root, version);
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lease_path)
        .with_context(|| format!("failed to open TUI version lease {lease_path:?}"))
}

/// A process-lifetime shared lease protecting one managed version directory
/// from garbage collection. Dropping the file releases the OS lock.
pub(crate) struct VersionLease {
    _file: fs::File,
}

impl VersionLease {
    /// Acquires a lease for the version containing the running executable.
    /// Unmanaged Cargo builds and legacy flat installs do not participate.
    pub(crate) fn acquire_for_current_process() -> Result<Option<Self>> {
        InstallLayout::detect()
            .map(|layout| {
                Self::acquire(&layout)
                    .context(
                        "failed to protect this managed Warp Agent CLI version; retry the \
                         command, or reinstall Warp Agent CLI if the problem persists",
                    )
                    .map(Some)
            })
            .unwrap_or(Ok(None))
    }

    /// Acquires and validates the running version's shared lease.
    fn acquire(layout: &InstallLayout) -> Result<Self> {
        let version = layout
            .running_version_dir
            .file_name()
            .context("managed TUI version directory has no version name")?;
        let lease_path = version_lease_path(&layout.root, version);
        let file = open_version_lease(&layout.root, version)?;
        fs4::fs_std::FileExt::lock_shared(&file)
            .with_context(|| format!("failed to acquire TUI version lease {lease_path:?}"))?;

        if !is_complete_version_dir(layout, &layout.running_version_dir) {
            bail!(
                "the managed Warp Agent CLI version at {:?} was retired while this process was \
                 starting; retry the command, or reinstall Warp Agent CLI if the problem persists",
                layout.running_version_dir
            );
        }

        Ok(Self { _file: file })
    }
}

/// The result of a single update check.
#[derive(Debug)]
enum UpdateOutcome {
    /// Skipped: another process is installing an update right now.
    #[cfg(unix)]
    Locked,
    /// The running build is already the channel's latest version.
    UpToDate { version: String },
    /// A newer version was already staged by a previous check and `current`
    /// points at it; nothing to do until the next launch.
    PendingRestart { version: String },
    /// A newer version was staged and `current` now points at it. It takes
    /// effect on the next launch.
    Installed { version: String },
    /// A newer Homebrew cask is available and must be installed by Homebrew.
    UpdateAvailable { version: String },
}

impl UpdateOutcome {
    /// Stable identifier for this kind of outcome, used for telemetry and
    /// for detecting transitions between consecutive checks.
    fn kind(&self) -> &'static str {
        match self {
            #[cfg(unix)]
            UpdateOutcome::Locked => "locked",
            UpdateOutcome::UpToDate { .. } => "up_to_date",
            UpdateOutcome::PendingRestart { .. } => "pending_restart",
            UpdateOutcome::Installed { .. } => "installed",
            UpdateOutcome::UpdateAvailable { .. } => "update_available",
        }
    }

    /// The version associated with this outcome, if any.
    fn version(&self) -> Option<&str> {
        match self {
            #[cfg(unix)]
            UpdateOutcome::Locked => None,
            UpdateOutcome::UpToDate { version }
            | UpdateOutcome::PendingRestart { version }
            | UpdateOutcome::Installed { version }
            | UpdateOutcome::UpdateAvailable { version } => Some(version),
        }
    }
}

/// User-visible status of the background updater, shown next to the version
/// in the transcript zero state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TuiAutoupdateStatus {
    /// Nothing to show: updates are disabled for this process, or no check
    /// has produced a stable result yet.
    Idle,
    /// Fetching the latest version for this channel.
    Checking,
    /// Downloading and staging a newer version.
    Updating,
    /// The running build is the channel's latest version.
    UpToDate,
    /// The most recent check or update attempt failed.
    Failed,
    /// A newer version is staged and takes effect on the next launch.
    PendingRestart,
    /// A newer Homebrew cask is available.
    UpdateAvailable,
}

/// Events emitted by [`TuiAutoupdater`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TuiAutoupdaterEvent {
    /// [`TuiAutoupdater::status`] changed.
    StatusChanged,
}

/// Whether this process runs the background update loop.
///
/// Package-manager-owned installs use explicit variants because each manager
/// can require different version checks, status copy, and update commands.
#[derive(Clone, Debug)]
enum AutoupdateEligibility {
    /// Eligible for native background installation.
    Native(InstallLayout),
    /// Eligible for update checks, with installation owned by Homebrew.
    Homebrew,
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
        match InstallMethod::detect() {
            InstallMethod::Native(layout) => Self::Native(layout),
            InstallMethod::Homebrew => Self::Homebrew,
            InstallMethod::Unmanaged => Self::Disabled {
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
/// [`AutoupdateEligibility::Native`] or [`AutoupdateEligibility::Homebrew`].
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
            AutoupdateEligibility::Native(layout) => me.check_native_now(layout, ctx),
            AutoupdateEligibility::Homebrew => me.check_homebrew_now(ctx),
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
    fn check_native_now(&mut self, layout: InstallLayout, ctx: &mut ModelContext<Self>) {
        // Where the status settles when this pass is skipped because another
        // process is installing, and the status to preserve once an update is
        // pending restart.
        let fallback_status = self.status;
        self.set_status(TuiAutoupdateStatus::Checking, ctx);
        let check_layout = layout.clone();
        ctx.spawn(
            async move { check_for_update(check_layout).await },
            move |me, decision, ctx| match decision {
                Ok(CheckDecision::Settled(outcome)) => {
                    me.finish_check(Ok(outcome), fallback_status, layout, ctx);
                }
                Ok(CheckDecision::NeedsInstall { latest_version }) => {
                    me.set_status(TuiAutoupdateStatus::Updating, ctx);
                    let install_layout = layout.clone();
                    ctx.spawn(
                        async move { install_update(install_layout, latest_version).await },
                        move |me, result, ctx| {
                            me.finish_check(result, fallback_status, layout, ctx);
                        },
                    );
                }
                Err(error) => me.finish_check(Err(error), fallback_status, layout, ctx),
            },
        );
    }

    /// Checks for a newer Homebrew cask without invoking Homebrew or modifying
    /// the Caskroom-managed installation.
    fn check_homebrew_now(&mut self, ctx: &mut ModelContext<Self>) {
        let fallback_status = self.status;
        self.set_status(TuiAutoupdateStatus::Checking, ctx);
        ctx.spawn(check_homebrew_update(), move |me, result, ctx| {
            me.finish_homebrew_check(result, fallback_status, ctx)
        });
    }

    fn finish_homebrew_check(
        &mut self,
        result: Result<UpdateOutcome>,
        fallback_status: TuiAutoupdateStatus,
        ctx: &mut ModelContext<Self>,
    ) {
        match &result {
            Ok(outcome) => log::info!("TUI Homebrew update check finished: {outcome:?}"),
            Err(error) => log::warn!("TUI Homebrew update check failed: {error:#}"),
        }
        self.report_outcome(&result, ctx);
        let status = settled_status(&result, fallback_status);
        self.set_status(status, ctx);
        ctx.spawn(
            async { Timer::after(CHECK_INTERVAL).await },
            move |me, _, ctx| me.check_homebrew_now(ctx),
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
            // Let the next poll retry; transient network errors (e.g. waking
            // from sleep) are common here.
            Err(error) => log::warn!("TUI autoupdate check failed: {error:#}"),
        }
        self.report_outcome(&result, ctx);
        let status = settled_status(&result, fallback_status);
        self.set_status(status, ctx);
        ctx.spawn(
            async { Timer::after(CHECK_INTERVAL).await },
            move |me, _, ctx| me.check_native_now(layout, ctx),
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

fn settled_status(
    result: &Result<UpdateOutcome>,
    fallback_status: TuiAutoupdateStatus,
) -> TuiAutoupdateStatus {
    // Once an update is staged, only a restart clears it: never downgrade
    // from `PendingRestart` (e.g. on a failed check or server-side rollback).
    if fallback_status == TuiAutoupdateStatus::PendingRestart {
        return TuiAutoupdateStatus::PendingRestart;
    }

    match result {
        Ok(UpdateOutcome::UpToDate { .. }) => TuiAutoupdateStatus::UpToDate,
        Ok(UpdateOutcome::PendingRestart { .. } | UpdateOutcome::Installed { .. }) => {
            TuiAutoupdateStatus::PendingRestart
        }
        Ok(UpdateOutcome::UpdateAvailable { .. }) => TuiAutoupdateStatus::UpdateAvailable,
        #[cfg(unix)]
        Ok(UpdateOutcome::Locked) => fallback_status,
        Err(_) => TuiAutoupdateStatus::Failed,
    }
}

/// The result of the lightweight check phase of an update pass.
#[derive(Debug)]
enum CheckDecision {
    /// Nothing to install; the pass is complete with this outcome.
    Settled(UpdateOutcome),
    /// A newer version needs the install phase ([`install_update`]).
    NeedsInstall { latest_version: String },
}

/// Performs the check phase of an update pass: a single lightweight
/// `/client_version` request plus local filesystem checks, deciding whether
/// the (heavier) install phase is needed.
async fn check_for_update(layout: InstallLayout) -> Result<CheckDecision> {
    let current_version =
        ChannelState::app_version().context("no release version tag baked into this build")?;

    let client = http_client::Client::new();
    let latest_version = fetch_latest_version(&client).await?;
    if !is_newer_version(current_version, &latest_version)? {
        return Ok(CheckDecision::Settled(UpdateOutcome::UpToDate {
            version: current_version.to_owned(),
        }));
    }

    let version_dir = layout.versions_dir.join(&latest_version);
    if version_dir == layout.running_version_dir {
        bail!("refusing to overwrite the running version directory {version_dir:?}");
    }

    // Errors from this check abort only the current background update pass.
    // `finish_check` restores the previous status and schedules the next poll.
    match version_dir_state(&layout, &version_dir)? {
        VersionDirState::Complete if current_points_at(&layout, &latest_version) => {
            return Ok(CheckDecision::Settled(UpdateOutcome::PendingRestart {
                version: latest_version,
            }));
        }
        VersionDirState::Invalid => {
            bail!(
                "refusing to replace incomplete or invalid installed TUI version at \
                 {version_dir:?}; remove that directory or reinstall Warp Agent CLI, then retry"
            );
        }
        VersionDirState::Complete | VersionDirState::Missing => {}
    }

    Ok(CheckDecision::NeedsInstall { latest_version })
}

async fn check_homebrew_update() -> Result<UpdateOutcome> {
    let current_version =
        ChannelState::app_version().context("no release version tag baked into this build")?;
    let latest_version = fetch_latest_version(&http_client::Client::new()).await?;
    homebrew_update_outcome(current_version, latest_version)
}

fn homebrew_update_outcome(current_version: &str, latest_version: String) -> Result<UpdateOutcome> {
    if is_newer_version(current_version, &latest_version)? {
        Ok(UpdateOutcome::UpdateAvailable {
            version: latest_version,
        })
    } else {
        Ok(UpdateOutcome::UpToDate {
            version: current_version.to_owned(),
        })
    }
}

fn is_newer_version(current_version: &str, latest_version: &str) -> Result<bool> {
    let latest_parsed = ParsedVersion::try_from(latest_version)
        .with_context(|| format!("invalid latest version {latest_version:?}"))?;
    if !is_safe_version_component(latest_version) {
        bail!("invalid latest version {latest_version:?}");
    }
    let current_parsed = ParsedVersion::try_from(current_version)
        .with_context(|| format!("invalid current version {current_version:?}"))?;
    Ok(latest_parsed > current_parsed)
}

fn is_safe_version_component(version: &str) -> bool {
    !version.is_empty()
        && !version.contains("..")
        && version.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
        && is_safe_windows_path_component(version)
        && Path::new(version)
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}
fn is_safe_windows_path_component(component: &str) -> bool {
    if component.is_empty()
        || matches!(component, "." | "..")
        || component.ends_with([' ', '.'])
        || component
            .chars()
            .any(|character| character.is_control() || r#"<>:"/\|?*"#.contains(character))
    {
        return false;
    }

    let stem = component
        .split_once('.')
        .map_or(component, |(stem, _)| stem)
        .to_ascii_uppercase();
    !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !(stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

/// Performs the Unix install phase. Download and extraction happen without
/// the global install lock; finalization and activation are serialized.
#[cfg(unix)]
async fn install_update(layout: InstallLayout, latest_version: String) -> Result<UpdateOutcome> {
    let version_dir = layout.versions_dir.join(&latest_version);
    let staged = match version_dir_state(&layout, &version_dir)? {
        VersionDirState::Complete => None,
        VersionDirState::Missing => {
            let client = http_client::Client::new();
            Some(download_update(&layout, &client, &latest_version).await?)
        }
        VersionDirState::Invalid => {
            bail!(
                "refusing to replace incomplete or invalid installed TUI version at \
                 {version_dir:?}; remove that directory or reinstall Warp Agent CLI, then retry"
            );
        }
    };
    blocking::unblock(move || {
        let Some(_lock) = InstallLock::acquire(&layout.root)? else {
            return Ok(UpdateOutcome::Locked);
        };
        // Another installer may have completed this exact immutable version
        // while this process downloaded, so recheck under the install lock.
        match version_dir_state(&layout, &version_dir)? {
            VersionDirState::Complete => {}
            VersionDirState::Missing => {
                let staged = staged.context(
                    "the completed TUI version disappeared before activation; retry the update",
                )?;
                finalize_staged_version(&layout, &latest_version, staged, &version_dir)?;
            }
            VersionDirState::Invalid => {
                bail!(
                    "refusing to replace incomplete or invalid installed TUI version at \
                     {version_dir:?}; remove that directory or reinstall Warp Agent CLI, then retry"
                );
            }
        }

        point_current_at(&layout, &latest_version)?;
        prune_old_versions(&layout, &latest_version);

        Ok(UpdateOutcome::Installed {
            version: latest_version,
        })
    })
    .await
}

/// Builds the Inno Setup `/DIR` argument naming the install root.
///
/// [`InstallLayout::detect`] canonicalizes the running executable, which on
/// Windows yields a Win32 extended-length path (`\\?\C:\...`). Inno Setup
/// validates `/DIR` as a plain folder name and rejects the `?` and second `:`
/// that prefix introduces, reporting "Folder names cannot include any of the
/// following characters" and exiting with code 3 before installing anything.
/// `/SUPPRESSMSGBOXES` hides that dialog, so the failure would otherwise
/// surface only as a bare exit code.
#[cfg(windows)]
fn installer_dir_argument(root: &Path) -> OsString {
    let mut argument = OsString::from("/DIR=");
    argument.push(dunce::simplified(root));
    argument
}

#[cfg(windows)]
async fn install_update(layout: InstallLayout, latest_version: String) -> Result<UpdateOutcome> {
    let client = http_client::Client::new();
    let installer = download_windows_installer(&layout, &client, &latest_version).await?;
    let install_dir = installer_dir_argument(&layout.root);

    let output = command::r#async::Command::new(&installer.path)
        .args([
            OsString::from("/SP-"),
            OsString::from("/VERYSILENT"),
            OsString::from("/SUPPRESSMSGBOXES"),
            OsString::from("/NOCANCEL"),
            OsString::from("/NORESTART"),
            OsString::from("/CURRENTUSER"),
            OsString::from("/update=1"),
            OsString::from("/NOCLOSEAPPLICATIONS"),
            install_dir,
            OsString::from("/skip_path_update=1"),
        ])
        .output()
        .await
        .context("failed to launch the Warp Agent CLI installer")?;
    if !output.status.success() {
        bail!(
            "Warp Agent CLI installer exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    blocking::unblock(move || {
        let version_dir = layout.versions_dir.join(&latest_version);
        if version_dir_state(&layout, &version_dir)? != VersionDirState::Complete {
            bail!("Warp Agent CLI installer did not create a complete version at {version_dir:?}");
        }
        if !current_points_at(&layout, &latest_version) {
            bail!("Warp Agent CLI installer did not activate expected version {latest_version:?}");
        }
        prune_old_versions(&layout, &latest_version);
        Ok(UpdateOutcome::Installed {
            version: latest_version,
        })
    })
    .await
}

#[cfg(not(any(unix, windows)))]
async fn install_update(_layout: InstallLayout, _latest_version: String) -> Result<UpdateOutcome> {
    bail!("TUI auto-update is not supported on this platform")
}

/// State of an immutable final version path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VersionDirState {
    Missing,
    Complete,
    Invalid,
}

/// A complete version is a real directory containing the expected regular
/// binary and a real `resources/` directory. Symlinks never satisfy these
/// checks.
fn is_complete_version_dir(layout: &InstallLayout, version_dir: &Path) -> bool {
    fs::symlink_metadata(version_dir).is_ok_and(|metadata| metadata.file_type().is_dir())
        && fs::symlink_metadata(version_dir.join(&layout.binary_name))
            .is_ok_and(|metadata| metadata.file_type().is_file())
        && fs::symlink_metadata(version_dir.join("resources"))
            .is_ok_and(|metadata| metadata.file_type().is_dir())
}

fn version_dir_state(layout: &InstallLayout, version_dir: &Path) -> Result<VersionDirState> {
    match fs::symlink_metadata(version_dir) {
        Ok(_) if is_complete_version_dir(layout, version_dir) => Ok(VersionDirState::Complete),
        Ok(_) => Ok(VersionDirState::Invalid),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(VersionDirState::Missing),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect TUI version directory {version_dir:?}")),
    }
}

/// Whether the platform-specific `current` pointer names `version`.
fn current_points_at(layout: &InstallLayout, version: &str) -> bool {
    read_current_pointer(layout).is_ok_and(|current| current == OsStr::new(version))
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
    latest_version_for(ChannelState::channel(), versions)
}

fn latest_version_for(channel: Channel, versions: &ChannelVersions) -> Result<String> {
    let channel_version = match channel {
        Channel::Dev => &versions.dev,
        Channel::Preview => &versions.preview,
        Channel::Stable => &versions.stable,
        channel @ (Channel::Local | Channel::Oss | Channel::Integration) => {
            bail!("no TUI release artifacts exist for the {channel} channel")
        }
    };
    Ok(channel_version.version_info().tui_version().to_owned())
}

/// The Warp Agent CLI artifact endpoint for a release channel.
fn download_endpoint(channel: Channel) -> &'static str {
    match channel {
        Channel::Preview => "/download/agent-cli-preview/artifact",
        Channel::Stable => "/download/agent-cli/artifact",
        Channel::Dev | Channel::Local | Channel::Oss | Channel::Integration => {
            "/download/agent-cli-dev/artifact"
        }
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
    } else if cfg!(target_os = "windows") {
        Some("windows")
    } else {
        None
    }
}

fn download_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    }
}

#[cfg(windows)]
struct DownloadedInstaller {
    path: PathBuf,
}

#[cfg(windows)]
impl Drop for DownloadedInstaller {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(windows)]
async fn download_windows_installer(
    layout: &InstallLayout,
    client: &http_client::Client,
    version: &str,
) -> Result<DownloadedInstaller> {
    async_fs::create_dir_all(&layout.root)
        .await
        .with_context(|| format!("failed to create TUI install root {:?}", layout.root))?;
    let mut installer = None;
    for _ in 0..MAX_STAGING_DIR_ATTEMPTS {
        let path = layout.root.join(format!(
            ".warp-agent-cli-update-{}-{}.exe",
            std::process::id(),
            NEXT_UNIQUE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        match async_fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(file) => {
                installer = Some((path, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to create installer at {path:?}"));
            }
        }
    }
    let (path, mut file) =
        installer.context("failed to allocate a unique Warp Agent CLI installer path")?;
    let installer = DownloadedInstaller { path };
    let response = client
        .get(download_url(version)?.as_str())
        .timeout(DOWNLOAD_TIMEOUT)
        .send()
        .await
        .context("failed to download the Warp Agent CLI installer")?
        .error_for_status()
        .context("failed to download the Warp Agent CLI installer")?;
    futures_lite::io::copy(
        response
            .bytes_stream()
            .map_err(std::io::Error::other)
            .into_async_read(),
        &mut file,
    )
    .await
    .context("failed to download the Warp Agent CLI installer")?;
    file.sync_data().await?;
    drop(file);
    Ok(installer)
}

/// A validated Unix update payload staged next to the final version directories.
#[cfg(unix)]
struct StagedUpdate {
    staging_dir: PathBuf,
    payload_dir: PathBuf,
}

#[cfg(unix)]
impl StagedUpdate {
    fn finalize(self, version_dir: &Path) -> Result<()> {
        fs::rename(&self.payload_dir, version_dir)
            .with_context(|| format!("failed to move the staged TUI update into {version_dir:?}"))
    }
}

#[cfg(unix)]
fn finalize_staged_version(
    layout: &InstallLayout,
    version: &str,
    staged: StagedUpdate,
    version_dir: &Path,
) -> Result<()> {
    staged.finalize(version_dir)?;
    open_version_lease(&layout.root, OsStr::new(version))
        .context("failed to mark the finalized TUI version as lease-aware")?;
    Ok(())
}

#[cfg(unix)]
impl Drop for StagedUpdate {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.staging_dir);
    }
}

#[cfg(unix)]
async fn create_unique_staging_dir(versions_dir: &Path, version: &str) -> Result<PathBuf> {
    create_unique_staging_dir_with(|| {
        let staging_id = NEXT_UNIQUE_ID.fetch_add(1, Ordering::Relaxed);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        versions_dir.join(format!(
            ".staging-{version}-{}-{timestamp}-{staging_id}",
            std::process::id()
        ))
    })
    .await
}

#[cfg(any(unix, test))]
async fn create_unique_staging_dir_with(
    mut next_candidate: impl FnMut() -> PathBuf,
) -> Result<PathBuf> {
    for _ in 0..MAX_STAGING_DIR_ATTEMPTS {
        let staging_dir = next_candidate();
        match async_fs::create_dir(&staging_dir).await {
            Ok(()) => return Ok(staging_dir),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to create staging dir {staging_dir:?}"));
            }
        }
    }
    bail!("failed to allocate a unique TUI update staging directory")
}

#[cfg(unix)]
async fn download_update(
    layout: &InstallLayout,
    client: &http_client::Client,
    version: &str,
) -> Result<StagedUpdate> {
    async_fs::create_dir_all(&layout.versions_dir)
        .await
        .with_context(|| format!("failed to create {:?}", layout.versions_dir))?;
    let staging_dir = create_unique_staging_dir(&layout.versions_dir, version).await?;
    let _cleanup = RemoveDirOnDrop(staging_dir.clone());

    log::info!("TUI autoupdate: downloading version {version}");
    let response = client
        .get(download_url(version)?.as_str())
        .timeout(DOWNLOAD_TIMEOUT)
        .send()
        .await
        .context("failed to download the TUI update")?
        .error_for_status()
        .context("failed to download the TUI update")?;
    let archive_path = staging_dir.join("warp-tui.tar.gz");
    let mut archive_file = async_fs::File::create(&archive_path)
        .await
        .with_context(|| format!("failed to create {archive_path:?}"))?;
    futures_lite::io::copy(
        response
            .bytes_stream()
            .map_err(std::io::Error::other)
            .into_async_read(),
        &mut archive_file,
    )
    .await
    .context("failed to download the TUI update")?;
    archive_file.sync_data().await?;
    drop(archive_file);

    let payload_dir = staging_dir.join("payload");
    async_fs::create_dir_all(&payload_dir).await?;
    let output = command::r#async::Command::new("tar")
        .arg("xzf")
        .arg(&archive_path)
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

    use std::os::unix::fs::PermissionsExt as _;
    async_fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))
        .await
        .context("failed to mark the TUI binary as executable")?;

    #[cfg(target_os = "macos")]
    {
        let _ = command::r#async::Command::new("xattr")
            .arg("-dr")
            .arg("com.apple.quarantine")
            .arg(&payload_dir)
            .output()
            .await;
    }

    std::mem::forget(_cleanup);
    Ok(StagedUpdate {
        staging_dir,
        payload_dir,
    })
}

/// Atomically points `current` at `versions/<version>`.
#[cfg(unix)]
fn point_current_at(layout: &InstallLayout, version: &str) -> Result<()> {
    let staged_link = layout.root.join(".current.new");
    let _ = fs::remove_file(&staged_link);
    std::os::unix::fs::symlink(Path::new(VERSIONS_DIR_NAME).join(version), &staged_link)
        .context("failed to stage the new `current` symlink")?;
    fs::rename(&staged_link, &layout.current_pointer)
        .context("failed to retarget the `current` symlink")
}

#[cfg(not(any(unix, windows)))]
fn point_current_at(_layout: &InstallLayout, _version: &str) -> Result<()> {
    bail!("TUI auto-update is not supported on this platform")
}

/// Removes inactive, lease-aware versions while the caller holds the global
/// install lock. Unmarked versions predate this protocol and are retained.
fn prune_old_versions(layout: &InstallLayout, new_version: &str) {
    let running_version = layout
        .running_version_dir
        .file_name()
        .map(ToOwned::to_owned);
    let previous_version = previous_pointer_version(layout);
    let entries = match fs::read_dir(&layout.versions_dir) {
        Ok(entries) => entries,
        Err(error) => {
            safe_warn!(
                safe: ("TUI autoupdate: failed to inspect installed versions: {error}"),
                full: (
                    "TUI autoupdate: failed to inspect installed versions at {:?}: {error}",
                    layout.versions_dir
                )
            );
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                log::warn!("TUI autoupdate: failed to inspect installed version: {error}");
                continue;
            }
        };
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.')
            || name == *new_version
            || Some(&name) == running_version.as_ref()
            || Some(&name) == previous_version.as_ref()
        {
            continue;
        }
        let is_dir = match entry.file_type() {
            Ok(file_type) => file_type.is_dir(),
            Err(error) => {
                log::warn!("TUI autoupdate: retaining {name:?}; inspection failed: {error}");
                continue;
            }
        };
        if !is_dir || !is_complete_version_dir(layout, &entry.path()) {
            log::info!(
                "TUI autoupdate: retaining {name:?}; it is not a completed version directory"
            );
            continue;
        }

        let mut lease_name = name.clone();
        lease_name.push(".lock");
        let lease_path = layout.root.join(VERSION_LEASES_DIR_NAME).join(lease_name);
        let lease_is_file = match fs::symlink_metadata(&lease_path) {
            Ok(metadata) => metadata.file_type().is_file(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                log::info!("TUI autoupdate: retaining unmarked version {name:?}");
                continue;
            }
            Err(error) => {
                safe_warn!(
                    safe: (
                        "TUI autoupdate: retaining {name:?}; failed to inspect its lease: {error}"
                    ),
                    full: (
                        "TUI autoupdate: retaining {name:?}; failed to inspect lease \
                         {lease_path:?}: {error}"
                    )
                );
                continue;
            }
        };
        if !lease_is_file {
            safe_warn!(
                safe: (
                    "TUI autoupdate: retaining {name:?}; its lease path is not a regular file"
                ),
                full: (
                    "TUI autoupdate: retaining {name:?}; lease path is not a regular file: \
                     {lease_path:?}"
                )
            );
            continue;
        }
        let lease = match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lease_path)
        {
            Ok(lease) => lease,
            Err(error) => {
                safe_warn!(
                    safe: (
                        "TUI autoupdate: retaining {name:?}; failed to open its lease: {error}"
                    ),
                    full: (
                        "TUI autoupdate: retaining {name:?}; failed to open lease \
                         {lease_path:?}: {error}"
                    )
                );
                continue;
            }
        };
        match fs4::fs_std::FileExt::try_lock_exclusive(&lease) {
            Ok(true) => {}
            Ok(false) => {
                log::info!("TUI autoupdate: retaining live version {name:?}");
                continue;
            }
            Err(error) => {
                safe_warn!(
                    safe: (
                        "TUI autoupdate: retaining {name:?}; failed to lock its lease: {error}"
                    ),
                    full: (
                        "TUI autoupdate: retaining {name:?}; failed to lock lease \
                         {lease_path:?}: {error}"
                    )
                );
                continue;
            }
        }

        // `current` may have changed while the lease was being acquired.
        // Re-read it immediately before deletion.
        match read_current_pointer(layout) {
            Ok(current) if current == name => {
                log::info!("TUI autoupdate: retaining current version {name:?}");
                continue;
            }
            Ok(_) => {}
            Err(error) => {
                safe_warn!(
                    safe: (
                        "TUI autoupdate: retaining {name:?}; failed to inspect current pointer: \
                         {error}"
                    ),
                    full: (
                        "TUI autoupdate: retaining {name:?}; failed to inspect current pointer \
                         {:?}: {error}",
                        layout.current_pointer
                    )
                );
                continue;
            }
        }
        log::info!("TUI autoupdate: pruning inactive version {name:?}");
        if let Err(error) = fs::remove_dir_all(entry.path()) {
            log::warn!("TUI autoupdate: failed to prune {name:?}: {error}");
        }
    }
}

/// An owned Unix install lock with stale-process recovery.
#[cfg(unix)]
struct InstallLock {
    path: PathBuf,
    owner: String,
}

#[cfg(unix)]
impl InstallLock {
    fn acquire(root: &Path) -> Result<Option<Self>> {
        Self::acquire_with_stale_age(root, STALE_LOCK_AGE)
    }

    fn acquire_with_stale_age(root: &Path, stale_age: Duration) -> Result<Option<Self>> {
        let path = root.join(LOCK_FILE_NAME);
        let owner = format!(
            "{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            NEXT_UNIQUE_ID.fetch_add(1, Ordering::Relaxed)
        );
        for attempt in 0..2 {
            match fs::create_dir(&path) {
                Ok(()) => {
                    let owner_path = path.join(LOCK_OWNER_FILE_NAME);
                    if let Err(error) = fs::write(&owner_path, &owner) {
                        let _ = fs::remove_dir_all(&path);
                        return Err(error)
                            .with_context(|| format!("failed to write lock owner {owner_path:?}"));
                    }
                    return Ok(Some(Self {
                        path,
                        owner: owner.clone(),
                    }));
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let metadata = match fs::symlink_metadata(&path) {
                        Ok(metadata) => metadata,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) => {
                            return Err(error).with_context(|| {
                                format!("failed to inspect install lock {path:?}")
                            });
                        }
                    };
                    let is_stale = metadata
                        .modified()
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > stale_age);
                    if !is_stale || attempt > 0 {
                        return Ok(None);
                    }
                    safe_warn!(
                        safe: ("TUI autoupdate: breaking stale install lock"),
                        full: ("TUI autoupdate: breaking stale install lock at {path:?}")
                    );
                    let remove_result = if metadata.file_type().is_dir() {
                        fs::remove_dir_all(&path)
                    } else {
                        fs::remove_file(&path)
                    };
                    if let Err(error) = remove_result
                        && error.kind() != std::io::ErrorKind::NotFound
                    {
                        safe_warn!(
                            safe: ("TUI autoupdate: failed to break stale install lock: {error}"),
                            full: (
                                "TUI autoupdate: failed to break stale install lock at \
                                 {path:?}: {error}"
                            )
                        );
                        return Ok(None);
                    }
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("failed to create lock directory {path:?}"));
                }
            }
        }
        Ok(None)
    }
}

#[cfg(unix)]
impl Drop for InstallLock {
    fn drop(&mut self) {
        let owner_path = self.path.join(LOCK_OWNER_FILE_NAME);
        if fs::read_to_string(&owner_path).is_ok_and(|owner| owner == self.owner)
            && let Err(error) = fs::remove_dir_all(&self.path)
        {
            safe_warn!(
                safe: ("TUI autoupdate: failed to release install lock: {error}"),
                full: (
                    "TUI autoupdate: failed to release install lock at {:?}: {error}",
                    self.path
                )
            );
        }
    }
}

/// Removes a directory tree when dropped. Used to clean up the staging
/// directory on both success and failure.
#[cfg(unix)]
struct RemoveDirOnDrop(PathBuf);
#[cfg(unix)]
impl Drop for RemoveDirOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
#[path = "autoupdate_tests.rs"]
mod tests;
