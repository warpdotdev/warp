//! A reusable component for displaying text with a copy button that shows
//! checkmark feedback when clicked.

use std::time::Duration;

use instant::Instant;
use warpui::color::ColorU;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Element, Expanded, Flex, MouseStateHandle,
    ParentElement, Shrinkable, Text,
};
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::UiComponent;
use warpui::{AppContext, SingletonEntity};

use crate::appearance::Appearance;
use crate::ui_components::icons::Icon;

/// Duration to show the checkmark after copying.
pub const COPY_FEEDBACK_DURATION: Duration = Duration::from_secs(2);

/// Configuration for the copyable text field.
pub struct CopyableTextFieldConfig<'a> {
    /// The text to display.
    pub text: String,
    /// Font size for the text.
    pub font_size: f32,
    /// Text color (optional - defaults to theme's active_ui_text_color if not set).
    pub text_color: Option<ColorU>,
    /// Size of the copy button icon.
    pub icon_size: f32,
    /// Mouse state handle for the copy button.
    pub copy_button_mouse_state: MouseStateHandle,
    /// When the text was last copied (for showing checkmark feedback).
    pub last_copied_at: Option<&'a Instant>,
    /// Whether the text should be selectable.
    pub is_selectable: bool,
    /// Whether the text should soft-wrap instead of being ellipsized.
    pub wrap_text: bool,
    /// Placement of the copy button relative to the text.
    pub copy_button_placement: CopyButtonPlacement,
    /// Cross-axis alignment of the row (text + copy button).
    pub cross_axis_alignment: Option<CrossAxisAlignment>,
}

#[derive(Clone, Copy)]
pub enum CopyButtonPlacement {
    NextToText,
    EndOfContainer,
}
impl<'a> CopyableTextFieldConfig<'a> {
    /// Creates a new config with the given text.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            font_size: 14.0,
            text_color: None,
            icon_size: 12.0,
            copy_button_mouse_state: MouseStateHandle::default(),
            last_copied_at: None,
            is_selectable: true,
            wrap_text: false,
            copy_button_placement: CopyButtonPlacement::EndOfContainer,
            cross_axis_alignment: None,
        }
    }

    /// Sets the font size.
    pub fn with_font_size(mut self, font_size: f32) -> Self {
        self.font_size = font_size;
        self
    }

    /// Sets the text color.
    pub fn with_text_color(mut self, color: ColorU) -> Self {
        self.text_color = Some(color);
        self
    }

    /// Sets the icon size.
    pub fn with_icon_size(mut self, icon_size: f32) -> Self {
        self.icon_size = icon_size;
        self
    }

    /// Sets whether the text should soft-wrap instead of being ellipsized.
    pub fn with_wrap_text(mut self, wrap_text: bool) -> Self {
        self.wrap_text = wrap_text;
        self
    }

    /// Sets the mouse state handle for the copy button.
    pub fn with_mouse_state(mut self, mouse_state: MouseStateHandle) -> Self {
        self.copy_button_mouse_state = mouse_state;
        self
    }

    /// Sets when the text was last copied (for checkmark feedback).
    pub fn with_last_copied_at(mut self, last_copied_at: Option<&'a Instant>) -> Self {
        self.last_copied_at = last_copied_at;
        self
    }
    /// Sets the placement of the copy button relative to the text.
    pub fn with_copy_button_placement(mut self, placement: CopyButtonPlacement) -> Self {
        self.copy_button_placement = placement;
        self
    }

    /// Sets the cross-axis alignment of the row.
    pub fn with_cross_axis_alignment(mut self, alignment: CrossAxisAlignment) -> Self {
        self.cross_axis_alignment = Some(alignment);
        self
    }

    /// Returns true if the checkmark feedback should be shown.
    pub fn should_show_checkmark(&self) -> bool {
        self.last_copied_at
            .is_some_and(|time| time.elapsed() < COPY_FEEDBACK_DURATION)
    }
}

/// Renders a text field with a copy button that shows checkmark feedback.
///
/// The copy action must be handled by the caller via the `on_copy` callback.
/// The caller is also responsible for tracking `last_copied_at` and scheduling
/// a re-render after `COPY_FEEDBACK_DURATION` to clear the checkmark.
///
/// # Example
/// ```ignore
/// let element = render_copyable_text_field(
///     CopyableTextFieldConfig::new("some text to copy")
///         .with_font_size(14.0)
///         .with_text_color(theme.active_ui_text_color())
///         .with_mouse_state(mouse_state.clone())
///         .with_last_copied_at(last_copied_times.get(&id)),
///     |ctx| {
///         ctx.clipboard().write(ClipboardContent::plain_text("some text to copy"));
///         // Track the copy time and schedule re-render
///     },
///     app,
/// );
/// ```
pub fn render_copyable_text_field<F>(
    config: CopyableTextFieldConfig,
    on_copy: F,
    app: &AppContext,
) -> Box<dyn Element>
where
    F: FnMut(&mut warpui::EventContext) + 'static,
{
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let show_checkmark = config.should_show_checkmark();
    let text_color = config
        .text_color
        .unwrap_or_else(|| theme.active_ui_text_color().into());

    let text_element = if config.wrap_text {
        Text::new(config.text, appearance.ui_font_family(), config.font_size)
            .with_color(text_color)
            .with_selectable(config.is_selectable)
            .finish()
    } else {
        Text::new_inline(config.text, appearance.ui_font_family(), config.font_size)
            .with_color(text_color)
            .with_selectable(config.is_selectable)
            .with_clip(ClipConfig::ellipsis())
            .finish()
    };

    let copy_button: Box<dyn Element> = if show_checkmark {
        // Show green checkmark
        let check_icon = warpui::elements::Icon::new(Icon::Check.into(), theme.ansi_fg_green());
        ConstrainedBox::new(check_icon.finish())
            .with_width(config.icon_size)
            .with_height(config.icon_size)
            .finish()
    } else {
        // Show copy button
        let mut on_copy = on_copy;
        appearance
            .ui_builder()
            .copy_button(config.icon_size, config.copy_button_mouse_state)
            .build()
            .on_click(move |ctx, _, _| {
                on_copy(ctx);
            })
            .finish()
    };

    let cross_axis_alignment = config
        .cross_axis_alignment
        .unwrap_or(CrossAxisAlignment::Center);

    let mut row = Flex::row().with_cross_axis_alignment(cross_axis_alignment);
    match config.copy_button_placement {
        CopyButtonPlacement::NextToText => {
            row.add_child(Shrinkable::new(1., text_element).finish());
            row.add_child(Container::new(copy_button).with_padding_left(4.).finish());
        }
        CopyButtonPlacement::EndOfContainer => {
            row.add_child(Expanded::new(1., text_element).finish());
            row.add_child(Container::new(copy_button).with_padding_left(4.).finish());
        }
    }

    row.finish()
}
