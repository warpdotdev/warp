use std::ops::Deref;

use serde::{Serialize, Serializer};
use warpui::platform::Cursor;
use warpui::ViewContext;

use crate::send_telemetry_from_ctx;
use crate::server::telemetry::{LinkOpenMethod, TelemetryEvent};
use crate::terminal::model::grid::grid_handler::Link;
use crate::terminal::model::index::Point;
use crate::terminal::model::terminal_model::{WithinBlock, WithinModel};
use crate::terminal::model::RespectObfuscatedSecrets;
use crate::terminal::TerminalModel;

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use crate::{
            code::buffer_location::LocalOrRemotePath,
            terminal::model::grid::grid_handler,
            terminal::model::session::{SessionId, SessionType},
            terminal::ShellLaunchData,
            util::file::{FileLink, absolute_path_if_valid, ShellPathType},
            util::openable_file_type::FileTarget,
        };
        use std::path::PathBuf;
        use warp_util::remote_path::RemotePath;
        use warp_util::standardized_path::StandardizedPath;
        use warp_util::path::CleanPathResult;
        use warp_util::path::LineAndColumnArg;
    }
}

use super::{FindLinkArg, TerminalEditor};

// "a/" and "b/" are prefixes specific to Git Diff
#[cfg(feature = "local_fs")]
const PREFIXES_TO_REMOVE: [&str; 2] = ["a/", "b/"];

/// "@" is a suffix that can be added to symlinks. It appears in Git Bash's default configuration
/// for `ls`.
#[cfg(feature = "local_fs")]
const SUFFIXES_TO_REMOVE: [&str; 1] = ["@"];

#[cfg(feature = "local_fs")]
const MARKDOWN_EXTENSIONS: [&str; 2] = ["md", "markdown"];

#[cfg(feature = "local_fs")]
const MARKDOWN_FILE_NAMES: [&str; 3] = ["README", "CHANGELOG", "LICENSE"];

#[cfg(feature = "local_fs")]
#[derive(Clone)]
enum FileLinkScanContext {
    Local {
        working_directory: String,
        shell_launch_data: Option<ShellLaunchData>,
    },
    RemoteMarkdown {
        working_directory: String,
        host_id: warp_core::HostId,
    },
}

#[cfg(feature = "local_fs")]
fn is_standardized_markdown_file(path: &StandardizedPath) -> bool {
    match path.extension() {
        Some(extension) => MARKDOWN_EXTENSIONS
            .iter()
            .any(|markdown_extension| extension.eq_ignore_ascii_case(markdown_extension)),
        None => path.file_name().is_some_and(|file_name| {
            MARKDOWN_FILE_NAMES
                .iter()
                .any(|markdown_file_name| file_name.eq_ignore_ascii_case(markdown_file_name))
        }),
    }
}

#[cfg(feature = "local_fs")]
fn remote_markdown_location(
    clean_path_result: &CleanPathResult,
    working_directory: &str,
    host_id: &warp_core::HostId,
) -> Option<LocalOrRemotePath> {
    let standardized = StandardizedPath::try_new(&clean_path_result.path)
        .or_else(|_| {
            StandardizedPath::try_new(working_directory)
                .map(|working_directory| working_directory.join(&clean_path_result.path))
        })
        .ok()?;
    if !is_standardized_markdown_file(&standardized) {
        return None;
    }

    Some(LocalOrRemotePath::Remote(RemotePath::new(
        host_id.clone(),
        standardized,
    )))
}

/// Highlighted link within a terminal model grid.
#[derive(Debug, Clone)]
pub enum GridHighlightedLink {
    Url(WithinModel<Link>),
    #[cfg(feature = "local_fs")]
    File(WithinModel<FileLink>),
}

impl GridHighlightedLink {
    pub fn contains(&self, position: &WithinModel<Point>) -> bool {
        match self {
            GridHighlightedLink::Url(url) => url.contains(position),
            #[cfg(feature = "local_fs")]
            GridHighlightedLink::File(file_link) => file_link.contains(position),
        }
    }

