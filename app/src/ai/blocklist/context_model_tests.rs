//! Unit tests for [`BlocklistAIContextModel`].
//!
//! These tests use [`BlocklistAIContextModel::new_for_test`] and a small conversation-selection
//! fake to avoid unrelated subscriptions while exercising context behavior.

use std::sync::Arc;

use parking_lot::FairMutex;
#[cfg(feature = "local_fs")]
use repo_metadata::DirectoryWatcher;
#[cfg(feature = "local_fs")]
use warp_util::standardized_path::StandardizedPath;
use warpui::r#async::executor::Background;
use warpui::{App, EntityId, ModelHandle, SingletonEntity};

use super::{BlocklistAIContextModel, PendingAttachment, PendingFile};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIAgentContext, ImageContext};
use crate::ai::blocklist::agent_view::{AgentViewEntryOrigin, EnterAgentViewError};
use crate::ai::blocklist::conversation_selection::{
    ConversationSelection, ConversationSelectionEvent,
};
use crate::ai::blocklist::{
    BlocklistAIHistoryEvent, BlocklistAIHistoryModel, QueuedQuery, QueuedQueryModel,
    QueuedQueryOrigin,
};
#[cfg(feature = "local_fs")]
use crate::code_review::git_repo_model::GitRepoStatusModel;
#[cfg(feature = "local_fs")]
use crate::code_review::github_repo_model::GitHubRepoModel;
use crate::terminal::color::{self, Colors};
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::test_utils::block_size;
use crate::terminal::model::{BlockId, TerminalModel};
use crate::test_util::settings::initialize_history_persistence_for_tests;
use crate::util::git::{PrInfo, RepositoryInfo};

impl BlocklistAIContextModel {
    pub(crate) fn append_pending_attachments_for_test(
        &mut self,
        attachments: Vec<PendingAttachment>,
    ) {
        self.pending_attachments.extend(attachments);
    }

    pub(crate) fn insert_pending_block_id_for_test(&mut self, block_id: BlockId) {
        self.pending_context_block_ids.insert(block_id);
    }

    pub(crate) fn set_pending_selected_text_for_test(&mut self, text: Option<String>) {
        self.pending_context_selected_text = text;
    }
}

struct TestConversationSelection {
    terminal_surface_id: EntityId,
    selected_conversation_id: Option<AIConversationId>,
}

impl TestConversationSelection {
    fn new(
        terminal_surface_id: EntityId,
        _: &mut warpui::ModelContext<Box<dyn ConversationSelection>>,
    ) -> Self {
        Self {
            terminal_surface_id,
            selected_conversation_id: None,
        }
    }
}

impl ConversationSelection for TestConversationSelection {
    fn selected_conversation_id(&self, _: &warpui::AppContext) -> Option<AIConversationId> {
        self.selected_conversation_id
    }

    fn is_conversation_active(&self, _: &warpui::AppContext) -> bool {
        self.selected_conversation_id.is_some()
    }

    fn is_conversation_fullscreen(&self, _: &warpui::AppContext) -> bool {
        self.selected_conversation_id.is_some()
    }

    fn select_existing_conversation(
        &mut self,
        conversation_id: AIConversationId,
        _: AgentViewEntryOrigin,
        ctx: &mut warpui::ModelContext<Box<dyn ConversationSelection>>,
    ) {
        if self.selected_conversation_id != Some(conversation_id) {
            self.selected_conversation_id = Some(conversation_id);
            ctx.emit(ConversationSelectionEvent::Changed);
        }
    }

    fn select_new_conversation(
        &mut self,
        _: AgentViewEntryOrigin,
        ctx: &mut warpui::ModelContext<Box<dyn ConversationSelection>>,
    ) {
        if self.selected_conversation_id.take().is_some() {
            ctx.emit(ConversationSelectionEvent::Changed);
        }
    }

