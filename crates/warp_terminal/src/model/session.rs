use anyhow::Result;
pub use warp_core::SessionId;
/// Returns the hostname for the local machine where Warp is running.
pub fn get_local_hostname() -> Result<String> {
    cfg_if::cfg_if! {
        if #[cfg(not(target_family = "wasm"))] {
            use gethostname::gethostname;

            gethostname()
                .into_string()
                .map_err(|os_string| {
                    anyhow::anyhow!("Failed to convert local hostname OsString {os_string:?} into String.")
                })
        } else {
            anyhow::bail!("Cannot get machine hostname from wasm")
        }
    }
}
