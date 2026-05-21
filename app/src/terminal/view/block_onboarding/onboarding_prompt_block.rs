use crate::appearance::Appearance;
use crate::context_chips::prompt::Prompt;
use crate::report_if_error;
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::{PromptChoice, TelemetryEvent};
use crate::settings::EnforceMinimumContrast;
use crate::terminal::blockgrid_element::BlockGridElement;
use crate::terminal::model::blockgrid::BlockGrid;
use crate::terminal::model::ObfuscateSecrets;
use crate::terminal::session_settings::SessionSettings;
use crate::terminal::view::block_onboarding::util;
use crate::terminal::SizeInfo;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use settings::Setting as _;
use warpui::{
    elements::{
        Align, Border, Clipped, ConstrainedBox, Container, CornerRadius, Flex,
        FormattedTextElement, HighlightedHyperlink, Hoverable, HyperlinkUrl, MainAxisAlignment,
        MainAxisSize, MouseStateHandle, ParentElement, Radius, Shrinkable, Text, Wrap,
    },
    fonts::Weight,
    platform::Cursor,
    ui_components::components::{UiComponent, UiComponentStyles},
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
};

const CONFIRM_MARGIN_TOP: f32 = 16.;

pub struct OnboardingPromptBlock {
    learn_more_highlight_index: HighlightedHyperlink,
    mouse_state_handle_look_incorrect: MouseStateHandle,
    mouse_state_handle_warp_prompt: MouseStateHandle,
    mouse_state_handle_existing_prompt: MouseStateHandle,
    mouse_state_handle_confirm: MouseStateHandle,
    ps1_grid_info: Option<(BlockGrid, SizeInfo)>,
    selected_prompt: Option<OnboardingPromptType>,
    block_completed: bool,
}

impl OnboardingPromptBlock {
    pub fn new(ps1_grid_info: Option<(BlockGrid, SizeInfo)>) -> Self {
        Self {
            learn_more_highlight_index: Default::default(),
            mouse_state_handle_look_incorrect: Default::default(),
            mouse_state_handle_warp_prompt: Default::default(),
            mouse_state_handle_existing_prompt: Default::default(),
            mouse_state_handle_confirm: Default::default(),
            ps1_grid_info,
            selected_prompt: None,
            block_completed: false,
        }
    }

    pub fn interrupt_block(&mut self, ctx: &mut ViewContext<Self>) {
        self.block_completed = true;
        ctx.notify();
    }

    fn render_text(&self, appearance: &Appearance) -> Box<dyn Element> {
        let current_theme = appearance.theme();
        let font_family = appearance.monospace_font_family();
        let font_size = appearance.monospace_font_size();
        let font_color = current_theme.main_text_color(current_theme.background());

        // Copy - https://docs.google.com/document/d/1zttBLI5Mw07kUupvrMQoC5aTwTXSHIUOIFFnxZ8GQEU/edit
        const LINE_ONE: &str = "Next, let’s set up your prompt. Warp has a custom prompt builder or you can select PS1 to honor your pre-existing prompt configuration.";
        const LINE_TWO: &str =
            "Warp works with many custom prompts like oh-my-zsh, Starship, Powerlevel10K. ";
        const LINK_TEXT: &str = "Learn more";
        const LINK_DESTINATION: &str =
            "https://docs.warp.dev/terminal/appearance/prompt#custom-prompt-compatibility-table";

        Flex::column()
            .with_children([
                Container::new(
                    Text::new(LINE_ONE, font_family, font_size)
                        .with_color(font_color.into_solid())
                        .finish(),
                )
                .with_margin_top(14.)
                .finish(),
                Container::new(
                    FormattedTextElement::new(
                        FormattedText::new([FormattedTextLine::Line(vec![
                            FormattedTextFragment::plain_text(LINE_TWO),
                            FormattedTextFragment::hyperlink(LINK_TEXT, LINK_DESTINATION),
                        ])]),
                        font_size,
                        font_family,
                        font_family,
                        font_color.into_solid(),
                        self.learn_more_highlight_index.clone(),
                    )
                    .with_hyperlink_font_color(current_theme.accent().into_solid())
                    .register_default_click_handlers(|url, ctx, _| {
                        ctx.dispatch_typed_action(OnboardingPromptBlockAction::HyperlinkClick(url));
                    })
                    .finish(),
                )
                .with_margin_top(14.)
                .finish(),
            ])
            .with_main_axis_size(MainAxisSize::Min)
            .finish()
    }

