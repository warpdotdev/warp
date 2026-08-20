use itertools::Itertools;
use rand::Rng;
use warp_core::SessionId;
use warpui::AssetProvider;

use crate::shell::ShellType;

const BYTE_ORDER_MARK: &str = "\u{FEFF}";
/// Returns the script in the file at `file_path` to be passed as a single-quoted argument in the
/// shell (e.g. as a single quoted argument to `eval`).
///
/// The script is transformed in two ways:
///   * Newlines are stripped and replaced with semi-colons
///   * Single quotes are escaped (' is replaced with '"'"')
///   * Lines starting with '#' are removed -- this enables use of comments in scripts. Note,
///   however, that you still cannot use a 'partial line' comment, since this logic only considers
///   whole lines.
pub fn load_and_escape_script(file_path: &str, assets: &dyn AssetProvider) -> String {
    load_script(file_path, assets)
        .replace('\'', r#"'"'"'"#)
        .replace("@@USING_CON_PTY_BOOLEAN@@", &(cfg!(windows).to_string()))
}

fn load_script(file_path: &str, assets: &dyn AssetProvider) -> String {
    let script_bytes = assets
        .get(file_path)
        .unwrap_or_else(|_| panic!("Failed to retrieve {file_path} from assets"));

    std::str::from_utf8(&script_bytes)
        .expect("InitShell script should be utf8 encoded.")
        .trim_start_matches(BYTE_ORDER_MARK)
        .lines()
        .filter(|line| !line.trim_start().starts_with('#') && !line.trim().is_empty())
        .join(";")
}
/// Returns the raw init shell script for the given `shell_type`, without
/// single-quote escaping. Suitable for passing as an environment variable
/// where the caller controls the eval context (e.g. Docker sandbox init).
///
/// Gated on `unix` because the sole caller today is the Unix Docker
/// sandbox spawn path (`local_tty::unix::prepare_docker_sandbox`); on
/// Windows/wasm the function is dead code.
#[cfg(unix)]
pub fn raw_init_shell_script_for_shell(
    shell_type: ShellType,
    assets: &dyn AssetProvider,
    session_id: SessionId,
) -> String {
    let file = match shell_type {
        ShellType::Bash => "bundled/bootstrap/bash_init_shell.sh",
        ShellType::Zsh => "bundled/bootstrap/zsh_init_shell.sh",
        ShellType::Fish => "bundled/bootstrap/fish_init_shell.sh",
        ShellType::PowerShell => "bundled/bootstrap/pwsh_init_shell.ps1",
    };
    load_script(file, assets)
        .replace("@@USING_CON_PTY_BOOLEAN@@", &(cfg!(windows).to_string()))
        .replace(SESSION_ID_PLACEHOLDER, &session_id.as_u64().to_string())
}
/// Placeholder in init shell scripts that gets replaced with the client-generated session ID.
pub const SESSION_ID_PLACEHOLDER: &str = "@@WARP_SESSION_ID@@";

/// Returns the init shell script for the given `shell_type` (e.g. the script that emits the
/// InitShell DCS hook).
///
/// The returned script is one line and, for shells that need it, has escaped single-quotes for the
/// purposes of being passed as a single-quoted argument to 'eval'.
pub fn init_shell_script_for_shell(
    shell_type: ShellType,
    assets: &dyn AssetProvider,
    session_id: SessionId,
) -> String {
    let script = match shell_type {
        ShellType::Zsh => load_and_escape_script("bundled/bootstrap/zsh_init_shell.sh", assets),
        ShellType::Bash => load_and_escape_script("bundled/bootstrap/bash_init_shell.sh", assets),
        ShellType::Fish => load_and_escape_script("bundled/bootstrap/fish_init_shell.sh", assets),
        ShellType::PowerShell => load_script("bundled/bootstrap/pwsh_init_shell.ps1", assets),
    };
    script.replace(SESSION_ID_PLACEHOLDER, &session_id.as_u64().to_string())
}
/// Generates a cryptographically random session ID for use as both a session
/// identifier and an integrity token for DCS hook validation.
pub fn generate_session_id() -> SessionId {
    let mut rng = rand::thread_rng();
    loop {
        let session_id = rng.r#gen::<u64>();
        if session_id != 0 {
            return SessionId::from(session_id);
        }
    }
}
