//! Generic, reusable JSON tree rendering component.
//!
//! Renders a `serde_json::Value` as an interactive, collapsible tree with
//! theme-driven colors.
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pathfinder_color::ColorU;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::WarpTheme;
use warpui::elements::{
    ConstrainedBox, CrossAxisAlignment, Empty, Flex, Hoverable, MainAxisSize, MouseState,
    MouseStateHandle, ParentElement, SavePosition, Shrinkable, Text,
};
use warpui::{Element, EventContext};

use crate::appearance::Appearance;

// ---------------------------------------------------------------------------
// Callback type aliases
// ---------------------------------------------------------------------------

/// Callback invoked when a collapsible container node is toggled.
pub type ToggleFn = dyn Fn(&mut EventContext, Vec<PathSegment>, usize) + Send + Sync;
/// Callback invoked when a long-string value is toggled.
pub type ToggleStringFn = dyn Fn(&mut EventContext, Vec<PathSegment>) + Send + Sync;
/// Callback invoked when "Copy JSON" is activated via right-click on a row.
pub type CopyJsonFn =
    dyn Fn(&mut EventContext, Vec<PathSegment>, serde_json::Value, String) + Send + Sync;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The indent width in logical pixels per nesting level.
const INDENT_PX: f32 = 12.;

/// The icon size for chevron expanders.
const CHEVRON_SIZE: f32 = 12.;

/// The font size used for all tree rows.
pub const TREE_FONT_SIZE: f32 = 12.;

/// Strings longer than this character count, or containing `\n`, are elided
/// by default and can be expanded in place.
pub const LONG_STRING_THRESHOLD: usize = 120;

// ---------------------------------------------------------------------------
// PathSegment
// ---------------------------------------------------------------------------

/// A single segment of a path into a `serde_json::Value` tree.
///
/// A sequence of segments uniquely identifies any node in the tree by its
/// structural position (key in an object, index in an array).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathSegment {
    /// A named key in a JSON object.
    Key(String),
    /// A 0-based index in a JSON array.
    Index(usize),
}

// ---------------------------------------------------------------------------
// JsonTreeState
// ---------------------------------------------------------------------------

/// Stores per-node expansion state for a rendered JSON tree.
///
/// State is keyed by `Vec<PathSegment>` which identifies each node by its
/// structural path in the tree. Path-keyed state is stable across
/// streaming re-parses because the path for any given node is deterministic
/// as long as the surrounding JSON structure is unchanged.
#[derive(Debug, Default, Clone)]
pub struct JsonTreeState {
    /// Expansion state for object/array nodes. Absent = expanded at depth 0,
    /// collapsed at depth 1+. An explicit entry always takes precedence.
    node_expansion: HashMap<Vec<PathSegment>, bool>,
    /// Expansion state for long string values. Absent = collapsed (elided).
    string_expansion: HashMap<Vec<PathSegment>, bool>,
    /// Per-node `MouseStateHandle`s used by the `Hoverable` elements in each
    /// rendered row.
    ///
    /// WarpUI requires that `MouseStateHandle`s be created once and reused
    /// across renders: creating `MouseStateHandle::default()` inline during
    /// render discards the `click_count` set during `LeftMouseDown` before
    /// `LeftMouseUp` fires, so click handlers never trigger. Storing handles
    /// here (keyed by node path) gives each row a stable handle that persists
    /// across re-renders triggered by `ctx.notify()`.
    ///
    /// A single map covers both container nodes and long-string rows. The same
    /// path cannot appear in both roles at once — a node is either a container
    /// or a scalar string, never both — so one handle per path is sufficient.
    mouse_states: RefCell<HashMap<Vec<PathSegment>, MouseStateHandle>>,
}

impl JsonTreeState {
    /// Returns whether the node at `path` should be expanded.
    ///
    /// Root nodes (depth 0) are expanded by default; all deeper nodes start
    /// collapsed. An explicit toggle entry always takes precedence.
    pub fn is_expanded(&self, path: &[PathSegment], depth: usize) -> bool {
        self.node_expansion.get(path).copied().unwrap_or(depth == 0)
    }

