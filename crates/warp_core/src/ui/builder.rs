use std::borrow::Cow;
use std::rc::Rc;

use warpui::color::ColorU;
use warpui::elements::{
    ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, Hoverable,
    Icon, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds,
    Radius, Stack, Text,
};
use warpui::fonts::{FamilyId, Properties, Weight};
use warpui::geometry::vector::{vec2f, Vector2F};
use warpui::keymap::Keystroke;
use warpui::platform::Cursor;
use warpui::ui_components::button::{Button, ButtonVariant};
use warpui::ui_components::checkbox::Checkbox;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::keyboard_shortcut::KeyboardShortcut;
use warpui::ui_components::link::{Link, LinkStyles, OnClickFn};
use warpui::ui_components::list::{List, ListStyle};
use warpui::ui_components::progress_bar::ProgressBar;
use warpui::ui_components::radio_buttons::{
    RadioButtonItem, RadioButtonLayout, RadioButtonStateHandle, RadioButtons,
};
use warpui::ui_components::slider::{Slider, SliderStateHandle};
use warpui::ui_components::switch::{Switch, SwitchStateHandle, TRACK_COLOR};
use warpui::ui_components::text::{Paragraph, Span, WrappableText};
use warpui::ui_components::text_input::TextInput;
use warpui::ui_components::toggle_menu::{
    ToggleMenu, ToggleMenuCallback, ToggleMenuItem, ToggleMenuStateHandle,
};
use warpui::ui_components::tool_tip::{Tooltip, TooltipWithSublabel};
use warpui::{Element, View, ViewHandle};

use super::color::blend::Blend;
use super::color::contrast::MinimumAllowedContrast;
use super::color::ContrastingColor;
use super::theme::color::internal_colors::{self, text_main};
use super::theme::{Fill, WarpTheme};

const CLOSE_SVG_PATH: &str = "bundled/svg/close.svg";
const COPY_SVG_PATH: &str = "bundled/svg/copy.svg";
const INFO_SVG_PATH: &str = "bundled/svg/info.svg";
pub const CHECK_SVG_PATH: &str = "bundled/svg/check-thick.svg";
const PLUS_SVG_PATH: &str = "bundled/svg/add.svg";
const LEFT_CHEVRON_PATH: &str = "bundled/svg/chevron-left.svg";
const HELP_SVG_PATH: &str = "bundled/svg/help-circle.svg";
const ENTER_SVG_PATH: &str = "bundled/svg/enter.svg";
const RETRY_SVG_PATH: &str = "bundled/svg/retry.svg";
const LOCAL_ONLY_SVG_PATH: &str = "bundled/svg/cloud-off.svg";

pub const MIN_FONT_SIZE: f32 = 5.;
pub const DEFAULT_KEYBOARD_SHORTCUT_HEIGHT: f32 = 24.;

#[derive(Clone, Debug)]
pub struct UiBuilder {
    warp_theme: WarpTheme,
    ui_font_family: FamilyId,
    ui_font_size: f32,
    command_palette_font_size: f32,
    line_height_ratio: f32,
}

impl UiBuilder {
    pub fn new(
        warp_theme: WarpTheme,
        ui_font_family: FamilyId,
        ui_font_size: f32,
        command_palette_font_size: f32,
        line_height_ratio: f32,
    ) -> Self {
        UiBuilder {
            warp_theme,
            ui_font_family,
            ui_font_size,
            command_palette_font_size,
            line_height_ratio,
        }
    }

    fn base_styles(
        &self,
        background: Option<Fill>,
        border: Fill,
        font_color: Fill,
    ) -> UiComponentStyles {
        UiComponentStyles {
            font_size: Some(self.ui_font_size),
            font_family_id: Some(self.ui_font_family),
            padding: Some(Coords {
                top: 5.,
                bottom: 5.,
                left: 15.,
                right: 15.,
            }),
            border_width: Some(1.),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
            background: background.map(|background| background.into()),
            border_color: Some(border.into()),
            font_color: Some(font_color.into_solid()),
            ..Default::default()
        }
    }

    fn text_button_styles(&self, font_color: Fill) -> UiComponentStyles {
        UiComponentStyles {
            font_size: Some(self.ui_font_size),
            font_family_id: Some(self.ui_font_family),
            font_color: Some(font_color.into_solid()),
            ..Default::default()
        }
    }

    fn default_text_input_styles(&self) -> UiComponentStyles {
        UiComponentStyles {
            font_size: None,
            font_family_id: None,
            padding: Some(Coords {
                top: 10.,
                bottom: 10.,
                left: 10.,
                right: 10.,
            }),
            border_width: Some(1.),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
            background: Some(self.warp_theme.background().into()),
            border_color: Some(self.warp_theme.foreground().with_opacity(20).into()),
            font_color: None,
            ..Default::default()
        }
    }

