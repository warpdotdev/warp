//! Private filesystem registry for discovering running local Warp instances.
//!
//! This module answers “which compatible instances are available, and where
//! can a client begin authentication?” It does not listen for control requests
//! and does not grant control authority. `app/src/local_control/mod.rs` owns the
//! running app-side listeners and uses these types to publish their routing
//! metadata.
//!
//! An enabled instance publishes an owner-only JSON record containing
//! instance/build metadata, implemented actions, its exact loopback HTTP
//! endpoint, and the filename of its instance-bound credential-broker socket.
//! The client reads that record, connects to the Unix socket to request a
//! short-lived credential for one exact action, and then presents the credential
//! to the HTTP endpoint. Discovery records never contain bearer tokens or
//! reusable credentials.
//!
//! Before following a record, clients require the endpoint host to be exactly
//! `127.0.0.1` and the broker filename to be derived from the instance ID. A
//! discovery scan also rejects incompatible records, prunes dead PIDs, and
//! performs an authenticated `app.ping` probe. When Scripting is disabled,
//! records contain neither an endpoint nor a broker reference.
//!
//! The owner-only directory, records, and broker sockets protect against other
//! OS users. The broker's kernel-reported peer-UID check is the authoritative
//! same-user check before credential issuance. Neither mechanism distinguishes
//! trusted Warp code from arbitrary software already running as that user.
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
#[cfg(windows)]
use command::blocking::Command;
use serde::{Deserialize, Serialize};

use crate::protocol::{ActionMetadata, ControlError, ErrorCode, PROTOCOL_VERSION};

const DISCOVERY_DIR_ENV: &str = "WARP_LOCAL_CONTROL_DISCOVERY_DIR";
const BROKER_SOCKET_SUFFIX: &str = ".broker.sock";

/// Stable identifier for one running Warp instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InstanceId(pub String);

impl InstanceId {
    pub fn new() -> Self {
        Self(format!("inst_{}", uuid::Uuid::new_v4().simple()))
    }
}

impl Default for InstanceId {
    fn default() -> Self {
        Self::new()
    }
}

/// Exact loopback HTTP route used after a client obtains a broker-issued credential.
///
/// Publishing this endpoint lets clients route requests; it does not authorize
/// them to invoke actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlEndpoint {
    pub host: String,
    pub port: u16,
}

impl ControlEndpoint {
    pub fn localhost(port: u16) -> Self {
        Self {
            host: "127.0.0.1".to_owned(),
            port,
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}:{}/v1/control", self.host, self.port)
    }
}

/// Discovery reference to the owner-authenticated socket that issues credentials.
///
/// Enabled records publish the instance-derived filename, not an arbitrary
/// socket path or a credential. Clients validate the filename and resolve it
/// inside the owner-only discovery directory before connecting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialBrokerReference {
    pub socket_path: PathBuf,
}

/// Filesystem-published routing metadata for a running Warp app process.
///
/// An enabled record connects the three stages of the protocol: filesystem
/// discovery, Unix-socket credential issuance, and authenticated loopback HTTP
/// dispatch. The optional endpoint and broker reference are present together or
/// absent together, so a disabled record cannot accidentally publish a usable
/// partial control route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceRecord {
    pub protocol_version: u32,
    pub instance_id: InstanceId,
    pub pid: u32,
    pub channel: String,
    pub app_id: String,
    pub app_version: Option<String>,
    pub started_at: DateTime<Utc>,
    pub executable_path: Option<PathBuf>,
    pub endpoint: Option<ControlEndpoint>,
    pub credential_broker: Option<CredentialBrokerReference>,
    pub actions: Vec<ActionMetadata>,
}

impl InstanceRecord {
    pub fn for_current_process(
        endpoint: Option<ControlEndpoint>,
        channel: impl Into<String>,
        app_id: impl Into<String>,
        app_version: Option<String>,
        actions: Vec<ActionMetadata>,
    ) -> Self {
        let instance_id = InstanceId::new();
        let credential_broker = endpoint.as_ref().map(|_| CredentialBrokerReference {
            socket_path: broker_socket_filename(&instance_id),
        });
        Self {
            protocol_version: PROTOCOL_VERSION,
            instance_id,
            pid: std::process::id(),
            channel: channel.into(),
            app_id: app_id.into(),
            app_version,
            started_at: Utc::now(),
            executable_path: std::env::current_exe().ok(),
            credential_broker,
            endpoint,
            actions,
        }
    }

    /// Rejects records that could redirect a client away from the selected instance.
    ///
    /// This validates routing metadata rather than granting authority: an
    /// enabled record must name exactly loopback and the broker filename derived
    /// from its instance ID. The broker and app bridge still authenticate and
    /// authorize the eventual request.
    pub fn validate_local_control_authority(&self) -> Result<(), ControlError> {
        match (&self.endpoint, &self.credential_broker) {
            (None, None) => Ok(()),
            (Some(endpoint), Some(credential_broker))
                if endpoint.host == "127.0.0.1"
                    && credential_broker.socket_path
                        == broker_socket_filename(&self.instance_id) =>
            {
                Ok(())
            }
            _ => Err(ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "local-control discovery record contains unsafe or inconsistent endpoint authority",
            )),
        }
    }

    /// Resolves the validated broker filename inside the private discovery directory.
    pub fn broker_socket_path(&self) -> Result<PathBuf, ControlError> {
        self.validate_local_control_authority()?;
        let credential_broker = self.credential_broker.as_ref().ok_or_else(|| {
            ControlError::new(
                ErrorCode::LocalControlDisabled,
                "local-control credential broker is disabled for this instance",
            )
        })?;
        Ok(discovery_dir().join(&credential_broker.socket_path))
    }
}