    fn render_confirm_skip_buttons(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut confirm_button = appearance
            .ui_builder()
            .button(
                warpui::ui_components::button::ButtonVariant::Accent,
                self.mouse_state_handle_confirm.clone(),
            )
            .with_style(UiComponentStyles {
                font_weight: Some(Weight::Bold),
                width: Some(80.),
                height: Some(40.),
                font_size: Some(14.),
                ..Default::default()
            })
            .with_centered_text_label(crate::i18n::tr_static(app, "Confirm").to_owned());
        if self.selected_prompt.is_none() {
            confirm_button = confirm_button.disabled();
        }

        let button_row = Flex::row().with_child(
            Container::new(
                confirm_button
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(OnboardingPromptBlockAction::PromptConfirmed)
                    })
                    .with_cursor(Cursor::PointingHand)
                    .finish(),
            )
            .with_margin_right(util::BUTTON_GAP)
            .finish(),
        );

        Container::new(button_row.finish())
            .with_margin_top(CONFIRM_MARGIN_TOP)
            .finish()
    }

    fn render_prompt_button(
        &self,
        appearance: &Appearance,
        mouse_state_handle: MouseStateHandle,
        prompt_type: OnboardingPromptType,
    ) -> Box<dyn Element> {
        // Pixel values pulled from Figma mocks
        // https://www.figma.com/file/y888viqzWBoMpFTxQqkQEN/Activation?node-id=568:1595&mode=dev
        const PROMPT_WIDTH: f32 = 442.;
        const PROMPT_HEIGHT: f32 = 136.;
        const PROMPT_BORDER_WIDTH: f32 = 1.;
        const PROMPT_BORDER_RADIUS: f32 = 8.;
        const PROMPT_MARGIN_RIGHT: f32 = 32.;
        const PROMPT_MARGIN_TOP: f32 = 31.;
        const PROMPT_PADDING: f32 = 16.;
        let active_theme = appearance.theme();

        let col = Flex::column()
            .with_child(
                Hoverable::new(mouse_state_handle.clone(), |state| {
                    let (border_color, border_width) =
                        match (self.selected_prompt, state.is_hovered()) {
                            (Some(selected_prompt), _) if selected_prompt == prompt_type => {
                                // If the selected prompt exists and matches, we use the current theme accent color
                                (active_theme.accent_button_color(), PROMPT_BORDER_WIDTH)
                            }
                            (_, true) => {
                                // Otherwise, if the selector is hovered, we use the highlight color
                                (active_theme.accent_button_color(), PROMPT_BORDER_WIDTH)
                            }
                            (_, false) => {
                                // For a non-hovered, non-selected prompt button, we use the default border style
                                (active_theme.surface_2(), PROMPT_BORDER_WIDTH)
                            }
                        };

                    ConstrainedBox::new(
                        Container::new(if prompt_type == OnboardingPromptType::WarpDefault {
                            self.render_warp_prompt_button_interior(appearance)
                        } else {
                            self.render_existing_prompt_button_interior(appearance)
                        })
                        .with_uniform_padding(PROMPT_PADDING)
                        .with_border(Border::all(border_width).with_border_fill(border_color))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                            PROMPT_BORDER_RADIUS,
                        )))
                        .finish(),
                    )
                    .with_height(PROMPT_HEIGHT)
                    .with_width(PROMPT_WIDTH)
                    .finish()
                })
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(OnboardingPromptBlockAction::PromptSelected(
                        prompt_type,
                    ))
                })
                .with_cursor(Cursor::PointingHand)
                .finish(),
            )
            .finish();

        Container::new(ConstrainedBox::new(col).with_width(PROMPT_WIDTH).finish())
            .with_margin_right(PROMPT_MARGIN_RIGHT)
            .with_margin_top(PROMPT_MARGIN_TOP)
            .finish()
    }

    fn render_ps1_prompt(
        &self,
        prompt_grid: &BlockGrid,
        size_info: &SizeInfo,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let left_padding = size_info.padding_x_px();

        Container::new(
            BlockGridElement::new(
                prompt_grid,
                appearance,
                EnforceMinimumContrast::OnlyNamedColors,
                ObfuscateSecrets::No,
                *size_info,
            )
            .finish(),
        )
        // Remove the padding that's built into the prompt
        .with_padding_left(-left_padding.as_f32())
        .finish()
    }

    fn render_existing_prompt_button_interior(&self, appearance: &Appearance) -> Box<dyn Element> {
        // Pixel values pulled from Figma mocks
        // https://www.figma.com/file/y888viqzWBoMpFTxQqkQEN/Activation?node-id=568:1595&mode=dev
        const HEADER_TEXT: &str = "Shell prompt (PS1)";
        const NO_PS1_TEXT: &str = "No existing prompt.";
        const CORRECTION_TEXT: &str = "Look incorrect? ";
        const LINK_TEXT: &str = "Let us know.";
        const LINK_DESTINATION: &str = "https://github.com/warpdotdev/Warp/issues/new?assignees=&labels=Bug&projects=&template=01_bug_report.yml";

        const HEADER_MARGIN_LEFT: f32 = 4.;
        const PS1_PADDING_VERTICAL: f32 = 12.;
        const PS1_PADDING_HORIZONTAL: f32 = 8.;
        const PS1_MARGIN_TOP: f32 = 8.;
        const CORNER_RADIUS_PIXELS: f32 = 4.;
        const CORRECTION_OPACITY: u8 = 60;

        let current_theme = appearance.theme();
        let font_family = appearance.monospace_font_family();
        let font_size = appearance.ui_font_size();
        let font_color = current_theme.main_text_color(current_theme.background());
        let prompt_body = if let Some((grid, size_info)) = &self.ps1_grid_info {
            let prompt_grid = self.render_ps1_prompt(grid, size_info, appearance);
            let clipped: Box<dyn Element> = Clipped::new(prompt_grid).finish();
            Container::new(clipped)
                .with_vertical_padding(PS1_PADDING_VERTICAL)
                .with_horizontal_padding(PS1_PADDING_HORIZONTAL)
                .with_margin_top(PS1_MARGIN_TOP)
                .with_background(current_theme.surface_1())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS_PIXELS)))
                .finish()
        } else {
            Text::new_inline(NO_PS1_TEXT, font_family, font_size)
                .with_color(font_color.into_solid())
                .finish()
        };

        let link_style = UiComponentStyles {
            font_size: Some(font_size),
            font_family_id: Some(font_family),
            ..Default::default()
        };

        Flex::column()
            .with_child(
                Container::new(
                    Text::new_inline(HEADER_TEXT, font_family, font_size)
                        .with_color(font_color.into_solid())
                        .finish(),
                )
                .with_margin_left(HEADER_MARGIN_LEFT)
                .finish(),
            )
            .with_child(prompt_body)
            .with_child(
                Shrinkable::new(
                    1.,
                    Align::new(
                        Flex::row()
                            .with_children([
                                Text::new_inline(CORRECTION_TEXT, font_family, font_size)
                                    .with_color(
                                        font_color.with_opacity(CORRECTION_OPACITY).into_solid(),
                                    )
                                    .finish(),
                                appearance
                                    .ui_builder()
                                    .link(
                                        LINK_TEXT.to_string(),
                                        Some(LINK_DESTINATION.to_string()),
                                        None,
                                        self.mouse_state_handle_look_incorrect.clone(),
                                    )
                                    .soft_wrap(false)
                                    .with_style(link_style)
                                    .build()
                                    .finish(),
                            ])
                            .with_main_axis_size(MainAxisSize::Min)
                            .finish(),
                    )
                    .bottom_right()
                    .finish(),
                )
                .finish(),
            )
            .with_main_axis_size(MainAxisSize::Max)
            .finish()
    }

    fn render_warp_prompt_button_interior(&self, appearance: &Appearance) -> Box<dyn Element> {
        // Pixel values pulled from Figma mocks
        // https://www.figma.com/file/y888viqzWBoMpFTxQqkQEN/Activation?node-id=568:1595&mode=dev
        const HEADER_TEXT: &str = "Warp prompt";
        const HEADER_MARGIN_LEFT: f32 = 4.;
        const SECTION_MARGIN_TOP: f32 = 8.;
        const OUTER_CORNER_RADIUS: f32 = 4.;
        const PROMPT_VERTICAL_PADDING: f32 = 12.;
        const PROMPT_HORIZONTAL_PADDING: f32 = 8.;

        let current_theme = appearance.theme();
        let font_family = appearance.monospace_font_family();
        let mono_font_size = appearance.monospace_font_size();
        let ui_font_size = appearance.ui_font_size();
        let font_color = current_theme.main_text_color(current_theme.background());
        let terminal_yellow = current_theme.terminal_colors().normal.yellow.into();
        let terminal_green = current_theme.terminal_colors().normal.green.into();
        let terminal_magenta = current_theme.terminal_colors().normal.magenta.into();

        let prompt_body = Container::new(
            Flex::row()
                .with_children([
                    Text::new_inline("(myenv)", font_family, mono_font_size)
                        .with_color(terminal_yellow)
                        .finish(),
                    Text::new_inline(" ~/myproject", font_family, mono_font_size)
                        .with_color(terminal_magenta)
                        .finish(),
                    Text::new_inline(" git:(", font_family, mono_font_size)
                        .with_color(terminal_green)
                        .finish(),
                    Text::new_inline("main", font_family, mono_font_size)
                        .with_color(terminal_yellow)
                        .finish(),
                    Text::new_inline(")", font_family, mono_font_size)
                        .with_color(terminal_green)
                        .finish(),
                    Text::new_inline("±2", font_family, mono_font_size)
                        .with_color(terminal_green)
                        .finish(),
                ])
                .with_main_axis_size(MainAxisSize::Min)
                .finish(),
        )
        .with_background(current_theme.surface_1())
        .with_margin_top(SECTION_MARGIN_TOP)
        .with_horizontal_padding(PROMPT_HORIZONTAL_PADDING)
        .with_vertical_padding(PROMPT_VERTICAL_PADDING)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(OUTER_CORNER_RADIUS)))
        .finish();

        Flex::column()
            .with_child(
                Container::new(
                    Text::new_inline(HEADER_TEXT, font_family, appearance.ui_font_size())
                        .with_color(font_color.into_solid())
                        .finish(),
                )
                .with_margin_left(HEADER_MARGIN_LEFT)
                .finish(),
            )
            .with_child(prompt_body)
            .with_child(
                Shrinkable::new(
                    1.,
                    Align::new(
                        Text::new_inline(
                            "Customizable in appearance settings.",
                            font_family,
                            ui_font_size,
                        )
                        .with_color(font_color.with_opacity(60).into_solid())
                        .finish(),
                    )
                    .bottom_right()
                    .finish(),
                )
                .finish(),
            )
            .with_main_axis_size(MainAxisSize::Max)
            .finish()
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum OnboardingPromptType {
    PS1,
    WarpDefault,
}