    fn default_progress_bar_styles(&self) -> UiComponentStyles {
        UiComponentStyles {
            background: Some(self.warp_theme.background().into()),
            foreground: Some(self.warp_theme.accent().into()),
            width: Some(70.),
            height: Some(2.),
            ..Default::default()
        }
    }

    pub fn default_tool_tip_styles(&self) -> UiComponentStyles {
        UiComponentStyles {
            background: Some(self.warp_theme().tooltip_background().into()),
            font_color: Some(self.warp_theme.background().into_solid()),
            padding: Some(Coords {
                top: 4.,
                bottom: 4.,
                left: 8.,
                right: 8.,
            }),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
            border_width: Some(1.),
            border_color: Some(self.warp_theme.outline().into()),
            font_family_id: Some(self.ui_font_family),
            font_size: Some(10.),
            ..Default::default()
        }
    }

    fn autosuggestion_tool_tip_styles(&self) -> UiComponentStyles {
        UiComponentStyles {
            background: Some(self.warp_theme().tooltip_background().into()),
            font_color: Some(self.warp_theme.background().into_solid()),
            padding: Some(Coords {
                top: 6.,
                bottom: 6.,
                left: 14.,
                right: 14.,
            }),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
            border_width: Some(1.),
            border_color: Some(self.warp_theme.outline().into()),
            font_family_id: Some(self.ui_font_family),
            font_size: Some(10.),
            ..Default::default()
        }
    }

    // TODO default/hovered/clicked can most likely be refactored even more
    fn default_button_styles(&self, variant: ButtonVariant) -> UiComponentStyles {
        let details = self.warp_theme.details();
        let (background, border) = match variant {
            ButtonVariant::Accent => (
                self.warp_theme.accent().blend(
                    &self
                        .warp_theme
                        .foreground()
                        .with_opacity(*details.accent_button_opacity()),
                ),
                self.warp_theme.foreground().with_opacity(0),
            ),
            ButtonVariant::Basic => (
                self.warp_theme.background().blend(
                    &self
                        .warp_theme
                        .foreground()
                        .with_opacity(*details.foreground_button_opacity()),
                ),
                self.warp_theme.foreground().with_opacity(0),
            ),
            ButtonVariant::Secondary => {
                return self.base_styles(
                    None,
                    self.warp_theme.outline(),
                    self.warp_theme
                        .main_text_color(self.warp_theme.background()),
                )
            }
            ButtonVariant::Warn => (Fill::warn(), self.warp_theme.foreground().with_opacity(0)),
            ButtonVariant::Error => (Fill::error(), self.warp_theme.foreground().with_opacity(0)),
            ButtonVariant::Outlined => {
                return self.base_styles(
                    None,
                    self.warp_theme.foreground().with_opacity(20),
                    self.warp_theme.sub_text_color(self.warp_theme.background()),
                );
            }
            ButtonVariant::Text => {
                return self.text_button_styles(
                    self.warp_theme.sub_text_color(self.warp_theme.background()),
                );
            }
            ButtonVariant::Link => {
                return self.text_button_styles(self.warp_theme.accent());
            }
        };
        let font_color = match variant {
            ButtonVariant::Outlined => self.warp_theme.sub_text_color(background),
            ButtonVariant::Text => unreachable!(),
            _ => self.warp_theme.main_text_color(background),
        };

        self.base_styles(Some(background), background.blend(&border), font_color)
    }

    fn disabled_button_styles(&self, variant: ButtonVariant) -> UiComponentStyles {
        let background: Fill = self.warp_theme().surface_3();
        let font_color = self.warp_theme().disabled_text_color(background);
        match variant {
            // TODO: we should re-investigate if we want to do this for disabled Text buttons,
            // as it doesn't conform to our design specification here:
            // https://www.figma.com/file/chk9pwt35jTJhf9KnHmZyE/Components?node-id=401%3A260.
            ButtonVariant::Text => self.text_button_styles(font_color),
            ButtonVariant::Link => self.text_button_styles(font_color),
            _ => self.base_styles(Some(background), background, font_color),
        }
    }

