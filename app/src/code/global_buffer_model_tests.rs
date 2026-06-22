use lsp::LspManagerModel;
use remote_server::proto::TextEdit;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::watcher::DirectoryWatcher;
use repo_metadata::RepoMetadataModel;
use warp_files::FileModel;
use warp_util::content_version::ContentVersion;
use warp_util::host_id::HostId;
use warp_util::standardized_path::StandardizedPath;
use warpui::{App, ModelHandle, SingletonEntity};

use super::{BufferSource, CharOffsetEdit, GlobalBufferModel, PendingEditBatch};
use crate::test_util::settings::initialize_settings_for_tests;

// ── Test-only helpers on GlobalBufferModel ────────────────────────
// These live here (child module) rather than in global_buffer_model.rs
// to keep test infrastructure out of the production source file.
//
// Note: `seed_remote_buffer_for_test` and `sync_clock_for_remote_test`
// are `pub(crate)` in global_buffer_model.rs because they're shared
// with `buffer_location_tests`.

impl GlobalBufferModel {
    /// Returns whether a pending edit batch exists for a Remote buffer.
    fn has_pending_batch_for_test(&self, file_id: warp_util::file::FileId) -> bool {
        self.buffers.get(&file_id).is_some_and(|state| {
            matches!(&state.source, BufferSource::Remote { pending_batch, .. } if pending_batch.is_some())
        })
    }

