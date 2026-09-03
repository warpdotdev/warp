//! Docker-exec implementation of [`RemoteTransport`].
//!
//! [`DevContainerTransport`] uses non-TTY `docker exec` against a running
//! container to check/install the remote server binary and to launch the
//! `remote-server-proxy` process whose stdin/stdout become the protocol channel.
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Output;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use remote_server::auth::RemoteServerAuthContext;
use remote_server::client::RemoteServerClient;
use remote_server::manager::RemoteServerExitStatus;
use remote_server::setup::{PreinstallCheckResult, RemotePlatform, parse_uname_output};
use remote_server::transport::{Connection, ControlPath, Error, InstallOutcome, RemoteTransport};
use warpui::r#async::executor;
use warpui_core::r#async::FutureExt as _;

#[path = "dev_container_transport/installation.rs"]
mod installation;

/// Docker-exec transport: connects via `docker exec -i` into a running container.
#[derive(Clone)]
pub struct DevContainerTransport {
    docker_path: PathBuf,
    container_id: String,
    remote_user: Option<String>,
    remote_workspace: String,
    auth_context: Arc<RemoteServerAuthContext>,
}

impl fmt::Debug for DevContainerTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DevContainerTransport")
            .field("docker_path", &self.docker_path)
            .field("container_id", &self.container_id)
            .field("remote_user", &self.remote_user)
            .field("remote_workspace", &self.remote_workspace)
            .finish_non_exhaustive()
    }
}

impl DevContainerTransport {
    pub fn new(
        docker_path: PathBuf,
        container_id: String,
        remote_user: Option<String>,
        remote_workspace: String,
        auth_context: Arc<RemoteServerAuthContext>,
    ) -> Self {
        Self {
            docker_path,
            container_id,
            remote_user,
            remote_workspace,
            auth_context,
        }
    }

    pub fn container_id(&self) -> &str {
        &self.container_id
    }

    fn remote_proxy_command(&self) -> String {
        let binary = remote_server::setup::remote_server_binary();
        let identity_key = self.auth_context.remote_server_identity_key();
        let quoted_identity_key = shell_words::quote(&identity_key);
        format!("{binary} remote-server-proxy --identity-key {quoted_identity_key}")
    }

    fn exec_prefix(&self, interactive: bool) -> Vec<OsString> {
        docker_exec_prefix(
            &self.container_id,
            self.remote_user.as_deref(),
            &self.remote_workspace,
            interactive,
        )
    }

    pub(crate) fn command_args(&self, remote_command: &str) -> Vec<OsString> {
        let mut args = self.exec_prefix(false);
        args.push(OsString::from("sh"));
        args.push(OsString::from("-c"));
        args.push(OsString::from(remote_command));
        args
    }

    pub(crate) fn script_args(&self) -> Vec<OsString> {
        let mut args = self.exec_prefix(true);
        args.push(OsString::from("bash"));
        args.push(OsString::from("-s"));
        args
    }

    pub(crate) fn proxy_args(&self) -> Vec<OsString> {
        let mut args = self.exec_prefix(true);
        args.push(OsString::from("sh"));
        args.push(OsString::from("-c"));
        args.push(OsString::from(self.remote_proxy_command()));
        args
    }

    pub(crate) fn cp_args(
        container_id: &str,
        host_path: &Path,
        container_path: &str,
    ) -> Vec<OsString> {
        vec![
            OsString::from("cp"),
            host_path.as_os_str().to_owned(),
            OsString::from(format!("{container_id}:{container_path}")),
        ]
    }
}

pub(crate) fn docker_exec_prefix(
    container_id: &str,
    remote_user: Option<&str>,
    remote_workspace: &str,
    interactive: bool,
) -> Vec<OsString> {
    let mut args = vec![OsString::from("exec")];
    if interactive {
        args.push(OsString::from("-i"));
    }
    if let Some(remote_user) = remote_user {
        args.push(OsString::from("-u"));
        args.push(OsString::from(remote_user));
    }
    args.extend([
        OsString::from("-w"),
        OsString::from(remote_workspace),
        OsString::from(container_id),
    ]);
    args
}

#[derive(Debug, thiserror::Error)]
pub(super) enum DockerCommandError {
    #[error("Timed out after {timeout:?}")]
    TimedOut { timeout: Duration },
    #[error("Failed to spawn docker: {0}")]
    SpawnFailed(std::io::Error),
    #[error("Failed to write to docker stdin: {0}")]
    StdinWriteFailed(std::io::Error),
    #[error("Docker I/O error: {0}")]
    IoError(std::io::Error),
}

impl From<DockerCommandError> for Error {
    fn from(err: DockerCommandError) -> Self {
        match err {
            DockerCommandError::TimedOut { .. } => Self::TimedOut,
            other => Self::Other(other.into()),
        }
    }
}

pub(super) async fn run_docker_command(
    docker_path: &Path,
    args: &[OsString],
    timeout: Duration,
) -> Result<Output, DockerCommandError> {
    async {
        command::r#async::Command::new(docker_path)
            .args(args)
            .kill_on_drop(true)
            .output()
            .await
    }
    .with_timeout(timeout)
    .await
    .map_err(|_| DockerCommandError::TimedOut { timeout })?
    .map_err(DockerCommandError::IoError)
}

