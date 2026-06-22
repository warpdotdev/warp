//! Unit tests for [`BlocklistAIContextModel`].
//!
//! These tests deliberately bypass the production [`BlocklistAIContextModel::new`] constructor
//! (which subscribes to several singletons) and instead use [`BlocklistAIContextModel::new_for_test`]
//! together with [`super::agent_view::AgentViewController::new`]. That keeps the fixture small
//! enough to focus on context logic without standing up `BlocklistAIHistoryModel`,
//! `LLMPreferences`, `CloudModel`, `UpdateManager`, or `AppExecutionMode`.

use std::sync::Arc;

use parking_lot::FairMutex;
#[cfg(feature = "local_fs")]
use repo_metadata::DirectoryWatcher;
#[cfg(feature = "local_fs")]
use warp_util::standardized_path::StandardizedPath;
use warpui::r#async::executor::Background;
use warpui::{App, EntityId, ModelHandle};

use super::{BlocklistAIContextModel, PendingAttachment, PendingFile};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIAgentContext, ImageContext};
use crate::ai::blocklist::agent_view::{AgentViewController, EphemeralMessageModel};
use crate::ai::blocklist::{
    BlocklistAIHistoryModel, QueuedQuery, QueuedQueryModel, QueuedQueryOrigin,
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

    let ephemeral_message_model = app.add_model(|_| EphemeralMessageModel::new());
    let agent_view_controller = app.add_model(|_| {
        AgentViewController::new(
            terminal_model.clone(),
            terminal_view_id,
            ephemeral_message_model,
        )
    });

    app.add_model(|_| {
        BlocklistAIContextModel::new_for_test(
            terminal_model,
            terminal_view_id,
            agent_view_controller,
        )
    })
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
        initialize_history_persistence_for_tests(&mut app);
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let queued = app.add_singleton_model(QueuedQueryModel::new);
        let model = build_test_context_model(&mut app);
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
