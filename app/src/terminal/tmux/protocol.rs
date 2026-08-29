use std::collections::HashSet;
use std::ffi::OsString;
use std::io::{self, Read as _};
#[cfg(unix)]
use std::os::fd::{FromRawFd as _, IntoRawFd as _};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use command::blocking::Command;
use instant::Instant;
use parking_lot::Mutex;
use warp_core::SessionId;
use warp_core::paths::cache_dir;
use warp_terminal::bootstrap::{
    generate_session_id, raw_init_shell_script_for_shell, script_for_shell,
};
use warp_terminal::local_tty::shell::{
    DirectShellStarter, arguments_for_session_spawning_command, supported_shell_path_and_type,
};
use warp_terminal::shell::ShellType;
use warp_util::path::resolve_executable;

use super::parser::PaneId;
use crate::ASSETS;
use crate::terminal::available_shells::AvailableShell;
use crate::terminal::shell::ShellLaunchData;

const SEND_KEYS_CHUNK_BYTES: usize = 128;
const KILL_SERVER_TIMEOUT: Duration = Duration::from_secs(2);
const KILL_SERVER_WAIT_SLICE: Duration = Duration::from_millis(10);

/// Stop the dedicated server when Warp's control client is the last to detach.
pub const DEDICATED_TMUX_CONFIG: &str = "set -s exit-unattached on\n";

#[cfg(unix)]
const DETACHED_KILL_BODY: &str = r#"
tmux_bin=$1
sock=$2
conf=$3
errfile=${sock}.kill-err
"$tmux_bin" -S "$sock" kill-server >"$errfile" 2>&1 &
pid=$!
n=0
while [ "$n" -lt 20 ]; do
  if ! kill -0 "$pid" 2>/dev/null; then
    wait "$pid"
    status=$?
    err=$(cat "$errfile" 2>/dev/null)
    rm -f "$errfile"
    if [ "$status" -eq 0 ] || printf '%s' "$err" | grep -q "no server running"; then
      rm -f "$sock" "$conf"
    fi
    exit 0
  fi
  sleep 0.1
  n=$((n + 1))
done
kill "$pid" 2>/dev/null
wait "$pid" 2>/dev/null
rm -f "$errfile"
exit 1
"#;

#[cfg(unix)]
const APP_EXIT_REAPER_SCRIPT: &str = r#"
list=$1
tmux_bin=$2
cat >/dev/null
[ -f "$list" ] || exit 0
while IFS= read -r sock || [ -n "$sock" ]; do
  [ -n "$sock" ] || continue
  conf=${sock%.sock}.conf
  errfile=${sock}.kill-err
  "$tmux_bin" -S "$sock" kill-server >"$errfile" 2>&1 &
  pid=$!
  n=0
  while [ "$n" -lt 20 ]; do
    if ! kill -0 "$pid" 2>/dev/null; then
      wait "$pid"
      status=$?
      err=$(cat "$errfile" 2>/dev/null)
      rm -f "$errfile"
      if [ "$status" -eq 0 ] || printf '%s' "$err" | grep -q "no server running"; then
        rm -f "$sock" "$conf"
      fi
      break
    fi
    sleep 0.1
    n=$((n + 1))
  done
  if kill -0 "$pid" 2>/dev/null; then
    kill "$pid" 2>/dev/null
    wait "$pid" 2>/dev/null
    rm -f "$errfile"
  fi
done < "$list"
rm -f "$list"
"#;

/// Bootstrap data for the Warp-managed pane process (not the control client).
#[derive(Debug, Clone)]
pub struct PaneBootstrap {
    pub session_id: SessionId,
    pub shell_type: ShellType,
    pub shell_path: PathBuf,
    pub args: Vec<OsString>,
    pub init_script: Option<String>,
}

impl PaneBootstrap {
    pub fn command_argv(&self) -> Vec<OsString> {
        let mut argv = Vec::with_capacity(1 + self.args.len());
        argv.push(self.shell_path.clone().into());
        argv.extend(self.args.iter().cloned());
        argv
    }
}

