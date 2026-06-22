use lazy_static::lazy_static;
use markdown_parser::weight::CustomWeight;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use regex::Regex;
use serde::{Deserialize, Serialize};
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex,
    FormattedTextElement, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement, Radius, Text, Wrap,
};
use warpui::keymap::Keystroke;
use warpui::platform::Cursor;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    WeakViewHandle,
};

use crate::ai::blocklist::{
    ai_brand_color, BlocklistAIActionEvent, BlocklistAIActionModel, BlocklistAIHistoryEvent,
    BlocklistAIHistoryModel,
};
use crate::terminal::event::BlockType;
use crate::terminal::model::session::SessionId;
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::terminal::shell::ShellType;
use crate::terminal::{History, TerminalView};
use crate::ui_components::icons as UIIcon;
use crate::user_config::themes_dir;

const ONBOARDING_BOX_WIDTH: f32 = 210.;
const ONBOARDING_BOX_HEIGHT: f32 = 140.;
const CONTAINER_MARGIN_TOP: f32 = 20.;
const CONTAINER_MARGIN_RIGHT: f32 = 16.;
const KEYBOARD_ICON_SIZE: f32 = 20.;
const INTERIOR_BLOCK_SPACING: f32 = 7.;
const PADDING_INTERIOR: f32 = 16.;
const PADDING_VERTICAL: f32 = 24.;
const PADDING_HORIZONTAL: f32 = 18.;
const TOOL_PATTERNS: [&str; 5] = ["aws", "gcloud", "az", "kubectl", "docker"];

