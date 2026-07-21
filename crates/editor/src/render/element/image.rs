use warp_core::ui::appearance::Appearance;
use warpui_core::elements::{
    CacheOption, ConstrainedBox, Container, CrossAxisAlignment, Flex, Icon, Image,
    MouseStateHandle, ParentElement, Text,
};
use warpui_core::geometry::vector::{Vector2F, vec2f};
use warpui_core::{Element, SingletonEntity, SizeConstraint};

use super::{RenderContext, RenderableBlock};
use crate::extract_block;
use crate::render::element::paint::{CursorData, CursorDisplayType};
use crate::render::model::viewport::ViewportItem;
use crate::render::model::{BlockItem, RenderState, RichTextStyles};

/// Below-right nudge of the alt-text tooltip from the mouse pointer, so the
/// cursor sits just above-left of the tooltip's top-left corner rather than
/// covering its text. Mirrors the conventional pointer-anchored tooltip offset.
fn tooltip_pointer_offset() -> Vector2F {
    vec2f(12., 16.)
}

/// Whether an image should carry an alt-text hover tooltip. Only images with
/// non-empty (non-whitespace) alt text do: an image without alt text is
/// decorative, and a tooltip with nothing to say shouldn't exist. This mirrors
/// the accessibility-tree rule (`alt=""` → decorative) so the two affordances
/// agree on which images are meaningful.
fn image_has_tooltip(alt_text: &str) -> bool {
    !alt_text.trim().is_empty()
}

