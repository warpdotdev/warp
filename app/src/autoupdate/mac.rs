#![allow(deprecated)]

use std::ffi::{CString, OsString};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd as _;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{env, fs, str};

use anyhow::{Context, Result, anyhow, bail, ensure};
use channel_versions::VersionInfo;
use command::r#async::Command;
use command::blocking;
use futures::{StreamExt, TryStreamExt as _};
use futures_lite::future;
use instant::Instant;
use nix::errno::Errno;
use nix::unistd::{fchown, getgid, getuid};
use warp_core::macos::get_bundle_path;
use warp_core::safe_error;
use warp_errors::report_error;
use warpui::{AppContext, ModelContext, SingletonEntity};

use super::{DownloadReady, release_assets_directory_url};
use crate::appearance::AppearanceManager;
use crate::autoupdate::{AutoupdateStage, AutoupdateState};
use crate::channel::{Channel, ChannelState};
use crate::safe_info;

// Relative path to the directory containing old executables from before an autoupdate.
//
// TODO(vorporeal): This and relevant code should be deleted after auto-updates have been
//      storing the old executable in the user application data directory for a couple
//      releases.
const OLD_EXECUTABLE_PATH: &str = "Contents/MacOS/old";

// Name of the old executable file that was kept around during an autoupdate.
const OLD_EXECUTABLE_FILE_NAME: &str = "old";

// Tmp file name used to check if the user has the correct permissions for autoupdate.
const PERMISSIONS_TMP_FILE_NAME: &str = "permission_test";

fn old_executable_file_path() -> PathBuf {
    warp_core::paths::state_dir().join(OLD_EXECUTABLE_FILE_NAME)
}

/// Removes the old executable dir from the app bundle. This is necessary because after an
/// autoupdate deleting the running executable causes the pty to not start for a reason we don't
/// fully understand. This allows to clean up old executables when the app is first launched.
pub(super) fn remove_old_executable() -> Result<()> {
    // TODO(vorporeal): This code should be deleted after auto-updates have been
    //      storing the old executable in the user application data directory for
    //      a couple releases.
    log::info!("Removing old executable dir...");
    let old_executable_path = PathBuf::from(get_bundle_path()?).join(OLD_EXECUTABLE_PATH);
    if let Ok(metadata) = fs::metadata(&old_executable_path)
        && metadata.is_dir()
    {
        fs::remove_dir_all(old_executable_path)?;
    }

    log::info!("Removing old executable file...");
    let old_executable_file_path = old_executable_file_path();
    if let Ok(metadata) = fs::metadata(&old_executable_file_path)
        && metadata.is_file()
    {
        fs::remove_file(old_executable_file_path)?;
    }

    Ok(())
}

pub(super) fn manually_download_version(
    channel: &Channel,
    version_info: &VersionInfo,
    ctx: &mut AppContext,
) {
    let url = update_url(*channel, version_info.version.as_str());
    ctx.open_url(&url);
}

/// If the autoupdate state is ready, asynchronously apply the update and cleanup the autoupdate artifacts.
///
/// The completion callback is invoked with `Ok(Some(version))` if an update was applied, and `Ok(None)` if there was no update.
/// If there was an update, but applying it failed, it's invoked with `Err(err)`.
pub(super) fn apply_update_async<F>(app: &mut AppContext, callback: F)
where
    F: FnOnce(
            &mut AutoupdateState,
            Result<Option<VersionInfo>>,
            &mut ModelContext<AutoupdateState>,
        ) + Send
        + 'static,
{
    AutoupdateState::handle(app).update(app, |autoupdate_state, ctx| {
        match autoupdate_state.stage.clone() {
            AutoupdateStage::UpdateReady {
                new_version,
                update_id,
            }
            | AutoupdateStage::Updating {
                new_version,
                update_id,
            } => {
                let update_id_clone = update_id.clone();
                // Apply the update in a background thread.
                ctx.spawn(
                    async move {
                        let result =
                            apply_update(ChannelState::channel(), &new_version, &update_id)
                                .await
                                .map(|_| Some(new_version));
                        cleanup(&update_id).await;
                        result
                    },
                    move |autoupdate_state, result, ctx| {
                        if result.is_ok() {
                            // Reset app icon to previously selected app icon
                            AppearanceManager::as_ref(ctx).set_app_icon(ctx);
                        }
                        autoupdate_state.clear_downloaded_update(&update_id_clone, ctx);
                        callback(autoupdate_state, result, ctx);
                    },
                );
            }
            _ => {
                callback(autoupdate_state, Ok(None), ctx);
            }
        }
    })
}

