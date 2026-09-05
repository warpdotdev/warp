use std::path::Path;

use remote_server::transport::Error;

use crate::remote_server::tarball_cache;

/// Installs the remote server via SCP fallback.
///
/// The tarball is downloaded or reused from the local cache first, then uploaded
/// to the remote host and passed to the install script as an already-downloaded
/// archive. This avoids requiring the remote host to download the tarball itself.
pub(super) async fn install(socket_path: &Path) -> Result<(), Error> {
    let platform = super::super::detect_remote_platform(socket_path).await?;

    let client_tarball_path = tarball_cache::cached_remote_server_tarball(&platform)
        .await
        .map_err(Error::Other)?;
    let timeout = remote_server::setup::SCP_INSTALL_TIMEOUT;
    let install_dir = remote_server::setup::remote_server_dir();
    let remote_tarball_name = format!("oz-upload-{}.tar.gz", uuid::Uuid::new_v4());
    let remote_tarball_path = format!("{install_dir}/{remote_tarball_name}");

    // The normal install script creates this directory before downloading, but
    // SCP fallback can run after a failure that happened before that point.
    // Ensure the destination exists before uploading the staged tarball.
    let mkdir_output = remote_server::ssh::run_ssh_command(
        socket_path,
        &format!("mkdir -p {install_dir}"),
        remote_server::setup::CHECK_TIMEOUT,
    )
    .await
    .map_err(Error::from)?;
    if !mkdir_output.status.success() {
        let code = mkdir_output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&mkdir_output.stderr).to_string();
        return Err(Error::ScriptFailed {
            exit_code: code,
            stderr,
        });
    }

    log::info!("Uploading tarball to remote at {remote_tarball_path}");
    remote_server::ssh::scp_upload(
        socket_path,
        &client_tarball_path,
        &remote_tarball_path,
        timeout,
    )
    .await
    .map_err(Error::Other)?;

    log::info!("Running extraction via install script with tarball at {remote_tarball_path}");
    let script = remote_server::setup::install_script(Some(&remote_tarball_path));

    let output = remote_server::ssh::run_ssh_script(socket_path, &script, timeout)
        .await
        .map_err(Error::from)?;
    if output.status.success() {
        Ok(())
    } else {
        let code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Err(Error::ScriptFailed {
            exit_code: code,
            stderr,
        })
    }
}
