use std::iter;

use pathfinder_geometry::vector::vec2f;
use warpui::clipboard::ClipboardContent;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Empty, Expanded, Flex, HyperlinkLens, MainAxisAlignment, MainAxisSize, OffsetPositioning,
    ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use warpui::elements::FormattedTextElement;

use crate::ai::blocklist::code_block::{
    render_code_block_plain, CodeBlockOptions, CodeSnippetButtonHandles,
};
use crate::appearance::Appearance;
use crate::terminal::cli_agent_sessions::plugin_manager::PluginInstructions;
use crate::terminal::CLIAgent;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{ActionButton, ButtonSize, NakedTheme};
use crate::view_components::DismissibleToast;
use crate::workspace::{ToastStack, WorkspaceAction};

pub(crate) struct PluginInstructionsBlock {
    instructions: &'static PluginInstructions,
    is_remote_session: bool,
    close_button: ViewHandle<ActionButton>,
    step_code_handles: Vec<CodeSnippetButtonHandles>,
    should_hide: bool,
    /// Pre-computed commands with the custom command prefix substituted in.
    /// Each entry corresponds to the same-indexed step in `instructions.steps`.
    resolved_commands: Vec<String>,
}

impl PluginInstructionsBlock {
    pub fn new(
        instructions: &'static PluginInstructions,
        agent: CLIAgent,
        custom_command_prefix: Option<String>,
        is_remote_session: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let total_steps = instructions.steps.len();
        let mut step_code_handles = Vec::with_capacity(total_steps);
        step_code_handles.resize_with(total_steps, CodeSnippetButtonHandles::default);

        // When the session was detected via a custom toolbar command, the
        // user's binary likely differs from the agent's standard CLI tool
        // (e.g. `my-claude-wrapper` instead of `claude`).
        // Swap the agent's default prefix in each instruction command so
        // the displayed steps reference the command the user actually has.
        let resolved_commands = instructions
            .steps
            .iter()
            .map(|step| match &custom_command_prefix {
                Some(prefix) if step.command.starts_with(agent.command_prefix()) => {
                    format!("{prefix}{}", &step.command[agent.command_prefix().len()..])
                }
                _ => step.command.to_owned(),
            })
            .collect();

        let close_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("", NakedTheme)
                .with_icon(Icon::X)
                .with_size(ButtonSize::XSmall)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(PluginInstructionsBlockAction::Close);
                })
        });

        Self {
            instructions,
            is_remote_session,
            close_button,
            step_code_handles,
            should_hide: false,
            resolved_commands,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_step(
        &self,
        index: usize,
        description: &str,
        command: &str,
        executable: bool,
        link: Option<&str>,
        handles: CodeSnippetButtonHandles,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::handle(app).as_ref(app);
        let theme = appearance.theme();

        let step_number = render_step_number(index + 1, appearance);

        let desc_element: Box<dyn Element> = if let Some(url) = link {
            let fragments = vec![
                FormattedTextFragment::plain_text(format!("{description} ")),
                FormattedTextFragment::hyperlink("Learn more", url),
            ];
            let formatted = FormattedText::new(vec![FormattedTextLine::Line(fragments)]);
            FormattedTextElement::new(
                formatted,
                14.,
                appearance.ui_font_family(),
                appearance.monospace_font_family(),
                theme.nonactive_ui_text_color().into_solid(),
                Default::default(),
            )
            .with_hyperlink_font_color(theme.accent().into())
            .register_default_click_handlers_with_action_support(|hyperlink, _evt, app| {
                if let HyperlinkLens::Url(url) = hyperlink {
                    app.open_url(url);
                }
            })
            .finish()
        } else {
            Text::new(description.to_owned(), appearance.ui_font_family(), 14.)
                .with_color(theme.nonactive_ui_text_color().into_solid())
                .finish()
        };

        let title_row = Flex::row()
            .with_spacing(12.)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(step_number)
            .with_child(Expanded::new(1., desc_element).finish())
            .finish();

        let mut column = Flex::column().with_spacing(8.);
        column.add_child(title_row);

        if !command.is_empty() {
            let code_block = render_code_block_plain(
                command,
                Box::new(iter::empty()),
                CodeBlockOptions {
                    on_open: None,
                    on_execute: if executable {
                        Some(Box::new(move |code, ctx| {
                            ctx.dispatch_typed_action(WorkspaceAction::RunCommand(code));
                        }))
                    } else {
                        None
                    },
                    on_copy: Some(Box::new(move |_code, ctx| {
                        ctx.dispatch_typed_action(PluginInstructionsBlockAction::CopyCommand(
                            index,
                        ));
                    })),
                    on_insert: None,
                    footer_element: None,
                    mouse_handles: Some(handles),
                    file_path: None,
                },
                true,
                app,
                None,
            );
            column.add_child(Container::new(code_block).with_padding_left(40.).finish());
        }

        column.finish()
    }
}

