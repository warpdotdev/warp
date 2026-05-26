use ui_components::{button, Component as _, Options as _};
use warp_core::features::FeatureFlag;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    ClippedScrollStateHandle, Container, CrossAxisAlignment, Flex, FormattedTextElement,
    MainAxisSize, MouseStateHandle, ParentElement,
};
use warpui::fonts::Weight;
use warpui::keymap::Keystroke;
use warpui::prelude::Align;
use warpui::text_layout::TextAlignment;
use warpui::ui_components::components::{UiComponent as _, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity as _, TypedActionView, View,
    ViewContext,
};

use super::toggle_card::{render_toggle_card, ChipSpec, ToggleCardSpec};
use super::OnboardingSlide;
use crate::model::{OnboardingStateEvent, OnboardingStateModel, UICustomizationSettings};
use crate::slides::{bottom_nav, layout, slide_content};
use crate::visuals::{intention_terminal_visual, intention_visual};
use crate::OnboardingIntention;

/// Which setting card is currently selected (expanded).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingCard {
    TabStyling,
    ToolsPanel,
    CodeReview,
}

/// Sub-settings within the tools panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolsPanelSubSetting {
    ConversationHistory,
    ProjectExplorer,
    GlobalSearch,
    WarpDrive,
}

#[derive(Debug, Clone)]
pub enum CustomizeSlideAction {
    SelectSettingCard { card_index: usize },
    SetTabStylingVertical { vertical: bool },
    SetToolsPanelEnabled { enabled: bool },
    ToggleToolsSubSetting { setting: ToolsPanelSubSetting },
    HoverToolsChip { setting: ToolsPanelSubSetting },
    SetCodeReviewEnabled { enabled: bool },
    BackClicked,
    NextClicked,
}

pub struct CustomizeUISlide {
    onboarding_state: ModelHandle<OnboardingStateModel>,
    selected_setting: Option<SettingCard>,
    /// The last-hovered tools panel chip; persists until a different chip is hovered
    /// or a different card is selected.
    hovered_chip: Option<ToolsPanelSubSetting>,
    // Mouse states for setting cards
    tab_styling_mouse_state: MouseStateHandle,
    tools_panel_mouse_state: MouseStateHandle,
    code_review_mouse_state: MouseStateHandle,
    // Mouse states for segmented control options (2 per card)
    tab_seg_left_mouse: MouseStateHandle,
    tab_seg_right_mouse: MouseStateHandle,
    tools_seg_left_mouse: MouseStateHandle,
    tools_seg_right_mouse: MouseStateHandle,
    code_seg_left_mouse: MouseStateHandle,
    code_seg_right_mouse: MouseStateHandle,
    // Mouse states for tools panel chip buttons
    chip_conversation_mouse: MouseStateHandle,
    chip_file_explorer_mouse: MouseStateHandle,
    chip_global_search_mouse: MouseStateHandle,
    chip_warp_drive_mouse: MouseStateHandle,
    // Buttons
    back_button: button::Button,
    next_button: button::Button,
    scroll_state: ClippedScrollStateHandle,
}

impl CustomizeUISlide {
    pub(crate) fn new(
        onboarding_state: ModelHandle<OnboardingStateModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&onboarding_state, |me, _model, event, ctx| {
            if matches!(event, OnboardingStateEvent::IntentionChanged) {
                me.selected_setting = None;
                me.hovered_chip = None;
                ctx.notify();
            }
        });