    /// Returns whether the long string at `path` should be expanded.
    pub fn is_string_expanded(&self, path: &[PathSegment]) -> bool {
        self.string_expansion.get(path).copied().unwrap_or(false)
    }

    /// Returns the stable `MouseStateHandle` for the row at `path`, creating
    /// one on first access. The handle is reused across renders so that
    /// click-state (press → notify → re-render → release) is preserved.
    pub fn mouse_state_for(&self, path: &[PathSegment]) -> MouseStateHandle {
        self.mouse_states
            .borrow_mut()
            .entry(path.to_vec())
            .or_insert_with(|| Arc::new(Mutex::new(MouseState::default())))
            .clone()
    }

    /// Toggles the expansion state of the node at `path`.
    pub fn toggle(&mut self, path: &[PathSegment], depth: usize) {
        let current = self.is_expanded(path, depth);
        self.node_expansion.insert(path.to_vec(), !current);
    }

    /// Toggles the expansion state of the long string at `path`.
    pub fn toggle_string(&mut self, path: &[PathSegment]) {
        let current = self.is_string_expanded(path);
        self.string_expansion.insert(path.to_vec(), !current);
    }
}

// ---------------------------------------------------------------------------
// JsonTreeColors
// ---------------------------------------------------------------------------

/// Pre-resolved colors for each JSON value category, sourced from the active
/// `WarpTheme`. Build this once per render from `JsonTreeColors::from_theme`.
#[derive(Debug, Clone, Copy)]
pub struct JsonTreeColors {
    /// Color for object/array keys and array indices.
    pub key: ColorU,
    /// Color for string values.
    pub string: ColorU,
    /// Color for number values.
    pub number: ColorU,
    /// Color for boolean values.
    pub bool: ColorU,
    /// Color for null values.
    pub null: ColorU,
    /// Color for type/size annotations (`{} 4 keys`) and punctuation.
    pub annotation: ColorU,
}

impl JsonTreeColors {
    /// Resolve colors from a `WarpTheme` and its background color.
    ///
    /// Each JSON value type maps to a visually distinct ANSI foreground color
    /// so that keys, strings, numbers, booleans, and null are easy to
    /// distinguish at a glance. Annotations and punctuation use the theme's
    /// subdued text color so they don't compete with value content.
    pub fn from_theme(theme: &WarpTheme) -> Self {
        let bg = theme.background();
        Self {
            key: theme.ansi_fg_cyan(),
            string: theme.ansi_fg_green(),
            number: theme.ansi_fg_yellow(),
            bool: theme.ansi_fg_magenta(),
            null: internal_colors::text_disabled(theme, bg),
            annotation: internal_colors::text_sub(theme, bg),
        }
    }
}

// ---------------------------------------------------------------------------
// Annotation helpers
// ---------------------------------------------------------------------------

/// Formats the annotation for a collapsible container node, e.g. `{} 3 keys`.
pub fn format_object_annotation(key_count: usize) -> String {
    match key_count {
        0 => "{} 0 keys".to_string(),
        1 => "{} 1 key".to_string(),
        n => format!("{{}} {} keys", n),
    }
}

/// Formats the annotation for a collapsible array node, e.g. `[] 2 items`.
pub fn format_array_annotation(item_count: usize) -> String {
    match item_count {
        0 => "[] 0 items".to_string(),
        1 => "[] 1 item".to_string(),
        n => format!("[] {} items", n),
    }
}

// ---------------------------------------------------------------------------
// Long-string helpers
// ---------------------------------------------------------------------------

/// Returns `true` when a string should be elided by default:
/// - Length > `LONG_STRING_THRESHOLD`, OR
/// - Contains a newline character.
pub fn is_long_string(s: &str) -> bool {
    s.len() > LONG_STRING_THRESHOLD || s.contains('\n')
}

