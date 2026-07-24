//! Link-opening behavior for notebooks.
use std::borrow::Cow;
use std::fmt;
use std::future::{self, Future};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures_util::future::Either;
use url::Url;
use warp_util::path::{CleanPathResult, LineAndColumnArg};
use warpui::r#async::SpawnedFutureHandle;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity, WindowId};

use super::file::is_markdown_file;
use crate::drive::OpenWarpDriveObjectArgs;
use crate::terminal::model::session::Session;
use crate::uri::parse_url_paths::{WarpWebLink, get_item_data_from_warp_link};
#[cfg(feature = "local_fs")]
use crate::util::file::external_editor::EditorSettings;
#[cfg(feature = "local_fs")]
use crate::util::openable_file_type::{FileTarget, is_supported_image_file, resolve_file_target};
use crate::workspace::ActiveSession;

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;

/// The target of a notebook link.
#[derive(Debug, Clone)]
pub enum LinkTarget {
    Url(Url),
    LocalFile {
        path: PathBuf,
        line_and_column: Option<LineAndColumnArg>,
        /// A `#fragment` anchor to scroll to once the destination Markdown document loads.
        /// Split off the link before file resolution (so `other-file.md#section` resolves the
        /// file, not a literal `#section` path) and carried here so the destination notebook can
        /// perform a deferred scroll — the cross-document analog of `line_and_column`.
        anchor: Option<String>,
        /// The base session when the link was resolved, if there was one. Stored here in case it
        /// changes between resolving and opening the link. `None` for a link resolved in a
        /// standalone Markdown viewer tab (opened from Finder / `open -a Warp file.md`), which has
        /// a base directory — the document's own dir — but no terminal session.
        session: Option<Arc<Session>>,
        /// Whether or not this file is a Markdown file viewable in Warp.
        is_markdown: bool,
    },
    LocalDirectory {
        path: PathBuf,
    },
}

impl LinkTarget {
    /// A secondary action to show in the tooltip for this link.
    pub fn secondary_action(&self) -> Option<SecondaryAction> {
        match self {
            LinkTarget::LocalDirectory { .. } => Some(SecondaryAction {
                label: "New session".into(),
                tooltip: Some("Open a new terminal session in this directory".into()),
                accessibility_content: "Open in terminal session".into(),
            }),
            LinkTarget::LocalFile {
                is_markdown: true, ..
            } => Some(SecondaryAction {
                label: "Open in editor".into(),
                tooltip: None,
                accessibility_content: "Edit Markdown file".into(),
            }),
            LinkTarget::Url(_) | LinkTarget::LocalFile { .. } => None,
        }
    }
}

impl PartialEq for LinkTarget {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Url(my_url), Self::Url(other_url)) => my_url == other_url,
            (
                Self::LocalFile {
                    path: my_path,
                    line_and_column: my_location,
                    anchor: my_anchor,
                    session: my_session,
                    ..
                },
                Self::LocalFile {
                    path: other_path,
                    line_and_column: other_location,
                    anchor: other_anchor,
                    session: other_session,
                    ..
                },
            ) => {
                my_path == other_path
                    && my_location == other_location
                    && my_anchor == other_anchor
                    && match (my_session, other_session) {
                        (Some(my_session), Some(other_session)) => {
                            Arc::ptr_eq(my_session, other_session)
                        }
                        (None, None) => true,
                        _ => false,
                    }
            }
            (Self::LocalDirectory { path: my_path }, Self::LocalDirectory { path: other_path }) => {
                my_path == other_path
            }
            _ => false,
        }
    }
}

impl fmt::Display for LinkTarget {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            LinkTarget::Url(url) => url.fmt(f),
            LinkTarget::LocalFile { path, .. } => path.display().fmt(f),
            LinkTarget::LocalDirectory { path, .. } => path.display().fmt(f),
        }
    }
}

/// Split a trailing `#fragment` anchor off a scheme-less link target.
///
/// Returns the path portion and the decoded anchor (if any). A `#L<digits>[:<digits>]`
/// line-number suffix is intentionally *not* treated as an anchor — it is left on the path so the
/// downstream `CleanPathResult` line/column parser continues to handle it. An empty fragment
/// (`file.md#`) yields no anchor. Only the final `#` is split, so an earlier `#` in the path
/// (e.g. `weird#name.md`) is preserved.
fn split_anchor_fragment(link: &str) -> (&str, Option<String>) {
    let Some((path, fragment)) = link.rsplit_once('#') else {
        return (link, None);
    };

    // Preserve `#L100` / `#L100:200` line-number routing: these are stripped by
    // `CleanPathResult`, so they must stay attached to the path, not be peeled as an anchor.
    if is_line_number_fragment(fragment) {
        return (link, None);
    }

    if fragment.is_empty() {
        return (path, None);
    }

    let anchor = urlencoding::decode(fragment)
        .map(|decoded| decoded.into_owned())
        .unwrap_or_else(|_| fragment.to_owned());
    (path, Some(anchor))
}

