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

        // A path with nothing at it reports `DoesNotExist` rather than a raw IO error, so callers
        // can offer an empty buffer instead of an error treatment (APP-5266).
        let event = receiver.recv().await.expect("Could not receive the result");
        match event {
            TestFileModelEvent::FailedToLoad(err) => assert_eq!(err, "DoesNotExist"),
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

/// APP-5266: only a genuinely absent path may be reported as `DoesNotExist`, because that is what
/// callers turn into an empty buffer. Anything that exists but cannot be read stays an error.
#[test]
fn test_read_classifies_missing_paths_apart_from_unreadable_ones() {
    let directory = tempfile::tempdir().expect("temp dir");

    let missing = directory.path().join("not-here.md");
    assert!(matches!(
        block_on(FileModel::read_and_classify(&missing)),
        Err(FileLoadError::DoesNotExist)
    ));

    // A directory exists, so reading it is a real failure.
    assert!(matches!(
        block_on(FileModel::read_and_classify(directory.path())),
        Err(FileLoadError::IOError(_))
    ));

    let readable = directory.path().join("readable.md");
    std::fs::write(&readable, "# readable").expect("write file");
    assert_eq!(
        block_on(FileModel::read_and_classify(&readable)).expect("read"),
        "# readable"
    );
}

/// A dangling symlink reads as `NotFound`, but something *is* at the path, so opening it as a new
/// empty file would quietly write through the link. It stays an error.
#[cfg(unix)]
#[test]
fn test_read_treats_a_dangling_symlink_as_an_error() {
    let directory = tempfile::tempdir().expect("temp dir");
    let link = directory.path().join("dangling.md");
    std::os::unix::fs::symlink(directory.path().join("no-such-target.md"), &link)
        .expect("create symlink");

    assert!(matches!(
        block_on(FileModel::read_and_classify(&link)),
        Err(FileLoadError::IOError(_))
    ));
}

/// A failed existence probe must not be read as "missing". Only the probe's own `NotFound`
/// confirms absence; a permission or I/O failure would otherwise hand the user an empty buffer
/// over a file that is really there.
#[test]
fn test_a_failed_existence_probe_is_not_treated_as_missing() {
    let not_found = || io::Error::from(io::ErrorKind::NotFound);

    // The probe succeeded, so something is at the path (e.g. a dangling symlink).
    assert!(matches!(
        FileModel::classify_missing_read(not_found(), None),
        FileLoadError::IOError(_)
    ));

    // The probe agrees nothing is there.
    assert!(matches!(
        FileModel::classify_missing_read(not_found(), Some(not_found())),
        FileLoadError::DoesNotExist
    ));

    // The probe failed for a reason that says nothing about existence.
    for kind in [
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::Other,
        io::ErrorKind::InvalidInput,
    ] {
        assert!(
            matches!(
                FileModel::classify_missing_read(not_found(), Some(io::Error::from(kind))),
                FileLoadError::IOError(_)
            ),
            "{kind:?} must not be reported as a missing file"
        );
    }
}

/// APP-5266: the first write of a buffer opened at a missing path must not clobber a file that
/// appeared in the meantime. Nothing watches such a path, so there is no reload or conflict to
/// warn us — the write itself has to refuse.
#[test]
fn test_creating_a_new_file_refuses_to_clobber() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.add_singleton_model(|_| DetectedRepositories::default());
        let files = app.add_singleton_model(FileModel::new);
        let receiver = setup_event_channel(app, &files);

        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("raced.md");

        let file_id = files.update(app, |model, ctx| model.open(&path, true, ctx));
        await_load(&receiver).await;

        // Someone else creates the file between the open and the first save.
        std::fs::write(&path, "written by someone else").expect("write file");

        let dispatched = files.update(app, |model, ctx| {
            model.create_new_file(
                file_id,
                "our content".to_string(),
                ContentVersion::new(),
                ctx,
            )
        });
        std::mem::drop(dispatched.expect("save should dispatch"));

        match receiver.recv().await.expect("Could not receive the result") {
            TestFileModelEvent::FailedToSave => (),
            event => panic!("Expected the save to be refused, got {event:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(&path).expect("the other file should survive"),
            "written by someone else"
        );

        // With nothing in the way, the same call creates the file.
        let fresh = directory.path().join("unraced.md");
        let fresh_id = files.update(app, |model, ctx| model.open(&fresh, true, ctx));
        await_load(&receiver).await;
        let dispatched = files.update(app, |model, ctx| {
            model.create_new_file(fresh_id, "ours".to_string(), ContentVersion::new(), ctx)
        });
        std::mem::drop(dispatched.expect("save should dispatch"));
        match receiver.recv().await.expect("Could not receive the result") {
            TestFileModelEvent::FileSaved => (),
            event => panic!("Expected the save to succeed, got {event:?}"),
        }
        assert_eq!(std::fs::read_to_string(&fresh).expect("created"), "ours");
    });
}

static TEST_FILE_CONTENT: &[u8] = include_bytes!("../test_data/test_file.rs");