    pub fn tooltip_text(&self) -> &'static str {
        match &self {
            #[cfg(feature = "local_fs")]
            GridHighlightedLink::File(file_link)
                if file_link
                    .get_inner()
                    .absolute_path()
                    .map(|path| path.is_dir())
                    .unwrap_or(false) =>
            {
                "Open folder"
            }
            #[cfg(feature = "local_fs")]
            GridHighlightedLink::File(_) => "Open file",
            GridHighlightedLink::Url(_) => "Open link",
        }
    }
}

impl Serialize for GridHighlightedLink {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match &self {
            GridHighlightedLink::Url(_) => {
                serializer.serialize_unit_variant("HighlightedLink", 0, "Url")
            }
            #[cfg(feature = "local_fs")]
            GridHighlightedLink::File(_) => {
                serializer.serialize_unit_variant("HighlightedLink", 1, "File")
            }
        }
    }
}

impl TryFrom<GridHighlightedLink> for Link {
    type Error = anyhow::Error;

    fn try_from(value: GridHighlightedLink) -> Result<Self, Self::Error> {
        match value {
            GridHighlightedLink::Url(WithinModel::AltScreen(url)) => Ok(url),
            #[cfg(feature = "local_fs")]
            GridHighlightedLink::File(WithinModel::AltScreen(file_link)) => Ok(file_link.link),
            _ => Err(anyhow::anyhow!(
                "HighlightedLink is not within the alt screen"
            )),
        }
    }
}

impl TryFrom<GridHighlightedLink> for WithinBlock<Link> {
    type Error = anyhow::Error;

    fn try_from(value: GridHighlightedLink) -> Result<Self, Self::Error> {
        match value {
            GridHighlightedLink::Url(WithinModel::BlockList(url)) => Ok(url),
            #[cfg(feature = "local_fs")]
            GridHighlightedLink::File(WithinModel::BlockList(file_link)) => {
                Ok(file_link.map(|file_link| file_link.link))
            }
            _ => Err(anyhow::anyhow!(
                "HighlightedLink is not within the block list"
            )),
        }
    }
}

/// The highlighted_link state is synced with both the BlockList and AltScreen so that they can
/// use the highlighted_link to override the normal smart-selection behavior. The
/// highlighted_link can, for example, verify that a file path actually exists on disk, and
/// include file paths with spaces. Smart-select can do neither of those things.
/// Since this value must be kept in sync, we need to prevent any mutation of the value outside
/// of this wrapper.
#[derive(Debug, Default)]
pub struct HighlightedLinkOption {
    inner: Option<GridHighlightedLink>,
    /// True if the underlying content has changed such that the link may no longer be valid.
    invalidated: bool,
}

#[derive(Clone, Debug)]
pub enum RichContentLink {
    Url(String),
    #[cfg(feature = "local_fs")]
    FilePath {
        absolute_path: PathBuf,
        line_and_column_num: Option<LineAndColumnArg>,
        target_override: Option<FileTarget>,
    },
}