pub(super) async fn run_docker_script(
    docker_path: &Path,
    args: &[OsString],
    script: &str,
    timeout: Duration,
) -> Result<Output, DockerCommandError> {
    use std::process::Stdio;

    let mut child = command::r#async::Command::new(docker_path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(DockerCommandError::SpawnFailed)?;

    if let Some(mut stdin) = child.stdin.take() {
        use futures_lite::io::AsyncWriteExt;
        stdin
            .write_all(script.as_bytes())
            .await
            .map_err(DockerCommandError::StdinWriteFailed)?;
        drop(stdin);
    }

    child
        .output()
        .with_timeout(timeout)
        .await
        .map_err(|_| DockerCommandError::TimedOut { timeout })?
        .map_err(DockerCommandError::IoError)
}

pub(super) async fn detect_remote_platform(
    transport: &DevContainerTransport,
) -> Result<RemotePlatform, Error> {
    let args = transport.command_args("uname -sm");
    let output = run_docker_command(
        &transport.docker_path,
        &args,
        remote_server::setup::CHECK_TIMEOUT,
    )
    .await?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_uname_output(&stdout)
    } else {
        let code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(Error::Other(anyhow::anyhow!(
            "uname -sm exited with code {code}: {stderr}"
        )))
    }
}

impl RemoteTransport for DevContainerTransport {
    fn detect_platform(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<RemotePlatform, Error>> + Send>> {
        let transport = self.clone();
        Box::pin(async move { detect_remote_platform(&transport).await })
    }

    fn run_preinstall_check(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<PreinstallCheckResult, Error>> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            let args = transport.script_args();
            match run_docker_script(
                &transport.docker_path,
                &args,
                remote_server::setup::PREINSTALL_CHECK_SCRIPT,
                remote_server::setup::CHECK_TIMEOUT,
            )
            .await
            {
                Ok(output) if output.status.success() => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    Ok(PreinstallCheckResult::parse(&stdout))
                }
                Ok(output) => {
                    let exit_code = output.status.code().unwrap_or(-1);
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    Err(Error::ScriptFailed { exit_code, stderr })
                }
                Err(e) => Err(e.into()),
            }
        })
    }

    fn check_binary(&self) -> Pin<Box<dyn Future<Output = Result<bool, Error>> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            let cmd = remote_server::setup::binary_check_command();
            log::info!("Running binary check");
            let args = transport.command_args(&cmd);
            let output = run_docker_command(
                &transport.docker_path,
                &args,
                remote_server::setup::CHECK_TIMEOUT,
            )
            .await?;
            let code = output.status.code();
            match code {
                Some(0) => Ok(true),
                Some(126) | Some(127) => Ok(false),
                Some(code) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Err(Error::Other(anyhow::anyhow!(
                        "binary check exited with code {code}: {stderr}"
                    )))
                }
                None => Err(Error::Other(anyhow::anyhow!(
                    "binary check terminated by signal"
                ))),
            }
        })
    }

    fn check_has_old_binary(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            let cmd = format!("test -d {}", remote_server::setup::remote_server_dir());
            let args = transport.command_args(&cmd);
            let output = run_docker_command(
                &transport.docker_path,
                &args,
                remote_server::setup::CHECK_TIMEOUT,
            )
            .await?;
            match output.status.code() {
                Some(0) => Ok(true),
                Some(1) => Ok(false),
                Some(code) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Err(anyhow::anyhow!(
                        "remote-server dir check exited with code {code}: {stderr}"
                    ))
                }
                None => Err(anyhow::anyhow!(
                    "remote-server dir check terminated by signal"
                )),
            }
        })
    }

    fn install_binary(&self) -> Pin<Box<dyn Future<Output = InstallOutcome> + Send>> {
        let transport = self.clone();
        Box::pin(async move { installation::install_binary(&transport).await })
    }

    fn connect(
        &self,
        executor: Arc<executor::Background>,
    ) -> Pin<Box<dyn Future<Output = Result<Connection>> + Send>> {
        let docker_path = self.docker_path.clone();
        let args = self.proxy_args();
        Box::pin(async move {
            let mut child = command::r#async::Command::new(&docker_path)
                .args(&args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()?;

            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| anyhow::anyhow!("Failed to capture child stdin"))?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| anyhow::anyhow!("Failed to capture child stdout"))?;
            let stderr = child
                .stderr
                .take()
                .ok_or_else(|| anyhow::anyhow!("Failed to capture child stderr"))?;

            let (client, event_rx, failure_rx, host_response_rx, stderr_tail) =
                RemoteServerClient::from_child_streams(stdin, stdout, stderr, &executor);
            Ok(Connection {
                client,
                event_rx,
                failure_rx,
                host_response_rx,
                child,
                control_path: ControlPath::None,
                stderr_tail,
            })
        })
    }

    fn remove_remote_server_binary(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> {
        let transport = self.clone();
        Box::pin(async move {
            let cmd = remote_server::setup::remote_server_removal_command();
            log::info!("Removing stale remote server binary");
            let args = transport.command_args(&cmd);
            let output = run_docker_command(
                &transport.docker_path,
                &args,
                remote_server::setup::CHECK_TIMEOUT,
            )
            .await?;
            if output.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(anyhow::anyhow!("Failed to remove binary: {stderr}"))
            }
        })
    }

    fn is_reconnectable(&self, exit_status: Option<&RemoteServerExitStatus>) -> bool {
        match exit_status {
            Some(s) => s.code != Some(125) && !s.signal_killed,
            None => true,
        }
    }
}

#[cfg(test)]
#[path = "dev_container_transport_tests.rs"]
mod tests;
