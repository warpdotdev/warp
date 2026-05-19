use std::cell::OnceCell;
use std::sync::Arc;
use std::{fmt, vec};

use crate::safe_triangle::SafeTriangle;
use crate::themes::theme::Fill;
use crate::util::time_format::format_approx_duration_from_now_sentence_case;
use crate::{appearance::Appearance, ui_components::icons};
use chrono::{DateTime, Local};
use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F};
use warp_core::ui::color::blend::Blend;
use warpui::elements::{
    ChildAnchor, ClippedScrollStateHandle, ClippedScrollable, DropShadow, OffsetPositioning,
    ParentAnchor, ParentOffsetBounds, PositionedElementAnchor, PositionedElementOffsetBounds,
    ScrollTarget, ScrollToPositionMode, ScrollbarWidth, Stack,
};
use warpui::text_layout::ClipConfig;
use warpui::WindowId;
use warpui::{
    accessibility::{AccessibilityContent, ActionAccessibilityContent, WarpA11yRole},
    assets::asset_cache::AssetSource,
    elements::{
        Align, Border, CacheOption, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
        Dismiss, DispatchEventResult, Element, EventHandler, Flex, Hoverable, Icon, Image,
        MainAxisAlignment, MainAxisSize, MouseInBehavior, MouseStateHandle, ParentElement, Radius,
        Rect, SavePosition, Shrinkable, Text,
    },
    fonts::{FamilyId, Properties},
    keymap::FixedBinding,
    platform::Cursor,
    ui_components::components::UiComponent,
    Action, AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext,
};

pub const CHEVRON_RIGHT_ALIGN_SVG_PATH: &str = "bundled/svg/chevron-right-align.svg";

const SUBMENU_OVERLAP: f32 = 8.;
const MENU_VERTICAL_PADDING: f32 = 9.;
pub const MENU_ITEM_VERTICAL_PADDING: f32 = 5.;
pub const MENU_ITEM_HORIZONTAL_PADDING: f32 = 14.;
pub const SEPARATOR_VERTICAL_MARGIN: f32 = 4.;
const MINIMUM_MENU_ITEM_FONT_SIZE: f32 = 5.;
const PADDING_TO_ICON_SIZE_MULTIPLIER: f32 = 3.;
const MENU_ITEM_LEFT_PADDING_MULTIPLIER: f32 = 1.5;
const DROP_SHADOW_COLOR: ColorU = ColorU {
    r: 0,
    g: 0,
    b: 0,
    a: 48,
};
const SECONDARY_TEXT_RATIO: f32 = 0.9;

#[derive(Clone, Debug)]
/// At the current time, its not recommended to have more than 1 nested submenu due to
/// layout constraints.
pub struct SubMenu<A: Action + Clone = ()> {
    depth: usize,
    items: Vec<MenuItem<A>>,
    height: f32,
    /// The menu can have different items, including single item (standard menu) or a row of items,
    /// for example, for tab colors that are rendered in a single row. selected_row_index points
    /// to the specific item (or, in UI, specific row).
    selected_row_index: Option<usize>,
    /// Selected Item Field points to the actual idx within the row. For a Single item, it'll
    /// always be 0, or None for separator.
    selected_item_index: Option<usize>,
    /// The index of the item or row of items that is currently hovered.
    hovered_row_index: Option<usize>,
    /// Tracks whether the most recent selection movement came from pointer
    /// hover or from a keyboard/programmatic path.
    last_selection_source: Option<MenuSelectionSource>,
    /// Contains variant specific state.
    menu_variant: MenuVariant,
}

/// Menu contains the menu items and defines the logic for managing the actions and rendering of the
/// items. It has two variants (scrollable and fixed) which slightly change the way it's laid out.
/// TODO: In the future, if we keep bumping into more rendering changes here, it may make sense to
/// abstract the Menu functionality into a trait and have separte implementations for dropdown and
/// context menu.
pub struct Menu<A: Action + Clone = ()> {
    /// Whether or not the element should make the rest of the window unresponsive. All mouse events
    /// are handled by the [`Menu`] rather than being propagated further down in the element
    /// hierarchy.
    prevent_interaction_with_other_elements: bool,

    with_drop_shadow: bool,

    border: Option<Border>,

    /// All submenus must be the same width.
    submenu_width: f32,

    /// If present, this will be used to determine which way submenus should expand.
    origin: Option<Vector2F>,

    window_id: OnceCell<WindowId>,

    menu: SubMenu<A>,

    /// When set, the item at this index gets a subtle background highlight
    /// to indicate that a submenu/sidecar panel is being shown for it.
    submenu_being_shown_for_item_index: Option<usize>,

    /// If true, menu items won't fire hover events when covered by another element.
    ignore_hover_when_covered: bool,

    /// Optional safe triangle for suppressing intermediate hovers when moving toward a sidecar.
    safe_triangle: Option<SafeTriangle>,

    /// When true, the menu's container uses flat bottom corners (radius 0).
    /// Used when an external footer is rendered below the menu in the same visual container.
    flatten_bottom_corners: bool,

    /// Optional pinned element rendered below the scrollable items but inside the
    /// `Dismiss`, so clicks on it do not trigger the dismiss handler.
    pinned_footer_builder: Option<Box<PinnedFooterBuilder>>,

    /// Optional pinned element rendered above the scrollable items but inside the
    /// `Dismiss`, so clicks on it do not trigger the dismiss handler.
    pinned_header_builder: Option<Box<PinnedHeaderBuilder>>,

    /// Optional overrides for the depth-0 menu content padding.
    content_top_padding_override: Option<f32>,
    content_bottom_padding_override: Option<f32>,
    /// If false, selecting a menu item updates selection and emits menu events
    /// without dispatching the item's typed action directly from the menu.
    dispatch_item_actions: bool,
}

#[derive(Clone, Default)]
pub enum MenuVariant {
    #[default]
    Fixed,
    Scrollable(ClippedScrollStateHandle),
}

impl std::fmt::Debug for MenuVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixed => write!(f, "Fixed"),
            Self::Scrollable(_) => write!(f, "Scrollable"),
        }
    }
}

impl MenuVariant {
    pub fn scrollable() -> Self {
        Self::Scrollable(Default::default())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum MenuTooltipPosition {
    #[default]
    Right,
    Above,
}

pub type CustomMenuItemLabelFn =
    Arc<dyn Fn(bool, bool, &Appearance, &AppContext) -> Box<dyn Element>>;

pub type PinnedFooterBuilder = dyn Fn(&AppContext) -> Box<dyn Element>;
pub type PinnedHeaderBuilder = dyn Fn(&AppContext) -> Box<dyn Element>;

#[derive(Clone)]
pub enum MenuItemLabel {
    Text(String),
    // A label that can take up multiple lines. Doesn't support submenus yet.
    MultilineText {
        label: String,
        max_lines: usize,
    },
    Icon {
        path: &'static str,
        color: Fill,
        alt_text: String,
    },
    // Label with primary and secondary text, spaced apart (e.g., for keybinding hints).
    LabeledText {
        primary_text: String,
        secondary_text: String,
    },
    // Label with primary and secondary text stacked vertically (follows dropdown pattern).
    StackedText {
        primary_text: String,
        secondary_text: String,
    },
    Custom {
        builder: CustomMenuItemLabelFn,
        label: Option<String>,
    },
}

impl std::fmt::Debug for MenuItemLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(arg0) => f.debug_tuple("Text").field(arg0).finish(),
            Self::MultilineText { label, max_lines } => f
                .debug_struct("MultilineText")
                .field("label", label)
                .field("max_lines", max_lines)
                .finish(),
            Self::Icon {
                path,
                color,
                alt_text,
            } => f
                .debug_struct("Icon")
                .field("path", path)
                .field("color", color)
                .field("alt_text", alt_text)
                .finish(),
            Self::LabeledText {
                primary_text,
                secondary_text,
            } => f
                .debug_struct("LabeledText")
                .field("primary_text", primary_text)
                .field("secondary_text", secondary_text)
                .finish(),
            Self::StackedText {
                primary_text,
                secondary_text,
            } => f
                .debug_struct("StackedText")
                .field("primary_text", primary_text)
                .field("secondary_text", secondary_text)
                .finish(),
            Self::Custom { .. } => f.debug_tuple("Custom").finish(),
        }
    }
}

impl Default for MenuItemLabel {
    fn default() -> Self {
        Self::Text("".to_string())
    }
}