    fn try_start_new_conversation(
        &mut self,
        _: AgentViewEntryOrigin,
        ctx: &mut warpui::ModelContext<Box<dyn ConversationSelection>>,
    ) -> Result<AIConversationId, EnterAgentViewError> {
        let conversation_id = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.start_new_conversation(self.terminal_surface_id, false, false, false, ctx)
        });
        self.select_existing_conversation(conversation_id, AgentViewEntryOrigin::Cli, ctx);
        Ok(conversation_id)
    }

    fn pending_query_autoexecute_override(
        &self,
        app: &warpui::AppContext,
    ) -> crate::ai::agent::conversation::AIConversationAutoexecuteMode {
        self.selected_conversation_id
            .as_ref()
            .and_then(|conversation_id| {
                BlocklistAIHistoryModel::as_ref(app).conversation(conversation_id)
            })
            .map(|conversation| conversation.autoexecute_override())
            .unwrap_or_default()
    }

    fn toggle_pending_query_autoexecute(
        &mut self,
        ctx: &mut warpui::ModelContext<Box<dyn ConversationSelection>>,
    ) {
        if let Some(conversation_id) = self.selected_conversation_id {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
                history.toggle_autoexecute_override(
                    &conversation_id,
                    self.terminal_surface_id,
                    ctx,
                );
            });
        }
    }

    fn handle_history_event(
        &mut self,
        _: &BlocklistAIHistoryEvent,
        _: &mut warpui::ModelContext<Box<dyn ConversationSelection>>,
    ) {
    }
}

#[cfg(feature = "local_fs")]
#[test]
fn repository_context_reads_github_repo_model() {
    App::test((), |mut app| async move {
        let context_model = build_test_context_model(&mut app);
        let temp_dir = tempfile::TempDir::new().unwrap();
        let watcher_handle = app.add_singleton_model(DirectoryWatcher::new_for_testing);
        let repository = watcher_handle.update(&mut app, |watcher, ctx| {
            watcher
                .add_directory(
                    StandardizedPath::from_local_canonicalized(temp_dir.path()).unwrap(),
                    ctx,
                )
                .unwrap()
        });
        let git_status =
            app.add_model(move |ctx| GitRepoStatusModel::new_local_for_test(repository, None, ctx));
        let github_repo_model =
            app.add_model(move |ctx| GitHubRepoModel::new_local_for_test(git_status, ctx));

        github_repo_model.update(&mut app, |model, ctx| {
            model.set_repository_info_for_test(
                Some(RepositoryInfo {
                    name: "warp-internal".to_owned(),
                    owner: Some("warpdotdev".to_owned()),
                }),
                ctx,
            );
        });

        context_model.update(&mut app, |model, _| {
            model.set_github_repo_model(Some(github_repo_model.downgrade()));
        });

        context_model.read(&app, |model, ctx| {
            assert_eq!(
                model.repository_context(ctx),
                Some(AIAgentContext::Repository {
                    name: "warp-internal".to_owned(),
                    owner: Some("warpdotdev".to_owned()),
                })
            );
        });

        context_model.update(&mut app, |model, _| {
            model.set_github_repo_model(None);
        });

        context_model.read(&app, |model, ctx| {
            assert_eq!(model.repository_context(ctx), None);
        });
    });
}

/// Builds a [`BlocklistAIContextModel`] with stub dependencies. None of the dependencies are
/// exercised by the methods under test; they only need to satisfy the struct's field types.
fn build_test_context_model(app: &mut App) -> ModelHandle<BlocklistAIContextModel> {
    initialize_history_persistence_for_tests(app);
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::new_for_test(
        block_size(),
        color::List::from(&Colors::default()),
        ChannelEventListener::new_for_test(),
        Arc::new(Background::default()),
        false, /* should_show_bootstrap_block */
        None,  /* restored_blocks */
        false, /* honor_ps1 */
        false, /* is_inverted */
        None,  /* session_startup_path */
    )));
    let terminal_view_id = EntityId::new();

    let conversation_selection = app.add_model(|ctx| {
        Box::new(TestConversationSelection::new(terminal_view_id, ctx))
            as Box<dyn ConversationSelection>
    });

    app.add_model(|_| {
        BlocklistAIContextModel::new_for_test(
            terminal_model,
            terminal_view_id,
            conversation_selection,
        )
    })
}