        Self {
            onboarding_state,
            selected_setting: None,
            hovered_chip: None,
            tab_styling_mouse_state: MouseStateHandle::default(),
            tools_panel_mouse_state: MouseStateHandle::default(),
            code_review_mouse_state: MouseStateHandle::default(),
            tab_seg_left_mouse: MouseStateHandle::default(),
            tab_seg_right_mouse: MouseStateHandle::default(),
            tools_seg_left_mouse: MouseStateHandle::default(),
            tools_seg_right_mouse: MouseStateHandle::default(),
            code_seg_left_mouse: MouseStateHandle::default(),
            code_seg_right_mouse: MouseStateHandle::default(),
            chip_conversation_mouse: MouseStateHandle::default(),
            chip_file_explorer_mouse: MouseStateHandle::default(),
            chip_global_search_mouse: MouseStateHandle::default(),
            chip_warp_drive_mouse: MouseStateHandle::default(),
            back_button: button::Button::default(),
            next_button: button::Button::default(),
            scroll_state: ClippedScrollStateHandle::new(),
        }
    }

    fn model_intention(&self, app: &AppContext) -> OnboardingIntention {
        *self.onboarding_state.as_ref(app).intention()
    }

    fn model_ui_customization(&self, app: &AppContext) -> UICustomizationSettings {
        self.onboarding_state.as_ref(app).ui_customization().clone()
    }

    fn render_content(
        &self,
        appearance: &Appearance,
        intention: OnboardingIntention,
        ui: &UICustomizationSettings,
    ) -> Box<dyn Element> {
        let bottom_nav = Align::new(self.render_bottom_nav(appearance, intention)).finish();

        slide_content::onboarding_slide_content(
            vec![
                Align::new(self.render_header(appearance)).left().finish(),
                self.render_setting_cards(appearance, intention, ui),
            ],
            bottom_nav,
            self.scroll_state.clone(),
            appearance,
        )
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let title = appearance
            .ui_builder()
            .paragraph("Customize your Warp")
            .with_style(UiComponentStyles {
                font_size: Some(36.),
                font_weight: Some(Weight::Medium),
                ..Default::default()
            })
            .build()
            .finish();

        let subtitle = FormattedTextElement::from_str(
            "Tailor your features and UI to your working style.",
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
            .with_child(
                Container::new(subtitle)
                    .with_margin_top(16.)
                    .with_margin_bottom(40.)
                    .finish(),
            )
            .finish()
    }

    // --- Setting cards ---

    fn render_setting_cards(
        &self,
        appearance: &Appearance,
        intention: OnboardingIntention,
        ui: &UICustomizationSettings,
    ) -> Box<dyn Element> {
        let tab_card = self.render_tab_styling_card(appearance, ui);
        let tools_card = self.render_tools_panel_card(appearance, intention, ui);
        let code_card = self.render_code_review_card(appearance, ui);

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(12.)
                .with_child(tab_card)
                .with_child(tools_card)
                .with_child(code_card)
                .finish(),
        )
        .with_margin_top(12.)
        .finish()
    }

    fn render_tab_styling_card(
        &self,
        appearance: &Appearance,
        ui: &UICustomizationSettings,
    ) -> Box<dyn Element> {
        let is_selected = self.selected_setting == Some(SettingCard::TabStyling);

        render_toggle_card(
            appearance,
            ToggleCardSpec {
                title: "Tab styling",
                is_expanded: is_selected,
                is_left_selected: ui.use_vertical_tabs,
                left_label: "Vertical",
                right_label: "Horizontal",
                card_mouse_state: self.tab_styling_mouse_state.clone(),
                on_expand: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(CustomizeSlideAction::SelectSettingCard {
                        card_index: 0,
                    });
                }),
                left_mouse: self.tab_seg_left_mouse.clone(),
                right_mouse: self.tab_seg_right_mouse.clone(),
                on_left: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(CustomizeSlideAction::SetTabStylingVertical {
                        vertical: true,
                    });
                }),
                on_right: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(CustomizeSlideAction::SetTabStylingVertical {
                        vertical: false,
                    });
                }),
                chips: vec![],
            },
        )
    }
    fn render_tools_panel_card(
        &self,
        appearance: &Appearance,
        intention: OnboardingIntention,
        ui: &UICustomizationSettings,
    ) -> Box<dyn Element> {
        let is_selected = self.selected_setting == Some(SettingCard::ToolsPanel);
        let is_agent = matches!(intention, OnboardingIntention::AgentDrivenDevelopment);

        let mut chips = vec![];

        if ui.tools_panel_enabled(&intention) {
            // Conversation history chip is only shown for the agent intention.
            if is_agent {
                chips.push(ChipSpec {
                    label: "Conversation history",
                    is_enabled: ui.show_conversation_history,
                    mouse_state: self.chip_conversation_mouse.clone(),
                    on_click: Box::new(|ctx, _, _| {
                        ctx.dispatch_typed_action(CustomizeSlideAction::ToggleToolsSubSetting {
                            setting: ToolsPanelSubSetting::ConversationHistory,
                        });
                    }),
                    on_hover: Some(Box::new(|is_hovered, ctx, _, _| {
                        if is_hovered {
                            ctx.dispatch_typed_action(CustomizeSlideAction::HoverToolsChip {
                                setting: ToolsPanelSubSetting::ConversationHistory,
                            });
                        }
                    })),
                });
            }

            chips.push(ChipSpec {
                label: "File explorer",
                is_enabled: ui.show_project_explorer,
                mouse_state: self.chip_file_explorer_mouse.clone(),
                on_click: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(CustomizeSlideAction::ToggleToolsSubSetting {
                        setting: ToolsPanelSubSetting::ProjectExplorer,
                    });
                }),
                on_hover: Some(Box::new(|is_hovered, ctx, _, _| {
                    if is_hovered {
                        ctx.dispatch_typed_action(CustomizeSlideAction::HoverToolsChip {
                            setting: ToolsPanelSubSetting::ProjectExplorer,
                        });
                    }
                })),
            });

            chips.push(ChipSpec {
                label: "Global file search",
                is_enabled: ui.show_global_search,
                mouse_state: self.chip_global_search_mouse.clone(),
                on_click: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(CustomizeSlideAction::ToggleToolsSubSetting {
                        setting: ToolsPanelSubSetting::GlobalSearch,
                    });
                }),
                on_hover: Some(Box::new(|is_hovered, ctx, _, _| {
                    if is_hovered {
                        ctx.dispatch_typed_action(CustomizeSlideAction::HoverToolsChip {
                            setting: ToolsPanelSubSetting::GlobalSearch,
                        });
                    }
                })),
            });

            chips.push(ChipSpec {
                label: "Warp Drive",
                is_enabled: ui.show_warp_drive,
                mouse_state: self.chip_warp_drive_mouse.clone(),
                on_click: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(CustomizeSlideAction::ToggleToolsSubSetting {
                        setting: ToolsPanelSubSetting::WarpDrive,
                    });
                }),
                on_hover: Some(Box::new(|is_hovered, ctx, _, _| {
                    if is_hovered {
                        ctx.dispatch_typed_action(CustomizeSlideAction::HoverToolsChip {
                            setting: ToolsPanelSubSetting::WarpDrive,
                        });
                    }
                })),
            });
        }

        render_toggle_card(
            appearance,
            ToggleCardSpec {
                title: "Tools panel",
                is_expanded: is_selected,
                is_left_selected: ui.tools_panel_enabled(&intention),
                left_label: "Enabled",
                right_label: "Disabled",
                card_mouse_state: self.tools_panel_mouse_state.clone(),
                on_expand: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(CustomizeSlideAction::SelectSettingCard {
                        card_index: 1,
                    });
                }),
                left_mouse: self.tools_seg_left_mouse.clone(),
                right_mouse: self.tools_seg_right_mouse.clone(),
                on_left: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(CustomizeSlideAction::SetToolsPanelEnabled {
                        enabled: true,
                    });
                }),
                on_right: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(CustomizeSlideAction::SetToolsPanelEnabled {
                        enabled: false,
                    });
                }),
                chips,
            },
        )
    }

    fn render_code_review_card(
        &self,
        appearance: &Appearance,
        ui: &UICustomizationSettings,
    ) -> Box<dyn Element> {
        let is_selected = self.selected_setting == Some(SettingCard::CodeReview);

        render_toggle_card(
            appearance,
            ToggleCardSpec {
                title: "Code review",
                is_expanded: is_selected,
                is_left_selected: ui.show_code_review_button,
                left_label: "Enabled",
                right_label: "Disabled",
                card_mouse_state: self.code_review_mouse_state.clone(),
                on_expand: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(CustomizeSlideAction::SelectSettingCard {
                        card_index: 2,
                    });
                }),
                left_mouse: self.code_seg_left_mouse.clone(),
                right_mouse: self.code_seg_right_mouse.clone(),
                on_left: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(CustomizeSlideAction::SetCodeReviewEnabled {
                        enabled: true,
                    });
                }),
                on_right: Box::new(|ctx, _, _| {
                    ctx.dispatch_typed_action(CustomizeSlideAction::SetCodeReviewEnabled {
                        enabled: false,
                    });
                }),
                chips: vec![],
            },
        )
    }

    // --- Bottom nav ---

    fn render_bottom_nav(
        &self,
        appearance: &Appearance,
        intention: OnboardingIntention,
    ) -> Box<dyn Element> {
        let back_button = self.back_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Back".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(CustomizeSlideAction::BackClicked);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let enter = Keystroke::parse("enter").unwrap_or_default();
        let next_button = self.next_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Next".into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    keystroke: Some(enter),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(CustomizeSlideAction::NextClicked);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let is_terminal = matches!(intention, OnboardingIntention::Terminal);
        let (step_index, step_count) = if is_terminal { (1, 4) } else { (1, 5) };
        bottom_nav::onboarding_bottom_nav(
            appearance,
            step_index,
            step_count,
            Some(back_button),
            Some(next_button),
        )
    }

    // --- Visual (right column) ---

    /// All bundled image paths used by the customize slide visual.
    /// Used for preloading into the asset cache.
    pub(crate) const VISUAL_IMAGE_PATHS: &'static [&'static str] = &[
        // Welcome / default
        "async/png/onboarding/welcome_agent.png",
        "async/png/onboarding/welcome_terminal.png",
        // Agent intention
        "async/png/onboarding/agent_intention/customize_vertical_tabs.png",
        "async/png/onboarding/agent_intention/customize_horizontal_tabs.png",
        "async/png/onboarding/agent_intention/customize_tools_disabled_vertical.png",
        "async/png/onboarding/agent_intention/customize_tools_disabled_horizontal.png",
        "async/png/onboarding/agent_intention/customize_conversation_vertical.png",
        "async/png/onboarding/agent_intention/customize_conversation_horizontal.png",
        "async/png/onboarding/agent_intention/customize_fileexplorer_vertical.png",
        "async/png/onboarding/agent_intention/customize_fileexplorer_horizontal.png",
        "async/png/onboarding/agent_intention/customize_filesearch_vertical.png",
        "async/png/onboarding/agent_intention/customize_filesearch_horizontal.png",
        "async/png/onboarding/agent_intention/customize_warpdrive_vertical.png",
        "async/png/onboarding/agent_intention/customize_warpdrive_horizontal.png",
        "async/png/onboarding/agent_intention/customize_codereview_enabled_vertical.png",
        "async/png/onboarding/agent_intention/customize_codereview_enabled_horizontal.png",
        "async/png/onboarding/agent_intention/customize_codereview_disabled_vertical.png",
        "async/png/onboarding/agent_intention/customize_codereview_disabled_horizontal.png",
        // Terminal intention
        "async/png/onboarding/terminal_intention/terminal_customize_vertical_tabs.png",
        "async/png/onboarding/terminal_intention/terminal_customize_horizontal_tabs.png",
        "async/png/onboarding/terminal_intention/terminal_customize_fileexplorer_vertical.png",
        "async/png/onboarding/terminal_intention/terminal_customize_fileexplorer_horizontal.png",
        "async/png/onboarding/terminal_intention/terminal_customize_filesearch_vertical.png",
        "async/png/onboarding/terminal_intention/terminal_customize_filesearch_horizontal.png",
        "async/png/onboarding/terminal_intention/terminal_customize_warpdrive_vertical.png",
        "async/png/onboarding/terminal_intention/terminal_customize_warpdrive_horizontal.png",
        "async/png/onboarding/terminal_intention/terminal_codereview_enabled.png",
        "async/png/onboarding/terminal_intention/terminal_codereview_disabled.png",
    ];

    /// Returns the image path for the current visual state.
    /// When `OpenWarpNewSettingsModes` is enabled, assets depend on the tab layout setting.
    fn visual_image_path(
        selected_setting: Option<SettingCard>,
        hovered_chip: Option<ToolsPanelSubSetting>,
        intention: OnboardingIntention,
        ui: &UICustomizationSettings,
    ) -> &'static str {
        let is_agent = matches!(intention, OnboardingIntention::AgentDrivenDevelopment);
        let vertical = ui.use_vertical_tabs;
        match selected_setting {
            None => match intention {
                OnboardingIntention::AgentDrivenDevelopment => {
                    "async/png/onboarding/welcome_agent.png"
                }
                OnboardingIntention::Terminal => "async/png/onboarding/welcome_terminal.png",
            },
            Some(SettingCard::TabStyling) => {
                if is_agent {
                    if !ui.tools_panel_enabled(&intention) {
                        if vertical {
                            "async/png/onboarding/agent_intention/customize_tools_disabled_vertical.png"
                        } else {
                            "async/png/onboarding/agent_intention/customize_tools_disabled_horizontal.png"
                        }
                    } else if vertical {
                        "async/png/onboarding/agent_intention/customize_vertical_tabs.png"
                    } else {
                        "async/png/onboarding/agent_intention/customize_horizontal_tabs.png"
                    }
                } else if vertical {
                    "async/png/onboarding/terminal_intention/terminal_customize_vertical_tabs.png"
                } else {
                    "async/png/onboarding/terminal_intention/terminal_customize_horizontal_tabs.png"
                }
            }
            Some(SettingCard::ToolsPanel) => {
                if !ui.tools_panel_enabled(&intention) {
                    // Terminal: tools disabled uses the same image as tab layout.
                    if is_agent {
                        if vertical {
                            "async/png/onboarding/agent_intention/customize_tools_disabled_vertical.png"
                        } else {
                            "async/png/onboarding/agent_intention/customize_tools_disabled_horizontal.png"
                        }
                    } else if vertical {
                        "async/png/onboarding/terminal_intention/terminal_customize_vertical_tabs.png"
                    } else {
                        "async/png/onboarding/terminal_intention/terminal_customize_horizontal_tabs.png"
                    }
                } else {
                    // Default chip: conversation for agent, file explorer for terminal.
                    let default_chip = if is_agent {
                        ToolsPanelSubSetting::ConversationHistory
                    } else {
                        ToolsPanelSubSetting::ProjectExplorer
                    };
                    let chip = hovered_chip.unwrap_or(default_chip);
                    if is_agent {
                        match (chip, vertical) {
                            (ToolsPanelSubSetting::ConversationHistory, true) => "async/png/onboarding/agent_intention/customize_conversation_vertical.png",
                            (ToolsPanelSubSetting::ConversationHistory, false) => "async/png/onboarding/agent_intention/customize_conversation_horizontal.png",
                            (ToolsPanelSubSetting::ProjectExplorer, true) => "async/png/onboarding/agent_intention/customize_fileexplorer_vertical.png",
                            (ToolsPanelSubSetting::ProjectExplorer, false) => "async/png/onboarding/agent_intention/customize_fileexplorer_horizontal.png",
                            (ToolsPanelSubSetting::GlobalSearch, true) => "async/png/onboarding/agent_intention/customize_filesearch_vertical.png",
                            (ToolsPanelSubSetting::GlobalSearch, false) => "async/png/onboarding/agent_intention/customize_filesearch_horizontal.png",
                            (ToolsPanelSubSetting::WarpDrive, true) => "async/png/onboarding/agent_intention/customize_warpdrive_vertical.png",
                            (ToolsPanelSubSetting::WarpDrive, false) => "async/png/onboarding/agent_intention/customize_warpdrive_horizontal.png",
                        }
                    } else {
                        // Terminal: no conversation chip; ConversationHistory falls through to file explorer.
                        match (chip, vertical) {
                            (ToolsPanelSubSetting::ConversationHistory | ToolsPanelSubSetting::ProjectExplorer, true) => "async/png/onboarding/terminal_intention/terminal_customize_fileexplorer_vertical.png",
                            (ToolsPanelSubSetting::ConversationHistory | ToolsPanelSubSetting::ProjectExplorer, false) => "async/png/onboarding/terminal_intention/terminal_customize_fileexplorer_horizontal.png",
                            (ToolsPanelSubSetting::GlobalSearch, true) => "async/png/onboarding/terminal_intention/terminal_customize_filesearch_vertical.png",
                            (ToolsPanelSubSetting::GlobalSearch, false) => "async/png/onboarding/terminal_intention/terminal_customize_filesearch_horizontal.png",
                            (ToolsPanelSubSetting::WarpDrive, true) => "async/png/onboarding/terminal_intention/terminal_customize_warpdrive_vertical.png",
                            (ToolsPanelSubSetting::WarpDrive, false) => "async/png/onboarding/terminal_intention/terminal_customize_warpdrive_horizontal.png",
                        }
                    }
                }
            }
            Some(SettingCard::CodeReview) => {
                if is_agent {
                    match (ui.show_code_review_button, vertical) {
                        (true, true) => "async/png/onboarding/agent_intention/customize_codereview_enabled_vertical.png",
                        (true, false) => "async/png/onboarding/agent_intention/customize_codereview_enabled_horizontal.png",
                        (false, true) => "async/png/onboarding/agent_intention/customize_codereview_disabled_vertical.png",
                        (false, false) => "async/png/onboarding/agent_intention/customize_codereview_disabled_horizontal.png",
                    }
                } else if ui.show_code_review_button {
                    "async/png/onboarding/terminal_intention/terminal_codereview_enabled.png"
                } else {
                    "async/png/onboarding/terminal_intention/terminal_codereview_disabled.png"
                }
            }
        }
    }

    fn render_visual(
        &self,
        appearance: &Appearance,
        intention: OnboardingIntention,
        ui: &UICustomizationSettings,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        if FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
            let path =
                Self::visual_image_path(self.selected_setting, self.hovered_chip, intention, ui);
            let fg_layout = match self.selected_setting {
                None => layout::FOREGROUND_LAYOUT_DEFAULT,
                Some(SettingCard::CodeReview) => layout::FOREGROUND_LAYOUT_CODE_REVIEW,
                _ => layout::FOREGROUND_LAYOUT_WIDE,
            };
            layout::onboarding_right_panel_with_bg(path, fg_layout)
        } else {
            let panel_background = internal_colors::neutral_2(theme);
            let neutral = internal_colors::neutral_4(theme);

            let visual = if matches!(intention, OnboardingIntention::Terminal) {
                let neutral_highlight = internal_colors::neutral_6(theme);
                let accent = internal_colors::accent(theme);
                intention_terminal_visual(
                    panel_background,
                    neutral,
                    neutral_highlight,
                    accent.into_solid(),
                )
            } else {
                let blue = theme.ansi_fg_blue();
                let green = theme.ansi_fg_green();
                let yellow = theme.ansi_fg_yellow();
                intention_visual(panel_background, neutral, blue, green, yellow)
            };

            Container::new(visual)
                .with_background_color(internal_colors::neutral_1(theme))
                .finish()
        }
    }
}

