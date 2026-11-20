//! Filesystem layout, single-instance locking, and the listening socket of the
//! terminal daemon.

use std::ffi::OsString;
use std::fs::{DirBuilder, File, OpenOptions, Permissions};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::super::api::DAEMON_PROTOCOL_VERSION;

/// Socket paths longer than this fall back to a per-user runtime or temporary
/// directory.
const MAX_SOCKET_PATH_LEN: usize = 100;

/// The files used by one daemon protocol version.
#[derive(Debug, Clone)]
pub(crate) struct DaemonPaths {
    pub dir: PathBuf,
    pub socket: PathBuf,
    pub lock: PathBuf,
    pub log: PathBuf,
}

impl DaemonPaths {
    pub fn in_dir(dir: PathBuf) -> Self {
        let version = DAEMON_PROTOCOL_VERSION;
        Self {
            socket: dir.join(format!("v{version}.sock")),
            lock: dir.join(format!("v{version}.lock")),
            log: dir.join(format!("v{version}.log")),
            dir,
        }
    }

    /// The paths used by the app and the daemon it spawns: `<state dir>/ts`,
    /// or a fallback directory if that would make the socket path too long.
    pub fn default_for_user() -> Self {
        Self::for_state_dir(
            &warp_core::paths::state_dir(),
            runtime_dir(),
            std::env::var_os("TMPDIR"),
            nix::unistd::geteuid().as_raw(),
        )
    }

    /// The paths for the daemon of `state_dir`: `<state dir>/ts`, or a
    /// fallback directory unique to `state_dir` if that would make the socket
    /// path too long, so that different channels and homes never share a
    /// daemon.
    pub(crate) fn for_state_dir(
        state_dir: &Path,
        runtime_dir: Option<OsString>,
        tmpdir: Option<OsString>,
        uid: u32,
    ) -> Self {
        if !state_dir.as_os_str().is_empty() {
            let paths = Self::in_dir(state_dir.join("ts"));
            if paths.socket.as_os_str().len() <= MAX_SOCKET_PATH_LEN {
                return paths;
            }
        }
        Self::in_dir(fallback_dir(runtime_dir, tmpdir, uid, state_dir))
    }

    /// Creates the directory with mode 0700 if needed and checks that it is a
    /// real directory owned by the current user. An existing directory with
    /// looser permissions is tightened to 0700.
    pub fn ensure_private_dir(&self) -> Result<()> {
        if let Some(parent) = self.dir.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        match DirBuilder::new().mode(0o700).create(&self.dir) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("Failed to create {}", self.dir.display()));
            }
        }
        let metadata = check_owned_dir(&self.dir)?;
        if metadata.mode() & 0o777 != 0o700 {
            std::fs::set_permissions(&self.dir, Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    /// Checks, without changing anything, that the directory is a real
    /// directory owned by the current user with mode 0700. Returns `false` if
    /// it does not exist.
    pub fn validate_private_dir(&self) -> Result<bool> {
        match std::fs::symlink_metadata(&self.dir) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("Failed to inspect {}", self.dir.display()));
            }
            Ok(_) => {}
        }
        let metadata = check_owned_dir(&self.dir)?;
        if metadata.mode() & 0o077 != 0 {
            bail!("{} is accessible by other users", self.dir.display());
        }
        Ok(true)
    }
}

/// Returns `$XDG_RUNTIME_DIR` on Linux, if it is an absolute path.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn runtime_dir() -> Option<OsString> {
    std::env::var_os("XDG_RUNTIME_DIR")
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn runtime_dir() -> Option<OsString> {
    None
}

/// The directory used when the state directory makes the socket path too
/// long: `<runtime dir>/warp-ts-<key>` (the runtime dir is already per-user
/// and private), else `<$TMPDIR or /tmp>/warp-ts-<uid>-<key>`, where `<key>`
/// is derived from the state directory.
pub(crate) fn fallback_dir(
    runtime_dir: Option<OsString>,
    tmpdir: Option<OsString>,
    uid: u32,
    state_dir: &Path,
) -> PathBuf {
    let key = state_dir_key(state_dir);
    if let Some(runtime_dir) = runtime_dir
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
    {
        return runtime_dir.join(format!("warp-ts-{key}"));
    }
    let tmp = tmpdir
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    tmp.join(format!("warp-ts-{uid}-{key}"))
}

/// A short key that is stable across processes and builds: the low 48 bits
/// of the 64-bit FNV-1a hash of the path bytes, in hex.
pub(crate) fn state_dir_key(state_dir: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let hash = state_dir
        .as_os_str()
        .as_bytes()
        .iter()
        .fold(OFFSET_BASIS, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
        });
    format!("{:012x}", hash & 0xffff_ffff_ffff)
}

/// Fails unless `dir` is a directory (not a symlink) owned by the current
/// user.
fn check_owned_dir(dir: &Path) -> Result<std::fs::Metadata> {
    let metadata = std::fs::symlink_metadata(dir)
        .with_context(|| format!("Failed to inspect {}", dir.display()))?;
    if !metadata.is_dir() || metadata.uid() != nix::unistd::geteuid().as_raw() {
        bail!(
            "{} is not a directory owned by the current user",
            dir.display()
        );
    }
    Ok(metadata)
}

/// Fails unless the process on the other end of `fd` runs as `expected_uid`.
pub(crate) fn check_peer_uid(fd: RawFd, expected_uid: u32) -> Result<()> {
    let uid = peer_uid(fd).context("Failed to read the peer credentials")?;
    if uid != expected_uid {
        bail!("peer runs as uid {uid}, expected {expected_uid}");
    }
    Ok(())
}

/// An exclusive `flock` held for the lifetime of the daemon.
pub(crate) struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    /// Takes the lock without blocking. Returns `None` if another process
    /// holds it.
    pub fn try_acquire(path: &Path) -> Result<Option<Self>> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("Failed to open {}", path.display()))?;
        // SAFETY: `file` is a valid open fd.
        let res = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if res == 0 {
            return Ok(Some(Self { _file: file }));
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
            Ok(None)
        } else {
            Err(err).context("flock failed")
        }
    }
}

/// Binds the daemon socket with mode 0600, replacing any stale socket file.
pub(crate) fn bind_socket(path: &Path) -> Result<UnixListener> {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).context("Failed to remove stale socket"),
    }
    // SAFETY: umask has no memory-safety requirements.
    let previous = unsafe { libc::umask(0o077) };
    let result = UnixListener::bind(path);
    // SAFETY: as above.
    unsafe { libc::umask(previous) };
    let listener = result.with_context(|| format!("Failed to bind {}", path.display()))?;
    std::fs::set_permissions(path, Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

/// Returns the effective uid of the process on the other end of `fd`.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) fn peer_uid(fd: RawFd) -> std::io::Result<u32> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` and `len` are valid for writes of the sizes passed.
    let res = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    if res == 0 {
        Ok(cred.uid)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Returns the effective uid of the process on the other end of `fd`.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub(crate) fn peer_uid(fd: RawFd) -> std::io::Result<u32> {
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    // SAFETY: `uid` and `gid` are valid for writes.
    let res = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
    if res == 0 {
        Ok(uid)
    } else {
        Err(std::io::Error::last_os_error())
    }
}