/// Build a Warp-bootstrapped pane command for the user's preferred supported shell.
pub fn pane_bootstrap_for_available_shell(
    preferred_shell: AvailableShell,
) -> Option<PaneBootstrap> {
    let launch_data = preferred_shell.get_valid_shell_path_and_type()?;
    let (shell_path, shell_type) = match launch_data {
        ShellLaunchData::Executable {
            executable_path,
            shell_type,
        } => (executable_path, shell_type),
        ShellLaunchData::WSL { .. }
        | ShellLaunchData::MSYS2 { .. }
        | ShellLaunchData::DockerSandbox { .. } => return None,
    };
    Some(pane_bootstrap_for_shell(shell_path, shell_type))
}

pub fn pane_bootstrap_for_shell(shell_path: PathBuf, shell_type: ShellType) -> PaneBootstrap {
    let session_id = generate_session_id();
    let args = arguments_for_session_spawning_command(
        shell_path.to_string_lossy().as_ref(),
        shell_type,
        session_id,
    );
    let init_script = matches!(shell_type, ShellType::Zsh)
        .then(|| raw_init_shell_script_for_shell(shell_type, &ASSETS, session_id));
    PaneBootstrap {
        session_id,
        shell_type,
        shell_path,
        args,
        init_script,
    }
}

/// PATH-based pane spawn for in-place `/tmux`, valid locally and on an already-remote shell.
pub fn in_place_pane_spawn(shell_type: ShellType) -> PaneBootstrap {
    pane_bootstrap_for_shell(PathBuf::from(shell_type.name()), shell_type)
}

/// PATH-based pane argv for in-place `/tmux`, valid locally and on an already-remote shell.
pub fn in_place_pane_spawn_argv(shell_type: ShellType) -> (Vec<String>, SessionId) {
    let bootstrap = in_place_pane_spawn(shell_type);
    let argv = bootstrap
        .command_argv()
        .into_iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    (argv, bootstrap.session_id)
}

/// Dedicated tmux server socket under Warp's cache directory.
pub fn dedicated_socket_path(session_id: SessionId) -> PathBuf {
    cache_dir()
        .join("tmux-control-prototype")
        .join(format!("warp-{}.sock", session_id.as_u64()))
}

pub fn dedicated_config_path(session_id: SessionId) -> PathBuf {
    cache_dir()
        .join("tmux-control-prototype")
        .join(format!("warp-{}.conf", session_id.as_u64()))
}

pub fn resolve_tmux_binary() -> Option<PathBuf> {
    resolve_executable("tmux").map(|path| path.into_owned())
}

/// `tmux` argv that starts a dedicated control-mode server and one Warp-bootstrapped pane.
pub fn control_client_argv(
    tmux_path: &Path,
    socket: &Path,
    config: &Path,
    bootstrap: &PaneBootstrap,
    columns: usize,
    rows: usize,
) -> Vec<OsString> {
    let mut argv = vec![
        tmux_path.as_os_str().to_owned(),
        "-S".into(),
        socket.as_os_str().to_owned(),
        "-f".into(),
        config.as_os_str().to_owned(),
        "-CC".into(),
        "new-session".into(),
        "-s".into(),
        format!("warp-{}", bootstrap.session_id.as_u64()).into(),
        "-n".into(),
        "warp".into(),
        "-x".into(),
        columns.to_string().into(),
        "-y".into(),
        rows.to_string().into(),
        "--".into(),
    ];
    argv.extend(bootstrap.command_argv());
    argv
}

pub fn tmux_shell_starter(
    argv: Vec<OsString>,
    session_id: SessionId,
) -> Option<DirectShellStarter> {
    let mut argv = argv.into_iter();
    let tmux_path = PathBuf::from(argv.next()?);
    Some(DirectShellStarter::new(
        ShellType::Bash,
        tmux_path,
        argv.collect(),
        session_id,
    ))
}

pub fn refresh_client_command(columns: usize, rows: usize) -> String {
    format!("refresh-client -C {columns}x{rows}\n")
}

/// Side-by-side (`-h`) for Warp left/right splits; stacked (`-v`) for up/down.
/// `-P -F '#{pane_id}'` puts the new pane id in the command-reply payload.
pub fn split_window_command(target: &PaneId, side_by_side: bool) -> String {
    let flag = if side_by_side { "-h" } else { "-v" };
    format!(
        "split-window {flag} -t {} -P -F '#{{pane_id}}'\n",
        target.as_str()
    )
}

