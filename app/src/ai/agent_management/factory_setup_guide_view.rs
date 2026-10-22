use warpui::clipboard::ClipboardContent;
use warpui::elements::new_scrollable::{ClippedAxisConfiguration, DualAxisConfig, NewScrollable};
use warpui::elements::{
    Align, Border, ClippedScrollStateHandle, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Element, Expanded, Flex, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement, Radius, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::prelude::ChildView;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use crate::ai::blocklist::code_block::{
    CodeBlockOptions, CodeSnippetButtonHandles, render_code_block_plain,
};
use crate::appearance::Appearance;
use crate::view_components::action_button::{ActionButton, SecondaryTheme};

const FACTORY_DOCS_URL: &str = "https://docs.warp.dev/agent-platform/cloud-agents/self-hosting";

const CONTENT_MAX_WIDTH: f32 = 720.;

const CREATE_API_KEY_CMD: &str = "oz api-key create worker-key --no-expiration";
const DOCKER_RUN_CMD: &str = "docker run -v /var/run/docker.sock:/var/run/docker.sock \\\n  -e WARP_API_KEY=\"<your-api-key>\" \\\n  warpdotdev/oz-agent-worker:latest \\\n  --worker-id \"my-worker\"";

pub struct FactorySetupGuideView {
    create_api_key_code_handles: CodeSnippetButtonHandles,
    docker_run_code_handles: CodeSnippetButtonHandles,
    docs_link_mouse_state: MouseStateHandle,
    visit_docs_button: ViewHandle<ActionButton>,
    vertical_scroll_state: ClippedScrollStateHandle,
    horizontal_scroll_state: ClippedScrollStateHandle,
}

#[derive(Debug, Clone)]
pub enum FactorySetupGuideAction {
    CopyCode { code: String },
    VisitDocs,
}

impl FactorySetupGuideView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let visit_docs_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Visit docs", SecondaryTheme)
                .on_click(|ctx| ctx.dispatch_typed_action(FactorySetupGuideAction::VisitDocs))
        });

        Self {
            create_api_key_code_handles: CodeSnippetButtonHandles::default(),
            docker_run_code_handles: CodeSnippetButtonHandles::default(),
            docs_link_mouse_state: MouseStateHandle::default(),
            visit_docs_button,
            vertical_scroll_state: ClippedScrollStateHandle::default(),
            horizontal_scroll_state: ClippedScrollStateHandle::default(),
        }
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let title_font_size = 24.;
        let subtitle_font_size = 16.;

        let mut header_container = Flex::column().with_spacing(8.);

        let title = Text::new(
            "Getting started with Oz Factory",
            appearance.ui_font_family(),
            title_font_size,
        )
        .with_style(Properties::default().weight(Weight::Semibold))
        .with_color(theme.active_ui_text_color().into_solid())
        .finish();
        header_container.add_child(title);

        let subtitle = Text::new(
            "Run Warp cloud agents on your own infrastructure with a self-hosted worker. Your code and data stay on your machines.",
            appearance.ui_font_family(),
            subtitle_font_size,
        )
        .with_color(theme.nonactive_ui_text_color().into_solid())
        .finish();
        header_container.add_child(subtitle);

        let docs_line = Flex::row()
            .with_child(
                Text::new_inline(
                    "Check out the ",
                    appearance.ui_font_family(),
                    subtitle_font_size,
                )
                .with_color(theme.nonactive_ui_text_color().into_solid())
                .finish(),
            )
            .with_child(
                appearance
                    .ui_builder()
                    .link(
                        "self-hosting documentation".to_string(),
                        None,
                        Some(Box::new(|ctx| {
                            ctx.dispatch_typed_action(FactorySetupGuideAction::VisitDocs);
                        })),
                        self.docs_link_mouse_state.clone(),
                    )
                    .with_style(UiComponentStyles {
                        font_size: Some(subtitle_font_size),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_child(
                Text::new_inline(
                    " to learn more.",
                    appearance.ui_font_family(),
                    subtitle_font_size,
                )
                .with_color(theme.nonactive_ui_text_color().into_solid())
                .finish(),
            );
        header_container.add_child(docs_line.finish());

        header_container.finish()
    }

    fn render_quick_start_banner(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let font_size = 16.;

        let text = Text::new_inline(
            "Quick start: Visit the self-hosting docs for full setup instructions.",
            appearance.ui_font_family(),
            font_size,
        )
        .with_style(Properties::default().weight(Weight::Semibold))
        .with_color(theme.active_ui_text_color().into_solid())
        .finish();

        let border_color = theme.ansi_overlay_2(theme.terminal_colors().normal.cyan);

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(text)
                .with_child(ChildView::new(&self.visit_docs_button).finish())
                .finish(),
        )
        .with_background(theme.surface_overlay_1())
        .with_border(Border::all(1.).with_border_fill(border_color))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_horizontal_padding(16.)
        .with_vertical_padding(12.)
        .finish()
    }

    fn render_step_number(number: u32, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let number_text = Text::new(
            number.to_string(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_style(Properties::default().weight(Weight::Semibold))
        .with_color(theme.active_ui_text_color().into_solid())
        .finish();

        let centered_number = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(number_text)
            .finish();

        Container::new(
            ConstrainedBox::new(centered_number)
                .with_width(28.)
                .with_height(28.)
                .finish(),
        )
        .with_background(theme.surface_1())
        .with_border(Border::all(1.).with_border_fill(theme.outline()))
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
        .finish()
    }

    fn render_code_block_with_copy_only(
        &self,
        code: &'static str,
        handles: CodeSnippetButtonHandles,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let code_to_copy = code.to_string();
        render_code_block_plain(
            code,
            std::iter::empty(),
            CodeBlockOptions {
                on_open: None,
                on_execute: None,
                on_copy: Some(Box::new(move |_code, ctx| {
                    ctx.dispatch_typed_action(FactorySetupGuideAction::CopyCode {
                        code: code_to_copy.clone(),
                    });
                })),
                on_insert: None,
                footer_element: None,
                mouse_handles: Some(handles),
                file_path: None,
            },
            true,
            app,
            None,
        )
    }

    /// Render step 1: Create a team API key.
    fn render_step_1(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();
        let step_title_font_size = 14.;
        let step_desc_font_size = 14.;

        let title_row = Flex::row()
            .with_spacing(16.)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Self::render_step_number(1, appearance))
            .with_child(
                Text::new(
                    "Create a team API key",
                    appearance.ui_font_family(),
                    step_title_font_size,
                )
                .with_style(Properties::default().weight(Weight::Semibold))
                .with_color(theme.active_ui_text_color().into_solid())
                .finish(),
            )
            .finish();

        let description = Container::new(
            Text::new(
                "The worker needs a team API key to authenticate with Oz. Create one in Warp Settings under Platform > API Keys, or use the CLI:",
                appearance.ui_font_family(),
                step_desc_font_size,
            )
            .with_color(theme.nonactive_ui_text_color().into_solid())
            .finish(),
        )
        .with_padding_left(46.)
        .with_padding_bottom(8.)
        .finish();

        let code_block = Container::new(self.render_code_block_with_copy_only(
            CREATE_API_KEY_CMD,
            self.create_api_key_code_handles.clone(),
            app,
        ))
        .with_padding_left(46.)
        .finish();

        Flex::column()
            .with_spacing(8.)
            .with_child(title_row)
            .with_child(description)
            .with_child(code_block)
            .finish()
    }

    /// Render step 2: Start the oz-agent-worker.
    fn render_step_2(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();
        let step_title_font_size = 14.;
        let step_desc_font_size = 14.;

        let title_row = Flex::row()
            .with_spacing(16.)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Self::render_step_number(2, appearance))
            .with_child(
                Text::new(
                    "Start the oz-agent-worker",
                    appearance.ui_font_family(),
                    step_title_font_size,
                )
                .with_style(Properties::default().weight(Weight::Semibold))
                .with_color(theme.active_ui_text_color().into_solid())
                .finish(),
            )
            .finish();

        let description = Container::new(
            Text::new(
                "Pull and run the oz-agent-worker Docker image with your team API key. The worker connects to Oz and executes agent tasks on your infrastructure.",
                appearance.ui_font_family(),
                step_desc_font_size,
            )
            .with_color(theme.nonactive_ui_text_color().into_solid())
            .finish(),
        )
        .with_padding_left(46.)
        .with_padding_bottom(8.)
        .finish();

        let code_block = Container::new(self.render_code_block_with_copy_only(
            DOCKER_RUN_CMD,
            self.docker_run_code_handles.clone(),
            app,
        ))
        .with_padding_left(46.)
        .finish();

        Flex::column()
            .with_spacing(8.)
            .with_child(title_row)
            .with_child(description)
            .with_child(code_block)
            .finish()
    }
}