impl Entity for CustomizeUISlide {
    type Event = ();
}

impl View for CustomizeUISlide {
    fn ui_name() -> &'static str {
        "CustomizeUISlide"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let intention = self.model_intention(app);
        let ui = self.model_ui_customization(app);

        layout::static_left(
            || self.render_content(appearance, intention, &ui),
            || self.render_visual(appearance, intention, &ui),
        )
    }
}

impl CustomizeUISlide {
    fn select_setting_card(&mut self, card_index: usize, ctx: &mut ViewContext<Self>) {
        let card = match card_index {
            0 => SettingCard::TabStyling,
            1 => SettingCard::ToolsPanel,
            2 => SettingCard::CodeReview,
            _ => return,
        };
        // Only select — don't toggle. Clicking a different card replaces the selection.
        self.selected_setting = Some(card);
        // Reset chip hover when switching cards.
        self.hovered_chip = None;
        ctx.notify();
    }

    fn next(&mut self, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |model, ctx| {
            model.next(ctx);
        });
    }
}

impl OnboardingSlide for CustomizeUISlide {
    fn on_up(&mut self, ctx: &mut ViewContext<Self>) {
        // Move setting selection up
        self.selected_setting = match self.selected_setting {
            Some(SettingCard::ToolsPanel) => Some(SettingCard::TabStyling),
            Some(SettingCard::CodeReview) => Some(SettingCard::ToolsPanel),
            _ => self.selected_setting,
        };
        ctx.notify();
    }

