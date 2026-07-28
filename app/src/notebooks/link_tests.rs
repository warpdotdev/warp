use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lazy_static::lazy_static;
use parking_lot::Mutex;
use settings::Setting as _;
use tempfile::tempdir;
use url::Url;
use warp_util::path::LineAndColumnArg;
use warpui::{App, ModelHandle, SingletonEntity, WindowId};

use super::{split_anchor_fragment, LinkTarget, NotebookLinks, ResolveError, SessionSource};
use crate::notebooks::file::is_markdown_file;
use crate::notebooks::link::LinkEvent;
use crate::terminal::model::session::Session;
use crate::terminal::shell::ShellType;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::util::file::external_editor::EditorSettings;
use crate::util::openable_file_type::FileTarget;
use crate::workspace::ActiveSession;

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
        anchor: None,
        session: Some(TEST_SESSION.clone()),
    }
}

/// Like [`local_file`], but for the standalone-viewer case where the link was resolved without a
/// terminal session (see [`init_link_model_no_session`]).
fn local_file_no_session(path: impl Into<PathBuf>) -> LinkTarget {
    let path = path.into();
    LinkTarget::LocalFile {
        is_markdown: is_markdown_file(&path),
        path,
        line_and_column: None,
        anchor: None,
        session: None,
    }
}

fn local_file_anchor(path: impl Into<PathBuf>, anchor: &str) -> LinkTarget {
    let path = path.into();
    LinkTarget::LocalFile {
        is_markdown: is_markdown_file(&path),
        path,
        line_and_column: None,
        anchor: Some(anchor.to_owned()),
        session: Some(TEST_SESSION.clone()),
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
        anchor: None,
        session: Some(TEST_SESSION.clone()),
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
        None => SessionSource::active(window_id),
    };
    app.add_singleton_model(|ctx| {
        let mut session = ActiveSession::default();
        session.set_session_for_test(window_id, TEST_SESSION.clone(), base_directory, None, ctx);
        session
    });
    app.add_model(|ctx| NotebookLinks::new(source, ctx))
}