/// Formats a `serde_json::Number` as a string, rendering whole-valued floats
/// as integers (e.g. `5.0` → `"5"`, `3.14` → `"3.14"`).
pub fn format_number(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    if let Some(f) = n.as_f64() {
        // Display whole-valued floats without the `.0` suffix.
        if f.fract() == 0.0 && f.is_finite() {
            return format!("{}", f as i64);
        }
        return format!("{}", f);
    }
    n.to_string()
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Builds a `Box<dyn Element>` that renders `root` as an interactive,
/// collapsible JSON tree.
///
/// # Parameters
/// - `root`               — the JSON value to render.
/// - `root_label`         — optional label printed above the tree (e.g. "Request").
/// - `state`              — current expansion state; queried on every render.
/// - `colors`             — pre-resolved theme colors.
/// - `position_id_prefix` — stable prefix for per-row `SavePosition` IDs used to
///   anchor the right-click context menu to the clicked row.
///   Must be unique per tree instance in the same window.
/// - `on_toggle`          — called with the event context and path of a clicked
///   collapsible node; dispatch an action to update state.
/// - `on_copy_json`       — called with the event context, path, value, and the
///   anchor position ID of the row when "Copy JSON" is activated via right-click.
///   The anchor ID can be used to position a context menu below the clicked row.
/// - `appearance`         — provides font families and sizes.
#[allow(clippy::too_many_arguments)]
pub fn render_json_tree(
    root: &serde_json::Value,
    root_label: Option<&str>,
    state: &JsonTreeState,
    colors: &JsonTreeColors,
    position_id_prefix: &str,
    on_toggle: Arc<ToggleFn>,
    on_toggle_string: Arc<ToggleStringFn>,
    on_copy_json: Arc<CopyJsonFn>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let font_family = appearance.ui_font_family();
    let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    // Optional section label.
    if let Some(label) = root_label {
        let label_text = Text::new_inline(label.to_owned(), font_family, TREE_FONT_SIZE)
            .with_color(colors.annotation)
            .soft_wrap(false)
            .finish();
        column.add_child(label_text);
    }

    // Render the root node and all visible descendants.
    render_value(
        root,
        vec![],
        0,
        None,
        state,
        colors,
        position_id_prefix,
        &on_toggle,
        &on_toggle_string,
        &on_copy_json,
        font_family,
        &mut column,
    );

    column.finish()
}

/// Recursively renders a JSON value into `column`, producing one row per
/// visible node.
///
/// `label` is `Some("key")` for object members and `Some("0")` for array
/// elements; it is `None` for the root call.
#[allow(clippy::too_many_arguments)]
fn render_value(
    value: &serde_json::Value,
    path: Vec<PathSegment>,
    depth: usize,
    label: Option<String>,
    state: &JsonTreeState,
    colors: &JsonTreeColors,
    position_id_prefix: &str,
    on_toggle: &Arc<ToggleFn>,
    on_toggle_string: &Arc<ToggleStringFn>,
    on_copy_json: &Arc<CopyJsonFn>,
    font_family: warpui::fonts::FamilyId,
    column: &mut Flex,
) {
    match value {
        serde_json::Value::Object(map) => {
            render_container_node(
                &format_object_annotation(map.len()),
                map.len(),
                value.clone(),
                path.clone(),
                depth,
                label,
                state,
                colors,
                position_id_prefix,
                on_toggle,
                on_copy_json,
                font_family,
                column,
            );

            if state.is_expanded(&path, depth) {
                for (key, child_value) in map {
                    let child_path = {
                        let mut p = path.clone();
                        p.push(PathSegment::Key(key.clone()));
                        p
                    };
                    render_value(
                        child_value,
                        child_path,
                        depth + 1,
                        Some(key.clone()),
                        state,
                        colors,
                        position_id_prefix,
                        on_toggle,
                        on_toggle_string,
                        on_copy_json,
                        font_family,
                        column,
                    );
                }
            }
        }

        serde_json::Value::Array(arr) => {
            render_container_node(
                &format_array_annotation(arr.len()),
                arr.len(),
                value.clone(),
                path.clone(),
                depth,
                label,
                state,
                colors,
                position_id_prefix,
                on_toggle,
                on_copy_json,
                font_family,
                column,
            );

            if state.is_expanded(&path, depth) {
                for (idx, child_value) in arr.iter().enumerate() {
                    let child_path = {
                        let mut p = path.clone();
                        p.push(PathSegment::Index(idx));
                        p
                    };
                    render_value(
                        child_value,
                        child_path,
                        depth + 1,
                        Some(idx.to_string()),
                        state,
                        colors,
                        position_id_prefix,
                        on_toggle,
                        on_toggle_string,
                        on_copy_json,
                        font_family,
                        column,
                    );
                }
            }
        }

        serde_json::Value::String(s) => {
            if is_long_string(s) {
                render_long_string_row(
                    path,
                    depth,
                    label,
                    s,
                    value.clone(),
                    state,
                    colors,
                    position_id_prefix,
                    on_toggle_string,
                    on_copy_json,
                    font_family,
                    column,
                );
            } else {
                let display = format!("\"{}\"", s);
                let text = Text::new_inline(display, font_family, TREE_FONT_SIZE)
                    .with_color(colors.string)
                    .soft_wrap(false)
                    .finish();
                render_scalar_row(
                    path,
                    depth,
                    label,
                    text,
                    value.clone(),
                    state,
                    colors,
                    position_id_prefix,
                    on_copy_json,
                    font_family,
                    column,
                );
            }
        }

        serde_json::Value::Number(n) => {
            let text = Text::new_inline(format_number(n), font_family, TREE_FONT_SIZE)
                .with_color(colors.number)
                .soft_wrap(false)
                .finish();
            render_scalar_row(
                path,
                depth,
                label,
                text,
                value.clone(),
                state,
                colors,
                position_id_prefix,
                on_copy_json,
                font_family,
                column,
            );
        }

        serde_json::Value::Bool(b) => {
            let display = if *b { "true" } else { "false" };
            let text = Text::new_inline(display, font_family, TREE_FONT_SIZE)
                .with_color(colors.bool)
                .soft_wrap(false)
                .finish();
            render_scalar_row(
                path,
                depth,
                label,
                text,
                value.clone(),
                state,
                colors,
                position_id_prefix,
                on_copy_json,
                font_family,
                column,
            );
        }

        serde_json::Value::Null => {
            let text = Text::new_inline("null", font_family, TREE_FONT_SIZE)
                .with_color(colors.null)
                .soft_wrap(false)
                .finish();
            render_scalar_row(
                path,
                depth,
                label,
                text,
                value.clone(),
                state,
                colors,
                position_id_prefix,
                on_copy_json,
                font_family,
                column,
            );
        }
    }
}

/// Renders an expandable/collapsible long string row.
///
/// Collapsed: shows first-line preview with ellipsis + right-pointing chevron.
/// Expanded: shows the full string with word-wrap + down-pointing chevron.
/// The chevron position mirrors container nodes so the column aligns consistently.
#[allow(clippy::too_many_arguments)]
fn render_long_string_row(
    path: Vec<PathSegment>,
    depth: usize,
    label: Option<String>,
    s: &str,
    value_for_copy: serde_json::Value,
    state: &JsonTreeState,
    colors: &JsonTreeColors,
    position_id_prefix: &str,
    on_toggle_string: &Arc<ToggleStringFn>,
    on_copy_json: &Arc<CopyJsonFn>,
    font_family: warpui::fonts::FamilyId,
    column: &mut Flex,
) {
    let anchor_id = format!("{position_id_prefix}:row:{path:?}");
    let is_expanded = state.is_string_expanded(&path);

    let icon = if is_expanded {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };

    let mut row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);

    // Indent spacer.
    row.add_child(indent_spacer(depth));

    // Chevron in the standard left position.
    row.add_child(
        ConstrainedBox::new(
            icon.to_warpui_icon(warp_core::ui::theme::Fill::Solid(colors.annotation))
                .finish(),
        )
        .with_width(CHEVRON_SIZE)
        .with_height(CHEVRON_SIZE)
        .finish(),
    );

    // Key/index label (if inside an object or array).
    if let Some(ref key) = label {
        let key_text = Text::new_inline(format!("{}:  ", key), font_family, TREE_FONT_SIZE)
            .with_color(colors.key)
            .soft_wrap(false)
            .finish();
        row.add_child(key_text);
    }

    // String value: preview when collapsed, full text when expanded.
    let string_element: Box<dyn Element> = if is_expanded {
        let display = format!("\"{}\"", s);
        Shrinkable::new(
            1.,
            Text::new(display, font_family, TREE_FONT_SIZE)
                .with_color(colors.string)
                .with_selectable(true)
                .finish(),
        )
        .finish()
    } else {
        let first_line = s.lines().next().unwrap_or("");
        let preview: String = first_line.chars().take(LONG_STRING_THRESHOLD).collect();
        let display = format!("\"{}\u{2026}\"", preview);
        Shrinkable::new(
            1.,
            Text::new_inline(display, font_family, TREE_FONT_SIZE)
                .with_color(colors.string)
                .soft_wrap(false)
                .finish(),
        )
        .finish()
    };
    row.add_child(string_element);

    let row_element = row.finish();

    let on_toggle_clone = on_toggle_string.clone();
    let path_for_toggle = path.clone();
    let on_copy_clone = on_copy_json.clone();
    let path_for_copy = path.clone();
    let anchor_for_copy = anchor_id.clone();
    // Stable handle reused across renders so click-state survives re-notify.
    let state_handle = state.mouse_state_for(&path);

    let hoverable = Hoverable::new(state_handle, move |_| row_element)
        .on_click(move |ctx, _app, _pos| {
            on_toggle_clone(ctx, path_for_toggle.clone());
        })
        .on_right_click(move |ctx, _app, _pos| {
            on_copy_clone(
                ctx,
                path_for_copy.clone(),
                value_for_copy.clone(),
                anchor_for_copy.clone(),
            );
        })
        .finish();

    column.add_child(SavePosition::new(hoverable, &anchor_id).finish());
}

