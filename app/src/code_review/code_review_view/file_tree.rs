//! File-tree rendering for the code review sidebar.
//!
//! This module is a child of [`code_review_view`] and adds tree-structured rendering
//! of changed files as an alternative to the previous flat list.  It reuses the shared
//! visual primitives from [`crate::code::file_tree::row_renderer`] so the tree rows
//! look identical to those in the left-panel Project Explorer.

use std::collections::HashMap;

use indexmap::IndexMap;
use warp_core::features::FeatureFlag;
use warpui::elements::new_scrollable::{NewScrollable, ScrollableAppearance, SingleAxisConfig};
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, DragBarSide, Empty, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Resizable, ScrollbarWidth,
    Shrinkable, Text,
};
use warpui::fonts::Properties;
use warpui::platform::Cursor;
use warpui::Element;

use super::{CodeReviewAction, CodeReviewView, FileState, LoadedState};
use crate::appearance::Appearance;
use crate::code::editor::{add_color, remove_color};
use crate::code::file_tree::ordering::compare_file_tree_entries;
use crate::code::file_tree::row_renderer::{
    render_tree_row, TreeRowConfig, FOLDER_INDENT, ITEM_FONT_SIZE, ITEM_PADDING,
};
use crate::ui_components::icons::Icon;
use crate::ui_components::item_highlight::{ImageOrIcon, ItemHighlightState};

// ─── Data model ──────────────────────────────────────────────────────────────

/// A node in the code review file tree.
pub(super) enum CodeReviewTreeNode {
    /// A directory grouping.  `path` is the repo-relative path of the directory
    /// (e.g. `"src/foo"`); used as the key in the expanded-directories set.
    Dir {
        name: String,
        path: String,
        children: Vec<CodeReviewTreeNode>,
    },
    /// A changed file leaf node.
    File {
        name: String,
        /// Repo-relative path — key into [`LoadedState::file_states`].
        file_path: String,
        additions: usize,
        deletions: usize,
        /// Index into the ordered `file_states` map (for `FileSelected`).
        file_index: usize,
    },
}

impl CodeReviewTreeNode {
    fn name(&self) -> &str {
        match self {
            CodeReviewTreeNode::Dir { name, .. } => name,
            CodeReviewTreeNode::File { name, .. } => name,
        }
    }

    fn is_dir(&self) -> bool {
        matches!(self, CodeReviewTreeNode::Dir { .. })
    }
}

// ─── Tree building ────────────────────────────────────────────────────────────

/// Builds a tree from the given ordered file-state map.
///
/// Directories are inferred from path components.  All directories default to
/// expanded (callers gate rendering on [`CodeReviewView::expanded_dirs`]).
pub(super) fn build_code_review_tree(
    file_states: &IndexMap<String, FileState>,
) -> Vec<CodeReviewTreeNode> {
    let mut roots: Vec<CodeReviewTreeNode> = Vec::new();

    for (file_index, (path, state)) in file_states.iter().enumerate() {
        let additions = state.file_diff.additions();
        let deletions = state.file_diff.deletions();
        insert_into_tree(&mut roots, path, "", file_index, additions, deletions);
    }

    sort_nodes(&mut roots);
    roots
}

/// Recursively inserts a file path into the tree.
///
/// `remaining` is the portion of the path not yet consumed;
/// `prefix` is the cumulative directory path built so far.
fn insert_into_tree(
    nodes: &mut Vec<CodeReviewTreeNode>,
    remaining: &str,
    prefix: &str,
    file_index: usize,
    additions: usize,
    deletions: usize,
) {
    if let Some(slash) = remaining.find('/') {
        let dir_name = &remaining[..slash];
        let rest = &remaining[slash + 1..];
        let dir_path = if prefix.is_empty() {
            dir_name.to_string()
        } else {
            format!("{prefix}/{dir_name}")
        };

        // Find an existing Dir with this path and recurse into it.
        for node in nodes.iter_mut() {
            if let CodeReviewTreeNode::Dir { path, children, .. } = node {
                if *path == dir_path {
                    insert_into_tree(children, rest, &dir_path, file_index, additions, deletions);
                    return;
                }
            }
        }

        // No matching directory yet — create one.
        let mut children = Vec::new();
        insert_into_tree(
            &mut children,
            rest,
            &dir_path,
            file_index,
            additions,
            deletions,
        );
        nodes.push(CodeReviewTreeNode::Dir {
            name: dir_name.to_string(),
            path: dir_path,
            children,
        });
    } else {
        // Leaf file node.
        let full_path = if prefix.is_empty() {
            remaining.to_string()
        } else {
            format!("{prefix}/{remaining}")
        };
        nodes.push(CodeReviewTreeNode::File {
            name: remaining.to_string(),
            file_path: full_path,
            additions,
            deletions,
            file_index,
        });
    }
}

