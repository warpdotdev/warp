use warp_core::ui::appearance::Appearance;
use warp_core::ui::icons::Icon as CoreIcon;
use warp_core::ui::theme::color::internal_colors::neutral_2;
use warpui::elements::{
    Clipped, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Expanded, Flex,
    Hoverable, Icon, MouseStateHandle, ParentElement, Radius, Text,
};
use warpui::platform::Cursor;
use warpui::{AppContext, Element, SingletonEntity};

use crate::ai::blocklist::block::view_impl::WithContentItemSpacing;
use crate::ai::blocklist::inline_action::inline_action_header::{
    self, INLINE_ACTION_HORIZONTAL_PADDING, INLINE_ACTION_VERTICAL_PADDING,
};
use crate::ai::blocklist::inline_action::inline_action_icons::icon_size;
use crate::ai::document::ai_document_model::{AIDocumentId, AIDocumentModel, AIDocumentVersion};

pub struct CreateOrEditDocumentAction {
    document_id: AIDocumentId,
    document_title: String,
    document_version: AIDocumentVersion,
    mouse_state: MouseStateHandle,
}

impl CreateOrEditDocumentAction {
    pub fn new(
        document_id: AIDocumentId,
        document_version: AIDocumentVersion,
        mouse_state: MouseStateHandle,
        app: &AppContext,
    ) -> Option<Self> {
        let document = AIDocumentModel::as_ref(app).get_document(&document_id, document_version)?;
        Some(Self {
            document_id,
            document_title: document.get_title(),
            document_version: document.get_version(),
            mouse_state,
        })
    }

    pub fn render(self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let subtext_color = theme.sub_text_color(theme.background()).into_solid();
        let main_text_color = theme.main_text_color(theme.background()).into_solid();

        let file_icon = Container::new(
            ConstrainedBox::new(Icon::new(CoreIcon::Compass.into(), main_text_color).finish())
                .with_width(icon_size(app))
                .with_height(icon_size(app))
                .finish(),
        )
        .with_margin_right(inline_action_header::ICON_MARGIN)
        .finish();

        let title_text = Clipped::new(
            Text::new_inline(
                self.document_title.to_string(),
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(main_text_color)
            .with_selectable(false)
            .finish(),
        )
        .finish();

        let version_text = Container::new(
            Text::new_inline(
                self.document_version.to_string(),
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(subtext_color)
            .with_selectable(false)
            .finish(),
        )
        .with_margin_right(inline_action_header::ICON_MARGIN)
        .finish();

        let view_document_icon =
            ConstrainedBox::new(Icon::new(CoreIcon::Share3.into(), main_text_color).finish())
                .with_width(icon_size(app))
                .with_height(icon_size(app))
                .finish();

        let view_document_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(0.)
            .with_child(version_text)
            .with_child(view_document_icon)
            .finish();

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(file_icon)
            .with_child(Expanded::new(1., title_text).finish())
            .with_child(view_document_row);

        let content = Container::new(row.finish())
            .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_vertical_padding(INLINE_ACTION_VERTICAL_PADDING)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_background_color(neutral_2(theme))
            .with_border(warpui::elements::Border::all(1.).with_border_fill(theme.surface_2()))
            .finish();

        Hoverable::new(self.mouse_state, move |_mouse_state| content)
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _app, _| {
                ctx.dispatch_typed_action(crate::WorkspaceAction::OpenAIDocumentPane {
                    document_id: self.document_id,
                    document_version: self.document_version,
                });
            })
            .finish()
            .with_agent_output_item_spacing(app)
            .finish()
    }
}