/// Renders a collapsible object/array node row with a chevron expander.
///
/// Empty containers (0 keys/items) are non-interactive (no chevron, no click).
#[allow(clippy::too_many_arguments)]
fn render_container_node(
    annotation: &str,
    child_count: usize,
    value_for_copy: serde_json::Value,
    path: Vec<PathSegment>,
    depth: usize,
    label: Option<String>,
    state: &JsonTreeState,
    colors: &JsonTreeColors,
    position_id_prefix: &str,
    on_toggle: &Arc<ToggleFn>,
    on_copy_json: &Arc<CopyJsonFn>,
    font_family: warpui::fonts::FamilyId,
    column: &mut Flex,
) {
    // Empty containers have no chevron and are not interactive.
    let is_empty = child_count == 0;
    let is_expanded = state.is_expanded(&path, depth);

    let mut row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);

    // Indent spacer.
    row.add_child(indent_spacer(depth));

    // Chevron or placeholder.
    if is_empty {
        // Empty containers: no chevron, render a same-width placeholder.
        row.add_child(
            ConstrainedBox::new(Empty::new().finish())
                .with_width(CHEVRON_SIZE)
                .with_height(CHEVRON_SIZE)
                .finish(),
        );
    } else {
        let icon = if is_expanded {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };
        let icon_color = colors.annotation;
        row.add_child(
            ConstrainedBox::new(
                icon.to_warpui_icon(warp_core::ui::theme::Fill::Solid(icon_color))
                    .finish(),
            )
            .with_width(CHEVRON_SIZE)
            .with_height(CHEVRON_SIZE)
            .finish(),
        );
    }

    // Key/index label (if inside an object or array).
    if let Some(ref key) = label {
        let key_text = Text::new_inline(format!("{}:  ", key), font_family, TREE_FONT_SIZE)
            .with_color(colors.key)
            .soft_wrap(false)
            .finish();
        row.add_child(key_text);
    }

    // Type annotation.
    row.add_child(
        Shrinkable::new(
            1.,
            Text::new_inline(annotation.to_owned(), font_family, TREE_FONT_SIZE)
                .with_color(colors.annotation)
                .soft_wrap(false)
                .finish(),
        )
        .finish(),
    );

    let row_element = row.finish();

    // Wrap in a Hoverable for click (toggle) and right-click (copy JSON).
    if is_empty {
        // Non-interactive.
        column.add_child(row_element);
    } else {
        let anchor_id = format!("{position_id_prefix}:row:{path:?}");
        let on_toggle_clone = on_toggle.clone();
        let path_for_toggle = path.clone();
        let on_copy_clone = on_copy_json.clone();
        let path_for_copy = path.clone();
        let anchor_for_copy = anchor_id.clone();
        // Stable handle keyed by path — see `JsonTreeState::mouse_state_for`.
        let state_handle = state.mouse_state_for(&path);

        let row_for_hover = row_element;
        let hoverable = Hoverable::new(state_handle, move |_| row_for_hover)
            .on_click(move |ctx, _app, _pos| {
                on_toggle_clone(ctx, path_for_toggle.clone(), depth);
            })
            .on_right_click(move |ctx, _app, _pos| {
                on_copy_clone(
                    ctx,
                    path_for_copy.clone(),
                    value_for_copy.clone(),
                    anchor_for_copy.clone(),
                );
            })
            .finish();

        column.add_child(SavePosition::new(hoverable, &anchor_id).finish());
    }
}

