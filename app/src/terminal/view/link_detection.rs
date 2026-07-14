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
            terminal::model::grid::grid_handler,
            terminal::ShellLaunchData,
            util::file::{FileLink, absolute_path_if_valid, ShellPathType},
            util::openable_file_type::FileTarget,
        };
        use std::path::PathBuf;
        use unicode_general_category::{get_general_category, GeneralCategory};
        use unicode_width::UnicodeWidthChar;
        use warp_util::path::CleanPathResult;
        use warp_util::path::LineAndColumnArg;
    }
}

#[cfg(feature = "local_fs")]
use warp_errors::report_error;

use super::{FindLinkArg, TerminalEditor};

// "a/" and "b/" are prefixes specific to Git Diff
#[cfg(feature = "local_fs")]
const PREFIXES_TO_REMOVE: [&str; 2] = ["a/", "b/"];

/// "@" is a suffix that can be added to symlinks. It appears in Git Bash's default configuration
/// for `ls`.
#[cfg(feature = "local_fs")]
const SUFFIXES_TO_REMOVE: [&str; 1] = ["@"];

#[cfg(feature = "local_fs")]
struct TrimmedSentencePunctuation<'a> {
    path: &'a str,
    removed_width: usize,
}

#[cfg(feature = "local_fs")]
fn is_trailing_sentence_punctuation(c: char) -> bool {
    if c == '.' {
        return true;
    }
    if c.is_ascii() {
        return false;
    }
    matches!(
        get_general_category(c),
        GeneralCategory::ClosePunctuation
            | GeneralCategory::FinalPunctuation
            | GeneralCategory::OtherPunctuation
    )
}

