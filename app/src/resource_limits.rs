/// Adjusts resource limits applied to the Warp process (e.g.: the limit on open
/// file descriptors) to ensure proper behavior.
pub fn adjust_resource_limits() {
    #[cfg(target_os = "macos")]
    {
        /// Our desired limit on the maximum number of file descriptors we can open.
        ///
        /// MacOS sets this value to 256, by default, which can interfere with
        /// normal usage of Warp for users who open lots of tabs.
        const TARGET_FD_LIMIT: u64 = 2560;

        use nix::sys::resource::Resource::RLIMIT_NOFILE;
        use nix::sys::resource::{getrlimit, setrlimit};

        let (cur_limit, hard_limit) = match getrlimit(RLIMIT_NOFILE) {
            Ok(val) => val,
            Err(err) => {
                log::error!("Failed to retrieve resource limit for number of files: {err:#}");
                return;
            }
        };

        log::info!(
            "Initial open file descriptor limit is {cur_limit}, with a hard limit of {hard_limit}"
        );
        let new_limit = TARGET_FD_LIMIT.min(hard_limit);
        if cur_limit < new_limit {
            match setrlimit(RLIMIT_NOFILE, new_limit, hard_limit) {
                Ok(_) => log::info!("Increased open file descriptor limit to {new_limit}"),
                Err(err) => log::error!("Failed to increase open file descriptor limit: {err:#}"),
            }
        }
    }
}