pub fn select_pane_command(target: &PaneId) -> String {
    format!("select-pane -t {}\n", target.as_str())
}

pub fn kill_pane_command(target: &PaneId) -> String {
    format!("kill-pane -t {}\n", target.as_str())
}

pub fn resize_pane_command(target: &PaneId, columns: usize, rows: usize) -> String {
    format!(
        "resize-pane -t {} -x {columns} -y {rows}\n",
        target.as_str()
    )
}

pub fn new_window_command() -> String {
    "new-window -P -F '#{window_id}'\n".to_owned()
}

pub fn select_window_command(window_id: &super::parser::WindowId) -> String {
    format!("select-window -t {}\n", window_id.as_str())
}

pub fn kill_window_command(window_id: &super::parser::WindowId) -> String {
    format!("kill-window -t {}\n", window_id.as_str())
}

/// Detach this control client without killing the tmux server or pane journals.
pub fn detach_client_command() -> &'static str {
    "detach-client\n"
}

/// Append-only pane journal. Replay from a stored byte offset on reattach; tmux
/// `%output` is live-only and `capture-pane` drops Warp lifecycle hooks.
pub fn pipe_pane_journal_command(target: &PaneId, journal_path: &str) -> String {
    format!(
        "pipe-pane -t {} -O 'cat >> {}'\n",
        target.as_str(),
        journal_path
    )
}

/// Seed a fresh model with the pane's current visible screen after attach.
pub fn capture_pane_command(target: &PaneId) -> String {
    format!("capture-pane -p -t {}\n", target.as_str())
}

/// Tear down the dedicated Warp tmux server. `kill-session` can leave that
/// server process alive on this socket after the control client detaches.
pub fn kill_server_command() -> &'static str {
    "kill-server\n"
}

pub fn kill_server_argv(tmux_path: &Path, socket: &Path) -> Vec<OsString> {
    vec![
        tmux_path.as_os_str().to_owned(),
        "-S".into(),
        socket.as_os_str().to_owned(),
        "kill-server".into(),
    ]
}

#[derive(Debug)]
enum KillDedicatedServerError {
    TmuxNotFound,
    Io(io::Error),
    NonZeroExit(ExitStatus),
    TimedOut,
}

impl std::fmt::Display for KillDedicatedServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TmuxNotFound => write!(f, "tmux binary not found"),
            Self::Io(err) => write!(f, "{err}"),
            Self::NonZeroExit(status) => write!(f, "tmux kill-server failed: {status}"),
            Self::TimedOut => write!(f, "tmux kill-server timed out"),
        }
    }
}

/// Out-of-band teardown if the control-client write never lands.
/// Unlinks files only after `tmux kill-server` succeeds or reports no server.
pub fn kill_dedicated_server(socket: &Path) {
    kill_dedicated_server_with(
        resolve_tmux_binary().as_deref(),
        socket,
        KILL_SERVER_TIMEOUT,
    );
}

fn kill_dedicated_server_with(tmux_path: Option<&Path>, socket: &Path, timeout: Duration) {
    match try_kill_dedicated_server(tmux_path, socket, timeout) {
        Ok(()) => remove_dedicated_server_files(socket),
        Err(err) => {
            log::error!(
                "leaving tmux socket {} in place after kill-server failure: {err}",
                socket.display()
            );
        }
    }
}

pub fn cleanup_unspawned_dedicated_files(socket: &Path) {
    remove_dedicated_server_files(socket);
}

fn remove_dedicated_server_files(socket: &Path) {
    for path in [socket, &socket.with_extension("conf")] {
        if let Err(err) = std::fs::remove_file(path)
            && err.kind() != io::ErrorKind::NotFound
        {
            log::warn!("failed to remove {}: {err}", path.display());
        }
    }
}

fn dedicated_server_dir() -> PathBuf {
    cache_dir().join("tmux-control-prototype")
}

fn registry_instance_token() -> u64 {
    static TOKEN: OnceLock<u64> = OnceLock::new();
    *TOKEN.get_or_init(rand::random)
}

fn registry_list_filename() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        format!(
            "active-{}-{:016x}.list",
            std::process::id(),
            registry_instance_token()
        )
    })
}