impl MenuItemLabel {
    fn label(&self) -> Option<&str> {
        match self {
            Self::Text(label) => Some(label),
            Self::MultilineText { label, .. } => Some(label),
            Self::StackedText { primary_text, .. } => Some(primary_text),
            Self::Custom { label, .. } => label.as_deref(),
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render(
        &self,
        primary_color: Fill,
        secondary_color: Fill,
        font_family: FamilyId,
        font_size: f32,
        _vertical_padding: f32, // for the future use cases
        horizontal_padding: f32,
        menu_width: f32,
        is_selected: bool,
        is_hovered: bool,
        clip_config: Option<ClipConfig>,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        match self {
            Self::Text(label) => {
                let mut text = Text::new_inline(label.clone(), font_family, font_size)
                    .with_color(primary_color.into())
                    .autosize_text(MINIMUM_MENU_ITEM_FONT_SIZE);
                if let Some(config) = clip_config {
                    text = text.with_clip(config).soft_wrap(false);
                }
                Shrinkable::new(4., text.finish()).finish()
            }
            Self::MultilineText { label, max_lines } => {
                let max_height_for_n_lines =
                    *max_lines as f32 * font_size * appearance.line_height_ratio();
                ConstrainedBox::new(
                    Text::new(label.clone(), font_family, font_size)
                        .with_color(primary_color.into())
                        .autosize_text(MINIMUM_MENU_ITEM_FONT_SIZE)
                        .soft_wrap(true)
                        .finish(),
                )
                .with_max_height(max_height_for_n_lines)
                .finish()
            }
            Self::Icon { path, color, .. } => {
                // TODO figure out the sizes
                let container = Container::new(Icon::new(path, *color).finish());
                ConstrainedBox::new(container.finish())
                    .with_height(horizontal_padding * PADDING_TO_ICON_SIZE_MULTIPLIER)
                    .with_width(horizontal_padding * PADDING_TO_ICON_SIZE_MULTIPLIER)
                    .finish()
            }
            Self::LabeledText {
                primary_text,
                secondary_text,
            } => {
                let mut row = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_main_axis_size(MainAxisSize::Max);
                row.add_child(
                    Shrinkable::new(
                        1.,
                        Text::new_inline(primary_text.clone(), font_family, font_size)
                            .with_color(primary_color.into())
                            .autosize_text(MINIMUM_MENU_ITEM_FONT_SIZE)
                            .finish(),
                    )
                    .finish(),
                );
                row.add_child(
                    Container::new(
                        Text::new_inline(
                            secondary_text.clone(),
                            font_family,
                            font_size * SECONDARY_TEXT_RATIO,
                        )
                        .with_color(secondary_color.into())
                        .autosize_text(MINIMUM_MENU_ITEM_FONT_SIZE)
                        .finish(),
                    )
                    .finish(),
                );
                let max_width = menu_width - (horizontal_padding * 2.0);
                ConstrainedBox::new(row.finish())
                    .with_max_width(max_width)
                    .finish()
            }
            Self::StackedText {
                primary_text,
                secondary_text,
            } => {
                // Create column layout for stacked text
                let mut column = Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_main_axis_alignment(MainAxisAlignment::Start);

                // Add primary text
                column.add_child(
                    Text::new_inline(primary_text.clone(), font_family, font_size)
                        .with_color(primary_color.into())
                        .autosize_text(MINIMUM_MENU_ITEM_FONT_SIZE)
                        .finish(),
                );

                // Add secondary text with spacing
                column.add_child(
                    Container::new(
                        Text::new_inline(
                            secondary_text.clone(),
                            font_family,
                            font_size * SECONDARY_TEXT_RATIO,
                        )
                        .with_color(secondary_color.into())
                        .autosize_text(MINIMUM_MENU_ITEM_FONT_SIZE)
                        .finish(),
                    )
                    .with_margin_top(2.)
                    .finish(),
                );

                let max_width = menu_width - (horizontal_padding * 2.0);
                Box::new(
                    ConstrainedBox::new(column.finish())
                        .with_max_width(max_width)
                        .with_min_width(max_width),
                )
            }
            Self::Custom { builder, .. } => {
                let width = menu_width - (horizontal_padding * 2.0);
                ConstrainedBox::new(builder(is_selected, is_hovered, appearance, app))
                    .with_width(width)
                    .finish()
            }
        }
    }
}

#[derive(Clone, Debug)]
struct RightSideLabel {
    text: String,
    font_properties: Properties,
}

#[derive(Clone, Default)]
pub struct MenuItemFields<A: Action + Clone> {
    element: MenuItemLabel,
    timestamp: Option<DateTime<Local>>,
    on_select_action: Option<A>,
    key_shortcut_label: Option<String>,
    has_submenu: bool,
    disabled: bool,
    mouse_state: MouseStateHandle,
    icon: Option<icons::Icon>,
    /// Path to a full-color image asset (e.g. `bundled/svg/file_type/rust.svg`).
    /// When set, the icon is rendered via [`Image::new`] preserving original colors.
    image_icon: Option<&'static str>,
    override_icon_color: Option<Fill>,
    override_text_color: Option<ColorU>,
    override_font_family: Option<FamilyId>,
    override_font_size: Option<f32>,
    highlight_on_hover: bool,
    no_interaction_on_hover: bool,
    indent: bool,
    vertical_padding_override: Option<f32>,
    horizontal_padding_override: Option<f32>,
    tooltip: Option<String>,
    tooltip_position: MenuTooltipPosition,
    right_side_label: Option<RightSideLabel>,
    right_side_icon: Option<(icons::Icon, Option<Fill>)>,
    /// Optional override for the background color
    /// hovered or selected. When `None`, the default hover/selected background
    /// from the theme is used (accent or dark overlay, depending on
    /// `highlight_on_hover`).
    override_hover_background_color: Option<Fill>,
    /// Optional override for the leading icon size in logical pixels. When
    /// `None`, the icon is sized to `appearance.ui_font_size()`.
    icon_size_override: Option<f32>,
    /// Optional clip config controlling how the label text is clipped when it
    /// would overflow the available width. Only applied to plain
    /// [`MenuItemLabel::Text`] labels — multiline/stacked/labeled/icon/custom
    /// variants ignore this field.
    clip_config: Option<ClipConfig>,
}

impl<A: Action + Clone> std::fmt::Debug for MenuItemFields<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MenuItemFields")
            .field("element", &self.element)
            .field("on_select_action", &self.on_select_action)
            .field("key_shortcut_label", &self.key_shortcut_label)
            .field("disabled", &self.disabled)
            .finish()
    }
}

impl<A: Action + Clone> MenuItemFields<A> {
    pub fn new<T: Into<String>>(label: T) -> Self {
        MenuItemFields {
            element: MenuItemLabel::Text(label.into()),
            timestamp: None,
            on_select_action: None,
            key_shortcut_label: None,
            disabled: false,
            mouse_state: Default::default(),
            icon: None,
            image_icon: None,
            override_icon_color: None,
            override_font_family: None,
            override_font_size: None,
            override_text_color: None,
            highlight_on_hover: true,
            indent: false,
            no_interaction_on_hover: false,
            vertical_padding_override: None,
            horizontal_padding_override: None,
            has_submenu: false,
            tooltip: None,
            tooltip_position: MenuTooltipPosition::default(),
            right_side_label: None,
            right_side_icon: None,
            override_hover_background_color: None,
            icon_size_override: None,
            clip_config: None,
        }
    }

    pub fn new_submenu<T: Into<String>>(label: T) -> Self {
        MenuItemFields {
            element: MenuItemLabel::Text(label.into()),
            timestamp: None,
            on_select_action: None,
            key_shortcut_label: None,
            disabled: false,
            mouse_state: Default::default(),
            icon: None,
            image_icon: None,
            override_icon_color: None,
            override_font_family: None,
            override_font_size: None,
            override_text_color: None,
            highlight_on_hover: true,
            indent: false,
            no_interaction_on_hover: false,
            vertical_padding_override: None,
            horizontal_padding_override: None,
            has_submenu: true,
            tooltip: None,
            tooltip_position: MenuTooltipPosition::default(),
            right_side_label: None,
            right_side_icon: None,
            override_hover_background_color: None,
            icon_size_override: None,
            clip_config: None,
        }
    }

    pub fn new_with_label<T: Into<String>>(text: T, label: T) -> Self {
        MenuItemFields {
            element: MenuItemLabel::LabeledText {
                primary_text: text.into(),
                secondary_text: label.into(),
            },
            timestamp: None,
            on_select_action: None,
            key_shortcut_label: None,
            disabled: false,
            mouse_state: Default::default(),
            icon: None,
            image_icon: None,
            override_icon_color: None,
            override_font_family: None,
            override_font_size: None,
            override_text_color: None,
            highlight_on_hover: true,
            indent: false,
            no_interaction_on_hover: false,
            vertical_padding_override: None,
            horizontal_padding_override: None,
            has_submenu: false,
            tooltip: None,
            tooltip_position: MenuTooltipPosition::default(),
            right_side_label: None,
            right_side_icon: None,
            override_hover_background_color: None,
            icon_size_override: None,
            clip_config: None,
        }
    }

    /// Creates a new menu item with vertically stacked
    /// This is useful for items that need both a title and description/subtitle,
    /// such as slash commands with their descriptions.
    pub fn new_with_stacked_label<T: Into<String>>(title: T, subtitle: T) -> Self {
        MenuItemFields {
            element: MenuItemLabel::StackedText {
                primary_text: title.into(),
                secondary_text: subtitle.into(),
            },
            timestamp: None,
            on_select_action: None,
            key_shortcut_label: None,
            disabled: false,
            mouse_state: Default::default(),
            icon: None,
            image_icon: None,
            override_icon_color: None,
            override_font_family: None,
            override_font_size: None,
            override_text_color: None,
            highlight_on_hover: true,
            indent: false,
            no_interaction_on_hover: false,
            vertical_padding_override: None,
            horizontal_padding_override: None,
            has_submenu: false,
            tooltip: None,
            tooltip_position: MenuTooltipPosition::default(),
            right_side_label: None,
            right_side_icon: None,
            override_hover_background_color: None,
            icon_size_override: None,
            clip_config: None,
        }
    }

    pub fn new_with_icon(icon_path: &'static str, icon_color: Fill, icon_label: String) -> Self {
        MenuItemFields {
            element: MenuItemLabel::Icon {
                path: icon_path,
                color: icon_color,
                alt_text: icon_label,
            },
            timestamp: None,
            on_select_action: None,
            key_shortcut_label: None,
            disabled: false,
            mouse_state: Default::default(),
            icon: None,
            image_icon: None,
            override_icon_color: None,
            override_font_family: None,
            override_font_size: None,
            override_text_color: None,
            highlight_on_hover: true,
            indent: false,
            no_interaction_on_hover: false,
            vertical_padding_override: None,
            horizontal_padding_override: None,
            has_submenu: false,
            tooltip: None,
            tooltip_position: MenuTooltipPosition::default(),
            right_side_label: None,
            right_side_icon: None,
            override_hover_background_color: None,
            icon_size_override: None,
            clip_config: None,
        }
    }

    pub fn new_multiline<T: Into<String>>(label: T, max_lines: usize) -> Self {
        MenuItemFields {
            element: MenuItemLabel::MultilineText {
                label: label.into(),
                max_lines,
            },
            timestamp: None,
            on_select_action: None,
            key_shortcut_label: None,
            disabled: false,
            mouse_state: Default::default(),
            icon: None,
            image_icon: None,
            override_icon_color: None,
            override_font_family: None,
            override_font_size: None,
            override_text_color: None,
            highlight_on_hover: true,
            indent: false,
            no_interaction_on_hover: false,
            vertical_padding_override: None,
            horizontal_padding_override: None,
            has_submenu: false,
            tooltip: None,
            tooltip_position: MenuTooltipPosition::default(),
            right_side_label: None,
            right_side_icon: None,
            override_hover_background_color: None,
            icon_size_override: None,
            clip_config: None,
        }
    }

    pub fn new_with_custom_label(builder: CustomMenuItemLabelFn, label: Option<String>) -> Self {
        MenuItemFields {
            element: MenuItemLabel::Custom { builder, label },
            timestamp: None,
            on_select_action: None,
            key_shortcut_label: None,
            disabled: false,
            mouse_state: Default::default(),
            icon: None,
            image_icon: None,
            override_icon_color: None,
            override_font_family: None,
            override_font_size: None,
            override_text_color: None,
            highlight_on_hover: true,
            indent: false,
            no_interaction_on_hover: false,
            vertical_padding_override: None,
            horizontal_padding_override: None,
            has_submenu: false,
            tooltip: None,
            tooltip_position: MenuTooltipPosition::default(),
            right_side_label: None,
            right_side_icon: None,
            override_hover_background_color: None,
            icon_size_override: None,
            clip_config: None,
        }
    }

    pub fn toggle_pane_action(is_maximized: bool) -> Self {
        Self::new(if is_maximized {
            "Minimize pane"
        } else {
            "Maximize pane"
        })
    }