pub(super) fn relaunch() -> Result<()> {
    let bundle_path = PathBuf::from(get_bundle_path()?);
    // Set the -n option to open a new instance of the app even if one is
    // running so we still launch the new version even if the user was running
    // multiple instances of Warp.
    let mut launch_command = OsString::from("/usr/bin/open -n ");
    launch_command.push(bundle_path.as_os_str());
    // Pass a flag to the app to let it know it was restarted as part of the
    // autoupdate process.
    launch_command.push(format!(" --args {}", warp_cli::finish_update_flag()));
    // If we're testing with a local copy of channel_versions.json, have the
    // newly-started binary also reference that same file (so we can test
    // displaying an updated changelog after an autoupdate).
    if let Ok(path) = env::var("WARP_CHANNEL_VERSIONS_PATH") {
        launch_command.push(format!(" --env WARP_CHANNEL_VERSIONS_PATH={path}"));
    }

    // We need to make sure that the current Warp process is no longer running
    // before we spawn the new one, otherwise we can end up showing multiple
    // icons in the macOS dock.  To do this, we use an intermediary /bin/sh
    // process that watches for this process to terminate, and then spawns a
    // new Warp process.
    //
    // Wait until the current process is no longer running, checking every
    // 200ms.  Once the current process has terminated, launch the new one.
    let pid = std::process::id();
    let mut relaunch_command = OsString::from(format!(
        "while ps -p {pid} >/dev/null 2>&1; do sleep 0.2; done; "
    ));
    relaunch_command.push(launch_command);

    log::info!("Executing relaunch command {relaunch_command:?}");
    blocking::Command::new("sh")
        .arg("-c")
        .arg(relaunch_command)
        .spawn()?;
    Ok(())
}

pub async fn cleanup(update_id: &str) {
    let download_dir = get_download_dir(update_id);
    if download_dir.exists() {
        log::info!("Cleaning up download dir {:?}", &download_dir);
        if let Err(e) = async_fs::remove_dir_all(&download_dir).await {
            safe_error!(
                safe: ("Error cleaning up download dir: {e:?}"),
                full: ("Error cleaning up download dir {:?}: {:?}", &download_dir, e)
            );
        }
    }
}

/// Clean up all autoupdate directories except the specified one.
/// This helps prevent accumulation of old update directories from failed downloads,
/// race conditions, or incomplete cleanups.
pub async fn cleanup_all_except(preserve_update_id: Option<&str>) {
    let mut autoupdate_dir = warp_core::paths::cache_dir();
    autoupdate_dir.push("autoupdate");

    if !autoupdate_dir.exists() {
        return;
    }

    log::debug!("Cleaning up all autoupdate directories except {preserve_update_id:?}");

    let mut entries = match async_fs::read_dir(&autoupdate_dir).await {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("Could not read autoupdate directory {autoupdate_dir:?}: {e:?}");
            return;
        }
    };

    while let Some(entry) = entries.next().await {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                log::warn!("Error reading autoupdate directory entry: {e:?}");
                continue;
            }
        };

        let path = entry.path();
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name,
            None => continue,
        };

        // Skip the directory we want to preserve
        if let Some(preserve_id) = preserve_update_id
            && file_name == preserve_id
        {
            log::debug!("Preserving autoupdate directory: {path:?}");
            continue;
        }

        let metadata = match async_fs::metadata(&path).await {
            Ok(metadata) => metadata,
            Err(e) => {
                log::warn!("Could not get metadata for {path:?}: {e:?}");
                continue;
            }
        };

        if metadata.is_dir() {
            log::debug!("Removing old autoupdate directory: {path:?}");
            if let Err(e) = async_fs::remove_dir_all(&path).await {
                log::warn!("Failed to remove autoupdate directory {path:?}: {e:?}");
            }
        }
    }
}

