use std::path::Path;

use anyhow::Context as _;
use futures::TryStreamExt as _;
use tokio_util::io::StreamReader;

pub(crate) async fn write_response_body_to_path(
    response: http_client::Response,
    path: &Path,
) -> anyhow::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        tokio::fs::create_dir_all(parent).await.with_context(|| {
            format!("Failed to create download directory '{}'", parent.display())
        })?;
    }

    let mut file = tokio::fs::File::create(path)
        .await
        .with_context(|| format!("Failed to create download file '{}'", path.display()))?;
    let mut response_stream =
        StreamReader::new(response.bytes_stream().map_err(std::io::Error::other));
    tokio::io::copy(&mut response_stream, &mut file)
        .await
        .with_context(|| format!("Failed to write download file '{}'", path.display()))?;
    file.sync_data()
        .await
        .with_context(|| format!("Failed to sync download file '{}'", path.display()))?;

    Ok(())
}