    fn hovered_button_styles(&self, variant: ButtonVariant) -> UiComponentStyles {
        let details = self.warp_theme.details();
        let (background, border) = match variant {
            ButtonVariant::Accent => (
                self.warp_theme
                    .accent()
                    .blend(&self.warp_theme.foreground().with_opacity(
                        details.accent_button_opacity() + details.button_hover_opacity(),
                    )),
                self.warp_theme.foreground().with_opacity(20),
            ),
            ButtonVariant::Basic => (
                self.warp_theme
                    .background()
                    .blend(&self.warp_theme.foreground().with_opacity(
                        details.foreground_button_opacity() + details.button_hover_opacity(),
                    )),
                self.warp_theme.foreground().with_opacity(20),
            ),
            ButtonVariant::Secondary => (self.warp_theme.surface_3(), self.warp_theme.outline()),
            ButtonVariant::Warn => (
                Fill::warn().with_opacity(80),
                self.warp_theme.foreground().with_opacity(20),
            ),
            ButtonVariant::Error => (
                Fill::error().with_opacity(80),
                self.warp_theme.foreground().with_opacity(20),
            ),
            ButtonVariant::Outlined => {
                return self.base_styles(
                    None,
                    self.warp_theme.accent(),
                    self.warp_theme.sub_text_color(self.warp_theme.background()),
                );
            }
            ButtonVariant::Text => {
                return self.text_button_styles(
                    self.warp_theme
                        .main_text_color(self.warp_theme.background()),
                );
            }
            ButtonVariant::Link => {
                return self.text_button_styles(self.warp_theme.accent());
            }
        };
        let font_color = match variant {
            ButtonVariant::Outlined => self.warp_theme.sub_text_color(background),
            ButtonVariant::Text => unreachable!(),
            _ => self.warp_theme.main_text_color(background),
        };

        self.base_styles(Some(background), background.blend(&border), font_color)
    }

    fn clicked_button_styles(&self, variant: ButtonVariant) -> UiComponentStyles {
        let details = self.warp_theme.details();
        let (background, border) = match variant {
            ButtonVariant::Accent => (
                self.warp_theme
                    .accent()
                    .blend(&Fill::black().with_opacity(*details.button_click_opacity())),
                Fill::black().with_opacity(30),
            ),
            ButtonVariant::Basic => (
                self.warp_theme
                    .background()
                    .blend(&Fill::black().with_opacity(*details.button_click_opacity())),
                Fill::black().with_opacity(30),
            ),
            ButtonVariant::Secondary => (self.warp_theme.surface_3(), self.warp_theme.outline()),
            ButtonVariant::Warn => (Fill::warn(), Fill::black().with_opacity(30)),
            ButtonVariant::Error => (Fill::error(), Fill::black().with_opacity(30)),
            ButtonVariant::Outlined => (
                self.warp_theme.accent().blend(
                    &self
                        .warp_theme
                        .foreground()
                        .with_opacity(*details.accent_button_opacity()),
                ),
                Fill::black().with_opacity(30),
            ),
            ButtonVariant::Text => {
                return self.text_button_styles(
                    self.warp_theme
                        .main_text_color(self.warp_theme.background()),
                );
            }
            ButtonVariant::Link => {
                return self.text_button_styles(self.warp_theme.accent());
            }
        };
        self.base_styles(
            Some(background),
            background.blend(&border),
            self.warp_theme.main_text_color(background),
        )
    }

    pub fn default_keyboard_shortcut_styles(&self) -> UiComponentStyles {
        UiComponentStyles {
            font_family_id: Some(self.ui_font_family),
            font_size: Some(self.command_palette_font_size),
            font_color: Some(
                self.warp_theme
                    .hint_text_color(self.warp_theme.background())
                    .into_solid(),
            ),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
            background: Some(internal_colors::fg_overlay_1(self.warp_theme()).into()),
            height: Some(DEFAULT_KEYBOARD_SHORTCUT_HEIGHT),
            padding: Some(Coords::default().left(6.).right(6.)),
            margin: Some(Coords::default().left(3.)),
            ..Default::default()
        }
    }

    pub fn button(&self, variant: ButtonVariant, mouse_state: MouseStateHandle) -> Button {
        Button::new(
            mouse_state,
            self.default_button_styles(variant),
            Some(self.hovered_button_styles(variant)),
            Some(self.clicked_button_styles(variant)),
            Some(self.disabled_button_styles(variant)),
        )
    }

    pub fn reset_button(
        &self,
        variant: ButtonVariant,
        mouse_state: MouseStateHandle,
        changed_from_default: bool,
        disabled_text_color: ColorU,
    ) -> Button {
        let mut button = Button::new(
            mouse_state,
            self.default_button_styles(variant),
            Some(self.hovered_button_styles(variant)),
            Some(self.clicked_button_styles(variant)),
            Some(self.disabled_button_styles(variant)),
        );
        if !changed_from_default {
            button = button.disabled().with_style(UiComponentStyles {
                font_color: Some(disabled_text_color),
                ..Default::default()
            });
        }
        button
    }

