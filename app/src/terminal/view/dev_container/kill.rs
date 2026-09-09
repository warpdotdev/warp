use std::sync::Arc;

use parking_lot::Mutex;

/// Holds the process-group id until the first terminate, so Drop cannot
/// SIGKILL a pid that has already been reused.
#[derive(Clone)]
pub(crate) struct ProcessGroupKillOnDrop {
    process_group_id: Arc<Mutex<Option<u32>>>,
}

impl ProcessGroupKillOnDrop {
    pub(crate) fn new(process_group_id: u32) -> Self {
        Self {
            process_group_id: Arc::new(Mutex::new(Some(process_group_id))),
        }
    }

    pub(crate) fn terminate_now(&self) {
        if let Some(process_group_id) = self.process_group_id.lock().take() {
            terminate_process_group(process_group_id);
        }
    }
}

impl Drop for ProcessGroupKillOnDrop {
    fn drop(&mut self) {
        self.terminate_now();
    }
}

#[cfg(test)]
thread_local! {
    static PROCESS_GROUP_TERMINATIONS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn take_process_group_terminations() -> u32 {
    PROCESS_GROUP_TERMINATIONS.with(std::cell::Cell::take)
}

pub(crate) fn terminate_process_group(process_group_id: u32) {
    #[cfg(test)]
    PROCESS_GROUP_TERMINATIONS.with(|count| count.set(count.get() + 1));
    #[cfg(unix)]
    {
        use nix::sys::signal::{Signal, kill};
        use nix::unistd::Pid;
        if process_group_id < 2 {
            log::warn!("Refusing to signal process group {process_group_id}: pid is below 2");
            return;
        }
        match kill(Pid::from_raw(-(process_group_id as i32)), Signal::SIGKILL) {
            Ok(()) => log::info!("Sent SIGKILL to process group {process_group_id}"),
            Err(error @ nix::errno::Errno::ESRCH) => {
                log::info!("Process group {process_group_id} had already exited: {error}");
            }
            Err(error) => {
                log::warn!("Failed to kill process group {process_group_id}: {error}");
            }
        }
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};

        if process_group_id < 2 {
            log::warn!("Refusing to terminate process {process_group_id}: pid is below 2");
            return;
        }
        // Windows has no POSIX process groups; `child.id()` is the process id.
        // SAFETY: `process_group_id` is the pid of a process we spawned. The handle is used only
        // to terminate that process and is closed before returning.
        match unsafe { OpenProcess(PROCESS_TERMINATE, false, process_group_id) } {
            Ok(handle) => {
                let result = unsafe { TerminateProcess(handle, 1) };
                let _ = unsafe { CloseHandle(handle) };
                match result {
                    Ok(()) => log::info!("Terminated process {process_group_id}"),
                    Err(error) => {
                        log::warn!("Failed to terminate process {process_group_id}: {error}")
                    }
                }
            }
            Err(error) => {
                log::info!("Process {process_group_id} had already exited: {error}");
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    let _ = process_group_id;
}
