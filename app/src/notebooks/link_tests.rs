use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
};

use lazy_static::lazy_static;
use parking_lot::Mutex;
use tempfile::tempdir;
use url::Url;
use warp_util::path::LineAndColumnArg;
use warpui::{App, ModelHandle, SingletonEntity, WindowId};

use crate::{
    notebooks::{file::is_markdown_file, link::LinkEvent},
    terminal::{model::session::Session, shell::ShellType},
    test_util::settings::initialize_settings_for_tests,
    util::{file::external_editor::EditorSettings, openable_file_type::FileTarget},
    workspace::ActiveSession,
};
use settings::Setting as _;

use super::{LinkTarget, NotebookLinks, ResolveError, SessionSource};

fn url(s: &str) -> LinkTarget {
    LinkTarget::Url(Url::parse(s).expect("Invalid URL"))
}

fn local_directory(path: impl Into<PathBuf>) -> LinkTarget {
    LinkTarget::LocalDirectory { path: path.into() }
}

fn local_file(path: impl Into<PathBuf>) -> LinkTarget {
    let path = path.into();
    LinkTarget::LocalFile {
        is_markdown: is_markdown_file(&path),
        path,
        line_and_column: None,
        session: TEST_SESSION.clone(),
    }
}

fn local_file_location(path: impl Into<PathBuf>, line: usize, column: Option<usize>) -> LinkTarget {
    let path = path.into();
    LinkTarget::LocalFile {
        is_markdown: is_markdown_file(&path),
        path,
        line_and_column: Some(LineAndColumnArg {
            line_num: line,
            column_num: column,
        }),
        session: TEST_SESSION.clone(),
    }
}

lazy_static! {
    // ActiveSession holds a weak reference to the session, so we need this strong one to keep it
    // alive.
    static ref TEST_SESSION: Arc<Session> = Arc::new(Session::test().with_shell_launch_data(crate::terminal::ShellLaunchData::Executable { executable_path: PathBuf::from("/bin/bash"), shell_type: ShellType::Bash }));
}

/// Initialize the app and link resolver. For test purposes, we only care about the base
/// directory's value, not how it was obtained.
fn init_link_model(app: &mut App, base_directory: Option<&Path>) -> ModelHandle<NotebookLinks> {
    initialize_settings_for_tests(app);

    let window_id = WindowId::new();
    let source = match base_directory {
        Some(dir) => SessionSource::Target {
            session: TEST_SESSION.clone(),
            base_directory: dir.to_owned(),
        },
        // File links can't be resolved without a session, even if there's no working directory.
        None => SessionSource::Active(window_id),
    };
    app.add_singleton_model(|ctx| {
        let mut session = ActiveSession::default();
        session.set_session_for_test(window_id, TEST_SESSION.clone(), base_directory, None, ctx);
        session
    });
    app.add_model(|ctx| NotebookLinks::new(source, ctx))
}

async fn resolve(app: &App, links: &ModelHandle<NotebookLinks>, link: &str) -> LinkTarget {
    match links.read(app, |links, ctx| links.resolve(link, ctx)).await {
        Ok(target) => target,
        Err(err) => panic!("Error resolving {link}: {err}"),
    }
}

/// Ensure a file exists, creating its parents if necessary.
async fn touch(path: impl AsRef<Path>) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if let Err(err) = async_fs::create_dir_all(parent).await {
            if err.kind() != ErrorKind::AlreadyExists {
                panic!("Creating parent {} failed: {}", parent.display(), err);
            }
        }
    }

    async_fs::File::create(path)
        .await
        .expect("Creating test file failed")
        .sync_all()
        .await
        .expect("Syncing test file failed");
}

fn next_link_event(events: &Arc<Mutex<Vec<LinkEvent>>>) -> LinkEvent {
    events.lock().remove(0)
}