    fn on_down(&mut self, ctx: &mut ViewContext<Self>) {
        self.selected_setting = match self.selected_setting {
            Some(SettingCard::TabStyling) => Some(SettingCard::ToolsPanel),
            Some(SettingCard::ToolsPanel) => Some(SettingCard::CodeReview),
            None => Some(SettingCard::TabStyling),
            other => other,
        };
        ctx.notify();
    }

    fn on_left(&mut self, ctx: &mut ViewContext<Self>) {
        match self.selected_setting {
            Some(SettingCard::TabStyling) => {
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_use_vertical_tabs(true, ctx);
                });
                ctx.notify();
            }
            Some(SettingCard::ToolsPanel) => {
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_tools_panel_enabled(true, ctx);
                });
                ctx.notify();
            }
            Some(SettingCard::CodeReview) => {
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_show_code_review_button(true, ctx);
                });
                ctx.notify();
            }
            None => {}
        }
    }

    fn on_right(&mut self, ctx: &mut ViewContext<Self>) {
        match self.selected_setting {
            Some(SettingCard::TabStyling) => {
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_use_vertical_tabs(false, ctx);
                });
                ctx.notify();
            }
            Some(SettingCard::ToolsPanel) => {
                self.hovered_chip = None;
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_tools_panel_enabled(false, ctx);
                });
                ctx.notify();
            }
            Some(SettingCard::CodeReview) => {
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_show_code_review_button(false, ctx);
                });
                ctx.notify();
            }
            None => {}
        }
    }

    fn on_enter(&mut self, ctx: &mut ViewContext<Self>) {
        self.next(ctx);
    }
}

