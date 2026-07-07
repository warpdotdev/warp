//! Module containing helper functions to render the windows traffic lights.

use std::sync::Arc;

use pathfinder_color::ColorU;
use warp_core::ui::theme::{Fill, WarpTheme};
use warpui::elements::{
    Align, ConstrainedBox, Container, CrossAxisAlignment, Flex, Hoverable, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, ParentElement, Text,
};
use warpui::fonts::FamilyId;
use warpui::platform::FullscreenState;
use warpui::{AppContext, Element, SingletonEntity};

use crate::util::traffic_lights::windows::RendererState;
use crate::util::traffic_lights::windows_only::WINDOWS_BRIGHT_RED;
use crate::util::traffic_lights::{TrafficLightData, TrafficLightMouseStates};
use crate::workspace::TOTAL_TAB_BAR_HEIGHT;

/// Possible window traffic light icons.
#[derive(Copy, Clone)]
pub(super) enum WindowsTrafficLightIcon {
    Close,
    Minimize,
    Maximize,
    Restore,
}

/// The golden ratio. Windows uses this ratio as the line height--using it ensures that each symbol
/// icon is perfectly centered within its bounding box.
const GOLDEN_RATIO: f32 = 1.618_034;

/// The width of each icon. Though not well documented, this matches the exact width of the window
/// controls when rendered natively by the OS.
const ICON_WIDTH: f32 = 46.;

/// The font size each icon should be rendered at when using a symbol font.
const ICON_FONT_SIZE: f32 = 10.;

impl WindowsTrafficLightIcon {
    /// Returns the unicode point of each traffic light icon when using a native windows symbol font.
    /// See https://learn.microsoft.com/en-us/windows/apps/design/style/segoe-fluent-icons-font#pua-e700-e900
    /// for reference.
    fn unicode_code_point(&self) -> &'static str {
        match self {
            Self::Minimize => "\u{e921}",
            Self::Restore => "\u{e923}",
            Self::Maximize => "\u{e922}",
            Self::Close => "\u{e8bb}",
        }
    }

    fn background_hover_color(&self, theme: &WarpTheme) -> Fill {
        match self {
            Self::Close => WINDOWS_BRIGHT_RED.into(),
            Self::Minimize | Self::Maximize | Self::Restore => theme.surface_3(),
        }
    }

    fn icon_hover_color(&self, theme: &WarpTheme) -> ColorU {
        match self {
            Self::Close => ColorU::white(),
            Self::Minimize | Self::Maximize | Self::Restore => self.icon_color(theme),
        }
    }

    fn icon_color(&self, theme: &WarpTheme) -> ColorU {
        theme.foreground().into_solid()
    }

    fn render(
        &self,
        mouse_state_handle: MouseStateHandle,
        theme: &WarpTheme,
        icon_font_family: FamilyId,
        action_name: &'static str,
    ) -> Box<dyn Element> {
        let hoverable = Hoverable::new(mouse_state_handle, |state| {
            let icon_color = if state.is_hovered() {
                self.icon_hover_color(theme)
            } else {
                self.icon_color(theme)
            };

            let icon = Text::new(self.unicode_code_point(), icon_font_family, ICON_FONT_SIZE)
                .with_color(icon_color)
                .with_line_height_ratio(GOLDEN_RATIO)
                .finish();
            let icon = Align::new(icon).finish();
            if state.is_hovered() {
                Container::new(icon)
                    .with_background(self.background_hover_color(theme))
                    .finish()
            } else {
                icon
            }
        })
        .on_click(move |evt, _, _| {
            evt.dispatch_action(action_name, ());
        })
        .finish();

        ConstrainedBox::new(hoverable)
            .with_width(ICON_WIDTH)
            .finish()
    }
}