/// Strips trailing sentence punctuation from a captured path token when the
/// punctuation is prose around the path rather than a meaningful path component.
///
/// File paths written at the end of a sentence frequently capture the trailing
/// punctuation (e.g. `notes/README.md.` or `notes/README.md，`). On Windows the
/// NT path normalizer silently strips a trailing `.` during path resolution, so
/// without trimming, the captured token keeps the period in both the highlight
/// range and the file extension, defeating extension-based classification (e.g.
/// opening markdown in the viewer instead of as raw text).
///
/// Returns `None` when there is no trailing sentence punctuation, or when a
/// trailing period is part of a `.`/`..` path component (e.g. `.`, `..`, `foo/.`,
/// `foo/..`), which are legitimate path segments and must be preserved.
#[cfg(feature = "local_fs")]
fn path_without_trailing_sentence_punctuation(
    path: &str,
) -> Option<TrimmedSentencePunctuation<'_>> {
    let mut trimmed = path;
    let mut removed_width = 0;

    while let Some(c) = trimmed.chars().next_back() {
        if !is_trailing_sentence_punctuation(c) {
            break;
        }

        let new_trimmed = trimmed.strip_suffix(c)?;
        if new_trimmed.is_empty() {
            break;
        }

        if c == '.' {
            match new_trimmed.chars().next_back() {
                // Empty (`.`) or a dot/separator immediately before the trailing
                // `.` means the period is a real path component (`..`, `foo/.`,
                // `foo\.`), not sentence punctuation.
                None | Some('.') | Some('/') | Some('\\') => break,
                _ => {}
            }
        }

        trimmed = new_trimmed;
        removed_width += UnicodeWidthChar::width(c).unwrap_or(1);
    }

    (removed_width > 0).then_some(TrimmedSentencePunctuation {
        path: trimmed,
        removed_width,
    })
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
                let link = link.get_inner();
                if let Some(path) = link.absolute_path() {
                    self.open_file_path(path.clone(), link.line_and_column_num, ctx);
                }
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
    /// Scans the terminal model at the given position to see if it is
    /// contained within a path that should be linkified.
    fn scan_for_file_path(
        &mut self,
        position: WithinModel<Point>,
        from_editor: TerminalEditor,
        ctx: &mut ViewContext<Self>,
    ) {
        // For AltScreen we scan for relative path with the current working directory.
        // For BlockList we scan for relative path with the pwd of the hovered block.
        let pwd_to_scan_for = match position {
            WithinModel::AltScreen(_) => self.pwd_if_local(ctx),
            WithinModel::BlockList(inner) => self
                .model
                .lock()
                .block_list()
                .block_at(inner.block_index)
                .filter(|block| !self.is_block_considered_remote(block.session_id(), None, ctx)) // Don't scan for file links if the block is on remote sessions
                .and_then(|block| block.pwd().map(String::from)),
        };

        match pwd_to_scan_for {
            // Check if we are hovering on any file path. Don't scan for file path
            // if user is hovering from an editor like vim or nano.
            Some(path) if matches!(from_editor, TerminalEditor::No) => {
                let possible_paths = self.model.lock().possible_file_paths_at_point(position);
                let max_columns = self.size_info.columns;
                let shell_launch_data = self
                    .active_block_session_id()
                    .and_then(|active_session_id| self.sessions.as_ref(ctx).get(active_session_id))
                    .and_then(|active_session| active_session.launch_data().cloned());

                // Using the thread builder instead of ctx.spawn here so that the previous
                // scanning job will be dropped once there is a new scanning job created.
                let (tx, rx) = futures::channel::oneshot::channel();
                self.file_link_scanning_join_handle = std::thread::Builder::new()
                    .name("Compute file paths".into())
                    .spawn(move || {
                        let paths = Self::compute_valid_paths(
                            &path,
                            possible_paths,
                            max_columns,
                            shell_launch_data,
                        );
                        let _ = tx.send(paths);
                    })
                    .map_err(|e| {
                        report_error!(anyhow::Error::new(e).context("Unable to spawn thread"));
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

    fn compute_valid_paths(
        working_directory: &str,
        possible_paths: impl Iterator<Item = WithinModel<grid_handler::PossiblePath>>,
        max_columns: usize,
        shell_launch_data: Option<ShellLaunchData>,
    ) -> Option<GridHighlightedLink> {
        let mut link = None;
        'path_loop: for within_model_possible_path in possible_paths {
            let possible_path = within_model_possible_path.get_inner();

            // A file path at the end of a sentence often captures trailing prose
            // punctuation (e.g. `notes/README.md.` or `notes/README.md，`). Try the
            // punctuation-trimmed candidate first so the resolved file, highlight
            // range, and extension-based classification all exclude it. This must
            // run before the untrimmed lookup because on Windows the NT path
            // normalizer strips trailing dots, so the untrimmed path would
            // otherwise resolve and leave the period inside the captured link.
            if let Some(trimmed_path) =
                path_without_trailing_sentence_punctuation(&possible_path.path.path)
            {
                let trimmed_cleaned_path = CleanPathResult {
                    path: trimmed_path.path.into(),
                    line_and_column_num: possible_path.path.line_and_column_num,
                };
                if let Some(absolute_path) = absolute_path_if_valid(
                    &trimmed_cleaned_path,
                    ShellPathType::ShellNative(working_directory.to_string()),
                    shell_launch_data.as_ref(),
                ) {
                    let new_end_point = possible_path
                        .range
                        .end()
                        .wrapping_sub(max_columns, trimmed_path.removed_width);
                    link = Some(Self::create_valid_link(
                        absolute_path,
                        trimmed_cleaned_path.line_and_column_num,
                        *possible_path.range.start()..=new_end_point,
                        &within_model_possible_path,
                    ));
                    break 'path_loop;
                }
            }

            // We want to check if the clean path result is a valid path and get the canonical
            // absolute path back.
            let absolute_path = absolute_path_if_valid(
                &possible_path.path,
                ShellPathType::ShellNative(working_directory.to_string()),
                shell_launch_data.as_ref(),
            );

            if let Some(absolute_path) = absolute_path {
                link = Some(Self::create_valid_link(
                    absolute_path,
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
                    let absolute_path = absolute_path_if_valid(
                        &new_possible_cleaned_path,
                        ShellPathType::ShellNative(working_directory.to_string()),
                        shell_launch_data.as_ref(),
                    );

                    // check if new_possible_path is valid
                    if let Some(absolute_path) = absolute_path {
                        let new_start_point = possible_path
                            .range
                            .start()
                            .wrapping_add(max_columns, prefix.len());

                        link = Some(Self::create_valid_link(
                            absolute_path,
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
                    let absolute_path = absolute_path_if_valid(
                        &new_possible_cleaned_path,
                        ShellPathType::ShellNative(working_directory.to_string()),
                        shell_launch_data.as_ref(),
                    );

                    // check if new_possible_path is valid
                    if let Some(absolute_path) = absolute_path {
                        let new_end_point = possible_path
                            .range
                            .end()
                            .wrapping_sub(max_columns, suffix.len());

                        link = Some(Self::create_valid_link(
                            absolute_path,
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

    fn create_valid_link(
        absolute_path: PathBuf,
        line_and_column_num: Option<LineAndColumnArg>,
        path_range: std::ops::RangeInclusive<Point>,
        possible_path: &WithinModel<grid_handler::PossiblePath>,
    ) -> WithinModel<FileLink> {
        let inner_link = FileLink {
            link: Link {
                range: path_range,
                is_empty: false,
            },
            absolute_path,
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
#[path = "link_detection_tests.rs"]
mod tests;