impl Entity for OnboardingPromptBlock {
    type Event = ();
}

impl View for OnboardingPromptBlock {
    fn ui_name() -> &'static str {
        "OnboardingPromptBlock"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        const PADDING_HORIZONTAL: f32 = 18.;
        const PADDING_TOP: f32 = 18.;
        const PADDING_BOTTOM: f32 = 18.;
        const BORDER_TOP_WIDTH: f32 = 1.;

        let appearance: &Appearance = Appearance::as_ref(ctx);
        let current_theme = appearance.theme();
        let border_color = current_theme.outline();

        let mut col = Flex::column()
            .with_child(self.render_text(appearance))
            .with_child(
                Wrap::row()
                    .with_run_spacing(-PADDING_BOTTOM)
                    .with_main_axis_alignment(MainAxisAlignment::Start)
                    .with_child(self.render_prompt_button(
                        appearance,
                        self.mouse_state_handle_warp_prompt.clone(),
                        OnboardingPromptType::WarpDefault,
                    ))
                    .with_child(self.render_prompt_button(
                        appearance,
                        self.mouse_state_handle_existing_prompt.clone(),
                        OnboardingPromptType::PS1,
                    ))
                    .finish(),
            );
        if !self.block_completed {
            col.add_child(self.render_confirm_skip_buttons(appearance, ctx));
        }

