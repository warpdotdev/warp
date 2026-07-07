use std::collections::HashMap;

use futures::future::{BoxFuture, FutureExt as _};
use remote_server::proto::{
    file_context_proto, FileContextProto, ReadFileContextFile, ReadFileContextRequest,
};
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, SingletonEntity};

use crate::remote_server::manager::RemoteServerManager;

#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) const REMOTE_CONTEXT_MAX_FILE_BYTES: u32 = 1024 * 1024;
#[cfg_attr(target_family = "wasm", allow(dead_code))]
pub(crate) const REMOTE_CONTEXT_MAX_BATCH_BYTES: u32 = 5 * 1024 * 1024;

/// Reads text contents for exact paths on one remote host.
///
/// Responses may be reordered or omit unreadable files, and their file names do not include the
/// host ID. Pairing by path preserves the original host-qualified identities and request order.
pub(crate) fn read_remote_text_file_contents(
    paths: Vec<LocalOrRemotePath>,
    max_file_bytes: Option<u32>,
    max_batch_bytes: Option<u32>,
    ctx: &AppContext,
) -> BoxFuture<'static, anyhow::Result<Vec<(LocalOrRemotePath, String)>>> {
    let host_id = match remote_context_host_id(&paths) {
        Ok(Some(host_id)) => host_id,
        Ok(None) => return futures::future::ready(Ok(Vec::new())).boxed(),
        Err(error) => return futures::future::ready(Err(error)).boxed(),
    };

    let request = remote_text_file_read_request(&paths, max_file_bytes, max_batch_bytes);
    let handle = RemoteServerManager::as_ref(ctx).host_request_handle(&host_id);
    async move {
        let response = handle.read_file_context(request).await?;
        Ok(pair_remote_text_file_contents(
            paths,
            response.file_contexts,
        ))
    }
    .boxed()
}

fn remote_context_host_id(paths: &[LocalOrRemotePath]) -> anyhow::Result<Option<HostId>> {
    let Some(first_path) = paths.first() else {
        return Ok(None);
    };
    let Some(first_remote) = first_path.as_remote() else {
        anyhow::bail!("Expected remote context paths");
    };
    if paths.iter().any(|path| {
        path.as_remote()
            .is_none_or(|remote| remote.host_id != first_remote.host_id)
    }) {
        anyhow::bail!("Remote context paths span multiple locations");
    }
    Ok(Some(first_remote.host_id.clone()))
}
fn remote_text_file_read_request(
    paths: &[LocalOrRemotePath],
    max_file_bytes: Option<u32>,
    max_batch_bytes: Option<u32>,
) -> ReadFileContextRequest {
    ReadFileContextRequest {
        files: paths
            .iter()
            .filter_map(|path| {
                let remote = path.as_remote()?;
                Some(ReadFileContextFile {
                    path: remote.path.as_str().to_string(),
                    line_ranges: Vec::new(),
                })
            })
            .collect(),
        max_file_bytes,
        max_batch_bytes,
    }
}

fn pair_remote_text_file_contents(
    paths: Vec<LocalOrRemotePath>,
    file_contexts: Vec<FileContextProto>,
) -> Vec<(LocalOrRemotePath, String)> {
    let content_by_path = file_contexts
        .into_iter()
        .filter_map(|file_context| {
            let file_context_proto::Content::TextContent(content) = file_context.content? else {
                return None;
            };
            Some((file_context.file_name, content))
        })
        .collect::<HashMap<_, _>>();
    paths
        .into_iter()
        .filter_map(|path| {
            let content = content_by_path
                .get(path.as_remote()?.path.as_str())?
                .clone();
            Some((path, content))
        })
        .collect()
}

#[cfg(test)]
#[path = "remote_context_files_tests.rs"]
mod tests;
