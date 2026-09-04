use async_channel::{Receiver, unbounded};
use warpui_core::r#async::block_on;
use warpui_core::{App, ModelHandle};

// lib_tests.rs
use super::*;

const WRITE_TEST_PATH: &str = "test_data/test_write/";

/// This enum is used so that we can pass the event through the async channel.
/// io::Error is not clonable, so we can't clone the FileModelEvent.
#[derive(Debug)]
enum TestFileModelEvent {
    FileLoaded {
        id: FileId,
        content: String,
        _version: ContentVersion,
    },
    FileSaved,
    FailedToLoad(String),
    FailedToSave,
}

impl From<&FileModelEvent> for TestFileModelEvent {
    fn from(event: &FileModelEvent) -> Self {
        match event {
            FileModelEvent::FileLoaded {
                id,
                content,
                version,
            } => TestFileModelEvent::FileLoaded {
                id: *id,
                content: content.clone(),
                _version: *version,
            },
            FileModelEvent::FileSaved { .. } => TestFileModelEvent::FileSaved,
            FileModelEvent::FailedToLoad {
                id: _id,
                error: err,
            } => TestFileModelEvent::FailedToLoad(format!("{err:?}")),
            FileModelEvent::FailedToSave { .. } => TestFileModelEvent::FailedToSave,
            FileModelEvent::FileUpdated { .. } => {
                // For now, we don't handle file updated events in tests
                // This could be extended to include a FileUpdated variant in TestFileModelEvent if needed
                TestFileModelEvent::FileLoaded {
                    id: event.file_id(),
                    content: String::new(),
                    _version: ContentVersion::new(),
                }
            }
        }
    }
}

/// Setup a Tokio channel that will forward any events from the FileModel to the receiver.
fn setup_event_channel(
    app: &mut App,
    files: &ModelHandle<FileModel>,
) -> Receiver<TestFileModelEvent> {
    let (sender, receiver) = unbounded();
    app.update(|ctx| {
        ctx.subscribe_to_model(files, move |_model, event, _ctx| {
            block_on(sender.send(TestFileModelEvent::from(event)))
                .expect("Could not send the result");
        });
    });
    receiver
}

#[test]
fn test_load() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let files = app.add_singleton_model(FileModel::new);
        let receiver = setup_event_channel(app, &files);

        // Load the test file.
        files.update(app, |model, ctx| {
            model.open(Path::new("test_data/test_file.rs"), false, ctx);
        });

        // Check that the first event out is the file loaded event.
        let event = receiver.recv().await.expect("Could not receive the result");
        match event {
            TestFileModelEvent::FileLoaded { content, .. } => {
                assert_eq!(content.as_bytes(), TEST_FILE_CONTENT)
            }
            _ => panic!("Failed to load file"),
        }
    });
}

#[test]
fn test_save_uninitialized_file() {
    App::test((), |mut app| async move {
        let app = &mut app;

        let files = app.add_singleton_model(FileModel::new);
        let id = FileId::new();

        // This file has not been initialized with the model.  Make sure trying to save it fails immediately.
        files.update(app, |model, ctx| {
            let result = model.save(
                id,
                "This file doesn't exist".to_string(),
                ContentVersion::new(),
                ctx,
            );
            assert!(
                matches!(result, Err(FileSaveError::NoFilePath(file_id)) if file_id == id),
                "expected NoFilePath error"
            );
        });
    });
}

#[test]
fn test_save_file() {
    // Create the test write directory if it doesn't exist.
    std::fs::create_dir_all(WRITE_TEST_PATH).unwrap();

    // Write the test file content to a random file in the test write directory.
    let path = PathBuf::from(WRITE_TEST_PATH).join("test_save_file.rs");
    std::fs::write(&path, TEST_FILE_CONTENT).unwrap();

    App::test((), |mut app| async move {
        let app = &mut app;
        let files = app.add_singleton_model(FileModel::new);
        let receiver = setup_event_channel(app, &files);

        // Open the newly created file.
        let path_clone = path.clone();
        files.update(app, |model, ctx| {
            model.open(&path_clone, false, ctx);
        });

        let file_id = match receiver.recv().await.expect("Could not receive the result") {
            TestFileModelEvent::FileLoaded { id, .. } => id,
            _ => panic!("Failed to load file"),
        };

        let old_version = files.read(app, |files, _ctx| files.version(file_id));
        let new_version = ContentVersion::new();

        // Save new content to the file.
        files.update(app, |model, ctx| {
            let result = model.save(file_id, "Overwrite content".to_string(), new_version, ctx);
            assert!(result.is_ok());
        });

        // Make sure that the file saved event was emitted.
        match receiver.recv().await.expect("Could not receive the result") {
            TestFileModelEvent::FileSaved => (),
            _ => panic!("Failed to save file"),
        }

        // Make sure the content on disk matches the content we saved.
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "Overwrite content");

        // Make sure the version was updated.
        let model_version = files.read(app, |files, _ctx| files.version(file_id));
        assert_ne!(old_version, model_version);
        assert_eq!(Some(new_version), model_version);
    });
}

