use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

use crate::SizeInfo;
use crate::local_tty::PtyOptions;
use crate::local_tty::shell::{DirectShellStarter, ShellStarter};
use crate::shell::ShellType;

/// Options that run `script` with `/bin/sh -c`.
pub(crate) fn sh_options(script: &str) -> PtyOptions {
    PtyOptions {
        size: SizeInfo::new_without_font_metrics(24, 80),
        window_id: None,
        shell_starter: ShellStarter::Direct(DirectShellStarter::new_for_test(
            ShellType::Bash,
            PathBuf::from("/bin/sh"),
            vec![OsString::from("-c"), OsString::from(script)],
        )),
        start_dir: None,
        env_vars: HashMap::new(),
        enable_ssh_wrapper: false,
        reuse_ssh_control_master: false,
        shell_debug_mode: false,
        honor_ps1: false,
        node_version_chip_enabled: false,
        close_fds: false,
        base_env: None,
        child_rlimits: None,
    }
}
