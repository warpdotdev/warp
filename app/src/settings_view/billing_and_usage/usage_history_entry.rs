use crate::{
    ai::blocklist::format_credits,
    ai::blocklist::usage::conversation_usage_view::{
        ConversationUsageInfo, ConversationUsageView, DisplayMode,
    },
    settings_view::billing_and_usage_page::BillingAndUsagePageAction,
    ui_components::{blended_colors, icons::Icon},
};
use chrono::Local;
use warp_core::ui::appearance::Appearance;
use warp_graphql::queries::get_conversation_usage::ConversationUsage;
use warpui::{
    elements::{
        Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty, Flex,
        Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius,
        Shrinkable, Text,
    },
    platform::Cursor,
    AppContext, Element, View,
};

pub struct UsageHistoryEntry {
    // If no entry is provided, we will assume that this is a placeholder entry
    // to display in the loading UI.
    entry: Option<ConversationUsage>,
    is_expanded: bool,
    mouse_state: Option<MouseStateHandle>,
    tooltip_mouse_state: MouseStateHandle,
}

impl UsageHistoryEntry {
    pub fn new(
        entry: Option<ConversationUsage>,
        is_expanded: bool,
        mouse_state: Option<MouseStateHandle>,
        tooltip_mouse_state: MouseStateHandle,
    ) -> Self {
        Self {
            entry,
            mouse_state,
            is_expanded,
            tooltip_mouse_state,
        }
    }

    pub fn render(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let mut res = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(self.render_header(appearance));

        if let Some(entry) = &self.entry {
            if self.is_expanded {
                res = res
                    .with_child(
                        // Separator between header and usage component
                        Container::new(Empty::new().finish())
                            .with_border(
                                Border::top(2.0).with_border_fill(appearance.theme().outline()),
                            )
                            .with_overdraw_bottom(0.)
                            .finish(),
                    )
                    .with_child(
                        ConversationUsageView::new(
                            ConversationUsageInfo::from(entry),
                            DisplayMode::Settings,
                            None,
                            self.tooltip_mouse_state.clone(),
                        )
                        .render(app),
                    );
            }
        }

        Container::new(res.finish())
            .with_border(Border::all(2.).with_border_fill(appearance.theme().surface_3()))
            .with_background(appearance.theme().surface_2())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .finish()
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let Some(entry) = &self.entry else {
            return self.render_loading_entry(appearance);
        };
        let Some(mouse_state) = &self.mouse_state else {
            // If there is a provided entry, there should always be a mouse state as well.
            log::error!("Mouse state is required to render usage history entry header");
            return Empty::new().finish();
        };

        let title_text = Text::new_inline(entry.title.clone(), appearance.ui_font_family(), 14.)
            .with_color(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().surface_2())
                    .into(),
            )
            .finish();

        let formatted_time = entry
            .last_updated
            .utc()
            .with_timezone(&Local)
            .format("%-m/%-d/%y %-I:%M %p")
            .to_string();
        let time_text = Text::new_inline(formatted_time, appearance.ui_font_family(), 12.)
            .with_color(blended_colors::text_sub(
                appearance.theme(),
                appearance.theme().surface_1(),
            ))
            .finish();

        let credits_spent = Text::new_inline(
            format_credits(entry.usage_metadata.credits_spent as f32),
            appearance.ui_font_family(),
            14.,
        )
        .with_color(blended_colors::text_sub(
            appearance.theme(),
            appearance.theme().surface_1(),
        ))
        .finish();

        let chevron_icon = if self.is_expanded {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };
        let chevron = ConstrainedBox::new(
            chevron_icon
                .to_warpui_icon(appearance.theme().foreground())
                .finish(),
        )
        .with_width(16.)
        .with_height(16.)
        .finish();

        let header_row = Container::new(
            Flex::row()
                .with_child(
                    Shrinkable::new(
                        1.,
                        Container::new(
                            Flex::column()
                                .with_child(title_text)
                                .with_child(Container::new(time_text).with_margin_top(4.).finish())
                                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                                .finish(),
                        )
                        .finish(),
                    )
                    .finish(),
                )
                .with_child(
                    Flex::row()
                        .with_child(credits_spent)
                        .with_child(chevron)
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .finish(),
                )
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_main_axis_size(MainAxisSize::Max)
                .finish(),
        )
        .with_uniform_padding(12.)
        .with_corner_radius(if self.is_expanded {
            CornerRadius::with_top(Radius::Pixels(6.))
        } else {
            CornerRadius::with_all(Radius::Pixels(6.))
        })
        .with_background(appearance.theme().surface_2())
        .finish();

        let conversation_id = entry.conversation_id.clone();
        Hoverable::new(mouse_state.clone(), |_| header_row)
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(BillingAndUsagePageAction::ToggleUsageEntryExpanded {
                    conversation_id: conversation_id.clone(),
                });
            })
            .finish()
    }

    /// Render a placeholder entry for the loading state
    fn render_loading_entry(&self, appearance: &Appearance) -> Box<dyn Element> {
        let left_side = Flex::column()
            .with_child(self.render_empty_text_placeholder(360., 16., appearance))
            .with_child(self.render_empty_text_placeholder(160., 12., appearance))
            .with_spacing(4.)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .finish();

        Container::new(
            Flex::row()
                .with_child(left_side)
                .with_child(self.render_empty_text_placeholder(52., 16., appearance))
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_main_axis_size(MainAxisSize::Max)
                .finish(),
        )
        .with_uniform_padding(12.)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
        .with_background(appearance.theme().surface_2())
        .finish()
    }

    /// Renders an empty rectangle to represent loading text
    fn render_empty_text_placeholder(
        &self,
        width: f32,
        height: f32,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(Empty::new().finish())
            .with_padding_left(width)
            .with_padding_top(height)
            .with_background(appearance.theme().surface_3())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .finish()
    }
}
