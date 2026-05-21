use super::OnboardingSlide;
use crate::model::{OnboardingStateEvent, OnboardingStateModel};
use crate::slides::{bottom_nav, layout, slide_content};
use crate::telemetry::OnboardingEvent;
use crate::visuals::theme_picker_visual;
use crate::OnboardingIntention;
use pathfinder_color::ColorU;
use ui_components::{button, Component as _, Options as _};
use warp_core::features::FeatureFlag;
use warp_core::send_telemetry_from_ctx;
use warp_core::ui::{appearance::Appearance, theme::color::internal_colors, theme::WarpTheme};
use warpui::{
    elements::{
        Border, ClippedScrollStateHandle, ConstrainedBox, Container, CornerRadius,
        CrossAxisAlignment, Empty, Flex, FormattedTextElement, Hoverable, MainAxisAlignment,
        MainAxisSize, MouseStateHandle, ParentElement, Radius, Text,
    },
    fonts::{Properties, Weight},
    keymap::Keystroke,
    platform::Cursor,
    text_layout::TextAlignment,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
};

#[derive(Debug, Clone)]
pub enum ThemePickerSlideEvent {
    ThemeSelected {
        theme_name: String,
    },
    SyncWithOsToggled {
        enabled: bool,
    },
    /// Emitted when the user clicks the "Privacy Settings" link on the terminal
    /// intention theme slide. The parent orchestrator is expected to open the
    /// privacy settings (e.g. via a LoginSlideView in privacy-only mode).
    PrivacySettingsRequested,
}

#[derive(Debug, Clone)]
pub enum ThemePickerSlideAction {
    SelectTheme {
        index: usize,
    },
    ToggleSyncWithOs,
    BackClicked,
    NextClicked,
    /// Dispatched when the user clicks the "Privacy Settings" link in the
    /// terminal-intention disclaimer block below the theme options.
    PrivacySettingsClicked,
}

const TOS_URL: &str = "https://www.warp.dev/terms-of-service";

#[derive(Debug, Clone)]
struct ThemeOption {
    theme: WarpTheme,
    mouse_state: MouseStateHandle,
}

pub struct ThemePickerSlide {
    onboarding_state: ModelHandle<OnboardingStateModel>,
    theme_options: [ThemeOption; 4],
    selected_theme_index: usize,
    sync_with_os: bool,
    sync_with_os_mouse: MouseStateHandle,
    tos_mouse_state: MouseStateHandle,
    privacy_settings_mouse_state: MouseStateHandle,
    back_button: button::Button,
    next_button: button::Button,
    scroll_state: ClippedScrollStateHandle,
}

impl ThemePickerSlide {
    pub(crate) fn new(
        themes: [WarpTheme; 4],
        onboarding_state: ModelHandle<OnboardingStateModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let theme_options = themes.map(|theme| ThemeOption {
            theme,
            mouse_state: MouseStateHandle::default(),
        });

        ctx.subscribe_to_model(&onboarding_state, |_me, _model, event, ctx| {
            if matches!(event, OnboardingStateEvent::IntentionChanged) {
                ctx.notify();
            }
        });

        let appearance = Appearance::as_ref(ctx);
        let current_theme_name = appearance.theme().name();

        let selected_theme_index = current_theme_name
            .as_ref()
            .and_then(|name| {
                theme_options
                    .iter()
                    .position(|option| option.theme.name().as_ref() == Some(name))
            })
            .unwrap_or_else(|| {
                // If the current appearance theme isn't one of the provided choices, reset it to
                // the first option so our selection and the rendered theme match.
                Appearance::handle(ctx).update(ctx, |appearance, ctx| {
                    appearance.set_theme(theme_options[0].theme.clone(), ctx);
                });
                0
            });

        Self {
            onboarding_state,
            theme_options,
            selected_theme_index,
            sync_with_os: false,
            sync_with_os_mouse: MouseStateHandle::default(),
            tos_mouse_state: MouseStateHandle::default(),
            privacy_settings_mouse_state: MouseStateHandle::default(),
            back_button: button::Button::default(),
            next_button: button::Button::default(),
            scroll_state: ClippedScrollStateHandle::new(),
        }
    }

