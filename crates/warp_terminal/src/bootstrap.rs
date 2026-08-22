use std::borrow::Cow;

use itertools::Itertools;
use lazy_static::lazy_static;
use memo_map::MemoMap;
use rand::Rng;
use warp_core::SessionId;
use warpui_core::AssetProvider;

use crate::shell::ShellType;

const BYTE_ORDER_MARK: &str = "\u{FEFF}";

lazy_static! {
    /// A memoized cache of the fully-interpolated bootstrap script for each
    /// shell.  We store the full version here as an optimization so that we
    /// don't have to regenerate it every time we spawn a shell.
    static ref BOOTSTRAP_CACHE: MemoMap<ShellType, Vec<u8>> = Default::default();
}
/// Returns the script in the file at `file_path` to be passed as a single-quoted argument in the
/// shell (e.g. as a single quoted argument to `eval`).
///
/// The script is transformed in three ways:
/// - Newlines are stripped and replaced with semicolons.
/// - Single quotes are escaped (`'` is replaced with `'"'"'`).
/// - Lines starting with `#` are removed. This enables comments in scripts, but partial-line
///   comments are not supported because this logic only considers whole lines.
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
/// Returns the bootstrap script that should be used when initializing a shell
/// of the given type.
///
/// This supports a very basic form of interpolation:
///
/// ```shell
/// #include bundled/bootstrap/zsh_body.sh
/// ```
///
/// The directive above instructs this function to replace that line with the
/// contents of the file in our asset cache with the path `bundled/bootstrap/zsh_body.sh`.
///
/// At the moment, this interpolation is only performed for the top-level file,
/// and is not performed recursively, but it would be useful to add such support
/// in the future.
pub fn script_for_shell(shell_type: ShellType, assets: &dyn AssetProvider) -> Cow<'static, [u8]> {
    let file = match shell_type {
        ShellType::Bash => "bash.sh",
        ShellType::Zsh => "zsh.sh",
        ShellType::Fish => "fish.sh",
        ShellType::PowerShell => "pwsh.ps1",
    };

    BOOTSTRAP_CACHE
        .get_or_insert(&shell_type, || {
            let file_path = format!("bundled/bootstrap/{file}");
            let bootstrap = assets
                .get(&file_path)
                .unwrap_or_else(|_| panic!("failed to retrieve {file_path} from assets"));

            // Interpret the file as UTF-8.  We do this in an unchecked way
            // for performance, expecting that any issues here will be caught by
            // unit tests.
            let bootstrap = unsafe { String::from_utf8_unchecked(bootstrap.to_vec()) };

            let additional_files = memo_map::MemoMap::new();

            // Parse through the file, looking for any lines which start with
            // "#include", and replacing that line with the contents of the file
            // located at the path specified.
            //
            // We trim most leading and all trailing whitespace from lines, and
            // drop all empty lines and lines that only contain a comment.  We
            // keep a single leading space on each line, if one exists, to
            // avoid interfering with histignorespace behavior.
            //
            // This minimizes the number of bytes we send over the pty during the
            // bootstrap process.
            fn trim_and_borrow_line(mut line: &str) -> Cow<'_, str> {
                let len = line.len();
                let trimmed_len = line.trim_start().len();
                if trimmed_len < len {
                    let trimmed_chars = len - trimmed_len;
                    line = &line[trimmed_chars - 1..];
                }
                Cow::Borrowed(line.trim_end())
            }
            let mut script = bootstrap
                .trim_start_matches(BYTE_ORDER_MARK)
                .split('\n')
                .map(trim_and_borrow_line)
                .flat_map(|line| {
                    if let Some(path) = line.strip_prefix("#include ") {
                        additional_files
                            .get_or_insert(path, || {
                                let data = assets.get(path).unwrap_or_else(|_| {
                                    panic!("failed to retrieve {path} from assets")
                                });
                                let data_string =
                                    unsafe { String::from_utf8_unchecked(data.to_vec()) };
                                data_string.replace(
                                    "@@USING_CON_PTY_BOOLEAN@@",
                                    &(cfg!(windows).to_string()),
                                )
                            })
                            .split('\n')
                            .map(trim_and_borrow_line)
                            .collect_vec()
                    } else {
                        vec![line]
                    }
                })
                // Filter out empty lines and comments, to minimize the amount
                // of data we send over the pty during the bootstrap process.
                .filter(|line| {
                    let line = line.trim_start();
                    !(line.is_empty()
                        || line.starts_with('#')
                        || shell_type == ShellType::PowerShell
                            && line
                                .starts_with("[Diagnostics.CodeAnalysis.SuppressMessageAttribute"))
                })
                .join("\n");

            // Make sure there's a newline at the end of the bootstrap script,
            // otherwise we'll never submit the final line to the shell.
            script.push('\n');
            script.into_bytes()
        })
        .into()
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

#[cfg(test)]
#[path = "bootstrap_tests.rs"]
mod tests;