/// Whether a fragment is a `#L<digits>` or `#L<digits>:<digits>` line-number suffix (handled by
/// `CleanPathResult`), rather than a document anchor.
fn is_line_number_fragment(fragment: &str) -> bool {
    let Some(rest) = fragment.strip_prefix('L') else {
        return false;
    };
    match rest.split_once(':') {
        Some((line, col)) => {
            !line.is_empty()
                && line.bytes().all(|b| b.is_ascii_digit())
                && !col.is_empty()
                && col.bytes().all(|b| b.is_ascii_digit())
        }
        None => !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()),
    }
}

/// Model for resolving and opening links in a notebook, taking into account their context (for
/// example, resolving relative file paths).
pub struct NotebookLinks {
    session_source: SessionSource,
}

impl NotebookLinks {
    pub fn new(session_source: SessionSource, ctx: &mut ModelContext<Self>) -> Self {
        ctx.observe(
            &ActiveSession::handle(ctx),
            Self::handle_active_session_change,
        );

        Self { session_source }
    }

    /// Resolve a link target. If the link is a valid URL or starts with a potential domain name,
    /// it's treated as an URL. Otherwise, it's treated as a local file path, possibly with a line
    /// and column number. This returns `None` if the link is known to be invalid (for example, it
    /// resolves to a nonexistent file path).
    pub fn resolve(
        &self,
        link: &str,
        ctx: &AppContext,
    ) -> impl Future<Output = Result<LinkTarget, ResolveError>> + use<> {
        if let Ok(url) = Url::parse(link) {
            // The `url` crate only provides `to_file_path` on certain platforms.
            #[cfg(feature = "local_fs")]
            if url.scheme() == "file" {
                // Unlike below, if there's missing information, we can still fall back to the
                // system for file:// URL handling.
                if let Some(session) = self.session_source.session(ctx)
                    && let Ok(file) = url.to_file_path()
                {
                    // TODO(ben): Support line and column in file:// URLs.
                    return Either::Left(Self::resolve_file(file, Some(session), None, None));
                }
            }

            return Either::Right(future::ready(Ok(LinkTarget::Url(url))));
        }

        // Peel a trailing `#fragment` off before any file resolution, so a cross-document link
        // like `other-file.md#section` resolves the file (not a literal `#section` path) and the
        // fragment rides along on the resolved target for a deferred scroll. A `#L100`
        // line-number suffix is deliberately left in place — it is handled downstream by
        // `CleanPathResult` and must not be mistaken for an anchor. (A bare `#fragment` with no
        // path never reaches here: it is intercepted earlier by the viewer's `maybe_open_url`
        // `#`-branch.)
        let (link, anchor) = split_anchor_fragment(link);

        // REPAIR: a bare `file.md` (no `./`, no `/`) is classified as a valid public domain by
        // the heuristic below, because `.md` (Moldova), `.dev`, `.com`, … are known suffixes.
        // That misroutes an existing local Markdown file to the browser. So before applying the
        // domain heuristic, if the scheme-less target resolves to an existing file relative to
        // the base directory, treat it as a file. A genuine bare domain with no matching local
        // file (e.g. `warp.dev`) still falls through to the browser.
        #[cfg(feature = "local_fs")]
        let resolves_to_local_file = {
            let session = self.session_source.session(ctx);
            match self.session_source.base_directory(ctx) {
                Some(base_directory) => {
                    let clean_path = CleanPathResult::with_line_and_column_number(link);
                    crate::util::file::absolute_path_if_valid(
                        &clean_path,
                        crate::util::file::ShellPathType::PlatformNative(
                            base_directory.to_path_buf(),
                        ),
                        session.as_ref().and_then(|s| s.launch_data()),
                    )
                    .is_some()
                }
                None => false,
            }
        };
        #[cfg(not(feature = "local_fs"))]
        let resolves_to_local_file = false;

        if !resolves_to_local_file {
            // If parsing failed, see if this is a web URL without a scheme.
            // The heuristic we use is to take the substring up to the first slash (if present), and
            // check for a valid public domain name or IP address.
            let maybe_domain = link.split_once('/').map_or(link, |(start, _)| start);
            if (addr::parse_domain_name(maybe_domain)
                .is_ok_and(|domain| domain.has_known_suffix() && domain.root().is_some())
                || maybe_domain.parse::<IpAddr>().is_ok())
                && let Ok(url) = Url::parse(&format!("http://{link}"))
            {
                return Either::Right(future::ready(Ok(LinkTarget::Url(url))));
            }
        }

        // At this point, we can only resolve file targets. These are normally resolved against a
        // session's context, but a standalone Markdown viewer tab has no session — only a base
        // directory (the document's own dir). The second arm below handles that case so relative
        // links still resolve there.
        match self.session_source.session(ctx) {
            Some(session) if session.launch_data().is_some() => {
                let launch_data = session
                    .launch_data()
                    .expect("Session launch data should exist");
                let clean_path = CleanPathResult::with_line_and_column_number(link);
                let path = match self.session_source.base_directory(ctx) {
                    Some(base_directory) => {
                        cfg_if::cfg_if! {
                            if #[cfg(feature = "local_fs")] {
                                let Some(path) = crate::util::file::absolute_path_if_valid(
                                    &clean_path,
                                    crate::util::file::ShellPathType::PlatformNative(base_directory.to_path_buf()),
                                    Some(launch_data),
                                ) else {
                                    return Either::Right(future::ready(Err(ResolveError::FileNotFound)));
                                };
                                path
                            } else {
                                // If we don't have a local filesystem, we append the path naively.
                                base_directory.join(clean_path.path)
                            }
                        }
                    }
                    None => {
                        let Some(path) = launch_data.maybe_convert_absolute_path(&clean_path.path)
                        else {
                            return Either::Right(future::ready(Err(ResolveError::MissingContext)));
                        };
                        // To open a relative path, we must have a base directory. Otherwise, we don't know for
                        // sure how the path will be resolved.
                        if path.is_relative() {
                            return Either::Right(future::ready(Err(ResolveError::MissingContext)));
                        }
                        path
                    }
                };

                Either::Left(Self::resolve_file(
                    path,
                    Some(session),
                    clean_path.line_and_column_num,
                    anchor,
                ))
            }
            session => {
                // Either a session without launch data, or no session at all (a standalone
                // Markdown viewer tab). Both resolve a relative path against the base directory,
                // which must be present — the document's own directory supplies it in the
                // no-session case.
                let clean_path_result = CleanPathResult::with_line_and_column_number(link);
                let clean_path = Path::new(&clean_path_result.path);
                let path = if clean_path.is_relative() {
                    // To open a relative path, we must have a base directory. Otherwise, we don't know for
                    // sure how the path will be resolved.
                    match self.session_source.base_directory(ctx) {
                        Some(directory) => directory.join(clean_path),
                        None => {
                            return Either::Right(future::ready(Err(ResolveError::MissingContext)));
                        }
                    }
                } else {
                    clean_path.to_path_buf()
                };

                Either::Left(Self::resolve_file(
                    path,
                    session,
                    clean_path_result.line_and_column_num,
                    anchor,
                ))
            }
        }
    }

    /// Resolve a file path into a [`LinkTarget`], checking if it exists. `session` is `None` for a
    /// standalone Markdown viewer tab, which resolves relative links against the document's own
    /// directory without any terminal session.
    async fn resolve_file(
        path: PathBuf,
        session: Option<Arc<Session>>,
        line_and_column: Option<LineAndColumnArg>,
        anchor: Option<String>,
    ) -> Result<LinkTarget, ResolveError> {
        let metadata = async_fs::metadata(&path).await?;
        Ok(if metadata.is_dir() {
            // Discard line/column and anchor information, which don't make sense for a directory.
            LinkTarget::LocalDirectory { path }
        } else {
            LinkTarget::LocalFile {
                is_markdown: is_markdown_file(&path),
                path,
                line_and_column,
                anchor,
                session,
            }
        })
    }

    /// Open a resolved link:
    /// * URLs are opened in the web browser or system-default application.
    /// * Markdown files are opened according to the user's Markdown Viewer preference.
    /// * Other files are opened in the configured editor or system-default application.
    pub fn open(&self, link: LinkTarget, ctx: &mut ModelContext<Self>) {
        match link {
            LinkTarget::Url(url) => {
                if let Some(WarpWebLink::DriveObject(args)) = get_item_data_from_warp_link(&url) {
                    return ctx.emit(LinkEvent::OpenWarpDriveLink {
                        open_warp_drive_args: *args,
                    });
                }

                ctx.open_url(url.as_str())
            }
            LinkTarget::LocalFile {
                path,
                line_and_column,
                anchor,
                session,
                is_markdown: true,
            } => {
                #[cfg(not(feature = "local_fs"))]
                let _ = line_and_column;

                #[cfg(feature = "local_fs")]
                {
                    let settings = EditorSettings::as_ref(ctx);
                    if *settings.prefer_markdown_viewer {
                        ctx.emit(LinkEvent::OpenFileNotebook {
                            path,
                            session,
                            anchor,
                        });
                    } else {
                        // The external editor / code viewer has no heading-slug concept, so the
                        // anchor is dropped here per the product non-goal: the file opens
                        // unscrolled.
                        let _ = anchor;
                        open_file(path, line_and_column, ctx);
                    }
                }

                #[cfg(not(feature = "local_fs"))]
                {
                    ctx.emit(LinkEvent::OpenFileNotebook {
                        path,
                        session,
                        anchor,
                    });
                }
            }
            LinkTarget::LocalFile {
                path,
                line_and_column,
                ..
            } => open_file(path, line_and_column, ctx),
            LinkTarget::LocalDirectory { path, .. } => ctx.open_file_path(&path),
        }
    }

    /// Perform the secondary action for this link.
    pub fn secondary_action(&self, link: &LinkTarget, ctx: &mut ModelContext<Self>) {
        match link {
            LinkTarget::LocalDirectory { path } => {
                ctx.emit(LinkEvent::StartLocalSession { path: path.clone() })
            }
            LinkTarget::LocalFile {
                path,
                line_and_column,
                is_markdown: true,
                ..
            } => {
                // The default action for Markdown file links is to open them in Warp. As a
                // secondary action, open them in an external app.
                open_file(path.clone(), *line_and_column, ctx)
            }
            _ => (),
        }
    }

    /// Asynchronously resolve and open a link.
    pub fn resolve_and_open(
        &self,
        link: &str,
        ctx: &mut ModelContext<Self>,
    ) -> SpawnedFutureHandle {
        ctx.spawn(self.resolve(link, ctx), |me, resolved, ctx| {
            if let Ok(link) = resolved {
                me.open(link, ctx);
            }
        })
    }

    pub fn set_session_source(&mut self, source: SessionSource, ctx: &mut ModelContext<Self>) {
        self.session_source = source;
        ctx.emit(LinkEvent::RefreshLinks);
    }

    /// Listen for session changes that might invalidate resolved links.
    fn handle_active_session_change(
        &mut self,
        _handle: ModelHandle<ActiveSession>,
        ctx: &mut ModelContext<Self>,
    ) {
        // Re-resolve links against the new session info, especially if the working directory
        // changed.
        if matches!(self.session_source, SessionSource::Active { .. }) {
            ctx.emit(LinkEvent::RefreshLinks);
        }
    }
}