lazy_static! {
    static ref PATH_REGEX: Regex =
        Regex::new(r##"(?i)(?:Set-Location|cd)\s+(?:-\S+\s+)*["']?([^"'\r\n]+)["']?"##)
            .expect("command line path regex invalid");
}

pub struct OnboardingAgenticSuggestionsBlock {
    agent_suggestions: Vec<(AgenticSuggestionsContent, MouseStateHandle)>,
    block_completed: bool,
    terminal_view: WeakViewHandle<TerminalView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnboardingChipType {
    FixAnIssue,
    PullCloudLogs,
    StartAFeature,
    PythonSnakeGame,
    ExploreGitHistory,
    MatrixThemePicker,
    Other,
}

#[derive(Clone)]
pub struct AgenticSuggestionsContent {
    title: String,
    description: String,
    prompt: String,
    chip_type: OnboardingChipType,
    icon: UIIcon::Icon,
}

pub enum OnboardingAgenticSuggestionsBlockEvent {
    RunAgentModeCommand {
        prompt: String,
        chip_type: OnboardingChipType,
    },
}

impl OnboardingAgenticSuggestionsBlock {
    pub fn new(
        session_id: SessionId,
        shell_type: ShellType,
        terminal_view: WeakViewHandle<TerminalView>,
        model_events_handle: ModelHandle<ModelEventDispatcher>,
        action_model: ModelHandle<BlocklistAIActionModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        // Subscribe to AI history events to update button state when conversations change
        let parent_view_id = terminal_view.id();
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            move |_, _, event, ctx| match event {
                BlocklistAIHistoryEvent::StartedNewConversation {
                    terminal_view_id, ..
                }
                | BlocklistAIHistoryEvent::AppendedExchange {
                    terminal_view_id, ..
                }
                | BlocklistAIHistoryEvent::ClearedConversationsInTerminalView {
                    terminal_view_id,
                    ..
                } if *terminal_view_id == parent_view_id => {
                    ctx.notify();
                }
                _ => {}
            },
        );
        ctx.subscribe_to_model(&model_events_handle, |_, _, event, ctx| match event {
            ModelEvent::AfterBlockStarted {
                is_for_in_band_command,
                ..
            } if !is_for_in_band_command => {
                ctx.notify();
            }
            ModelEvent::AfterBlockCompleted(event)
                if matches!(event.block_type, BlockType::User(_)) =>
            {
                ctx.notify();
            }
            _ => {}
        });
        ctx.subscribe_to_model(&action_model, |_, _, event, ctx| {
            if let BlocklistAIActionEvent::FinishedAction { .. } = event {
                ctx.notify();
            }
        });

        let git_repo_dirty_path = Self::find_most_used_git_repository(session_id, shell_type, ctx);
        let git_repo_trimmed = Self::get_git_repo_name(shell_type, git_repo_dirty_path.clone());
        let git_repo_path = Self::reconstruct_path_backwards(
            History::as_ref(ctx)
                .commands(session_id)
                .into_iter()
                .flatten()
                .map(|entry| entry.command.clone())
                .collect(),
            shell_type,
            git_repo_trimmed.clone(),
        );

        let matrix_save_directory = themes_dir()
            .into_os_string()
            .into_string()
            .unwrap_or("the Warp themes directory.".to_string());

        let agent_suggestions = vec![
            (
                AgenticSuggestionsContent {
                    title: "Create a snake game in Python from scratch".to_string(),
                    description: "Have Agent Mode walk you through creating a snake game from end-to-end".to_string(),
                    prompt: "Make a snake game for playing in the terminal using python. Use the code tool and requested commands to do it for me. Before deciding on a solution, make sure I have all the prerequisites installed. At the end of our conversation, the app should run without any additional steps.".to_string(),
                    chip_type: OnboardingChipType::PythonSnakeGame,
                    icon: UIIcon::Icon::GamingPad,
                },
                Default::default(),
            ),
            (
                AgenticSuggestionsContent {
                    title: format!("Explore git history in {git_repo_trimmed}"),
                    description: "Work with Agent Mode to understand recent changes to a git repository".to_string(),
                    prompt: format!("Explore my git history in {git_repo_path} and provide me a summary."),
                    chip_type: OnboardingChipType::ExploreGitHistory,
                    icon: UIIcon::Icon::BookOpen,
                },
                Default::default(),
            ),
            (
                AgenticSuggestionsContent {
                    title: "Create a Matrix-styled custom theme".to_string(),
                    description: "Make your terminal look like you entered the Matrix".to_string(),
                    prompt: format!("First check if {matrix_save_directory} exists, and create this path if it doesn't already exist. Then create a matrix theme for my Warp terminal without a background image field, following exact YAML structure on the warp website without any extra or missing fields. Call it matrix.yaml and save it in the directory we previously created. Once you've verified that the theme is correct and ready to be applied, let me know by only saying 'The matrix theme is now available at <path>.'."),
                    chip_type: OnboardingChipType::MatrixThemePicker,
                    icon: UIIcon::Icon::PaintBrush,
                },
                Default::default(),
            ),
            (
                AgenticSuggestionsContent {
                    title: "Something else?".to_string(),
                    description: "Pair with an Agent to accomplish another task".to_string(),
                    prompt: "What can you help with me on?".to_string(),
                    chip_type: OnboardingChipType::Other,
                    icon: UIIcon::Icon::Stars,
                },
                Default::default(),
            ),
        ];

        Self {
            agent_suggestions,
            block_completed: false,
            terminal_view,
        }
    }

    pub fn interrupt_block(&mut self, ctx: &mut ViewContext<Self>) {
        self.block_completed = true;
        ctx.notify();
    }

    pub fn is_block_completed(&self) -> bool {
        self.block_completed
    }

    /// Check if we can start a new AM block for the user. If we cannot, the
    /// suggestion buttons should be disabled.
    fn can_start_new_am_block(&self, ctx: &AppContext) -> bool {
        let Some(terminal_view) = self.terminal_view.upgrade(ctx) else {
            return false;
        };

        let model = terminal_view.as_ref(ctx).model.lock();
        terminal_view.as_ref(ctx).is_input_box_visible(&model, ctx)
    }

    pub fn split_path(path: &str, shell_type: ShellType) -> Vec<&str> {
        if matches!(shell_type, ShellType::PowerShell) {
            path.split(&['\\', '/']).collect()
        } else {
            path.split('/').collect()
        }
    }

    pub fn reconstruct_path_backwards(
        history: Vec<String>,
        shell_type: ShellType,
        root_level: String,
    ) -> String {
        let mut stack: Vec<String> = vec![root_level.clone()];
        let mut root_level = root_level.clone();

        for action in history {
            if action.to_lowercase().starts_with("cd ")
                || action.to_lowercase().starts_with("set-location")
            {
                let dir = PATH_REGEX
                    .captures(&action)
                    .map(|c| c.get(1).map(|m| m.as_str()).unwrap_or_default())
                    .unwrap_or_default();
                let dir_vec = Self::split_path(dir, shell_type)
                    .into_iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<String>>();
                if dir_vec.contains(&root_level) {
                    let index = dir_vec.iter().position(|s| s == &root_level);
                    if let Some(index) = index {
                        stack.append(
                            &mut dir_vec[..index]
                                .iter()
                                .rev()
                                .filter(|s| *s != "." && !s.is_empty())
                                .cloned()
                                .collect::<Vec<String>>(),
                        );
                        root_level = stack.last().unwrap_or(&root_level).clone();

                        if root_level == "~"
                            || root_level == "$HOME"
                            || root_level.starts_with("/")
                            || root_level.starts_with("C:")
                        {
                            // We found the root directory, we don't need to continue.
                            break;
                        }
                    }
                }
            }
        }

        stack
            .iter()
            .rev()
            .cloned()
            .collect::<Vec<String>>()
            .join(match shell_type {
                ShellType::PowerShell => "\\",
                _ => "/",
            })
    }

    fn find_most_used_git_repository(
        session_id: SessionId,
        shell_type: ShellType,
        ctx: &mut ViewContext<Self>,
    ) -> Option<String> {
        History::as_ref(ctx)
            .commands(session_id)
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(i, entry)| {
                if entry.command.starts_with("git clone") {
                    entry.command.split_whitespace().last().map(|url| {
                        // Find the git repository name from the git clone URL.
                        (
                            Self::split_path(url.trim_end_matches(".git"), shell_type)
                                .last()
                                .cloned()
                                .unwrap_or(url)
                                .to_string(),
                            1,
                        )
                    })
                } else if entry.command.starts_with("gh repo clone") {
                    entry.command.split_whitespace().last().map(|url| {
                        (
                            Self::split_path(url, shell_type)
                                .last()
                                .cloned()
                                .unwrap_or(url)
                                .to_string(),
                            1,
                        )
                    })
                } else if (entry.command.starts_with("git") || entry.command.starts_with("gh"))
                    && i > 0
                {
                    let commands = History::as_ref(ctx)
                        .commands(session_id)
                        .unwrap_or_default();

                    // Find the previous cd command, as this will likely have the git repository name.
                    let previous_cd_command = commands[..i]
                        .iter()
                        .rfind(|entry| {
                            let command = entry.command.to_lowercase();
                            (command.starts_with("cd ")
                                && !command.starts_with("cd ..")
                                && !command.starts_with("cd ."))
                                || (command.starts_with("set-location")
                                    && !command.starts_with("set-location ..")
                                    && !command.starts_with("set-location ."))
                        })
                        .and_then(|cd_entry| {
                            PATH_REGEX
                                .captures(cd_entry.command.as_str())
                                .map(|c| c.get(1).map(|m| m.as_str()).unwrap_or_default())
                        });

                    previous_cd_command.map(|dir| (dir.to_string(), 1))
                } else {
                    None
                }
            })
            .max_by_key(|(_, count)| *count)
            .map(|(dir, _)| dir)
    }

    fn get_git_repo_name(shell_type: ShellType, git_repo_path: Option<String>) -> String {
        Self::split_path(
            &git_repo_path.unwrap_or("my repository".to_string()),
            shell_type,
        )
        .into_iter()
        .rfind(|s| *s != ".." && *s != "." && !s.is_empty())
        .unwrap_or_default()
        .to_string()
    }

    pub fn find_most_used_cloud_tool(
        session_id: SessionId,
        ctx: &mut ViewContext<Self>,
    ) -> Option<String> {
        History::as_ref(ctx)
            .commands(session_id)
            .and_then(|commands| {
                commands
                    .iter()
                    .fold(std::collections::HashMap::new(), |mut counts, entry| {
                        for tool in TOOL_PATTERNS {
                            if entry.command.starts_with(format!("{tool} ").as_str())
                                || entry.command == tool
                            {
                                *counts.entry(tool.to_string()).or_insert(0) += 1;
                            }
                        }
                        counts
                    })
                    .into_iter()
                    .max_by_key(|(_, count)| *count)
                    .map(|(tool, _)| tool)
            })
    }

    pub fn handle_key_pressed(&mut self, key: i32, ctx: &mut ViewContext<Self>) {
        let key_add_by_one = key - 1;
        if key_add_by_one > self.agent_suggestions.len() as i32 {
            return;
        }

        let suggestion = self.agent_suggestions[key_add_by_one as usize].0.clone();
        ctx.emit(
            OnboardingAgenticSuggestionsBlockEvent::RunAgentModeCommand {
                prompt: suggestion.prompt.clone(),
                chip_type: suggestion.chip_type,
            },
        );
        self.block_completed = true;
        ctx.notify();
    }

    fn render_suggestion_button_interior(
        &self,
        appearance: &Appearance,
        title: String,
        description: String,
        index: usize,
        is_disabled: bool,
    ) -> Box<dyn Element> {
        let font_family = appearance.ui_font_family();
        let font_size = appearance.ui_font_size();

        let main_text_color = if is_disabled {
            appearance
                .theme()
                .disabled_text_color(appearance.theme().background())
        } else {
            appearance
                .theme()
                .main_text_color(appearance.theme().background())
        };

        let keyboard_shortcut_style = UiComponentStyles {
            height: Some(KEYBOARD_ICON_SIZE),
            width: Some(KEYBOARD_ICON_SIZE),
            font_color: Some(main_text_color.into_solid()),
            ..Default::default()
        };

        let mut content = Flex::row().with_child(
            ConstrainedBox::new(
                self.agent_suggestions[index]
                    .0
                    .icon
                    .to_warpui_icon(main_text_color)
                    .finish(),
            )
            .with_height(KEYBOARD_ICON_SIZE)
            .with_width(KEYBOARD_ICON_SIZE)
            .finish(),
        );

        if !self.is_block_completed() {
            content = content.with_child(
                appearance
                    .ui_builder()
                    .keyboard_shortcut(
                        &Keystroke::parse(format!("cmdorctrl-{}", index + 1).as_str())
                            .expect("Valid keystroke expected"),
                    )
                    .with_style(keyboard_shortcut_style)
                    .build()
                    .finish(),
            );
        }

        Flex::column()
            .with_child(
                Container::new(
                    Flex::column()
                        .with_children([
                            content
                                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                                .with_main_axis_size(MainAxisSize::Max)
                                .finish(),
                            Container::new(
                                Text::new_inline(title, font_family, font_size)
                                    .with_color(main_text_color.into_solid())
                                    .soft_wrap(true)
                                    .finish(),
                            )
                            .with_margin_top(INTERIOR_BLOCK_SPACING)
                            .finish(),
                            Container::new(
                                Text::new_inline(description, font_family, font_size)
                                    .with_color(if is_disabled {
                                        appearance
                                            .theme()
                                            .disabled_text_color(appearance.theme().background())
                                            .into_solid()
                                    } else {
                                        appearance
                                            .theme()
                                            .sub_text_color(appearance.theme().background())
                                            .into_solid()
                                    })
                                    .soft_wrap(true)
                                    .finish(),
                            )
                            .with_margin_top(INTERIOR_BLOCK_SPACING)
                            .finish(),
                        ])
                        .finish(),
                )
                .with_uniform_padding(PADDING_INTERIOR)
                .finish(),
            )
            .finish()
    }

    fn render_suggestion_button(
        &self,
        appearance: &Appearance,
        content: AgenticSuggestionsContent,
        mouse_state_handle: MouseStateHandle,
        index: usize,
        is_disabled: bool,
    ) -> Box<dyn Element> {
        let col = Flex::column()
            .with_child({
                let button = Hoverable::new(mouse_state_handle.clone(), |state| {
                    let theme = appearance.theme();
                    let background_color = if is_disabled {
                        // Use a more muted background for disabled state
                        theme.surface_1()
                    } else {
                        warp_core::ui::theme::Fill::Solid(internal_colors::neutral_1(theme))
                    };

                    let mut button_content =
                        Container::new(self.render_suggestion_button_interior(
                            appearance,
                            content.title,
                            content.description,
                            index,
                            is_disabled,
                        ))
                        .with_background(background_color)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));

                    if is_disabled {
                        button_content = button_content
                            .with_border(Border::all(1.0).with_border_fill(theme.surface_2()));
                    } else if state.is_hovered() {
                        button_content = button_content.with_border(
                            Border::all(1.0).with_border_fill(theme.accent_button_color()),
                        );
                    } else {
                        button_content = button_content.with_border(
                            Border::all(1.0).with_border_fill(internal_colors::neutral_1(theme)),
                        );
                    };

                    ConstrainedBox::new(button_content.finish())
                        .with_width(ONBOARDING_BOX_WIDTH)
                        .with_min_height(ONBOARDING_BOX_HEIGHT)
                        .finish()
                });

                if is_disabled {
                    button.finish()
                } else {
                    button
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(
                                OnboardingAgenticSuggestionsBlockAction::SuggestionSelected {
                                    prompt: content.prompt.clone(),
                                    chip_type: content.chip_type,
                                },
                            )
                        })
                        .with_cursor(Cursor::PointingHand)
                        .finish()
                }
            })
            .finish();

        Container::new(col)
            .with_margin_top(CONTAINER_MARGIN_TOP)
            .with_margin_right(CONTAINER_MARGIN_RIGHT)
            .finish()
    }

    fn render_text(&self, appearance: &Appearance) -> Box<dyn Element> {
        let current_theme = appearance.theme();
        let font_family = appearance.ui_font_family();
        let font_size = appearance.monospace_font_size();
        let font_color = current_theme.main_text_color(current_theme.background());

        const WELCOME_TEXT_LINE_ONE: &str = "Welcome to Warp!";
        const WELCOME_TEXT_LINE_TWO_PART_ONE: &str =
            "Here are a few examples of how to leverage the power of AI in your terminal using";
        const WELCOME_TEXT_LINE_TWO_PART_TWO: &str = " Agent Mode";

        Flex::column()
            .with_children(vec![
                Container::new(
                    Text::new(WELCOME_TEXT_LINE_ONE, font_family, font_size)
                        .with_color(font_color.into_solid())
                        .finish(),
                )
                .with_margin_bottom(10.)
                .finish(),
                FormattedTextElement::new(
                    FormattedText::new([FormattedTextLine::Line(vec![
                        FormattedTextFragment::plain_text(WELCOME_TEXT_LINE_TWO_PART_ONE),
                        FormattedTextFragment::weighted(
                            WELCOME_TEXT_LINE_TWO_PART_TWO,
                            Some(CustomWeight::Bold),
                        ),
                    ])]),
                    font_size,
                    font_family,
                    font_family,
                    font_color.into(),
                    Default::default(),
                )
                .finish(),
            ])
            .finish()
    }

    fn render_content(
        &self,
        appearance: &Appearance,
        icon_size: f32,
        ctx: &AppContext,
    ) -> Box<dyn Element> {
        let current_theme = appearance.theme();

        if self.agent_suggestions.is_empty() {
            Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        Container::new(
                            ConstrainedBox::new(
                                UIIcon::Icon::Loading
                                    .to_warpui_icon(ai_brand_color(appearance.theme()).into())
                                    .finish(),
                            )
                            .with_height(icon_size)
                            .with_width(icon_size)
                            .finish(),
                        )
                        .with_margin_right(8.)
                        .finish(),
                    )
                    .with_child(
                        Text::new(
                            "Thinking...".to_owned(),
                            appearance.ui_font_family(),
                            appearance.monospace_font_size(),
                        )
                        .with_color(
                            current_theme
                                .hint_text_color(current_theme.background())
                                .into_solid(),
                        )
                        .with_selectable(false)
                        .finish(),
                    )
                    .finish(),
            )
            .with_margin_top(10.)
            .finish()
        } else {
            let is_disabled = !self.can_start_new_am_block(ctx);
            Wrap::row()
                .with_children(self.agent_suggestions.iter().enumerate().map(
                    |(index, (content, mouse_handle))| {
                        self.render_suggestion_button(
                            appearance,
                            content.clone(),
                            mouse_handle.clone(),
                            index,
                            is_disabled,
                        )
                    },
                ))
                .finish()
        }
    }
}

