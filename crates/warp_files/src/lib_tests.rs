use async_channel::{unbounded, Receiver};
use warpui::{r#async::block_on, App, ModelHandle};

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
            assert!(result.is_err());

            let e = result.unwrap_err();
            assert!(matches!(e, FileSaveError::NoFilePath(file_id) if file_id == id));
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

static TEST_FILE_CONTENT: &[u8] = include_bytes!("../test_data/test_file.rs");