fn registry_list_path() -> PathBuf {
    dedicated_server_dir().join(registry_list_filename())
}

fn persist_registry_list(sockets: &HashSet<PathBuf>) -> io::Result<()> {
    persist_registry_list_at(&registry_list_path(), sockets)
}

fn persist_registry_list_at(path: &Path, sockets: &HashSet<PathBuf>) -> io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "registry list path has no parent",
        ));
    };
    std::fs::create_dir_all(parent)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let contents: String = sockets
        .iter()
        .filter_map(|socket| socket.to_str())
        .map(|socket| format!("{socket}\n"))
        .collect();
    std::fs::write(&tmp, contents)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

fn persist_current_registry() -> bool {
    match persist_registry_list(&lock_dedicated_sockets()) {
        Ok(()) => true,
        Err(err) => {
            log::error!(
                "failed to persist tmux registry list {}: {err}",
                registry_list_path().display()
            );
            false
        }
    }
}

fn dedicated_sockets() -> &'static Mutex<HashSet<PathBuf>> {
    static SOCKETS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    SOCKETS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn lock_dedicated_sockets() -> parking_lot::MutexGuard<'static, HashSet<PathBuf>> {
    dedicated_sockets().lock()
}

fn app_exit_reaper_started() -> &'static Mutex<bool> {
    static STARTED: OnceLock<Mutex<bool>> = OnceLock::new();
    STARTED.get_or_init(|| Mutex::new(false))
}

#[cfg(unix)]
fn parent_death_pipe() -> &'static Mutex<Option<UnixStream>> {
    static PIPE: OnceLock<Mutex<Option<UnixStream>>> = OnceLock::new();
    PIPE.get_or_init(|| Mutex::new(None))
}

/// Track a dedicated socket so last-tab app exit can still tear it down.
pub fn register_dedicated_server(socket: PathBuf) {
    lock_dedicated_sockets().insert(socket);
    if persist_current_registry() {
        ensure_app_exit_reaper();
    }
}

fn unregister_dedicated_server(socket: &Path) {
    lock_dedicated_sockets().remove(socket);
    let _ = persist_current_registry();
}

/// Last-tab close on Linux quits the app without dropping pane managers.
pub fn schedule_kill_registered_dedicated_servers() {
    #[cfg(unix)]
    {
        if persist_current_registry() && ensure_app_exit_reaper() {
            return;
        }
        let sockets: Vec<PathBuf> = lock_dedicated_sockets().iter().cloned().collect();
        for socket in sockets {
            schedule_kill_dedicated_server(socket);
        }
    }
    #[cfg(not(unix))]
    {
        let sockets: Vec<PathBuf> = lock_dedicated_sockets().drain().collect();
        let _ = persist_current_registry();
        for socket in sockets {
            schedule_kill_dedicated_server(socket);
        }
    }
}

fn ensure_app_exit_reaper() -> bool {
    #[cfg(unix)]
    {
        ensure_app_exit_reaper_with(resolve_tmux_binary().as_deref())
    }
    #[cfg(not(unix))]
    {
        false
    }
}

#[cfg(unix)]
fn ensure_app_exit_reaper_with(tmux_path: Option<&Path>) -> bool {
    let mut started = app_exit_reaper_started().lock();
    if *started {
        return true;
    }
    if !persist_current_registry() {
        return false;
    }
    let Some(tmux_path) = tmux_path else {
        return false;
    };
    if spawn_app_exit_reaper(&registry_list_path(), tmux_path) {
        *started = true;
        true
    } else {
        false
    }
}

/// Kill the dedicated server without blocking the caller.
pub fn schedule_kill_dedicated_server(socket: PathBuf) {
    #[cfg(unix)]
    spawn_detached_kill_helper(resolve_tmux_binary().as_deref(), &socket);
    #[cfg(not(unix))]
    {
        unregister_dedicated_server(&socket);
        spawn_kill_dedicated_server_thread(socket);
    }
}

#[cfg(not(unix))]
fn spawn_kill_dedicated_server_thread(socket: PathBuf) {
    if let Err(err) = std::thread::Builder::new()
        .name("tmux-control-prototype-kill-server".into())
        .spawn(move || kill_dedicated_server(&socket))
    {
        log::error!("failed to spawn tmux kill-server worker: {err}");
    }
}

