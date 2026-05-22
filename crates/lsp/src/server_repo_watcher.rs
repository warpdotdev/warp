use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use async_channel::Sender;
use lsp_types::FileChangeType;
use repo_metadata::repository::{RepositorySubscriber, SubscriberId};
use repo_metadata::{DirectoryWatcher, Repository, RepositoryUpdate};
use warp_util::standardized_path::StandardizedPath;
use warpui::{ModelContext, SingletonEntity, WeakModelHandle};

use crate::model::LspServerModel;
use crate::types::WatchedFileChangeEvent;
use crate::LspServerConfig;

enum RepoWatchState {
    NotWatching,
    Starting {
        repository: WeakModelHandle<Repository>,
        subscriber_id: SubscriberId,
    },
    Watching {
        repository: WeakModelHandle<Repository>,
        subscriber_id: SubscriberId,
    },
}

pub(crate) struct LspRepoWatcher {
    state: RepoWatchState,
}

impl LspRepoWatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn teardown(&mut self, ctx: &mut ModelContext<LspServerModel>) {
        let old_state = std::mem::replace(&mut self.state, RepoWatchState::NotWatching);

        match old_state {
            RepoWatchState::NotWatching => {}
            RepoWatchState::Starting {
                repository,
                subscriber_id,
            }
            | RepoWatchState::Watching {
                repository,
                subscriber_id,
            } => {
                if let Some(repository) = repository.upgrade(ctx) {
                    repository.update(ctx, |repo, ctx| {
                        repo.stop_watching(subscriber_id, ctx);
                    });
                }
            }
        }
    }

    pub fn ensure(&mut self, config: &LspServerConfig, ctx: &mut ModelContext<LspServerModel>) {
        if !matches!(self.state, RepoWatchState::NotWatching) {
            return;
        }

        let (tx, rx) = async_channel::unbounded::<RepositoryUpdate>();

        let workspace_root: PathBuf = config.initial_workspace().to_path_buf();
        let workspace_root_for_log = workspace_root.display().to_string();

        let repository = DirectoryWatcher::handle(ctx).update(ctx, |watcher, ctx| {
            let Ok(standardized) = StandardizedPath::from_local_canonicalized(&workspace_root)
            else {
                return None;
            };

            watcher.add_directory(standardized, ctx).ok()
        });

        let Some(repository) = repository else {
            log::warn!(
                "Unable to find or watch directory for LSP workspace: {workspace_root_for_log}"
            );
            return;
        };

        let start = repository.update(ctx, |repo, ctx| {
            repo.start_watching(Box::new(LspRepoSubscriber { tx }), ctx)
        });

        let repository_for_spawn = repository.downgrade();
        let subscriber_id = start.subscriber_id;

        self.state = RepoWatchState::Starting {
            repository: repository_for_spawn.clone(),
            subscriber_id,
        };

        ctx.spawn(start.registration_future, move |me, res, ctx| match res {
            Ok(()) => {
                if matches!(
                    me.repo_watcher_mut().state,
                    RepoWatchState::Starting { subscriber_id: s, .. } if s == subscriber_id
                ) {
                    me.repo_watcher_mut().state = RepoWatchState::Watching {
                        repository: repository_for_spawn.clone(),
                        subscriber_id,
                    };
                }
            }
            Err(err) => {
                if matches!(
                    me.repo_watcher_mut().state,
                    RepoWatchState::Starting { subscriber_id: s, .. } if s == subscriber_id
                ) {
                    me.repo_watcher_mut().state = RepoWatchState::NotWatching;
                }

                log::warn!("Unable to start LSP server: {err}");
                if let Some(repository) = repository_for_spawn.upgrade(ctx) {
                    repository.update(ctx, |repo, ctx| {
                        repo.stop_watching(subscriber_id, ctx);
                    });
                }
            }
        });

        ctx.spawn_stream_local(
            rx,
            |me, update, _ctx| {
                let mut events = Vec::new();

                events.extend(update.added.into_iter().map(|file| WatchedFileChangeEvent {
                    path: file.path,
                    typ: FileChangeType::CREATED,
                }));

                events.extend(
                    update
                        .modified
                        .into_iter()
                        .map(|file| WatchedFileChangeEvent {
                            path: file.path,
                            typ: FileChangeType::CHANGED,
                        }),
                );

                events.extend(
                    update
                        .deleted
                        .into_iter()
                        .map(|file| WatchedFileChangeEvent {
                            path: file.path,
                            typ: FileChangeType::DELETED,
                        }),
                );

                for (to, from) in update.moved {
                    events.push(WatchedFileChangeEvent {
                        path: to.path,
                        typ: FileChangeType::CREATED,
                    });

                    events.push(WatchedFileChangeEvent {
                        path: from.path,
                        typ: FileChangeType::DELETED,
                    });
                }

                if events.is_empty() {
                    return;
                }

                if let Err(e) = me.did_change_watched_files(events) {
                    log::warn!("Failed to send didChangeWatchedFiles notification: {e}");
                }
            },
            |_, _| {},
        );
    }
}

impl Default for LspRepoWatcher {
    fn default() -> Self {
        Self {
            state: RepoWatchState::NotWatching,
        }
    }
}

struct LspRepoSubscriber {
    tx: Sender<RepositoryUpdate>,
}

impl RepositorySubscriber for LspRepoSubscriber {
    fn on_scan(
        &mut self,
        _repository: &Repository,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        Box::pin(async {})
    }

    fn on_files_updated(
        &mut self,
        _repository: &Repository,
        update: &RepositoryUpdate,
        _ctx: &mut ModelContext<Repository>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let tx = self.tx.clone();
        let update = update.clone();
        Box::pin(async move {
            let _ = tx.send(update).await;
        })
    }
}
