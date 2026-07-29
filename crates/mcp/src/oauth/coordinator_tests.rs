//! Tests for the cross-process MCP OAuth coordinator.
//!
//! These run against a temporary `WARP_MCP_OAUTH_COORDINATION_DIR` so they never
//! touch a developer's real runtime/keychain state. They cover leader election
//! exclusivity, the follower wait loop, promotion after a leader release,
//! serialized credential read/merge/write (including concurrent installations
//! and malformed-map rejection), owner-only directory/file permissions, owner
//! metadata publication, and the forwarded-callback validation.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use instant::Instant;
use tempfile::TempDir;
use uuid::Uuid;

use super::{
    CoordinatorConfig, CredentialKey, CredentialNamespace, OAuthCoordinator, OwnerMetadata,
    ResolveOutcome, SecureCredentialBackend,
};
use crate::oauth::PersistedCredentials;

/// Sets `WARP_MCP_OAUTH_COORDINATION_DIR` to a fresh temp dir for the test and
/// holds the shared `ENV_LOCK` so parallel tests do not race on the global env
/// var.
struct CoordinationDirScope {
    _env_guard: tokio::sync::MutexGuard<'static, ()>,
    _dir: TempDir,
}

impl CoordinationDirScope {
    async fn new() -> Self {
        let env_guard = crate::oauth::env_lock().await;
        let dir = TempDir::with_prefix("mcp-oauth-coord").expect("temp dir");
        // SAFETY: the `ENV_LOCK` serializes all tests that touch this env var, so
        // no other test reads or writes it while this scope holds the guard.
        unsafe {
            std::env::set_var("WARP_MCP_OAUTH_COORDINATION_DIR", dir.path());
        }
        Self {
            _env_guard: env_guard,
            _dir: dir,
        }
    }
}

impl Drop for CoordinationDirScope {
    fn drop(&mut self) {
        // SAFETY: the `ENV_LOCK` guard is still held, see `new`.
        unsafe {
            std::env::remove_var("WARP_MCP_OAUTH_COORDINATION_DIR");
        }
    }
}