#[test]
fn test_load_missing_file() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let files = app.add_singleton_model(FileModel::new);
        let receiver = setup_event_channel(app, &files);

        // Load a file that doesn't exist.
        files.update(app, |model, ctx| {
            model.open(Path::new("test_data/missing_file.rs"), false, ctx);
        });

        // Check that the first event out is the failed to load event.
        let event = receiver.recv().await.expect("Could not receive the result");
        match event {
            TestFileModelEvent::FailedToLoad(err) => {
                // File not found error strings differ across operating systems.
                #[cfg(not(windows))]
                let os_error_message = "No such file or directory";
                #[cfg(windows)]
                let os_error_message = "The system cannot find the file specified.";

                assert_eq!(
                    err,
                    format!(
                        "IOError(Os {{ code: 2, kind: NotFound, message: \"{os_error_message}\" }})"
                    )
                );
            }
            _ => panic!("Failed to load file"),
        }
    });
}

#[test]
fn test_save_missing_directory() {
    // Create the test write directory if it doesn't exist.
    let directory = PathBuf::from(WRITE_TEST_PATH).join("missing-directory");
    std::fs::create_dir_all(&directory).unwrap();

    // Write the test file content to a random file in the test write directory.
    let path = directory.join("test_save_missing_directory.rs");
    std::fs::write(&path, TEST_FILE_CONTENT).unwrap();

    App::test((), |mut app| async move {
        let app = &mut app;
        let files = app.add_singleton_model(FileModel::new);
        let receiver = setup_event_channel(app, &files);

        // Save a file to a directory that doesn't exist.
        let file_id = files.update(app, |model, ctx| model.open(&path, false, ctx));

        // Check that the first event out is the successful load.
        let event = receiver.recv().await.expect("Could not receive the result");
        match event {
            TestFileModelEvent::FileLoaded { content, .. } => {
                assert_eq!(content.as_bytes(), TEST_FILE_CONTENT)
            }
            event => panic!("Failed to load file {event:?}"),
        }

        // Delete the directory that the file is in.
        std::fs::remove_dir_all(directory).unwrap();

        // Save new content to the file.
        files.update(app, |model, ctx| {
            let result = model.save(
                file_id,
                "Overwrite content".to_string(),
                ContentVersion::new(),
                ctx,
            );
            assert!(result.is_ok());
        });

        // Now we expect the save to succeed because ensure_parent_directories will create the missing directory
        match receiver.recv().await.expect("Could not receive the result") {
            TestFileModelEvent::FileSaved => {
                // Make sure the content on disk matches the content we saved.
                let content = std::fs::read_to_string(&path).unwrap();
                assert_eq!(content, "Overwrite content");
            }
            event => panic!("Save should have succeeded but got event: {event:?}"),
        }
    });
}

/// APP-5243: a bare relative file name has an empty parent, which platform watchers resolve to
/// Warp's own process directory. Watching (or worse, unwatching) that directory is never what the
/// caller asked for, so such files get no individual watcher at all.
#[test]
fn test_watch_path_ignores_empty_parents() {
    assert_eq!(FileModel::watch_path_for(Path::new("README.md")), None);
    assert_eq!(FileModel::watch_path_for(Path::new("")), None);
    assert_eq!(
        FileModel::watch_path_for(Path::new("docs/README.md")),
        Some(PathBuf::from("docs"))
    );

    let directory = std::env::temp_dir().join("app-5243");
    assert_eq!(
        FileModel::watch_path_for(&directory.join("README.md")),
        Some(directory)
    );
}

/// Waits for the read that `FileModel::open` spawned to settle, whichever way it resolves.
async fn await_load(receiver: &Receiver<TestFileModelEvent>) {
    match receiver.recv().await.expect("Could not receive the result") {
        TestFileModelEvent::FileLoaded { .. } | TestFileModelEvent::FailedToLoad(_) => (),
        event => panic!("Expected a load result, got {event:?}"),
    }
}