impl Entity for OnboardingAgenticSuggestionsBlock {
    type Event = OnboardingAgenticSuggestionsBlockEvent;
}

impl View for OnboardingAgenticSuggestionsBlock {
    fn ui_name() -> &'static str {
        "OnboardingAgenticSuggestionsBlock"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);
        let current_theme = appearance.theme();
        let font_size = appearance.monospace_font_size();
        let icon_size = font_size + 2.;

        let col = Flex::column().with_children([
            self.render_text(appearance),
            self.render_content(appearance, icon_size, ctx),
        ]);

        Container::new(col.finish())
            .with_horizontal_padding(PADDING_HORIZONTAL)
            .with_vertical_padding(PADDING_VERTICAL)
            .with_border(Border::top(1.0).with_border_fill(current_theme.outline()))
            .with_background(current_theme.surface_2())
            .finish()
    }
}

#[derive(Debug, Clone)]
pub enum OnboardingAgenticSuggestionsBlockAction {
    SuggestionSelected {
        prompt: String,
        chip_type: OnboardingChipType,
    },
}

impl TypedActionView for OnboardingAgenticSuggestionsBlock {
    type Action = OnboardingAgenticSuggestionsBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            OnboardingAgenticSuggestionsBlockAction::SuggestionSelected { prompt, chip_type } => {
                // Don't handle the action if we can't start a new AM block
                if !self.can_start_new_am_block(ctx) {
                    log::error!(
                        "Failed to handle suggestion selected action because we can't start a new AM block"
                    );
                    return;
                }

                self.block_completed = true;
                ctx.emit(
                    OnboardingAgenticSuggestionsBlockEvent::RunAgentModeCommand {
                        prompt: prompt.clone(),
                        chip_type: *chip_type,
                    },
                );
                ctx.notify();
            }
        }
    }
}
