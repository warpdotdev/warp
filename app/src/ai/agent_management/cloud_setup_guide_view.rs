use crate::ai::agent_management::telemetry::{AgentManagementTelemetryEvent, SetupGuideStep};
use crate::ai::blocklist::code_block::{
    render_code_block_plain, CodeBlockOptions, CodeSnippetButtonHandles,
};
use crate::appearance::Appearance;
use crate::completer::SessionAgnosticContext;
use crate::send_telemetry_from_ctx;
use crate::view_components::action_button::{ActionButton, SecondaryTheme};
use crate::workflows::workflow::{Argument, ArgumentType, Workflow};
use crate::workflows::WorkflowType;
use serde::Serialize;
use std::collections::HashMap;
use string_offset::CharCounter;
use warp_completer::signatures::CommandRegistry;
use warp_completer::{util::parse_current_commands_and_tokens, ParsedTokensSnapshot};
use warp_core::report_error;
use warp_core::ui::theme::{AnsiColorIdentifier, AnsiColors};
use warpui::clipboard::ClipboardContent;
use warpui::elements::{
    new_scrollable::{ClippedAxisConfiguration, DualAxisConfig, NewScrollable},
    Align, Border, ClippedScrollStateHandle, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Element, Empty, Expanded, Flex, Highlight, HighlightedRange,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::prelude::ChildView;
use warpui::text_layout::TextStyle;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::ViewHandle;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext};

const DOCS_URL: &str = "https://docs.warp.dev/agent-platform/cloud-agents/overview";
const ENV_DOCS_URL: &str =
    "https://docs.warp.dev/reference/cli/integration-setup#creating-an-environment";
const OZ_URL: &str = "https://oz.warp.dev";

const CONTENT_MAX_WIDTH: f32 = 720.;

const CREATE_ENV_SLASH_CMD: &str = "/create-environment";
const CREATE_ENV_CLI_CMD: &str =
    "oz environment create [OPTIONS] --name <NAME> --docker-image <DOCKER_IMAGE>";
const CREATE_SLACK_INTEGRATION_CMD: &str =
    "oz integration create slack --environment {{environment_id}}";
const CREATE_LINEAR_INTEGRATION_CMD: &str =
    "oz integration create linear --environment {{environment_id}}";

pub struct CloudSetupGuideView {
    create_env_code_handles: CodeSnippetButtonHandles,
    create_env_cli_code_handles: CodeSnippetButtonHandles,
    create_slack_integration_code_handles: CodeSnippetButtonHandles,
    create_linear_integration_code_handles: CodeSnippetButtonHandles,
    docs_link_mouse_state: MouseStateHandle,
    env_docs_link_mouse_state: MouseStateHandle,
    integration_docs_link_mouse_state: MouseStateHandle,
    visit_oz_button: ViewHandle<ActionButton>,
    parsed_tokens: HashMap<&'static str, ParsedTokensSnapshot>,
    vertical_scroll_state: ClippedScrollStateHandle,
    horizontal_scroll_state: ClippedScrollStateHandle,
}

#[derive(Debug, Clone)]
pub enum CloudSetupGuideAction {
    CopyCode {
        code: String,
        step: SetupGuideStep,
    },
    RunWorkflow {
        workflow: Box<WorkflowType>,
        step: SetupGuideStep,
    },
    VisitOz,
    OpenDocs {
        docs: SetupGuideDocs,
    },
}

/// Which URL the user clicked in the setup guide (also used in telemetry)
#[derive(Clone, Copy, Debug, Serialize)]
pub enum SetupGuideDocs {
    Main,
    Environment,
    Integration,
}

pub enum CloudSetupGuideEvent {
    OpenNewTabAndInsertWorkflow(WorkflowType),
}