#[cfg(unix)]
fn spawn_app_exit_reaper(list: &Path, tmux_path: &Path) -> bool {
    let Ok((parent_end, child_end)) = UnixStream::pair() else {
        log::error!("failed to create tmux parent-death pipe");
        return false;
    };
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(APP_EXIT_REAPER_SCRIPT)
        .arg("tmux-control-prototype-exit-reaper")
        .arg(list)
        .arg(tmux_path)
        // SAFETY: child_end is the unique owner of this socket.
        .stdin(unsafe { Stdio::from_raw_fd(child_end.into_raw_fd()) })
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Last-tab Warp exit otherwise kills this reaper with the UI process group.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    match command.spawn() {
        Ok(mut child) => {
            *parent_death_pipe().lock() = Some(parent_end);
            let _ = std::thread::Builder::new()
                .name("tmux-control-prototype-exit-reaper-wait".into())
                .spawn(move || {
                    let _ = child.wait();
                });
            true
        }
        Err(err) => {
            log::error!("failed to spawn tmux parent-exit reaper: {err}");
            false
        }
    }
}

#[cfg(unix)]
fn spawn_detached_kill_helper(tmux_path: Option<&Path>, socket: &Path) -> bool {
    let Some(tmux_path) = tmux_path else {
        log::error!(
            "leaving tmux socket {} in place: tmux binary not found",
            socket.display()
        );
        return false;
    };
    let config = socket.with_extension("conf");
    match spawn_setsid_sh(
        DETACHED_KILL_BODY,
        "tmux-control-prototype-kill-server",
        &[
            tmux_path.as_os_str(),
            socket.as_os_str(),
            config.as_os_str(),
        ],
    ) {
        Ok(mut child) => {
            let socket = socket.to_path_buf();
            let _ = std::thread::Builder::new()
                .name("tmux-control-prototype-kill-server-reap".into())
                .spawn(move || {
                    let confirmed = child.wait().is_ok_and(|status| status.success())
                        && !socket.exists()
                        && !socket.with_extension("conf").exists();
                    if confirmed {
                        unregister_dedicated_server(&socket);
                    }
                });
            true
        }
        Err(err) => {
            log::error!(
                "failed to spawn tmux kill-server helper for {}: {err}",
                socket.display()
            );
            false
        }
    }
}

#[cfg(unix)]
fn spawn_setsid_sh(script: &str, arg0: &str, args: &[&std::ffi::OsStr]) -> io::Result<Child> {
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(script).arg(arg0);
    for arg in args {
        command.arg(arg);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Last-tab Warp exit otherwise kills this helper with the UI process group.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn()
}

fn server_already_gone(stderr: &str) -> bool {
    stderr.contains("no server running")
}

#[cfg(test)]
fn registry_test_lock() -> parking_lot::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock()
}

#[cfg(test)]
fn registered_dedicated_server_count() -> usize {
    lock_dedicated_sockets().len()
}

#[cfg(test)]
fn app_exit_reaper_has_started() -> bool {
    *app_exit_reaper_started().lock()
}

fn try_kill_dedicated_server(
    tmux_path: Option<&Path>,
    socket: &Path,
    timeout: Duration,
) -> Result<(), KillDedicatedServerError> {
    let Some(tmux_path) = tmux_path else {
        return Err(KillDedicatedServerError::TmuxNotFound);
    };
    let argv = kill_server_argv(tmux_path, socket);
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::piped());
    let mut child = command.spawn().map_err(KillDedicatedServerError::Io)?;
    let status = wait_child_with_timeout(&mut child, timeout)?;
    if status.success() {
        return Ok(());
    }
    let mut stderr = String::new();
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr);
    }
    if server_already_gone(&stderr) {
        Ok(())
    } else {
        Err(KillDedicatedServerError::NonZeroExit(status))
    }
}

fn wait_child_with_timeout(
    child: &mut Child,
    timeout: Duration,
) -> Result<ExitStatus, KillDedicatedServerError> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(KillDedicatedServerError::TimedOut);
                }
                std::thread::sleep(KILL_SERVER_WAIT_SLICE);
            }
            Err(err) => return Err(KillDedicatedServerError::Io(err)),
        }
    }
}