/// Registration and unregistration must use the exact same path, whichever entry point registered
/// the watcher. `register_file_path` used to watch the file itself while `unsubscribe` unwatched
/// its parent directory, so teardown removed a watch that was never added and left the real one
/// behind — the same asymmetry class as the crash this fixes.
#[test]
fn test_registration_and_unregistration_use_the_same_path() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.add_singleton_model(|_| DetectedRepositories::default());
        let files = app.add_singleton_model(FileModel::new);
        let receiver = setup_event_channel(app, &files);

        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("watched.md");
        std::fs::write(&path, "# watched").expect("write file");

        // `register_file_path` registers up front...
        let registered_id =
            files.update(app, |model, ctx| model.register_file_path(&path, true, ctx));

        // ...and `open` registers once the read succeeds. Both must land on the same directory.
        let opened_id = files.update(app, |model, ctx| model.open(&path, true, ctx));
        await_load(&receiver).await;

        files.read(app, |model, _| {
            let stored_path = model.file_path(opened_id).expect("stored path");
            let watch_path = FileModel::watch_path_for(&stored_path).expect("watch path");
            assert_eq!(Some(watch_path.as_path()), stored_path.parent());
            assert_eq!(
                model.registered_watch_path(registered_id),
                Some(watch_path.as_path())
            );
            assert_eq!(
                model.registered_watch_path(opened_id),
                Some(watch_path.as_path())
            );
        });

        // Unsubscribing releases exactly what was registered, and nothing is left tracked.
        files.update(app, |model, ctx| {
            model.unsubscribe(registered_id, ctx);
            model.unsubscribe(opened_id, ctx);
        });
        files.read(app, |model, _| {
            assert_eq!(model.registered_watch_path(registered_id), None);
            assert_eq!(model.registered_watch_path(opened_id), None);
            assert_eq!(model.file_path(registered_id), None);
            assert_eq!(model.file_path(opened_id), None);
        });
    });
}

/// A file whose read fails never gets a watcher, so teardown must not try to remove one. This is
/// the exact shape of the crash: an unresolved relative path failed to load, and unsubscribing it
/// handed an empty directory to the platform watcher.
#[test]
fn test_a_failed_open_registers_no_watcher() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.add_singleton_model(|_| DetectedRepositories::default());
        let files = app.add_singleton_model(FileModel::new);
        let receiver = setup_event_channel(app, &files);

        let file_id = files.update(app, |model, ctx| {
            model.open(Path::new("app-5243-does-not-exist.md"), true, ctx)
        });
        await_load(&receiver).await;

        files.read(app, |model, _| {
            assert_eq!(model.registered_watch_path(file_id), None);
        });

        files.update(app, |model, ctx| model.unsubscribe(file_id, ctx));
        assert_eq!(files.read(app, |model, _| model.file_path(file_id)), None);
    });
}

/// Regression for APP-4801: `FileModel::open`'s read must be rejected up front for an oversized
/// file, not read wholesale into memory.
#[test]
fn test_load_oversized_file_reports_error_not_content() {
    App::test((), |mut app| async move {
        let app = &mut app;
        let files = app.add_singleton_model(FileModel::new);
        let receiver = setup_event_channel(app, &files);

        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("big.txt");
        std::fs::write(&path, vec![b'a'; (MAX_EDITOR_FILE_BYTES + 1) as usize])
            .expect("write oversized file");

        files.update(app, |model, ctx| {
            model.open(&path, false, ctx);
        });

        match receiver.recv().await.expect("Could not receive the result") {
            TestFileModelEvent::FailedToLoad(err) => {
                assert!(err.contains("too large"), "got: {err}");
                assert!(!err.contains("NotFound"), "got: {err}");
            }
            event => panic!("Expected the oversized read to fail, got {event:?}"),
        }
    });
}

/// Regression for APP-4801: `read_content_for_file` must reject an oversized file, and must
/// keep that distinct from a genuinely missing file (`FileLoadError::DoesNotExist`).
#[test]
fn test_read_content_for_file_rejects_oversized_file_distinctly_from_missing() {
    let directory = tempfile::tempdir().expect("temp dir");
    let oversized_path = directory.path().join("big.txt");
    std::fs::write(
        &oversized_path,
        vec![b'a'; (MAX_EDITOR_FILE_BYTES + 1) as usize],
    )
    .expect("write oversized file");

    let oversized_error = block_on(FileModel::read_content_for_file(&oversized_path))
        .expect_err("should reject an oversized file");
    assert!(!matches!(oversized_error, FileLoadError::DoesNotExist));

    let missing_path = directory.path().join("does-not-exist.txt");
    let missing_error = block_on(FileModel::read_content_for_file(&missing_path))
        .expect_err("should fail for a missing file");
    assert!(matches!(missing_error, FileLoadError::DoesNotExist));
}

/// Regression for APP-4801: the autoreload loop's read helper must skip an oversized file
/// without failing the batch, matching the existing "skip files that fail to read" behavior.
#[test]
fn read_reload_contents_skips_oversized_file() {
    let directory = tempfile::tempdir().expect("temp dir");
    let oversized_path = directory.path().join("big.txt");
    std::fs::write(
        &oversized_path,
        vec![b'a'; (MAX_EDITOR_FILE_BYTES + 1) as usize],
    )
    .expect("write oversized file");
    let ok_path = directory.path().join("small.txt");
    std::fs::write(&ok_path, "fine").expect("write small file");

    let contents = block_on(read_reload_contents(vec![oversized_path, ok_path.clone()]));

    assert_eq!(contents, vec![(ok_path, "fine".to_string())]);
}

static TEST_FILE_CONTENT: &[u8] = include_bytes!("../test_data/test_file.rs");