    fn theme_display_name(&self, index: usize) -> String {
        self.theme_options
            .get(index)
            .and_then(|option| option.theme.name())
            .unwrap_or_else(|| format!("Theme {}", index + 1))
    }

    fn render_theme_picker_content(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        // The option "chrome" (background, borders, text) should be styled using the currently
        // selected theme, if sync_with_os is not selected.
        let selected_theme = if self.sync_with_os {
            appearance.theme().clone()
        } else {
            self.theme_options
                .get(self.selected_theme_index)
                .map(|option| option.theme.clone())
                .unwrap_or_else(|| self.theme_options[0].theme.clone())
        };

        let bottom_nav = self.render_bottom_nav(appearance, app);

        let theme_options = self.render_theme_options(appearance, &selected_theme);

        // Apply a semi-transparent overlay to visually disable the theme options
        // when the "Sync with OS" checkbox is checked.
        let theme_options_section: Box<dyn Element> = if self.sync_with_os {
            let bg = appearance.theme().background().into_solid();
            let overlay_color = ColorU::new(bg.r, bg.g, bg.b, 128);
            Container::new(theme_options)
                .with_foreground_overlay(overlay_color)
                .finish()
        } else {
            theme_options
        };

        let mut content = vec![self.render_header_text(appearance), theme_options_section];

        if FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
            content.push(self.render_sync_with_os_section(appearance));
        }

        // Add the Privacy Settings / Terms of Service disclaimer block below the
        // theme options when the user has selected the terminal intention and
        // won't hit the login slide afterwards. The terminal-intent flow skips
        // the login slide (which surfaces the same links) unless Warp Drive is
        // enabled — in that case the login slide will still run after the theme
        // step and show the disclaimer, so duplicating it here is unnecessary.
        let state = self.onboarding_state.as_ref(app);
        let is_terminal = matches!(state.intention(), OnboardingIntention::Terminal);
        let warp_drive_enabled = state.ui_customization().show_warp_drive;
        if is_terminal && !warp_drive_enabled && FeatureFlag::OpenWarpNewSettingsModes.is_enabled()
        {
            content.push(self.render_disclaimer_section(appearance));
        }