/// Sorts nodes at every level using the same directory-first, dotfile-first,
/// natural ordering as the left-panel file tree.
fn sort_nodes(nodes: &mut [CodeReviewTreeNode]) {
    for node in nodes.iter_mut() {
        if let CodeReviewTreeNode::Dir { children, .. } = node {
            sort_nodes(children);
        }
    }
    nodes.sort_by(|a, b| {
        compare_file_tree_entries(a.is_dir(), Some(a.name()), b.is_dir(), Some(b.name()))
    });
}

// ─── Mouse-state management ───────────────────────────────────────────────────

/// Inserts every directory path in the tree into `expanded_dirs`, marking all
/// of them as expanded.  Called on every diff reload, so any previously
/// collapsed directory will be re-expanded when a new diff is loaded.
pub(super) fn collect_expanded_dirs(
    nodes: &[CodeReviewTreeNode],
    expanded_dirs: &mut std::collections::HashSet<String>,
) {
    for node in nodes {
        if let CodeReviewTreeNode::Dir { path, children, .. } = node {
            expanded_dirs.insert(path.clone());
            collect_expanded_dirs(children, expanded_dirs);
        }
    }
}

/// Ensures every directory node in the tree has an entry in `states`.
///
/// Existing handles are preserved (so hover tracking survives re-renders);
/// new handles are created with `MouseStateHandle::default()`.
pub(super) fn rebuild_dir_mouse_states(
    nodes: &[CodeReviewTreeNode],
    states: &mut HashMap<String, MouseStateHandle>,
) {
    for node in nodes {
        if let CodeReviewTreeNode::Dir { path, children, .. } = node {
            states.entry(path.clone()).or_default();
            rebuild_dir_mouse_states(children, states);
        }
    }
}

// ─── Rendering (impl CodeReviewView) ─────────────────────────────────────────

impl CodeReviewView {
    /// Replaces the old flat `render_file_sidebar` with a collapsible file tree.
    /// `tree` is the pre-built cache from `CodeReviewView::cached_sidebar_tree`,
    /// avoiding a fresh allocation on every render frame.
    pub(super) fn render_file_tree_sidebar(
        &self,
        tree: &[CodeReviewTreeNode],
        state: &LoadedState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let tree_rows = self.render_tree_nodes(tree, 0, state, appearance);

        let mut column = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Start);
        for row in tree_rows {
            column.add_child(row);
        }