/// Builds context state for a TUI conversation surface.
fn build_tui_context_model(app: &mut App) -> (ModelHandle<BlocklistAIContextModel>, EntityId) {
    initialize_history_persistence_for_tests(app);
    app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::new_for_test(
        block_size(),
        color::List::from(&Colors::default()),
        ChannelEventListener::new_for_test(),
        Arc::new(Background::default()),
        false,
        None,
        false,
        false,
        None,
    )));
    let terminal_surface_id = EntityId::new();
    let conversation_selection = app.add_model(|ctx| {
        Box::new(TestConversationSelection::new(terminal_surface_id, ctx))
            as Box<dyn ConversationSelection>
    });
    let model = app.add_model(|_| {
        BlocklistAIContextModel::new_for_test(
            terminal_model,
            terminal_surface_id,
            conversation_selection,
        )
    });
    (model, terminal_surface_id)
}

#[test]
fn tui_context_tracks_selected_conversation() {
    App::test((), |mut app| async move {
        let (model, _) = build_tui_context_model(&mut app);
        let conversation_id = AIConversationId::new();

        model.update(&mut app, |model, ctx| {
            model.set_pending_query_state_for_existing_conversation(
                conversation_id,
                AgentViewEntryOrigin::Cli,
                ctx,
            );
        });
        model.read(&app, |model, ctx| {
            assert_eq!(model.selected_conversation_id(ctx), Some(conversation_id));
        });

        model.update(&mut app, |model, ctx| {
            model.set_pending_query_state_for_new_conversation(AgentViewEntryOrigin::Cli, ctx);
        });
        model.read(&app, |model, ctx| {
            assert_eq!(model.selected_conversation_id(ctx), None);
        });
    });
}

#[test]
fn tui_new_conversation_is_selected_and_terminal_surface_scoped() {
    App::test((), |mut app| async move {
        let (model, terminal_surface_id) = build_tui_context_model(&mut app);
        let history = BlocklistAIHistoryModel::handle(&app);

        let conversation_id = model
            .update(&mut app, |model, ctx| {
                model.try_start_new_conversation(AgentViewEntryOrigin::Cli, ctx)
            })
            .expect("TUI conversation creation should succeed");

        model.read(&app, |model, ctx| {
            assert_eq!(model.selected_conversation_id(ctx), Some(conversation_id));
        });
        history.read(&app, |history, _| {
            assert_eq!(
                history
                    .all_live_conversations_for_terminal_surface(terminal_surface_id)
                    .map(|conversation| conversation.id())
                    .collect::<Vec<_>>(),
                vec![conversation_id]
            );
        });
    });
}

fn make_image_attachment(file_name: &str) -> PendingAttachment {
    PendingAttachment::Image(ImageContext {
        data: String::new(),
        mime_type: "image/png".to_owned(),
        file_name: file_name.to_owned(),
        is_figma: false,
    })
}

fn make_file_attachment(file_name: &str) -> PendingAttachment {
    PendingAttachment::File(PendingFile {
        file_name: file_name.to_owned(),
        file_path: file_name.into(),
        mime_type: "text/plain".to_owned(),
    })
}

#[test]
fn has_locking_attachment_is_false_for_default_state() {
    App::test((), |mut app| async move {
        let model = build_test_context_model(&mut app);

        model.read(&app, |m, _| {
            assert!(!m.has_locking_attachment());
        });
    });
}

#[test]
fn has_locking_attachment_is_false_with_only_pending_block_id() {
    // A pending block alone is *not* a locking attachment: only image/file attachments
    // should force the input into AI mode (skipping NLD).
    App::test((), |mut app| async move {
        let model = build_test_context_model(&mut app);

        model.update(&mut app, |m, _| {
            m.insert_pending_block_id_for_test(BlockId::new());
        });

        model.read(&app, |m, _| assert!(!m.has_locking_attachment()));
    });
}