/// Determines if the user needs authorization in order to update Warp.
async fn needs_authorization(bundle_path: &Path) -> Result<bool> {
    // For the bundle path itself, check permissions without creating a test file so as to not
    // interfere with code signing.
    let bundle_dir_writable = permissions::is_writable(bundle_path)?;
    if !bundle_dir_writable {
        log::info!("App location is not writable, needs authorization");
        return Ok(true);
    } else {
        log::info!("App location is writable");
    }

    if let Some(bundle_parent_path) = bundle_path.parent() {
        if !is_directory_writable(bundle_parent_path).await? {
            log::info!("App parent location is not writable, needs authorization");
            return Ok(true);
        } else {
            log::info!("App parent location is writable");
        }
    }

    Ok(false)
}

/// Determines if a directory is writable as part of an update. This means:
/// * Warp can create files in the directory
/// * Warp can modify the permissions of created files
async fn is_directory_writable(directory: &Path) -> Result<bool> {
    // Just because we have writability access does not mean we can set the correct owner/group.
    // Test if we can set the owner/group on a temporarily created file. If we can, then we can
    // probably perform an update without authorization.
    let tmp_file_name = directory.join(PERMISSIONS_TMP_FILE_NAME);

    safe_info!(
        safe: ("Writing to a tmp file to determine if permissions are correct"),
        full: ("Writing to a tmp file to determine if permissions are correct in {}", directory.display())
    );

    let needs_authorization = match async_fs::File::create(&tmp_file_name).await {
        Ok(file) => {
            let fchown_result = fchown(file.as_raw_fd(), Some(getuid()), Some(getgid()));
            if let Err(err) = &fchown_result {
                log::warn!("Could not set permissions on tmp file: {err:#}");
            }

            // Only remove the tmp file if it was created - otherwise, we'll mask permission
            // errors.
            async_fs::remove_file(&tmp_file_name).await?;
            fchown_result.is_ok()
        }
        Err(e) => {
            // Obvious indicator we may need authorization.
            log::warn!("Could not create tmp file: {e:#}");
            false
        }
    };

    Ok(needs_authorization)
}

/// Verifies that the staged bundle path has a valid macOS code signature, and that its
/// team identifier matches Warp's team identifier.
async fn verify_code_signature(component: &str, path: &Path) -> Result<()> {
    // Verify the signature of the staged update bundle with team identifier
    let codesign_verify_output = Command::new("/usr/bin/codesign")
        .arg("-v")
        .arg(format!(
            "-R=certificate leaf[subject.OU] = \"{}\"",
            warp_core::macos::APPLE_TEAM_ID
        ))
        .arg(path)
        .output()
        .await?;
    ensure!(
        codesign_verify_output.status.success(),
        "Failed to verify code signature for {component} with team identifier: {codesign_verify_output:?}"
    );

    safe_info!(
        safe: ("Code signature is valid for {component}"),
        full: ("Code signature is valid for {}", path.display())
    );

    Ok(())
}

pub(super) async fn download_update_and_cleanup(
    version_info: &VersionInfo,
    update_id: &str,
    last_successful_update_id: Option<&str>,
    client: &http_client::Client,
) -> Result<DownloadReady> {
    let result =
        download_and_extract_binary(ChannelState::channel(), version_info, update_id, client).await;
    if result.is_err() {
        cleanup_all_except(last_successful_update_id).await;
    }
    result
}