impl RichContentLink {
    pub fn tooltip_text(&self) -> &'static str {
        match &self {
            #[cfg(feature = "local_fs")]
            RichContentLink::FilePath { absolute_path, .. } if absolute_path.is_dir() => {
                "Open folder"
            }
            #[cfg(feature = "local_fs")]
            RichContentLink::FilePath { .. } => "Open file",
            RichContentLink::Url(_) => "Open link",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RichContentLinkTooltipInfo {
    pub link: RichContentLink,
    pub position_id: String,
}

impl HighlightedLinkOption {
    /// Assigns the inner value and syncs it with the BlockList and AltScreen
    pub fn set(&mut self, link: GridHighlightedLink, model: &mut TerminalModel) {
        match &link {
            GridHighlightedLink::Url(within_model) => match within_model {
                WithinModel::BlockList(within_block) => {
                    let point_range = WithinBlock::new(
                        within_block.inner.range.clone(),
                        within_block.block_index,
                        within_block.grid,
                    );
                    model
                        .block_list_mut()
                        .set_smart_select_override(point_range);
                }
                WithinModel::AltScreen(link) => {
                    model
                        .alt_screen_mut()
                        .set_smart_select_override(link.range.clone());
                }
            },
            #[cfg(feature = "local_fs")]
            GridHighlightedLink::File(within_model) => match within_model {
                WithinModel::BlockList(within_block) => {
                    let point_range = WithinBlock::new(
                        within_block.inner.link.range.clone(),
                        within_block.block_index,
                        within_block.grid,
                    );
                    model
                        .block_list_mut()
                        .set_smart_select_override(point_range);
                }
                WithinModel::AltScreen(file_link) => {
                    model
                        .alt_screen_mut()
                        .set_smart_select_override(file_link.link.range.clone());
                }
            },
        }
        self.inner = Some(link);
    }

    /// Wrapper method for Option::take that also keeps the derived state in the BlockList and
    /// AltScreen in sync
    pub fn take(&mut self, model: &mut TerminalModel) -> Option<GridHighlightedLink> {
        model.block_list_mut().clear_smart_select_override();
        model.alt_screen_mut().clear_smart_select_override();
        self.invalidated = false;
        self.inner.take()
    }

    pub fn invalidate(&mut self) {
        self.invalidated = true;
    }

    pub fn is_invalidated(&self) -> bool {
        self.invalidated
    }

    pub fn clone_inner(&self) -> Option<GridHighlightedLink> {
        self.inner.clone()
    }
}

impl Deref for HighlightedLinkOption {
    type Target = Option<GridHighlightedLink>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl super::TerminalView {
    pub(super) fn maybe_link_hover(
        &mut self,
        position: &Option<WithinModel<Point>>,
        from_editor: TerminalEditor,
        ctx: &mut ViewContext<Self>,
    ) {
        // Do not highlight the url while selecting text or blocks, or if the window is not active.
        if self.terminal_is_selecting(&self.model.lock(), ctx)
            || self.is_navigated_away_from_window(ctx)
        {
            if self.highlighted_link.take(&mut self.model.lock()).is_some() {
                ctx.reset_cursor();
                ctx.notify();
            }
            return;
        }

        // If the mouse isn't in the terminal view, we're not hovering any link.
        let Some(position) = position else {
            if self.highlighted_link.take(&mut self.model.lock()).is_some() {
                ctx.reset_cursor();
                // Clear last_hover_fragment_boundary when mouse is out of block bounds.
                self.last_hover_fragment_boundary = None;
                ctx.notify();
            }
            return;
        };

        // If the mouse is still on top of the previous highlighted link and that link is
        // still valid, we can keep highlighting it.
        if let Some(link) = self.highlighted_link.as_ref() {
            if link.contains(position) && !self.highlighted_link.is_invalidated() {
                // If already hovering on a highlighted link, return.
                return;
            }
        }

        // Updating the cursor shape repeatedly can cause flashing, so we only set it once, and only
        // when necessary.
        let mut new_cursor_shape = None;

        // If a link is highlighted and it's invalidated or we're not hovering it, remove that
        // hover and look for a new one.
        if self.highlighted_link.is_some() {
            // Remove the current highlighted link because we are no longer
            // hovering over it.
            self.highlighted_link.take(&mut self.model.lock());
            new_cursor_shape = Some(Cursor::Arrow);
        }

        let (url_at_point, new_fragment_boundary) = {
            let model = self.model.lock();
            (
                model.url_at_point(position),
                model.fragment_boundary_at_point(position),
            )
        };

        match (url_at_point, &self.last_hover_fragment_boundary) {
            (Some(url), _) => {
                self.highlighted_link
                    .set(GridHighlightedLink::Url(url), &mut self.model.lock());
                new_cursor_shape = Some(Cursor::PointingHand);
            }
            // Only scan for links if the mouse hovered on a new word.
            (_, Some(last_hover_fragment_boundary))
                if !last_hover_fragment_boundary.contains(position) =>
            {
                // Use try_send to return an error directly when the channel is full
                // instead of blocking main thread.
                let _ = self.find_link_tx.try_send(FindLinkArg {
                    position: *position,
                    from_editor,
                });
            }
            // If there's no last hover fragment boundary, we scan for links.
            (_, None) => {
                let _ = self.find_link_tx.try_send(FindLinkArg {
                    position: *position,
                    from_editor,
                });
            }
            _ => (),
        };

        if let Some(new_cursor_shape) = new_cursor_shape {
            ctx.set_cursor_shape(new_cursor_shape);
            ctx.notify();
        }

        self.last_hover_fragment_boundary = Some(new_fragment_boundary);
    }

    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub(super) fn handle_find_link(
        &mut self,
        find_link_arg: FindLinkArg,
        ctx: &mut ViewContext<Self>,
    ) {
        let FindLinkArg {
            position,
            from_editor,
        } = find_link_arg;

        // Already highlighted the hovered link, returning.
        if self
            .highlighted_link
            .as_ref()
            .is_some_and(|url| url.contains(&position))
        {
            #[cfg_attr(not(feature = "local_fs"), allow(clippy::needless_return))]
            return;
        }

        #[cfg(feature = "local_fs")]
        self.scan_for_file_path(position, from_editor, ctx);
    }

    pub(super) fn open_highlighted_link(
        &mut self,
        link: &GridHighlightedLink,
        ctx: &mut ViewContext<Self>,
    ) {
        self.dismiss_tooltips(ctx);
        ctx.focus(&self.input);
        ctx.notify();

        send_telemetry_from_ctx!(
            TelemetryEvent::OpenLink {
                link: link.clone(),
                open_with: LinkOpenMethod::ToolTip
            },
            ctx
        );
        match link {
            #[cfg(feature = "local_fs")]
            GridHighlightedLink::File(link) => {
                self.open_file_link(link, ctx);
            }
            GridHighlightedLink::Url(url) => {
                let model = self.model.lock();
                ctx.open_url(&model.link_at_range(url, RespectObfuscatedSecrets::No));
            }
        };
    }

    pub(super) fn open_rich_content_link(
        &mut self,
        link: &RichContentLink,
        ctx: &mut ViewContext<Self>,
    ) {
        self.dismiss_tooltips(ctx);
        ctx.focus(&self.input);
        ctx.notify();

        match link {
            #[cfg(feature = "local_fs")]
            RichContentLink::FilePath {
                absolute_path,
                line_and_column_num,
                target_override,
            } => {
                if let Some(target_override) = target_override {
                    self.open_file_path_with_target(
                        absolute_path.clone(),
                        target_override.clone(),
                        *line_and_column_num,
                        ctx,
                    );
                } else {
                    self.open_file_path(absolute_path.clone(), *line_and_column_num, ctx);
                }
            }
            RichContentLink::Url(url) => {
                ctx.open_url(url);
            }
        };
    }
}

// A collection of link detection functions that are only valid on platforms
// where we can spawn a local tty.
#[cfg(feature = "local_fs")]
impl super::TerminalView {
    fn session_for_file_link(
        &self,
        link: &WithinModel<FileLink>,
        ctx: &mut ViewContext<Self>,
    ) -> Option<std::sync::Arc<crate::terminal::model::session::Session>> {
        let session_id = match link {
            WithinModel::AltScreen(_) => self.active_block_session_id(),
            WithinModel::BlockList(inner) => self
                .model
                .lock()
                .block_list()
                .block_at(inner.block_index)
                .and_then(|block| block.session_id()),
        }?;

        self.sessions.as_ref(ctx).get(session_id)
    }

    pub(super) fn open_file_link(
        &mut self,
        link: &WithinModel<FileLink>,
        ctx: &mut ViewContext<Self>,
    ) {
        let file_link = link.get_inner();
        match file_link.location() {
            LocalOrRemotePath::Local(path) => {
                self.open_file_path(path.clone(), file_link.line_and_column_num, ctx);
            }
            LocalOrRemotePath::Remote(_) => {
                if let Some(session) = self.session_for_file_link(link, ctx) {
                    ctx.emit(super::Event::OpenFileInWarp {
                        path: file_link.location().clone(),
                        session,
                    });
                }
            }
        }
    }

    /// Scans the terminal model at the given position to see if it is
    /// contained within a path that should be linkified.
    fn scan_for_file_path(
        &mut self,
        position: WithinModel<Point>,
        from_editor: TerminalEditor,
        ctx: &mut ViewContext<Self>,
    ) {
        // For AltScreen we scan relative paths with the active working directory. For BlockList we
        // scan relative paths with the pwd and session of the hovered block.
        let scan_context = match position {
            WithinModel::AltScreen(_) => {
                self.pwd_if_local(ctx)
                    .map(|working_directory| FileLinkScanContext::Local {
                        working_directory,
                        shell_launch_data: self
                            .active_block_session_id()
                            .and_then(|active_session_id| {
                                self.sessions.as_ref(ctx).get(active_session_id)
                            })
                            .and_then(|active_session| active_session.launch_data().cloned()),
                    })
            }
            WithinModel::BlockList(inner) => {
                let block_context = {
                    let model = self.model.lock();
                    model
                        .block_list()
                        .block_at(inner.block_index)
                        .and_then(|block| Some((block.pwd()?.clone(), block.session_id())))
                };

                block_context.and_then(|(working_directory, session_id)| {
                    Self::file_link_scan_context_for_session(
                        working_directory,
                        session_id,
                        self.sessions.as_ref(ctx),
                    )
                })
            }
        };

        match scan_context {
            // Check if we are hovering on any file path. Don't scan for file path
            // if user is hovering from an editor like vim or nano.
            Some(scan_context) if matches!(from_editor, TerminalEditor::No) => {
                let possible_paths = self.model.lock().possible_file_paths_at_point(position);
                let max_columns = self.size_info.columns;

                // Using the thread builder instead of ctx.spawn here so that the previous
                // scanning job will be dropped once there is a new scanning job created.
                let (tx, rx) = futures::channel::oneshot::channel();
                self.file_link_scanning_join_handle = std::thread::Builder::new()
                    .name("Compute file paths".into())
                    .spawn(move || {
                        let paths =
                            Self::compute_valid_paths(scan_context, possible_paths, max_columns);
                        let _ = tx.send(paths);
                    })
                    .map_err(|e| {
                        log::error!("Unable to spawn thread {e:?}");
                    })
                    .ok();

                let _ = ctx.spawn(
                    async move { rx.await.ok().flatten() },
                    Self::handle_file_link_completed,
                );
            }
            _ if self.highlighted_link.take(&mut self.model.lock()).is_some() => {
                ctx.reset_cursor();
                ctx.notify();
            }
            _ => (),
        };
    }

    fn file_link_scan_context_for_session(
        working_directory: String,
        session_id: Option<SessionId>,
        sessions: &crate::terminal::model::session::Sessions,
    ) -> Option<FileLinkScanContext> {
        let Some(session_id) = session_id else {
            return Some(FileLinkScanContext::Local {
                working_directory,
                shell_launch_data: None,
            });
        };

        let session = sessions.get(session_id)?;
        match session.session_type() {
            SessionType::Local => Some(FileLinkScanContext::Local {
                working_directory,
                shell_launch_data: session.launch_data().cloned(),
            }),
            SessionType::WarpifiedRemote {
                host_id: Some(host_id),
            } => Some(FileLinkScanContext::RemoteMarkdown {
                working_directory,
                host_id,
            }),
            SessionType::WarpifiedRemote { host_id: None } => None,
        }
    }

    fn compute_valid_paths(
        scan_context: FileLinkScanContext,
        possible_paths: impl Iterator<Item = WithinModel<grid_handler::PossiblePath>>,
        max_columns: usize,
    ) -> Option<GridHighlightedLink> {
        let mut link = None;
        'path_loop: for within_model_possible_path in possible_paths {
            let possible_path = within_model_possible_path.get_inner();
            // We want to check if the clean path result is a valid path and get the canonical
            // absolute path back.
            let location = Self::valid_file_link_location(&possible_path.path, &scan_context);

            if let Some(location) = location {
                link = Some(Self::create_valid_link(
                    location,
                    possible_path.path.line_and_column_num,
                    possible_path.range.clone(),
                    &within_model_possible_path,
                ));
                break;
            }

            for prefix in PREFIXES_TO_REMOVE {
                if let Some(new_possible_path) = possible_path.path.path.strip_prefix(prefix) {
                    let new_possible_cleaned_path = CleanPathResult {
                        path: new_possible_path.into(),
                        line_and_column_num: possible_path.path.line_and_column_num,
                    };
                    let location =
                        Self::valid_file_link_location(&new_possible_cleaned_path, &scan_context);

                    // check if new_possible_path is valid
                    if let Some(location) = location {
                        let new_start_point = possible_path
                            .range
                            .start()
                            .wrapping_add(max_columns, prefix.len());

                        link = Some(Self::create_valid_link(
                            location,
                            new_possible_cleaned_path.line_and_column_num,
                            new_start_point..=*possible_path.range.end(),
                            &within_model_possible_path,
                        ));

                        // break outer_loop
                        break 'path_loop;
                    }
                }
            }

            for suffix in SUFFIXES_TO_REMOVE {
                if let Some(new_possible_path) = possible_path.path.path.strip_suffix(suffix) {
                    let new_possible_cleaned_path = CleanPathResult {
                        path: new_possible_path.into(),
                        line_and_column_num: possible_path.path.line_and_column_num,
                    };
                    let location =
                        Self::valid_file_link_location(&new_possible_cleaned_path, &scan_context);

                    // check if new_possible_path is valid
                    if let Some(location) = location {
                        let new_end_point = possible_path
                            .range
                            .end()
                            .wrapping_sub(max_columns, suffix.len());

                        link = Some(Self::create_valid_link(
                            location,
                            new_possible_cleaned_path.line_and_column_num,
                            *possible_path.range.start()..=new_end_point,
                            &within_model_possible_path,
                        ));

                        // break outer_loop
                        break 'path_loop;
                    }
                }
            }
        }

        link.map(GridHighlightedLink::File)
    }

    fn valid_file_link_location(
        clean_path_result: &CleanPathResult,
        scan_context: &FileLinkScanContext,
    ) -> Option<LocalOrRemotePath> {
        match scan_context {
            FileLinkScanContext::Local {
                working_directory,
                shell_launch_data,
            } => absolute_path_if_valid(
                clean_path_result,
                ShellPathType::ShellNative(working_directory.to_string()),
                shell_launch_data.as_ref(),
            )
            .map(LocalOrRemotePath::Local),
            FileLinkScanContext::RemoteMarkdown {
                working_directory,
                host_id,
            } => remote_markdown_location(clean_path_result, working_directory, host_id),
        }
    }

    fn create_valid_link(
        location: LocalOrRemotePath,
        line_and_column_num: Option<LineAndColumnArg>,
        path_range: std::ops::RangeInclusive<Point>,
        possible_path: &WithinModel<grid_handler::PossiblePath>,
    ) -> WithinModel<FileLink> {
        let inner_link = FileLink {
            link: Link {
                range: path_range,
                is_empty: false,
            },
            location,
            line_and_column_num,
        };

        match possible_path {
            WithinModel::AltScreen(_) => WithinModel::AltScreen(inner_link),
            WithinModel::BlockList(inner) => {
                WithinModel::BlockList(WithinBlock::new(inner_link, inner.block_index, inner.grid))
            }
        }
    }

    fn handle_file_link_completed(
        &mut self,
        link_result: Option<GridHighlightedLink>,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut model = self.model.lock();
        if self.highlighted_link.take(&mut model).is_some() {
            ctx.reset_cursor();
            ctx.notify();
        }

        if let Some(new_link) = link_result {
            self.highlighted_link.set(new_link, &mut model);
            ctx.set_cursor_shape(Cursor::PointingHand);
            ctx.notify();
        }
    }
}

#[cfg(all(test, feature = "local_fs"))]
mod tests {
    use super::*;

    fn remote_markdown_path(raw_path: &str, working_directory: &str) -> Option<String> {
        let host_id = warp_core::HostId::new("test-host".to_string());
        let clean_path_result = CleanPathResult {
            path: raw_path.to_string(),
            line_and_column_num: None,
        };
        let scan_context = FileLinkScanContext::RemoteMarkdown {
            working_directory: working_directory.to_string(),
            host_id: host_id.clone(),
        };

        let location = super::super::TerminalView::valid_file_link_location(
            &clean_path_result,
            &scan_context,
        )?;
        match location {
            LocalOrRemotePath::Remote(remote) => {
                assert_eq!(remote.host_id, host_id);
                Some(remote.path.as_str().to_string())
            }
            LocalOrRemotePath::Local(_) => None,
        }
    }

    #[test]
    fn remote_markdown_absolute_unix_path_is_not_joined_to_pwd() {
        assert_eq!(
            remote_markdown_path("/home/alice/repo/README.md", "/tmp/current").as_deref(),
            Some("/home/alice/repo/README.md")
        );
    }

    #[test]
    fn remote_markdown_absolute_windows_path_is_not_joined_to_pwd() {
        assert_eq!(
            remote_markdown_path(r"D:\Users\alice\docs\README.md", r"C:\Users\alice\repo")
                .as_deref(),
            Some(r"D:\Users\alice\docs\README.md")
        );
    }

    #[test]
    fn remote_markdown_relative_unix_path_uses_remote_pwd() {
        assert_eq!(
            remote_markdown_path("docs/README.md", "/home/alice/repo").as_deref(),
            Some("/home/alice/repo/docs/README.md")
        );
    }

    #[test]
    fn remote_markdown_relative_windows_path_uses_remote_pwd() {
        assert_eq!(
            remote_markdown_path(r"..\README.md", r"C:\Users\alice\repo\docs").as_deref(),
            Some(r"C:\Users\alice\repo\README.md")
        );
    }

    #[test]
    fn remote_markdown_extensionless_windows_file_name_is_detected() {
        assert_eq!(
            remote_markdown_path(r"C:\Users\alice\repo\README", r"C:\Users\alice\repo").as_deref(),
            Some(r"C:\Users\alice\repo\README")
        );
    }

    #[test]
    fn remote_markdown_extensionless_unix_file_name_is_detected() {
        assert_eq!(
            remote_markdown_path("/home/alice/repo/CHANGELOG", "/home/alice/repo").as_deref(),
            Some("/home/alice/repo/CHANGELOG")
        );
    }

    #[test]
    fn remote_markdown_non_markdown_path_is_ignored() {
        assert_eq!(
            remote_markdown_path("src/main.rs", "/home/alice/repo"),
            None
        );
        assert_eq!(remote_markdown_path("notes.txt", "/home/alice/repo"), None);
    }
}
