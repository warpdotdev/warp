use crate::{
    color::ColorU,
    elements::{Element, Fill},
    fonts::{FamilyId, Properties, Weight},
    scene::CornerRadius,
};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum BorderStyle {
    #[default]
    None,
    Solid,
    Double,
    Dotted,
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Coords {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

impl Coords {
    pub fn uniform(val: f32) -> Self {
        Coords {
            top: val,
            bottom: val,
            left: val,
            right: val,
        }
    }

    pub fn top(mut self, top: f32) -> Self {
        self.top = top;
        self
    }

    pub fn bottom(mut self, bottom: f32) -> Self {
        self.bottom = bottom;
        self
    }

    pub fn left(mut self, left: f32) -> Self {
        self.left = left;
        self
    }

    pub fn right(mut self, right: f32) -> Self {
        self.right = right;
        self
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct UiComponentStyles {
    pub width: Option<f32>, // TODO should be possible to spec units/equations (eg. 100% - 5px)
    pub height: Option<f32>,
    pub position: Option<Coords>,
    pub background: Option<Fill>,
    pub foreground: Option<Fill>,
    pub border_color: Option<Fill>,
    pub border_width: Option<f32>,
    pub border_style: Option<BorderStyle>,
    pub border_radius: Option<CornerRadius>,
    pub font_family_id: Option<FamilyId>,
    pub font_size: Option<f32>,
    pub font_color: Option<ColorU>,
    pub font_weight: Option<Weight>,
    // TODO add text_decorations (underline, etc.)
    pub padding: Option<Coords>,
    pub margin: Option<Coords>,
}

impl UiComponentStyles {
    /// `merge` combines 2 styles together. Self (usually a default) is overwritten by the styles
    /// from `style` element (in other words: `style` values has higher precedence than `self`).
    pub fn merge(&self, style: UiComponentStyles) -> Self {
        UiComponentStyles {
            width: style.width.or(self.width),
            height: style.height.or(self.height),
            position: style.position.or(self.position),
            background: style.background.or(self.background),
            foreground: style.foreground.or(self.foreground),
            border_color: style.border_color.or(self.border_color),
            border_width: style.border_width.or(self.border_width),
            border_style: style.border_style.or(self.border_style),
            border_radius: style.border_radius.or(self.border_radius),
            font_family_id: style.font_family_id.or(self.font_family_id),
            font_size: style.font_size.or(self.font_size),
            font_color: style.font_color.or(self.font_color),
            font_weight: style.font_weight.or(self.font_weight),
            padding: style.padding.or(self.padding),
            margin: style.margin.or(self.margin),
        }
    }
    pub fn set_width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
    pub fn set_height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }
    pub fn set_position(mut self, position: Coords) -> Self {
        self.position = Some(position);
        self
    }
    pub fn set_background(mut self, background: Fill) -> Self {
        self.background = Some(background);
        self
    }
    pub fn set_border_color(mut self, border_color: Fill) -> Self {
        self.border_color = Some(border_color);
        self
    }
    pub fn set_border_width(mut self, border_width: f32) -> Self {
        self.border_width = Some(border_width);
        self
    }
    pub fn set_border_style(mut self, border_style: BorderStyle) -> Self {
        self.border_style = Some(border_style);
        self
    }
    pub fn set_border_radius(mut self, border_radius: CornerRadius) -> Self {
        self.border_radius = Some(border_radius);
        self
    }
    pub fn set_font_family_id(mut self, font_family_id: FamilyId) -> Self {
        self.font_family_id = Some(font_family_id);
        self
    }
    pub fn set_font_size(mut self, font_size: f32) -> Self {
        self.font_size = Some(font_size);
        self
    }
    pub fn set_font_color(mut self, font_color: ColorU) -> Self {
        self.font_color = Some(font_color);
        self
    }
    pub fn set_font_weight(mut self, font_weight: Weight) -> Self {
        self.font_weight = Some(font_weight);
        self
    }
    pub fn set_padding(mut self, padding: Coords) -> Self {
        self.padding = Some(padding);
        self
    }
    pub fn set_margin(mut self, margin: Coords) -> Self {
        self.margin = Some(margin);
        self
    }

    pub fn font_properties(&self) -> Properties {
        self.font_weight.map_or_else(Properties::default, |weight| {
            Properties::default().weight(weight)
        })
    }
}

pub trait UiComponent {
    type ElementType: Element;

    fn build(self) -> Self::ElementType;
    fn with_style(self, style: UiComponentStyles) -> Self;
}

#[cfg(test)]
#[path = "components_tests.rs"]
mod tests;