    /// Creates a [`MenuItemFields`] where the `on_select_action` is of a
    /// specified type B.
    /// Ideally, we can do this with a `From` implementation but lack of
    /// impl speciailization makes it impossible today:
    /// https://github.com/rust-lang/rfcs/blob/master/text/1210-impl-specialization.md.
    pub fn with_different_on_select_action_type<B: Action + Clone>(
        self,
        on_select_action: Option<B>,
    ) -> MenuItemFields<B> {
        MenuItemFields {
            timestamp: self.timestamp,
            on_select_action,
            element: self.element,
            key_shortcut_label: self.key_shortcut_label,
            disabled: self.disabled,
            mouse_state: self.mouse_state,
            icon: self.icon,
            image_icon: self.image_icon,
            override_icon_color: self.override_icon_color,
            override_font_family: self.override_font_family,
            override_font_size: self.override_font_size,
            override_text_color: self.override_text_color,
            highlight_on_hover: self.highlight_on_hover,
            indent: self.indent,
            no_interaction_on_hover: self.no_interaction_on_hover,
            vertical_padding_override: self.vertical_padding_override,
            horizontal_padding_override: self.horizontal_padding_override,
            has_submenu: self.has_submenu,
            tooltip: self.tooltip,
            tooltip_position: self.tooltip_position,
            right_side_label: self.right_side_label,
            right_side_icon: self.right_side_icon,
            override_hover_background_color: self.override_hover_background_color,
            icon_size_override: self.icon_size_override,
            clip_config: self.clip_config,
        }
    }

    pub fn with_timestamp(mut self, timestamp: DateTime<Local>) -> Self {
        self.timestamp = Some(timestamp);
        self
    }

    pub fn on_select_action(&self) -> Option<&A> {
        self.on_select_action.as_ref()
    }

    pub fn no_highlight_on_hover(mut self) -> Self {
        self.highlight_on_hover = false;
        self
    }

    pub fn with_no_interaction_on_hover(mut self) -> Self {
        self.no_interaction_on_hover = true;
        self
    }

    pub fn with_padding_override(mut self, vertical_padding: f32, horizontal_padding: f32) -> Self {
        self.vertical_padding_override = Some(vertical_padding);
        self.horizontal_padding_override = Some(horizontal_padding);
        self
    }

    pub fn with_font_override(mut self, override_font_family: FamilyId) -> Self {
        self.override_font_family = Some(override_font_family);
        self
    }

    pub fn with_font_size_override(mut self, override_font_size: f32) -> Self {
        self.override_font_size = Some(override_font_size);
        self
    }

    pub fn with_on_select_action(mut self, action: A) -> Self {
        self.on_select_action = Some(action);
        self
    }

    pub fn with_override_text_color(mut self, override_text_color: impl Into<ColorU>) -> Self {
        self.override_text_color = Some(override_text_color.into());
        self
    }

    /// Overrides the background color rendered when this item is hovered or
    /// selected. Takes precedence over the default hover color (which depends
    /// on `highlight_on_hover`). Has no effect when `no_interaction_on_hover`
    /// is set, or when the item is disabled.
    pub fn with_override_hover_background_color(mut self, color: impl Into<Fill>) -> Self {
        self.override_hover_background_color = Some(color.into());
        self
    }

    /// Overrides the leading icon size (in logical pixels) for this item.
    /// The default is `appearance.ui_font_size()`.
    pub fn with_icon_size_override(mut self, size: f32) -> Self {
        self.icon_size_override = Some(size);
        self
    }

    pub fn with_key_shortcut_label(mut self, label: Option<impl Into<String>>) -> Self {
        self.key_shortcut_label = label.map(Into::into);
        self
    }