        slide_content::onboarding_slide_content(
            content,
            bottom_nav,
            self.scroll_state.clone(),
            appearance,
        )
    }

    fn render_header_text(&self, appearance: &Appearance) -> Box<dyn Element> {
        let title = appearance
            .ui_builder()
            .paragraph("Choose a theme")
            .with_style(UiComponentStyles {
                font_size: Some(36.),
                font_weight: Some(Weight::Medium),
                ..Default::default()
            })
            .build()
            .finish();

        let subtitle = FormattedTextElement::from_str(
            "Click or use arrow keys to select, Enter to confirm.",
            appearance.ui_font_family(),
            16.,
        )
        .with_color(internal_colors::text_sub(
            appearance.theme(),
            appearance.theme().background().into_solid(),
        ))
        .with_weight(Weight::Normal)
        .with_alignment(TextAlignment::Left)
        .with_line_height_ratio(1.0)
        .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(title)
            .with_child(Container::new(subtitle).with_margin_top(16.).finish())
            .finish()
    }

    fn render_theme_options(
        &self,
        appearance: &Appearance,
        chrome_theme: &WarpTheme,
    ) -> Box<dyn Element> {
        let options = (0..self.theme_options.len())
            .map(|index| {
                let theme_name = self.theme_display_name(index);
                let option = &self.theme_options[index];

                self.render_theme_option(
                    appearance,
                    chrome_theme,
                    index,
                    theme_name,
                    &option.theme,
                    option.mouse_state.clone(),
                    !self.sync_with_os,
                )
            })
            .collect::<Vec<_>>();

        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_children(options)
                .finish(),
        )
        .with_margin_top(40.)
        .finish()
    }

    fn render_bottom_nav(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let back_button = self.back_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Back".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(ThemePickerSlideAction::BackClicked);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let theme_picker_last = FeatureFlag::OpenWarpNewSettingsModes.is_enabled();
        let next_label = if theme_picker_last {
            "Get Warping"
        } else {
            "Next"
        };

        let enter = Keystroke::parse("enter").unwrap_or_default();
        let next_button = self.next_button.render(
            appearance,
            button::Params {
                content: button::Content::Label(next_label.into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    keystroke: Some(enter),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(ThemePickerSlideAction::NextClicked);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let (step_index, step_count) = if theme_picker_last {
            let is_terminal = matches!(
                self.onboarding_state.as_ref(app).intention(),
                OnboardingIntention::Terminal
            );
            if is_terminal {
                (3, 4)
            } else {
                (4, 5)
            }
        } else {
            (0, 4)
        };

        bottom_nav::onboarding_bottom_nav(
            appearance,
            step_index,
            step_count,
            Some(back_button),
            Some(next_button),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_theme_option(
        &self,
        appearance: &Appearance,
        chrome_theme: &WarpTheme,
        index: usize,
        theme_name: String,
        option_theme: &WarpTheme,
        mouse_state: MouseStateHandle,
        interactive: bool,
    ) -> Box<dyn Element> {
        const SWATCH_RADIUS_PX: f32 = 11.0;
        const SWATCH_DIAMETER_PX: f32 = SWATCH_RADIUS_PX * 2.0;

        let theme_name_owned = theme_name;
        let is_selected = !self.sync_with_os && self.selected_theme_index == index;

        let background = if is_selected {
            internal_colors::accent_bg(chrome_theme)
        } else {
            chrome_theme.surface_2()
        };

        let border_color = if is_selected {
            chrome_theme.accent()
        } else {
            chrome_theme.surface_overlay_1()
        };

        // Choose text color based on the actual background fill.
        let text_color = chrome_theme.main_text_color(background);

        // Only the swatches use the *option* theme.
        let swatch_colors = [
            option_theme.ansi_fg_red(),
            option_theme.ansi_fg_green(),
            option_theme.ansi_fg_blue(),
            option_theme.ansi_fg_yellow(),
        ];

        let selected_index_for_action = index;

        let button = Hoverable::new(mouse_state, move |_| {
            let swatches = Flex::row()
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_children(
                    swatch_colors
                        .iter()
                        .enumerate()
                        .map(|(i, color)| {
                            Container::new(
                                ConstrainedBox::new(
                                    Container::new(Empty::new().finish())
                                        .with_background_color(*color)
                                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                            SWATCH_RADIUS_PX,
                                        )))
                                        .finish(),
                                )
                                .with_width(SWATCH_DIAMETER_PX)
                                .with_height(SWATCH_DIAMETER_PX)
                                .finish(),
                            )
                            .with_margin_left(if i > 0 { 4. } else { 0. })
                            .finish()
                        })
                        .collect::<Vec<_>>(),
                )
                .finish();

            ConstrainedBox::new(
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(
                            appearance
                                .ui_builder()
                                .paragraph(theme_name_owned.clone())
                                .with_style(UiComponentStyles {
                                    font_size: Some(14.),
                                    font_weight: Some(Weight::Medium),
                                    font_color: Some(text_color.into()),
                                    ..Default::default()
                                })
                                .build()
                                .finish(),
                        )
                        .with_child(swatches)
                        .finish(),
                )
                .with_vertical_padding(16.)
                .with_horizontal_padding(24.)
                .with_background(background)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_border(Border::all(1.).with_border_fill(border_color))
                .finish(),
            )
            .with_height(56.)
            .finish()
        });

        let button = if interactive {
            button
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(ThemePickerSlideAction::SelectTheme {
                        index: selected_index_for_action,
                    });
                })
                .finish()
        } else {
            button.finish()
        };

        Container::new(button).with_margin_bottom(12.).finish()
    }

    /// All onboarding image paths used by the theme picker slide visual.
    pub(crate) const VISUAL_IMAGE_PATHS: &'static [&'static str] = &[
        // Terminal intention
        "async/png/onboarding/terminal_intention/theme/theme_phenomenon_vertical.png",
        "async/png/onboarding/terminal_intention/theme/theme_phenomenon_horizontal.png",
        "async/png/onboarding/terminal_intention/theme/theme_dark_vertical.png",
        "async/png/onboarding/terminal_intention/theme/theme_dark_horizontal.png",
        "async/png/onboarding/terminal_intention/theme/theme_light_vertical.png",
        "async/png/onboarding/terminal_intention/theme/theme_light_horizontal.png",
        "async/png/onboarding/terminal_intention/theme/theme_adeberry_vertical.png",
        "async/png/onboarding/terminal_intention/theme/theme_adeberry_horizontal.png",
        // Agent intention
        "async/png/onboarding/agent_intention/theme/theme_phenomenon_vertical.png",
        "async/png/onboarding/agent_intention/theme/theme_phenomenon_horizontal.png",
        "async/png/onboarding/agent_intention/theme/theme_dark_vertical.png",
        "async/png/onboarding/agent_intention/theme/theme_dark_horizontal.png",
        "async/png/onboarding/agent_intention/theme/theme_light_vertical.png",
        "async/png/onboarding/agent_intention/theme/theme_light_horizontal.png",
        "async/png/onboarding/agent_intention/theme/theme_adeberry_vertical.png",
        "async/png/onboarding/agent_intention/theme/theme_adeberry_horizontal.png",
    ];

    fn theme_visual_path(&self, app: &AppContext) -> &'static str {
        let state = self.onboarding_state.as_ref(app);
        let vertical = state.ui_customization().use_vertical_tabs;
        let intention_dir = match state.intention() {
            OnboardingIntention::AgentDrivenDevelopment => "agent_intention",
            OnboardingIntention::Terminal => "terminal_intention",
        };
        let theme_name = self.theme_display_name(self.selected_theme_index);
        let name_key = match theme_name.as_str() {
            "Phenomenon" => "phenomenon",
            "Dark" => "dark",
            "Light" => "light",
            "Adeberry" => "adeberry",
            _ => "dark",
        };
        let orientation = if vertical { "vertical" } else { "horizontal" };
        // Safety: all combinations are in VISUAL_IMAGE_PATHS.
        Self::VISUAL_IMAGE_PATHS
            .iter()
            .find(|p| p.contains(intention_dir) && p.contains(name_key) && p.contains(orientation))
            .unwrap_or(&Self::VISUAL_IMAGE_PATHS[0])
    }

    fn render_theme_picker_visual(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        if FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
            let path = self.theme_visual_path(app);
            layout::onboarding_right_panel_with_bg(path, layout::FOREGROUND_LAYOUT_DEFAULT)
        } else {
            theme_picker_visual(appearance)
        }
    }
}

