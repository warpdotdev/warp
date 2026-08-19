use std::collections::HashMap;
use std::time::Duration;

use instant::Instant;
use typed_path::TypedPath;
use warp_completer::completer::{CommandExitStatus, EngineDirEntry};
use warpui::r#async::FutureExt as AsyncFutureExt;

use crate::completer::SessionContext;
use crate::terminal::model::session::ExecuteCommandOptions;

/// Budget for a full guest-driven directory listing. The host cannot resolve an
/// `IO_REPARSE_TAG_LX_SYMLINK` over `\\wsl$` (APP-3993), so a WSL directory listing goes through
/// the guest unconditionally rather than as a rare patch to a host listing. That makes this call
/// part of the primary listing path for every fresh WSL directory view, so it gets its own
/// budget instead of reusing one sized for a narrower, less frequent call.
const GUEST_LISTING_TIMEOUT: Duration = Duration::from_secs(5);

/// Lists `directory`'s entries by asking the WSL guest directly, following symlinks (`-L`) so a
/// symlink-to-directory completes as a directory and a directory reached only by traversing a
/// symlink can be listed at all -- both cases the host cannot handle over `\\wsl$`.
///
/// Returns `None` on any failure (timeout, non-zero exit, or output the parser can't use) so the
/// caller can fall back to the host listing. A slow or failing guest must never leave the user
/// with fewer completions than the host-only listing already provides.
pub(super) async fn list_entries(
    session_context: &SessionContext,
    directory: &TypedPath<'_>,
) -> Option<Vec<EngineDirEntry>> {
    let script = super::ls_script_for_dir(directory)?;
    let env_vars = session_context
        .session
        .path()
        .as_deref()
        .map(|path| HashMap::from_iter([("PATH".to_string(), path.to_string())]));

    let started = Instant::now();
    let result = session_context
        .session
        .execute_command(&script, None, env_vars, ExecuteCommandOptions::default())
        .with_timeout(GUEST_LISTING_TIMEOUT)
        .await;
    let elapsed_ms = started.elapsed().as_millis();

    match result {
        Ok(Ok(output)) if output.status == CommandExitStatus::Success => {
            let entries = super::parse_ls_script_output(output.output());
            log::debug!(
                "[APP-3993 wsl-list] ok entries={} elapsed_ms={elapsed_ms}",
                entries.len()
            );
            Some(entries)
        }
        Ok(Ok(_)) => {
            log::warn!(
                "[APP-3993 wsl-list] non-zero exit elapsed_ms={elapsed_ms}, falling back to host listing"
            );
            None
        }
        Ok(Err(err)) => {
            log::warn!(
                "[APP-3993 wsl-list] failed elapsed_ms={elapsed_ms}, falling back to host listing: {err:#}"
            );
            None
        }
        Err(_timed_out) => {
            log::warn!(
                "[APP-3993 wsl-list] timed out elapsed_ms={elapsed_ms}, falling back to host listing"
            );
            None
        }
    }
}

#[cfg(test)]
#[path = "wsl_guest_listing_tests.rs"]
mod tests;