#[test]
fn test_resolve_bare_url() {
    App::test((), |mut app| async move {
        let base = tempdir().unwrap();
        let base_path = base.path();
        touch(base_path.join("nodot/slash")).await;
        touch(base_path.join(".vscode/settings.json")).await;
        touch(base_path.join("myfile.swift")).await;
        touch(base_path.join("license.txt")).await;
        touch(base_path.join("app/src/main.rs")).await;

        let links = init_link_model(&mut app, Some(base_path));

        assert_eq!(
            resolve(&app, &links, "example.com/some-path").await,
            url("http://example.com/some-path")
        );

        // These should not be considered URLs.
        assert_eq!(
            resolve(&app, &links, "nodot/slash").await,
            local_file(base_path.join("nodot/slash"))
        );
        assert_eq!(
            resolve(&app, &links, ".vscode/settings.json").await,
            local_file(base_path.join(".vscode/settings.json"))
        );

        // These rely on domain name validation.
        assert_eq!(
            resolve(&app, &links, "google.com").await,
            url("http://google.com")
        );
        assert_eq!(
            resolve(&app, &links, "warp.dev").await,
            url("http://warp.dev")
        );
        assert_eq!(
            resolve(&app, &links, "bbc.co.uk").await,
            url("http://bbc.co.uk")
        );
        assert_eq!(
            resolve(&app, &links, "192.168.0.1/admin").await,
            url("http://192.168.0.1/admin")
        );
        assert_eq!(
            resolve(&app, &links, "myfile.swift").await,
            local_file(base_path.join("myfile.swift"))
        );
        assert_eq!(
            resolve(&app, &links, "license.txt").await,
            local_file(base_path.join("license.txt"))
        );

        // `app` is a valid TLD, so this tests that we need both a TLD and a root domain to link as an
        // URL.
        assert_eq!(
            resolve(&app, &links, "app/src/main.rs").await,
            local_file(base_path.join("app/src/main.rs"))
        );
    });
}

#[test]
fn test_open_local_image_uses_system_generic_target() {
    App::test((), |mut app| async move {
        let base = tempdir().unwrap();
        let base_path = base.path();
        let image_path = base_path.join("images/example.png");
        touch(&image_path).await;
        let links = init_link_model(&mut app, Some(base_path));

        let events = Arc::new(Mutex::new(vec![]));
        {
            let events = events.clone();
            app.update(|ctx| {
                ctx.subscribe_to_model(&links, move |_, event, _| {
                    events.lock().push(event.clone());
                })
            });
        }

        links.update(&mut app, |links, ctx| {
            links.open(local_file(&image_path), ctx);
        });

        match next_link_event(&events) {
            LinkEvent::OpenFileWithTarget {
                path,
                target,
                line_col,
            } => {
                assert_eq!(path, image_path);
                assert_eq!(target, FileTarget::SystemGeneric);
                assert_eq!(line_col, None);
            }
            other => panic!("Expected OpenFileWithTarget event, got {other:?}"),
        }
    });
}

#[test]
fn test_resolve_valid_url() {
    App::test((), |mut app| async move {
        let links = init_link_model(&mut app, None);

        assert_eq!(
            resolve(&app, &links, "https://warp.dev").await,
            url("https://warp.dev")
        );
        assert_eq!(
            resolve(&app, &links, "mailto:test@warp.dev").await,
            url("mailto:test@warp.dev")
        );
    });
}

#[cfg_attr(windows, ignore = "TODO(CORE-3626)")]
#[test]
fn test_resolve_file_url() {
    App::test((), |mut app| async move {
        let base = tempdir().unwrap();
        let base_path = base.path();
        let test_file = base_path.join("some/path.txt");
        touch(&test_file).await;
        let links = init_link_model(&mut app, Some(base_path));

        assert_eq!(
            resolve(&app, &links, &format!("file://{}", test_file.display())).await,
            local_file(&test_file)
        );
        assert_eq!(
            resolve(
                &app,
                &links,
                &format!("file://localhost{}", test_file.display())
            )
            .await,
            local_file(&test_file)
        );

        // file:// URLs can have non-local hosts on Windows. If we encounter one, it should be kept a
        // URL for the system to handle.
        assert_eq!(
            resolve(&app, &links, "file://remote/some/path.txt").await,
            url("file://remote/some/path.txt")
        );
    });
}

#[test]
fn test_resolve_relative_file_no_base() {
    App::test((), |mut app| async move {
        let links = init_link_model(&mut app, None);

        let absolute_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("Cargo.toml")
            .canonicalize()
            .expect("Path exists");

        assert_eq!(
            resolve(&app, &links, absolute_path.to_str().unwrap()).await,
            local_file(absolute_path)
        );

        let absolute_directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .canonicalize()
            .expect("Path exists");
        assert_eq!(
            resolve(&app, &links, absolute_directory.to_str().unwrap()).await,
            local_directory(absolute_directory)
        );

        assert_eq!(
            links
                .read(&app, |links, ctx| links.resolve("relative/path.txt", ctx))
                .await,
            Err(ResolveError::MissingContext)
        );
    });
}