/// Apply the downloaded update.
///
/// This is async and should be run in a background task.
async fn apply_update(channel: Channel, version_info: &VersionInfo, update_id: &str) -> Result<()> {
    let update_start = Instant::now();

    let bundle_path = PathBuf::from(get_bundle_path()?);
    let bundle_parent_path = bundle_path
        .parent()
        .ok_or_else(|| anyhow!("Could not get parent directory of application bundle"))?;

    // Double-check that we have permissions to apply the update.
    if !permissions::is_writable(&bundle_path)? {
        bail!("App location is not writable, cannot apply update");
    }
    if !is_directory_writable(bundle_parent_path).await? {
        bail!("App parent location is not writable, cannot apply update");
    }

    // Read a file out of the old bundle to ensure that we've triggered macOS' directory
    // permissions checks.
    let old_info_plist = bundle_path.join("Contents/Info.plist");
    if async_fs::File::open(&old_info_plist).await.is_err() {
        bail!("App location is not readable, cannot apply update");
    }

    let dmg_path = dmg_path(&channel, version_info, update_id);
    let temp_app_path = temporary_target_path(channel, version_info, &dmg_path)?;

    let staged_bundle =
        StagedBundle::for_bundle_path(channel, version_info, temp_app_path, &bundle_path).await?;

    // Copy permissions to new app
    let bundle_metadata = async_fs::metadata(&bundle_path).await?;
    async_fs::set_permissions(&staged_bundle.path, bundle_metadata.permissions()).await?;

    // Verify that the new version actually exists before proceeding
    let executable_path_buf = staged_bundle.path.join(executable_path(channel));
    if !executable_path_buf.exists() {
        bail!(
            "New executable does not exist at path: {:?}",
            executable_path_buf
        );
    }

    // Atomically rename the new app to have the same name as the old one.
    log::info!("Renaming new app to original app name");
    let from = CString::new(staged_bundle.path.as_os_str().as_bytes())?;
    let to = CString::new(bundle_path.as_os_str().as_bytes())?;

    Errno::result(unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_SWAP) })
        .context("Error swapping old and new app bundles")?;

    // Move the current running executable into a temporary directory so we can delete the
    // rest of the old bundle without removing the running executable (since removing it
    // causes the `fork` syscall to fail).
    let executable_temp_file = old_executable_file_path();
    if async_fs::metadata(executable_temp_file.as_path())
        .await
        .is_ok()
    {
        // If we performed this process already but didn't relaunch Warp, the old executable will
        // still be located in the user application data directory.  In that case, leave it there.
        log::info!("Already autoupdated without relaunching; ignoring executable from old bundle");
    } else {
        // Compute the location of the old executable (which, after the swap of the app contents,
        // is located in the "new app" directory).
        let new_app_executable_path = staged_bundle.path.join(executable_path(channel));

        log::info!(
            "Moving old executable at path {new_app_executable_path:?} into user application data dir at path {executable_temp_file:?}"
        );
        let mv_output = Command::new("mv")
            .arg(new_app_executable_path)
            .arg(executable_temp_file)
            .output()
            .await?;

        ensure!(
            mv_output.status.success(),
            "Failed to move old executable: {mv_output:?}"
        );
    }

    log::info!("Setting installed version to {:?}", &version_info);
    log::info!("Applied update in {:?}", update_start.elapsed());

    Ok(())
}

/// The staged app bundle that we're about to install. It's copied out of the `.dmg` file into a
/// temporary location.
struct StagedBundle {
    /// Path to the on-disk temporary bundle.
    path: PathBuf,
    /// Whether or not the temporary bundle was copied into the same directory as the existing app.
    /// This is only necessary if `$TMPDIR` and the app are on different filesystems.
    in_app_directory: bool,
}

impl StagedBundle {
    async fn for_bundle_path(
        channel: Channel,
        version_info: &VersionInfo,
        temp_app_path: PathBuf,
        bundle_path: &Path,
    ) -> Result<Self> {
        let temp_device_id = async_fs::metadata(&temp_app_path)
            .await
            .context("Could not get metadata for temporary app bundle")?
            .dev();
        let bundle_device_id = async_fs::metadata(bundle_path)
            .await
            .context("Could not get metadata for app bundle")?
            .dev();

        if temp_device_id == bundle_device_id {
            // The old and new app bundles are on the same filesystem (this is the expected case).
            Ok(Self {
                path: temp_app_path,
                in_app_directory: false,
            })
        } else {
            let bundle_parent_path = bundle_path
                .parent()
                .ok_or_else(|| anyhow!("Could not get parent directory of application bundle"))?;
            log::info!("Copying app contents from {temp_app_path:?} to {bundle_parent_path:?}");

            let cp_output = Command::new("cp")
                // Recursively copy the directory, preserving symlinks.
                .arg("-R")
                // Overwrite files at the destination.
                .arg("-f")
                .arg(&temp_app_path)
                .arg(bundle_parent_path)
                .output()
                .await?;

            ensure!(
                cp_output.status.success(),
                "Failed to copy app contents from temporary directory into bundle directory: {cp_output:?}"
            );

            Ok(Self {
                path: bundle_parent_path.join(versioned_app_name(channel, &version_info.version)),
                in_app_directory: true,
            })
        }
    }
}

