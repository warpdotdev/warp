use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, anyhow};
use opentelemetry::Context;
use opentelemetry::propagation::TextMapPropagator as _;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use serde::{Deserialize, Serialize};
use tempfile::{NamedTempFile, TempPath};
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

use super::cloud_agent_auth::CredentialSnapshot;

const MAX_HANDOFF_BYTES: u64 = 64 * 1024;

#[derive(Serialize, Deserialize)]
pub(super) struct ChildProcessTracingConfig {
    pub(super) endpoint: String,
    pub(super) credential: CredentialSnapshot,
    #[serde(default)]
    trace_context: HashMap<String, String>,
}

fn handoff_path(scope_file: &Path) -> PathBuf {
    let mut path = scope_file.as_os_str().to_owned();
    path.push(".otlp");
    path.into()
}

impl ChildProcessTracingConfig {
    pub(super) fn capture(endpoint: String, credential: CredentialSnapshot) -> Self {
        let mut trace_context = HashMap::new();
        TraceContextPropagator::new()
            .inject_context(&tracing::Span::current().context(), &mut trace_context);
        Self {
            endpoint,
            credential,
            trace_context,
        }
    }

    pub(super) fn parent_context(&self) -> Context {
        TraceContextPropagator::new().extract_with_context(&Context::new(), &self.trace_context)
    }

    pub(super) fn write(self, scope_file: &Path) -> anyhow::Result<TempPath> {
        let directory = scope_file
            .parent()
            .context("Missing child-process handoff directory")?;
        let mut file = NamedTempFile::new_in(directory)
            .map_err(|_| anyhow!("Could not create child-process tracing handoff"))?;
        serde_json::to_writer(&mut file, &self)
            .map_err(|_| anyhow!("Could not write child-process tracing handoff"))?;
        file.flush()
            .map_err(|_| anyhow!("Could not flush child-process tracing handoff"))?;
        let path = handoff_path(scope_file);
        file.into_temp_path()
            .persist_noclobber(&path)
            .map_err(|_| anyhow!("Could not publish child-process tracing handoff"))?;
        Ok(TempPath::from_path(path))
    }

    pub(super) fn consume(scope_file: &Path) -> anyhow::Result<Option<Self>> {
        let path = handoff_path(scope_file);
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            // A substituted FIFO must not block open before the regular-file check.
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(anyhow!("Could not open child-process tracing handoff")),
        };
        let result = (|| {
            let metadata = file
                .metadata()
                .map_err(|_| anyhow!("Could not inspect child-process tracing handoff"))?;
            anyhow::ensure!(
                metadata.is_file(),
                "Child-process tracing handoff is not a file"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
                anyhow::ensure!(
                    metadata.permissions().mode() & 0o077 == 0
                        && metadata.uid() == unsafe { libc::geteuid() },
                    "Child-process tracing handoff is not private"
                );
            }
            let mut bytes = Vec::new();
            file.take(MAX_HANDOFF_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| anyhow!("Could not read child-process tracing handoff"))?;
            anyhow::ensure!(
                bytes.len() as u64 <= MAX_HANDOFF_BYTES,
                "Child-process tracing handoff is too large"
            );
            Ok(bytes)
        })();
        fs::remove_file(&path)
            .map_err(|_| anyhow!("Could not remove child-process tracing handoff"))?;
        let bytes = result?;
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| anyhow!("Invalid child-process tracing handoff"))
    }
}

#[cfg(test)]
#[path = "child_process_tests.rs"]
mod tests;
