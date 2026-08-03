use ai::project_context::model::ProjectRuleContents;
use futures::future::{BoxFuture, FutureExt as _};
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::AppContext;

use super::remote_context_files::read_remote_text_file_contents;

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
        Some(LocalOrRemotePath::Remote(_)) => {
            read_remote_text_file_contents(rule_paths, None, None, ctx)
        }
    }
}
