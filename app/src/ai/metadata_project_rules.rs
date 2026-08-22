use ai::project_context::model::ProjectRuleContents;
use futures::future::{BoxFuture, FutureExt as _};
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::AppContext;

use super::remote_context_files::read_remote_text_file_contents;

/// Project rule files are hand-written markdown; 1 MiB matches the cap used for other small
/// project-context text files (e.g. `REMOTE_CONTEXT_MAX_FILE_BYTES`) while still rejecting the
/// pathological on-disk sizes that motivated APP-4801.
const MAX_PROJECT_RULE_FILE_BYTES: u64 = 1024 * 1024;

pub(crate) fn read_project_rule_contents(
    rule_paths: Vec<LocalOrRemotePath>,
    ctx: &AppContext,
) -> BoxFuture<'static, anyhow::Result<ProjectRuleContents>> {
    match rule_paths.first() {
        None => futures::future::ready(Ok(Vec::new())).boxed(),
        Some(LocalOrRemotePath::Local(_)) => read_local_rule_contents(rule_paths).boxed(),
        Some(LocalOrRemotePath::Remote(_)) => {
            read_remote_text_file_contents(rule_paths, None, None, ctx)
        }
    }
}

async fn read_local_rule_contents(
    rule_paths: Vec<LocalOrRemotePath>,
) -> anyhow::Result<ProjectRuleContents> {
    let mut contents = Vec::new();
    for path in rule_paths {
        let Some(local_path) = path.to_local_path().map(std::path::Path::to_path_buf) else {
            anyhow::bail!("Project rule paths mixed local and remote locations");
        };
        match warp_util::file::read_to_string_capped(&local_path, MAX_PROJECT_RULE_FILE_BYTES).await
        {
            Ok(content) => contents.push((path, content)),
            Err(error) => log::debug!(
                "Failed to read project rule file {}: {error}",
                local_path.display()
            ),
        }
    }
    Ok(contents)
}

#[cfg(test)]
#[path = "metadata_project_rules_tests.rs"]
mod tests;