        Container::new(col.finish())
            .with_padding_left(PADDING_HORIZONTAL)
            .with_padding_right(PADDING_HORIZONTAL)
            .with_padding_top(PADDING_TOP)
            .with_padding_bottom(PADDING_BOTTOM)
            .with_border(Border::top(BORDER_TOP_WIDTH).with_border_fill(border_color))
            .finish()
    }
}

#[derive(Debug, Clone)]
pub enum OnboardingPromptBlockAction {
    PromptSelected(OnboardingPromptType),
    PromptConfirmed,
    HyperlinkClick(HyperlinkUrl),
}

impl TypedActionView for OnboardingPromptBlock {
    type Action = OnboardingPromptBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            OnboardingPromptBlockAction::PromptSelected(prompt) => {
                self.selected_prompt = Some(*prompt);

                match prompt {
                    OnboardingPromptType::WarpDefault => {
                        self.selected_prompt = Some(OnboardingPromptType::WarpDefault);
                        Prompt::handle(ctx).update(ctx, |prompt, ctx| {
                            report_if_error!(prompt.reset(ctx));
                        });
                        ctx.notify();
                        send_telemetry_from_ctx!(
                            TelemetryEvent::PromptEdited {
                                prompt: PromptChoice::Default,
                                entrypoint: "setup_guide".to_string()
                            },
                            ctx
                        );
                    }
                    OnboardingPromptType::PS1 => {
                        self.selected_prompt = Some(OnboardingPromptType::PS1);
                        SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
                            report_if_error!(settings.honor_ps1.set_value(true, ctx));
                        });
                        ctx.notify();
                        send_telemetry_from_ctx!(
                            TelemetryEvent::PromptEdited {
                                prompt: PromptChoice::PS1,
                                entrypoint: "setup_guide".to_string()
                            },
                            ctx
                        );
                    }
                }
            }
            OnboardingPromptBlockAction::PromptConfirmed => {
                self.block_completed = true;
                ctx.notify();
            }
            OnboardingPromptBlockAction::HyperlinkClick(hyperlink) => {
                ctx.notify();
                ctx.open_url(&hyperlink.url);
            }
        }
    }
}
