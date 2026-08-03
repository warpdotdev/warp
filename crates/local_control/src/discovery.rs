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
use std::collections::HashSet;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
#[cfg(windows)]
use command::blocking::Command;
use serde::{Deserialize, Serialize};

use crate::protocol::{ActionMetadata, ControlError, ErrorCode, PROTOCOL_VERSION};

const DISCOVERY_DIR_ENV: &str = "WARP_LOCAL_CONTROL_DISCOVERY_DIR";
const BROKER_SOCKET_SUFFIX: &str = ".broker.sock";
const TEMP_RECORD_SUFFIX: &str = ".json.tmp";
const ORPHAN_SOCKET_GRACE_PERIOD: Duration = Duration::from_secs(60);

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
    let temp_path = path.with_extension("json.tmp");
    fs::write(&temp_path, bytes).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to write temporary local-control discovery record",
            err.to_string(),
        )
    })?;
    if let Err(error) = set_private_permissions(&temp_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    fs::rename(&temp_path, path).map_err(|err| {
        let _ = fs::remove_file(&temp_path);
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to publish local-control discovery record",
            err.to_string(),
        )
    })?;
    Ok(())
}

impl Drop for RegisteredInstance {
    // Drop-time cleanup is the best-effort fast path for graceful shutdown.
    // `list_instances_from_dir` is the robust cleanup path: it removes stale
    // records, matching broker sockets, and abandoned registry artifacts.
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

/// Returns compatible live instances from `channel` that pass an authenticated app ping.
///
/// The ping follows the normal broker-to-HTTP flow and verifies the responding
/// app's instance ID, so a live PID and parseable record alone are insufficient.
pub fn list_instances(channel: &str) -> Vec<InstanceRecord> {
    let dir = discovery_dir();
    list_instances_from_dir(&dir, channel)
        .into_iter()
        .filter(|record| {
            if crate::client::probe_instance(record).is_ok() {
                return true;
            }
            if !is_pid_alive(record.pid) {
                remove_instance_artifacts(&dir, &record.instance_id);
            }
            false
        })
        .collect()
}

/// Parses structurally valid candidate records from `channel` and prunes records with dead PIDs.
///
/// This lower-level scan does not contact the advertised endpoint; callers that
/// need invokable instances should use [`list_instances`] so candidates also
/// pass the authenticated probe.
pub fn list_instances_from_dir(dir: &Path, channel: &str) -> Vec<InstanceRecord> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut records = Vec::new();
    let mut retained_broker_sockets = HashSet::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !is_record_path(&path) {
            continue;
        }
        let contents = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => {
                remove_malformed_record_artifacts(dir, &path);
                continue;
            }
        };
        let record = match serde_json::from_str::<InstanceRecord>(&contents) {
            Ok(r) => r,
            Err(_) => {
                remove_malformed_record_artifacts(dir, &path);
                continue;
            }
        };
        if !is_pid_alive(record.pid) {
            remove_instance_artifacts(dir, &record.instance_id);
            continue;
        }
        if record.credential_broker.is_some() {
            retained_broker_sockets.insert(broker_socket_filename(&record.instance_id));
        }
        if record.protocol_version != PROTOCOL_VERSION {
            continue;
        }
        if record.channel != channel {
            continue;
        }
        if record.validate_local_control_authority().is_err() {
            continue;
        }
        records.push(record);
    }
    sweep_orphan_broker_sockets(dir, &retained_broker_sockets, ORPHAN_SOCKET_GRACE_PERIOD);
    sweep_abandoned_temp_records(dir, ORPHAN_SOCKET_GRACE_PERIOD);
    records.sort_by_key(|record| record.started_at);
    records
}

fn is_record_path(path: &Path) -> bool {
    path.extension().and_then(|extension| extension.to_str()) == Some("json")
}

fn remove_malformed_record_artifacts(dir: &Path, path: &Path) {
    let _ = fs::remove_file(path);
    let Some(instance_id) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return;
    };
    let _ = fs::remove_file(dir.join(format!("{instance_id}{BROKER_SOCKET_SUFFIX}")));
}

fn remove_instance_artifacts(dir: &Path, instance_id: &InstanceId) {
    let _ = fs::remove_file(record_path(dir, instance_id));
    let _ = fs::remove_file(dir.join(broker_socket_filename(instance_id)));
}

fn sweep_orphan_broker_sockets(
    dir: &Path,
    retained_broker_sockets: &HashSet<PathBuf>,
    grace_period: Duration,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Some(filename) = path.file_name().and_then(|filename| filename.to_str()) else {
            continue;
        };
        if !filename.ends_with(BROKER_SOCKET_SUFFIX)
            || retained_broker_sockets.contains(Path::new(filename))
        {
            continue;
        }
        let Ok(age) = entry.metadata().and_then(|metadata| metadata.modified()) else {
            continue;
        };
        let Ok(age) = age.elapsed() else {
            continue;
        };
        if age >= grace_period {
            let _ = fs::remove_file(path);
        }
    }
}

fn sweep_abandoned_temp_records(dir: &Path, grace_period: Duration) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Some(filename) = path.file_name().and_then(|filename| filename.to_str()) else {
            continue;
        };
        if !filename.ends_with(TEMP_RECORD_SUFFIX) {
            continue;
        }
        let Ok(age) = entry.metadata().and_then(|metadata| metadata.modified()) else {
            continue;
        };
        let Ok(age) = age.elapsed() else {
            continue;
        };
        if age >= grace_period {
            let _ = fs::remove_file(path);
        }
    }
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