/// Encode pane input as `send-keys -H` so arbitrary bytes never pass through tmux key-name parsing.
pub fn send_keys_commands(pane_id: &PaneId, bytes: &[u8]) -> Vec<String> {
    if bytes.is_empty() {
        return Vec::new();
    }
    bytes
        .chunks(SEND_KEYS_CHUNK_BYTES)
        .map(|chunk| {
            let mut command = format!("send-keys -t {} -H", pane_id.as_str());
            for byte in chunk {
                command.push_str(&format!(" {byte:02x}"));
            }
            command.push('\n');
            command
        })
        .collect()
}

pub const STAGE_COMPLETE_OSC_PREFIX: &[u8] = b"\x1b]9278;t;";

pub fn zsh_init_bytes(init_script: &str, shell_type: ShellType, session_id: SessionId) -> Vec<u8> {
    let mut bytes = init_script.as_bytes().to_vec();
    while matches!(bytes.last(), Some(b'\n' | b';')) {
        bytes.pop();
    }
    if !bytes.is_empty() {
        bytes.push(b';');
    }
    bytes.extend_from_slice(&stage_complete_script(shell_type, session_id));
    bytes
}

pub fn stage_complete_script(shell_type: ShellType, session_id: SessionId) -> Vec<u8> {
    let mut bytes = format!("printf '\\033]9278;t;{}\\007'", session_id.as_u64()).into_bytes();
    bytes.extend_from_slice(shell_type.execute_command_bytes());
    bytes
}

pub fn split_stage_complete(bytes: &[u8]) -> (Vec<u8>, Vec<SessionId>) {
    let mut kept = bytes.to_vec();
    let mut ids = Vec::new();
    while let Some((next, id)) = take_stage_complete_marker(&kept) {
        kept = next;
        ids.push(id);
    }
    (kept, ids)
}

fn take_stage_complete_marker(bytes: &[u8]) -> Option<(Vec<u8>, SessionId)> {
    let start = bytes
        .windows(STAGE_COMPLETE_OSC_PREFIX.len())
        .position(|window| window == STAGE_COMPLETE_OSC_PREFIX)?;
    let rest = &bytes[start + STAGE_COMPLETE_OSC_PREFIX.len()..];
    let term = rest.iter().position(|&b| b == 0x07 || b == 0x1b)?;
    let text = std::str::from_utf8(&rest[..term]).ok()?;
    let id = text.parse::<u64>().ok()?;
    let mut end = start + STAGE_COMPLETE_OSC_PREFIX.len() + term + 1;
    if rest[term] == 0x1b && rest.get(term + 1) == Some(&0x5c) {
        end += 1;
    }
    let end = end.min(bytes.len());
    let mut kept = Vec::with_capacity(bytes.len().saturating_sub(end - start));
    kept.extend_from_slice(&bytes[..start]);
    kept.extend_from_slice(&bytes[end..]);
    Some((kept, SessionId::from(id)))
}

// Snapshot history options, suppress only this injected setup/body, then restore
// the prior enabled/disabled and set/unset state before the user prompt.
fn in_band_history_setup(shell_type: ShellType) -> &'static [u8] {
    match shell_type {
        ShellType::Bash => {
            b"if shopt -qo history; then __warp_hist_on=1; else __warp_hist_on=0; fi; if [ \"${HISTFILE+x}\" ]; then __warp_histfile=$HISTFILE; __warp_histfile_set=1; else __warp_histfile_set=0; fi; if [ \"${HISTCONTROL+x}\" ]; then __warp_histcontrol=$HISTCONTROL; __warp_histcontrol_set=1; else __warp_histcontrol_set=0; fi; if [ \"${HISTIGNORE+x}\" ]; then __warp_histignore=$HISTIGNORE; __warp_histignore_set=1; else __warp_histignore_set=0; fi; __warp_histcmd=${HISTCMD-}; set +o history; HISTFILE=/dev/null; HISTCONTROL=ignorespace; HISTIGNORE='*'\n"
        }
        ShellType::Zsh => {
            b"if (( ${+HISTFILE} )); then typeset __warp_histfile=$HISTFILE; typeset __warp_histfile_set=1; else typeset __warp_histfile_set=0; fi; if (( ${+SAVEHIST} )); then typeset __warp_savehist=$SAVEHIST; typeset __warp_savehist_set=1; else typeset __warp_savehist_set=0; fi; fc -p /dev/null\n"
        }
        ShellType::Fish => {
            b"if set -q fish_history; set -g __warp_fish_history $fish_history; set -g __warp_fish_history_set 1; else; set -g __warp_fish_history_set 0; end; set -g fish_history ''\n"
        }
        ShellType::PowerShell => b"",
    }
}

