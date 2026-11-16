use std::collections::HashMap;
use std::time::Duration;

use instant::Instant;
use typed_path::TypedPath;
use warp_completer::completer::{CommandExitStatus, EngineDirEntry};
use warpui::r#async::FutureExt as AsyncFutureExt;

use crate::completer::SessionContext;
use crate::terminal::model::session::ExecuteCommandOptions;

const GUEST_LISTING_TIMEOUT: Duration = Duration::from_millis(500);

#[cfg_attr(not(windows), allow(dead_code))]
pub(super) async fn list_entries(
    session_context: &SessionContext,
    directory: &TypedPath<'_>,
) -> Option<Vec<EngineDirEntry>> {
    let script = super::ls_script_for_dir(directory)?;
    run_guest_listing(session_context, &script, GUEST_LISTING_TIMEOUT).await
}

async fn run_guest_listing(
    session_context: &SessionContext,
    script: &str,
    timeout: Duration,
) -> Option<Vec<EngineDirEntry>> {
    let env_vars = session_context
        .session
        .path()
        .as_deref()
        .map(|path| HashMap::from_iter([("PATH".to_string(), path.to_string())]));

    let started = Instant::now();
    let result = session_context
        .session
        .execute_command(script, None, env_vars, ExecuteCommandOptions::default())
        .with_timeout(timeout)
        .await;
    let elapsed_ms = started.elapsed().as_millis();

    match result {
        Ok(Ok(output)) if output.status == CommandExitStatus::Success => {
            match super::parse_ls_script_output(output.output()) {
                Some(entries) => {
                    log::debug!(
                        "[APP-3993 wsl-list] ok entries={} elapsed_ms={elapsed_ms}",
                        entries.len()
                    );
                    Some(entries)
                }
                None => {
                    log::warn!(
                        "[APP-3993 wsl-list] malformed or truncated output elapsed_ms={elapsed_ms}, falling back to host listing"
                    );
                    None
                }
            }
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