    /// Returns the number of edits in the pending batch, or 0 if none.
    fn pending_batch_edit_count_for_test(&self, file_id: warp_util::file::FileId) -> usize {
        self.buffers
            .get(&file_id)
            .and_then(|state| match &state.source {
                BufferSource::Remote { pending_batch, .. } => {
                    pending_batch.as_ref().map(|b| b.edits.len())
                }
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Inserts a fake pending batch so tests can verify discard/flush
    /// behavior without needing a real `RemoteServerClient` or the
    /// `ContentChanged` subscription path.
    fn insert_pending_batch_for_test(
        &mut self,
        file_id: warp_util::file::FileId,
        expected_server_version: u64,
        edits: Vec<remote_server::proto::TextEdit>,
        client_version: ContentVersion,
    ) {
        let Some(state) = self.buffers.get_mut(&file_id) else {
            return;
        };
        if let BufferSource::Remote {
            pending_batch,
            sync_clock,
            ..
        } = &mut state.source
        {
            if let Some(clock) = sync_clock.as_mut() {
                clock.client_version = client_version;
            }
            *pending_batch = Some(PendingEditBatch {
                expected_server_version,
                edits,
                latest_client_version: client_version,
                debounce_timer: None,
            });
        }
    }
}

// ── Test setup ────────────────────────────────────────────────────

fn init_app(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| LspManagerModel::new());
    app.add_singleton_model(DirectoryWatcher::new);
    app.add_singleton_model(|_| DetectedRepositories::default());
    app.add_singleton_model(RepoMetadataModel::new);
    app.add_singleton_model(FileModel::new);
}

fn gbm(app: &App) -> ModelHandle<GlobalBufferModel> {
    GlobalBufferModel::handle(app)
}

fn content(app: &App, file_id: warp_util::file::FileId) -> String {
    let handle = gbm(app);
    app.read(|ctx| {
        handle
            .as_ref(ctx)
            .content_for_file(file_id, ctx)
            .unwrap_or_default()
    })
}

fn text_edit(start: u64, end: u64, text: &str) -> TextEdit {
    TextEdit {
        start_offset: start,
        end_offset: end,
        text: text.to_string(),
    }
}

fn char_edit(start: usize, end: usize, text: &str) -> CharOffsetEdit {
    CharOffsetEdit {
        start: string_offset::CharOffset::from(start),
        end: string_offset::CharOffset::from(end),
        text: text.to_string(),
    }
}

fn test_host_id() -> HostId {
    HostId::new("test-host".to_string())
}

fn test_path() -> StandardizedPath {
    StandardizedPath::try_new("/test/file.txt").unwrap()
}

// ── Pending edit batch: discard on server push ───────────────────

#[test]
fn pending_batch_discarded_on_server_push_with_conflict() {
    App::test((), |mut app| async move {
        init_app(&mut app);
        app.add_singleton_model(GlobalBufferModel::new);

        let host_id = test_host_id();
        let path = test_path();

        // Seed a remote buffer at server_version=1, client_version=0.
        let _buffer_state = gbm(&app).update(&mut app, |gbm, ctx| {
            gbm.seed_remote_buffer_for_test(host_id.clone(), path.clone(), "hello", 1, ctx)
        });
        let file_id = _buffer_state.file_id;

        // Simulate client edits that haven't been flushed yet.
        let client_cv = ContentVersion::new();
        gbm(&app).update(&mut app, |gbm, _ctx| {
            gbm.insert_pending_batch_for_test(
                file_id,
                1, // expected_server_version
                vec![text_edit(6, 6, " world")],
                client_cv,
            );
        });

        // Verify the batch exists.
        let handle = gbm(&app);
        app.read(|ctx| {
            assert!(handle.as_ref(ctx).has_pending_batch_for_test(file_id));
            assert_eq!(
                handle
                    .as_ref(ctx)
                    .pending_batch_edit_count_for_test(file_id),
                1
            );
        });

        // Server push arrives. Since client_cv != 0, this triggers a conflict
        // and the batch should be discarded.
        gbm(&app).update(&mut app, |gbm, ctx| {
            gbm.handle_buffer_updated_push(
                &host_id,
                path.as_str(),
                2, // new_server_version
                0, // expected_client_version (server doesn't know about our edits)
                &[char_edit(6, 6, " push")],
                ctx,
            );
        });

        // Batch should be discarded.
        let handle = gbm(&app);
        app.read(|ctx| {
            assert!(
                !handle.as_ref(ctx).has_pending_batch_for_test(file_id),
                "Pending batch should be discarded on server push"
            );
        });

        // Content should be unchanged (conflict path, push not applied).
        assert_eq!(content(&app, file_id), "hello");
    })
}

#[test]
fn pending_batch_discarded_on_conflict_detected() {
    App::test((), |mut app| async move {
        init_app(&mut app);
        app.add_singleton_model(GlobalBufferModel::new);

        let host_id = test_host_id();
        let path = test_path();

        let _buffer_state = gbm(&app).update(&mut app, |gbm, ctx| {
            gbm.seed_remote_buffer_for_test(host_id.clone(), path.clone(), "hello", 1, ctx)
        });
        let file_id = _buffer_state.file_id;

        // Insert a pending batch.
        let client_cv = ContentVersion::new();
        gbm(&app).update(&mut app, |gbm, _ctx| {
            gbm.insert_pending_batch_for_test(
                file_id,
                1,
                vec![text_edit(6, 6, " edit")],
                client_cv,
            );
        });

        let handle = gbm(&app);
        app.read(|ctx| {
            assert!(handle.as_ref(ctx).has_pending_batch_for_test(file_id));
        });

        // BufferConflictDetected arrives.
        gbm(&app).update(&mut app, |gbm, ctx| {
            gbm.handle_buffer_conflict_detected(&host_id, path.as_str(), ctx);
        });

        // Batch should be discarded.
        let handle = gbm(&app);
        app.read(|ctx| {
            assert!(
                !handle.as_ref(ctx).has_pending_batch_for_test(file_id),
                "Pending batch should be discarded on conflict detected"
            );
        });
    })
}

#[test]
fn server_push_accepted_without_pending_batch() {
    App::test((), |mut app| async move {
        init_app(&mut app);
        app.add_singleton_model(GlobalBufferModel::new);

        let host_id = test_host_id();
        let path = test_path();

        let _buffer_state = gbm(&app).update(&mut app, |gbm, ctx| {
            gbm.seed_remote_buffer_for_test(host_id.clone(), path.clone(), "hello", 1, ctx)
        });
        let file_id = _buffer_state.file_id;

        // No pending batch — clean push should be accepted.
        let handle = gbm(&app);
        app.read(|ctx| {
            assert!(!handle.as_ref(ctx).has_pending_batch_for_test(file_id));
        });

        gbm(&app).update(&mut app, |gbm, ctx| {
            gbm.handle_buffer_updated_push(
                &host_id,
                path.as_str(),
                2,
                0, // matches client_version=0
                &[char_edit(6, 6, " world")],
                ctx,
            );
        });

        assert_eq!(content(&app, file_id), "hello world");

        // Clock should be updated.
        let handle = gbm(&app);
        app.read(|ctx| {
            let clock = handle
                .as_ref(ctx)
                .sync_clock_for_remote_test(file_id)
                .unwrap();
            assert_eq!(clock.server_version, ContentVersion::from_raw(2));
            assert_eq!(clock.client_version, ContentVersion::from_raw(0));
        });
    })
}

#[test]
fn pending_batch_bumps_client_version_immediately() {
    App::test((), |mut app| async move {
        init_app(&mut app);
        app.add_singleton_model(GlobalBufferModel::new);

        let host_id = test_host_id();
        let path = test_path();

        let _buffer_state = gbm(&app).update(&mut app, |gbm, ctx| {
            gbm.seed_remote_buffer_for_test(host_id.clone(), path.clone(), "hello", 1, ctx)
        });
        let file_id = _buffer_state.file_id;

        // Insert a batch — this simulates what the ContentChanged handler does:
        // sync_clock.client_version is bumped immediately.
        let client_cv = ContentVersion::new();
        gbm(&app).update(&mut app, |gbm, _ctx| {
            gbm.insert_pending_batch_for_test(
                file_id,
                1,
                vec![text_edit(6, 6, " edit")],
                client_cv,
            );
        });

        // The sync clock's client_version should already reflect the edit,
        // even though the batch hasn't been flushed.
        let handle = gbm(&app);
        app.read(|ctx| {
            let clock = handle
                .as_ref(ctx)
                .sync_clock_for_remote_test(file_id)
                .unwrap();
            assert_eq!(clock.client_version, client_cv);
            // server_version unchanged
            assert_eq!(clock.server_version, ContentVersion::from_raw(1));
        });
    })
}
