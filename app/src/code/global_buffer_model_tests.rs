use std::cell::RefCell;
use std::io;
use std::rc::Rc;

use lsp::LspManagerModel;
use remote_server::proto::TextEdit;
use repo_metadata::RepoMetadataModel;
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::watcher::DirectoryWatcher;
use warp_editor::content::buffer::Buffer;
use warp_files::{FileModel, FileModelEvent};
use warp_util::content_version::ContentVersion;
use warp_util::file::{FileId, FileLoadError};
use warp_util::host_id::HostId;
use warp_util::standardized_path::StandardizedPath;
use warpui::{App, ModelHandle, SingletonEntity};

use super::{
    BufferSource, CharOffsetEdit, GlobalBufferModel, GlobalBufferModelEvent, InternalBufferState,
    MAX_EDITOR_BUFFER_NEWLINE_COUNT, PendingEditBatch,
};
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

fn seed_local_buffer(
    app: &mut App,
    content: &str,
    loaded: bool,
) -> (FileId, ModelHandle<Buffer>, ContentVersion) {
    let buffer = app.add_model(|_| Buffer::default());
    let version = ContentVersion::new();
    if !content.is_empty() {
        buffer.update(app, |buffer, ctx| {
            buffer.replace_all(content, ctx);
            buffer.set_version(version);
        });
    }

    let file_id = FileId::new();
    gbm(app).update(app, |model, _| {
        model.buffers.insert(
            file_id,
            InternalBufferState {
                buffer: buffer.downgrade(),
                latest_buffer_version: None,
                pending_diff_parse: None,
                source: BufferSource::Local {
                    base_content_version: loaded.then_some(version),
                    initial_content_version: loaded.then_some(version),
                },
            },
        );
    });
    (file_id, buffer, version)
}

fn record_load_failures(
    app: &mut App,
    global_buffer: &ModelHandle<GlobalBufferModel>,
) -> Rc<RefCell<Vec<FileId>>> {
    let failed_file_ids = Rc::new(RefCell::new(Vec::new()));
    app.update(|ctx| {
        let failed_file_ids = failed_file_ids.clone();
        ctx.subscribe_to_model(global_buffer, move |_, event, _| {
            if let GlobalBufferModelEvent::FailedToLoad { file_id, error } = event {
                assert!(matches!(
                    error.as_ref(),
                    FileLoadError::IOError(error) if error.kind() == io::ErrorKind::FileTooLarge
                ));
                failed_file_ids.borrow_mut().push(*file_id);
            }
        });
    });
    failed_file_ids
}

fn excessive_newline_content() -> String {
    "\n".repeat(MAX_EDITOR_BUFFER_NEWLINE_COUNT + 1)
}

#[test]
fn file_update_preserves_buffer_when_content_exceeds_editor_limits() {
    App::test((), |mut app| async move {
        init_app(&mut app);
        app.add_singleton_model(GlobalBufferModel::new);
        let (file_id, buffer, base_version) = seed_local_buffer(&mut app, "preserved", true);
        let global_buffer = gbm(&app);
        let failed_file_ids = record_load_failures(&mut app, &global_buffer);
        let event = FileModelEvent::FileUpdated {
            id: file_id,
            content: excessive_newline_content(),
            base_version,
            new_version: ContentVersion::new(),
        };
        let files = FileModel::handle(&app);

        global_buffer.update(&mut app, |model, ctx| {
            model.handle_file_model_events(files, &event, ctx);
        });

        app.read(|ctx| {
            assert_eq!(buffer.as_ref(ctx).text().into_string(), "preserved");
            assert!(global_buffer.as_ref(ctx).buffer_loaded(file_id));
        });
        assert_eq!(failed_file_ids.borrow().as_slice(), &[file_id]);
    })
}

#[test]
fn rejected_initial_load_recovers_when_file_update_is_within_limits() {
    App::test((), |mut app| async move {
        init_app(&mut app);
        app.add_singleton_model(GlobalBufferModel::new);
        let (file_id, buffer, base_version) = seed_local_buffer(&mut app, "", false);
        let files = FileModel::handle(&app);
        let global_buffer = gbm(&app);
        let failed_file_ids = record_load_failures(&mut app, &global_buffer);

        let rejected_event = FileModelEvent::FileLoaded {
            content: excessive_newline_content(),
            id: file_id,
            version: base_version,
        };
        global_buffer.update(&mut app, |model, ctx| {
            model.handle_file_model_events(files.clone(), &rejected_event, ctx);
        });
        app.read(|ctx| {
            assert_eq!(buffer.as_ref(ctx).text().into_string(), "");
            assert!(!global_buffer.as_ref(ctx).buffer_loaded(file_id));
        });
        assert_eq!(failed_file_ids.borrow().as_slice(), &[file_id]);

        let new_version = ContentVersion::new();
        let recovered_event = FileModelEvent::FileUpdated {
            id: file_id,
            content: "now loadable".to_string(),
            base_version,
            new_version,
        };
        global_buffer.update(&mut app, |model, ctx| {
            model.handle_file_model_events(files, &recovered_event, ctx);
        });

        app.read(|ctx| {
            assert_eq!(buffer.as_ref(ctx).text().into_string(), "now loadable");
            assert_eq!(buffer.as_ref(ctx).version(), new_version);
            assert!(global_buffer.as_ref(ctx).buffer_loaded(file_id));
        });
    })
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