    pub fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn with_icon(mut self, icon: icons::Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Set a full-color image asset as the icon for this menu item.
    /// The image is rendered via [`Image::new`], preserving its original colors
    /// (e.g. for language logos from `bundled/svg/file_type/`).
    pub fn with_image_icon(mut self, path: &'static str) -> Self {
        self.image_icon = Some(path);
        self
    }

    pub fn with_indent(mut self) -> Self {
        self.indent = true;
        self
    }

    /// Set an override color for an icon added by [`Self::with_icon`]. The
    /// default is to match the text color, which is based on the hover state.
    pub fn with_override_icon_color(mut self, color: Fill) -> Self {
        self.override_icon_color = Some(color);
        self
    }

    /// Set a tooltip to display when hovering over this menu item.
    pub fn with_tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Set a [`ClipConfig`] that controls how the label text is clipped when
    /// it would overflow the available width. Only applied to plain
    /// [`MenuItemLabel::Text`] labels.
    pub fn with_clip_config(mut self, config: ClipConfig) -> Self {
        self.clip_config = Some(config);
        self
    }

    pub(crate) fn with_tooltip_position(mut self, position: MenuTooltipPosition) -> Self {
        self.tooltip_position = position;
        self
    }

    /// Adds a right-aligned secondary label with custom font properties to this menu item.
    pub fn with_right_side_label(
        mut self,
        label: impl Into<String>,
        font_properties: Properties,
    ) -> Self {
        self.right_side_label = Some(RightSideLabel {
            text: label.into(),
            font_properties,
        });
        self
    }

    pub fn with_right_side_icon(mut self, icon: icons::Icon) -> Self {
        self.right_side_icon = Some((icon, None));
        self
    }

    pub fn into_item(self) -> MenuItem<A> {
        MenuItem::Item(self)
    }

    pub fn label(&self) -> &str {
        self.element.label().unwrap_or_default()
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub fn get_a11y_text(&self) -> &str {
        match &self.element {
            MenuItemLabel::Text(label) | MenuItemLabel::MultilineText { label, .. } => label,
            MenuItemLabel::Icon { alt_text, .. } => alt_text,
            MenuItemLabel::LabeledText { primary_text, .. } => primary_text,
            MenuItemLabel::StackedText { primary_text, .. } => primary_text,
            MenuItemLabel::Custom { label, .. } => label.as_deref().unwrap_or(""),
        }
    }

    pub fn override_font_family(&self) -> Option<FamilyId> {
        self.override_font_family
    }

    pub fn icon(&self) -> Option<icons::Icon> {
        self.icon
    }

    pub fn override_icon_color(&self) -> Option<Fill> {
        self.override_icon_color
    }

    fn render_icon(&self, appearance: &Appearance, color: Fill) -> Option<Box<dyn Element>> {
        let icon_size = appearance.ui_font_size();
        if let Some(path) = self.image_icon {
            return Some(
                Shrinkable::new(
                    1.,
                    Container::new(
                        ConstrainedBox::new(
                            Image::new(AssetSource::Bundled { path }, CacheOption::BySize).finish(),
                        )
                        .with_width(icon_size)
                        .with_height(icon_size)
                        .finish(),
                    )
                    .with_margin_right(icon_size / 2.)
                    .finish(),
                )
                .finish(),
            );
        }
        if let Some(icon) = self.icon {
            let icon_size = self
                .icon_size_override
                .unwrap_or_else(|| appearance.ui_font_size());
            let icon_color = self.override_icon_color.unwrap_or(color);
            Some(
                Shrinkable::new(
                    1.,
                    Container::new(
                        ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
                            .with_width(icon_size)
                            .with_height(icon_size)
                            .finish(),
                    )
                    .with_margin_right(icon_size / 2.)
                    .finish(),
                )
                .finish(),
            )
        } else {
            None
        }
    }

    fn render_right_side_label(
        &self,
        appearance: &Appearance,
        color: ColorU,
    ) -> Option<Box<dyn Element>> {
        let label = self.right_side_label.as_ref()?;
        Some(
            Shrinkable::new(
                1.,
                Align::new(
                    Text::new_inline(
                        label.text.clone(),
                        appearance.ui_builder().ui_font_family(),
                        appearance.ui_builder().ui_font_size() * 0.75,
                    )
                    .with_color(color)
                    .with_style(label.font_properties)
                    .finish(),
                )
                .right()
                .finish(),
            )
            .finish(),
        )
    }

    fn render_key_shortcut(
        &self,
        appearance: &Appearance,
        color: ColorU,
    ) -> Option<Box<dyn Element>> {
        if let Some(key_shortcut) = self.key_shortcut_label.clone() {
            Some(
                Shrinkable::new(
                    1.,
                    Align::new(
                        Text::new_inline(
                            key_shortcut,
                            appearance.ui_builder().ui_font_family(),
                            appearance.ui_builder().ui_font_size(),
                        )
                        .with_color(color)
                        .finish(),
                    )
                    .right()
                    .finish(),
                )
                .finish(),
            )
        } else {
            None
        }
    }

    fn render_right_side_icon(
        &self,
        appearance: &Appearance,
        color: Fill,
    ) -> Option<Box<dyn Element>> {
        let (icon, override_color) = self.right_side_icon.as_ref()?;
        let icon_size = self
            .icon_size_override
            .unwrap_or_else(|| appearance.ui_font_size());
        let icon_color = override_color.unwrap_or(color);
        Some(
            Shrinkable::new(
                1.,
                Container::new(
                    ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
                        .with_width(icon_size)
                        .with_height(icon_size)
                        .finish(),
                )
                .with_margin_left(icon_size / 2.)
                .finish(),
            )
            .finish(),
        )
    }

    fn render_right_aligned_chevron(
        &self,
        appearance: &Appearance,
        icon_color: Fill,
    ) -> Box<dyn Element> {
        let icon_size = appearance.ui_font_size() * 1.2;
        let icon =
            ConstrainedBox::new(Icon::new(CHEVRON_RIGHT_ALIGN_SVG_PATH, icon_color).finish())
                .with_height(icon_size)
                .with_width(icon_size)
                .finish();
        Shrinkable::new(
            2.,
            Align::new(Container::new(icon).finish()).right().finish(),
        )
        .finish()
    }

    fn render_right_aligned_time_estimation(
        &self,
        timestamp: &DateTime<Local>,
        font_family: FamilyId,
        font_size: f32,
        text_background_color: Fill,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Shrinkable::new(
            1.,
            Align::new(self.render_time_estimation(
                timestamp,
                font_family,
                font_size,
                text_background_color,
                appearance,
            ))
            .right()
            .finish(),
        )
        .finish()
    }

    pub fn render_time_estimation(
        &self,
        timestamp: &DateTime<Local>,
        font_family: FamilyId,
        font_size: f32,
        text_background_color: Fill,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let est_time_ago = format_approx_duration_from_now_sentence_case(*timestamp);
        Text::new_inline(est_time_ago, font_family, font_size)
            .with_color(theme.sub_text_color(text_background_color).into())
            .finish()
    }

    #[allow(clippy::too_many_arguments)]
    fn render(
        &self,
        menu_background_color: Fill,
        depth: usize,
        row_index: usize,
        item_index: usize,
        dispatch_item_actions: bool,
        is_selected: bool,
        ignore_hover_when_covered: bool,
        safe_zone_suppresses_hover: bool,
        submenu_being_shown_for_item: bool,
        appearance: &Appearance,
        vertical_padding: f32,
        horizontal_padding: f32,
        menu_width: f32,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let vertical_padding = self.vertical_padding_override.unwrap_or(vertical_padding);
        let horizontal_padding = self
            .horizontal_padding_override
            .unwrap_or(horizontal_padding);
        let mut ret = Hoverable::new(self.mouse_state.clone(), |state| {
            let is_hovered = state.is_hovered() && !safe_zone_suppresses_hover;
            let is_hovered_or_selected = is_hovered || is_selected;
            let default_hover_background = if self.highlight_on_hover {
                theme.accent_button_color()
            } else {
                theme.dark_overlay()
            };
            let hover_background = self
                .override_hover_background_color
                .unwrap_or(default_hover_background);
            let background_color =
                if is_hovered_or_selected && !self.disabled && !self.no_interaction_on_hover {
                    Some(hover_background)
                } else if submenu_being_shown_for_item {
                    // Preserve prior behavior: the submenu-open state always uses the
                    // accent color (or the explicit override), regardless of
                    // `highlight_on_hover`.
                    Some(
                        self.override_hover_background_color
                            .unwrap_or_else(|| theme.accent_button_color()),
                    )
                } else {
                    None
                };
            let text_background_color = match background_color {
                Some(overlay) => menu_background_color.blend(&overlay),
                None => menu_background_color,
            };

            let primary_color = if self.disabled {
                theme.disabled_text_color(text_background_color)
            } else if let Some(color) = self.override_text_color {
                color.into()
            } else {
                theme.main_text_color(text_background_color)
            };
            let secondary_color = appearance
                .theme()
                .disabled_text_color(text_background_color);

            let mut label_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

            if let Some(icon) = self.render_icon(appearance, primary_color) {
                label_row.add_child(icon);
            }

            let font_family = self
                .override_font_family
                .unwrap_or_else(|| appearance.ui_builder().ui_font_family());
            let font_size = self
                .override_font_size
                .unwrap_or_else(|| appearance.ui_builder().ui_font_size());

            let label_element = self.element.render(
                primary_color,
                secondary_color,
                font_family,
                font_size,
                vertical_padding,
                horizontal_padding,
                menu_width,
                is_selected,
                state.is_hovered(),
                self.clip_config,
                appearance,
                app,
            );

            if matches!(self.element, MenuItemLabel::MultilineText { .. }) {
                // In multiline labels, the timestamp is positioned underneath the label.
                let mut content_column = Flex::column()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_child(label_element);
                if let Some(timestamp) = &self.timestamp {
                    content_column.add_child(
                        Container::new(self.render_time_estimation(
                            timestamp,
                            font_family,
                            font_size - 1.,
                            text_background_color,
                            appearance,
                        ))
                        .with_margin_top(2.)
                        .finish(),
                    );
                }
                label_row.add_child(Shrinkable::new(1., content_column.finish()).finish());
            } else {
                label_row.add_child(label_element);

                if self.has_submenu {
                    label_row
                        .add_child(self.render_right_aligned_chevron(appearance, primary_color));
                } else if let Some(right_label) =
                    self.render_right_side_label(appearance, secondary_color.into())
                {
                    label_row.add_child(right_label);
                } else if let Some(key_shortcut) =
                    self.render_key_shortcut(appearance, secondary_color.into())
                {
                    label_row.add_child(key_shortcut);
                } else if let Some(timestamp) = &self.timestamp {
                    label_row.add_child(self.render_right_aligned_time_estimation(
                        timestamp,
                        font_family,
                        font_size,
                        text_background_color,
                        appearance,
                    ));
                }

                if let Some(right_icon) = self.render_right_side_icon(appearance, primary_color) {
                    label_row.add_child(right_icon);
                }
            }

            // If menu item doesn't have an icon but we want to indent it, add left padding so it aligns with menu items that do have icons
            let left_padding = if self.indent {
                horizontal_padding + (appearance.ui_font_size() * MENU_ITEM_LEFT_PADDING_MULTIPLIER)
            } else {
                horizontal_padding
            };

            let horizontal_alignment =
                if matches!(self.element, MenuItemLabel::MultilineText { .. }) {
                    CrossAxisAlignment::Start
                } else {
                    CrossAxisAlignment::Center
                };

            let container = Container::new(
                label_row
                    .with_main_axis_alignment(MainAxisAlignment::SpaceEvenly)
                    .with_cross_axis_alignment(horizontal_alignment)
                    .finish(),
            )
            .with_padding_top(vertical_padding)
            .with_padding_bottom(vertical_padding)
            .with_padding_left(left_padding)
            .with_padding_right(horizontal_padding);

            let container_element = if let Some(background_color) = background_color {
                container.with_background(background_color).finish()
            } else {
                container.finish()
            };

            // Render tooltip if present and hovered
            if let Some(tooltip_text) = &self.tooltip {
                if state.is_hovered() {
                    let tooltip_element = appearance
                        .ui_builder()
                        .tool_tip(tooltip_text.clone())
                        .build()
                        .finish();
                    let positioning = match self.tooltip_position {
                        MenuTooltipPosition::Right => OffsetPositioning::offset_from_parent(
                            vec2f(4., 0.),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::MiddleRight,
                            ChildAnchor::MiddleLeft,
                        ),
                        MenuTooltipPosition::Above => OffsetPositioning::offset_from_parent(
                            vec2f(0., -4.),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::TopMiddle,
                            ChildAnchor::BottomMiddle,
                        ),
                    };
                    let mut stack = Stack::new();
                    stack.add_child(container_element);
                    // Use add_positioned_child instead of add_positioned_overlay_child
                    // to prevent the tooltip from intercepting mouse events and causing
                    // hover state flickering on the parent menu item.
                    stack.add_positioned_child(tooltip_element, positioning);
                    return stack.finish();
                }
            }

            container_element
        });

        let has_submenu = self.has_submenu;
        let is_enabled = !self.disabled;
        ret = ret.on_hover(move |is_hovered, ctx, _, position| {
            if has_submenu && is_enabled {
                ctx.dispatch_typed_action(if is_hovered {
                    MenuAction::HoverSubmenuWithChildren(
                        depth,
                        SelectAction::Index {
                            row: row_index,
                            item: item_index,
                        },
                    )
                } else {
                    MenuAction::UnhoverSubmenuParent(depth)
                });
            } else if is_hovered {
                // Only dispatch on hover-in, not hover-out. Dispatching on
                // hover-out sends a stale row_index that can overwrite the
                // correct hovered_row_index set by a subsequently-entered
                // submenu parent. Continuous position tracking for the
                // safe-triangle is handled by the on_mouse_in handler below.
                ctx.dispatch_typed_action(MenuAction::HoverSubmenuLeafNode {
                    depth,
                    row_index,
                    position,
                });
            }
        });

        let on_select_action = self.on_select_action.clone();

        if !self.disabled {
            if !self.no_interaction_on_hover {
                ret = ret.with_cursor(Cursor::PointingHand);
            }
            ret = ret.on_click(move |ctx, _, _| {
                if let Some(action) = &on_select_action {
                    ctx.dispatch_typed_action(MenuAction::Select(SelectAction::Index {
                        row: row_index,
                        item: item_index,
                    }));
                    if dispatch_item_actions {
                        ctx.dispatch_typed_action(action.clone());
                    }
                    ctx.dispatch_typed_action(MenuAction::Close(true));
                }
            });
        }

        let mut element = ret.finish();

        // For leaf items (no submenu), also emit hover on every mouse move within bounds,
        // not just on hover-state transitions. This ensures ItemHovered fires when moving over
        // an already-selected item, and keeps the safe triangle's position tracking current
        // even when the mouse passes over disabled items.
        if !has_submenu {
            let mouse_in_behavior = if ignore_hover_when_covered {
                Some(MouseInBehavior {
                    fire_on_synthetic_events: true,
                    fire_when_covered: false,
                })
            } else {
                None
            };
            element = EventHandler::new(element)
                .on_mouse_in(
                    move |ctx, _app, pos| {
                        ctx.dispatch_typed_action(MenuAction::HoverSubmenuLeafNode {
                            depth,
                            row_index,
                            position: pos,
                        });
                        DispatchEventResult::PropagateToParent
                    },
                    mouse_in_behavior,
                )
                .finish();
        }

        SavePosition::new(element, self.label()).finish()
    }
}

#[derive(Debug, Clone)]
pub enum MenuItem<A: Action + Clone = ()> {
    Item(MenuItemFields<A>),
    /// Separator item in the menu is used to separate two sections. Note that the way it's
    /// rendered makes it only look nice if the MenuVariant used is Fixed (vs Scrollable).
    Separator,
    /// One row with multiple items. This is used for example for tab colors in the context menu.
    ItemsRow {
        items: Vec<MenuItemFields<A>>,
    },
    /// Nested menu item. Expands on hover.
    Submenu {
        fields: MenuItemFields<A>,
        menu: SubMenu<A>,
    },
    /// Header item used to group items. May or may not be clickable.
    Header {
        fields: MenuItemFields<A>,
        clickable: bool,
        right_side_fields: Option<MenuItemFields<A>>,
    },
}

impl<A: Action + Clone> MenuItem<A> {
    #[deprecated(note = "Submenus are not ready for use yet.")]
    pub fn submenu<T: Into<String>>(label: T, items: Vec<MenuItem<A>>) -> Self {
        let menu = SubMenu::new(items);
        MenuItem::Submenu {
            fields: MenuItemFields::new_submenu(label),
            menu,
        }
    }

    fn items_len(&self) -> Option<usize> {
        match self {
            MenuItem::Item(_) => Some(1),
            MenuItem::Separator => None,
            MenuItem::ItemsRow { items } => Some(items.len()),
            MenuItem::Submenu { menu, .. } => {
                // This includes the label as well.
                Some(menu.items_len() + 1)
            }
            MenuItem::Header { clickable, .. } => Some(if *clickable { 1 } else { 0 }),
        }
    }

    pub fn item_on_select_action(&self) -> Option<&A> {
        match self {
            MenuItem::Item(fields) => fields.on_select_action.as_ref(),
            _ => None,
        }
    }

    pub fn selectable(&self) -> bool {
        match self {
            // Whether it's selectable depends only on whether the item is disabled or not.
            MenuItem::Item(fields) => !fields.disabled && !fields.no_interaction_on_hover,
            // For items row, right now, we assume that it's either entire row that's selectable or
            // not (so for simplicity, either all items are selectable or none of them).
            // TODO be smarter about it
            MenuItem::ItemsRow { items } => items
                .iter()
                .any(|i| !i.disabled && !i.no_interaction_on_hover),
            // Separator is simply a non-selectable option in the menu list.
            MenuItem::Separator => false,
            MenuItem::Submenu { menu, .. } => !menu.items.is_empty(),
            MenuItem::Header { clickable, .. } => *clickable,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render(
        &self,
        menu_background_color: Fill,
        depth: usize,
        row_index: usize,
        selected_item_in_row: Option<usize>,
        dispatch_item_actions: bool,
        is_row_selected: bool,
        ignore_hover_when_covered: bool,
        safe_zone_suppresses_hover: bool,
        submenu_being_shown_for_item: bool,
        appearance: &Appearance,
        menu_width: f32,
        app: &AppContext,
    ) -> Box<dyn Element> {
        match self {
            MenuItem::Item(fields) => fields.render(
                menu_background_color,
                depth,
                row_index,
                0, // there's only 1 item
                dispatch_item_actions,
                is_row_selected,
                ignore_hover_when_covered,
                safe_zone_suppresses_hover,
                submenu_being_shown_for_item,
                appearance,
                MENU_ITEM_VERTICAL_PADDING,
                MENU_ITEM_HORIZONTAL_PADDING,
                menu_width,
                app,
            ),
            MenuItem::ItemsRow { items } => {
                let horizontal_padding = ((menu_width - (MENU_ITEM_HORIZONTAL_PADDING * 2.))
                    / (items.len() as f32)
                    / 5.)
                    .round();
                let items_row = Flex::row()
                    .with_children(items.iter().enumerate().map(|(item_idx, fields)| {
                        fields.render(
                            menu_background_color,
                            depth,
                            row_index,
                            item_idx,
                            dispatch_item_actions,
                            is_row_selected && selected_item_in_row.unwrap_or_default() == item_idx,
                            ignore_hover_when_covered,
                            safe_zone_suppresses_hover,
                            submenu_being_shown_for_item,
                            appearance,
                            MENU_ITEM_VERTICAL_PADDING,
                            horizontal_padding,
                            menu_width,
                            app,
                        )
                    }))
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_main_axis_size(MainAxisSize::Max);

                Container::new(items_row.finish())
                    .with_margin_top(SEPARATOR_VERTICAL_MARGIN)
                    .with_margin_bottom(SEPARATOR_VERTICAL_MARGIN)
                    .with_padding_left(MENU_ITEM_HORIZONTAL_PADDING)
                    .with_padding_right(MENU_ITEM_HORIZONTAL_PADDING)
                    .finish()
            }
            MenuItem::Separator => Container::new(
                ConstrainedBox::new(
                    Rect::new()
                        .with_background(appearance.theme().disabled_ui_text_color())
                        .finish(),
                )
                .with_height(1.)
                .finish(),
            )
            .with_margin_top(SEPARATOR_VERTICAL_MARGIN)
            .with_margin_bottom(SEPARATOR_VERTICAL_MARGIN)
            .with_padding_left(MENU_ITEM_HORIZONTAL_PADDING)
            .with_padding_right(MENU_ITEM_HORIZONTAL_PADDING)
            .finish(),
            MenuItem::Submenu { fields, .. } => {
                fields.render(
                    menu_background_color,
                    depth,
                    row_index,
                    0, // there's only 1 item
                    dispatch_item_actions,
                    is_row_selected,
                    ignore_hover_when_covered,
                    safe_zone_suppresses_hover,
                    submenu_being_shown_for_item,
                    appearance,
                    MENU_ITEM_VERTICAL_PADDING,
                    MENU_ITEM_HORIZONTAL_PADDING,
                    menu_width,
                    app,
                )
            }
            MenuItem::Header {
                fields,
                clickable,
                right_side_fields,
            } => {
                let mut fields = fields.clone();
                let mut right_side_fields = right_side_fields.clone();
                if !*clickable {
                    fields = fields.with_no_interaction_on_hover();
                    right_side_fields =
                        right_side_fields.map(|fields| fields.with_no_interaction_on_hover());
                }

                if let Some(right_side_fields) = right_side_fields {
                    Flex::row()
                        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_children([
                            ConstrainedBox::new(fields.render(
                                menu_background_color,
                                depth,
                                row_index,
                                0, // there's only 1 item
                                dispatch_item_actions,
                                is_row_selected,
                                ignore_hover_when_covered,
                                safe_zone_suppresses_hover,
                                submenu_being_shown_for_item,
                                appearance,
                                MENU_ITEM_VERTICAL_PADDING,
                                MENU_ITEM_HORIZONTAL_PADDING,
                                menu_width,
                                app,
                            ))
                            .with_max_width(menu_width / 2.)
                            .finish(),
                            ConstrainedBox::new(right_side_fields.render(
                                menu_background_color,
                                depth,
                                row_index,
                                0, // there's only 1 item
                                dispatch_item_actions,
                                is_row_selected,
                                ignore_hover_when_covered,
                                safe_zone_suppresses_hover,
                                submenu_being_shown_for_item,
                                appearance,
                                MENU_ITEM_VERTICAL_PADDING,
                                MENU_ITEM_HORIZONTAL_PADDING,
                                menu_width,
                                app,
                            ))
                            .with_max_width(menu_width / 2.)
                            .finish(),
                        ])
                        .finish()
                } else {
                    fields.render(
                        menu_background_color,
                        depth,
                        row_index,
                        0, // there's only 1 item
                        dispatch_item_actions,
                        is_row_selected,
                        ignore_hover_when_covered,
                        safe_zone_suppresses_hover,
                        submenu_being_shown_for_item,
                        appearance,
                        MENU_ITEM_VERTICAL_PADDING,
                        MENU_ITEM_HORIZONTAL_PADDING,
                        menu_width,
                        app,
                    )
                }
            }
        }
    }
}

pub const DEFAULT_WIDTH: f32 = 186.;
const DEFAULT_HEIGHT: f32 = 140.;

#[derive(Copy, Clone, Debug)]
pub enum SelectAction {
    Previous,
    Next,
    Index { row: usize, item: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuSelectionSource {
    KeyboardOrProgrammatic,
    Pointer,
}

#[derive(Debug)]
pub enum MenuAction {
    Select(SelectAction),
    OpenSubmenu,
    /// Fires when the mouse leaves the Menu item containing a submenu.
    UnhoverSubmenuParent(usize),
    HoverSubmenuWithChildren(usize, SelectAction),
    /// Fires when the mouse enters a submenu item with no children
    HoverSubmenuLeafNode {
        depth: usize,
        row_index: usize,
        position: Vector2F,
    },
    CloseSubmenu(usize),
    Close(bool),
    Enter,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new(
            "up",
            MenuAction::Select(SelectAction::Previous),
            id!(Menu::<()>::ui_name()),
        ),
        FixedBinding::new(
            "down",
            MenuAction::Select(SelectAction::Next),
            id!(Menu::<()>::ui_name()),
        ),
        FixedBinding::new("right", MenuAction::OpenSubmenu, id!(Menu::<()>::ui_name())),
        FixedBinding::new(
            "escape",
            MenuAction::Close(false),
            id!(Menu::<()>::ui_name()),
        ),
        FixedBinding::new("enter", MenuAction::Enter, id!(Menu::<()>::ui_name())),
    ]);
}

#[derive(Debug, Copy, Clone)]
pub enum Event {
    ItemSelected,
    // via_select_item is true when you close a menu by selecting an item (by click or enter)
    // TODO: improve this logic since this is not the ideal solution
    Close { via_select_item: bool },
    ItemHovered,
}

impl<A: Action + Clone> SubMenu<A> {
    pub fn new(mut items: Vec<MenuItem<A>>) -> Self {
        Self::increment_depth(&mut items);
        Self {
            depth: 0,
            items,
            selected_row_index: None,
            selected_item_index: None,
            hovered_row_index: None,
            last_selection_source: None,
            height: DEFAULT_HEIGHT,
            menu_variant: Default::default(),
        }
    }

    fn increment_depth(items: &mut [MenuItem<A>]) {
        for item in items.iter_mut() {
            if let MenuItem::Submenu { menu, .. } = item {
                menu.depth += 1;
                Self::increment_depth(&mut menu.items);
            }
        }
    }

    pub fn with_height(&mut self, height: f32) -> &mut Self {
        self.height = height;
        self
    }

    pub fn with_menu_variant(&mut self, menu_variant: MenuVariant) -> &mut Self {
        self.menu_variant = menu_variant;
        self
    }

    pub fn reset_selection(&mut self, ctx: &mut ViewContext<Menu<A>>) {
        self.selected_row_index = None;
        self.selected_item_index = None;
        self.last_selection_source = None;
        ctx.notify();
    }

    pub fn selected_item(&self) -> Option<MenuItem<A>> {
        self.selected_row_index
            .and_then(|index| self.items.get(index).cloned())
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected_row_index
    }

    pub fn hovered_index(&self) -> Option<usize> {
        self.hovered_row_index
    }

    pub fn last_selection_source(&self) -> Option<MenuSelectionSource> {
        self.last_selection_source
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn items(&self) -> &[MenuItem<A>] {
        &self.items
    }

    pub fn items_len(&self) -> usize {
        self.items.len()
    }

    /// Select the menu item at the given index. If the index is out of bounds, this clears the selection.
    pub fn set_selected_by_index(&mut self, selected_index: usize, ctx: &mut ViewContext<Menu<A>>) {
        if selected_index < self.items.len() {
            self.select(
                SelectAction::Index {
                    row: selected_index,
                    item: 0,
                },
                ctx,
            );
        } else {
            self.reset_selection(ctx);
        }
    }

    fn select_internal(&mut self, action: SelectAction) {
        let (selected_row_index, selected_item_index) =
            match (action, self.selected_row_index, self.selected_item_index) {
                (SelectAction::Index { row, item }, _, _) => (row, item),
                (SelectAction::Previous, Some(item_idx), Some(field_idx)) if field_idx > 0 => {
                    // Currently we're assuming that for a selectable item row all items in a row are
                    // also selectable, and so - we just need to reduce the index of fields.
                    // TODO be smarter about it
                    (item_idx, field_idx.saturating_sub(1))
                }
                (SelectAction::Previous, Some(item_idx), _) => {
                    // Getting the previous element that is selectable. We compute the indexes of
                    // elements first, and then reverse the iterator and look for a first selectable
                    // element after the currently selected one.
                    // If there's none, for some reason, we default to the currently selected one.

                    let items_to_skip = self.items.len().saturating_sub(item_idx);
                    let selected_row_index = self
                        .items
                        .iter()
                        .enumerate()
                        .rev()
                        .cycle()
                        .skip(items_to_skip)
                        .take(self.items.len())
                        .find(|(_, item)| item.selectable())
                        .map(|(index, _)| index)
                        .unwrap_or(item_idx);
                    (
                        selected_row_index,
                        self.items[selected_row_index]
                            .items_len()
                            .map_or(0, |l| l.saturating_sub(1)),
                    )
                }
                (SelectAction::Next, Some(item_idx), Some(field_idx))
                    if field_idx
                        < self.items[item_idx]
                            .items_len()
                            .map_or(0, |l| l.saturating_sub(1)) =>
                {
                    (item_idx, field_idx.saturating_add(1))
                }
                (SelectAction::Next, Some(item_idx), _) => {
                    // Getting the next element that is selectable. We compute the indexes of
                    // elements first, and look for a first selectable element after the currently
                    // selected one.
                    // If there's none, for some reason, we default to the currently selected one.

                    let items_to_skip = item_idx.saturating_add(1);
                    (
                        self.items
                            .iter()
                            .enumerate()
                            .cycle()
                            .skip(items_to_skip)
                            .take(self.items.len())
                            .find(|(_, item)| item.selectable())
                            .map(|(index, _)| index)
                            .unwrap_or(item_idx),
                        0,
                    )
                }
                _ => (0, 0),
            };

        self.selected_row_index = Some(selected_row_index);
        self.selected_item_index = Some(selected_item_index);
        self.hovered_row_index = Some(selected_row_index);
    }

    fn select_with_source(
        &mut self,
        action: SelectAction,
        selection_source: MenuSelectionSource,
        ctx: &mut ViewContext<Menu<A>>,
    ) {
        self.select_internal(action);
        self.last_selection_source = Some(selection_source);
        if matches!(
            selection_source,
            MenuSelectionSource::KeyboardOrProgrammatic
        ) {
            if let MenuVariant::Scrollable(scroll_state) = &self.menu_variant {
                scroll_state.scroll_to_position(ScrollTarget {
                    position_id: Self::save_position_id(self.depth),
                    mode: ScrollToPositionMode::FullyIntoView,
                });
            }
        }
        ctx.emit(Event::ItemSelected);
        ctx.notify();
    }

    fn select(&mut self, action: SelectAction, ctx: &mut ViewContext<Menu<A>>) {
        self.select_with_source(action, MenuSelectionSource::KeyboardOrProgrammatic, ctx);
    }

    #[cfg(test)]
    fn selected_submenu(&self) -> Option<&SubMenu<A>> {
        let selected_row_index = self.selected_row_index?;
        match self.items.get(selected_row_index)? {
            MenuItem::Submenu { menu, .. } => Some(menu),
            _ => None,
        }
    }

    fn selected_submenu_mut(&mut self) -> Option<&mut SubMenu<A>> {
        let selected_row_index = self.selected_row_index?;
        match self.items.get_mut(selected_row_index)? {
            MenuItem::Submenu { menu, .. } => Some(menu),
            _ => None,
        }
    }

    fn active_menu_mut(&mut self) -> &mut Self {
        match self.selected_row_index {
            Some(selected_row_index)
                if matches!(
                    self.items.get(selected_row_index),
                    Some(MenuItem::Submenu { menu, .. }) if menu.selected_row_index.is_some()
                ) =>
            {
                let Some(MenuItem::Submenu { menu, .. }) = self.items.get_mut(selected_row_index)
                else {
                    unreachable!("checked selected submenu above");
                };
                menu.active_menu_mut()
            }
            _ => self,
        }
    }

    fn select_first_selectable(&mut self, ctx: &mut ViewContext<Menu<A>>) -> bool {
        let Some((row, _)) = self
            .items
            .iter()
            .enumerate()
            .find(|(_, item)| item.selectable())
        else {
            return false;
        };
        self.select(SelectAction::Index { row, item: 0 }, ctx);
        true
    }

    fn open_selected_submenu(&mut self, ctx: &mut ViewContext<Menu<A>>) -> bool {
        let Some(submenu) = self.active_menu_mut().selected_submenu_mut() else {
            return false;
        };
        submenu.select_first_selectable(ctx)
    }

    fn selected_action_for_enter(&mut self, ctx: &mut ViewContext<Menu<A>>) -> Option<A> {
        let active_menu = self.active_menu_mut();
        let selected_row_index = active_menu.selected_row_index?;
        let selected_item_index = active_menu.selected_item_index.unwrap_or_default();
        match active_menu.items.get_mut(selected_row_index)? {
            MenuItem::Item(fields) => fields.on_select_action.clone(),
            MenuItem::Separator => None,
            MenuItem::ItemsRow { items } => items
                .get(selected_item_index)
                .and_then(|fields| fields.on_select_action.clone()),
            MenuItem::Submenu { menu, .. } => {
                menu.select_first_selectable(ctx);
                None
            }
            MenuItem::Header {
                fields, clickable, ..
            } => {
                if *clickable {
                    fields.on_select_action.clone()
                } else {
                    None
                }
            }
        }
    }

    /// Select the menu item with the given name. If no such item exists, this clears the selection.
    /// Returns true if the item was found and selected, false otherwise.
    pub fn set_selected_by_name<S>(
        &mut self,
        selected_item: S,
        ctx: &mut ViewContext<Menu<A>>,
    ) -> bool
    where
        S: AsRef<str>,
    {
        let selected_item = selected_item.as_ref();
        if let Some(index) = self.items.iter().position(|item| match item {
            MenuItem::Item(MenuItemFields {
                element: MenuItemLabel::Text(label) | MenuItemLabel::MultilineText { label, .. },
                ..
            }) => selected_item == label,
            _ => false,
        }) {
            self.select(
                SelectAction::Index {
                    row: index,
                    item: 0,
                },
                ctx,
            );
            true
        } else {
            self.reset_selection(ctx);
            false
        }
    }

    /// Select the menu item whose on-select action equals the given action. If no such item exists,
    /// this clears the selection.
    ///
    /// This is primarily useful when items are dynamically generated and correspond to some backing data that's captured by the action.
    pub fn set_selected_by_action(&mut self, action: &A, ctx: &mut ViewContext<Menu<A>>)
    where
        A: PartialEq,
    {
        let selected_index = self.items.iter().position(|item| match item {
            MenuItem::Item(MenuItemFields {
                on_select_action: Some(item_action),
                ..
            }) => item_action == action,
            _ => false,
        });
        match selected_index {
            Some(index) => {
                self.select(
                    SelectAction::Index {
                        row: index,
                        item: 0,
                    },
                    ctx,
                );
            }
            None => self.reset_selection(ctx),
        }
    }

    /// Currently, this assumes only one Menu with any depth submenu can be open
    /// at a time. We need a way to uniquely identify Menus.
    /// TODO(asweet): Allow multiple submenus to be open at a time.
    fn save_position_id(depth: usize) -> String {
        format!("submenu_{depth}")
    }

    #[allow(clippy::too_many_arguments)]
    fn render_submenus(
        &self,
        submenu_width: f32,
        menu_background_color: Fill,
        selected_row: Option<usize>,
        selected_item: Option<usize>,
        dispatch_item_actions: bool,
        ignore_hover_when_covered: bool,
        safe_zone_anchor_row: Option<usize>,
        submenu_being_shown_for_item_index: Option<usize>,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Vec<Box<dyn Element>> {
        let height = self.height;
        let depth = self.depth;
        match &self.menu_variant {
            MenuVariant::Fixed => {
                let mut menus = vec![Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_children(self.items.iter().enumerate().map(
                        |(index, item)| -> Box<dyn Element> {
                            let is_selected = selected_row == Some(index);
                            // When the safe zone is active, suppress hover highlighting on
                            // non-anchor rows so intermediate items don't flash as the
                            // mouse moves toward the sidecar.
                            let safe_zone_suppresses_hover =
                                safe_zone_anchor_row.is_some_and(|anchor| anchor != index);
                            let submenu_being_shown_for_item =
                                submenu_being_shown_for_item_index == Some(index);
                            let item = item.render(
                                menu_background_color,
                                depth,
                                index,
                                selected_item,
                                dispatch_item_actions,
                                is_selected,
                                ignore_hover_when_covered,
                                safe_zone_suppresses_hover,
                                submenu_being_shown_for_item,
                                appearance,
                                submenu_width,
                                app,
                            );
                            let item = if is_selected {
                                let save_position = Self::save_position_id(depth);
                                SavePosition::new(item, &save_position).finish()
                            } else {
                                item
                            };
                            Container::new(item).finish()
                        },
                    ))
                    .finish()];
                let Some(selected_row) = self.selected_item() else {
                    return menus;
                };
                let MenuItem::Submenu { menu, .. } = selected_row else {
                    return menus;
                };

                menus.extend(menu.render_submenus(
                    submenu_width,
                    menu_background_color,
                    menu.selected_row_index,
                    menu.selected_item_index,
                    dispatch_item_actions,
                    ignore_hover_when_covered,
                    None,
                    None,
                    appearance,
                    app,
                ));
                menus
            }
            MenuVariant::Scrollable(scroll_state) => {
                let items = self.items.clone();
                // TODO(asweet): handle scrollable, or make scrollable nested items invalid
                let column_of_items = Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_children(items.iter().enumerate().map(|(index, item)| {
                        let is_selected = selected_row == Some(index);
                        let safe_zone_suppresses_hover =
                            safe_zone_anchor_row.is_some_and(|anchor| anchor != index);
                        let submenu_being_shown_for_item =
                            submenu_being_shown_for_item_index == Some(index);
                        let item = item.render(
                            menu_background_color,
                            depth,
                            index,
                            selected_item,
                            dispatch_item_actions,
                            is_selected,
                            ignore_hover_when_covered,
                            safe_zone_suppresses_hover,
                            submenu_being_shown_for_item,
                            appearance,
                            submenu_width,
                            app,
                        );
                        let item = if is_selected {
                            let save_position = Self::save_position_id(depth);
                            SavePosition::new(item, &save_position).finish()
                        } else {
                            item
                        };
                        Container::new(item).finish()
                    }));

                vec![ConstrainedBox::new(
                    ClippedScrollable::vertical(
                        scroll_state.clone(),
                        column_of_items.finish(),
                        ScrollbarWidth::Auto,
                        appearance.theme().nonactive_ui_detail().into(),
                        appearance.theme().active_ui_detail().into(),
                        warpui::elements::Fill::None,
                    )
                    .with_overlayed_scrollbar()
                    .finish(),
                )
                .with_max_height(height)
                .finish()]
            }
        }
    }
}

impl<A: Action + Clone> Menu<A> {
    pub fn new() -> Self {
        Self {
            prevent_interaction_with_other_elements: false,
            border: None,
            with_drop_shadow: false,
            origin: None,
            window_id: Default::default(),
            submenu_width: DEFAULT_WIDTH,
            menu: SubMenu::new(vec![]),
            submenu_being_shown_for_item_index: None,
            ignore_hover_when_covered: false,
            safe_triangle: None,
            flatten_bottom_corners: false,
            pinned_footer_builder: None,
            pinned_header_builder: None,
            content_top_padding_override: None,
            content_bottom_padding_override: None,
            dispatch_item_actions: true,
        }
    }

    fn window_id(&self, app: &AppContext) -> Option<WindowId> {
        if let Some(window_id) = self.window_id.get() {
            return Some(*window_id);
        }
        if let Some(window_id) = app.windows().active_window() {
            let _ = self.window_id.set(window_id);
        }
        self.window_id.get().copied()
    }

    pub fn with_border(mut self, border: Border) -> Self {
        self.border = Some(border);
        self
    }

    pub fn with_width(mut self, width: f32) -> Self {
        self.submenu_width = width;
        self
    }

    pub fn with_menu_variant(mut self, menu_variant: MenuVariant) -> Self {
        self.menu.with_menu_variant(menu_variant);
        self
    }

    /// Prevents interactions with any other elements outside of the [`Menu`]. All events are
    /// handled by this element and are _not_ propagated further down the element hierarchy.
    pub fn prevent_interaction_with_other_elements(mut self) -> Self {
        self.prevent_interaction_with_other_elements = true;
        self
    }

    pub fn with_drop_shadow(mut self) -> Self {
        self.with_drop_shadow = true;
        self
    }

    /// Prevent menu item clicks and Enter from dispatching their typed actions
    /// directly. Selection and close events still occur normally.
    pub fn without_item_action_dispatch(mut self) -> Self {
        self.dispatch_item_actions = false;
        self
    }

    /// If true, menu items won't fire hover events when covered by another element.
    /// This is useful when a sidecar panel overlays the menu.
    pub fn with_ignore_hover_when_covered(mut self) -> Self {
        self.ignore_hover_when_covered = true;
        self
    }

    /// Enable safe triangle tracking for suppressing intermediate hovers
    /// when moving toward a sidecar/submenu.
    pub fn with_safe_triangle(mut self) -> Self {
        self.safe_triangle = Some(SafeTriangle::new());
        self
    }

    /// Set or clear the target rect for the safe triangle. The rect should be the
    /// bounding box of the sidecar/submenu panel that the user is moving toward.
    pub fn set_safe_zone_target(&mut self, rect: Option<RectF>) {
        if let Some(st) = &mut self.safe_triangle {
            st.set_target_rect(rect);
        }
    }

    /// Origin is only used to determine which direction submenus should expand in.
    pub fn set_origin(&mut self, origin: Option<Vector2F>) {
        self.origin = origin;
    }

    pub fn set_submenu_being_shown_for_item_index(&mut self, index: Option<usize>) {
        self.submenu_being_shown_for_item_index = index;
    }

    pub fn set_width(&mut self, width: f32) {
        self.submenu_width = width;
    }

    pub fn set_border(&mut self, border: Option<Border>) {
        self.border = border;
    }

    pub fn set_flatten_bottom_corners(&mut self, flatten: bool) {
        self.flatten_bottom_corners = flatten;
    }

    /// Set a pinned footer element
    /// **inside** the `Dismiss`, so clicks on it never trigger the dismiss handler.
    /// Use `on_click` (not `on_mouse_down`) on interactive elements in the footer.
    pub fn set_pinned_footer_builder<F>(&mut self, builder: F)
    where
        F: Fn(&AppContext) -> Box<dyn Element> + 'static,
    {
        self.pinned_footer_builder = Some(Box::new(builder));
    }

    pub fn clear_pinned_footer_builder(&mut self) {
        self.pinned_footer_builder = None;
    }

    /// Set a pinned header element rendered above the scrollable items
    /// **inside** the `Dismiss`, so clicks on it never trigger the dismiss handler.
    pub fn set_pinned_header_builder<F>(&mut self, builder: F)
    where
        F: Fn(&AppContext) -> Box<dyn Element> + 'static,
    {
        self.pinned_header_builder = Some(Box::new(builder));
    }

    pub fn clear_pinned_header_builder(&mut self) {
        self.pinned_header_builder = None;
    }

    pub fn set_content_padding_overrides(
        &mut self,
        top_padding: Option<f32>,
        bottom_padding: Option<f32>,
    ) {
        self.content_top_padding_override = top_padding;
        self.content_bottom_padding_override = bottom_padding;
    }

    pub fn set_height(&mut self, height: f32) {
        self.menu.with_height(height);
    }

    pub fn set_items(
        &mut self,
        items: impl IntoIterator<Item = MenuItem<A>>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.menu.items = items.into_iter().collect();
        // No need to ctx.notify, since reset_selection will.
        self.reset_selection(ctx);
    }

    #[allow(dead_code)]
    pub fn add_item(&mut self, item: MenuItem<A>) {
        self.menu.items.push(item);
    }

    pub fn add_items(&mut self, items: impl IntoIterator<Item = MenuItem<A>>) {
        self.menu.items.extend(items);
    }

    pub fn reset_selection(&mut self, ctx: &mut ViewContext<Self>) {
        self.menu.reset_selection(ctx)
    }

    pub fn selected_item(&self) -> Option<MenuItem<A>> {
        self.menu.selected_item()
    }

    pub fn is_empty(&self) -> bool {
        self.menu.is_empty()
    }

    pub fn items(&self) -> &[MenuItem<A>] {
        self.menu.items()
    }

    pub fn items_len(&self) -> usize {
        self.menu.items_len()
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.menu.selected_index()
    }

    pub fn hovered_index(&self) -> Option<usize> {
        self.menu.hovered_index()
    }

    pub fn last_selection_source(&self) -> Option<MenuSelectionSource> {
        self.menu.last_selection_source()
    }

    /// Select the menu item at the given index. If the index is out of bounds, this clears the selection.
    pub fn set_selected_by_index(&mut self, selected_index: usize, ctx: &mut ViewContext<Self>) {
        self.menu.set_selected_by_index(selected_index, ctx);
    }

    /// Select the menu item with the given name. If no such item exists, this clears the selection.
    /// Returns true if the item was found and selected, false otherwise.
    pub fn set_selected_by_name<S>(&mut self, selected_item: S, ctx: &mut ViewContext<Self>) -> bool
    where
        S: AsRef<str>,
    {
        self.menu.set_selected_by_name(selected_item, ctx)
    }

    /// Select the menu item whose on-select action equals the given action. If no such item exists,
    /// this clears the selection.
    ///
    /// This is primarily useful when items are dynamically generated and correspond to some backing data that's captured by the action.
    pub fn set_selected_by_action(&mut self, action: &A, ctx: &mut ViewContext<Self>)
    where
        A: PartialEq,
    {
        self.menu.set_selected_by_action(action, ctx)
    }

    fn select(&mut self, action: SelectAction, ctx: &mut ViewContext<Self>) {
        self.menu.select(action, ctx);
    }

    pub fn select_previous(&mut self, ctx: &mut ViewContext<Self>) {
        self.select(SelectAction::Previous, ctx);
    }

    pub fn select_next(&mut self, ctx: &mut ViewContext<Self>) {
        self.select(SelectAction::Next, ctx);
    }

    /// Select the first selectable item in the menu. No-op if no item is selectable.
    pub fn select_first(&mut self, ctx: &mut ViewContext<Self>) {
        self.menu.select_first_selectable(ctx);
    }

    #[cfg(test)]
    pub fn mimic_confirm(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(item) = self.selected_item() {
            match item {
                MenuItem::Item(MenuItemFields {
                    on_select_action: Some(on_select_action),
                    ..
                }) => {
                    ctx.dispatch_typed_action(&on_select_action);
                }
                MenuItem::ItemsRow { items } => {
                    if let Some(on_select_action) = self
                        .menu
                        .selected_item_index
                        .and_then(|i| items.get(i))
                        .and_then(|item| item.on_select_action().cloned())
                    {
                        ctx.dispatch_typed_action(&on_select_action);
                    }
                }
                _ => {}
            }
        }
    }
}

impl<A: Action + Clone> Default for SubMenu<A> {
    fn default() -> Self {
        Self::new(vec![])
    }
}

impl<A: Action + Clone> Default for Menu<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: Action + Clone> Entity for SubMenu<A> {
    type Event = Event;
}

impl<A: Action + Clone> Entity for Menu<A> {
    type Event = Event;
}

impl<A: Action + Clone> SubMenu<A> {
    fn action_accessibility_contents(
        &mut self,
        action: &MenuAction,
        _: &mut ViewContext<Menu<A>>,
    ) -> ActionAccessibilityContent {
        use ActionAccessibilityContent::*;
        use MenuAction::*;

        match action {
            Select(_) => {
                let menu_item = match self.selected_item() {
                    Some(item) => match item {
                        MenuItem::Item(fields) => format!("{} Selected", fields.get_a11y_text()),
                        MenuItem::ItemsRow { items } => {
                            let selected_item_text = items
                                .get(self.selected_item_index.unwrap_or_default())
                                .map_or_else(|| "", |item| item.get_a11y_text());
                            format!("{selected_item_text} Selected")
                        }
                        MenuItem::Separator => String::from(""),
                        MenuItem::Submenu { fields, .. } => {
                            format!("{} Expanded", fields.get_a11y_text())
                        }
                        MenuItem::Header { fields, .. } => {
                            format!("{} Selected", fields.get_a11y_text())
                        }
                    },
                    None => String::from(""),
                };

                let instructions = if matches!(self.selected_item(), Some(MenuItem::Submenu { .. }))
                {
                    "Press the up key or the down key to select a menu item. Press the right key to open the submenu"
                } else {
                    "Press the up key or the down key to select a menu item"
                };

                Custom(AccessibilityContent::new(
                    menu_item,
                    instructions,
                    WarpA11yRole::TextRole,
                ))
            }
            OpenSubmenu => Custom(AccessibilityContent::new(
                String::from("Submenu Expanded"),
                "Press the right key to open the selected submenu",
                WarpA11yRole::TextRole,
            )),
            CloseSubmenu(_) => Custom(AccessibilityContent::new(
                String::from("Submenu Closed"),
                "Removing focus from a submenu will close the submenu",
                WarpA11yRole::TextRole,
            )),
            Close(_) => Custom(AccessibilityContent::new(
                String::from("Menu Closed"),
                "Press the escape key to close the menu",
                WarpA11yRole::TextRole,
            )),
            Enter => Custom(AccessibilityContent::new(
                String::from("Action Selected"),
                "Press the enter key to execute the selected menu item action",
                WarpA11yRole::TextRole,
            )),
            HoverSubmenuLeafNode { .. }
            | UnhoverSubmenuParent(_)
            | HoverSubmenuWithChildren(_, _) => ActionAccessibilityContent::Empty,
        }
    }

    fn handle_action(
        &mut self,
        action: &MenuAction,
        dispatch_item_actions: bool,
        ctx: &mut ViewContext<Menu<A>>,
    ) {
        match action {
            MenuAction::HoverSubmenuWithChildren(depth, selection) => {
                if *depth != self.depth {
                    return;
                }
                let selection = *selection;
                self.select_with_source(selection, MenuSelectionSource::Pointer, ctx);
            }
            MenuAction::UnhoverSubmenuParent(depth) => {
                if *depth != self.depth {
                    return;
                }
                self.handle_action(
                    &MenuAction::CloseSubmenu(self.depth),
                    dispatch_item_actions,
                    ctx,
                );
            }
            MenuAction::HoverSubmenuLeafNode {
                depth, row_index, ..
            } => {
                if *depth != self.depth {
                    return;
                }
                self.hovered_row_index = Some(*row_index);
                ctx.emit(Event::ItemHovered);
            }
            MenuAction::Select(selection) => self.active_menu_mut().select(*selection, ctx),
            MenuAction::OpenSubmenu => {
                self.open_selected_submenu(ctx);
            }
            MenuAction::CloseSubmenu(depth) => {
                if *depth != self.depth {
                    return;
                }
                self.reset_selection(ctx);
            }
            MenuAction::Close(via_select_item) => ctx.emit(Event::Close {
                via_select_item: *via_select_item,
            }),
            MenuAction::Enter => {
                if let Some(action) = self.selected_action_for_enter(ctx) {
                    if dispatch_item_actions {
                        ctx.dispatch_typed_action(&action);
                    } else {
                        ctx.emit(Event::ItemSelected);
                    }
                    ctx.emit(Event::Close {
                        via_select_item: true,
                    });
                }
            }
        }
    }
}

impl<A: Action + Clone> TypedActionView for Menu<A> {
    type Action = MenuAction;

    fn action_accessibility_contents(
        &mut self,
        action: &MenuAction,
        ctx: &mut ViewContext<Self>,
    ) -> ActionAccessibilityContent {
        self.menu.action_accessibility_contents(action, ctx)
    }

    fn handle_action(&mut self, action: &MenuAction, ctx: &mut ViewContext<Self>) {
        if let MenuAction::HoverSubmenuLeafNode { position, .. } = action {
            if let Some(st) = &mut self.safe_triangle {
                if st.should_suppress_hover(*position) {
                    return;
                }
                st.update_position(*position);
            }
        }

        self.menu
            .handle_action(action, self.dispatch_item_actions, ctx)
    }
}

impl<A: Action + Clone> SubMenu<A> {
    fn should_reverse_layout(
        &self,
        window: Option<WindowId>,
        origin: Option<Vector2F>,
        submenu_width: f32,
        // Including the main menu
        num_submenus: usize,
        app: &AppContext,
    ) -> bool {
        if num_submenus <= 1 {
            return false;
        }
        let extra_menus_width =
            ((num_submenus - 1) as f32 * (submenu_width - SUBMENU_OVERLAP)) + SUBMENU_OVERLAP;
        let total_width = submenu_width + extra_menus_width;
        let (Some(window), Some(origin)) = (window, origin) else {
            return false;
        };
        let Some(window) = app.windows().platform_window(window) else {
            return false;
        };
        let full_menu_end_x = origin.x() + total_width;
        full_menu_end_x >= window.size().x()
    }

    #[allow(clippy::too_many_arguments)]
    fn render(
        &self,
        border: Option<Border>,
        submenu_width: f32,
        with_drop_shadow: bool,
        origin: Option<Vector2F>,
        window_id: Option<WindowId>,
        prevent_interaction_with_other_elements: bool,
        dispatch_item_actions: bool,
        ignore_hover_when_covered: bool,
        safe_zone_anchor_row: Option<usize>,
        submenu_being_shown_for_item_index: Option<usize>,
        flatten_bottom_corners: bool,
        pinned_footer_builder: Option<&PinnedFooterBuilder>,
        pinned_header_builder: Option<&PinnedHeaderBuilder>,
        content_top_padding_override: Option<f32>,
        content_bottom_padding_override: Option<f32>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let selected_row_index = self.selected_row_index;
        let selected_item_index = self.selected_item_index;

        let background_color = appearance.theme().surface_2();
        let submenus = self.render_submenus(
            submenu_width,
            background_color,
            selected_row_index,
            selected_item_index,
            dispatch_item_actions,
            ignore_hover_when_covered,
            safe_zone_anchor_row,
            submenu_being_shown_for_item_index,
            appearance,
            app,
        );
        // TODO(asweet): Currently, this is based on the _current number of submenus_
        // instead of the maximum. We should likely change this.
        let num_submenus = submenus.len();

        let should_reverse_layout =
            self.should_reverse_layout(window_id, origin, submenu_width, num_submenus, app);

        let mut row = Flex::row();
        let mut stack = Stack::new();
        submenus
            .into_iter()
            .enumerate()
            .for_each(|(depth, submenu)| {
                let corner_radius = if flatten_bottom_corners && depth == 0 {
                    CornerRadius::with_top(Radius::Pixels(5.))
                } else {
                    CornerRadius::with_all(Radius::Pixels(5.))
                };

                // At depth 0, place pinned header/footer inside the styled container
                // so they inherit the menu box background, border, and corner radius.
                let (content, top_padding, bottom_padding) = if depth == 0 {
                    let has_header = pinned_header_builder.is_some();
                    let has_footer = pinned_footer_builder.is_some();
                    if has_header || has_footer {
                        let mut col = Flex::column();
                        if let Some(header_builder) = pinned_header_builder {
                            col.add_child(header_builder(app));
                        }
                        col.add_child(submenu);
                        if let Some(footer_builder) = pinned_footer_builder {
                            col.add_child(footer_builder(app));
                        }
                        let top_padding = content_top_padding_override.unwrap_or(if has_header {
                            0.
                        } else {
                            MENU_VERTICAL_PADDING
                        });
                        let bottom_padding =
                            content_bottom_padding_override.unwrap_or(if has_footer {
                                0.
                            } else {
                                MENU_VERTICAL_PADDING
                            });
                        (
                            col.finish() as Box<dyn Element>,
                            top_padding,
                            bottom_padding,
                        )
                    } else {
                        (
                            submenu,
                            content_top_padding_override.unwrap_or(MENU_VERTICAL_PADDING),
                            content_bottom_padding_override.unwrap_or(MENU_VERTICAL_PADDING),
                        )
                    }
                } else {
                    (submenu, MENU_VERTICAL_PADDING, MENU_VERTICAL_PADDING)
                };

                let mut container = Container::new(content)
                    .with_padding_top(top_padding)
                    .with_padding_bottom(bottom_padding)
                    .with_background(background_color)
                    .with_corner_radius(corner_radius);

                if let Some(border) = border {
                    container = container.with_border(border);
                }

                let mut menu = Container::new(
                    ConstrainedBox::new(container.finish())
                        .with_width(submenu_width)
                        .finish(),
                );

                if with_drop_shadow {
                    menu = menu.with_drop_shadow(DropShadow::new_with_standard_offset_and_spread(
                        DROP_SHADOW_COLOR,
                    ));
                }

                if depth == 0 {
                    row.add_child(menu.finish());
                } else {
                    let saved_position_id = Self::save_position_id(depth - 1);

                    stack.add_positioned_overlay_child(
                        menu.finish(),
                        OffsetPositioning::offset_from_save_position_element(
                            saved_position_id,
                            vec2f(
                                if should_reverse_layout {
                                    SUBMENU_OVERLAP
                                } else {
                                    -SUBMENU_OVERLAP
                                },
                                -MENU_VERTICAL_PADDING,
                            ),
                            PositionedElementOffsetBounds::WindowByPosition,
                            if should_reverse_layout {
                                PositionedElementAnchor::TopLeft
                            } else {
                                PositionedElementAnchor::TopRight
                            },
                            if should_reverse_layout {
                                ChildAnchor::TopRight
                            } else {
                                ChildAnchor::TopLeft
                            },
                        ),
                    );
                }
            });

        row.add_child(stack.finish());

        // The footer (if any) was already placed inside the styled container above.
        // Wrap the full row in an EventHandler before passing to Dismiss.
        let dismiss_child = EventHandler::new(row.finish()).finish();
        let mut dismiss = Dismiss::new(dismiss_child)
            .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(MenuAction::Close(false)));

        if prevent_interaction_with_other_elements {
            dismiss = dismiss.prevent_interaction_with_other_elements()
        }

        dismiss.finish()
    }
}

impl<A: Action + Clone> View for Menu<A> {
    fn ui_name() -> &'static str {
        "Menu"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let safe_zone_anchor_row = self
            .safe_triangle
            .as_ref()
            .filter(|st| st.is_suppressing())
            .and(self.menu.hovered_row_index);

        self.menu.render(
            self.border,
            self.submenu_width,
            self.with_drop_shadow,
            self.origin,
            self.window_id(app),
            self.prevent_interaction_with_other_elements,
            self.dispatch_item_actions,
            self.ignore_hover_when_covered,
            safe_zone_anchor_row,
            self.submenu_being_shown_for_item_index,
            self.flatten_bottom_corners,
            self.pinned_footer_builder.as_deref(),
            self.pinned_header_builder.as_deref(),
            self.content_top_padding_override,
            self.content_bottom_padding_override,
            app,
        )
    }
}

/// Testing utilities.
#[cfg(test)]
impl<A: Action + Clone> MenuItem<A> {
    pub fn fields(&self) -> Option<&MenuItemFields<A>> {
        // This method is used purely for the unit tests purposes.
        // It only returns the item fields for the single menu item.
        match self {
            MenuItem::Item(fields) => Some(fields),
            _ => None,
        }
    }

    pub fn is_separator(&self) -> bool {
        matches!(self, MenuItem::Separator)
    }

    /// Returns true iff a and b are "approximately" equal. By "approximately", we mean:
    /// - they are either both separators
    /// - or, they are both single items with the same label
    /// - or, they are both item rows with items whose labels match pairwise
    pub fn is_approximately_same_item_as(&self, other: &MenuItem<A>) -> bool {
        match (self, other) {
            (MenuItem::Separator, MenuItem::Separator) => true,
            (MenuItem::Item(self_fields), MenuItem::Item(other_fields)) => {
                self_fields.label() == other_fields.label()
            }
            (
                MenuItem::ItemsRow { items: self_items },
                MenuItem::ItemsRow { items: other_items },
            ) => {
                self_items.len() == other_items.len()
                    && self_items
                        .iter()
                        .zip(other_items)
                        .all(|(self_fields, other_fields)| {
                            self_fields.label() == other_fields.label()
                        })
            }
            _ => false,
        }
    }
}

#[cfg(test)]
#[path = "menu_tests.rs"]
mod tests;