#[test]
fn test_resolve_relative_file_base() {
    // This absolute path is specifically not within the base directory.
    let absolute_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("Cargo.toml")
        .canonicalize()
        .expect("Path exists");

    App::test((), |mut app| async move {
        let base = tempdir().unwrap();
        let base_path = base.path();
        touch(base_path.join("relative/path.txt")).await;
        touch(base_path.join("dotted.txt")).await;
        let links = init_link_model(&mut app, Some(base_path));

        assert_eq!(
            resolve(&app, &links, absolute_path.to_str().unwrap()).await,
            local_file(&absolute_path)
        );

        assert_eq!(
            resolve(&app, &links, "relative/path.txt").await,
            local_file(base_path.join("relative/path.txt"))
        );
        assert_eq!(
            resolve(&app, &links, "./dotted.txt").await,
            local_file(base_path.join("dotted.txt"))
        );
        assert_eq!(
            resolve(&app, &links, "./relative/../dotted.txt").await,
            local_file(base_path.join("relative/../dotted.txt"))
        );

        assert_eq!(
            resolve(&app, &links, "./relative").await,
            local_directory(base_path.join("relative"))
        );

        assert_eq!(
            links
                .read(&app, |links, ctx| links.resolve("missing.txt", ctx))
                .await,
            Err(ResolveError::FileNotFound)
        );
    });
}

#[test]
fn test_resolve_file_with_line() {
    App::test((), |mut app| async move {
        let base = tempdir().unwrap();
        let base_path = base.path();
        touch(base_path.join("src/main.rs")).await;
        touch(base_path.join("path/to/index.html")).await;

        let links = init_link_model(&mut app, Some(base_path));

        assert_eq!(
            resolve(&app, &links, "./src/main.rs:123").await,
            local_file_location(base_path.join("src/main.rs"), 123, None)
        );

        assert_eq!(
            resolve(&app, &links, "path/to/index.html:99:51").await,
            local_file_location(base_path.join("path/to/index.html"), 99, Some(51))
        );
    });
}

#[test]
fn test_open_markdown_file_uses_viewer_when_preferred() {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if !root.join("README.md").exists() {
        root = root.parent().unwrap().to_path_buf();
    }

    App::test((), |mut app| async move {
        let links = init_link_model(&mut app, Some(&root));

        let events = Arc::new(Mutex::new(vec![]));
        {
            let events = events.clone();
            app.update(|ctx| {
                ctx.subscribe_to_model(&links, move |_, event, _| {
                    events.lock().push(event.clone());
                })
            });
        }

        links
            .update(&mut app, |links, ctx| {
                // The `./` in the link is important: `.md` is the TLD for Moldova, so this will be
                // resolved as a web link otherwise.
                let future = links.resolve_and_open("./README.md", ctx);
                ctx.await_spawned_future(future.future_id())
            })
            .await;

        let events = events.lock();
        assert_eq!(events.len(), 1);
        match events.first() {
            Some(LinkEvent::OpenFileNotebook { path, session }) => {
                assert_eq!(path, &root.join("README.md"));
                assert!(Arc::ptr_eq(&TEST_SESSION, session));
            }
            other => panic!("Expected OpenFileNotebook event, got {other:?}"),
        }
    });
}

#[test]
fn test_open_markdown_file_respects_disabled_viewer_preference() {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if !root.join("README.md").exists() {
        root = root.parent().unwrap().to_path_buf();
    }

    App::test((), |mut app| async move {
        let links = init_link_model(&mut app, Some(&root));

        EditorSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .prefer_markdown_viewer
                .set_value(false, ctx)
                .unwrap();
        });

        let events = Arc::new(Mutex::new(vec![]));
        {
            let events = events.clone();
            app.update(|ctx| {
                ctx.subscribe_to_model(&links, move |_, event, _| {
                    events.lock().push(event.clone());
                })
            });
        }

        links
            .update(&mut app, |links, ctx| {
                let future = links.resolve_and_open("./README.md", ctx);
                ctx.await_spawned_future(future.future_id())
            })
            .await;

        let events = events.lock();
        assert_eq!(events.len(), 1);
        match events.first() {
            Some(LinkEvent::OpenFileWithTarget {
                path,
                target,
                line_col,
            }) => {
                assert_eq!(path, &root.join("README.md"));
                assert_eq!(target, &FileTarget::SystemDefault);
                assert_eq!(line_col, &None);
            }
            other => panic!("Expected OpenFileWithTarget event, got {other:?}"),
        }
    });
}