impl TypedActionView for CustomizeUISlide {
    type Action = CustomizeSlideAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CustomizeSlideAction::SelectSettingCard { card_index } => {
                self.select_setting_card(*card_index, ctx);
            }
            CustomizeSlideAction::SetTabStylingVertical { vertical } => {
                let value = *vertical;
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_use_vertical_tabs(value, ctx);
                });
                ctx.notify();
            }
            CustomizeSlideAction::SetToolsPanelEnabled { enabled } => {
                let value = *enabled;
                if !value {
                    self.hovered_chip = None;
                }
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_tools_panel_enabled(value, ctx);
                });
                ctx.notify();
            }
            CustomizeSlideAction::HoverToolsChip { setting } => {
                self.hovered_chip = Some(*setting);
                ctx.notify();
            }
            CustomizeSlideAction::ToggleToolsSubSetting { setting } => {
                let setting = *setting;
                self.onboarding_state
                    .update(ctx, |model, ctx| match setting {
                        ToolsPanelSubSetting::ConversationHistory => {
                            let current = model.ui_customization().show_conversation_history;
                            model.set_show_conversation_history(!current, ctx);
                        }
                        ToolsPanelSubSetting::ProjectExplorer => {
                            let current = model.ui_customization().show_project_explorer;
                            model.set_show_project_explorer(!current, ctx);
                        }
                        ToolsPanelSubSetting::GlobalSearch => {
                            let current = model.ui_customization().show_global_search;
                            model.set_show_global_search(!current, ctx);
                        }
                        ToolsPanelSubSetting::WarpDrive => {
                            let current = model.ui_customization().show_warp_drive;
                            model.set_show_warp_drive(!current, ctx);
                        }
                    });
                ctx.notify();
            }
            CustomizeSlideAction::SetCodeReviewEnabled { enabled } => {
                let value = *enabled;
                self.onboarding_state.update(ctx, |model, ctx| {
                    model.set_show_code_review_button(value, ctx);
                });
                ctx.notify();
            }
            CustomizeSlideAction::BackClicked => {
                let onboarding_state = self.onboarding_state.clone();
                onboarding_state.update(ctx, |model, ctx| {
                    model.back(ctx);
                });
            }
            CustomizeSlideAction::NextClicked => {
                self.next(ctx);
            }
        }
    }
}