/// RAII registration for one app-owned discovery record and broker socket.
///
/// The registration publishes routing metadata for the lifetime of the running
/// server. Dropping it removes the record and socket on graceful shutdown;
/// discovery scans prune dead-PID records left behind by crashes.
pub struct RegisteredInstance {
    record: InstanceRecord,
    path: PathBuf,
    broker_socket_path: Option<PathBuf>,
}

impl RegisteredInstance {
    /// Publishes a record in the protected per-user registry.
    pub fn register(record: InstanceRecord) -> Result<Self, ControlError> {
        let dir = discovery_dir();
        fs::create_dir_all(&dir).map_err(|err| {
            ControlError::with_details(
                ErrorCode::Internal,
                "failed to create local-control discovery directory",
                err.to_string(),
            )
        })?;
        set_private_dir_permissions(&dir)?;
        let path = record_path(&dir, &record.instance_id);
        let broker_socket_path = record
            .credential_broker
            .as_ref()
            .map(|credential_broker| dir.join(&credential_broker.socket_path));
        write_record(&path, &record)?;
        Ok(Self {
            record,
            path,
            broker_socket_path,
        })
    }

    pub fn record(&self) -> &InstanceRecord {
        &self.record
    }

    pub fn update(&mut self, record: InstanceRecord) -> Result<(), ControlError> {
        let path = record_path(
            self.path.parent().unwrap_or_else(|| Path::new(".")),
            &record.instance_id,
        );
        write_record(&path, &record)?;
        if path != self.path {
            let _ = fs::remove_file(&self.path);
            self.path = path;
        }
        self.record = record;
        Ok(())
    }
}

fn write_record(path: &Path, record: &InstanceRecord) -> Result<(), ControlError> {
    let bytes = serde_json::to_vec_pretty(record).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to serialize local-control discovery record",
            err.to_string(),
        )
    })?;
    fs::write(path, bytes).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to write local-control discovery record",
            err.to_string(),
        )
    })?;
    set_private_permissions(path)?;
    Ok(())
}

impl Drop for RegisteredInstance {
    // Drop-time cleanup is the best-effort fast path for graceful shutdown.
    // `list_instances_from_dir` is the robust cleanup path: it treats records
    // whose PID is no longer alive as stale, removes them, and ignores malformed
    // or unreadable records so a crash can leave at most a temporary zombie
    // reference that is pruned on the next discovery scan.
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        if let Some(path) = &self.broker_socket_path {
            let _ = fs::remove_file(path);
        }
    }
}

/// Returns the private registry shared by app publishers and local clients.
pub fn discovery_dir() -> PathBuf {
    if let Some(path) = std::env::var_os(DISCOVERY_DIR_ENV) {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(path).join("warp").join("local-control");
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".warp").join("local-control")
}

/// Returns compatible live instances that pass an authenticated app ping.
///
/// The ping follows the normal broker-to-HTTP flow and verifies the responding
/// app's instance ID, so a live PID and parseable record alone are insufficient.
pub fn list_instances() -> Vec<InstanceRecord> {
    list_instances_from_dir(&discovery_dir())
        .into_iter()
        .filter(|record| crate::client::probe_instance(record).is_ok())
        .collect()
}

/// Parses structurally valid candidate records and prunes records with dead PIDs.
///
/// This lower-level scan does not contact the advertised endpoint; callers that
/// need invokable instances should use [`list_instances`] so candidates also
/// pass the authenticated probe.
pub fn list_instances_from_dir(dir: &Path) -> Vec<InstanceRecord> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let contents = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let record = match serde_json::from_str::<InstanceRecord>(&contents) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if record.protocol_version != PROTOCOL_VERSION {
            continue;
        }
        if record.validate_local_control_authority().is_err() {
            continue;
        }
        if !is_pid_alive(record.pid) {
            let _ = fs::remove_file(&path);
            continue;
        }
        records.push(record);
    }
    records.sort_by_key(|record| record.started_at);
    records
}

#[cfg(unix)]
fn is_pid_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(windows)]
fn is_pid_alive(pid: u32) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).contains("No tasks"))
        .unwrap_or(true)
}
#[cfg(all(not(unix), not(windows)))]
fn is_pid_alive(_: u32) -> bool {
    false
}

fn record_path(dir: &Path, instance_id: &InstanceId) -> PathBuf {
    dir.join(format!("{}.json", instance_id.0))
}
fn broker_socket_filename(instance_id: &InstanceId) -> PathBuf {
    PathBuf::from(format!("{}{BROKER_SOCKET_SUFFIX}", instance_id.0))
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path) -> Result<(), ControlError> {
    let mut permissions = fs::metadata(path)
        .map_err(|err| permissions_error("read local-control discovery directory", err))?
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)
        .map_err(|err| permissions_error("protect local-control discovery directory", err))
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path) -> Result<(), ControlError> {
    Err(ControlError::new(
        ErrorCode::LocalControlDisabled,
        "local-control discovery publication is disabled until this platform enforces record ACLs",
    ))
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<(), ControlError> {
    let mut permissions = fs::metadata(path)
        .map_err(|err| permissions_error("read local-control discovery record", err))?
        .permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)
        .map_err(|err| permissions_error("protect local-control discovery record", err))
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<(), ControlError> {
    Err(ControlError::new(
        ErrorCode::LocalControlDisabled,
        "local-control discovery publication is disabled until this platform enforces record ACLs",
    ))
}

#[cfg(unix)]
fn permissions_error(operation: &str, error: std::io::Error) -> ControlError {
    ControlError::with_details(
        ErrorCode::Internal,
        format!("failed to {operation}"),
        error.to_string(),
    )
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
