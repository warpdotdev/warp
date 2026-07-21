use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{ErrorKind, Read as _};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use super::*;
use crate::terminal::shell::ShellType;

const CONPTY_SMOKE_ROOT: &str = "WARP_CONPTY_SMOKE_ROOT";
const OUTPUT_SENTINEL: &[u8] = b"WARP_CONPTY_SMOKE_OK";
const SMOKE_TIMEOUT: Duration = Duration::from_secs(10);

struct CurrentDirectoryGuard(PathBuf);

impl Drop for CurrentDirectoryGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}

struct SpawnedPtyGuard(PtySpawnInfo);

impl Drop for SpawnedPtyGuard {
    fn drop(&mut self) {
        unsafe {
            self.0.result.conpty_api.close(self.0.result.pty_handle);
        }
    }
}

#[test]
#[ignore = "requires a packaged Windows ConPTY payload"]
fn spawns_powershell_with_packaged_conpty() {
    let packaged_root = std::env::var_os(CONPTY_SMOKE_ROOT)
        .map(PathBuf::from)
        .expect("WARP_CONPTY_SMOKE_ROOT must point to an installed Warp TUI version")
        .canonicalize()
        .expect("packaged Warp TUI root should exist");
    for relative_path in ["conpty.dll", "x64/OpenConsole.exe"] {
        assert!(
            packaged_root.join(relative_path).is_file(),
            "packaged Warp TUI is missing {relative_path}"
        );
    }

    let original_directory = std::env::current_dir().expect("current directory should be readable");
    std::env::set_current_dir(&packaged_root)
        .expect("packaged Warp TUI root should be usable as the current directory");
    let _current_directory_guard = CurrentDirectoryGuard(original_directory);

    let powershell_path = crate::util::windows::any_powershell_path()
        .expect("PowerShell should be installed")
        .clone();
    let shell_starter = ShellStarter::Direct(DirectShellStarter::new_for_test(
        ShellType::PowerShell,
        powershell_path,
        [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Write-Output WARP_CONPTY_SMOKE_OK; exit 0",
        ]
        .into_iter()
        .map(OsString::from)
        .collect(),
    ));
    let options = PtyOptions {
        size: SizeInfo::new_without_font_metrics(24, 80),
        window_id: None,
        shell_starter,
        start_dir: None,
        env_vars: HashMap::new(),
        enable_ssh_wrapper: false,
        reuse_ssh_control_master: false,
        shell_debug_mode: false,
        honor_ps1: false,
        node_version_chip_enabled: false,
        close_fds: false,
    };
    let (event_loop_tx, _event_loop_rx) = mio_channel::channel();
    let mut spawned_pty =
        SpawnedPtyGuard(spawn(options, event_loop_tx).expect("PowerShell should spawn via ConPTY"));

    let deadline = Instant::now() + SMOKE_TIMEOUT;
    let mut output = Vec::new();
    let mut child_terminated = false;
    while Instant::now() < deadline {
        let mut buffer = [0; 1024];
        match spawned_pty.0.result.pipe.read(&mut buffer) {
            Ok(0) => {}
            Ok(bytes_read) => output.extend_from_slice(&buffer[..bytes_read]),
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::BrokenPipe) => {
            }
            Err(error) => panic!("failed to read ConPTY output: {error}"),
        }

        child_terminated = spawned_pty.0.child.is_terminated();
        if child_terminated
            && output
                .windows(OUTPUT_SENTINEL.len())
                .any(|window| window == OUTPUT_SENTINEL)
        {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    let output = String::from_utf8_lossy(&output);
    assert!(
        output.contains("WARP_CONPTY_SMOKE_OK"),
        "PowerShell did not emit the expected ConPTY output before timeout: {output:?}"
    );
    assert!(
        child_terminated,
        "PowerShell did not terminate before timeout; ConPTY output: {output:?}"
    );
}
