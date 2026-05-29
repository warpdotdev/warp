use std::collections::HashMap;

use ai::project_context::model::{ProjectContextModel, ProjectRule};
use futures::future::{BoxFuture, FutureExt as _};
use remote_server::proto::{
    file_context_proto, FileContextProto, ReadFileContextFile, ReadFileContextRequest,
};
use repo_metadata::{
    RepoMetadataEvent, RepoMetadataModel, RepositoryIdentifier, StandingQueryContent,
};
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::remote_server::manager::RemoteServerManager;

pub(crate) struct MetadataProjectRulesModel {
    refresh_generations: HashMap<RepositoryIdentifier, u64>,
    next_refresh_generation: u64,
}

type ProjectRuleContentsFuture =
    BoxFuture<'static, anyhow::Result<Vec<(LocalOrRemotePath, String)>>>;

impl MetadataProjectRulesModel {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        ctx.subscribe_to_model(&RepoMetadataModel::handle(ctx), |me, event, ctx| {
            me.handle_repo_metadata_event(event, ctx);
        });

        let repo_ids = RepoMetadataModel::as_ref(ctx)
            .remote_repository_ids(ctx)
            .cloned()
            .map(RepositoryIdentifier::Remote)
            .collect::<Vec<_>>();
        let mut model = Self {
            refresh_generations: HashMap::new(),
            next_refresh_generation: 0,
        };
        for repo_id in repo_ids {
            model.refresh_project_rules_for_repo(&repo_id, ctx);
        }
        model
    }

    fn handle_repo_metadata_event(
        &mut self,
        event: &RepoMetadataEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            RepoMetadataEvent::RepositoryUpdated {
                id: repo_id @ RepositoryIdentifier::Remote(_),
            } => self.refresh_project_rules_for_repo(repo_id, ctx),
            RepoMetadataEvent::StandingQueryResultsUpdated {
                id: repo_id @ RepositoryIdentifier::Remote(_),
                delta,
            } => {
                if delta.project_rules_changed() {
                    self.refresh_project_rules_for_repo(repo_id, ctx);
                }
            }
            RepoMetadataEvent::RepositoryRemoved {
                id: repo_id @ RepositoryIdentifier::Remote(_),
            } => self.clear_project_rules_for_removed_repository(repo_id, ctx),
            RepoMetadataEvent::RepositoryUpdated {
                id: RepositoryIdentifier::Local(_),
            }
            | RepoMetadataEvent::RepositoryRemoved {
                id: RepositoryIdentifier::Local(_),
            }
            | RepoMetadataEvent::StandingQueryResultsUpdated {
                id: RepositoryIdentifier::Local(_),
                ..
            }
            | RepoMetadataEvent::FileTreeUpdated { .. }
            | RepoMetadataEvent::FileTreeEntryUpdated { .. }
            | RepoMetadataEvent::UpdatingRepositoryFailed { .. }
            | RepoMetadataEvent::IncrementalUpdateReady { .. } => {}
        }
    }

    fn refresh_project_rules_for_repo(
        &mut self,
        repo_id: &RepositoryIdentifier,
        ctx: &mut ModelContext<Self>,
    ) {
        let RepositoryIdentifier::Remote(remote_root) = repo_id else {
            return;
        };
        let refresh_generation = self.advance_refresh_generation(repo_id);
        let rule_paths = remote_project_rule_paths(
            repo_id,
            RepoMetadataModel::as_ref(ctx)
                .standing_query_results(repo_id, ctx)
                .into_iter()
                .flat_map(|results| results.project_rules()),
        );
        if rule_paths.is_empty() {
            self.apply_project_rules_if_current(
                repo_id,
                refresh_generation,
                remote_root.clone(),
                Vec::new(),
                ctx,
            );
            return;
        }
        let Some(read_rule_contents) = read_remote_project_rule_contents(rule_paths, ctx) else {
            return;
        };
        let repo_id_for_result = repo_id.clone();
        let remote_root_for_result = remote_root.clone();
        ctx.spawn(
            async move {
                let rule_contents = read_rule_contents.await?;
                Ok::<Vec<ProjectRule>, anyhow::Error>(build_project_rules(rule_contents))
            },
            move |me, hydrated_rules, ctx| match hydrated_rules {
                Ok(rules) => me.apply_project_rules_if_current(
                    &repo_id_for_result,
                    refresh_generation,
                    remote_root_for_result,
                    rules,
                    ctx,
                ),
                Err(error) => log::warn!("Failed to read remote project rules: {error}"),
            },
        );
    }

    fn clear_project_rules_for_removed_repository(
        &mut self,
        repo_id: &RepositoryIdentifier,
        ctx: &mut ModelContext<Self>,
    ) {
        self.refresh_generations.remove(repo_id);
        let RepositoryIdentifier::Remote(remote_root) = repo_id else {
            return;
        };
        ProjectContextModel::handle(ctx).update(ctx, |model, ctx| {
            model.clear_remote_project_rules_for_removed_metadata_root(remote_root.clone(), ctx);
        });
    }

    fn advance_refresh_generation(&mut self, repo_id: &RepositoryIdentifier) -> u64 {
        self.next_refresh_generation += 1;
        self.refresh_generations
            .insert(repo_id.clone(), self.next_refresh_generation);
        self.next_refresh_generation
    }

    fn apply_project_rules_if_current(
        &mut self,
        repo_id: &RepositoryIdentifier,
        refresh_generation: u64,
        remote_root: RemotePath,
        rules: Vec<ProjectRule>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.refresh_generations.get(repo_id) != Some(&refresh_generation) {
            return;
        }
        ProjectContextModel::handle(ctx).update(ctx, |model, ctx| {
            model.replace_remote_project_rules_from_metadata(remote_root, rules, ctx);
        });
    }
}

impl Entity for MetadataProjectRulesModel {
    type Event = ();
}

impl SingletonEntity for MetadataProjectRulesModel {}

fn remote_project_rule_paths<'a>(
    repo_id: &RepositoryIdentifier,
    contents: impl IntoIterator<Item = &'a StandingQueryContent>,
) -> Vec<LocalOrRemotePath> {
    let RepositoryIdentifier::Remote(remote_root) = repo_id else {
        return Vec::new();
    };
    contents
        .into_iter()
        .filter(|content| !content.is_directory)
        .map(|content| {
            LocalOrRemotePath::Remote(RemotePath::new(
                remote_root.host_id.clone(),
                content.path.clone(),
            ))
        })
        .collect()
}

fn read_remote_project_rule_contents(
    rule_paths: Vec<LocalOrRemotePath>,
    ctx: &AppContext,
) -> Option<ProjectRuleContentsFuture> {
    let LocalOrRemotePath::Remote(remote) = rule_paths.first()? else {
        return None;
    };
    let handle = RemoteServerManager::as_ref(ctx).host_request_handle(&remote.host_id);
    Some(
        async move {
            let response = handle
                .read_file_context(remote_rule_read_request(&rule_paths))
                .await?;
            Ok(match_remote_project_rule_contents(
                rule_paths,
                response.file_contexts,
            ))
        }
        .boxed(),
    )
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

fn match_remote_project_rule_contents(
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

fn build_project_rules(rule_contents: Vec<(LocalOrRemotePath, String)>) -> Vec<ProjectRule> {
    rule_contents
        .into_iter()
        .map(|(path, content)| ProjectRule { path, content })
        .collect()
}

#[cfg(test)]
#[path = "metadata_project_rules_tests.rs"]
mod tests;
