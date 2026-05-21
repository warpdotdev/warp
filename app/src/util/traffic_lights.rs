//! This module is meant to be a single source of truth for information about the windows' "traffic
//! light" buttons, the minimize, maximize, and close buttons in the corner of the window, so named
//! b/c of their resemblance to traffic lights on MacOS. How (whether or not) these are rendered
//! depends on the platform. The Warp app must use this information to avoid rendering UI elements
//! underneath them.

#[cfg(windows)]
pub mod windows;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod linux_only {
    pub(super) use crate::workspace::TOTAL_TAB_BAR_HEIGHT;
    pub(super) use pathfinder_color::ColorU;
    pub(super) use pathfinder_geometry::vector::vec2f;
    pub(super) use std::sync::Arc;
    pub(super) use warpui::elements::{
        Align, Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, Flex, Hoverable, Icon,
        OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Rect, Stack,
    };
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use linux_only::*;

#[cfg(target_os = "windows")]
mod windows_only {
    pub(super) use crate::ui_components::icons::Icon as IconComponent;
    pub(super) use pathfinder_color::ColorU;
    pub(super) use pathfinder_geometry::vector::vec2f;
    pub(super) use warp_core::ui::theme;
    pub(super) use warpui::elements::{
        Align, Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, Hoverable,
        OffsetPositioning, ParentAnchor, ParentOffsetBounds, Radius, Rect, Stack,
    };
    pub(super) const WINDOWS_BRIGHT_RED: ColorU = ColorU {
        r: 232,
        g: 17,
        b: 32,
        a: u8::MAX,
    };

