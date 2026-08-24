//! Low-level PTY tests for the zsh bootstrap heredoc-paste hardening (APP-5385).
//!
//! These spawn a real zsh process attached to a real PTY and drive it with the exact bytes
//! `init_shell_script_for_shell` / `script_for_shell` produce for zsh — the same bytes
//! `PtyController` writes in the real app. Unlike the full GUI integration test suite
//! (`crates/integration`), this lets us assert directly on the tty's line discipline (raw vs.
//! canonical, echo on/off) at each step, which is the actual mechanism this fix changes. A
//! user-visible reproduction of the reported leak depends on a timing race that neither this
//! sandbox nor a real macOS runner reliably hit end-to-end through the full app; these tests
//! instead pin the underlying mechanism directly, so they fail against the pre-fix scripts.

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::path::Path;
use std::process::Child;
use std::time::Duration;

use command::blocking::Command;
use instant::Instant;
use nix::pty::openpty;
use nix::sys::termios::{self, LocalFlags};

use super::*;
use crate::ASSETS;

/// Resolves the path to the zsh binary used to drive these tests, or `None` if zsh isn't
/// installed. Tests skip (rather than fail) when zsh is unavailable.
fn resolve_zsh() -> Option<&'static str> {
    [
        "/bin/zsh",
        "/usr/bin/zsh",
        "/usr/local/bin/zsh",
        "/opt/homebrew/bin/zsh",
    ]
    .into_iter()
    .find(|path| Path::new(path).is_file())
}