/// Open a file respecting user's editor settings.
///
/// For targets that would be handed to the OS default handler (`SystemGeneric` /
/// `SystemDefault`), we reveal the file in Finder / Explorer instead of opening it.
/// This prevents a malicious markdown link from triggering arbitrary code execution
/// via an executable disguised as a local file (e.g. an extensionless shell script).
// The `line_and_column` argument is unused when there is no local filesystem.
#[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
fn open_file(
    path: PathBuf,
    line_and_column: Option<LineAndColumnArg>,
    ctx: &mut ModelContext<NotebookLinks>,
) {
    #[cfg(feature = "local_fs")]
    {
        // Images are safe to open with the system default viewer.
        if is_supported_image_file(&path) {
            ctx.emit(LinkEvent::OpenFileWithTarget {
                path,
                target: FileTarget::SystemGeneric,
                line_col: line_and_column,
            });
            return;
        }

        let settings = EditorSettings::as_ref(ctx);
        let target = resolve_file_target(&path, settings, None);
        match target {
            // Safe targets: open in a viewer/editor that won't execute the file.
            FileTarget::MarkdownViewer(_)
            | FileTarget::CodeEditor(_)
            | FileTarget::ExternalEditor(_)
            | FileTarget::EnvEditor => {
                ctx.emit(LinkEvent::OpenFileWithTarget {
                    path,
                    target,
                    line_col: line_and_column,
                });
            }
            // Dangerous targets: the OS default handler could execute the file.
            // Reveal in Finder / Explorer instead.
            FileTarget::SystemGeneric | FileTarget::SystemDefault => {
                ctx.open_file_path_in_explorer(&path);
            }
        }
    }
    #[cfg(not(feature = "local_fs"))]
    ctx.open_file_path(&path);
}

