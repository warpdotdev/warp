use std::sync::OnceLock;
use std::{env, path};

use anyhow::{Result, anyhow};
use warpui::{AppContext, SingletonEntity};

use crate::system::SystemInfo;

const KASPERSKY_PROCESS_NAME: &str = "avp";

#[cfg(feature = "local_fs")]
pub fn install_dir() -> Result<path::PathBuf> {
    let current_exe = env::current_exe()?;
    current_exe
        .parent()
        .map(ToOwned::to_owned)
        .ok_or(anyhow!("Unable to get install dir"))
}

static KASPERSKY_RUNNING: OnceLock<bool> = OnceLock::new();

/// Determines if Kaspersky is currently running by checking if there is a
/// process with the name "avp" running.
///
/// The result is cached for the lifetime of the process: antivirus presence does not meaningfully
/// change mid-session, and the full process-table enumeration this requires is expensive enough on
/// Windows that repeating it per session bootstrap can trip the DPC watchdog.
pub fn is_kaspersky_running(ctx: &mut AppContext) -> bool {
    if let Some(cached) = KASPERSKY_RUNNING.get() {
        return *cached;
    }

    let running = SystemInfo::handle(ctx).update(ctx, |system_info, _| {
        system_info.refresh_all_processes();
        system_info
            .processes_by_name(KASPERSKY_PROCESS_NAME)
            .next()
            .is_some()
    });

    *KASPERSKY_RUNNING.get_or_init(|| running)
}