fn render_tab_row_with_glyph_icons(
    fullscreen_state: FullscreenState,
    mouse_states: &TrafficLightMouseStates,
    theme: &WarpTheme,
    icon_font_family: FamilyId,
) -> Box<dyn Element> {
    let flex = Flex::row()
        .with_main_axis_size(MainAxisSize::Min)
        .with_main_axis_alignment(MainAxisAlignment::End)
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_children([
            WindowsTrafficLightIcon::Minimize.render(
                mouse_states.minimize_window_button.clone(),
                theme,
                icon_font_family,
                "root_view:minimize_window",
            ),
            {
                let icon = if fullscreen_state == FullscreenState::Normal {
                    WindowsTrafficLightIcon::Maximize
                } else {
                    WindowsTrafficLightIcon::Restore
                };
                let action_name = if fullscreen_state == FullscreenState::Fullscreen {
                    "root_view:toggle_fullscreen"
                } else {
                    "root_view:toggle_maximize_window"
                };
                icon.render(
                    mouse_states.maximize_window_button.clone(),
                    theme,
                    icon_font_family,
                    action_name,
                )
            },
            WindowsTrafficLightIcon::Close.render(
                mouse_states.close_window_button.clone(),
                theme,
                icon_font_family,
                "root_view:close_window",
            ),
        ])
        .finish();

    ConstrainedBox::new(flex)
        .with_height(TOTAL_TAB_BAR_HEIGHT)
        .finish()
}

impl TrafficLightData {
    pub fn render_tab_row(
        &self,
        fullscreen_state: FullscreenState,
        mouse_states: &TrafficLightMouseStates,
        theme: &WarpTheme,
        app: &AppContext,
    ) -> Box<dyn Element> {
        match RendererState::handle(app).as_ref(app).icon_font_family() {
            Some(icon_font_family) => render_tab_row_with_glyph_icons(
                fullscreen_state,
                mouse_states,
                theme,
                icon_font_family,
            ),
            None => {
                // If we were unable to fetch the icon font family, render the tab bar using SVG
                // icons instead.
                log::warn!(
                    "Unable to fetch a windows font to render the tab bar, using svgs instead."
                );
                self.render_tab_row_with_svg_icons(fullscreen_state, mouse_states, theme)
            }
        }
    }

    /// Renders the windows traffic lights with SVG icons. This is a fallback approach if the system
    /// does not contain the symbol fonts needed to render the traffic lights.
    fn render_tab_row_with_svg_icons(
        &self,
        fullscreen_state: FullscreenState,
        mouse_states: &TrafficLightMouseStates,
        theme: &WarpTheme,
    ) -> Box<dyn Element> {
        let fg_color = theme.foreground().into_solid();
        ConstrainedBox::new(
            Align::new(
                Flex::row()
                    .with_children([
                        Container::new(
                            Self::render_button(
                                Arc::clone(&mouse_states.minimize_window_button),
                                Self::render_windows_minimize_button_icon(fg_color),
                                theme.surface_3().into(),
                            )
                            .on_click(|evt, _, _| {
                                evt.dispatch_action("root_view:minimize_window", ());
                            })
                            .finish(),
                        )
                        .finish(),
                        Self::render_button(
                            Arc::clone(&mouse_states.maximize_window_button),
                            Self::render_windows_maximize_button_icon(fg_color, fullscreen_state),
                            theme.surface_3().into(),
                        )
                        .on_click(move |evt, _, _| {
                            if fullscreen_state == FullscreenState::Fullscreen {
                                evt.dispatch_action("root_view:toggle_fullscreen", ());
                            } else {
                                evt.dispatch_action("root_view:toggle_maximize_window", ());
                            }
                        })
                        .finish(),
                        Container::new(
                            Self::render_windows_close_button(
                                fg_color,
                                mouse_states.close_window_button.clone(),
                            )
                            .on_click(|evt, _, _| {
                                evt.dispatch_action("root_view:close_window", ());
                            })
                            .finish(),
                        )
                        .finish(),
                    ])
                    .finish(),
            )
            .finish(),
        )
        .with_max_height(TOTAL_TAB_BAR_HEIGHT)
        .with_width(self.width)
        .finish()
    }
}
