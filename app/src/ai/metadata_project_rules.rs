use std::collections::HashMap;

use ai::project_context::model::ProjectRuleContents;
use futures::future::{BoxFuture, FutureExt as _};
use remote_server::proto::{
    file_context_proto, FileContextProto, ReadFileContextFile, ReadFileContextRequest,
};
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, SingletonEntity};

use crate::remote_server::manager::RemoteServerManager;

pub(crate) fn read_project_rule_contents(
    rule_paths: Vec<LocalOrRemotePath>,
    ctx: &AppContext,
) -> BoxFuture<'static, anyhow::Result<ProjectRuleContents>> {
    match rule_paths.first() {
        None => futures::future::ready(Ok(Vec::new())).boxed(),
        Some(LocalOrRemotePath::Local(_)) => async move {
            let mut contents = Vec::new();
            for path in rule_paths {
                let Some(local_path) = path.to_local_path().map(std::path::Path::to_path_buf)
                else {
                    anyhow::bail!("Project rule paths mixed local and remote locations");
                };
                match async_fs::read_to_string(&local_path).await {
                    Ok(content) => contents.push((path, content)),
                    Err(error) => log::debug!(
                        "Failed to read project rule file {}: {error}",
                        local_path.display()
                    ),
                }
            }
            Ok(contents)
        }
        .boxed(),
        Some(LocalOrRemotePath::Remote(remote)) => {
            let host_id = remote.host_id.clone();
            let handle = RemoteServerManager::as_ref(ctx).host_request_handle(&host_id);
            async move {
                if rule_paths.iter().any(|path| {
                    !matches!(
                        path,
                        LocalOrRemotePath::Remote(candidate) if candidate.host_id == host_id
                    )
                }) {
                    anyhow::bail!("Project rule paths span multiple locations");
                }
                let response = handle
                    .read_file_context(remote_rule_read_request(&rule_paths))
                    .await?;
                Ok(pair_remote_rule_paths_with_contents(
                    rule_paths,
                    response.file_contexts,
                ))
            }
            .boxed()
        }
    }
}

fn remote_rule_read_request(rule_paths: &[LocalOrRemotePath]) -> ReadFileContextRequest {
    ReadFileContextRequest {
        files: rule_paths
            .iter()
            .filter_map(|path| match path {
                LocalOrRemotePath::Remote(remote) => Some(ReadFileContextFile {
                    path: remote.path.as_str().to_string(),
                    line_ranges: Vec::new(),
                }),
                LocalOrRemotePath::Local(_) => None,
            })
            .collect(),
        max_file_bytes: None,
        max_batch_bytes: None,
    }
}

/// Pairs remote read responses with the original host-qualified paths.
///
/// Responses may be reordered or omit unreadable files, and their file names do not include the
/// host ID. Matching by path preserves the correct host identity without relying on response order.
fn pair_remote_rule_paths_with_contents(
    rule_paths: Vec<LocalOrRemotePath>,
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
    rule_paths
        .into_iter()
        .filter_map(|path| {
            let LocalOrRemotePath::Remote(remote) = &path else {
                return None;
            };
            let content = content_by_path.get(remote.path.as_str())?.clone();
            Some((path, content))
        })
        .collect()
}

#[cfg(test)]
#[path = "metadata_project_rules_tests.rs"]
mod tests;