fn in_band_history_restore(shell_type: ShellType) -> &'static [u8] {
    match shell_type {
        ShellType::Bash => {
            br#"[ -n "${__warp_histcmd}" ] && history -d "${__warp_histcmd}" 2>/dev/null; if [ "${__warp_histfile_set}" = 1 ]; then HISTFILE=$__warp_histfile; else unset HISTFILE; fi; if [ "${__warp_histcontrol_set}" = 1 ]; then HISTCONTROL=$__warp_histcontrol; else unset HISTCONTROL; fi; if [ "${__warp_histignore_set}" = 1 ]; then HISTIGNORE=$__warp_histignore; else unset HISTIGNORE; fi; if [ "${__warp_hist_on}" = 1 ]; then set -o history; else set +o history; fi; unset __warp_histfile __warp_histfile_set __warp_histcontrol __warp_histcontrol_set __warp_histignore __warp_histignore_set __warp_histcmd __warp_hist_on"#
        }
        ShellType::Zsh => {
            b"fc -P; if (( __warp_histfile_set )); then HISTFILE=$__warp_histfile; else unset HISTFILE; fi; if (( __warp_savehist_set )); then SAVEHIST=$__warp_savehist; else unset SAVEHIST; fi; unset __warp_histfile __warp_histfile_set __warp_savehist __warp_savehist_set"
        }
        ShellType::Fish => {
            b"if test \"$__warp_fish_history_set\" = 1; set -g fish_history $__warp_fish_history; else; set -e fish_history; end; set -e __warp_fish_history __warp_fish_history_set"
        }
        ShellType::PowerShell => b"",
    }
}

fn history_isolated_script(shell_type: ShellType, body: &[u8]) -> Vec<u8> {
    let mut bytes = in_band_history_setup(shell_type).to_vec();
    bytes.extend_from_slice(body);
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(in_band_history_restore(shell_type));
    bytes.extend_from_slice(shell_type.execute_command_bytes());
    bytes
}

// send-keys paste is echoed by the tty; disable echo and clear so setup never remains visible.
pub(crate) fn silent_history_isolated_script(shell_type: ShellType, body: &[u8]) -> Vec<u8> {
    match shell_type {
        ShellType::Zsh => silent_zsh_script(body),
        ShellType::PowerShell => history_isolated_script(shell_type, body),
        shell_type => silent_posix_script(shell_type, body),
    }
}

fn silent_posix_script(shell_type: ShellType, body: &[u8]) -> Vec<u8> {
    let mut wrapped = b"stty -echo 2>/dev/null || true\n".to_vec();
    wrapped.extend_from_slice(b"trap 'stty echo 2>/dev/null || true' EXIT INT TERM\n");
    wrapped.extend_from_slice(in_band_history_setup(shell_type));
    wrapped.extend_from_slice(body);
    if !wrapped.ends_with(b"\n") {
        wrapped.push(b'\n');
    }
    wrapped.extend_from_slice(br"printf '\033[H\033[2J' 2>/dev/null || true");
    wrapped.push(b'\n');
    wrapped.extend_from_slice(in_band_history_restore(shell_type));
    if !wrapped.ends_with(b"\n") {
        wrapped.push(b'\n');
    }
    wrapped.extend_from_slice(b"stty echo 2>/dev/null || true\n");
    wrapped.extend_from_slice(b"trap - EXIT INT TERM");
    wrapped.extend_from_slice(shell_type.execute_command_bytes());
    wrapped
}