impl Drop for StagedBundle {
    fn drop(&mut self) {
        // Clean up in the destructor so that it happens even if the installation errors.
        // If we used the original temporary app bundle, it'll get removed by the final cleanup
        // step, along with the dmg.
        if self.in_app_directory {
            log::info!("Removing temporary app bundle");
            if let Err(err) = fs::remove_dir_all(&self.path) {
                report_error!(anyhow::Error::new(err).context("Failed to remove temporary bundle"));
            }
        }
    }
}

async fn download_and_extract_binary(
    channel: Channel,
    version_info: &VersionInfo,
    update_id: &str,
    client: &http_client::Client,
) -> Result<DownloadReady> {
    let bundle_path = PathBuf::from(get_bundle_path()?);
    let needs_authorization = needs_authorization(bundle_path.as_path())
        .await
        .unwrap_or(true);
    if needs_authorization {
        return Ok(DownloadReady::NeedsAuthorization);
    }

    log::info!(
        "Downloading update, version {} on channel {channel}",
        &version_info.version,
    );

    let download_dir = get_download_dir(update_id);
    log::info!("Creating download dir {:?}", &download_dir);
    async_fs::create_dir_all(&download_dir).await?;

    let dmg_path = download_dmg(&channel, version_info, update_id, client).await?;

    // Mount the downloaded dmg so we can copy out the binary.
    let mountpoint = mount_dmg(&dmg_path, update_id).await?;

    let target = temporary_target_path(channel, version_info, &dmg_path)?;
    // Copy the binary into the temporary directory where we downloaded the dmg.
    copy_app_from_dmg(&channel, &mountpoint, &target).await?;

    // Unmount the dmg once we no longer need it. This prevents lingering images from unapplied
    // updates.
    if let Err(err) = unmount_dmg(mountpoint).await {
        let err = err.context("Error unmounting dmg for update");
        warp_errors::report_error!(&err);
    }

    // Ensure that the new app we just downloaded has both integrity (e.g. no corrupted files)
    // and validity (it was signed by us).
    // Store the executable path in a variable to prevent temporary value issues.
    let executable_path_buf = target.join(executable_path(channel));
    let verification_start = Instant::now();
    future::try_zip(
        verify_code_signature("bundle", &target),
        verify_code_signature("executable", executable_path_buf.as_path()),
    )
    .await?;

    log::info!(
        "Verified new app code signature in {:?}",
        verification_start.elapsed()
    );

    Ok(DownloadReady::Yes)
}

async fn unmount_dmg(mountpoint: PathBuf) -> Result<()> {
    let mut hdiutil_cmd = Command::new("/usr/bin/hdiutil");
    hdiutil_cmd.arg("detach");
    hdiutil_cmd.arg(&mountpoint);
    hdiutil_cmd.arg("-force");

    log::info!("Attempting to detach dmg with command \"{hdiutil_cmd:?}\"");

    let output = hdiutil_cmd.output().await?;

    ensure!(output.status.success(), "Failed to detach dmg: {output:?}");
    log::info!("hdiutil detach succeeded: {output:?}");
    Ok(())
}

async fn copy_app_from_dmg(channel: &Channel, mountpoint: &Path, target: &Path) -> Result<()> {
    let mounted_app_path = mountpoint.join(app_name(*channel));

    log::info!("Copying dmg contents from {mounted_app_path:?} to {target:?}");

    let cp_output = Command::new("cp")
        // Recursively copy the directory, preserving symlinks.
        .arg("-R")
        .arg(mounted_app_path)
        .arg(target)
        .output()
        .await?;

    ensure!(
        cp_output.status.success(),
        "Failed to copy app out of mounted dmg: {cp_output:?}"
    );

    Ok(())
}

// 10 minutes
const DMG_TIMEOUT_S: u64 = 600;

/// The temporary path for downloading the new dmg into.
fn dmg_path(channel: &Channel, version_info: &VersionInfo, update_id: &str) -> PathBuf {
    let mut dir = get_download_dir(update_id);
    let file_name = format!(
        "{}.{}.dmg",
        &version_info.version,
        app_name_prefix(*channel)
    );
    dir.push(file_name);
    dir
}

/// The temporary path for placing our downloaded app binary.
fn temporary_target_path(
    channel: Channel,
    version_info: &VersionInfo,
    dmg_path: &Path,
) -> Result<PathBuf> {
    Ok(dmg_path
        .parent()
        .ok_or_else(|| anyhow!("Could not get parent directory of downloaded DMG"))?
        .join(versioned_app_name(channel, &version_info.version)))
}