        let scrollable_content = NewScrollable::vertical(
            SingleAxisConfig::Clipped {
                handle: self.ui_state_handles.sidebar_scroll_state.clone(),
                child: column.finish(),
            },
            appearance.theme().nonactive_ui_detail().into(),
            appearance.theme().active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .with_vertical_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, false))
        .finish();

        let sidebar_on_right = FeatureFlag::GitOperationsInCodeReview.is_enabled();
        let sidebar_content = if sidebar_on_right {
            Container::new(scrollable_content)
                .with_padding_left(8.)
                .finish()
        } else {
            Container::new(scrollable_content)
                .with_padding_right(8.)
                .finish()
        };

        let mut resizable = Resizable::new(
            self.ui_state_handles.sidebar_resizable_state.clone(),
            sidebar_content,
        );
        if sidebar_on_right {
            resizable = resizable.with_dragbar_side(DragBarSide::Left);
        }
        resizable
            .on_resize(move |ctx, _| {
                ctx.notify();
            })
            .with_bounds_callback(Box::new(Self::file_sidebar_bounds_callback))
            .finish()
    }

    /// Recursively renders tree nodes at the given depth, returning a flat list of
    /// row elements for insertion into the sidebar column.
    fn render_tree_nodes(
        &self,
        nodes: &[CodeReviewTreeNode],
        depth: usize,
        state: &LoadedState,
        appearance: &Appearance,
    ) -> Vec<Box<dyn Element>> {
        let mut rows: Vec<Box<dyn Element>> = Vec::new();
        for node in nodes {
            match node {
                CodeReviewTreeNode::Dir {
                    name,
                    path,
                    children,
                } => {
                    rows.push(self.render_dir_row(name, path, depth, appearance));
                    if self.expanded_dirs.contains(path) {
                        rows.extend(self.render_tree_nodes(children, depth + 1, state, appearance));
                    }
                }
                CodeReviewTreeNode::File {
                    name,
                    file_path,
                    additions,
                    deletions,
                    file_index,
                } => {
                    rows.push(self.render_file_tree_row(
                        name,
                        file_path,
                        *file_index,
                        *additions,
                        *deletions,
                        depth,
                        state,
                        appearance,
                    ));
                }
            }
        }
        rows
    }

    /// Renders a directory row with a chevron that toggles expand/collapse.
    fn render_dir_row(
        &self,
        name: &str,
        path: &str,
        depth: usize,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let is_expanded = self.expanded_dirs.contains(path);
        // dir_mouse_states is populated for all tree directories in the diff-load handler
        // before any render of the sidebar occurs.  The fallback uses a handle created once
        // during construction (never during render), satisfying the WarpUI constraint.
        let mouse_state = self
            .dir_mouse_states
            .get(path)
            .cloned()
            .unwrap_or_else(|| self.sidebar_dir_fallback_mouse_state.clone());
        let dir_path = path.to_string();
        let dir_name = name.to_string();

        Hoverable::new(mouse_state, move |mouse_state| {
            let item_highlight_state = ItemHighlightState::new(false, mouse_state);
            let config = TreeRowConfig {
                depth,
                name: dir_name.clone(),
                icon: ImageOrIcon::Icon(Icon::Folder),
                is_expanded: Some(is_expanded),
                is_ignored: false,
                item_highlight_state,
            };
            let inner_row = render_tree_row(config, appearance);
            apply_row_container(inner_row, item_highlight_state, appearance)
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(CodeReviewAction::ToggleDirExpanded(dir_path.clone()));
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    /// Renders a file row with +/- change counts at the start (replacing the file-type icon)
    /// followed by the filename.  No trailing counts — code review only.
    #[allow(clippy::too_many_arguments)]
    fn render_file_tree_row(
        &self,
        name: &str,
        file_path: &str,
        file_index: usize,
        additions: usize,
        deletions: usize,
        depth: usize,
        state: &LoadedState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        // file_states always contains entries for every file in the loaded diff.
        // The fallback uses a handle created once during construction (never during render),
        // satisfying the WarpUI constraint that MouseStateHandles are not allocated per frame.
        let mouse_state = state
            .file_states
            .get(file_path)
            .map(|fs| fs.sidebar_mouse_state.clone())
            .unwrap_or_else(|| self.sidebar_file_fallback_mouse_state.clone());

        let file_name = name.to_string();

        Hoverable::new(mouse_state, move |mouse_state| {
            let item_highlight_state = ItemHighlightState::new(false, mouse_state);
            let text_color = item_highlight_state.text_and_icon_color(appearance);

            let mut row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center);

            // Indentation spacer.
            if depth > 0 {
                row.add_child(
                    Container::new(
                        ConstrainedBox::new(Empty::new().finish())
                            .with_width(depth as f32 * FOLDER_INDENT)
                            .finish(),
                    )
                    .finish(),
                );
            }

            // Empty placeholder matching the chevron slot on dir rows so
            // filenames stay left-aligned with directory names.
            row.add_child(
                Container::new(
                    ConstrainedBox::new(Empty::new().finish())
                        .with_width(FOLDER_INDENT)
                        .with_height(FOLDER_INDENT)
                        .finish(),
                )
                .with_margin_right(4.)
                .finish(),
            );

            // +N / -M counts in the icon slot, then the filename.
            // For pure renames (0 additions and 0 deletions) an empty placeholder keeps
            // the filename column aligned with rows that do have counts.
            if let Some(counts) = render_change_counts(additions, deletions, appearance) {
                row.add_child(Container::new(counts).with_margin_right(8.).finish());
            } else {
                row.add_child(
                    Container::new(
                        ConstrainedBox::new(Empty::new().finish())
                            .with_width(FOLDER_INDENT)
                            .finish(),
                    )
                    .with_margin_right(8.)
                    .finish(),
                );
            }
            row.add_child(
                Shrinkable::new(
                    1.,
                    Text::new_inline(
                        file_name.clone(),
                        appearance.ui_font_family(),
                        ITEM_FONT_SIZE,
                    )
                    .with_color(text_color)
                    .finish(),
                )
                .finish(),
            );

            apply_row_container(row.finish(), item_highlight_state, appearance)
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(CodeReviewAction::FileSelected(file_index));
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Wraps an inner element in a Container with the standard file-tree row padding
/// and the highlight background/corner-radius appropriate to the current state.
fn apply_row_container(
    inner: Box<dyn Element>,
    item_highlight_state: ItemHighlightState,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let mut container = Container::new(inner)
        .with_padding_top(ITEM_PADDING)
        .with_padding_bottom(ITEM_PADDING)
        .with_padding_left(8.)
        .with_padding_right(8.);

    if let Some(bg) = item_highlight_state.background_color(appearance) {
        container = container.with_background(bg);
    }
    if let Some(radius) = item_highlight_state.corner_radius() {
        container = container.with_corner_radius(radius);
    }
    container.finish()
}

/// Renders the `+N -M` change-count text for a file row.
/// Returns `None` if both `additions` and `deletions` are zero.
fn render_change_counts(
    additions: usize,
    deletions: usize,
    appearance: &Appearance,
) -> Option<Box<dyn Element>> {
    if additions == 0 && deletions == 0 {
        return None;
    }

    // Re-use the same font size as the row label for visual consistency.
    let font_size = ITEM_FONT_SIZE * 0.9;
    let mut text = Text::new("", appearance.ui_font_family(), font_size);

    if additions > 0 {
        text.add_text_with_highlights(
            format!("+{additions}"),
            add_color(appearance),
            Properties::default(),
        );
    }
    if deletions > 0 {
        if additions > 0 {
            text.add_text_with_highlights(" ", remove_color(appearance), Properties::default());
        }
        text.add_text_with_highlights(
            format!("-{deletions}"),
            remove_color(appearance),
            Properties::default(),
        );
    }

    Some(text.finish())
}

#[cfg(test)]
mod tests {
    use super::{sort_nodes, CodeReviewTreeNode};

    fn file(name: &str) -> CodeReviewTreeNode {
        CodeReviewTreeNode::File {
            name: name.to_string(),
            file_path: name.to_string(),
            additions: 0,
            deletions: 0,
            file_index: 0,
        }
    }

    fn dir(name: &str) -> CodeReviewTreeNode {
        CodeReviewTreeNode::Dir {
            name: name.to_string(),
            path: name.to_string(),
            children: Vec::new(),
        }
    }

    fn node_names(nodes: &[CodeReviewTreeNode]) -> Vec<&str> {
        nodes.iter().map(CodeReviewTreeNode::name).collect()
    }

    #[test]
    fn sort_nodes_matches_project_explorer_ordering() {
        let mut nodes = vec![
            file("file10.rs"),
            dir("src2"),
            file(".env"),
            dir(".config"),
            file("file1.rs"),
            dir("src10"),
            file("file2.rs"),
            dir("src1"),
        ];

        sort_nodes(&mut nodes);

        assert_eq!(
            node_names(&nodes),
            [
                ".config",
                "src1",
                "src2",
                "src10",
                ".env",
                "file1.rs",
                "file2.rs",
                "file10.rs",
            ]
        );
    }
}