/// Text shown alongside the broken-image glyph when an image fails to load.
/// Prefers the image's alt text; falls back to a generic notice when the alt
/// text is empty or whitespace-only.
fn broken_image_label(alt_text: &str) -> String {
    let trimmed = alt_text.trim();
    if trimmed.is_empty() {
        "Image failed to load".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Builds the placeholder element painted in place of an image that failed to
/// load: a failed-image glyph followed by the image's alt text (or a generic
/// notice when no alt text is present).
fn broken_image_placeholder(alt_text: &str, styles: &RichTextStyles) -> Box<dyn Element> {
    let icon = ConstrainedBox::new(
        Icon::new(
            styles.broken_image_style.icon_path,
            styles.broken_image_style.icon_color,
        )
        .with_opacity(1.0)
        .finish(),
    )
    .with_height(styles.base_text.font_size + 2.)
    .with_width(styles.base_text.font_size + 2.)
    .finish();

    let text = Container::new(
        Text::new(
            broken_image_label(alt_text),
            styles.base_text.font_family,
            styles.base_text.font_size,
        )
        .with_color(styles.placeholder_color)
        .finish(),
    )
    .with_padding_left(8.)
    .finish();

    Flex::row()
        .with_child(icon)
        .with_child(text)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .finish()
}

pub struct RenderableImage {
    viewport_item: ViewportItem,
    /// The image's alt text, shown as a hover tooltip. Empty when the source
    /// markdown gave no alt text, in which case no tooltip is attached.
    alt_text: String,
    /// Persists hover state across re-layouts so tooltip hover tracking is stable.
    mouse_state: MouseStateHandle,
    // TODO: The AssetCache does not currently support automatic eviction of assets when they are
    // dropped. We should consider implementing a mechanism to unload images when they are no longer
    // visible or referenced.
    image_element: Option<Box<dyn Element>>,
}

impl RenderableImage {
    pub fn new(
        viewport_item: ViewportItem,
        alt_text: String,
        mouse_state: MouseStateHandle,
    ) -> Self {
        Self {
            viewport_item,
            alt_text,
            mouse_state,
            image_element: None,
        }
    }
}

impl RenderableBlock for RenderableImage {
    fn viewport_item(&self) -> &ViewportItem {
        &self.viewport_item
    }

    fn layout(
        &mut self,
        model: &RenderState,
        ctx: &mut warpui_core::LayoutContext,
        app: &warpui_core::AppContext,
    ) {
        let content = model.content();
        let (asset_source, config) = extract_block!(
            self.viewport_item,
            content,
            (_block, BlockItem::Image { asset_source, config, .. }) => (asset_source.clone(), *config)
        );

        let placeholder = broken_image_placeholder(&self.alt_text, model.styles());

        let size = vec2f(config.width.as_f32(), config.height.as_f32());
        let image = Image::new(asset_source, CacheOption::BySize)
            .contain()
            .first_frame_preview()
            .on_load_failure(placeholder)
            .finish();

        // Show the alt text on hover so it's reachable without loading the image
        // (and remains available for the broken-image placeholder). Use the
        // overlay variant so the tooltip escapes the editor's viewport clip, and
        // anchor it at the mouse pointer with a browser-`title` fade animation
        // (fade in on rest, fade out on motion or exit, relocate on re-rest at a
        // new spot) so it reads as a cursor tooltip rather than a caption pinned
        // to the image.
        let mut element: Box<dyn Element> = if image_has_tooltip(&self.alt_text) {
            Appearance::as_ref(app)
                .ui_builder()
                .overlay_tool_tip_at_pointer(
                    self.alt_text.clone(),
                    self.mouse_state.clone(),
                    image,
                    tooltip_pointer_offset(),
                )
        } else {
            image
        };

        let constraint = SizeConstraint::new(vec2f(0., 0.), size);
        element.layout(constraint, ctx, app);

        self.image_element = Some(element);
    }

    fn paint(
        &mut self,
        model: &RenderState,
        ctx: &mut RenderContext,
        app: &warpui_core::AppContext,
    ) {
        let content = model.content();
        let positioned_image = extract_block!(
            self.viewport_item,
            content,
            (block, BlockItem::Image { config, .. }) => block.image(config)
        );

        let selected = model.offset_in_active_selection(positioned_image.start_char_offset);
        let draw_cursor = model.is_selection_head(positioned_image.start_char_offset);

        let content_position = positioned_image.content_origin();
        let screen_position = ctx.content_to_screen(content_position);
        let size = vec2f(
            positioned_image.item.width.as_f32(),
            positioned_image.item.height.as_f32(),
        );

        if let Some(ref mut image_element) = self.image_element {
            image_element.paint(screen_position, ctx.paint, app);
        }

        if selected {
            let rect_bounds = warpui_core::geometry::rect::RectF::new(screen_position, size);
            ctx.paint
                .scene
                .draw_rect_with_hit_recording(rect_bounds)
                .with_background(model.styles().selection_fill);
        }

        if draw_cursor {
            let end_of_line_position = content_position + vec2f(size.x(), 0.);
            ctx.draw_and_save_cursor(
                CursorDisplayType::Bar,
                end_of_line_position,
                vec2f(model.styles().cursor_width, size.y()),
                CursorData::default(),
                model.styles(),
            );
        }
    }

    fn after_layout(
        &mut self,
        ctx: &mut warpui_core::AfterLayoutContext,
        app: &warpui_core::AppContext,
    ) {
        if let Some(ref mut image_element) = self.image_element {
            image_element.after_layout(ctx, app);
        }
    }

    fn dispatch_event(
        &mut self,
        _model: &RenderState,
        event: &warpui_core::event::DispatchedEvent,
        ctx: &mut warpui_core::EventContext,
        app: &warpui_core::AppContext,
    ) -> bool {
        if let Some(ref mut image_element) = self.image_element {
            image_element.dispatch_event(event, ctx, app)
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{broken_image_label, image_has_tooltip};

    #[test]
    fn tooltip_present_when_alt_text_non_empty() {
        assert!(image_has_tooltip("A red bicycle"));
        assert!(image_has_tooltip("  spaced alt  "));
    }

    #[test]
    fn tooltip_absent_when_alt_text_empty_or_whitespace() {
        assert!(!image_has_tooltip(""));
        assert!(!image_has_tooltip("   "));
        assert!(!image_has_tooltip("\t\n"));
    }

    #[test]
    fn label_uses_alt_text_when_present() {
        assert_eq!(broken_image_label("A red bicycle"), "A red bicycle");
    }

    #[test]
    fn label_trims_surrounding_whitespace() {
        assert_eq!(broken_image_label("  spaced alt  "), "spaced alt");
    }

    #[test]
    fn label_falls_back_when_alt_text_empty() {
        assert_eq!(broken_image_label(""), "Image failed to load");
    }

    #[test]
    fn label_falls_back_when_alt_text_only_whitespace() {
        assert_eq!(broken_image_label("   \t"), "Image failed to load");
    }
}