/// Renders a scalar value row (string, number, bool, null).
#[allow(clippy::too_many_arguments)]
fn render_scalar_row(
    path: Vec<PathSegment>,
    depth: usize,
    label: Option<String>,
    value_element: Box<dyn Element>,
    value_for_copy: serde_json::Value,
    state: &JsonTreeState,
    colors: &JsonTreeColors,
    position_id_prefix: &str,
    on_copy_json: &Arc<CopyJsonFn>,
    font_family: warpui::fonts::FamilyId,
    column: &mut Flex,
) {
    let anchor_id = format!("{position_id_prefix}:row:{path:?}");

    let mut row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);

    // Indent spacer.
    row.add_child(indent_spacer(depth));

    // Placeholder where chevron would be, to keep column alignment.
    row.add_child(
        ConstrainedBox::new(Empty::new().finish())
            .with_width(CHEVRON_SIZE)
            .with_height(CHEVRON_SIZE)
            .finish(),
    );

    // Key/index label (if inside an object or array).
    if let Some(ref key) = label {
        let key_text = Text::new_inline(format!("{}:  ", key), font_family, TREE_FONT_SIZE)
            .with_color(colors.key)
            .soft_wrap(false)
            .finish();
        row.add_child(key_text);
    }

    // The typed value element.
    row.add_child(Shrinkable::new(1., value_element).finish());

    // Wrap in a Hoverable for right-click (copy JSON).
    let on_copy_clone = on_copy_json.clone();
    let path_for_copy = path.clone();
    let anchor_for_copy = anchor_id.clone();
    // Stable handle keyed by path — see `JsonTreeState::mouse_state_for`.
    let state_handle = state.mouse_state_for(&path);
    let row_element = row.finish();

    let hoverable = Hoverable::new(state_handle, move |_| row_element)
        .on_right_click(move |ctx, _app, _pos| {
            on_copy_clone(
                ctx,
                path_for_copy.clone(),
                value_for_copy.clone(),
                anchor_for_copy.clone(),
            );
        })
        .finish();

    column.add_child(SavePosition::new(hoverable, &anchor_id).finish());
}

/// Returns a fixed-width transparent spacer for the given indentation depth.
fn indent_spacer(depth: usize) -> Box<dyn Element> {
    if depth == 0 {
        Empty::new().finish()
    } else {
        ConstrainedBox::new(Empty::new().finish())
            .with_width(depth as f32 * INDENT_PX)
            .finish()
    }
}

#[cfg(test)]
#[path = "json_tree_tests.rs"]
mod tests;