    pub(super) const WINDOWS_BUTTON_PADDING_VERTICAL: f32 = 6.;
    pub(super) const WINDOWS_BUTTON_PADDING_HORIZONTAL: f32 = 12.;
}

#[cfg(target_os = "windows")]
use windows_only::*;

#[cfg(not(target_os = "windows"))]
use warpui::elements::Empty;

use crate::themes::theme::WarpTheme;
use warpui::elements::MouseStateHandle;
use warpui::platform::FullscreenState;
use warpui::{AppContext, Element, WindowId};

#[cfg(any(target_os = "windows", any(target_os = "linux", target_os = "freebsd")))]
const BUTTON_ICON_SIZE: f32 = 22.;

pub fn traffic_light_data(ctx: &AppContext, window_id: WindowId) -> Option<TrafficLightData> {
    // If native window frame is on, the traffic lights are already in the frame.
    if ctx
        .windows()
        .platform_window(window_id)
        .is_some_and(|window| window.uses_native_window_decorations())
    {
        return None;
    }

    if cfg!(target_os = "macos") {
        Some(TrafficLightData {
            width: 64.,
            side: TrafficLightSide::Left,
            scales_with_zoom: false,
        })
    } else if cfg!(any(target_os = "linux", target_os = "freebsd"))
        && !ctx.windows().is_tiling_window_manager()
    {
        Some(TrafficLightData {
            width: 116.,
            side: TrafficLightSide::Right,
            scales_with_zoom: true,
        })
    } else if cfg!(target_os = "windows") {
        Some(TrafficLightData {
            width: 136.,
            side: TrafficLightSide::Right,
            scales_with_zoom: true,
        })
    } else {
        None
    }
}

/// Are they in the upper-right or upper-left corner?
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrafficLightSide {
    Left,
    Right,
}

/// Mouse state handles that the containing View must manage.
#[derive(Default)]
#[cfg_attr(any(target_family = "wasm", target_os = "macos"), allow(dead_code))]
pub struct TrafficLightMouseStates {
    pub minimize_window_button: MouseStateHandle,
    pub maximize_window_button: MouseStateHandle,
    pub close_window_button: MouseStateHandle,
}

impl TrafficLightMouseStates {
    /// True if any of the traffic light buttons are hovered.
    pub fn are_traffic_lights_hovered(&self) -> bool {
        [
            &self.minimize_window_button,
            &self.maximize_window_button,
            &self.close_window_button,
        ]
        .into_iter()
        .any(|state| state.lock().is_ok_and(|state| state.is_hovered()))
    }
}

/// Data the Warp app needs to avoid rendering anything below the traffic lights.
#[derive(Clone, Debug)]
pub struct TrafficLightData {
    width: f32,
    pub side: TrafficLightSide,
    /// Whether the traffic lights can scale with the app's zoom level.
    ///
    /// If `false` that means the traffic light buttons are of fixed size as determined by the OS
    /// and we cannot scale them as the user configures the zoom level.
    scales_with_zoom: bool,
}

impl TrafficLightData {
    /// Horizontal space needed for the traffic light buttons.
    ///
    /// Normally, we don't need to manually adjust any sizes based on zoom level as it is handled
    /// by warpui. However, native traffic light buttons (e.g. on macOS) don't scale with zoom, so
    /// we need to divide by the zoom factor to keep the padding constant.
    pub fn width(&self, zoom_factor: f32) -> f32 {
        if self.scales_with_zoom {
            self.width
        } else {
            self.width / zoom_factor
        }
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub fn render(
        &self,
        fullscreen_state: FullscreenState,
        mouse_states: &TrafficLightMouseStates,
        theme: &WarpTheme,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        if !cfg!(any(target_os = "linux", target_os = "freebsd")) {
            return Empty::new().finish();
        }

        let fg_color = theme.foreground().into_solid();
        let maximize_button_icon =
            Self::render_linux_maximize_button_icon(fg_color, fullscreen_state);

        ConstrainedBox::new(
            Align::new(
                Flex::row()
                    .with_children([
                        Container::new(
                            Self::render_button(
                                Arc::clone(&mouse_states.minimize_window_button),
                                ConstrainedBox::new(
                                    Rect::new().with_background_color(fg_color).finish(),
                                )
                                .with_height(2.)
                                .with_width(8.)
                                .finish(),
                                theme,
                            )
                            .on_click(|evt, _, _| {
                                evt.dispatch_action("root_view:minimize_window", ());
                            })
                            .finish(),
                        )
                        .with_margin_right(16.)
                        .finish(),
                        Self::render_button(
                            Arc::clone(&mouse_states.maximize_window_button),
                            maximize_button_icon,
                            theme,
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
                            Self::render_button(
                                Arc::clone(&mouse_states.close_window_button),
                                ConstrainedBox::new(
                                    Icon::new("bundled/svg/linux/decorations/close.svg", fg_color)
                                        .finish(),
                                )
                                .with_height(8.)
                                .with_width(8.)
                                .finish(),
                                theme,
                            )
                            .on_click(|evt, _, _| {
                                evt.dispatch_action("root_view:close_window", ());
                            })
                            .finish(),
                        )
                        .with_margin_left(16.)
                        .with_margin_right(12.)
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

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn render_linux_maximize_button_icon(
        fg_color: ColorU,
        fullscreen_state: FullscreenState,
    ) -> Box<dyn Element> {
        let mut maximize_button_icon = ConstrainedBox::new(
            Rect::new()
                .with_border(Border::all(2.).with_border_color(fg_color))
                .finish(),
        )
        .with_width(6.)
        .with_height(6.)
        .finish();

        // If the window is already maximized, the icon looks a bit different.
        if fullscreen_state != FullscreenState::Normal {
            let mut stack = Stack::new();
            stack.add_positioned_child(
                maximize_button_icon,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::BottomLeft,
                    ChildAnchor::BottomLeft,
                ),
            );
            stack.add_positioned_child(
                ConstrainedBox::new(
                    Rect::new()
                        .with_border(
                            Border::new(1.)
                                .with_sides(true, false, false, true)
                                .with_border_color(fg_color),
                        )
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(1.)))
                        .finish(),
                )
                .with_width(6.)
                .with_height(6.)
                .finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::TopRight,
                    ChildAnchor::TopRight,
                ),
            );
            maximize_button_icon = ConstrainedBox::new(stack.finish())
                .with_width(8.)
                .with_height(8.)
                .finish();
        }

        maximize_button_icon
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn render_button(
        mouse_state: MouseStateHandle,
        child: Box<dyn Element>,
        theme: &WarpTheme,
    ) -> Hoverable {
        Hoverable::new(mouse_state, |state| {
            let background_color = if state.is_hovered() {
                theme.surface_3()
            } else {
                theme.surface_2()
            };
            Container::new(
                ConstrainedBox::new(Align::new(child).finish())
                    .with_width(BUTTON_ICON_SIZE)
                    .with_height(BUTTON_ICON_SIZE)
                    .finish(),
            )
            .with_background(background_color)
            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
            .finish()
        })
    }

    #[cfg(target_os = "windows")]
    pub fn render(
        &self,
        fullscreen_state: FullscreenState,
        mouse_states: &TrafficLightMouseStates,
        theme: &WarpTheme,
        app: &AppContext,
    ) -> Box<dyn Element> {
        self.render_tab_row(fullscreen_state, mouse_states, theme, app)
    }

    #[cfg(target_os = "windows")]
    fn render_windows_minimize_button_icon(fg_color: ColorU) -> Box<dyn Element> {
        ConstrainedBox::new(Rect::new().with_background_color(fg_color).finish())
            .with_height(1.)
            .with_width(12.)
            .finish()
    }

    #[cfg(target_os = "windows")]
    fn render_windows_maximize_button_icon(
        fg_color: ColorU,
        fullscreen_state: FullscreenState,
    ) -> Box<dyn Element> {
        let mut maximize_button_icon = ConstrainedBox::new(
            Rect::new()
                .with_border(Border::all(1.).with_border_color(fg_color))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
                .finish(),
        )
        .with_width(10.)
        .with_height(10.)
        .finish();

        // If the window is already maximized, the icon looks a bit different.
        if fullscreen_state != FullscreenState::Normal {
            let mut stack = Stack::new();
            stack.add_positioned_child(
                maximize_button_icon,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::BottomLeft,
                    ChildAnchor::BottomLeft,
                ),
            );
            stack.add_positioned_child(
                ConstrainedBox::new(
                    Rect::new()
                        .with_border(
                            Border::new(1.)
                                .with_sides(true, false, false, true)
                                .with_border_color(fg_color),
                        )
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
                        .finish(),
                )
                .with_width(10.)
                .with_height(10.)
                .finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::TopRight,
                    ChildAnchor::TopRight,
                ),
            );
            maximize_button_icon = ConstrainedBox::new(stack.finish())
                .with_width(12.)
                .with_height(12.)
                .finish();
        }

        maximize_button_icon
    }

    #[cfg(target_os = "windows")]
    fn render_windows_close_button(fg_color: ColorU, mouse_state: MouseStateHandle) -> Hoverable {
        Hoverable::new(mouse_state, |state| {
            let (background_color, icon_color) = if state.is_hovered() {
                (WINDOWS_BRIGHT_RED, ColorU::white())
            } else {
                (ColorU::transparent_black(), fg_color)
            };

            Container::new(
                ConstrainedBox::new(
                    Align::new(Self::render_windows_close_button_icon(icon_color)).finish(),
                )
                .with_width(BUTTON_ICON_SIZE)
                .with_height(BUTTON_ICON_SIZE)
                .finish(),
            )
            .with_vertical_padding(WINDOWS_BUTTON_PADDING_VERTICAL)
            .with_horizontal_padding(WINDOWS_BUTTON_PADDING_HORIZONTAL)
            .with_background_color(background_color)
            .finish()
        })
    }

    #[cfg(target_os = "windows")]
    fn render_windows_close_button_icon(icon_color: ColorU) -> Box<dyn Element> {
        ConstrainedBox::new(
            IconComponent::X
                .to_warpui_icon(theme::Fill::Solid(icon_color))
                .finish(),
        )
        .with_height(16.)
        .with_width(16.)
        .finish()
    }

    #[cfg(target_os = "windows")]
    fn render_button(
        mouse_state: MouseStateHandle,
        child: Box<dyn Element>,
        hover_color: ColorU,
    ) -> Hoverable {
        Hoverable::new(mouse_state, |state| {
            let background_color = if state.is_hovered() {
                hover_color
            } else {
                ColorU::transparent_black()
            };
            Container::new(
                ConstrainedBox::new(Align::new(child).finish())
                    .with_width(BUTTON_ICON_SIZE)
                    .finish(),
            )
            .with_vertical_padding(WINDOWS_BUTTON_PADDING_VERTICAL)
            .with_horizontal_padding(WINDOWS_BUTTON_PADDING_HORIZONTAL)
            .with_background_color(background_color)
            .finish()
        })
    }

    #[cfg(all(
        not(any(target_os = "linux", target_os = "freebsd")),
        not(target_os = "windows")
    ))]
    pub fn render(
        &self,
        _fullscreen_state: FullscreenState,
        _mouse_states: &TrafficLightMouseStates,
        _theme: &WarpTheme,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        Empty::new().finish()
    }
}