async fn download_dmg(
    channel: &Channel,
    version_info: &VersionInfo,
    update_id: &str,
    client: &http_client::Client,
) -> Result<PathBuf> {
    // TODO: Use a streaming fetch and and provide an api for tracking progress
    let update_url = update_url(*channel, &version_info.version);
    log::info!("Fetching new dmg at {update_url}");
    let res = client
        .get(&update_url)
        .timeout(Duration::from_secs(DMG_TIMEOUT_S))
        .send()
        .await?;
    let dmg_file = dmg_path(channel, version_info, update_id);

    let mut file = async_fs::File::create(&dmg_file).await?;
    futures_lite::io::copy(
        res.bytes_stream()
            .map_err(std::io::Error::other)
            .into_async_read(),
        &mut file,
    )
    .await?;
    file.sync_data().await?;

    log::info!("Wrote DMG to tempfile at {:?}", &dmg_file);
    Ok(dmg_file)
}

fn get_download_dir(update_id: &str) -> PathBuf {
    let mut dir = warp_core::paths::cache_dir();
    dir.push("autoupdate");
    dir.push(update_id);
    dir
}

fn get_mountpoint(update_id: &str) -> PathBuf {
    let mut volume = PathBuf::from("/Volumes");
    volume.push(update_id);
    volume
}

async fn mount_dmg(dmg_dir: &Path, update_id: &str) -> Result<PathBuf> {
    let volume = get_mountpoint(update_id);
    let mut hdiutil_cmd = Command::new("/usr/bin/hdiutil");
    hdiutil_cmd.args(["attach", "-mountpoint"]);
    hdiutil_cmd.arg(&volume);
    // Explanation of flags:
    // -nobrowse: Do not show the Warp DMG in Finder or similar apps.
    // -noautoopen: Do not open the Warp DMG in Finder.
    // -readonly: For safety, we mount read-only since there's no need to modify the new app version.
    // -autofsck: Ensure that the DMG contents are verified. This is on by default for quarantined images, but macOS
    //    doesn't necessarily recognize our download as such.
    hdiutil_cmd.args(["-nobrowse", "-noautoopen", "-readonly", "-autofsck"]);
    hdiutil_cmd.arg(dmg_dir);

    log::info!("Attempting to mount dmg with command \"{hdiutil_cmd:?}\"");

    let output = hdiutil_cmd.output().await?;

    ensure!(output.status.success(), "Failed to mount dmg: {output:?}");

    log::info!("hdiutil mount succeeded");
    Ok(volume)
}

fn update_url(channel: Channel, version: &str) -> String {
    format!(
        "{}/{}",
        release_assets_directory_url(channel, version),
        dmg_name(channel)
    )
}

fn app_name(channel: Channel) -> String {
    format!("{}.app", app_name_prefix(channel))
}

fn versioned_app_name(channel: Channel, version: &str) -> String {
    format!("{}({}).app", app_name_prefix(channel), version)
}

fn dmg_name(channel: Channel) -> String {
    // If the user is on an Apple Silicon Mac, download an arm64-only bundle.
    let is_arm64 = command::blocking::Command::new("uname")
        .arg("-m")
        .output()
        .is_ok_and(|output| output.stdout.starts_with(b"arm64"));
    if is_arm64 {
        return format!("{}-arm64.dmg", app_name_prefix(channel));
    }

    // Otherwise, download a universal bundle.
    format!("{}.dmg", app_name_prefix(channel))
}

fn app_name_prefix(channel: Channel) -> &'static str {
    match channel {
        Channel::Stable => "Warp",
        Channel::Preview => "WarpPreview",
        Channel::Local => "warp",
        Channel::Integration => "integration",
        Channel::Dev => "WarpDev",
        Channel::Oss => "warp-oss",
    }
}

fn executable_name(channel: Channel) -> &'static str {
    match channel {
        Channel::Stable => "stable",
        Channel::Preview => "preview",
        Channel::Local => "warp",
        Channel::Integration => "integration",
        Channel::Dev => "dev",
        Channel::Oss => "warp-oss",
    }
}

fn executable_path(channel: Channel) -> String {
    if ChannelState::is_release_bundle() {
        format!("Contents/MacOS/{}", executable_name(channel))
    } else {
        executable_name(channel).to_owned()
    }
}