/// Initialize the link resolver for a *standalone Markdown viewer* tab: a file-backed notebook
/// whose window has no active terminal session, so the only base-directory context is the
/// document's own directory (`SessionSource::active_for_document`). This mirrors the real GUI
/// condition of opening a `.md` from Finder / `open -a Warp file.md`, which
/// [`init_link_model`] cannot reproduce because it always installs a `TEST_SESSION`.
fn init_link_model_no_session(app: &mut App, document_dir: &Path) -> ModelHandle<NotebookLinks> {
    initialize_settings_for_tests(app);

    let window_id = WindowId::new();
    // Register the window with ActiveSession, but with *no* session and no working directory, so
    // `session(window_id)` and `path_if_local(window_id)` both return `None` — the standalone
    // viewer case. The document directory is the sole base-directory source.
    app.add_singleton_model(|_ctx| ActiveSession::default());
    let source = SessionSource::active_for_document(window_id, Some(document_dir.to_owned()));
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
    if let Some(parent) = path.parent()
        && let Err(err) = async_fs::create_dir_all(parent).await
        && err.kind() != ErrorKind::AlreadyExists
    {
        panic!("Creating parent {} failed: {}", parent.display(), err);
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
fn test_split_anchor_fragment_pure() {
    // No fragment.
    assert_eq!(
        split_anchor_fragment("other-file.md"),
        ("other-file.md", None)
    );
    // Simple fragment.
    assert_eq!(
        split_anchor_fragment("other-file.md#section"),
        ("other-file.md", Some("section".to_owned()))
    );
    // Dot-slash prefix preserved on the path side.
    assert_eq!(
        split_anchor_fragment("./other-file.md#section"),
        ("./other-file.md", Some("section".to_owned()))
    );
    // Only the final `#` splits; an earlier `#` stays in the path.
    assert_eq!(
        split_anchor_fragment("weird#name.md#frag"),
        ("weird#name.md", Some("frag".to_owned()))
    );
    // Empty fragment yields no anchor.
    assert_eq!(
        split_anchor_fragment("other-file.md#"),
        ("other-file.md", None)
    );
    // URL-encoded fragments are decoded.
    assert_eq!(
        split_anchor_fragment("doc.md#caf%C3%A9"),
        ("doc.md", Some("café".to_owned()))
    );
    // `#L<digits>` line-number suffixes are NOT peeled as anchors — left for CleanPathResult.
    assert_eq!(
        split_anchor_fragment("main.rs#L100"),
        ("main.rs#L100", None)
    );
    assert_eq!(
        split_anchor_fragment("main.rs#L100:50"),
        ("main.rs#L100:50", None)
    );
    // A fragment that merely starts with `L` but isn't a line number is a real anchor.
    assert_eq!(
        split_anchor_fragment("doc.md#License"),
        ("doc.md", Some("License".to_owned()))
    );
    assert_eq!(
        split_anchor_fragment("doc.md#L10x"),
        ("doc.md", Some("L10x".to_owned()))
    );
}

#[test]
fn test_bare_markdown_file_prefers_local_file_over_cctld() {
    // Regression test for the `.md`-is-Moldova ccTLD collision. A bare `README.md` (no `./`
    // prefix, no slash) is classified as a valid public domain by the bare-domain heuristic
    // (`.md` has a known suffix and `README` is a root), so before this fix it misrouted to the
    // browser. When such a target resolves to an existing local file relative to the base
    // directory, it must be opened as a file, not a URL.
    App::test((), |mut app| async move {
        let base = tempdir().unwrap();
        let base_path = base.path();
        touch(base_path.join("README.md")).await;
        touch(base_path.join("notes.md")).await;
        touch(base_path.join("docs/guide.md")).await;
        let links = init_link_model(&mut app, Some(base_path));

        // Single-segment `file.md` targets that exist on disk resolve as files, not URLs — the
        // shape the ccTLD heuristic misroutes (the heuristic takes the substring up to the first
        // `/`, so only a slash-free name can be mistaken for a domain).
        assert_eq!(
            resolve(&app, &links, "README.md").await,
            local_file(base_path.join("README.md"))
        );
        assert_eq!(
            resolve(&app, &links, "notes.md").await,
            local_file(base_path.join("notes.md"))
        );

        // Guard: multi-segment `.md` paths were already safe (any `/` makes the pre-`/` substring
        // a non-suffix string, so the heuristic can't fire) — assert the repair's file-first
        // ordering doesn't regress them.
        assert_eq!(
            resolve(&app, &links, "docs/guide.md").await,
            local_file(base_path.join("docs/guide.md"))
        );
        assert_eq!(
            resolve(&app, &links, "./docs/guide.md").await,
            local_file(base_path.join("docs/guide.md"))
        );

        // A genuine bare domain that does NOT resolve to a local file still opens the browser.
        assert_eq!(
            resolve(&app, &links, "warp.dev").await,
            url("http://warp.dev")
        );
        // A bare `.md` target with no matching local file also falls through to the browser,
        // preserving the domain heuristic where there's nothing to shadow it.
        assert_eq!(
            resolve(&app, &links, "nonexistent.md").await,
            url("http://nonexistent.md")
        );
    });
}

#[test]
fn test_split_fragment_before_file_resolution() {
    // A cross-document link `other-file.md#section` must have its `#section` fragment split off
    // before file resolution, so the file part resolves on disk and the fragment rides along on
    // the resolved target for a deferred scroll. Before this fix, the literal `#section` was
    // included in the stat and the file was never found.
    App::test((), |mut app| async move {
        let base = tempdir().unwrap();
        let base_path = base.path();
        touch(base_path.join("other-file.md")).await;
        touch(base_path.join("multi#hash.md")).await;
        let links = init_link_model(&mut app, Some(base_path));

        // Bare `file.md#section` resolves to the file with the anchor attached.
        assert_eq!(
            resolve(&app, &links, "other-file.md#section").await,
            local_file_anchor(base_path.join("other-file.md"), "section")
        );
        // `./`-prefixed form (which dodges the ccTLD heuristic) also splits the fragment.
        assert_eq!(
            resolve(&app, &links, "./other-file.md#section").await,
            local_file_anchor(base_path.join("other-file.md"), "section")
        );
        // No fragment → no anchor.
        assert_eq!(
            resolve(&app, &links, "./other-file.md").await,
            local_file(base_path.join("other-file.md"))
        );
        // Only the final `#…` is treated as the fragment; earlier `#` stays in the path.
        assert_eq!(
            resolve(&app, &links, "./multi#hash.md#frag").await,
            local_file_anchor(base_path.join("multi#hash.md"), "frag")
        );
        // A trailing empty fragment (`file.md#`) resolves the file with no anchor.
        assert_eq!(
            resolve(&app, &links, "./other-file.md#").await,
            local_file(base_path.join("other-file.md"))
        );
    });
}

#[test]
fn test_fragment_split_preserves_line_number_routing() {
    // The `#L100` line-number suffix must continue to route through the existing line/column
    // path (handled by `CleanPathResult`), not be peeled as an anchor fragment.
    App::test((), |mut app| async move {
        let base = tempdir().unwrap();
        let base_path = base.path();
        touch(base_path.join("src/main.rs")).await;
        let links = init_link_model(&mut app, Some(base_path));

        assert_eq!(
            resolve(&app, &links, "./src/main.rs#L100").await,
            local_file_location(base_path.join("src/main.rs"), 100, None)
        );
        assert_eq!(
            resolve(&app, &links, "./src/main.rs#L100:50").await,
            local_file_location(base_path.join("src/main.rs"), 100, Some(50))
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
fn test_open_extensionless_non_text_file_does_not_emit_open_event() {
    // Regression test: an extensionless file (e.g. a disguised executable) is classified as
    // binary by `is_file_openable_in_warp`, which previously routed it to `SystemGeneric` and
    // ultimately `NSWorkspace.openURL` — allowing arbitrary code execution. After the fix,
    // such files are revealed in Finder / Explorer instead of opened, so no `OpenFileWithTarget`
    // event should be emitted.
    App::test((), |mut app| async move {
        let base = tempdir().unwrap();
        let base_path = base.path();
        let malicious_path = base_path.join("abc");
        touch(&malicious_path).await;
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
            links.open(local_file(&malicious_path), ctx);
        });

        let events = events.lock();
        assert!(
            events.is_empty(),
            "Expected no LinkEvent to be emitted for an extensionless non-text file, \
             but got: {events:?}"
        );
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
                // With the ccTLD repair, a bare `README.md` that exists on disk resolves as a
                // file rather than the `.md` Moldova domain, so no `./` prefix is needed here.
                let future = links.resolve_and_open("README.md", ctx);
                ctx.await_spawned_future(future.future_id())
            })
            .await;

        let events = events.lock();
        assert_eq!(events.len(), 1);
        match events.first() {
            Some(LinkEvent::OpenFileNotebook {
                path,
                session,
                anchor,
            }) => {
                assert_eq!(path, &root.join("README.md"));
                assert!(session
                    .as_ref()
                    .is_some_and(|s| Arc::ptr_eq(&TEST_SESSION, s)));
                assert_eq!(anchor, &None);
            }
            other => panic!("Expected OpenFileNotebook event, got {other:?}"),
        }
    });
}

#[test]
fn test_cross_document_fragment_threads_anchor_to_open_event() {
    // A `file.md#section` cross-document link must carry its `section` anchor all the way to the
    // emitted `OpenFileNotebook` event, so the destination notebook can perform a deferred
    // scroll to the heading once it loads.
    App::test((), |mut app| async move {
        let base = tempdir().unwrap();
        let base_path = base.path();
        touch(base_path.join("other-file.md")).await;
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

        links
            .update(&mut app, |links, ctx| {
                let future = links.resolve_and_open("other-file.md#target-section", ctx);
                ctx.await_spawned_future(future.future_id())
            })
            .await;

        let events = events.lock();
        assert_eq!(events.len(), 1);
        match events.first() {
            Some(LinkEvent::OpenFileNotebook {
                path,
                session,
                anchor,
            }) => {
                assert_eq!(path, &base_path.join("other-file.md"));
                assert!(session
                    .as_ref()
                    .is_some_and(|s| Arc::ptr_eq(&TEST_SESSION, s)));
                assert_eq!(anchor.as_deref(), Some("target-section"));
            }
            other => panic!("Expected OpenFileNotebook event, got {other:?}"),
        }
    });
}

#[test]
fn test_cross_document_link_resolves_without_session() {
    // Regression (#13725): a standalone Markdown viewer tab — opened from Finder / `open -a Warp
    // file.md` — has a valid base directory (the document's own dir) but *no* terminal session.
    // Resolving a cross-document file link must still succeed against that base directory. Before
    // the fix, `resolve` fell through to its session `match` and returned `Err(MissingContext)`
    // for the no-session case even though the file existed, so every relative/cross-document link
    // was a silent no-op in the standalone viewer while in-document `#fragment` links (which never
    // touch the resolver) kept working.
    App::test((), |mut app| async move {
        let base = tempdir().unwrap();
        let base_path = base.path();
        touch(base_path.join("other-file.md")).await;
        let links = init_link_model_no_session(&mut app, base_path);

        let resolved = links
            .read(&app, |links, ctx| links.resolve("other-file.md", ctx))
            .await
            .expect("cross-document link should resolve without a session");
        assert_eq!(
            resolved,
            local_file_no_session(base_path.join("other-file.md"))
        );
    });
}

#[test]
fn test_cross_document_link_opens_in_viewer_without_session() {
    // The end-to-end standalone-viewer path: `resolve_and_open` must emit `OpenFileNotebook` (with
    // no session) so the workspace opens the target document. This is the seam the live GUI failure
    // exercised — `resolve_and_open` silently drops a resolve `Err`, so a session-gated failure
    // produced no event and no visible action.
    App::test((), |mut app| async move {
        let base = tempdir().unwrap();
        let base_path = base.path();
        touch(base_path.join("other-file.md")).await;
        let links = init_link_model_no_session(&mut app, base_path);

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
                let future = links.resolve_and_open("other-file.md#far-section", ctx);
                ctx.await_spawned_future(future.future_id())
            })
            .await;

        let events = events.lock();
        assert_eq!(events.len(), 1);
        match events.first() {
            Some(LinkEvent::OpenFileNotebook {
                path,
                session,
                anchor,
            }) => {
                assert_eq!(path, &base_path.join("other-file.md"));
                assert!(session.is_none());
                assert_eq!(anchor.as_deref(), Some("far-section"));
            }
            other => panic!("Expected OpenFileNotebook event, got {other:?}"),
        }
    });
}

#[test]
fn test_open_markdown_file_respects_disabled_viewer_preference() {
    // With `prefer_markdown_viewer = false`, the markdown file would otherwise
    // resolve to `FileTarget::SystemDefault`. The security fix in #25353 routes
    // both `SystemDefault` and `SystemGeneric` through
    // `open_file_path_in_explorer`, so no `OpenFileWithTarget` event is emitted
    // — the file is revealed in Finder / Explorer instead.
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
        assert!(
            events.is_empty(),
            "Expected no LinkEvent when markdown viewer is disabled (file is \
             revealed in explorer instead), but got: {events:?}"
        );
    });
}