    pub fn button_with_custom_styles(
        &self,
        variant: ButtonVariant,
        mouse_state: MouseStateHandle,
        default_styles: UiComponentStyles,
        hovered_styles: Option<UiComponentStyles>,
        clicked_styles: Option<UiComponentStyles>,
        disabled_styles: Option<UiComponentStyles>,
    ) -> Button {
        Button::new(
            mouse_state,
            self.default_button_styles(variant).merge(default_styles),
            hovered_styles,
            clicked_styles,
            disabled_styles,
        )
    }

    pub fn progress_bar(&self, progress: f32) -> ProgressBar {
        ProgressBar::new(progress, self.default_progress_bar_styles())
    }

    pub fn custom_progress_bar(
        &self,
        progress: f32,
        custom_styles: UiComponentStyles,
    ) -> ProgressBar {
        ProgressBar::new(progress, custom_styles)
    }

    pub fn tool_tip(&self, label: String) -> Tooltip {
        Tooltip::new(label, self.default_tool_tip_styles())
    }

    pub fn tool_tip_on_element(
        &self,
        label: String,
        mouse_state_handle: MouseStateHandle,
        element: Box<dyn Element>,
        element_anchor: ParentAnchor,
        tooltip_anchor: ChildAnchor,
        offset_from_element: Vector2F,
    ) -> Box<dyn Element> {
        self.styled_tool_tip_on_element(
            label,
            None,
            mouse_state_handle,
            element,
            element_anchor,
            tooltip_anchor,
            offset_from_element,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn styled_tool_tip_on_element(
        &self,
        label: String,
        styles: Option<UiComponentStyles>,
        mouse_state_handle: MouseStateHandle,
        element: Box<dyn Element>,
        element_anchor: ParentAnchor,
        tooltip_anchor: ChildAnchor,
        offset_from_element: Vector2F,
    ) -> Box<dyn Element> {
        self.styled_tool_tip_on_element_internal(
            label,
            styles,
            mouse_state_handle,
            element,
            element_anchor,
            tooltip_anchor,
            offset_from_element,
            false,
        )
    }

    /// Like `tool_tip_on_element`, but renders the tooltip as an overlay so it
    /// is not clipped by parent `Clipped` elements.
    pub fn overlay_tool_tip_on_element(
        &self,
        label: String,
        mouse_state_handle: MouseStateHandle,
        element: Box<dyn Element>,
        element_anchor: ParentAnchor,
        tooltip_anchor: ChildAnchor,
        offset_from_element: Vector2F,
    ) -> Box<dyn Element> {
        self.styled_tool_tip_on_element_internal(
            label,
            None,
            mouse_state_handle,
            element,
            element_anchor,
            tooltip_anchor,
            offset_from_element,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn styled_tool_tip_on_element_internal(
        &self,
        label: String,
        styles: Option<UiComponentStyles>,
        mouse_state_handle: MouseStateHandle,
        element: Box<dyn Element>,
        element_anchor: ParentAnchor,
        tooltip_anchor: ChildAnchor,
        offset_from_element: Vector2F,
        overlay: bool,
    ) -> Box<dyn Element> {
        Hoverable::new(mouse_state_handle, |state| {
            let mut stack = Stack::new().with_child(element);
            if state.is_hovered() {
                let mut tool_tip = self.tool_tip(label);
                if let Some(styles) = styles {
                    tool_tip = tool_tip.with_style(styles)
                }
                let tool_tip = tool_tip.build().finish();
                let offset = OffsetPositioning::offset_from_parent(
                    offset_from_element,
                    if overlay {
                        ParentOffsetBounds::WindowByPosition
                    } else {
                        ParentOffsetBounds::Unbounded
                    },
                    element_anchor,
                    tooltip_anchor,
                );
                if overlay {
                    stack.add_positioned_overlay_child(tool_tip, offset);
                } else {
                    stack.add_positioned_child(tool_tip, offset);
                }
            }
            stack.finish()
        })
        .finish()
    }

    pub fn tool_tip_with_sublabel(&self, label: String, sublabel: String) -> TooltipWithSublabel {
        TooltipWithSublabel::new(label, sublabel, self.default_tool_tip_styles())
    }

    pub fn autosuggestion_tool_tip(&self, label: String) -> Tooltip {
        Tooltip::new(label, self.autosuggestion_tool_tip_styles())
    }

    pub fn link(
        &self,
        description: String,
        url: Option<String>,
        callback: Option<OnClickFn>,
        mouse_state: MouseStateHandle,
    ) -> Link {
        Link::new(description, url, callback, mouse_state, self.link_styles())
    }

    pub fn tooltip_link(
        &self,
        description: String,
        url: Option<String>,
        callback: Option<OnClickFn>,
        mouse_state: MouseStateHandle,
    ) -> Link {
        Link::new(
            description,
            url,
            callback,
            mouse_state,
            self.tooltip_link_styles(),
        )
    }

    pub fn slider(&self, slider_state_handle: SliderStateHandle) -> Slider {
        Slider::new(slider_state_handle)
    }

    pub fn link_styles(&self) -> LinkStyles {
        let font_color: ColorU = self
            .warp_theme
            .accent()
            .on_background(self.warp_theme.surface_2(), MinimumAllowedContrast::Text)
            .into();

        let base_link_style = UiComponentStyles {
            font_size: Some(self.ui_font_size),
            font_family_id: Some(self.ui_font_family),
            font_color: Some(font_color),
            border_width: Some(1.),
            ..Default::default()
        };

        let hover_link_style = UiComponentStyles {
            border_color: Some(self.warp_theme.accent().into()),
            ..base_link_style
        };
        LinkStyles {
            base: base_link_style,
            hovered: Some(hover_link_style),
            clicked: Some(hover_link_style),
            soft_wrap: false,
        }
    }

    pub fn tooltip_link_styles(&self) -> LinkStyles {
        let tooltip_background = self.warp_theme().tooltip_background();
        let font_color: ColorU = self
            .warp_theme
            .accent()
            .on_background(
                Fill::Solid(tooltip_background),
                MinimumAllowedContrast::Text,
            )
            .into();

        let base_link_style = UiComponentStyles {
            font_size: Some(self.ui_font_size),
            font_family_id: Some(self.ui_font_family),
            font_color: Some(font_color),
            border_width: Some(1.),
            ..Default::default()
        };

        let hover_link_style = UiComponentStyles {
            border_color: Some(self.warp_theme.accent().into()),
            ..base_link_style
        };
        LinkStyles {
            base: base_link_style,
            hovered: Some(hover_link_style),
            clicked: Some(hover_link_style),
            soft_wrap: false,
        }
    }

    pub fn switch(&self, mouse_state: SwitchStateHandle) -> Switch {
        let switch_margin_styles = UiComponentStyles {
            margin: Some(Coords {
                top: 5.,
                bottom: 5.,
                left: 0.,
                right: 5.,
            }),
            ..Default::default()
        };

        Switch::new(
            mouse_state,
            switch_margin_styles.merge(self.base_styles(
                Some(Fill::Solid(*TRACK_COLOR)),
                Fill::white(),
                self.warp_theme.main_text_color(self.warp_theme.surface_2()),
            )),
            None,
            Some(switch_margin_styles.merge(self.base_styles(
                Some(self.warp_theme.accent()),
                Fill::white(),
                self.warp_theme.main_text_color(self.warp_theme.surface_2()),
            ))),
            Some(switch_margin_styles.merge(self.base_styles(
                Some(self.warp_theme.disabled_ui_text_color()),
                Fill::white(),
                self.warp_theme.main_text_color(self.warp_theme.surface_2()),
            ))),
        )
        .with_thumb_hover_border(10.)
    }

    pub fn checkbox(&self, mouse_state: MouseStateHandle, size: Option<f32>) -> Checkbox {
        let checkbox_size = size.or(Some(12.));

        let checked_checkbox_override_styles = UiComponentStyles {
            font_size: checkbox_size,
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(2.))),
            ..Default::default()
        };

        let default_checkbox_override_styles =
            checked_checkbox_override_styles.merge(UiComponentStyles {
                border_width: Some(1.),
                ..Default::default()
            });

        Checkbox::new(
            mouse_state,
            self.base_styles(
                None,
                self.warp_theme.outline(),
                self.warp_theme.nonactive_ui_text_color(),
            )
            .merge(default_checkbox_override_styles),
            None,
            Some(
                self.base_styles(
                    Some(self.warp_theme.accent()),
                    self.warp_theme.outline(),
                    self.warp_theme.font_color(self.warp_theme.accent()),
                )
                .merge(checked_checkbox_override_styles),
            ),
            None,
        )
    }

    pub fn radio_buttons<'a>(
        &self,
        mouse_states: Vec<MouseStateHandle>,
        items: Vec<RadioButtonItem<'a>>,
        radio_button_state_handle: RadioButtonStateHandle,
        default_option: Option<usize>,
        font_size: f32,
        layout: RadioButtonLayout,
    ) -> RadioButtons<'a> {
        RadioButtons::new(
            mouse_states,
            items,
            radio_button_state_handle,
            default_option,
            self.base_styles(
                None,
                self.warp_theme.nonactive_ui_text_color(),
                self.warp_theme.active_ui_text_color(),
            )
            .set_font_size(font_size),
            self.base_styles(
                Some(self.warp_theme.accent()),
                self.warp_theme.accent(),
                self.warp_theme.active_ui_text_color(),
            )
            .set_font_size(font_size),
            self.base_styles(
                None,
                self.warp_theme.disabled_ui_text_color(),
                self.warp_theme.disabled_ui_text_color(),
            )
            .set_font_size(font_size),
            layout,
        )
    }