#[test]
fn repository_context_from_repository_info_converts_to_agent_context() {
    let repository_info = RepositoryInfo {
        name: "warp-internal".to_owned(),
        owner: Some("warpdotdev".to_owned()),
    };

    assert_eq!(
        BlocklistAIContextModel::repository_context_from_repository_info(&repository_info),
        AIAgentContext::Repository {
            name: "warp-internal".to_owned(),
            owner: Some("warpdotdev".to_owned()),
        }
    );
}

#[test]
fn pull_request_context_from_pr_info_excludes_url() {
    let pr_info = PrInfo {
        number: 123,
        url: "https://github.com/warpdotdev/warp/pull/123".to_owned(),
        state: "OPEN".to_owned(),
        draft: true,
        base_branch: "main".to_owned(),
    };

    assert_eq!(
        BlocklistAIContextModel::pull_request_context_from_pr_info(&pr_info),
        Some(AIAgentContext::PullRequest {
            number: 123,
            state: "OPEN".to_owned(),
            draft: true,
            base_branch: "main".to_owned(),
        })
    );
}

#[test]
fn pull_request_context_from_pr_info_rejects_numbers_that_do_not_fit_agent_context() {
    let pr_info = PrInfo {
        number: i32::MAX as u64 + 1,
        url: "https://github.com/warpdotdev/warp/pull/2147483648".to_owned(),
        state: "OPEN".to_owned(),
        draft: false,
        base_branch: "main".to_owned(),
    };

    assert_eq!(
        BlocklistAIContextModel::pull_request_context_from_pr_info(&pr_info),
        None
    );
}

#[cfg(feature = "local_fs")]
#[test]
fn pull_request_context_reads_github_repo_model() {
    App::test((), |mut app| async move {
        let context_model = build_test_context_model(&mut app);
        let temp_dir = tempfile::TempDir::new().unwrap();
        let watcher_handle = app.add_singleton_model(DirectoryWatcher::new_for_testing);
        let repository = watcher_handle.update(&mut app, |watcher, ctx| {
            watcher
                .add_directory(
                    StandardizedPath::from_local_canonicalized(temp_dir.path()).unwrap(),
                    ctx,
                )
                .unwrap()
        });
        let git_status =
            app.add_model(move |ctx| GitRepoStatusModel::new_local_for_test(repository, None, ctx));
        let github_repo_model =
            app.add_model(move |ctx| GitHubRepoModel::new_local_for_test(git_status, ctx));

        github_repo_model.update(&mut app, |model, ctx| {
            model.set_pr_info_for_test(
                Some(PrInfo {
                    number: 123,
                    url: "https://github.com/warpdotdev/warp/pull/123".to_owned(),
                    state: "OPEN".to_owned(),
                    draft: false,
                    base_branch: "main".to_owned(),
                }),
                ctx,
            );
        });

        context_model.update(&mut app, |model, _| {
            model.set_github_repo_model(Some(github_repo_model.downgrade()));
        });

        context_model.read(&app, |model, ctx| {
            assert_eq!(
                model.pull_request_context(ctx),
                Some(AIAgentContext::PullRequest {
                    number: 123,
                    state: "OPEN".to_owned(),
                    draft: false,
                    base_branch: "main".to_owned(),
                })
            );
        });

        context_model.update(&mut app, |model, _| {
            model.set_github_repo_model(None);
        });

        context_model.read(&app, |model, ctx| {
            assert_eq!(model.pull_request_context(ctx), None);
        });
    });
}

#[test]
fn has_locking_attachment_is_false_with_only_pending_selected_text() {
    // Selected text alone is *not* a locking attachment: the user could be selecting shell
    // command text (e.g. to copy a previously-run command), and forcing the input into AI
    // mode in that case would be wrong. Only image or file attachments should force the lock.
    App::test((), |mut app| async move {
        let model = build_test_context_model(&mut app);

        model.update(&mut app, |m, _| {
            m.set_pending_selected_text_for_test(Some("hello".to_owned()));
        });

        model.read(&app, |m, _| assert!(!m.has_locking_attachment()));
    });
}

