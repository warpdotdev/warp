//! Types for declarative pane header content.
//!
//! This module provides the infrastructure for backing views to declaratively
//! specify their header content without worrying about draggable behavior.

use warp_core::ui::theme::Fill;
use warpui::{
    elements::{DraggableState, MouseStateHandle},
    fonts::Properties,
    text_layout::ClipConfig,
    AppContext, Element,
};

/// Closure that renders sharing controls (share button, view-only indicator) for a pane header.
/// Accepts optional icon color and button size overrides.
type RenderSharingControlsFn<'a> =
    Box<dyn Fn(&AppContext, Option<Fill>, Option<f32>) -> Option<Box<dyn Element>> + 'a>;

/// Context provided to backing views when rendering header content.
///
/// This provides read-only access to appearance and configuration,
/// plus a helper for creating draggable spacer elements.
pub struct HeaderRenderContext<'a> {
    /// Shared draggable state for the header.
    pub draggable_state: DraggableState,
    /// Mouse state for the pane close button (owned by PaneHeader).
    pub close_button_mouse_state: MouseStateHandle,
    /// Mouse state for the pane overflow button (owned by PaneHeader).
    pub overflow_button_mouse_state: MouseStateHandle,
    /// SavePosition ID for the overflow button (needed for overlay anchoring).
    pub overflow_button_position_id: String,
    /// Whether the overflow menu has any items to display.
    pub has_overflow_items: bool,
    /// Extra left inset for the header's left-side controls, used to avoid
    /// overlap with a floating button overlay (e.g. the vertical tabs toggle).
    pub header_left_inset: f32,
    /// Closure that renders the sharing controls. Use [`Self::sharing_controls`] to call this.
    pub(super) render_sharing_controls_fn: RenderSharingControlsFn<'a>,
}

impl HeaderRenderContext<'_> {
    /// Renders the sharing controls (share button, view-only indicator) for this pane.
    /// Returns `None` if sharing is not enabled.
    pub fn sharing_controls(
        &self,
        app: &AppContext,
        icon_color: Option<Fill>,
        button_size: Option<f32>,
    ) -> Option<Box<dyn Element>> {
        (self.render_sharing_controls_fn)(app, icon_color, button_size)
    }
}

/// Render-time options for the header that apply to all header types.
/// These control visual aspects that were previously stored in PaneConfiguration.
#[derive(Default)]
pub struct StandardHeaderOptions {
    /// If true, always show header icons (close button, overflow menu) regardless of hover state.
    pub always_show_icons: bool,
    /// If true, a menu within the header is currently open, so icons should remain visible.
    pub has_open_menu: bool,
    /// Width for the left and right edge containers (default: 80.0).
    pub control_container_width: Option<f32>,
    /// If true, hides the close button even when in a split pane.
    /// Use for panes that should be closed via other means (e.g., accept/reject buttons).
    pub hide_close_button: bool,
    /// If true, shows the close button even when this is the only pane in the tab.
    /// Use for singleton utility panes where closing the pane is the expected way
    /// to return to the previous workspace context.
    pub show_close_button_when_not_split: bool,
}

impl StandardHeaderOptions {
    /// Default control container width.
    pub const DEFAULT_CONTROL_CONTAINER_WIDTH: f32 = 80.0;

    /// Returns the control container width, using default if not specified.
    pub fn control_container_width(&self) -> f32 {
        self.control_container_width
            .unwrap_or(Self::DEFAULT_CONTROL_CONTAINER_WIDTH)
    }
}

pub struct StandardHeader {
    /// The title text to display.
    pub title: String,
    /// Optional secondary title text (displayed after main title).
    pub title_secondary: Option<String>,
    /// Optional title text styling.
    pub title_style: Option<Properties>,
    /// Configuration for clipping the title text when it overflows.
    pub title_clip_config: ClipConfig,
    /// Optional max width for the title in pixels.
    /// If set, the title will be constrained to at most this width.
    pub title_max_width: Option<f32>,
    /// Optional element rendered immediately left of the title.
    pub left_of_title: Option<Box<dyn Element>>,
    /// Optional element rendered immediately right of the title.
    pub right_of_title: Option<Box<dyn Element>>,
    /// Optional element rendered left of the overflow menu button.
    pub left_of_overflow: Option<Box<dyn Element>>,
    /// Render options controlling visual behavior.
    pub options: StandardHeaderOptions,
}

/// Content that a backing view can return for its pane header.
///
/// The framework handles wrapping the content with draggable behavior,
/// so backing views don't need to worry about drag-and-drop.
pub enum HeaderContent {
    /// Standard header with title and optional customization points.
    ///
    /// The framework renders this with the standard pane header layout:
    /// `[toolbelt buttons] [left_of_title] [title] [right_of_title] ... [left_of_overflow] [overflow] [close]`
    ///
    /// The entire header is wrapped with draggable behavior.
    Standard(StandardHeader),

    /// Fully custom header content.
    ///
    /// The framework wraps the entire element with draggable behavior.
    /// Use this for views that need complete control over header rendering
    /// but still want automatic drag-and-drop support.
    Custom {
        /// The custom element to render.
        element: Box<dyn Element>,

        /// If `true`, the framework does NOT automatically wrap this with
        /// draggable behavior. The view is responsible for calling
        /// `PaneHeader::render_pane_header_draggable()` on the appropriate elements.
        ///
        /// Use this for views like CodeView that have a custom tab bar where only
        /// part of the header (the empty space) should be draggable.
        has_custom_draggable_behavior: bool,
    },
}

impl HeaderContent {
    /// Creates a simple standard header with just a title.
    ///
    /// Uses `ClipConfig::start()` and default options. This is the most common
    /// header configuration for panes that just need to display a title.
    pub fn simple(title: impl Into<String>) -> Self {
        Self::Standard(StandardHeader {
            title: title.into(),
            title_secondary: None,
            title_style: None,
            title_clip_config: ClipConfig::start(),
            title_max_width: None,
            left_of_title: None,
            right_of_title: None,
            left_of_overflow: None,
            options: StandardHeaderOptions::default(),
        })
    }
}
