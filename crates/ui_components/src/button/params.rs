use std::borrow::Cow;

use warp_core::ui::Icon;
use warp_core::ui::appearance::Appearance;
use warpui_core::fonts;
use warpui_core::keymap::Keystroke;
use warpui_core::prelude::stack;

use super::Theme;
use crate::{keyboard_shortcut, tooltip};

/// The parameters for rendering a button.
pub struct Params<'a> {
    /// The content of the button.
    pub content: Content,
    /// The theme to use for the button.
    pub theme: &'a dyn Theme,
    /// The options for the button.
    pub options: Options,
}

impl<'a> crate::Params for Params<'a> {
    type Options<'b> = Options;
}

pub struct Options {
    /// Whether or not the button is disabled.
    ///
    /// Disabled buttons are rendered with a different theme and do not respond
    /// to mouse events.
    pub disabled: bool,
    /// The size of the button.
    pub size: Size,
    /// The tooltip to show on hover, if any.
    pub tooltip: Option<Tooltip>,
    /// The keystroke to display, if any.
    pub keystroke: Option<Keystroke>,
    /// The callback to invoke when the button is clicked, if any.
    pub on_click: Option<crate::MouseEventHandler>,
}

impl crate::Options for Options {
    fn default(appearance: &Appearance) -> Self {
        Self {
            disabled: false,
            size: Size::default(appearance),
            tooltip: None,
            keystroke: None,
            on_click: None,
        }
    }
}

/// The content of the button.
pub enum Content {
    /// A label-only button.
    Label(Cow<'static, str>),
    /// An icon-only button.
    Icon(Icon),
    /// A button with both an icon and a label.
    IconAndLabel(Icon, Cow<'static, str>),
}

/// The size of the button.
pub enum Size {
    Default,
    Small,
    Custom(Sizing),
}

impl crate::Options for Size {
    fn default(_: &Appearance) -> Self {
        Self::Default
    }
}

impl Size {
    pub(super) fn height(&self) -> f32 {
        self.sizing().height
    }

    pub(super) fn icon_size(&self) -> f32 {
        self.sizing().icon_size
    }

    pub(super) fn font_size(&self) -> f32 {
        self.sizing().font_size
    }

    pub(super) fn font_properties(&self) -> fonts::Properties {
        self.sizing().font_properties
    }

    pub(super) fn horizontal_padding(&self) -> f32 {
        self.sizing().horizontal_padding
    }

    pub(super) fn inner_spacing(&self) -> f32 {
        self.sizing().inner_spacing
    }

    pub(super) fn keyboard_shortcut_sizing(&self) -> keyboard_shortcut::Sizing {
        self.sizing().keyboard_shortcut_sizing
    }

    fn sizing(&self) -> &Sizing {
        match self {
            Size::Default => &DEFAULT_SIZE,
            Size::Small => &SMALL_SIZE,
            Size::Custom(custom) => custom,
        }
    }
}

/// The set of properties that vary with button size.
pub struct Sizing {
    pub height: f32,
    pub font_size: f32,
    pub icon_size: f32,
    pub font_properties: fonts::Properties,
    pub horizontal_padding: f32,
    pub inner_spacing: f32,
    pub keyboard_shortcut_sizing: keyboard_shortcut::Sizing,
}

/// Sizing for a default-sized button.
const DEFAULT_SIZE: Sizing = Sizing {
    height: 32.,
    font_size: 14.,
    icon_size: 16.,
    font_properties: fonts::Properties {
        weight: fonts::Weight::Semibold,
        style: fonts::Style::Normal,
    },
    horizontal_padding: 12.,
    inner_spacing: 4.,
    keyboard_shortcut_sizing: keyboard_shortcut::Sizing {
        font_size: 12.,
        padding: 2.,
    },
};

/// Sizing for a small-sized button.
const SMALL_SIZE: Sizing = Sizing {
    height: 24.,
    font_size: 12.,
    icon_size: 14.,
    font_properties: fonts::Properties {
        weight: fonts::Weight::Semibold,
        style: fonts::Style::Normal,
    },
    horizontal_padding: 8.,
    inner_spacing: 2.,
    keyboard_shortcut_sizing: keyboard_shortcut::Sizing {
        font_size: 10.,
        padding: 2.,
    },
};

/// The tooltip to show on hover, if any.
pub struct Tooltip {
    pub params: tooltip::Params,
    pub alignment: TooltipAlignment,
}

/// Alignment options for button tooltips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TooltipAlignment {
    Left,
    Center,
    #[default]
    Right,
}

impl TooltipAlignment {
    pub fn parent_anchor(&self) -> stack::ParentAnchor {
        match self {
            Self::Left => stack::ParentAnchor::TopLeft,
            Self::Center => stack::ParentAnchor::TopMiddle,
            Self::Right => stack::ParentAnchor::TopRight,
        }
    }

    pub fn child_anchor(&self) -> stack::ChildAnchor {
        match self {
            Self::Left => stack::ChildAnchor::BottomLeft,
            Self::Center => stack::ChildAnchor::BottomMiddle,
            Self::Right => stack::ChildAnchor::BottomRight,
        }
    }
}