/// An in-memory `SecureCredentialBackend` for tests. `read_raw`/`write_raw`
/// operate on a shared JSON string so multiple coordinators (simulating multiple
/// processes) see one another's writes.
#[derive(Clone, Default)]
struct FakeBackend {
    map_json: Arc<Mutex<Option<String>>>,
    notify_count: Arc<AtomicU32>,
    /// When set, `write_raw` fails to simulate a backend write failure.
    fail_writes: Arc<Mutex<bool>>,
    /// When set, `write_raw` blocks on `write_release` before completing, so a
    /// test can hold the credential-store lock open and exercise
    /// contention/timeout against a second coordinator.
    pause_writes: Arc<AtomicBool>,
    write_release: Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl SecureCredentialBackend for FakeBackend {
    async fn read_raw(&self) -> anyhow::Result<Option<String>> {
        Ok(self.map_json.lock().expect("map lock").clone())
    }

    async fn write_raw(&self, json: &str) -> anyhow::Result<()> {
        if *self.fail_writes.lock().expect("fail lock") {
            anyhow::bail!("injected backend write failure");
        }
        if self.pause_writes.load(Ordering::SeqCst) {
            self.write_release.notified().await;
        }
        *self.map_json.lock().expect("map lock") = Some(json.to_string());
        Ok(())
    }

    async fn notify_changed(&self) {
        self.notify_count.fetch_add(1, Ordering::SeqCst);
    }
}

fn test_config() -> CoordinatorConfig {
    CoordinatorConfig {
        wait_deadline: Duration::from_secs(2),
        poll_interval: Duration::from_millis(10),
        cred_store_lock_timeout: Duration::from_secs(2),
    }
}

fn templatable_coordinator(uuid: Uuid, backend: Arc<FakeBackend>) -> OAuthCoordinator {
    OAuthCoordinator::new(
        "test-channel",
        CredentialNamespace::Templatable,
        uuid,
        CredentialKey::Uuid(uuid),
        backend,
        test_config(),
        None,
    )
    .expect("coordinator constructs")
}

/// Build a minimal `PersistedCredentials` with a unique access token + refresh
/// token + client secret for the given installation, so concurrent installations
/// have distinguishable values.
fn make_credentials(installation: Uuid) -> PersistedCredentials {
    use rmcp::transport::auth::OAuthTokenResponse;
    let json = serde_json::json!({
        "access_token": format!("access-{installation}"),
        "token_type": "bearer",
        "expires_in": 3600,
        "refresh_token": format!("refresh-{installation}"),
    });
    let token_response: OAuthTokenResponse = serde_json::from_value(json).expect("token response");
    PersistedCredentials {
        credentials: rmcp::transport::auth::StoredCredentials::new(
            format!("client-{installation}"),
            Some(token_response),
            Vec::new(),
            Some(1_700_000_500),
        ),
        client_secret: Some(format!("secret-{installation}")),
    }
}

#[tokio::test]
async fn exactly_one_leader_is_elected_for_one_installation() {
    let _scope = CoordinationDirScope::new().await;
    let backend = Arc::new(FakeBackend::default());
    let uuid = Uuid::new_v4();

    // One process becomes leader and holds the lease across the two subsequent
    // attempts (simulating three concurrent processes racing for one flow).
    let coord0 = templatable_coordinator(uuid, backend.clone());
    let ResolveOutcome::Leader(lease0) = coord0.resolve_or_wait().await.expect("resolve0") else {
        panic!("first process should lead");
    };
    let mut leaders = 1;
    for _ in 0..2 {
        let coord = templatable_coordinator(uuid, backend.clone());
        if let ResolveOutcome::Leader(_lease) = coord.resolve_or_wait().await.expect("resolve") {
            leaders += 1;
        }
    }
    assert_eq!(
        leaders, 1,
        "exactly one leader must be elected while the lease is held"
    );
    drop(lease0);
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_coordination_root_is_canonicalized_without_escape() {
    // macOS's default temporary directory is commonly reached through a
    // `/var` symlink. The coordinator must normalize that harmless indirection
    // rather than rejecting the path because its spelling differs from the
    // canonical form.
    let env_guard = crate::oauth::env_lock().await;
    let real_root = TempDir::with_prefix("mcp-oauth-real").expect("real temp dir");
    let alias_parent = TempDir::with_prefix("mcp-oauth-alias").expect("alias parent");
    let alias = alias_parent.path().join("runtime");
    std::os::unix::fs::symlink(real_root.path(), &alias).expect("symlink temp root");
    // SAFETY: the shared environment lock serializes all coordinator tests.
    unsafe {
        std::env::set_var("WARP_MCP_OAUTH_COORDINATION_DIR", &alias);
    }

    let backend = Arc::new(FakeBackend::default());
    let uuid = Uuid::new_v4();
    let coordinator = templatable_coordinator(uuid, backend);
    let canonical_root = fs::canonicalize(real_root.path()).expect("canonical root");
    let canonical_dir = fs::canonicalize(coordinator.dir()).expect("canonical dir");
    assert_eq!(
        coordinator.dir(),
        canonical_dir,
        "coordinator stores canonical paths"
    );
    assert!(
        coordinator.dir().starts_with(&canonical_root),
        "coordination artifacts remain inside the trusted root"
    );

    // SAFETY: the shared environment lock is still held.
    unsafe {
        std::env::remove_var("WARP_MCP_OAUTH_COORDINATION_DIR");
    }
    drop(env_guard);
}
#[tokio::test]
async fn follower_waits_then_loads_published_credentials() {
    let _scope = CoordinationDirScope::new().await;
    let backend = Arc::new(FakeBackend::default());
    let uuid = Uuid::new_v4();

    // Process A becomes the leader and holds the lease.
    let coord_a = templatable_coordinator(uuid, backend.clone());
    let ResolveOutcome::Leader(_lease_a) = coord_a.resolve_or_wait().await.expect("resolve A")
    else {
        panic!("A should be the leader");
    };

    // Process B resolves while A holds the lease: it must not become leader and
    // must enter the wait loop. Publish credentials from A (simulating A
    // completing OAuth) so B converges.
    let coord_b = templatable_coordinator(uuid, backend.clone());
    let credentials = make_credentials(uuid);
    let publish_backend = backend.clone();
    let publish_uuid = uuid;
    tokio::spawn(async move {
        // Give B time to enter the wait loop, then publish.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let coord = templatable_coordinator(publish_uuid, publish_backend);
        coord.merge_and_write(credentials).await.expect("publish");
    });

    let outcome_b = coord_b.resolve_or_wait().await.expect("resolve B");
    match outcome_b {
        ResolveOutcome::Credentials(loaded) => {
            assert_eq!(
                loaded.credentials.client_id,
                format!("client-{uuid}"),
                "B must load the credentials A published"
            );
        }
        other => panic!("B must become a follower and load shared creds, got {other:?}"),
    }
}

#[tokio::test]
async fn promotion_after_leader_release_picks_exactly_one_successor() {
    let _scope = CoordinationDirScope::new().await;
    let backend = Arc::new(FakeBackend::default());
    let uuid = Uuid::new_v4();

    // First leader acquires and then releases (dropping the lease).
    {
        let coord_a = templatable_coordinator(uuid, backend.clone());
        let ResolveOutcome::Leader(lease) = coord_a.resolve_or_wait().await.expect("resolve A")
        else {
            panic!("A should lead");
        };
        drop(lease);
    }

    // After release, two waiters race; exactly one must promote to leader.
    let coord_b = templatable_coordinator(uuid, backend.clone());
    let coord_c = templatable_coordinator(uuid, backend.clone());
    let (b_outcome, c_outcome) = tokio::join!(async { coord_b.resolve_or_wait().await }, async {
        coord_c.resolve_or_wait().await
    },);
    let b_outcome = b_outcome.expect("resolve B");
    let c_outcome = c_outcome.expect("resolve C");
    let leaders = matches!(b_outcome, ResolveOutcome::Leader(_)) as u32
        + matches!(c_outcome, ResolveOutcome::Leader(_)) as u32;
    assert_eq!(leaders, 1, "exactly one successor must promote to leader");
}

#[tokio::test]
async fn merge_and_write_preserves_other_installations_in_the_shared_map() {
    let _scope = CoordinationDirScope::new().await;
    let backend = Arc::new(FakeBackend::default());
    let uuid_a = Uuid::new_v4();
    let uuid_b = Uuid::new_v4();

    let coord_a = templatable_coordinator(uuid_a, backend.clone());
    let coord_b = templatable_coordinator(uuid_b, backend.clone());
    coord_a
        .merge_and_write(make_credentials(uuid_a))
        .await
        .expect("write A");
    coord_b
        .merge_and_write(make_credentials(uuid_b))
        .await
        .expect("write B");

    // The stored map must contain both installations (no whole-map overwrite).
    let raw = backend
        .map_json
        .lock()
        .expect("map lock")
        .clone()
        .expect("map written");
    let map: HashMap<Uuid, PersistedCredentials> =
        serde_json::from_str(&raw).expect("map deserializes");
    assert!(map.contains_key(&uuid_a), "installation A preserved");
    assert!(map.contains_key(&uuid_b), "installation B preserved");

    // read_latest returns each installation's own credentials.
    assert!(coord_a.read_latest().await.expect("read A").is_some());
    assert!(coord_b.read_latest().await.expect("read B").is_some());
    assert_eq!(
        backend.notify_count.load(Ordering::SeqCst),
        2,
        "notify_changed fires once per successful write"
    );
}

#[tokio::test]
async fn concurrent_merge_writes_for_different_installations_do_not_lose_entries() {
    let _scope = CoordinationDirScope::new().await;
    let backend = Arc::new(FakeBackend::default());
    let uuids: Vec<Uuid> = (0..8).map(|_| Uuid::new_v4()).collect();

    // Spawn one merge per installation concurrently.
    let mut handles = Vec::new();
    for uuid in &uuids {
        let backend = backend.clone();
        let uuid = *uuid;
        handles.push(tokio::spawn(async move {
            let coord = templatable_coordinator(uuid, backend);
            coord.merge_and_write(make_credentials(uuid)).await
        }));
    }
    for handle in handles {
        handle.await.expect("task joins").expect("write ok");
    }

    let raw = backend
        .map_json
        .lock()
        .expect("map lock")
        .clone()
        .expect("map written");
    let map: HashMap<Uuid, PersistedCredentials> =
        serde_json::from_str(&raw).expect("map deserializes");
    for uuid in &uuids {
        assert!(
            map.contains_key(uuid),
            "installation {uuid} must survive concurrent writes"
        );
    }
}

#[tokio::test]
async fn merge_and_write_rejects_malformed_existing_map_and_leaves_it_untouched() {
    let _scope = CoordinationDirScope::new().await;
    let backend = Arc::new(FakeBackend::default());
    // Seed with a malformed map.
    *backend.map_json.lock().expect("map lock") = Some("{not valid json".to_string());
    let uuid = Uuid::new_v4();
    let coord = templatable_coordinator(uuid, backend.clone());
    let result = coord.merge_and_write(make_credentials(uuid)).await;
    assert!(
        result.is_err(),
        "merge must reject a malformed existing map"
    );
    // The prior (malformed) value is left untouched — not replaced with empty.
    assert_eq!(
        backend.map_json.lock().expect("map lock").as_deref(),
        Some("{not valid json"),
        "prior value must be left untouched on malformed read"
    );
}

#[tokio::test]
async fn merge_and_write_failure_leaves_previous_value_intact() {
    let _scope = CoordinationDirScope::new().await;
    let backend = Arc::new(FakeBackend::default());
    let uuid = Uuid::new_v4();
    let coord = templatable_coordinator(uuid, backend.clone());
    // Write a valid value first.
    coord
        .merge_and_write(make_credentials(uuid))
        .await
        .expect("first write");
    let original = backend
        .map_json
        .lock()
        .expect("map lock")
        .clone()
        .expect("map present");

    // Now inject a backend write failure and attempt another write.
    *backend.fail_writes.lock().expect("fail lock") = true;
    let result = coord.merge_and_write(make_credentials(uuid)).await;
    assert!(result.is_err(), "write must fail when the backend fails");
    // The previous valid value is intact.
    assert_eq!(
        backend.map_json.lock().expect("map lock").as_deref(),
        Some(original.as_str()),
        "previous valid value must remain after a failed write"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn coordination_directory_and_files_are_owner_only_and_secret_free() {
    let _scope = CoordinationDirScope::new().await;
    let backend = Arc::new(FakeBackend::default());
    let uuid = Uuid::new_v4();
    let coord = templatable_coordinator(uuid, backend.clone());
    // Acquire the lease so the lock file is created, and publish owner metadata.
    let ResolveOutcome::Leader(_lease) = coord.resolve_or_wait().await.expect("resolve") else {
        panic!("should lead");
    };
    let metadata = OwnerMetadata::new(uuid, "fake-endpoint".to_string());
    coord.publish_owner_metadata(&metadata).expect("publish");

    use std::os::unix::fs::PermissionsExt as _;
    let dir = coord.dir();
    let dir_mode = fs::metadata(dir).expect("dir").permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700, "coordination directory must be 0700");

    // Every file inside the directory must be owner-only (no group/world bits)
    // and must not contain access tokens, refresh tokens, or client secrets.
    for entry in fs::read_dir(dir).expect("read dir").filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let mode = fs::metadata(&path).expect("file").permissions().mode() & 0o777;
        assert!(
            mode & 0o077 == 0,
            "coordination file {:?} must be owner-only, got mode {mode:o}",
            path
        );
        if let Ok(contents) = fs::read_to_string(&path) {
            assert!(
                !contents.contains("access-")
                    && !contents.contains("refresh-")
                    && !contents.contains("secret-"),
                "coordination file {:?} must not contain credentials",
                path
            );
        }
    }
}

#[tokio::test]
async fn owner_metadata_round_trips_and_is_removable() {
    let _scope = CoordinationDirScope::new().await;
    let backend = Arc::new(FakeBackend::default());
    let uuid = Uuid::new_v4();
    let coord = templatable_coordinator(uuid, backend);
    let ResolveOutcome::Leader(_lease) = coord.resolve_or_wait().await.expect("resolve") else {
        panic!("should lead");
    };
    let metadata = OwnerMetadata::new(uuid, "endpoint-abc".to_string());
    coord.publish_owner_metadata(&metadata).expect("publish");

    let read = OAuthCoordinator::read_owner_metadata(
        "test-channel",
        CredentialNamespace::Templatable,
        uuid,
    )
    .expect("metadata present");
    assert_eq!(read, metadata, "owner metadata round-trips");
    assert_eq!(read.installation_uuid, uuid);
    assert_eq!(read.endpoint_address, "endpoint-abc");

    // read_all_owner_metadata finds the single published record.
    let all =
        OAuthCoordinator::read_all_owner_metadata("test-channel", CredentialNamespace::Templatable);
    assert_eq!(all.len(), 1, "exactly one owner metadata record");
    assert_eq!(all[0], metadata);

    coord.remove_owner_metadata();
    assert!(
        OAuthCoordinator::read_owner_metadata(
            "test-channel",
            CredentialNamespace::Templatable,
            uuid,
        )
        .is_none(),
        "owner metadata removed"
    );
}

#[tokio::test]
async fn owner_metadata_rejects_mismatched_protocol_version() {
    let _scope = CoordinationDirScope::new().await;
    let backend = Arc::new(FakeBackend::default());
    let uuid = Uuid::new_v4();
    let coord = templatable_coordinator(uuid, backend);
    let ResolveOutcome::Leader(_lease) = coord.resolve_or_wait().await.expect("resolve") else {
        panic!("should lead");
    };
    // Hand-write a metadata file with a future protocol version.
    let dir = coord.dir().to_path_buf();
    let path = dir.join(format!("owner-{}.json", uuid.simple()));
    fs::write(
        &path,
        serde_json::json!({
            "protocol_version": 999,
            "installation_uuid": uuid,
            "owner_nonce": "n",
            "endpoint_address": "e",
        })
        .to_string(),
    )
    .expect("write");
    assert!(
        OAuthCoordinator::read_owner_metadata(
            "test-channel",
            CredentialNamespace::Templatable,
            uuid,
        )
        .is_none(),
        "mismatched protocol version must be rejected"
    );
}

#[test]
fn forwarded_callback_looks_valid_accepts_well_formed_callback() {
    let uuid = Uuid::new_v4();
    assert!(super::forwarded_callback_looks_valid(
        "warp://mcp/oauth2callback?code=c&state=s",
        uuid,
    ));
}

#[test]
fn forwarded_callback_rejects_malformed_callbacks() {
    let uuid = Uuid::new_v4();
    // Wrong path.
    assert!(!super::forwarded_callback_looks_valid(
        "warp://mcp/other?code=c&state=s",
        uuid,
    ));
    // Missing state.
    assert!(!super::forwarded_callback_looks_valid(
        "warp://mcp/oauth2callback?code=c",
        uuid,
    ));
    // Unparseable.
    assert!(!super::forwarded_callback_looks_valid("not a url", uuid));
}

#[tokio::test]
async fn a_leftover_empty_lock_file_is_reusable() {
    // A crashed leader leaves an empty lock file behind. The OS releases the
    // exclusive lock on crash, so a successor must be able to re-acquire the
    // lease on that leftover file (no stale PID/metadata can block it). This
    // models the "stale empty lock file is reusable" half of crash recovery
    // without a subprocess; the OS-releases-on-crash behavior is exercised for
    // real in the cross-process CI variant described in the spec.
    let _scope = CoordinationDirScope::new().await;
    let backend = Arc::new(FakeBackend::default());
    let uuid = Uuid::new_v4();

    // Acquire and drop a lease, leaving the empty lock file on disk.
    {
        let coord = templatable_coordinator(uuid, backend.clone());
        let ResolveOutcome::Leader(lease) = coord.resolve_or_wait().await.expect("resolve") else {
            panic!("should lead");
        };
        drop(lease);
    }
    let lock_path = Path::new(&*std::env::var_os("WARP_MCP_OAUTH_COORDINATION_DIR").unwrap())
        .join("test-channel")
        .join("templatable")
        .join(format!("auth-{}.lock", uuid.simple()));
    assert!(lock_path.exists(), "leftover empty lock file remains");

    // A fresh coordinator must acquire the lease on the leftover file.
    let coord = templatable_coordinator(uuid, backend);
    let outcome = coord.resolve_or_wait().await.expect("resolve");
    assert!(
        matches!(outcome, ResolveOutcome::Leader(_)),
        "successor must acquire the lease on a leftover empty lock file"
    );
}

#[tokio::test]
async fn remove_and_write_preserves_other_installations_in_the_shared_map() {
    // Logout/deletion must go through the serialized remove-and-merge path so a
    // concurrent leader's just-published entry for another installation is not
    // clobbered by a whole-map write from a stale in-memory cache (spec
    // invariant #9).
    let _scope = CoordinationDirScope::new().await;
    let backend = Arc::new(FakeBackend::default());
    let uuid_a = Uuid::new_v4();
    let uuid_b = Uuid::new_v4();

    let coord_a = templatable_coordinator(uuid_a, backend.clone());
    let coord_b = templatable_coordinator(uuid_b, backend.clone());
    coord_a
        .merge_and_write(make_credentials(uuid_a))
        .await
        .expect("write A");
    coord_b
        .merge_and_write(make_credentials(uuid_b))
        .await
        .expect("write B");

    // Remove A's entry; B's entry must survive the serialized remove-and-write.
    coord_a.remove_and_write().await.expect("remove A");

    let raw = backend
        .map_json
        .lock()
        .expect("map lock")
        .clone()
        .expect("map written");
    let map: HashMap<Uuid, PersistedCredentials> =
        serde_json::from_str(&raw).expect("map deserializes");
    assert!(!map.contains_key(&uuid_a), "installation A must be removed");
    assert!(
        map.contains_key(&uuid_b),
        "installation B must survive A's removal (no whole-map clobber)"
    );
    assert!(
        coord_a.read_latest().await.expect("read A").is_none(),
        "A's own credentials are gone after removal"
    );
    assert!(
        coord_b.read_latest().await.expect("read B").is_some(),
        "B's credentials remain readable after A's removal"
    );
}

#[tokio::test]
async fn cred_store_lock_timeout_is_honored_under_contention() {
    // The credential-store lock must use non-blocking `try_lock_*` so a
    // contended lock returns within `cred_store_lock_timeout` instead of
    // blocking the async runtime indefinitely (the blocking `lock_*` bug).
    let _scope = CoordinationDirScope::new().await;
    let backend = Arc::new(FakeBackend::default());
    let uuid = Uuid::new_v4();

    // Coordinator A holds the exclusive lock open inside `write_raw` (writes are
    // paused). Coordinator B uses a short lock timeout and must time out quickly
    // rather than hang.
    let coord_a = templatable_coordinator(uuid, backend.clone());
    let mut config_b = test_config();
    config_b.cred_store_lock_timeout = Duration::from_millis(40);
    let coord_b = OAuthCoordinator::new(
        "test-channel",
        CredentialNamespace::Templatable,
        uuid,
        CredentialKey::Uuid(uuid),
        backend.clone(),
        config_b,
        None,
    )
    .expect("coord B");

    backend.pause_writes.store(true, Ordering::SeqCst);
    let backend_a = backend.clone();
    let write_task =
        tokio::spawn(async move { coord_a.merge_and_write(make_credentials(uuid)).await });
    // Give A time to acquire the exclusive lock and enter the paused write.
    tokio::time::sleep(Duration::from_millis(60)).await;

    let start = Instant::now();
    let outcome_b = tokio::time::timeout(
        Duration::from_secs(2),
        coord_b.merge_and_write(make_credentials(uuid)),
    )
    .await;
    let elapsed = start.elapsed();

    // Release A so the spawned task can finish and the test can clean up.
    backend_a.write_release.notify_waiters();
    write_task.await.expect("A joins").expect("A write");

    match outcome_b {
        Ok(Err(err)) => {
            assert!(
                err.to_string().contains("timed out"),
                "B must report a credential-store lock timeout, got {err:#}"
            );
            assert!(
                elapsed < Duration::from_secs(1),
                "B must honor the short timeout (non-blocking try_lock), took {elapsed:?}"
            );
        }
        Ok(Ok(())) => panic!("B must not acquire the lock while A holds it exclusively"),
        Err(_) => {
            panic!("B hung — the credential-store lock is blocking the async runtime (regression)")
        }
    }
}