impl Entity for ThemePickerSlide {
    type Event = ThemePickerSlideEvent;
}

impl View for ThemePickerSlide {
    fn ui_name() -> &'static str {
        "ThemePickerSlide"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        // Background is rendered by the parent onboarding view (including background images).
        layout::static_left(
            || self.render_theme_picker_content(appearance, app),
            || self.render_theme_picker_visual(appearance, app),
        )
    }
}

impl ThemePickerSlide {
    fn render_sync_with_os_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background_for_text = theme.background().into_solid();

        let checkbox = appearance
            .ui_builder()
            .checkbox(self.sync_with_os_mouse.clone(), Some(12.))
            .check(self.sync_with_os)
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(ThemePickerSlideAction::ToggleSyncWithOs)
            })
            .finish();

        let label = Text::new(
            "Sync light/dark theme with OS",
            appearance.ui_font_family(),
            14.0,
        )
        .with_color(internal_colors::text_sub(theme, background_for_text))
        .with_style(Properties {
            weight: Weight::Normal,
            ..Default::default()
        })
        .with_line_height_ratio(1.0)
        .finish();

        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(checkbox)
                .with_child(Container::new(label).with_margin_left(8.).finish())
                .finish(),
        )
        .with_margin_top(24.)
        .finish()
    }

    fn render_disclaimer_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let sub_text_color = internal_colors::text_sub(theme, theme.background().into_solid());
        let ui_builder = appearance.ui_builder();

        let disclaimer_styles = UiComponentStyles {
            font_color: Some(sub_text_color),
            font_size: Some(12.),
            ..Default::default()
        };
        let link_styles = UiComponentStyles {
            font_size: Some(12.),
            ..Default::default()
        };

        // The disclaimer block is only rendered on the Terminal-without-Drive
        // path (see `render_theme_picker_content`), where AI is not part of the
        // selected onboarding settings; skip the "and AI features" wording.
        let privacy_line = Flex::row()
            .with_child(
                ui_builder
                    .span("If you'd like to opt out of analytics, you can adjust your ")
                    .with_style(disclaimer_styles)
                    .build()
                    .finish(),
            )
            .with_child(
                ui_builder
                    .link(
                        "Privacy Settings".into(),
                        None,
                        Some(Box::new(|ctx| {
                            ctx.dispatch_typed_action(
                                ThemePickerSlideAction::PrivacySettingsClicked,
                            );
                        })),
                        self.privacy_settings_mouse_state.clone(),
                    )
                    .soft_wrap(false)
                    .with_style(link_styles)
                    .build()
                    .finish(),
            )
            .finish();

        let tos_line = Flex::row()
            .with_child(
                ui_builder
                    .span("By continuing, you agree to Warp's ")
                    .with_style(disclaimer_styles)
                    .build()
                    .finish(),
            )
            .with_child(
                ui_builder
                    .link(
                        "Terms of Service".into(),
                        Some(TOS_URL.into()),
                        None,
                        self.tos_mouse_state.clone(),
                    )
                    .soft_wrap(false)
                    .with_style(link_styles)
                    .build()
                    .finish(),
            )
            .finish();

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(privacy_line)
                .with_child(Container::new(tos_line).with_margin_top(8.).finish())
                .finish(),
        )
        .with_margin_top(24.)
        .finish()
    }

    fn select_theme(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        self.sync_with_os = false;
        self.selected_theme_index = index;
        let theme_name = self.theme_display_name(index);
        send_telemetry_from_ctx!(
            OnboardingEvent::SettingChanged {
                setting: "theme".to_string(),
                value: theme_name.clone(),
            },
            ctx
        );
        ctx.emit(ThemePickerSlideEvent::ThemeSelected { theme_name });
        ctx.notify();
    }

    fn next(&mut self, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |model, ctx| {
            if FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
                model.complete(ctx);
            } else {
                model.next(ctx);
            }
        });
    }
}