impl Entity for NotebookLinks {
    type Event = LinkEvent;
}

/// An error resolving a file link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// The target file does not exist.
    FileNotFound,
    /// The context needed to resolve a file is missing.
    MissingContext,
    Unknown,
}

impl From<std::io::Error> for ResolveError {
    fn from(err: std::io::Error) -> Self {
        if err.kind() == std::io::ErrorKind::NotFound {
            ResolveError::FileNotFound
        } else {
            ResolveError::Unknown
        }
    }
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ResolveError::FileNotFound => f.write_str("File not found"),
            ResolveError::MissingContext => f.write_str("No base directory"),
            ResolveError::Unknown => f.write_str("Broken file link"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum LinkEvent {
    /// Emitted when the view should open a Markdown file as a notebook.
    OpenFileNotebook {
        path: PathBuf,
        /// The session the link was resolved from, if any. `None` for a link opened in a
        /// standalone Markdown viewer tab, which has no terminal session.
        session: Option<Arc<Session>>,
        /// A `#fragment` anchor to scroll to once the destination notebook loads, if the link
        /// carried one (`other-file.md#section`). `None` for a plain file link.
        anchor: Option<String>,
    },
    OpenWarpDriveLink {
        open_warp_drive_args: OpenWarpDriveObjectArgs,
    },
    /// This event tells the parent pane group to open a new terminal session in the given
    /// directory.
    StartLocalSession { path: PathBuf },
    /// Signal to views that they should re-resolve links because the backing context for
    /// resolution has changed.
    RefreshLinks,
    #[cfg(feature = "local_fs")]
    /// Emitted when a file should be opened in Warp (code editor or markdown viewer).
    OpenFileWithTarget {
        path: PathBuf,
        target: FileTarget,
        line_col: Option<LineAndColumnArg>,
    },
}

/// A secondary action for a link, besides opening it.
#[derive(Debug, Clone)]
pub struct SecondaryAction {
    pub label: Cow<'static, str>,
    pub tooltip: Option<Cow<'static, str>>,
    pub accessibility_content: Cow<'static, str>,
}

/// Source for the [`Session`] and working directory to use when opening Markdown files as notebooks.
pub enum SessionSource {
    /// Use the specific target session and directory.
    Target {
        session: Arc<Session>,
        base_directory: PathBuf,
    },
    /// Use the window's active session and working directory.
    Active {
        window_id: WindowId,
        /// The open document's own parent directory, used as the base-directory fallback when the
        /// window's active session has no local working directory. Without this, a Markdown
        /// document opened in a standalone viewer (e.g. `open -a Warp file.md`, which has no
        /// terminal session cwd) can't resolve its own relative links — the document knows where
        /// it lives even when the window doesn't. The active session's cwd still takes precedence
        /// when present, preserving existing behavior for notebooks opened inside a session.
        document_dir: Option<PathBuf>,
    },
}

impl SessionSource {
    /// Use the window's active session, with no document-directory fallback. For surfaces that
    /// are not backed by a file on disk (comment editors, AI documents, in-memory notebooks).
    pub fn active(window_id: WindowId) -> Self {
        SessionSource::Active {
            window_id,
            document_dir: None,
        }
    }

    /// Use the window's active session, falling back to the given document directory when the
    /// active session has no local working directory. For file-backed Markdown notebooks.
    pub fn active_for_document(window_id: WindowId, document_dir: Option<PathBuf>) -> Self {
        SessionSource::Active {
            window_id,
            document_dir,
        }
    }

    fn session(&self, ctx: &AppContext) -> Option<Arc<Session>> {
        match self {
            SessionSource::Target { session, .. } => Some(session.clone()),
            SessionSource::Active { window_id, .. } => {
                ActiveSession::as_ref(ctx).session(*window_id)
            }
        }
    }

    fn base_directory<'a>(&'a self, ctx: &'a AppContext) -> Option<&'a Path> {
        match self {
            SessionSource::Target { base_directory, .. } => Some(base_directory.as_path()),
            SessionSource::Active {
                window_id,
                document_dir,
            } => ActiveSession::as_ref(ctx)
                .path_if_local(*window_id)
                .or(document_dir.as_deref()),
        }
    }
}