#[test]
fn has_locking_attachment_is_true_with_pending_image_attachment() {
    App::test((), |mut app| async move {
        let model = build_test_context_model(&mut app);

        model.update(&mut app, |m, _| {
            m.append_pending_attachments_for_test(vec![make_image_attachment("a.png")]);
        });

        model.read(&app, |m, _| assert!(m.has_locking_attachment()));
    });
}

#[test]
fn has_locking_attachment_is_true_with_only_file_attachments() {
    // File attachments are locking attachments — the user has explicitly attached a file as
    // context, which is unambiguously a signal that the next query is intended for the agent.
    App::test((), |mut app| async move {
        let model = build_test_context_model(&mut app);

        model.update(&mut app, |m, _| {
            m.append_pending_attachments_for_test(vec![
                make_file_attachment("notes.txt"),
                make_file_attachment("readme.md"),
            ]);
        });

        model.read(&app, |m, _| assert!(m.has_locking_attachment()));
    });
}

#[test]
fn has_locking_attachment_is_true_with_mixed_image_and_file_attachments() {
    App::test((), |mut app| async move {
        let model = build_test_context_model(&mut app);

        model.update(&mut app, |m, _| {
            m.append_pending_attachments_for_test(vec![
                make_file_attachment("notes.txt"),
                make_image_attachment("a.png"),
            ]);
        });

        model.read(&app, |m, _| assert!(m.has_locking_attachment()));
    });
}

#[test]
fn take_pending_attachments_drains_and_returns_all_staged() {
    App::test((), |mut app| async move {
        let model = build_test_context_model(&mut app);
        model.update(&mut app, |m, _| {
            m.append_pending_attachments_for_test(vec![
                make_image_attachment("a.png"),
                make_file_attachment("notes.txt"),
            ]);
        });

        let taken = model.update(&mut app, |m, ctx| m.take_pending_attachments(ctx));
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[0].file_name(), "a.png");
        assert_eq!(taken[1].file_name(), "notes.txt");

        // Draining clears the live staging so the input's attachment chips disappear.
        model.read(&app, |m, _| assert!(m.pending_attachments().is_empty()));
    });
}

#[test]
fn enqueue_moves_staged_attachments_onto_the_row_and_clears_input() {
    // Mirrors the enqueue sites in `input.rs`: `take_pending_attachments` drains the live input
    // staging and the drained set is stored on the queued row via `new_with_attachments`, leaving
    // no attachments behind in the input.
    App::test((), |mut app| async move {
        let model = build_test_context_model(&mut app);
        let queued = app.add_singleton_model(QueuedQueryModel::new);
        let conv = AIConversationId::new();

        model.update(&mut app, |m, _| {
            m.append_pending_attachments_for_test(vec![
                make_image_attachment("a.png"),
                make_file_attachment("notes.txt"),
            ]);
        });

        // Capture-and-clear, then store on the row (the exact composition used at enqueue time).
        let taken = model.update(&mut app, |m, ctx| m.take_pending_attachments(ctx));
        let id = queued.update(&mut app, |q, ctx| {
            q.append(
                conv,
                QueuedQuery::new_with_attachments(
                    "queued".to_owned(),
                    QueuedQueryOrigin::AutoQueueToggle,
                    taken,
                ),
                ctx,
            )
        });

        // Live staging is cleared; the row owns the attachments.
        model.read(&app, |m, _| assert!(m.pending_attachments().is_empty()));
        queued.read(&app, |q, _| {
            let attachments = q.attachments_for(conv, id);
            assert_eq!(attachments.len(), 2);
            assert_eq!(attachments[0].file_name(), "a.png");
            assert_eq!(attachments[1].file_name(), "notes.txt");
        });
    });
}
