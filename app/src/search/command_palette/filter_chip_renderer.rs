use crate::appearance::Appearance;
use crate::drive::cloud_object_styling::warp_drive_icon_color;
use crate::drive::DriveObjectType;
use crate::search::FilterChipRenderer as CommonFilterChipRenderer;
use crate::search::QueryFilter;
use crate::util::color::{ContrastingColor, MinimumAllowedContrast};
use pathfinder_color::ColorU;
use warpui::elements::{
    ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, Hoverable, Icon,
    MouseStateHandle, ParentElement, Radius, Text,
};
use warpui::platform::Cursor;
use warpui::{Element, EventContext};

/// Trait to render filter chips for the command palette.
pub trait FilterChipRenderer: crate::search::FilterChipRenderer {
    /// Renders the filter chip. When the filter chip is clicked, `on_click_fn` is called.
    fn render_filter_chip(
        &self,
        mouse_state_handle: MouseStateHandle,
        appearance: &Appearance,
        on_click_fn: fn(&mut EventContext, Self),
    ) -> Box<dyn Element>;

    /// Returns the color of the icon for the filter chip.
    fn icon_color(&self, appearance: &Appearance) -> ColorU;
}

impl FilterChipRenderer for QueryFilter {
    fn render_filter_chip(
        &self,
        mouse_state_handle: MouseStateHandle,
        appearance: &Appearance,
        on_click_fn: fn(&mut EventContext, Self),
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let self_copy: QueryFilter = *self;
        Hoverable::new(mouse_state_handle, |mouse_state| {
            let font_size = appearance.monospace_font_size() - 2.;
            Container::new({
                let flex_row = Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        Text::new_inline(
                            self.display_name(),
                            appearance.ui_font_family(),
                            font_size,
                        )
                        .with_color(
                            appearance
                                .theme()
                                .main_text_color(appearance.theme().surface_2())
                                .into_solid(),
                        )
                        .finish(),
                    );

                match self.icon_svg_path() {
                    None => flex_row.finish(),
                    Some(icon_name) => {
                        let icon_size = font_size + self.icon_size_offset();

                        let icon = Container::new(
                            ConstrainedBox::new(
                                Icon::new(
                                    icon_name,
                                    self.icon_color(appearance).on_background(
                                        appearance.theme().surface_2().into_solid(),
                                        MinimumAllowedContrast::NonText,
                                    ),
                                )
                                .finish(),
                            )
                            .with_width(icon_size)
                            .with_height(icon_size)
                            .finish(),
                        )
                        .with_margin_top(self.icon_margin_top());
                        flex_row
                            .with_child(icon.with_margin_left(8.).finish())
                            .finish()
                    }
                }
            })
            .with_vertical_padding(styles::vertical_padding(mouse_state))
            .with_horizontal_padding(styles::horizontal_padding(mouse_state))
            .with_background(styles::background_fill(mouse_state, theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
            .with_border(styles::border(mouse_state, theme))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |event_ctx, _, _| on_click_fn(event_ctx, self_copy))
        .finish()
    }

    fn icon_color(&self, appearance: &Appearance) -> ColorU {
        match self {
            QueryFilter::History
            | QueryFilter::NaturalLanguage
            | QueryFilter::Actions
            | QueryFilter::Sessions
            | QueryFilter::Tabs
            | QueryFilter::Drive
            | QueryFilter::LaunchConfigurations
            | QueryFilter::PromptHistory
            | QueryFilter::Files
            | QueryFilter::Commands
            | QueryFilter::Blocks
            | QueryFilter::Code
            | QueryFilter::Rules
            | QueryFilter::Repos
            | QueryFilter::DiffSets
            | QueryFilter::StaticSlashCommands
            | QueryFilter::Skills
            | QueryFilter::BaseModels
            | QueryFilter::FullTerminalUseModels
            | QueryFilter::CurrentDirectoryConversations => appearance
                .theme()
                .main_text_color(appearance.theme().surface_2())
                .into_solid(),
            QueryFilter::Conversations => appearance
                .theme()
                .main_text_color(appearance.theme().surface_2())
                .into_solid(),
            QueryFilter::Workflows => warp_drive_icon_color(appearance, DriveObjectType::Workflow),
            QueryFilter::Notebooks => warp_drive_icon_color(
                appearance,
                DriveObjectType::Notebook {
                    is_ai_document: false,
                },
            ),
            QueryFilter::Plans => warp_drive_icon_color(
                appearance,
                DriveObjectType::Notebook {
                    is_ai_document: true,
                },
            ),
            QueryFilter::EnvironmentVariables => {
                warp_drive_icon_color(appearance, DriveObjectType::EnvVarCollection)
            }
            QueryFilter::AgentModeWorkflows => {
                warp_drive_icon_color(appearance, DriveObjectType::AgentModeWorkflow)
            }
        }
    }
}

mod styles {
    use crate::themes::theme::{Blend, Fill, WarpTheme};
    use warpui::elements::{Border, MouseState};

    /// Size of the border when the query filter is hovered.
    const HOVERED_BORDER_SIZE: f32 = 2.;
    /// Size of the border when the query filter is _not_ hovered.
    const BORDER_SIZE: f32 = 1.;

    /// Vertical padding when the query filter is _not_ hovered.
    const VERTICAL_PADDING: f32 = 8.;

    /// Horizontal padding when the query filter is _not_ hovered.
    const HORIZONTAL_PADDING: f32 = 16.;

    /// Returns the amount of vertical padding that should be applied to the query filter while also
    /// ensuring the query filter doesn't "jump" when it is hovered.
    pub fn vertical_padding(mouse_state: &MouseState) -> f32 {
        if mouse_state.is_hovered() {
            VERTICAL_PADDING - (HOVERED_BORDER_SIZE - BORDER_SIZE)
        } else {
            VERTICAL_PADDING
        }
    }

    /// Returns the amount of horizontal padding that should be applied to the query filter while also
    /// ensuring the query filter doesn't "jump" when it is hovered.
    pub fn horizontal_padding(mouse_state: &MouseState) -> f32 {
        if mouse_state.is_hovered() {
            HORIZONTAL_PADDING - (HOVERED_BORDER_SIZE - BORDER_SIZE)
        } else {
            HORIZONTAL_PADDING
        }
    }

    /// Returns the border that should be applied to the query filter.
    pub fn border(mouse_state: &MouseState, theme: &WarpTheme) -> Border {
        if mouse_state.is_hovered() {
            Border::all(HOVERED_BORDER_SIZE).with_border_fill(theme.accent())
        } else {
            Border::all(BORDER_SIZE).with_border_fill(theme.sub_text_color(theme.surface_2()))
        }
    }

    /// Returns the background [`Fill`] that should be applied to the query filter.
    pub fn background_fill(mouse_state: &MouseState, theme: &WarpTheme) -> Fill {
        if mouse_state.is_hovered() {
            theme
                .surface_2()
                .blend(&theme.dark_overlay().with_opacity(25))
        } else {
            theme.surface_2()
        }
    }
}