impl Entity for FactorySetupGuideView {
    type Event = ();
}

impl View for FactorySetupGuideView {
    fn ui_name() -> &'static str {
        "FactorySetupGuideView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let steps = Flex::column()
            .with_spacing(24.)
            .with_child(self.render_step_1(appearance, app))
            .with_child(self.render_step_2(appearance, app))
            .finish();

        let mut content = Flex::column()
            .with_spacing(24.)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        content.add_child(self.render_header(appearance));
        content.add_child(self.render_quick_start_banner(appearance));
        content.add_child(steps);

        let content = content.finish();

        let scrollable = NewScrollable::horizontal_and_vertical(
            DualAxisConfig::Clipped {
                horizontal: ClippedAxisConfiguration {
                    handle: self.horizontal_scroll_state.clone(),
                    max_size: None,
                    stretch_child: true,
                },
                vertical: ClippedAxisConfiguration {
                    handle: self.vertical_scroll_state.clone(),
                    max_size: None,
                    stretch_child: false,
                },
                child: Align::new(
                    Container::new(
                        ConstrainedBox::new(content)
                            .with_max_width(CONTENT_MAX_WIDTH)
                            .finish(),
                    )
                    .with_uniform_padding(24.)
                    .finish(),
                )
                .top_center()
                .finish(),
            },
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(Expanded::new(1., scrollable).finish())
            .finish()
    }
}

impl TypedActionView for FactorySetupGuideView {
    type Action = FactorySetupGuideAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            FactorySetupGuideAction::CopyCode { code } => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(code.clone()));
            }
            FactorySetupGuideAction::VisitDocs => {
                ctx.open_url(FACTORY_DOCS_URL);
            }
        }
    }
}