    fn toggle_menu_styles(&self, background: Fill, font_color: Fill) -> UiComponentStyles {
        UiComponentStyles {
            font_size: Some(self.ui_font_size),
            font_family_id: Some(self.ui_font_family),
            background: Some(background.into()),
            font_color: Some(font_color.into_solid()),
            ..Default::default()
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn toggle_menu(
        &self,
        mouse_states: Vec<MouseStateHandle>,
        items: Vec<ToggleMenuItem>,
        toggle_menu_state_handle: ToggleMenuStateHandle,
        default_option: Option<usize>,
        default_background: Option<Fill>,
        selected_background: Option<Fill>,
        hover_background: Option<Fill>,
        font_size: f32,
        on_toggle_change: Rc<ToggleMenuCallback>,
    ) -> ToggleMenu {
        ToggleMenu::new(
            mouse_states,
            items,
            toggle_menu_state_handle,
            default_option,
            self.toggle_menu_styles(
                default_background.unwrap_or(self.warp_theme.surface_1()),
                self.warp_theme.active_ui_text_color(),
            )
            .set_font_size(font_size),
            self.toggle_menu_styles(
                selected_background.unwrap_or(self.warp_theme.surface_3()),
                self.warp_theme.active_ui_text_color(),
            ),
            self.toggle_menu_styles(
                hover_background.unwrap_or(self.warp_theme.surface_2()),
                self.warp_theme.active_ui_text_color(),
            ),
            on_toggle_change,
        )
    }

    pub fn span(&self, text: impl Into<Cow<'static, str>>) -> Span {
        Span::new(
            text,
            self.base_styles(
                Some(self.warp_theme.surface_2()),
                self.warp_theme.outline(),
                self.warp_theme.main_text_color(self.warp_theme.surface_2()),
            ),
        )
    }

    pub fn list(&self, list_style: ListStyle, items: Vec<String>) -> List {
        List::new(
            list_style,
            self.base_styles(
                Some(self.warp_theme.surface_2()),
                self.warp_theme.outline(),
                self.warp_theme.main_text_color(self.warp_theme.surface_2()),
            ),
            items,
        )
    }

    pub fn paragraph(&self, text: impl Into<Cow<'static, str>>) -> Paragraph {
        Paragraph::new(
            text,
            self.base_styles(
                Some(self.warp_theme.surface_2()),
                self.warp_theme.outline(),
                self.warp_theme.main_text_color(self.warp_theme.surface_2()),
            ),
        )
    }

    pub fn wrappable_text(
        &self,
        text: impl Into<Cow<'static, str>>,
        soft_wrap: bool,
    ) -> WrappableText {
        WrappableText::new(
            text.into(),
            soft_wrap,
            self.base_styles(
                Some(self.warp_theme.surface_2()),
                self.warp_theme.outline(),
                self.warp_theme.main_text_color(self.warp_theme.surface_2()),
            ),
        )
    }

    pub fn label(&self, text: impl Into<Cow<'static, str>>) -> Span {
        self.span(text)
    }

    pub fn text_input<T>(&self, editor: ViewHandle<T>) -> TextInput<T>
    where
        T: View,
    {
        TextInput::new(editor, self.default_text_input_styles())
    }

    pub fn animated_button(
        &self,
        mouse_state: MouseStateHandle,
        icon_path: &'static str,
        options: AnimatedButtonOptions,
    ) -> Button {
        let color = options.color.unwrap_or_else(|| {
            self.warp_theme
                .foreground()
                .on_background(self.warp_theme.surface_2(), MinimumAllowedContrast::NonText)
        });

        let border_color = if options.with_accent_animations {
            self.warp_theme.accent()
        } else {
            self.warp_theme.outline()
        };

        let border_radius = if options.circular {
            Some(CornerRadius::with_all(Radius::Percentage(50.)))
        } else {
            Some(CornerRadius::with_all(Radius::Percentage(20.)))
        };

        let base_styles = UiComponentStyles {
            height: Some(options.size),
            width: Some(options.size),
            border_radius,
            padding: options.padding.map(Coords::uniform),
            ..Default::default()
        };

        let hovered_styles = base_styles.merge(UiComponentStyles {
            background: Some(internal_colors::fg_overlay_2(&self.warp_theme).into()),
            ..Default::default()
        });

        let clicked_styles = hovered_styles.merge(UiComponentStyles {
            border_color: Some(border_color.into()),
            border_width: Some(1.),
            ..Default::default()
        });

        let disabled_button_styles = base_styles.merge(UiComponentStyles {
            font_color: Some(
                self.warp_theme()
                    .disabled_text_color(internal_colors::neutral_3(&self.warp_theme).into())
                    .into(),
            ),
            ..Default::default()
        });

        let icon = Icon::new(icon_path, color);

        Button::new(
            mouse_state,
            base_styles,
            Some(hovered_styles),
            Some(clicked_styles),
            Some(disabled_button_styles),
        )
        .with_icon_label(icon)
    }

    pub fn close_button(&self, size: f32, mouse_state: MouseStateHandle) -> Button {
        self.animated_button(
            mouse_state,
            CLOSE_SVG_PATH,
            AnimatedButtonOptions {
                size,
                padding: None,
                color: None,
                with_accent_animations: false,
                circular: false,
            },
        )
    }

    pub fn retry_button(&self, size: f32, mouse_state: MouseStateHandle) -> Button {
        self.animated_button(
            mouse_state,
            RETRY_SVG_PATH,
            AnimatedButtonOptions {
                size,
                padding: None,
                color: None,
                with_accent_animations: false,
                circular: false,
            },
        )
    }

    pub fn left_chevron_button(&self, size: f32, mouse_state: MouseStateHandle) -> Button {
        self.animated_button(
            mouse_state,
            LEFT_CHEVRON_PATH,
            AnimatedButtonOptions {
                size,
                padding: Some(3.),
                color: None,
                with_accent_animations: false,
                circular: false,
            },
        )
    }

    pub fn plus_button(&self, size: f32, mouse_state: MouseStateHandle) -> Button {
        self.animated_button(
            mouse_state,
            PLUS_SVG_PATH,
            AnimatedButtonOptions {
                size,
                padding: Some(5.),
                color: None,
                with_accent_animations: true,
                circular: true,
            },
        )
    }

    pub fn enter_button(&self, size: f32, mouse_state: MouseStateHandle) -> Button {
        self.animated_button(
            mouse_state,
            ENTER_SVG_PATH,
            AnimatedButtonOptions {
                size,
                padding: Some(2.),
                color: None,
                with_accent_animations: false,
                circular: false,
            },
        )
    }

    pub fn copy_button(&self, size: f32, mouse_state: MouseStateHandle) -> Button {
        let color = self.warp_theme.main_text_color(self.warp_theme.surface_2());
        let icon = Icon::new(COPY_SVG_PATH, color);
        let base_styles = UiComponentStyles {
            height: Some(size),
            width: Some(size),
            ..Default::default()
        };

        let hovered_and_clicked_styles = UiComponentStyles {
            height: Some(size),
            width: Some(size),
            font_color: Some(color.with_opacity(80).into_solid()),
            ..Default::default()
        };

        Button::new(
            mouse_state,
            base_styles,
            Some(hovered_and_clicked_styles),
            Some(hovered_and_clicked_styles),
            None,
        )
        .with_icon_label(icon)
    }

    /// An info button with a tooltip that is rendered when the info button is hovered.
    pub fn info_button_with_tooltip(
        &self,
        info_button_size: f32,
        tooltip_contents: impl Into<String>,
        mouse_state_handle: MouseStateHandle,
    ) -> Hoverable {
        self.icon_with_tooltip(
            info_button_size,
            INFO_SVG_PATH,
            tooltip_contents,
            mouse_state_handle,
            None,
        )
    }

    pub fn help_icon_with_tooltip(
        &self,
        help_icon_size: f32,
        tooltip_contents: impl Into<String>,
        mouse_state_handle: MouseStateHandle,
        tooltip_styles: Option<UiComponentStyles>,
    ) -> Hoverable {
        self.icon_with_tooltip(
            help_icon_size,
            HELP_SVG_PATH,
            tooltip_contents,
            mouse_state_handle,
            tooltip_styles,
        )
    }

    pub fn local_only_icon_with_tooltip(
        &self,
        help_icon_size: f32,
        tooltip_contents: impl Into<String>,
        mouse_state_handle: MouseStateHandle,
    ) -> Hoverable {
        self.icon_with_tooltip(
            help_icon_size,
            LOCAL_ONLY_SVG_PATH,
            tooltip_contents,
            mouse_state_handle,
            None,
        )
    }

    fn icon_with_tooltip(
        &self,
        icon_size: f32,
        icon_path: &'static str,
        tooltip_contents: impl Into<String>,
        mouse_state_handle: MouseStateHandle,
        tooltip_styles: Option<UiComponentStyles>,
    ) -> Hoverable {
        let icon = Container::new(
            ConstrainedBox::new(
                Icon::new(icon_path, self.warp_theme.active_ui_text_color()).finish(),
            )
            .with_width(icon_size)
            .with_height(icon_size)
            .finish(),
        )
        .finish();

        Hoverable::new(mouse_state_handle, |state| {
            let mut stack = Stack::new().with_child(icon);
            if state.is_hovered() {
                let mut tool_tip = self.tool_tip(tooltip_contents.into());
                if let Some(tooltip_styles) = tooltip_styles {
                    tool_tip = tool_tip.with_style(tooltip_styles);
                }
                stack.add_positioned_child(
                    tool_tip.build().finish(),
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -3.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopMiddle,
                        ChildAnchor::BottomMiddle,
                    ),
                );
            }
            stack.finish()
        })
        .with_cursor(Cursor::PointingHand)
    }

    pub fn span_with_tooltip(
        &self,
        span: Span,
        tooltip_contents: impl Into<String>,
        mouse_state_handle: MouseStateHandle,
    ) -> Hoverable {
        Hoverable::new(mouse_state_handle, |state| {
            let mut stack = Stack::new().with_child(span.build().finish());
            if state.is_hovered() {
                let tool_tip = self.tool_tip(tooltip_contents.into());
                stack.add_positioned_child(
                    tool_tip.build().finish(),
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., 0.),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::BottomRight,
                        ChildAnchor::BottomLeft,
                    ),
                );
            }
            stack.finish()
        })
        .with_cursor(Cursor::PointingHand)
    }

    pub fn keyboard_shortcut(&self, keystroke: &Keystroke) -> KeyboardShortcut {
        KeyboardShortcut::new(keystroke, self.default_keyboard_shortcut_styles())
            .with_icon_for_keystroke(crate::ui::Icon::icon_for_key)
    }

    pub fn keyboard_shortcut_button(
        &self,
        text: String,
        keystroke: &Keystroke,
        mouse_state: MouseStateHandle,
    ) -> Button {
        let default_styles = UiComponentStyles {
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
            border_color: Some(self.warp_theme.surface_3().into()),
            border_width: Some(1.),
            padding: Some(Coords {
                top: 8.,
                bottom: 8.,
                left: 12.,
                right: 12.,
            }),
            ..Default::default()
        };
        let hovered_styles = UiComponentStyles {
            background: Some(self.warp_theme().surface_3().into()),
            border_color: Some(self.warp_theme().accent().into()),
            ..default_styles
        };

        let text = Text::new_inline(text, self.ui_font_family(), self.ui_font_size() + 2.)
            .with_color(text_main(&self.warp_theme, self.warp_theme.background()))
            .with_style(Properties {
                weight: Weight::Bold,
                ..Default::default()
            });

        let keystroke_styles = self
            .default_keyboard_shortcut_styles()
            .set_font_color(text_main(&self.warp_theme, self.warp_theme.background()));
        let keystrokes = KeyboardShortcut::new(keystroke, keystroke_styles);

        self.button_with_custom_styles(
            ButtonVariant::Text,
            mouse_state,
            default_styles,
            Some(hovered_styles),
            Some(hovered_styles),
            None,
        )
        .with_custom_label(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_children([
                    Container::new(text.finish())
                        .with_margin_right(12.)
                        .finish(),
                    keystrokes.build().finish(),
                ])
                .finish(),
        )
    }

    pub fn ui_font_size(&self) -> f32 {
        self.ui_font_size
    }

    pub fn ui_font_family(&self) -> FamilyId {
        self.ui_font_family
    }

    pub fn command_palette_font_size(&self) -> f32 {
        self.command_palette_font_size
    }

    pub fn warp_theme(&self) -> &WarpTheme {
        &self.warp_theme
    }

    pub fn line_height_ratio(&self) -> f32 {
        self.line_height_ratio
    }
}

/// Options for how to render an animated button.
#[derive(Default, Clone)]
pub struct AnimatedButtonOptions {
    pub size: f32,
    pub padding: Option<f32>,
    pub color: Option<Fill>,
    pub with_accent_animations: bool,
    pub circular: bool,
}
