use std::path::Path;

use anyhow::Result;
use remote_server::transport::{Error, InstallOutcome, InstallSource};

use super::{
    DevContainerTransport, DockerCommandError, detect_remote_platform, run_docker_command,
    run_docker_script,
};
use crate::remote_server::tarball_cache;

pub(super) async fn install_binary(transport: &DevContainerTransport) -> InstallOutcome {
    let binary_path = remote_server::setup::remote_server_binary();
    log::info!("Installing remote server binary to {binary_path}");
    let mut outcome = match install_in_container(transport).await {
        Ok(()) => InstallOutcome {
            source: Some(InstallSource::Server),
            result: Ok(()),
        },
        Err(server_err) => {
            if tarball_cache::should_try_client_copy(&server_err) {
                log::info!("In-container install failed; falling back to docker cp");
                match install_via_client_copy(transport).await {
                    Ok(()) => InstallOutcome {
                        source: Some(InstallSource::Client),
                        result: Ok(()),
                    },
                    Err(e) => InstallOutcome {
                        source: Some(InstallSource::Client),
                        result: Err(e),
                    },
                }
            } else {
                InstallOutcome {
                    source: Some(InstallSource::Server),
                    result: Err(server_err),
                }
            }
        }
    };

    if outcome.result.is_ok() {
        log::info!("Running post-install verification for {binary_path}");
        let check_cmd = remote_server::setup::binary_check_command();
        let args = transport.command_args(&check_cmd);
        let verify = run_docker_command(
            &transport.docker_path,
            &args,
            remote_server::setup::CHECK_TIMEOUT,
        )
        .await;
        match verify {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                let code = output.status.code().unwrap_or(-1);
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                outcome.result = Err(Error::Other(anyhow::anyhow!(
                    "Post-install verification failed: binary not found or not \
                     executable at {binary_path} (exit {code}): {stderr}"
                )));
            }
            Err(e) => {
                outcome.result = Err(Error::Other(anyhow::anyhow!(
                    "Post-install verification failed: {e}"
                )));
            }
        }
    }

    outcome
}

async fn install_in_container(transport: &DevContainerTransport) -> Result<(), Error> {
    let script = remote_server::setup::install_script(None);
    let args = transport.script_args();
    match run_docker_script(
        &transport.docker_path,
        &args,
        &script,
        remote_server::setup::INSTALL_TIMEOUT,
    )
    .await
    {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let exit_code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(Error::ScriptFailed { exit_code, stderr })
        }
        Err(DockerCommandError::TimedOut { .. }) => Err(Error::TimedOut),
        Err(e) => Err(Error::Other(e.into())),
    }
}

async fn install_via_client_copy(transport: &DevContainerTransport) -> Result<(), Error> {
    let platform = detect_remote_platform(transport).await?;
    let client_tarball_path = tarball_cache::cached_remote_server_tarball(&platform)
        .await
        .map_err(Error::Other)?;
    let timeout = remote_server::setup::SCP_INSTALL_TIMEOUT;
    let home = container_home(transport).await?;
    let install_dir = expand_home_path(&home, &remote_server::setup::remote_server_dir());
    let remote_tarball_name = format!("oz-upload-{}.tar.gz", uuid::Uuid::new_v4());
    let remote_tarball_path = format!("{install_dir}/{remote_tarball_name}");

    let mkdir_args = transport.command_args(&format!("mkdir -p {install_dir}"));
    let mkdir_output = run_docker_command(
        &transport.docker_path,
        &mkdir_args,
        remote_server::setup::CHECK_TIMEOUT,
    )
    .await?;
    if !mkdir_output.status.success() {
        let code = mkdir_output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&mkdir_output.stderr).to_string();
        return Err(Error::ScriptFailed {
            exit_code: code,
            stderr,
        });
    }

    log::info!("Copying tarball into container");
    docker_cp(
        &transport.docker_path,
        &transport.container_id,
        &client_tarball_path,
        &remote_tarball_path,
        timeout,
    )
    .await?;

    let script = remote_server::setup::install_script(Some(&remote_tarball_path));
    let args = transport.script_args();
    let output = run_docker_script(&transport.docker_path, &args, &script, timeout).await?;
    let _ = run_docker_command(
        &transport.docker_path,
        &transport.command_args(&format!("rm -f {remote_tarball_path}")),
        remote_server::setup::CHECK_TIMEOUT,
    )
    .await;
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

async fn container_home(transport: &DevContainerTransport) -> Result<String, Error> {
    let args = transport.command_args("printenv HOME");
    let output = run_docker_command(
        &transport.docker_path,
        &args,
        remote_server::setup::CHECK_TIMEOUT,
    )
    .await?;
    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(Error::ScriptFailed {
            exit_code: code,
            stderr,
        });
    }
    let home = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if home.is_empty() {
        return Err(Error::Other(anyhow::anyhow!(
            "container HOME is empty; cannot copy remote-server tarball"
        )));
    }
    Ok(home)
}

fn expand_home_path(home: &str, path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else if path == "~" {
        home.to_string()
    } else {
        path.to_string()
    }
}

async fn docker_cp(
    docker_path: &Path,
    container_id: &str,
    host_path: &Path,
    container_path: &str,
    timeout: std::time::Duration,
) -> Result<(), Error> {
    let args = DevContainerTransport::cp_args(container_id, host_path, container_path);
    let output = run_docker_command(docker_path, &args, timeout).await?;
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

#[cfg(test)]
mod tests {
    use super::expand_home_path;

    #[test]
    fn expand_home_path_rewrites_tilde_prefix() {
        assert_eq!(
            expand_home_path("/home/vscode", "~/.warp/remote-server"),
            "/home/vscode/.warp/remote-server"
        );
    }

    #[test]
    fn expand_home_path_leaves_absolute_paths() {
        assert_eq!(
            expand_home_path("/home/vscode", "/tmp/oz.tar.gz"),
            "/tmp/oz.tar.gz"
        );
    }
}