fn silent_zsh_script(body: &[u8]) -> Vec<u8> {
    // Keep echo off for the whole injected tail so send-keys after restore is not
    // painted. INT/TERM still restores echo because the epilogue may not run.
    let mut wrapped = br"stty -echo 2>/dev/null || true
if [[ -o banghist ]]; then typeset __warp_banghist=1; else typeset __warp_banghist=0; fi
if [[ -o histfcntllock ]]; then typeset __warp_hist_fcntl=1; else typeset __warp_hist_fcntl=0; fi
if (( ${+HISTFILE} )); then typeset __warp_histfile=$HISTFILE; typeset __warp_histfile_set=1; else typeset __warp_histfile_set=0; fi
if (( ${+SAVEHIST} )); then typeset __warp_savehist=$SAVEHIST; typeset __warp_savehist_set=1; else typeset __warp_savehist_set=0; fi
__warp_silent_cleanup() {
  (( ${+__warp_silent_cleaned} )) && return
  __warp_silent_cleaned=1
  if (( __warp_histfile_set )); then HISTFILE=$__warp_histfile; else unset HISTFILE; fi
  if (( __warp_savehist_set )); then SAVEHIST=$__warp_savehist; else unset SAVEHIST; fi
  if (( __warp_banghist )); then setopt BANG_HIST; else unsetopt BANG_HIST; fi
  if (( __warp_hist_fcntl )); then setopt HIST_FCNTL_LOCK 2>/dev/null || true; else unsetopt HIST_FCNTL_LOCK 2>/dev/null || true; fi
  unset __warp_histfile __warp_histfile_set __warp_savehist __warp_savehist_set __warp_banghist __warp_hist_fcntl
  unfunction __warp_silent_cleanup 2>/dev/null || true
  stty echo 2>/dev/null || true
}
setopt NO_BANG_HIST
unsetopt HIST_FCNTL_LOCK 2>/dev/null || true
trap '__warp_silent_cleanup' EXIT INT TERM
HISTFILE=/dev/null
SAVEHIST=0
".to_vec();
    wrapped.extend_from_slice(body);
    if !wrapped.ends_with(b"\n") {
        wrapped.push(b'\n');
    }
    wrapped.extend_from_slice(b"trap - EXIT INT TERM\n");
    wrapped.extend_from_slice(
        br"if (( ! ${+__warp_silent_cleaned} )); then
  __warp_silent_cleaned=1
  if (( __warp_histfile_set )); then HISTFILE=$__warp_histfile; else unset HISTFILE; fi
  if (( __warp_savehist_set )); then SAVEHIST=$__warp_savehist; else unset SAVEHIST; fi
  if (( __warp_banghist )); then setopt BANG_HIST; else unsetopt BANG_HIST; fi
  if (( __warp_hist_fcntl )); then setopt HIST_FCNTL_LOCK 2>/dev/null || true; else unsetopt HIST_FCNTL_LOCK 2>/dev/null || true; fi
  unset __warp_histfile __warp_histfile_set __warp_savehist __warp_savehist_set __warp_banghist __warp_hist_fcntl __warp_silent_cleaned
  unfunction __warp_silent_cleanup 2>/dev/null || true
else
  unset __warp_silent_cleaned
fi
",
    );
    wrapped.extend_from_slice(br"printf '\033[H\033[2J' 2>/dev/null || true");
    wrapped.push(b'\n');
    wrapped.extend_from_slice(b"stty echo 2>/dev/null || true\n");
    wrapped
}

pub(crate) fn silent_bootstrap_bytes(shell_type: ShellType) -> Vec<u8> {
    let body = script_for_shell(shell_type, &ASSETS);
    silent_history_isolated_script(shell_type, &body)
}

pub fn in_band_init_bytes(shell_type: ShellType, session_id: SessionId) -> Option<Vec<u8>> {
    match shell_type {
        ShellType::PowerShell => None,
        shell_type => {
            let script = raw_init_shell_script_for_shell(shell_type, &ASSETS, session_id);
            let mut body = script.into_bytes();
            if !body.ends_with(b"\n") {
                body.push(b'\n');
            }
            body.extend_from_slice(&stage_complete_script(shell_type, session_id));
            Some(history_isolated_script(shell_type, &body))
        }
    }
}

pub fn fallback_supported_shell() -> Option<(PathBuf, ShellType)> {
    ["zsh", "bash", "fish"]
        .into_iter()
        .find_map(supported_shell_path_and_type)
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