impl OnboardingSlide for ThemePickerSlide {
    fn on_up(&mut self, ctx: &mut ViewContext<Self>) {
        if self.sync_with_os {
            return;
        }
        let selected_theme_index = self.selected_theme_index;
        let theme_options_len = self.theme_options.len();

        let up_index = if selected_theme_index == 0 {
            theme_options_len.saturating_sub(1)
        } else {
            selected_theme_index - 1
        };

        self.select_theme(up_index, ctx);
    }

    fn on_down(&mut self, ctx: &mut ViewContext<Self>) {
        if self.sync_with_os {
            return;
        }
        let selected_theme_index = self.selected_theme_index;
        let theme_options_len = self.theme_options.len();

        let down_index = if selected_theme_index + 1 >= theme_options_len {
            0
        } else {
            selected_theme_index + 1
        };

        self.select_theme(down_index, ctx);
    }

    fn on_enter(&mut self, ctx: &mut ViewContext<Self>) {
        self.next(ctx);
    }
}

impl TypedActionView for ThemePickerSlide {
    type Action = ThemePickerSlideAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ThemePickerSlideAction::SelectTheme { index } => {
                if !self.sync_with_os {
                    self.select_theme(*index, ctx);
                }
            }
            ThemePickerSlideAction::ToggleSyncWithOs => {
                self.sync_with_os = !self.sync_with_os;
                send_telemetry_from_ctx!(
                    OnboardingEvent::SettingChanged {
                        setting: "sync_with_os".to_string(),
                        value: self.sync_with_os.to_string(),
                    },
                    ctx
                );
                ctx.emit(ThemePickerSlideEvent::SyncWithOsToggled {
                    enabled: self.sync_with_os,
                });
                ctx.notify();
            }
            ThemePickerSlideAction::BackClicked => {
                let onboarding_state = self.onboarding_state.clone();
                onboarding_state.update(ctx, |model, ctx| {
                    model.back(ctx);
                });
            }
            ThemePickerSlideAction::NextClicked => {
                self.next(ctx);
            }
            ThemePickerSlideAction::PrivacySettingsClicked => {
                ctx.emit(ThemePickerSlideEvent::PrivacySettingsRequested);
            }
        }
    }
}