impl Entity for PluginInstructionsBlock {
    type Event = PluginInstructionsBlockEvent;
}

impl View for PluginInstructionsBlock {
    fn ui_name() -> &'static str {
        "PluginInstructionsBlock"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if self.should_hide {
            return Empty::new().finish();
        }

        let appearance = Appearance::handle(app).as_ref(app);
        let theme = appearance.theme();

        let title = Text::new(
            self.instructions.title.to_owned(),
            appearance.ui_font_family(),
            20.,
        )
        .with_style(Properties::default().weight(Weight::Bold))
        .with_color(theme.main_text_color(theme.background()).into_solid())
        .finish();

        let subtitle_text = if self.is_remote_session {
            format!(
                "{} Be sure to run these commands on your remote machine.",
                self.instructions.subtitle
            )
        } else {
            self.instructions.subtitle.to_owned()
        };

        let subtitle = Text::new(subtitle_text, appearance.ui_font_family(), 14.)
            .with_color(theme.nonactive_ui_text_color().into_solid())
            .finish();

        let mut content = Flex::column()
            .with_spacing(16.)
            .with_cross_axis_alignment(CrossAxisAlignment::Start);

        content.add_child(title);
        content.add_child(subtitle);

        for (step_index, step) in self.instructions.steps.iter().enumerate() {
            let handles = self
                .step_code_handles
                .get(step_index)
                .cloned()
                .unwrap_or_default();
            let command = self
                .resolved_commands
                .get(step_index)
                .map(String::as_str)
                .unwrap_or(step.command);
            content.add_child(self.render_step(
                step_index,
                step.description,
                command,
                step.executable,
                step.link,
                handles,
                app,
            ));
        }

        for note in self.instructions.post_install_notes {
            let post_note = Text::new((*note).to_owned(), appearance.ui_font_family(), 14.)
                .with_color(theme.nonactive_ui_text_color().into_solid())
                .finish();
            content.add_child(post_note);
        }

        let close_button = ChildView::new(&self.close_button).finish();

        let body = Container::new(content.finish())
            .with_horizontal_padding(*super::PADDING_LEFT)
            .with_vertical_padding(16.)
            .with_border(
                Border::new(1.)
                    .with_sides(true, false, true, false)
                    .with_border_fill(theme.outline()),
            )
            .finish();

        Stack::new()
            .with_child(body)
            .with_positioned_child(
                close_button,
                OffsetPositioning::offset_from_parent(
                    vec2f(-8., 8.),
                    ParentOffsetBounds::ParentBySize,
                    ParentAnchor::TopRight,
                    ChildAnchor::TopRight,
                ),
            )
            .finish()
    }
}

impl TypedActionView for PluginInstructionsBlock {
    type Action = PluginInstructionsBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            PluginInstructionsBlockAction::Close => {
                self.should_hide = true;
                ctx.emit(PluginInstructionsBlockEvent::Close);
                ctx.notify();
            }
            PluginInstructionsBlockAction::CopyCommand(index) => {
                let command = self.resolved_commands.get(*index).cloned().or_else(|| {
                    self.instructions
                        .steps
                        .get(*index)
                        .map(|s| s.command.to_owned())
                });
                if let Some(text) = command {
                    ctx.clipboard().write(ClipboardContent::plain_text(text));
                    let window_id = ctx.window_id();
                    ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                        toast_stack.add_ephemeral_toast(
                            DismissibleToast::success(
                                crate::i18n::tr_static(ctx, "Copied to clipboard").to_owned(),
                            ),
                            window_id,
                            ctx,
                        );
                    });
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum PluginInstructionsBlockAction {
    Close,
    CopyCommand(usize),
}

pub(crate) enum PluginInstructionsBlockEvent {
    Close,
}

fn render_step_number(
    number: usize,
    appearance: &crate::appearance::Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();

    let number_text = Text::new(
        number.to_string(),
        appearance.ui_font_family(),
        appearance.ui_font_size(),
    )
    .with_style(Properties::default().weight(Weight::Semibold))
    .with_color(theme.main_text_color(theme.background()).into_solid())
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