impl CloudSetupGuideView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let code_snippets: Vec<&'static str> = vec![
            CREATE_ENV_SLASH_CMD,
            CREATE_ENV_CLI_CMD,
            CREATE_SLACK_INTEGRATION_CMD,
            CREATE_LINEAR_INTEGRATION_CMD,
        ];

        // We spawn a background task to parse the commands in the code blocks,
        // and re-render the blocks once it's done. Colors are computed at render time
        // so they respond to theme changes.
        let context = SessionAgnosticContext::new(CommandRegistry::global_instance());
        ctx.spawn(
            async move {
                let mut results = HashMap::new();
                for snippet in code_snippets {
                    let parsed =
                        parse_current_commands_and_tokens(snippet.to_string(), &context).await;
                    results.insert(snippet, parsed);
                }
                results
            },
            |view, results, ctx| {
                view.parsed_tokens = results;
                ctx.notify();
            },
        );

        let visit_oz_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Visit Oz"), SecondaryTheme)
                .on_click(|ctx| ctx.dispatch_typed_action(CloudSetupGuideAction::VisitOz))
        });

        Self {
            create_env_code_handles: CodeSnippetButtonHandles::default(),
            create_env_cli_code_handles: CodeSnippetButtonHandles::default(),
            create_slack_integration_code_handles: CodeSnippetButtonHandles::default(),
            create_linear_integration_code_handles: CodeSnippetButtonHandles::default(),
            docs_link_mouse_state: MouseStateHandle::default(),
            env_docs_link_mouse_state: MouseStateHandle::default(),
            integration_docs_link_mouse_state: MouseStateHandle::default(),
            visit_oz_button,
            parsed_tokens: HashMap::new(),
            vertical_scroll_state: ClippedScrollStateHandle::default(),
            horizontal_scroll_state: ClippedScrollStateHandle::default(),
        }
    }

    /// Render the main header for the setup guide.
    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let title_font_size = 24.;
        let subtitle_font_size = 16.;

        let mut header_container = Flex::column().with_spacing(8.);

        let title = Text::new(
            "Getting started with Oz cloud agents",
            appearance.ui_font_family(),
            title_font_size,
        )
        .with_style(Properties::default().weight(Weight::Semibold))
        .with_color(theme.active_ui_text_color().into_solid())
        .finish();
        header_container.add_child(title);

        let subtitle = Text::new(
            "Start Oz cloud agents directly in Warp from an integration (Linear, Slack), with an event (GitHub, built-in schedule), or programmatically with the Oz SDK or CLI.",
            appearance.ui_font_family(),
            subtitle_font_size,
        )
        .with_color(theme.nonactive_ui_text_color().into_solid())
        .finish();
        header_container.add_child(subtitle);

        // Documentation link line.
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
                        "Oz documentation".to_string(),
                        None,
                        Some(Box::new(|ctx| {
                            ctx.dispatch_typed_action(CloudSetupGuideAction::OpenDocs {
                                docs: SetupGuideDocs::Main,
                            });
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

    /// Render the quick start banner with link to oz.warp.dev.
    fn render_quick_start_banner(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let font_size = 16.;

        let text = Text::new_inline(
            "Quick start: Visit oz.warp.dev for a UI-based setup experience.",
            appearance.ui_font_family(),
            font_size,
        )
        .with_style(Properties::default().weight(Weight::Semibold))
        .with_color(theme.active_ui_text_color().into_solid())
        .finish();

        // Use cyan overlay for the blue border per Figma spec.
        let border_color = theme.ansi_overlay_2(theme.terminal_colors().normal.cyan);

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(text)
                .with_child(ChildView::new(&self.visit_oz_button).finish())
                .finish(),
        )
        .with_background(theme.surface_overlay_1())
        .with_border(Border::all(1.).with_border_fill(border_color))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_horizontal_padding(16.)
        .with_vertical_padding(12.)
        .finish()
    }

    /// Render the manual setup section header.
    fn render_manual_setup_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let font_size = 16.;

        Text::new(
            "Manual setup: Create a Slack or Linear integration with the Oz CLI",
            appearance.ui_font_family(),
            font_size,
        )
        .with_style(Properties::default().weight(Weight::Semibold))
        .with_color(theme.active_ui_text_color().into_solid())
        .finish()
    }

    /// Render a styled number to be displayed with each step.
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

    /// Render a description that includes a link at the end
    /// (e.g. "Use warp's environment setup command to have an agent help you through it. LINK[Visit docs]")
    fn render_description_with_link(
        prefix: &'static str,
        link_text: &'static str,
        link_mouse_state: MouseStateHandle,
        telemetry_url: SetupGuideDocs,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let step_desc_font_size = 14.;
        let link = appearance
            .ui_builder()
            .link(
                link_text.to_string(),
                None,
                Some(Box::new(move |ctx| {
                    ctx.dispatch_typed_action(CloudSetupGuideAction::OpenDocs {
                        docs: telemetry_url,
                    });
                })),
                link_mouse_state,
            )
            .with_style(UiComponentStyles {
                font_size: Some(step_desc_font_size),
                ..Default::default()
            })
            .build()
            .finish();

        Flex::row()
            .with_child(
                Text::new_inline(prefix, appearance.ui_font_family(), step_desc_font_size)
                    .with_color(appearance.theme().nonactive_ui_text_color().into_solid())
                    .finish(),
            )
            .with_child(link)
            .finish()
    }

    /// Render a code block with buttons to copy and run the code.
    fn render_code_block(
        &self,
        code: &'static str,
        handles: CodeSnippetButtonHandles,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let terminal_colors = Appearance::as_ref(app).theme().terminal_colors().normal;
        let highlights = self
            .parsed_tokens
            .get(code)
            .map(|parsed| tokens_to_highlight_ranges(parsed, &terminal_colors))
            .unwrap_or_default();

        // Match command to formatted workflow with correct args.
        let Some((workflow, setup_step)) = (match code {
            CREATE_ENV_SLASH_CMD => Some((
                WorkflowType::Local(
                    Workflow::new("Create Environment", CREATE_ENV_SLASH_CMD).with_arguments(vec![
                        Argument::new("github link or local filepath", ArgumentType::Text)
                            .with_description("GitHub link or local filepath to the repository"),
                    ]),
                ),
                SetupGuideStep::CreateEnvironment,
            )),
            CREATE_ENV_CLI_CMD => Some((
                WorkflowType::Local(
                    Workflow::new("Create Environment (CLI)", CREATE_ENV_CLI_CMD).with_arguments(
                        vec![
                            Argument::new("NAME", ArgumentType::Text)
                                .with_description("Name for the environment"),
                            Argument::new("DOCKER_IMAGE", ArgumentType::Text)
                                .with_description("Docker image to use for the environment"),
                        ],
                    ),
                ),
                SetupGuideStep::CreateEnvironmentCli,
            )),
            CREATE_SLACK_INTEGRATION_CMD => Some((
                WorkflowType::Local(
                    Workflow::new("Create Slack Integration", CREATE_SLACK_INTEGRATION_CMD)
                        .with_arguments(vec![Argument::new("environment_id", ArgumentType::Text)
                            .with_description("ID of the environment to integrate with")]),
                ),
                SetupGuideStep::CreateSlackIntegration,
            )),
            CREATE_LINEAR_INTEGRATION_CMD => Some((
                WorkflowType::Local(
                    Workflow::new("Create Linear Integration", CREATE_LINEAR_INTEGRATION_CMD)
                        .with_arguments(vec![Argument::new("environment_id", ArgumentType::Text)
                            .with_description("ID of the environment to integrate with")]),
                ),
                SetupGuideStep::CreateLinearIntegration,
            )),
            _ => None,
        }) else {
            report_error!(anyhow::anyhow!(
                "Received unknown code in render_code_block: {}",
                code
            ));
            return Empty::new().finish();
        };

        render_code_block_plain(
            code,
            highlights.into_iter(),
            CodeBlockOptions {
                on_open: None,
                on_execute: Some(Box::new(move |_code, ctx| {
                    ctx.dispatch_typed_action(CloudSetupGuideAction::RunWorkflow {
                        workflow: Box::new(workflow.clone()),
                        step: setup_step,
                    });
                })),
                on_copy: Some(Box::new(move |_code, ctx| {
                    ctx.dispatch_typed_action(CloudSetupGuideAction::CopyCode {
                        code: code.to_string().clone(),
                        step: setup_step,
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

    /// Render step 1: Create an environment.
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
                    "Create an environment",
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
                "First, set up an environment to create an integration.",
                appearance.ui_font_family(),
                step_desc_font_size,
            )
            .with_color(theme.nonactive_ui_text_color().into_solid())
            .finish(),
        )
        .with_padding_left(46.)
        .finish();

        let sub_description = Container::new(Self::render_description_with_link(
            "Use Warp's environment setup command to have an agent help you through it. ",
            "Visit docs",
            self.env_docs_link_mouse_state.clone(),
            SetupGuideDocs::Environment,
            appearance,
        ))
        .with_padding_left(46.)
        .with_padding_bottom(8.)
        .finish();

        let slash_cmd_code_block = Container::new(self.render_code_block(
            CREATE_ENV_SLASH_CMD,
            self.create_env_code_handles.clone(),
            app,
        ))
        .with_padding_left(46.)
        .finish();

        let or_text = Container::new(
            Text::new(
                "Or, supply your own existing docker image.",
                appearance.ui_font_family(),
                step_desc_font_size,
            )
            .with_color(theme.nonactive_ui_text_color().into_solid())
            .finish(),
        )
        .with_padding_left(46.)
        .with_padding_top(8.)
        .with_padding_bottom(8.)
        .finish();

        let cli_code_block = Container::new(self.render_code_block(
            CREATE_ENV_CLI_CMD,
            self.create_env_cli_code_handles.clone(),
            app,
        ))
        .with_padding_left(46.)
        .finish();

        Flex::column()
            .with_spacing(8.)
            .with_child(title_row)
            .with_child(description)
            .with_child(sub_description)
            .with_child(slash_cmd_code_block)
            .with_child(or_text)
            .with_child(cli_code_block)
            .finish()
    }

    /// Render step 2: Create an integration.
    fn render_step_2(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();
        let step_title_font_size = 14.;

        let title_row = Flex::row()
            .with_spacing(16.)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Self::render_step_number(2, appearance))
            .with_child(
                Text::new(
                    "Create an integration",
                    appearance.ui_font_family(),
                    step_title_font_size,
                )
                .with_style(Properties::default().weight(Weight::Semibold))
                .with_color(theme.active_ui_text_color().into_solid())
                .finish(),
            )
            .finish();

        let sub_description = Container::new(Self::render_description_with_link(
            "Integrate Slack or Linear to assign Warp's Agent tasks with @Warp. ",
            "Visit docs",
            self.integration_docs_link_mouse_state.clone(),
            SetupGuideDocs::Integration,
            appearance,
        ))
        .with_padding_left(46.)
        .with_padding_bottom(8.)
        .finish();

        let code_block_slack = Container::new(self.render_code_block(
            CREATE_SLACK_INTEGRATION_CMD,
            self.create_slack_integration_code_handles.clone(),
            app,
        ))
        .with_padding_left(46.)
        .finish();

        let code_block_linear = Container::new(self.render_code_block(
            CREATE_LINEAR_INTEGRATION_CMD,
            self.create_linear_integration_code_handles.clone(),
            app,
        ))
        .with_padding_left(46.)
        .finish();

        Flex::column()
            .with_spacing(8.)
            .with_child(title_row)
            .with_child(sub_description)
            .with_child(code_block_slack)
            .with_child(code_block_linear)
            .finish()
    }
}

impl Entity for CloudSetupGuideView {
    type Event = CloudSetupGuideEvent;
}

impl View for CloudSetupGuideView {
    fn ui_name() -> &'static str {
        "AgentManagementHelpPageView"
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
        content.add_child(self.render_manual_setup_header(appearance));
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

impl TypedActionView for CloudSetupGuideView {
    type Action = CloudSetupGuideAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CloudSetupGuideAction::CopyCode { code, step } => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::SetupGuideStepCopy { step: *step },
                    ctx
                );
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(code.clone()));
            }
            CloudSetupGuideAction::RunWorkflow { workflow, step } => {
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::SetupGuideStepRun { step: *step },
                    ctx
                );
                ctx.emit(CloudSetupGuideEvent::OpenNewTabAndInsertWorkflow(
                    (**workflow).clone(),
                ));
            }
            CloudSetupGuideAction::VisitOz => {
                ctx.open_url(OZ_URL);
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::SetupGuideStepRun {
                        step: SetupGuideStep::VisitOz
                    },
                    ctx
                );
            }
            CloudSetupGuideAction::OpenDocs { docs } => {
                let url = match docs {
                    SetupGuideDocs::Main => DOCS_URL,
                    SetupGuideDocs::Environment => ENV_DOCS_URL,
                    SetupGuideDocs::Integration => DOCS_URL,
                };
                ctx.open_url(url);
                send_telemetry_from_ctx!(
                    AgentManagementTelemetryEvent::SetupGuideDocsLink { docs: *docs },
                    ctx
                );
            }
        }
    }
}