/// Spawns `zsh` the same way Warp does for a local session (`exec -a -zsh <path> -g --no-rcs`,
/// see `arguments_for_session_spawning_command`), attached to a fresh PTY, with `HOME` pointed
/// at `home_dir`.
fn spawn_zsh_in_pty(zsh_path: &str, home_dir: &Path) -> (std::fs::File, Child) {
    // Use a wide terminal so a long `stty -g` state string (used by one of the tests below)
    // never gets line-wrapped by the tty driver, which would otherwise split it across lines.
    let win_size = libc::winsize {
        ws_row: 24,
        ws_col: 1000,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let ends = openpty(Some(&win_size), None).expect("openpty should succeed");
    let master_fd: RawFd = ends.master;
    let slave_fd: RawFd = ends.slave;

    let mut command = Command::new(zsh_path);
    command
        .arg("-c")
        .arg(format!("exec -a -zsh '{zsh_path}' -g --no-rcs"))
        .env("HOME", home_dir)
        .env("TERM", "xterm-256color")
        .env_remove("ZDOTDIR");

    // SAFETY: the closure only calls async-signal-safe functions (setsid, dup2, ioctl, close).
    unsafe {
        command.pre_exec(move || {
            libc::setsid();
            libc::dup2(slave_fd, 0);
            libc::dup2(slave_fd, 1);
            libc::dup2(slave_fd, 2);
            #[allow(clippy::cast_lossless)]
            libc::ioctl(slave_fd, libc::TIOCSCTTY as _, 0);
            if slave_fd > 2 {
                libc::close(slave_fd);
            }
            Ok(())
        });
    }

    let child = command.spawn().expect("zsh should spawn");
    unsafe {
        libc::close(slave_fd);
    }
    let master = unsafe { std::fs::File::from_raw_fd(master_fd) };
    (master, child)
}

/// Reads whatever is available from `master`, appending it to `buf`, until `stop_when` returns
/// `true` for the accumulated buffer or `timeout` elapses.
fn read_into(
    master: &mut std::fs::File,
    buf: &mut Vec<u8>,
    timeout: Duration,
    stop_when: impl Fn(&[u8]) -> bool,
) {
    let fd = master.as_raw_fd();
    // SAFETY: fcntl with F_GETFL/F_SETFL on a valid, open fd is always safe.
    let original_flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    unsafe {
        libc::fcntl(fd, libc::F_SETFL, original_flags | libc::O_NONBLOCK);
    }
    let deadline = Instant::now() + timeout;
    let mut chunk = [0u8; 4096];
    while Instant::now() < deadline {
        match master.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if stop_when(buf) {
                    break;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => panic!("unexpected error reading from pty: {err}"),
        }
    }
    // Restore the fd to blocking mode so writes made between calls to `read_into` (e.g. pasting
    // the multi-KB bootstrap script) block on backpressure instead of failing with EWOULDBLOCK.
    unsafe {
        libc::fcntl(fd, libc::F_SETFL, original_flags);
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn count_occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = find_subslice(&haystack[start..], needle) {
        count += 1;
        start += pos + needle.len();
    }
    count
}

/// Returns `true` if `buf` contains a fully-received DCS hook (`ESC P $ d <hex> ESC \`) whose
/// decoded JSON payload's `"hook"` field is `hook_name`.
fn contains_hook(buf: &[u8], hook_name: &str) -> bool {
    const START_MARKER: &[u8] = b"\x1bP$d";
    const END_MARKER: &[u8] = b"\x1b\\";
    let needle = format!("\"hook\": \"{hook_name}\"");

    let mut search_from = 0;
    while let Some(rel_start) = find_subslice(&buf[search_from..], START_MARKER) {
        let payload_start = search_from + rel_start + START_MARKER.len();
        let Some(rel_end) = find_subslice(&buf[payload_start..], END_MARKER) else {
            return false;
        };
        let hex_payload = &buf[payload_start..payload_start + rel_end];
        if let Ok(decoded) = hex::decode(hex_payload)
            && let Ok(json) = String::from_utf8(decoded)
            && json.contains(&needle)
        {
            return true;
        }
        search_from = payload_start + rel_end + END_MARKER.len();
    }
    false
}

fn tty_local_flags(fd: RawFd) -> LocalFlags {
    termios::tcgetattr(fd)
        .expect("tcgetattr should succeed on a live pty")
        .local_flags
}

/// Waits until the tty attached to `fd` is in raw, echo-disabled mode, or `timeout` elapses.
fn wait_for_raw_noecho(fd: RawFd, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let flags = tty_local_flags(fd);
        if !flags.contains(LocalFlags::ECHO) && !flags.contains(LocalFlags::ICANON) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Creates a hermetic, empty-rc-files `HOME` directory. Both tests target the paste mechanism
/// itself, which — as demonstrated in the fix's commit message — is vulnerable regardless of
/// `.zshrc` content; they don't need to reproduce any particular rc-file trigger.
fn hermetic_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("should create temp home dir");
    std::fs::write(home.path().join(".zshrc"), "").expect("write .zshrc");
    std::fs::write(home.path().join(".zshenv"), "").expect("write .zshenv");
    home
}

#[test]
fn zsh_bootstrap_paste_does_not_echo_its_own_source_and_next_command_runs_uncorrupted() {
    let Some(zsh_path) = resolve_zsh() else {
        return;
    };
    let home = hermetic_home();
    let (mut master, mut child) = spawn_zsh_in_pty(zsh_path, home.path());
    let master_fd = master.as_raw_fd();

    let session_id = generate_session_id();
    let init_script = init_shell_script_for_shell(ShellType::Zsh, &ASSETS, session_id);
    master
        .write_all(init_script.as_bytes())
        .expect("write init script");
    master.write_all(b"\n").expect("write newline");

    // Wait for the init script to actually put the tty into raw/-echo mode, rather than
    // sleeping a fixed amount of time.
    wait_for_raw_noecho(master_fd, Duration::from_secs(5));
    let flags_during_paste = tty_local_flags(master_fd);
    assert!(
        !flags_during_paste.contains(LocalFlags::ECHO),
        "tty should have local echo disabled before the bootstrap script is pasted, got {flags_during_paste:?}"
    );
    assert!(
        !flags_during_paste.contains(LocalFlags::ICANON),
        "tty should be in non-canonical (raw) mode before the bootstrap script is pasted, got {flags_during_paste:?}"
    );

    // Discard whatever the init script itself produced (including its own echo, which is
    // expected and correct: the tty was still in normal canonical/echo mode while we were
    // typing that command, *before* it switched the tty to raw/-echo). The assertions below
    // are specifically about the bootstrap script paste that follows, not this setup step.
    let mut discard = Vec::new();
    read_into(
        &mut master,
        &mut discard,
        Duration::from_millis(200),
        |_| false,
    );

    let bootstrap_script = script_for_shell(ShellType::Zsh, &ASSETS);
    master
        .write_all(&bootstrap_script)
        .expect("write bootstrap script");

    let mut observed = Vec::new();
    read_into(&mut master, &mut observed, Duration::from_secs(10), |buf| {
        contains_hook(buf, "Bootstrapped")
    });
    assert!(
        contains_hook(&observed, "Bootstrapped"),
        "zsh should finish bootstrapping and emit the Bootstrapped hook; got: {}",
        String::from_utf8_lossy(&observed)
    );

    // The heredoc's own source text (the variable it reads into, and the `read` invocation
    // itself) must never appear as literal, visible bytes: if it does, some portion of the
    // pasted script was echoed back rather than silently consumed as input.
    assert!(
        find_subslice(&observed, b"WARP_BOOTSTRAP_VAR").is_none(),
        "the heredoc variable name leaked into the pty output, i.e. bootstrap source was echoed: {}",
        String::from_utf8_lossy(&observed)
    );
    assert!(
        find_subslice(&observed, b"read -r -d").is_none(),
        "the heredoc's own `read` invocation leaked into the pty output: {}",
        String::from_utf8_lossy(&observed)
    );

    // Run a plain command right after bootstrap completes and confirm its exact text made it
    // through uncorrupted — this is what a leaked `EOM` (or other bootstrap source text) would
    // have glued itself onto in the original bug report (e.g. the reported `EOMls`).
    let mut marker_observed = observed;
    master
        .write_all(b"echo WARP_TEST_MARKER_7f3a9c\n")
        .expect("write marker command");
    read_into(
        &mut master,
        &mut marker_observed,
        Duration::from_secs(5),
        |buf| count_occurrences(buf, b"WARP_TEST_MARKER_7f3a9c") >= 2,
    );
    assert!(
        find_subslice(&marker_observed, b"echo WARP_TEST_MARKER_7f3a9c").is_some(),
        "the next command should run with its exact text intact, not glued to leaked bootstrap output: {}",
        String::from_utf8_lossy(&marker_observed)
    );

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn zsh_bootstrap_restores_the_users_own_tty_settings_rather_than_a_generic_profile() {
    let Some(zsh_path) = resolve_zsh() else {
        return;
    };
    let home = hermetic_home();
    let (mut master, mut child) = spawn_zsh_in_pty(zsh_path, home.path());
    let master_fd = master.as_raw_fd();

    // Disable zsh's own line editor for this setup step so the terminal isn't also being
    // redrawn by zle while we simulate the user's pre-existing customization below (Warp's
    // own init script disables it the same way moments later; doing so here first just keeps
    // this setup step's rendering simple and doesn't affect what's under test).
    master
        .write_all(b" unsetopt zle\n")
        .expect("write unsetopt zle");
    let mut setup_discard = Vec::new();
    read_into(
        &mut master,
        &mut setup_discard,
        Duration::from_millis(300),
        |_| false,
    );

    // Simulate a user who customized their terminal discipline before Warp's bootstrap runs —
    // matching the exact repro used to catch a regression here: flow control disabled and a
    // non-default erase key. A generic `stty sane` restore would silently reset both of these.
    master
        .write_all(b" command -p stty -ixon erase '^H'; command -p stty -g\n")
        .expect("write custom stty command");
    let mut before_output = Vec::new();
    read_into(
        &mut master,
        &mut before_output,
        Duration::from_millis(500),
        |_| false,
    );
    let stty_state_before = last_line_of_stty_g_output(&before_output);

    let session_id = generate_session_id();
    let init_script = init_shell_script_for_shell(ShellType::Zsh, &ASSETS, session_id);
    master
        .write_all(init_script.as_bytes())
        .expect("write init script");
    master.write_all(b"\n").expect("write newline");
    wait_for_raw_noecho(master_fd, Duration::from_secs(5));

    // Discard whatever the init script itself produced (see the sibling test for why this is
    // expected and unrelated to what we're asserting here).
    let mut discard = Vec::new();
    read_into(
        &mut master,
        &mut discard,
        Duration::from_millis(200),
        |_| false,
    );

    let bootstrap_script = script_for_shell(ShellType::Zsh, &ASSETS);
    master
        .write_all(&bootstrap_script)
        .expect("write bootstrap script");
    let mut observed = Vec::new();
    read_into(&mut master, &mut observed, Duration::from_secs(10), |buf| {
        contains_hook(buf, "Bootstrapped")
    });
    assert!(
        contains_hook(&observed, "Bootstrapped"),
        "zsh should finish bootstrapping; got: {}",
        String::from_utf8_lossy(&observed)
    );

    let mut after_output = Vec::new();
    master
        .write_all(b" command -p stty -g\n")
        .expect("write stty -g command");
    read_into(
        &mut master,
        &mut after_output,
        Duration::from_millis(500),
        |_| false,
    );
    let stty_state_after = last_line_of_stty_g_output(&after_output);

    assert_eq!(
        stty_state_after, stty_state_before,
        "bootstrap should restore the user's own tty settings (e.g. the custom erase key and \
         disabled flow control), not a generic 'sane' profile"
    );

    let _ = child.kill();
    let _ = child.wait();
}

/// Extracts the machine-readable `stty -g` state string (a long `:`-separated hex value) from
/// raw pty output that also contains shell echo, prompts, DCS hooks, and other escape
/// sequences. Rather than requiring a whole *line* of pure hex/colon output — which breaks
/// when the value happens to immediately follow a DCS terminator on the same line, with no
/// `\n` in between — this scans for the last contiguous run of hex-digit-or-colon bytes that
/// actually contains a colon, which is what distinguishes it from a hex-encoded DCS payload
/// (pure hex, no colons) sitting right next to it.
fn last_line_of_stty_g_output(buf: &[u8]) -> String {
    // Long enough that it can't be mistaken for an accidental short run, but well under the
    // length of the hex-encoded DCS hook payloads nearby (which, unlike this, never contain a
    // ':' since they're pure hex encoding).
    const MIN_LEN: usize = 20;
    let is_state_char = |b: u8| b.is_ascii_hexdigit() || b == b':';

    let mut last_match: Option<&[u8]> = None;
    let mut run_start = None;
    for (i, &b) in buf.iter().chain(std::iter::once(&b'\0')).enumerate() {
        if is_state_char(b) {
            run_start.get_or_insert(i);
        } else if let Some(start) = run_start.take() {
            let run = &buf[start..i];
            if run.len() >= MIN_LEN && run.contains(&b':') {
                last_match = Some(run);
            }
        }
    }

    match last_match {
        Some(state) => String::from_utf8_lossy(state).into_owned(),
        None => panic!(
            "expected an `stty -g` state value in output: {}",
            String::from_utf8_lossy(buf)
        ),
    }
}
