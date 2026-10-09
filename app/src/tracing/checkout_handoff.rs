use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow};
use serde::{Deserialize, Serialize};
use tempfile::{NamedTempFile, TempPath};

use super::cloud_agent_auth::CredentialSnapshot;

const MAX_HANDOFF_BYTES: u64 = 64 * 1024;

#[derive(Serialize, Deserialize)]
pub(super) struct CheckoutTracingConfig {
    pub(super) endpoint: String,
    pub(super) credential: CredentialSnapshot,
}

fn handoff_path(requests_file: &Path) -> PathBuf {
    let mut path = requests_file.as_os_str().to_owned();
    path.push(".otlp");
    path.into()
}

impl CheckoutTracingConfig {
    pub(super) fn write(self, requests_file: &Path) -> anyhow::Result<TempPath> {
        let directory = requests_file
            .parent()
            .context("Missing checkout request directory")?;
        let mut file = NamedTempFile::new_in(directory)
            .map_err(|_| anyhow!("Could not create checkout tracing handoff"))?;
        serde_json::to_writer(&mut file, &self)
            .map_err(|_| anyhow!("Could not write checkout tracing handoff"))?;
        file.flush()
            .map_err(|_| anyhow!("Could not flush checkout tracing handoff"))?;
        let path = handoff_path(requests_file);
        file.into_temp_path()
            .persist_noclobber(&path)
            .map_err(|_| anyhow!("Could not publish checkout tracing handoff"))?;
        Ok(TempPath::from_path(path))
    }

    pub(super) fn consume(requests_file: &Path) -> anyhow::Result<Option<Self>> {
        let path = handoff_path(requests_file);
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(anyhow!("Could not open checkout tracing handoff")),
        };
        let result = (|| {
            let metadata = file
                .metadata()
                .map_err(|_| anyhow!("Could not inspect checkout tracing handoff"))?;
            anyhow::ensure!(metadata.is_file(), "Checkout tracing handoff is not a file");
            #[cfg(unix)]
            {
                use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
                anyhow::ensure!(
                    metadata.permissions().mode() & 0o077 == 0
                        && metadata.uid() == unsafe { libc::geteuid() },
                    "Checkout tracing handoff is not private"
                );
            }
            let mut bytes = Vec::new();
            file.take(MAX_HANDOFF_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| anyhow!("Could not read checkout tracing handoff"))?;
            anyhow::ensure!(
                bytes.len() as u64 <= MAX_HANDOFF_BYTES,
                "Checkout tracing handoff is too large"
            );
            Ok(bytes)
        })();
        fs::remove_file(&path).map_err(|_| anyhow!("Could not remove checkout tracing handoff"))?;
        let bytes = result?;
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| anyhow!("Invalid checkout tracing handoff"))
    }
}

#[cfg(test)]
#[path = "checkout_handoff_tests.rs"]
mod tests;