/// Highlight the commands in command blocks correctly (including the command prefix).
/// It's unfortunate that we have to do this manually, but the alternative is inserting a custom code editor into this component
/// and that would be a lot of bloat for not much benefit.
fn tokens_to_highlight_ranges(
    parsed_tokens: &ParsedTokensSnapshot,
    terminal_colors: &AnsiColors,
) -> Vec<HighlightedRange> {
    let code = &parsed_tokens.buffer_text;
    let mut highlights = Vec::new();

    // Handle slash commands: if code starts with '/', highlight the command prefix in magenta
    if code.starts_with('/') {
        if let Some(space_idx) = code.find(' ') {
            let color = AnsiColorIdentifier::Magenta.to_ansi_color(terminal_colors);
            highlights.push(HighlightedRange {
                highlight: Highlight::new()
                    .with_text_style(TextStyle::new().with_foreground_color(color.into())),
                highlight_indices: (0..space_idx).collect(),
            });
            return highlights;
        }
    }

    // Highlight commands in the code block (converting bytes to char indexes as we go).
    let mut char_counter = CharCounter::new(code);
    for token_data in &parsed_tokens.parsed_tokens {
        let Some(description) = &token_data.token_description else {
            continue;
        };

        let byte_start = token_data.token.span.start();
        let byte_end = token_data.token.span.end();

        let Some(char_start) = char_counter.char_offset(byte_start) else {
            continue;
        };
        let Some(char_end) = char_counter.char_offset(byte_end) else {
            continue;
        };

        let char_indices: Vec<usize> = (char_start.as_usize()..char_end.as_usize()).collect();
        if char_indices.is_empty() {
            continue;
        }

        let color_id: AnsiColorIdentifier = description.suggestion_type.to_name().into();
        let color = color_id.to_ansi_color(terminal_colors);

        highlights.push(HighlightedRange {
            highlight: Highlight::new()
                .with_text_style(TextStyle::new().with_foreground_color(color.into())),
            highlight_indices: char_indices,
        });
    }

    highlights
}
