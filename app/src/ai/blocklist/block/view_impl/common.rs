//! Contains UI rendering logic shared between AIBlock and other views that render AIAgentExchanges.

// Renders persistent input-compatible loading animation and text indicator the appropriate message
// based on the request type and status.

use std::borrow::Cow;
#[cfg(feature = "local_fs")]
use std::collections::HashMap;
use std::iter;
use std::path::Path;
#[cfg(feature = "local_fs")]
use std::path::PathBuf;

use itertools::Itertools;
use markdown_parser::{FormattedText, FormattedTextInline, TableAlignment};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use std::sync::Arc;
use warp_core::{
    features::FeatureFlag,
    ui::{appearance::Appearance, color::blend::Blend, theme::color::internal_colors},
};
use warpui::{
    assets::asset_cache::{AssetCache, AssetSource, AssetState},
    elements::{
        new_scrollable::{ScrollableAppearance, SingleAxisConfig},
        Align, Axis, Border, ChildAnchor, ChildView, Clipped, ClippedScrollStateHandle,
        ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DispatchEventResult, Empty,
        EventHandler, Expanded, Fill, Flex, FormattedTextElement, HeadingFontSizeMultipliers,
        Hoverable, Image as WarpImage, MainAxisAlignment, MainAxisSize, MouseStateHandle,
        NewScrollable, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius,
        SavePosition, ScrollTarget, ScrollToPositionMode, ScrollbarWidth, Shrinkable, Stack, Table,
        TableColumnWidth, TableConfig, TableHeader, TableVerticalSizing, Text, Wrap,
    },
    fonts::{Properties, Weight},
    image_cache::{CacheOption, ImageType},
    keymap::Keystroke,
    platform::Cursor,
    text_layout::{ClipConfig, TextAlignment, TextStyle},
    ui_components::{
        button::Button,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    Action, AppContext, Element, EventContext, SingletonEntity, View, ViewHandle,
};

use super::{add_highlights_to_rich_text, add_highlights_to_text, output::LinkActionConstructors};
use crate::ai::agent::MessageId;
use crate::terminal::find::BlockListMatch;
use crate::terminal::grid_renderer::{FOCUSED_MATCH_COLOR, MATCH_COLOR};
use crate::{
    ai::{
        agent::{conversation::AIConversation, icons, ShellCommandDelay},
        blocklist::{
            block::status_bar::BlocklistAIStatusBarAction, history_model::BlocklistAIHistoryModel,
            BlocklistAIActionModel, ShellCommandExecutor,
        },
        loading::shimmering_warp_loading_text,
    },
    terminal::{self, TerminalModel},
    util::link_detection::{add_link_detection_mouse_interactions, DetectedLinksState},
    workspaces::{user_workspaces::UserWorkspaces, workspace::CustomerType},
};
use crate::{
    ai::{
        agent::{
            icons::red_stop_icon, AIAgentAction, AIAgentActionType, AIAgentInput,
            AIAgentOutputMessageType, AIAgentTextSection, AgentOutputImage, AgentOutputImageLayout,
            AgentOutputMermaidDiagram, AgentOutputTable, AgentOutputTableRendering,
            ProgrammingLanguage, RenderableAIError, SummarizationType, UserQueryMode,
            WebSearchStatus,
        },
        blocklist::{
            block::{
                find::FindState, view_impl::CONTENT_HORIZONTAL_PADDING, AIBlockAction,
                CollapsibleElementState, CollapsibleExpansionState, EmbeddedCodeEditorView,
                TableSectionHandles,
            },
            code_block::{
                render_code_block_plain, render_code_block_with_warp_text, CodeBlockOptions,
                CodeSnippetButtonHandles,
            },
            inline_action::{
                aws_bedrock_credentials_error::AwsBedrockCredentialsErrorView,
                inline_action_header::{
                    INLINE_ACTION_HEADER_VERTICAL_PADDING, INLINE_ACTION_HORIZONTAL_PADDING,
                },
                inline_action_icons::{self, icon_size},
                requested_action::RenderableAction,
            },
            model::{AIBlockModel, AIBlockModelHelper},
            secret_redaction::{redact_secrets_in_element, SecretRedactionState},
            view_util::error_color,
            TextLocation,
        },
        AIRequestUsageModel,
    },
    code::{editor::view::CodeEditorView, editor_management::CodeSource},
    notebooks::editor::{markdown_table_appearance, rich_text_styles},
    settings_view::SettingsSection,
    terminal::{
        find::TerminalFindModel, safe_mode_settings::get_secret_obfuscation_mode,
        view::TerminalAction, ShellLaunchData,
    },
    ui_components::{
        avatar::{Avatar, AvatarContent},
        blended_colors,
        buttons::icon_button,
        icons::Icon,
    },
    workspace::WorkspaceAction,
};
use crate::{
    search::slash_command_menu::static_commands::commands,
    settings::{FontSettings, InputSettings},
};
use warp_core::channel::ChannelState;
use warp_editor::content::{
    edit::resolve_asset_source_relative_to_directory, mermaid_diagram::mermaid_asset_source,
};
use warp_util::path::to_relative_path;
use warpui::elements::shimmering_text::ShimmeringTextStateHandle;
use warpui::elements::{Highlight, HighlightedRange};

pub const STATUS_ICON_SIZE_DELTA: f32 = 4.;
pub const STATUS_FOOTER_VERTICAL_PADDING: f32 = 4.;
pub const WAITING_FOR_USER_INPUT_MESSAGE: &str = "Agent waiting for instructions...";
const IMAGE_SOURCE_LINK_LINE_INDEX: usize = 1;

const ERROR_APOLOGY_TEXT: &str = "I'm sorry, I couldn't complete that request.";
const INTERNAL_WARP_ERROR: &str = "Internal Warp error.";

pub const LOAD_OUTPUT_MESSAGE: &str = "Warping...";
pub const LOAD_OUTPUT_MESSAGE_FOR_ADJUSTING: &str = "Adjusting tasks...";
pub const LOAD_OUTPUT_MESSAGE_FOR_PASSIVE_CODE_GEN: &str = "Generating fix...";
pub const LOAD_OUTPUT_MESSAGE_FOR_CREATING_DIFF: &str = "Creating diff...";
pub const LOAD_OUTPUT_MESSAGE_FOR_PREPARING_QUESTION: &str = "Preparing question...";
pub const LOAD_OUTPUT_MESSAGE_FOR_GENERATING_PLAN: &str = "Generating plan...";
pub const LOAD_OUTPUT_MESSAGE_FOR_UPDATING_PLAN: &str = "Updating plan...";
pub const LOAD_OUTPUT_MESSAGE_FOR_SUMMARIZING_CONVERSATION: &str = "Summarizing conversation...";
pub const LOAD_OUTPUT_MESSAGE_FOR_SUMMARIZING_TOOL_CALL_RESULT: &str =
    "Summarizing command output...";
pub const LOAD_OUTPUT_MESSAGE_FOR_SEARCH_CODEBASE: &str = "Searching codebase...";
pub const LOAD_OUTPUT_MESSAGE_FOR_READING_FILES: &str = "Reading files...";
pub const LOAD_OUTPUT_MESSAGE_FOR_GREP: &str = "Grepping...";
pub const LOAD_OUTPUT_MESSAGE_FOR_FILE_GLOB: &str = "Finding files...";
pub const LOAD_OUTPUT_MESSAGE_FOR_RUNNING_COMMAND: &str = "Executing command...";
pub const LOAD_OUTPUT_MESSAGE_FOR_WRITING_TO_COMMAND: &str = "Writing command input...";
pub const LOAD_OUTPUT_MESSAGE_FOR_WAITING_FOR_COMMAND_COMPLETION: &str =
    "Waiting for command to exit...";
pub const LOAD_OUTPUT_MESSAGE_FOR_WEB_SEARCH: &str = "Searching the web...";
pub const LOAD_OUTPUT_MESSAGE_FOR_FETCHING_REVIEW_COMMENTS: &str = "Fetching PR comments...";

#[cfg(feature = "local_fs")]
pub(crate) type ResolvedBlocklistImageSources = HashMap<String, Option<AssetSource>>;

pub const BLOCKED_ACTION_MESSAGE_FOR_WRITE_TO_LONG_RUNNING_SHELL_COMMAND: &str =
    "Can I write the following to this running command?";
pub const BLOCKED_ACTION_MESSAGE_FOR_READING_FILES: &str = "Grant access to the following files?";
pub const BLOCKED_ACTION_MESSAGE_FOR_SEARCHING_CODEBASE: &str =
    "Grant access to the following repository?";
pub const BLOCKED_ACTION_MESSAGE_FOR_GREP_OR_FILE_GLOB: &str =
    "OK if I search the files in this directory?";

const BLOCKLIST_VISUAL_SECTION_HEIGHT_LINE_MULTIPLIER: f32 = 10.0;
const INLINE_IMAGE_HEIGHT: f32 = 164.;
const INLINE_IMAGE_MAX_WIDTH: f32 = 218.;
const INLINE_IMAGE_SPACING: f32 = 32.;
const INLINE_IMAGE_RUN_SPACING: f32 = 16.;
const BLOCK_IMAGE_THUMBNAIL_SIZE: f32 = 124.;
const BLOCK_IMAGE_ROW_SPACING: f32 = 12.;
const BLOCK_IMAGE_ROW_CONTENT_SPACING: f32 = 24.;
const VISUAL_CARD_CORNER_RADIUS: f32 = 8.;
const VISUAL_CARD_HEADER_VERTICAL_PADDING: f32 = 8.;
const VISUAL_CARD_HEADER_HORIZONTAL_PADDING: f32 = 16.;
const MERMAID_CANVAS_PADDING: f32 = 32.;

pub struct WarpingProps<'a, V> {
    pub model: &'a dyn AIBlockModel<View = V>,
    pub shimmering_text_handle: &'a ShimmeringTextStateHandle,
    pub summarization_start_time: Option<instant::Instant>,
    pub hide_responses_button: Option<(ButtonProps<'a>, bool)>,
    pub take_over_lrc_control_button: Option<ButtonProps<'a>>,
    pub auto_execute_button: Option<ButtonProps<'a>>,
    pub queue_next_prompt_button: Option<ButtonProps<'a>>,
    pub stop_button: Option<ButtonProps<'a>>,
    /// Inline `Check now` affordance displayed alongside `Last seen by agent ...`
    /// in the warping indicator. When set, the agent's pending poll future is
    /// short-circuited on click and a fresh snapshot is returned immediately.
    pub force_refresh_button: Option<ForceRefreshButtonProps<'a>>,
    pub action_model: &'a BlocklistAIActionModel,
    pub terminal_model: &'a TerminalModel,
    pub default_warping_text: String,
    pub secondary_element: Option<Box<dyn Element>>,
    /// When an LRC subagent has sent at least one snapshot, the timestamp of the most recent snapshot.
    pub last_snapshot_at: Option<instant::Instant>,
}

pub struct ButtonProps<'a> {
    pub button_handle: &'a MouseStateHandle,
    pub keystroke: Option<&'a Keystroke>,
    pub is_active: bool,
}

pub struct ForceRefreshButtonProps<'a> {
    pub button_handle: &'a MouseStateHandle,
    /// The block the force-refresh should target.
    pub block_id: crate::terminal::model::block::BlockId,
}

pub fn render_warping_indicator<V: View>(
    props: WarpingProps<'_, V>,
    app: &AppContext,
) -> Box<dyn Element> {
    let output_status = props.model.status(app);
    let output_to_render = output_status.output_to_render();

    // `true` if the input for this block's exchange was sent in the middle of the previous
    // exchange's output, i.e. interrupting it.
    let is_interrupt_query_for_same_conversation = props
        .model
        .exchange_id(app)
        .and_then(|exchange_id| {
            props
                .model
                .conversation(app)
                .and_then(|conversation| conversation.previous_exchange(&exchange_id))
        })
        .is_some_and(|previous_exchange| {
            previous_exchange
                .output_status
                .cancel_reason()
                .is_some_and(|r| r.is_follow_up_for_same_conversation())
        });

    let is_last_message_requesting_file_edits = output_to_render.as_ref().is_some_and(|output| {
        let output = output.get();
        output.messages.last().is_some_and(|m| {
            matches!(
                m.message,
                AIAgentOutputMessageType::Action(AIAgentAction {
                    action: AIAgentActionType::RequestFileEdits { .. },
                    ..
                })
            )
        })
    });

    let is_last_message_asking_user_question = output_to_render.as_ref().is_some_and(|output| {
        let output = output.get();
        output.messages.last().is_some_and(|m| {
            matches!(
                m.message,
                AIAgentOutputMessageType::Action(AIAgentAction {
                    action: AIAgentActionType::AskUserQuestion { .. },
                    ..
                })
            )
        })
    });
    let is_searching_web = output_to_render.as_ref().is_some_and(|output| {
        output.get().messages.last().is_some_and(|m| {
            matches!(
                m.message,
                AIAgentOutputMessageType::WebSearch(WebSearchStatus::Searching { .. })
            )
        })
    });

    let is_fetching_review_comments = props
        .model
        .inputs_to_render(app)
        .iter()
        .any(|input| matches!(input, AIAgentInput::FetchReviewComments { .. }));

    let summarization_type: Option<SummarizationType> =
        if FeatureFlag::SummarizationCancellationConfirmation.is_enabled() {
            output_to_render.as_ref().and_then(|output| {
                let output = output.get();
                output.messages.last().and_then(|m| {
                    if let AIAgentOutputMessageType::Summarization {
                        finished_duration: None,
                        summarization_type,
                        ..
                    } = m.message
                    {
                        Some(summarization_type)
                    } else {
                        None
                    }
                })
            })
        } else {
            None
        };

    let mut should_render_waiting_icon = false;
    let mut non_shimmering_text = None;
    let message = if let Some(summarization_type) = summarization_type {
        // Choose the appropriate message based on summarization type
        let base_message = match summarization_type {
            SummarizationType::ConversationSummary => {
                LOAD_OUTPUT_MESSAGE_FOR_SUMMARIZING_CONVERSATION
            }
            SummarizationType::ToolCallResultSummary => {
                LOAD_OUTPUT_MESSAGE_FOR_SUMMARIZING_TOOL_CALL_RESULT
            }
        };

        // Only show duration for conversation summarization, not tool call result
        // summarization
        if matches!(summarization_type, SummarizationType::ConversationSummary) {
            let timer_text = if let Some(start_time) = props.summarization_start_time {
                format!(" • {}", format_elapsed_seconds(start_time.elapsed()))
            } else {
                String::new()
            };

            // Move the timer / token text outside of the base message, we don't want it to shimmer
            // since that would cause the animation to reset every time the tokens or time changes.
            non_shimmering_text = Some(timer_text.to_string());
            base_message.into()
        } else {
            base_message.to_string()
        }
    } else if props.model.contains_update_document_action(app) {
        LOAD_OUTPUT_MESSAGE_FOR_UPDATING_PLAN.to_string()
    } else if props.model.contains_create_document_action(app) {
        LOAD_OUTPUT_MESSAGE_FOR_GENERATING_PLAN.to_string()
    } else if props.model.request_type(app).is_passive_code_diff() {
        LOAD_OUTPUT_MESSAGE_FOR_PASSIVE_CODE_GEN.to_string()
    } else if is_last_message_requesting_file_edits {
        LOAD_OUTPUT_MESSAGE_FOR_CREATING_DIFF.to_string()
    } else if is_last_message_asking_user_question {
        LOAD_OUTPUT_MESSAGE_FOR_PREPARING_QUESTION.to_string()
    } else if is_searching_web {
        LOAD_OUTPUT_MESSAGE_FOR_WEB_SEARCH.to_string()
    } else if is_fetching_review_comments {
        LOAD_OUTPUT_MESSAGE_FOR_FETCHING_REVIEW_COMMENTS.to_string()
    } else if is_interrupt_query_for_same_conversation
        && output_to_render
            .as_ref()
            .is_none_or(|output| output.get().messages.is_empty())
    {
        // Only "Adjusting..." if nothing from the current exchange has streamed yet.
        LOAD_OUTPUT_MESSAGE_FOR_ADJUSTING.to_string()
    } else {
        match props
            .action_model
            .get_async_running_action(app)
            .map(|action| &action.action)
        {
            Some(AIAgentActionType::SearchCodebase(..)) => {
                LOAD_OUTPUT_MESSAGE_FOR_SEARCH_CODEBASE.to_owned()
            }
            Some(AIAgentActionType::Grep { .. }) => LOAD_OUTPUT_MESSAGE_FOR_GREP.to_owned(),
            Some(AIAgentActionType::CallMCPTool { name, .. }) => {
                format!("Calling \"{name}\" MCP tool...")
            }
            Some(AIAgentActionType::ReadMCPResource { name, .. }) => {
                format!("Reading \"{name}\" MCP resource...")
            }
            Some(AIAgentActionType::FileGlob { .. })
            | Some(AIAgentActionType::FileGlobV2 { .. }) => {
                LOAD_OUTPUT_MESSAGE_FOR_FILE_GLOB.to_owned()
            }
            Some(AIAgentActionType::WriteToLongRunningShellCommand { .. }) => {
                LOAD_OUTPUT_MESSAGE_FOR_WRITING_TO_COMMAND.to_owned()
            }
            action => {
                let active_block = props.terminal_model.block_list().active_block();
                if !props.model.status(app).is_streaming()
                    && active_block.is_active_and_long_running()
                    && active_block.agent_interaction_metadata().is_some()
                {
                    if action.is_none() {
                        should_render_waiting_icon = true;
                        WAITING_FOR_USER_INPUT_MESSAGE.to_owned()
                    } else {
                        // Choose the base message depending on whether the agent is waiting
                        // for the command to exit or polling at a fixed interval.
                        let base = match action {
                            Some(AIAgentActionType::ReadShellCommandOutput {
                                delay: Some(ShellCommandDelay::OnCompletion),
                                ..
                            }) => LOAD_OUTPUT_MESSAGE_FOR_WAITING_FOR_COMMAND_COMPLETION,
                            _ => LOAD_OUTPUT_MESSAGE_FOR_RUNNING_COMMAND,
                        };
                        // Compute "Next check in {time}" for fixed-interval polls. Only
                        // `ReadShellCommandOutput { delay: Duration(_) }` has a meaningful
                        // countdown; `OnCompletion` is a safety cap rather than a poll
                        // interval, and the 2s default is too short to be useful.
                        let next_check_remaining = match action {
                            Some(AIAgentActionType::ReadShellCommandOutput {
                                delay: Some(ShellCommandDelay::Duration(d)),
                                ..
                            }) => props.last_snapshot_at.and_then(|last_snapshot_at| {
                                let capped =
                                    (*d).min(ShellCommandExecutor::MAX_AGENT_DELAY_DURATION);
                                let remaining = capped.saturating_sub(last_snapshot_at.elapsed());
                                // Hide the suffix once less than a whole second remains so the
                                // indicator disappears after the "1s" tick.
                                (remaining.as_secs() > 0).then_some(remaining)
                            }),
                            _ => None,
                        };
                        if let Some(remaining) = next_check_remaining {
                            let secs = remaining.as_secs();
                            let formatted = if secs < 60 {
                                format!("{secs}s")
                            } else {
                                format!("{}m", secs / 60)
                            };
                            let suffix = format!(" · Next check in {formatted}");

                            // Keep the base message constant so the shimmering animation
                            // isn't interrupted every time the countdown ticks. The
                            // suffix is rendered as a separate non-shimmering element,
                            // matching the same pattern used by the summarization timer.
                            non_shimmering_text = Some(suffix);
                            base.to_owned()
                        } else {
                            base.to_owned()
                        }
                    }
                } else {
                    props.default_warping_text.clone()
                }
            }
        }
    };

    let appearance = Appearance::as_ref(app);

    let mut buttons_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
    let mut has_buttons = false;
    if let Some((hide_responses_button_props, should_hide_responses)) = props.hide_responses_button
    {
        has_buttons = true;
        buttons_row.add_child(render_hide_responses_button(
            hide_responses_button_props,
            should_hide_responses,
            appearance,
        ));
    }

    if let Some(take_over_button_props) = props.take_over_lrc_control_button {
        has_buttons = true;
        buttons_row.add_child(render_switch_control_to_user_button(
            "Take over",
            "Take over control of the command",
            take_over_button_props,
            appearance,
        ));
    }

    if let Some(autoexecute_button_props) = props.auto_execute_button {
        has_buttons = true;
        buttons_row.add_child(render_auto_approve_button(
            autoexecute_button_props,
            appearance,
        ));
    }

    if let Some(queue_button_props) = props.queue_next_prompt_button {
        has_buttons = true;
        buttons_row.add_child(render_queue_next_prompt_button(
            queue_button_props,
            appearance,
        ));
    }

    if let Some(stop_button_props) = props.stop_button {
        has_buttons = true;
        buttons_row = buttons_row
            .with_child(render_stop_button(stop_button_props, appearance))
            .with_spacing(4.);
    }

    let warping_indicator_text = if !should_render_waiting_icon {
        MaybeShimmeringText::Shimmering {
            text: message.into(),
            shimmering_text_handle: props.shimmering_text_handle.clone(),
        }
    } else {
        MaybeShimmeringText::Static(message.into())
    };

    // Only render `Check now` when we also have non-shimmering text, since that's the
    // row we're appending to. This naturally scopes the affordance to situations where
    // `Last seen by agent ...` is visible.
    let non_shimmering_suffix = match (&non_shimmering_text, props.force_refresh_button) {
        (Some(_), Some(force_refresh_button_props)) => Some(render_force_refresh_inline(
            force_refresh_button_props,
            appearance,
        )),
        _ => None,
    };

    render_warping_indicator_base(
        WarpingIndicatorProps {
            icon: should_render_waiting_icon.then(|| icons::gray_clock_icon(appearance).finish()),
            warping_indicator_text,
            non_shimmering_text,
            non_shimmering_suffix,
            buttons: if has_buttons {
                Some(buttons_row.finish())
            } else {
                None
            },
            is_passive_code_diff: props.model.request_type(app).is_passive_code_diff(),
            secondary_element: props.secondary_element,
        },
        app,
    )
}

pub enum MaybeShimmeringText {
    Static(Cow<'static, str>),
    Shimmering {
        text: Cow<'static, str>,
        shimmering_text_handle: ShimmeringTextStateHandle,
    },
}

pub struct WarpingIndicatorProps {
    pub icon: Option<Box<dyn Element>>,
    pub warping_indicator_text: MaybeShimmeringText,
    pub non_shimmering_text: Option<String>,
    /// Optional element rendered inline to the right of `non_shimmering_text`. Used
    /// today for the `Check now` affordance next to `Last seen by agent ...`.
    pub non_shimmering_suffix: Option<Box<dyn Element>>,
    pub buttons: Option<Box<dyn Element>>,
    pub is_passive_code_diff: bool,
    pub secondary_element: Option<Box<dyn Element>>,
}

/// Helper function to render text in the "warping..." footer.
/// Additional text that does not use the shimmering text animation can be passed in via
/// `non_shimmering_text` which is useful if you want some part of the text to constantly update
/// without the animation resetting.
pub fn render_warping_indicator_base(
    props: WarpingIndicatorProps,
    app: &AppContext,
) -> Box<dyn Element> {
    let WarpingIndicatorProps {
        icon,
        warping_indicator_text,
        non_shimmering_text,
        non_shimmering_suffix,
        buttons,
        is_passive_code_diff,
        secondary_element,
    } = props;
    // Unicode code point for the Warp glyph that is embedded in the version of Roboto we bundle
    // into the app. This code point MUST be rendered using Roboto (the default ui font) or else the
    // glyph may not be rendered.
    const WARP_GLYPH: &str = "\u{E500}";

    let appearance = Appearance::as_ref(app);

    let should_indent_tip_for_warp_glyph = matches!(
        warping_indicator_text,
        MaybeShimmeringText::Shimmering { .. }
    );

    let text = render_output_status_text(warping_indicator_text, appearance, app);

    let text_content = {
        let mut row = Flex::row().with_child(Shrinkable::new(1., text).finish());

        if let Some(non_shimmering) = non_shimmering_text {
            let additional = render_output_status_text(
                MaybeShimmeringText::Static(non_shimmering.into()),
                appearance,
                app,
            );
            row = row.with_child(Shrinkable::new(1., additional).finish());
        }

        if let Some(suffix) = non_shimmering_suffix {
            row = row.with_child(Shrinkable::new(1., suffix).finish());
        }

        row.finish()
    };

    let mut text_col = Flex::column();
    if let Some(sub_element) = secondary_element {
        // Our warping indicator text prepends the Warp glyph (and a space) to the label.
        // If we render the tip directly underneath, it will align to the glyph instead of
        // the start of the actual warping text.
        let sub_element = if should_indent_tip_for_warp_glyph {
            let font_size = appearance.monospace_font_size() - 3.;
            let glyph_indent = Text::new_inline(
                format!("{WARP_GLYPH} "),
                appearance.ui_font_family(),
                font_size,
            )
            .with_color(ColorU::new(0, 0, 0, 0))
            .with_selectable(false)
            .soft_wrap(false)
            .finish();

            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(glyph_indent)
                .with_child(Shrinkable::new(1., sub_element).finish())
                .finish()
        } else {
            Shrinkable::new(1., sub_element).finish()
        };

        text_col = text_col
            .with_child(text_content)
            .with_child(Container::new(sub_element).with_margin_top(1.).finish());
    } else if FeatureFlag::AgentTips.is_enabled() && *InputSettings::as_ref(app).show_agent_tips {
        text_col = text_col.with_child(text_content);
    } else {
        text_col = text_col.with_child(
            Container::new(text_content)
                .with_margin_bottom(14.)
                .finish(),
        );
    }

    let mut row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_spacing(6.);

    if let Some(icon) = icon {
        row = row.with_child(
            ConstrainedBox::new(icon)
                .with_width(icon_size(app) - STATUS_ICON_SIZE_DELTA)
                .with_height(icon_size(app) - STATUS_ICON_SIZE_DELTA)
                .finish(),
        );
    }

    row = row.with_child(Expanded::new(1., text_col.finish()).finish());

    if let Some(buttons) = buttons {
        row = row.with_child(buttons);
    }

    let content = Clipped::new(row.finish()).finish();

    if is_passive_code_diff {
        Container::new(content)
            // Use custom padding for the passive code diff block
            .with_padding_top(8.)
            .with_padding_bottom(4.)
            .with_horizontal_padding(CONTENT_HORIZONTAL_PADDING)
            .finish()
    } else {
        let mut container = Container::new(
            ConstrainedBox::new(content)
                .with_height(STATUS_FOOTER_VERTICAL_PADDING * 2. + appearance.monospace_font_size())
                .finish(),
        )
        .with_padding_right(CONTENT_HORIZONTAL_PADDING);

        if FeatureFlag::AgentView.is_enabled() {
            container = container.with_padding_left(*terminal::view::PADDING_LEFT);
        } else {
            container = container
                .with_padding_left(CONTENT_HORIZONTAL_PADDING + (STATUS_ICON_SIZE_DELTA / 2.));
        }

        container.finish()
    }
}

/// Formats elapsed time as a human-readable string with proper singular/plural.
pub fn format_elapsed_seconds(elapsed: std::time::Duration) -> String {
    let total_seconds = elapsed.as_secs();
    if total_seconds == 1 {
        "1 second".to_string()
    } else {
        format!("{total_seconds} seconds")
    }
}

/// Render output text as shown in the "stopped" and "loading" status banners
pub fn render_output_status_text(
    label: MaybeShimmeringText,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let internal_element = match label {
        MaybeShimmeringText::Static(text) => {
            let sub_text_color =
                blended_colors::text_sub(appearance.theme(), appearance.theme().surface_1());
            Text::new(
                text,
                appearance.ui_font_family(),
                appearance.monospace_font_size() - 2.,
            )
            .with_color(sub_text_color)
            .with_style(Properties::default())
            .with_clip(ClipConfig::end())
            .with_selectable(false)
            .soft_wrap(false)
            .finish()
        }
        MaybeShimmeringText::Shimmering {
            text,
            shimmering_text_handle,
        } => shimmering_warp_loading_text(
            text.to_string(),
            appearance.monospace_font_size() - 2.,
            shimmering_text_handle,
            app,
        ),
    };

    Container::new(internal_element)
        .with_margin_top(1.)
        .finish()
}

struct ImageSourceLinkProps<'a> {
    source: &'a str,
    section_index: usize,
    detected_links: Option<&'a DetectedLinksState>,
    secret_redaction_state: &'a SecretRedactionState,
    is_ai_input_enabled: bool,
    is_selecting_text: bool,
    soft_wrap: bool,
}

fn render_image_source_link(props: ImageSourceLinkProps<'_>, app: &AppContext) -> Box<dyn Element> {
    render_text_section_with_options(
        TextSectionProps {
            text: props.source,
            text_color: blended_colors::text_sub(
                Appearance::as_ref(app).theme(),
                Appearance::as_ref(app).theme().surface_1(),
            ),
            is_ai_input_enabled: props.is_ai_input_enabled,
            section_index: props.section_index,
            line_index: IMAGE_SOURCE_LINK_LINE_INDEX,
            secret_redaction_state: props.secret_redaction_state,
            detected_links: props.detected_links,
            find_context: None,
            is_selecting_text: props.is_selecting_text,
        },
        props.soft_wrap,
        (!props.soft_wrap).then_some(ClipConfig::end()),
        app,
    )
}

fn render_hide_responses_button(
    props: ButtonProps,
    should_hide_responses: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let button_text = if should_hide_responses {
        "Show responses"
    } else {
        "Hide responses"
    };
    let text = Container::new(
        Text::new(
            button_text,
            appearance.ui_font_family(),
            get_keybinding_font_size(appearance),
        )
        .with_color(theme.foreground().into())
        .finish(),
    )
    .finish();

    let tooltip_text = if should_hide_responses {
        "Show agent responses"
    } else {
        "Hide agent responses"
    };

    render_warping_indicator_button(
        props.button_handle.clone(),
        appearance,
        text,
        props.keystroke,
        tooltip_text.to_string(),
        props.is_active,
        |ctx| {
            ctx.dispatch_typed_action(BlocklistAIStatusBarAction::ToggleHideResponses);
        },
    )
}

pub fn render_switch_control_to_user_button(
    text: &'static str,
    tooltip: &'static str,
    props: ButtonProps,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let text = Container::new(
        Text::new(
            text,
            appearance.ui_font_family(),
            get_keybinding_font_size(appearance),
        )
        .with_color(theme.foreground().into())
        .finish(),
    )
    .finish();

    render_warping_indicator_button(
        props.button_handle.clone(),
        appearance,
        text,
        props.keystroke,
        tooltip.to_string(),
        props.is_active,
        |ctx| {
            ctx.dispatch_typed_action(TerminalAction::SetInputModeTerminal);
        },
    )
}

fn render_stop_button(props: ButtonProps, appearance: &Appearance) -> Box<dyn Element> {
    let icon_size = get_icon_size(appearance);
    let stop_icon = Container::new(
        ConstrainedBox::new(red_stop_icon(appearance).finish())
            .with_height(icon_size)
            .with_width(icon_size)
            .finish(),
    )
    .finish();

    render_warping_indicator_button(
        props.button_handle.clone(),
        appearance,
        stop_icon,
        props.keystroke,
        "Stop agent task".to_string(),
        props.is_active,
        |ctx: &mut EventContext<'_>| {
            ctx.dispatch_typed_action(BlocklistAIStatusBarAction::Stop);
        },
    )
}

fn render_queue_next_prompt_button(
    props: ButtonProps,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let icon_color = if props.is_active {
        appearance.theme().accent()
    } else {
        appearance.theme().disabled_ui_text_color()
    };
    let icon_size = get_icon_size(appearance);
    let icon = Container::new(
        ConstrainedBox::new(Icon::ClockPlus.to_warpui_icon(icon_color).finish())
            .with_height(icon_size)
            .with_width(icon_size)
            .finish(),
    )
    .finish();

    let tooltip_text = if props.is_active {
        "Auto-queue is on: your next prompt will be queued"
    } else {
        "Auto-queue next prompt while agent is responding"
    };

    render_warping_indicator_button(
        props.button_handle.clone(),
        appearance,
        icon,
        props.keystroke,
        tooltip_text.to_string(),
        props.is_active,
        |ctx| {
            ctx.dispatch_typed_action(TerminalAction::ToggleQueueNextPrompt);
        },
    )
}

fn render_auto_approve_button(props: ButtonProps, appearance: &Appearance) -> Box<dyn Element> {
    let icon = if props.is_active {
        Icon::FastForwardFilled
    } else {
        Icon::FastForward
    };
    let icon_size = get_icon_size(appearance);
    let icon = Container::new(
        ConstrainedBox::new(
            icon.to_warpui_icon(appearance.theme().active_ui_text_color())
                .finish(),
        )
        .with_height(icon_size)
        .with_width(icon_size)
        .finish(),
    )
    .finish();

    let tooltip_text = if props.is_active {
        "Turn off auto-approve all agent actions"
    } else {
        "Auto-approve all agent actions for this task"
    };

    render_warping_indicator_button(
        props.button_handle.clone(),
        appearance,
        icon,
        props.keystroke,
        tooltip_text.to_string(),
        props.is_active,
        |ctx| {
            ctx.dispatch_typed_action(TerminalAction::ToggleAutoexecuteMode);
        },
    )
}

fn get_keybinding_font_size(appearance: &Appearance) -> f32 {
    appearance.ui_font_size() - 1.
}

fn get_icon_size(appearance: &Appearance) -> f32 {
    appearance.ui_font_size() + 1.
}

/// Renders the inline `Check now` affordance displayed alongside
/// `Last seen by agent ...` in the warping indicator. On click, short-circuits the
/// agent's pending poll timer for the given block and delivers a fresh snapshot.
fn render_force_refresh_inline(
    props: ForceRefreshButtonProps<'_>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let ui_builder = appearance.ui_builder().clone();
    let sub_text_color = blended_colors::text_sub(theme, theme.surface_1());
    let hovered_text_color: ColorU = theme.foreground().into();
    let font_family = appearance.ui_font_family();
    let font_size = appearance.monospace_font_size() - 2.;
    let block_id = props.block_id;
    let block_id_for_click = block_id.clone();

    Hoverable::new(props.button_handle.clone(), move |state| {
        let color = if state.is_hovered() {
            hovered_text_color
        } else {
            sub_text_color
        };

        // Mirror `render_output_status_text` exactly: same `Text` configuration plus
        // the `Container::with_margin_top(1.)` wrapper so this sits on the same
        // baseline as the adjacent `Last seen by agent ...` text.
        let text = Text::new(" · Check now".to_string(), font_family, font_size)
            .with_color(color)
            .with_style(Properties::default())
            .with_clip(ClipConfig::end())
            .with_selectable(false)
            .soft_wrap(false)
            .finish();
        let text_with_margin = Container::new(text).with_margin_top(1.).finish();

        // Tooltip overlay, positioned above the element on hover. Same pattern as
        // `render_ai_follow_up_icon` in `view_util.rs`.
        let mut stack = Stack::new().with_child(text_with_margin);
        if state.is_hovered() {
            let tool_tip = ui_builder
                .tool_tip("Ask the agent to check this command now, skipping its timer.".to_owned())
                .build()
                .finish();
            stack.add_positioned_overlay_child(
                tool_tip,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., -4.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopLeft,
                    ChildAnchor::BottomLeft,
                ),
            );
        }
        stack.finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| {
        ctx.dispatch_typed_action(BlocklistAIStatusBarAction::ForceRefreshAgentView {
            block_id: block_id_for_click.clone(),
        });
    })
    .finish()
}

fn render_warping_indicator_button<F>(
    mouse_state: MouseStateHandle,
    appearance: &Appearance,
    content: Box<dyn Element>,
    keybinding: Option<&Keystroke>,
    tooltip: String,
    is_active: bool,
    mut on_click: F,
) -> Box<dyn Element>
where
    F: 'static + FnMut(&mut EventContext),
{
    let theme = appearance.theme();
    let ui_builder = appearance.ui_builder().clone();

    let mut button_content = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(content)
        .with_spacing(4.0);

    if !warpui::platform::is_mobile_device() {
        let keybinding_string = keybinding.map(|k| k.displayed()).unwrap_or_default();
        let keybinding_label = Text::new_inline(
            keybinding_string,
            appearance.ui_font_family(),
            get_keybinding_font_size(appearance),
        )
        .with_color(theme.foreground().into())
        .finish();

        button_content.add_child(keybinding_label);
    }

    let button_content = button_content.finish();
    let styles = UiComponentStyles::default()
        .set_border_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .set_border_width(1.)
        .set_border_color(internal_colors::neutral_4(theme).into())
        .set_padding(Coords {
            top: 2.,
            bottom: 2.,
            left: 4.,
            right: 4.,
        });

    let hovered_styles = styles.merge(
        UiComponentStyles::default().set_background(internal_colors::fg_overlay_2(theme).into()),
    );

    let active_styles = styles.merge(
        UiComponentStyles::default().set_background(internal_colors::fg_overlay_3(theme).into()),
    );

    let mut button = Button::new(
        mouse_state,
        styles,
        Some(hovered_styles),
        Some(active_styles),
        None,
    )
    .with_custom_label(button_content)
    .with_tooltip(move || ui_builder.tool_tip(tooltip.clone()).build().finish())
    .with_cursor(Some(Cursor::PointingHand));

    if is_active {
        button = button.active();
    }

    button
        .build()
        .on_click(move |ctx, _, _| {
            on_click(ctx);
        })
        .finish()
}

pub struct TextSectionsProps<'a, V, A: 'static> {
    pub model: &'a dyn AIBlockModel<View = V>,
    pub starting_text_section_index: &'a mut usize,
    pub starting_code_section_index: &'a mut usize,
    pub starting_table_section_index: &'a mut usize,
    pub starting_image_section_index: &'a mut usize,
    pub sections: &'a [AIAgentTextSection],
    pub selectable: bool,
    pub text_color: ColorU,
    pub is_ai_input_enabled: bool,
    pub find_context: Option<FindContext<'a>>,
    pub shell_launch_data: Option<&'a ShellLaunchData>,
    pub current_working_directory: Option<&'a String>,
    pub embedded_code_editor_views: &'a [EmbeddedCodeEditorView],
    pub code_snippet_button_handles: &'a [CodeSnippetButtonHandles],
    pub table_section_handles: &'a [TableSectionHandles],
    /// Per-image persistent `MouseStateHandle`s, one per `AIAgentTextSection::Image`
    /// in source order. Pre-allocated on `AIBlockStateHandles` so the tooltip
    /// `Hoverable` sees the same handle across frames and `is_hovered()` can
    /// actually latch.
    pub image_section_tooltip_handles: &'a [MouseStateHandle],
    pub open_code_block_action_factory: Option<OpenCodeBlockActionFactory<A>>,
    pub copy_code_action_factory: Option<CopyCodeActionFactory<A>>,
    pub detected_links: Option<&'a DetectedLinksState>,
    pub secret_redaction_state: &'a SecretRedactionState,
    pub is_selecting_text: bool,
    pub item_spacing: f32,
    #[cfg(feature = "local_fs")]
    pub resolved_code_block_paths: Option<&'a HashMap<PathBuf, Option<PathBuf>>>,
    #[cfg(feature = "local_fs")]
    pub resolved_blocklist_image_sources: Option<&'a ResolvedBlocklistImageSources>,
}

pub fn render_text_sections<V: View, A: Action>(
    props: TextSectionsProps<'_, V, A>,
    app: &AppContext,
) -> Box<dyn Element> {
    let starting_text_section_index = *props.starting_text_section_index;
    *props.starting_text_section_index += props.sections.len();
    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(props.item_spacing);
    let indexed_sections =
        text_sections_with_indices(props.sections, starting_text_section_index).collect_vec();
    let lightbox_collection = collect_visual_markdown_lightbox_collection(
        &indexed_sections,
        props.current_working_directory,
        #[cfg(feature = "local_fs")]
        props.resolved_blocklist_image_sources,
        app,
    );
    let mut indexed_section_offset = 0;
    while indexed_section_offset < indexed_sections.len() {
        let (section_index, section) = indexed_sections[indexed_section_offset];
        if let AIAgentTextSection::Image { image } = section {
            let image_group = collect_renderable_image_group(
                &indexed_sections,
                indexed_section_offset,
                image.layout.clone(),
                props.current_working_directory,
                #[cfg(feature = "local_fs")]
                props.resolved_blocklist_image_sources,
                app,
            );
            if !image_group.images.is_empty() {
                // Carve off a subslice of the pre-allocated handles for just
                // this group, and advance the counter so subsequent image
                // sections pick up handles from later in the slice.
                let handle_start = *props.starting_image_section_index;
                let handle_end = (handle_start + image_group.images.len())
                    .min(props.image_section_tooltip_handles.len());
                let handles_for_group =
                    &props.image_section_tooltip_handles[handle_start..handle_end];
                *props.starting_image_section_index += image_group.images.len();
                let render_context = ImageRenderContext {
                    detected_links: props.detected_links,
                    secret_redaction_state: props.secret_redaction_state,
                    is_ai_input_enabled: props.is_ai_input_enabled,
                    is_selecting_text: props.is_selecting_text,
                    copy_action_factory: props.copy_code_action_factory,
                    current_working_directory: props.current_working_directory,
                    #[cfg(feature = "local_fs")]
                    resolved_blocklist_image_sources: props.resolved_blocklist_image_sources,
                    lightbox_collection: &lightbox_collection,
                    tooltip_mouse_state_handles: handles_for_group,
                };
                match image.layout {
                    AgentOutputImageLayout::Inline => {
                        column.add_child(render_inline_image_section_group(
                            &image_group.images,
                            &render_context,
                            app,
                        ));
                    }
                    AgentOutputImageLayout::Block => {
                        column.add_child(render_block_image_section_group(
                            &image_group.images,
                            &render_context,
                            app,
                        ));
                    }
                }
                indexed_section_offset += image_group.consumed_sections;
                continue;
            }
        }
        match section {
            AIAgentTextSection::PlainText { text } => {
                if text.text().trim().is_empty() {
                    indexed_section_offset += 1;
                    continue;
                }

                match &text.formatted_lines {
                    Some(formatted_text) => {
                        column.add_child(render_rich_text_output_text_section(
                            RichTextSectionProps {
                                is_restored: props.model.is_restored(),
                                formatted_text: formatted_text.formatted_text_arc(),
                                text_color: props.text_color,
                                is_ai_input_enabled: props.is_ai_input_enabled,
                                section_index,
                                find_context: props.find_context,
                                secret_redaction_state: props.secret_redaction_state,
                                detected_links: props.detected_links,
                                is_selecting_text: props.is_selecting_text,
                                selectable: props.selectable,
                            },
                            app,
                        ));
                    }
                    _ => column.add_child(render_text_section(
                        TextSectionProps {
                            text: text.text(),
                            text_color: props.text_color,
                            is_ai_input_enabled: props.is_ai_input_enabled,
                            section_index,
                            line_index: 0,
                            secret_redaction_state: props.secret_redaction_state,
                            detected_links: props.detected_links,
                            find_context: props.find_context,
                            is_selecting_text: props.is_selecting_text,
                        },
                        app,
                    )),
                }
            }
            AIAgentTextSection::Code {
                code,
                language,
                source,
            } => {
                let editor_view = props
                    .embedded_code_editor_views
                    .get(*props.starting_code_section_index)
                    .map(|view| &view.view);
                let button_handles = props
                    .code_snippet_button_handles
                    .get(*props.starting_code_section_index);
                *props.starting_code_section_index += 1;
                column.add_child(render_code_output_section(
                    CodeSectionProps {
                        code_snippet: code,
                        editor_view,
                        button_handles,
                        language: language.as_ref(),
                        source: source.as_ref(),
                        is_ai_input_enabled: props.is_ai_input_enabled,
                        section_index,
                        find_context: props.find_context,
                        shell_launch_data: props.shell_launch_data,
                        working_directory: props.current_working_directory,
                        open_code_block_action_factory: props.open_code_block_action_factory,
                        copy_code_action_factory: props.copy_code_action_factory,
                        selectable: props.selectable,
                        #[cfg(feature = "local_fs")]
                        resolved_code_block_paths: props.resolved_code_block_paths,
                    },
                    app,
                ));
            }
            AIAgentTextSection::Table { table } => {
                let table_handles = props
                    .table_section_handles
                    .get(*props.starting_table_section_index)
                    .cloned()
                    .unwrap_or_default();
                *props.starting_table_section_index += 1;
                column.add_child(render_table_section(
                    table,
                    table_handles,
                    props.is_ai_input_enabled,
                    props.selectable,
                    section_index,
                    props.find_context,
                    app,
                ));
            }
            AIAgentTextSection::Image { image } => {
                let tooltip_mouse_state = props
                    .image_section_tooltip_handles
                    .get(*props.starting_image_section_index)
                    .cloned();
                *props.starting_image_section_index += 1;
                column.add_child(render_image_section(
                    ImageSectionProps {
                        image,
                        text_color: props.text_color,
                        section_index,
                        detected_links: props.detected_links,
                        secret_redaction_state: props.secret_redaction_state,
                        is_ai_input_enabled: props.is_ai_input_enabled,
                        is_selecting_text: props.is_selecting_text,
                        copy_action_factory: props.copy_code_action_factory,
                        current_working_directory: props.current_working_directory,
                        #[cfg(feature = "local_fs")]
                        resolved_blocklist_image_sources: props.resolved_blocklist_image_sources,
                        tooltip_mouse_state,
                    },
                    app,
                ));
            }
            AIAgentTextSection::MermaidDiagram { diagram } => {
                column.add_child(render_mermaid_diagram_section(
                    diagram,
                    props.text_color,
                    section_index,
                    props.copy_code_action_factory,
                    &lightbox_collection,
                    app,
                ));
            }
        }
        indexed_section_offset += 1;
    }
    column.finish()
}

fn text_sections_with_indices(
    sections: &[AIAgentTextSection],
    starting_text_section_index: usize,
) -> impl Iterator<Item = (usize, &AIAgentTextSection)> {
    sections
        .iter()
        .enumerate()
        .map(move |(offset, section)| (starting_text_section_index + offset, section))
}

#[derive(Clone, Copy)]
struct IndexedImageSection<'a> {
    section_index: usize,
    image: &'a AgentOutputImage,
}

#[derive(Clone)]
struct VisualMarkdownLightboxTrigger {
    images: Arc<Vec<ui_components::lightbox::LightboxImage>>,
    initial_index: usize,
}

struct VisualMarkdownLightboxCollection {
    section_indices: Vec<usize>,
    images: Arc<Vec<ui_components::lightbox::LightboxImage>>,
}

struct RenderableImageGroup<'a> {
    images: Vec<IndexedImageSection<'a>>,
    consumed_sections: usize,
}

fn collect_renderable_image_group<'a>(
    indexed_sections: &[(usize, &'a AIAgentTextSection)],
    start_index: usize,
    layout: AgentOutputImageLayout,
    current_working_directory: Option<&String>,
    #[cfg(feature = "local_fs")] resolved_blocklist_image_sources: Option<
        &ResolvedBlocklistImageSources,
    >,
    app: &AppContext,
) -> RenderableImageGroup<'a> {
    let mut images = Vec::new();
    let mut section_offset = start_index;
    let mut last_image_offset = start_index;
    while section_offset < indexed_sections.len() {
        let (section_index, section) = indexed_sections[section_offset];
        // Skip whitespace-only plain text sections (e.g. blank lines between images)
        // so that adjacent images separated only by blank lines are grouped together.
        if let AIAgentTextSection::PlainText { text } = section {
            if !images.is_empty() && text.text().trim().is_empty() {
                section_offset += 1;
                continue;
            }
        }
        let AIAgentTextSection::Image { image } = section else {
            break;
        };
        if image.layout != layout
            || !can_render_blocklist_image(
                image,
                current_working_directory,
                #[cfg(feature = "local_fs")]
                resolved_blocklist_image_sources,
                app,
            )
        {
            break;
        }
        images.push(IndexedImageSection {
            section_index,
            image,
        });
        section_offset += 1;
        last_image_offset = section_offset;
    }
    RenderableImageGroup {
        images,
        consumed_sections: last_image_offset - start_index,
    }
}

fn collect_visual_markdown_lightbox_collection(
    indexed_sections: &[(usize, &AIAgentTextSection)],
    current_working_directory: Option<&String>,
    #[cfg(feature = "local_fs")] resolved_blocklist_image_sources: Option<
        &ResolvedBlocklistImageSources,
    >,
    app: &AppContext,
) -> VisualMarkdownLightboxCollection {
    let mut section_indices = Vec::new();
    let mut images = Vec::new();

    for (section_index, section) in indexed_sections {
        if let Some(image) = lightbox_image_for_text_section(
            section,
            current_working_directory,
            #[cfg(feature = "local_fs")]
            resolved_blocklist_image_sources,
            app,
        ) {
            section_indices.push(*section_index);
            images.push(image);
        }
    }

    VisualMarkdownLightboxCollection {
        section_indices,
        images: Arc::new(images),
    }
}

fn lightbox_image_for_text_section(
    section: &AIAgentTextSection,
    current_working_directory: Option<&String>,
    #[cfg(feature = "local_fs")] resolved_blocklist_image_sources: Option<
        &ResolvedBlocklistImageSources,
    >,
    app: &AppContext,
) -> Option<ui_components::lightbox::LightboxImage> {
    match section {
        AIAgentTextSection::Image { image } => lightbox_image_for_blocklist_image(
            image,
            current_working_directory,
            #[cfg(feature = "local_fs")]
            resolved_blocklist_image_sources,
            app,
        ),
        AIAgentTextSection::MermaidDiagram { diagram } => {
            lightbox_image_for_mermaid_diagram(diagram, app)
        }
        _ => None,
    }
}

fn lightbox_image_for_blocklist_image(
    image: &AgentOutputImage,
    current_working_directory: Option<&String>,
    #[cfg(feature = "local_fs")] resolved_blocklist_image_sources: Option<
        &ResolvedBlocklistImageSources,
    >,
    app: &AppContext,
) -> Option<ui_components::lightbox::LightboxImage> {
    let (asset_source, _) = load_renderable_image_asset(
        image,
        current_working_directory,
        #[cfg(feature = "local_fs")]
        resolved_blocklist_image_sources,
        app,
    )?;
    Some(ui_components::lightbox::LightboxImage {
        source: ui_components::lightbox::LightboxImageSource::Resolved { asset_source },
        description: Some(image.source.clone()),
    })
}

fn lightbox_image_for_mermaid_diagram(
    diagram: &AgentOutputMermaidDiagram,
    app: &AppContext,
) -> Option<ui_components::lightbox::LightboxImage> {
    if !FeatureFlag::BlocklistMarkdownImages.is_enabled()
        || !FeatureFlag::MarkdownMermaid.is_enabled()
    {
        return None;
    }

    let asset_source = mermaid_asset_source(&diagram.source);
    let asset_state = AssetCache::as_ref(app).load_asset::<ImageType>(asset_source.clone());
    if matches!(asset_state, AssetState::FailedToLoad(_)) {
        return None;
    }

    Some(ui_components::lightbox::LightboxImage {
        source: ui_components::lightbox::LightboxImageSource::Resolved { asset_source },
        description: None,
    })
}

fn lightbox_trigger_for_section(
    lightbox_collection: &VisualMarkdownLightboxCollection,
    section_index: usize,
) -> Option<VisualMarkdownLightboxTrigger> {
    let initial_index = lightbox_collection
        .section_indices
        .iter()
        .position(|current_index| *current_index == section_index)?;

    Some(VisualMarkdownLightboxTrigger {
        images: Arc::clone(&lightbox_collection.images),
        initial_index,
    })
}

pub(super) struct RichTextSectionProps<'a> {
    is_restored: bool,
    formatted_text: Arc<FormattedText>,
    text_color: ColorU,
    is_ai_input_enabled: bool,
    section_index: usize,
    find_context: Option<FindContext<'a>>,
    secret_redaction_state: &'a SecretRedactionState,
    detected_links: Option<&'a DetectedLinksState>,
    is_selecting_text: bool,
    selectable: bool,
}

pub(super) fn render_rich_text_output_text_section(
    props: RichTextSectionProps<'_>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let inline_code_bg_color = if props.is_restored {
        appearance
            .theme()
            .background()
            .blend(&appearance.theme().restored_ai_blocks_overlay())
            .into_solid()
    } else {
        appearance.theme().background().into_solid()
    };
    let inline_code_text_color = theme.terminal_colors().normal.green.into();

    let line_count = props.formatted_text.lines.len();
    let mut rich_text_element = FormattedTextElement::new_arc(
        props.formatted_text,
        appearance.monospace_font_size(),
        appearance.ai_font_family(),
        appearance.monospace_font_family(),
        props.text_color,
        Default::default(),
    )
    .with_selection_color(if props.is_ai_input_enabled {
        theme.text_selection_as_context_color().into_solid()
    } else {
        theme.text_selection_color().into_solid()
    })
    // Lower line height ratio than default (1.4) and smaller header font size multipliers for AI blocks.
    .with_line_height_ratio(1.2)
    .with_heading_to_font_size_multipliers(HeadingFontSizeMultipliers {
        h1: 1.55,
        h2: 1.4,
        h3: 1.2,
        ..Default::default()
    })
    .with_inline_code_properties(Some(inline_code_text_color), Some(inline_code_bg_color))
    .set_selectable(props.selectable);

    rich_text_element.register_handlers(|mut frame, (line_index, _)| {
        let location = TextLocation::Output {
            section_index: props.section_index,
            line_index,
        };
        if let Some(detected_links) = props.detected_links {
            frame = add_link_detection_mouse_interactions(
                frame,
                detected_links,
                LinkActionConstructors::<AIBlockAction>::build_ai_block_action(),
                location,
            );
        }

        let secret_redaction = get_secret_obfuscation_mode(app);
        if secret_redaction.should_redact_secret() {
            if let Some(secrets) = props.secret_redaction_state.secrets_for_location(&location) {
                frame = redact_secrets_in_element(
                    frame,
                    secrets,
                    location,
                    secret_redaction.is_visually_obfuscated(),
                );
            }
        }
        frame
    });

    rich_text_element = add_highlights_to_rich_text(
        rich_text_element,
        props.detected_links,
        props.secret_redaction_state,
        props.find_context,
        props.section_index,
        line_count,
        appearance.theme(),
        props.is_selecting_text,
        false,
        app,
    );

    rich_text_element.finish()
}

struct TextSectionProps<'a> {
    text: &'a str,
    text_color: ColorU,
    is_ai_input_enabled: bool,
    section_index: usize,
    line_index: usize,
    secret_redaction_state: &'a SecretRedactionState,
    detected_links: Option<&'a DetectedLinksState>,
    find_context: Option<FindContext<'a>>,
    is_selecting_text: bool,
}

fn render_text_section(props: TextSectionProps, app: &AppContext) -> Box<dyn Element> {
    render_text_section_with_options(props, true, None, app)
}

fn render_text_section_with_options(
    props: TextSectionProps,
    soft_wrap: bool,
    clip_config: Option<ClipConfig>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let mut text_element = Text::new(
        props.text.to_owned(),
        appearance.ai_font_family(),
        appearance.monospace_font_size(),
    )
    .with_style(Properties::default().weight(appearance.monospace_font_weight()))
    .with_color(props.text_color)
    .with_selection_color(if props.is_ai_input_enabled {
        theme.text_selection_as_context_color().into_solid()
    } else {
        theme.text_selection_color().into_solid()
    })
    .soft_wrap(soft_wrap);

    if let Some(clip_config) = clip_config {
        text_element = text_element.with_clip(clip_config);
    }

    if let Some(detected_links_state) = props.detected_links {
        text_element = add_link_detection_mouse_interactions(
            text_element,
            detected_links_state,
            LinkActionConstructors::<AIBlockAction>::build_ai_block_action(),
            TextLocation::Output {
                section_index: props.section_index,
                line_index: props.line_index,
            },
        );
    }

    let should_obfuscate = get_secret_obfuscation_mode(app);
    if should_obfuscate.should_redact_secret() {
        let location = TextLocation::Output {
            section_index: props.section_index,
            line_index: props.line_index,
        };
        if let Some(secrets) = props.secret_redaction_state.secrets_for_location(&location) {
            text_element = redact_secrets_in_element(
                text_element,
                secrets,
                location,
                should_obfuscate.is_visually_obfuscated(),
            );
        }
    }
    if let Some(detected_links) = props.detected_links {
        text_element = add_highlights_to_text(
            text_element,
            detected_links,
            props.secret_redaction_state,
            props.find_context,
            TextLocation::Output {
                section_index: props.section_index,
                line_index: props.line_index,
            },
            props.is_selecting_text,
            None,
            None,
            app,
        );
    }
    text_element.finish()
}

struct ImageSectionProps<'a, A: Action> {
    image: &'a AgentOutputImage,
    text_color: ColorU,
    section_index: usize,
    detected_links: Option<&'a DetectedLinksState>,
    secret_redaction_state: &'a SecretRedactionState,
    is_ai_input_enabled: bool,
    is_selecting_text: bool,
    copy_action_factory: Option<CopyCodeActionFactory<A>>,
    current_working_directory: Option<&'a String>,
    #[cfg(feature = "local_fs")]
    resolved_blocklist_image_sources: Option<&'a ResolvedBlocklistImageSources>,
    /// Persistent `MouseStateHandle` used to back the hover tooltip when the
    /// image carries a CommonMark title. `None` means the caller had no
    /// pre-allocated handle (e.g. a reasoning/collapsible block).
    tooltip_mouse_state: Option<MouseStateHandle>,
}

#[derive(Clone, Copy)]
struct ImageRenderContext<'a, A: Action + 'static> {
    detected_links: Option<&'a DetectedLinksState>,
    secret_redaction_state: &'a SecretRedactionState,
    is_ai_input_enabled: bool,
    is_selecting_text: bool,
    copy_action_factory: Option<CopyCodeActionFactory<A>>,
    current_working_directory: Option<&'a String>,
    #[cfg(feature = "local_fs")]
    resolved_blocklist_image_sources: Option<&'a ResolvedBlocklistImageSources>,
    lightbox_collection: &'a VisualMarkdownLightboxCollection,
    /// Per-image persistent `MouseStateHandle`s for the images in this group,
    /// aligned with the `IndexedImageSection` slice passed to the group
    /// renderer. Indexed positionally (i-th image -> i-th handle).
    tooltip_mouse_state_handles: &'a [MouseStateHandle],
}

#[derive(Clone, Copy, Debug)]
enum VisualMarkdownAlignment {
    Left,
    Center,
}

#[derive(Clone)]
struct VisualMarkdownBlockOptions<A: 'static> {
    height: f32,
    width: Option<f32>,
    copy_action_factory: Option<CopyCodeActionFactory<A>>,
    max_width: Option<f32>,
    alignment: VisualMarkdownAlignment,
    lightbox_trigger: Option<VisualMarkdownLightboxTrigger>,
    /// When `Some(non_empty)`, the rendered image is wrapped in the standard
    /// Warp tooltip primitive so hovering surfaces the CommonMark image title.
    /// Mermaid diagrams pass `None` here because CommonMark titles do not
    /// apply to them.
    tooltip: Option<String>,
    /// Persistent `MouseStateHandle` backing the tooltip `Hoverable`. When
    /// `None`, `render_visual_markdown_block` falls back to a fresh
    /// `MouseStateHandle::default()` which cannot latch hover state across
    /// frames — so prefer threading a stable handle from
    /// `AIBlockStateHandles` whenever the caller has one.
    tooltip_mouse_state: Option<MouseStateHandle>,
}

/// Choose the text shown when a block-list image fails to render.
///
/// Per product invariant 8, non-empty alt text takes precedence over the raw
/// markdown source so authored alt text is surfaced on load failure.
fn image_fallback_text(image: &AgentOutputImage) -> String {
    if image.alt_text.is_empty() {
        image.markdown_source.clone()
    } else {
        image.alt_text.clone()
    }
}
#[cfg(feature = "local_fs")]
fn blocklist_image_asset_source(
    source: &str,
    current_working_directory: Option<&String>,
    resolved_blocklist_image_sources: Option<&ResolvedBlocklistImageSources>,
) -> Option<AssetSource> {
    match resolved_blocklist_image_sources {
        Some(cache) => cache.get(source).cloned().flatten(),
        None => Some(resolve_asset_source_relative_to_directory(
            source,
            current_working_directory.map(Path::new),
        )),
    }
}

#[cfg(not(feature = "local_fs"))]
fn blocklist_image_asset_source(
    source: &str,
    current_working_directory: Option<&String>,
) -> Option<AssetSource> {
    Some(resolve_asset_source_relative_to_directory(
        source,
        current_working_directory.map(Path::new),
    ))
}

fn load_renderable_image_asset(
    image: &AgentOutputImage,
    current_working_directory: Option<&String>,
    #[cfg(feature = "local_fs")] resolved_blocklist_image_sources: Option<
        &ResolvedBlocklistImageSources,
    >,
    app: &AppContext,
) -> Option<(AssetSource, AssetState<ImageType>)> {
    if !FeatureFlag::BlocklistMarkdownImages.is_enabled()
        || !is_supported_blocklist_image_source(&image.source)
    {
        return None;
    }

    #[cfg(feature = "local_fs")]
    let asset_source = blocklist_image_asset_source(
        &image.source,
        current_working_directory,
        resolved_blocklist_image_sources,
    )?;

    #[cfg(not(feature = "local_fs"))]
    let asset_source = blocklist_image_asset_source(&image.source, current_working_directory)?;
    let asset_state = AssetCache::as_ref(app).load_asset::<ImageType>(asset_source.clone());
    if matches!(asset_state, AssetState::FailedToLoad(_)) {
        return None;
    }

    Some((asset_source, asset_state))
}

fn can_render_blocklist_image(
    image: &AgentOutputImage,
    current_working_directory: Option<&String>,
    #[cfg(feature = "local_fs")] resolved_blocklist_image_sources: Option<
        &ResolvedBlocklistImageSources,
    >,
    app: &AppContext,
) -> bool {
    load_renderable_image_asset(
        image,
        current_working_directory,
        #[cfg(feature = "local_fs")]
        resolved_blocklist_image_sources,
        app,
    )
    .is_some()
}

fn render_image_section<A: Action>(
    props: ImageSectionProps<'_, A>,
    app: &AppContext,
) -> Box<dyn Element> {
    if !can_render_blocklist_image(
        props.image,
        props.current_working_directory,
        #[cfg(feature = "local_fs")]
        props.resolved_blocklist_image_sources,
        app,
    ) {
        return render_visual_markdown_fallback(
            &image_fallback_text(props.image),
            props.text_color,
            app,
        );
    }
    let indexed_image = IndexedImageSection {
        section_index: props.section_index,
        image: props.image,
    };
    let image_section = AIAgentTextSection::Image {
        image: props.image.clone(),
    };
    let indexed_sections = [(props.section_index, &image_section)];
    let lightbox_collection = collect_visual_markdown_lightbox_collection(
        &indexed_sections,
        props.current_working_directory,
        #[cfg(feature = "local_fs")]
        props.resolved_blocklist_image_sources,
        app,
    );
    let handles_for_group: &[MouseStateHandle] = props.tooltip_mouse_state.as_slice();
    let render_context = ImageRenderContext {
        detected_links: props.detected_links,
        secret_redaction_state: props.secret_redaction_state,
        is_ai_input_enabled: props.is_ai_input_enabled,
        is_selecting_text: props.is_selecting_text,
        copy_action_factory: props.copy_action_factory,
        current_working_directory: props.current_working_directory,
        #[cfg(feature = "local_fs")]
        resolved_blocklist_image_sources: props.resolved_blocklist_image_sources,
        lightbox_collection: &lightbox_collection,
        tooltip_mouse_state_handles: handles_for_group,
    };
    match props.image.layout {
        AgentOutputImageLayout::Inline => {
            render_inline_image_section_group(&[indexed_image], &render_context, app)
        }
        AgentOutputImageLayout::Block => {
            render_block_image_section_group(&[indexed_image], &render_context, app)
        }
    }
}

fn render_inline_image_section_group<A: Action>(
    images: &[IndexedImageSection<'_>],
    render_context: &ImageRenderContext<'_, A>,
    app: &AppContext,
) -> Box<dyn Element> {
    Align::new(
        Wrap::new(Axis::Horizontal)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(INLINE_IMAGE_SPACING)
            .with_run_spacing(INLINE_IMAGE_RUN_SPACING)
            .with_children(images.iter().enumerate().map(|(i, indexed_image)| {
                render_inline_image_group_item(*indexed_image, i, render_context, app)
            }))
            .finish(),
    )
    .left()
    .finish()
}

fn render_inline_image_group_item<A: Action>(
    indexed_image: IndexedImageSection<'_>,
    image_index_in_group: usize,
    render_context: &ImageRenderContext<'_, A>,
    app: &AppContext,
) -> Box<dyn Element> {
    let (asset_source, asset_state) = load_renderable_image_asset(
        indexed_image.image,
        render_context.current_working_directory,
        #[cfg(feature = "local_fs")]
        render_context.resolved_blocklist_image_sources,
        app,
    )
    .expect("inline image groups should only contain renderable images");
    let width = visual_section_max_width(&asset_state, INLINE_IMAGE_HEIGHT)
        .map(|width| width.min(INLINE_IMAGE_MAX_WIDTH))
        .unwrap_or(INLINE_IMAGE_HEIGHT);
    let image_block = render_visual_markdown_block(
        asset_source,
        indexed_image.image.markdown_source.clone(),
        VisualMarkdownBlockOptions {
            height: INLINE_IMAGE_HEIGHT,
            width: Some(width),
            copy_action_factory: render_context.copy_action_factory,
            max_width: None,
            alignment: VisualMarkdownAlignment::Left,
            lightbox_trigger: lightbox_trigger_for_section(
                render_context.lightbox_collection,
                indexed_image.section_index,
            ),
            tooltip: indexed_image
                .image
                .title
                .as_deref()
                .filter(|t| !t.is_empty())
                .map(str::to_owned),
            tooltip_mouse_state: render_context
                .tooltip_mouse_state_handles
                .get(image_index_in_group)
                .cloned(),
        },
        app,
    );
    let source_label = inline_image_source_label(&indexed_image.image.source);
    let source_link = render_image_source_link(
        ImageSourceLinkProps {
            source: source_label.as_ref(),
            section_index: indexed_image.section_index,
            detected_links: render_context.detected_links,
            secret_redaction_state: render_context.secret_redaction_state,
            is_ai_input_enabled: render_context.is_ai_input_enabled,
            is_selecting_text: render_context.is_selecting_text,
            soft_wrap: false,
        },
        app,
    );

    ConstrainedBox::new(
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(16.)
            .with_child(image_block)
            .with_child(source_link)
            .finish(),
    )
    .with_width(width)
    .finish()
}

fn inline_image_source_label(source: &str) -> Cow<'_, str> {
    Path::new(source)
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Borrowed(source))
}

fn render_block_image_section_group<A: Action>(
    images: &[IndexedImageSection<'_>],
    render_context: &ImageRenderContext<'_, A>,
    app: &AppContext,
) -> Box<dyn Element> {
    let mut body = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_spacing(BLOCK_IMAGE_ROW_SPACING);
    for (i, indexed_image) in images.iter().enumerate() {
        body.add_child(render_block_image_group_row(
            *indexed_image,
            i,
            render_context,
            app,
        ));
    }
    body.finish()
}

fn render_block_image_group_row<A: Action>(
    indexed_image: IndexedImageSection<'_>,
    image_index_in_group: usize,
    render_context: &ImageRenderContext<'_, A>,
    app: &AppContext,
) -> Box<dyn Element> {
    let (asset_source, _) = load_renderable_image_asset(
        indexed_image.image,
        render_context.current_working_directory,
        #[cfg(feature = "local_fs")]
        render_context.resolved_blocklist_image_sources,
        app,
    )
    .expect("block image groups should only contain renderable images");
    let image_block = render_visual_markdown_block(
        asset_source,
        indexed_image.image.markdown_source.clone(),
        VisualMarkdownBlockOptions {
            height: BLOCK_IMAGE_THUMBNAIL_SIZE,
            width: Some(BLOCK_IMAGE_THUMBNAIL_SIZE),
            copy_action_factory: render_context.copy_action_factory,
            max_width: None,
            alignment: VisualMarkdownAlignment::Center,
            lightbox_trigger: lightbox_trigger_for_section(
                render_context.lightbox_collection,
                indexed_image.section_index,
            ),
            tooltip: indexed_image
                .image
                .title
                .as_deref()
                .filter(|t| !t.is_empty())
                .map(str::to_owned),
            tooltip_mouse_state: render_context
                .tooltip_mouse_state_handles
                .get(image_index_in_group)
                .cloned(),
        },
        app,
    );
    let source_link = render_image_source_link(
        ImageSourceLinkProps {
            source: &indexed_image.image.source,
            section_index: indexed_image.section_index,
            detected_links: render_context.detected_links,
            secret_redaction_state: render_context.secret_redaction_state,
            is_ai_input_enabled: render_context.is_ai_input_enabled,
            is_selecting_text: render_context.is_selecting_text,
            soft_wrap: true,
        },
        app,
    );

    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(BLOCK_IMAGE_ROW_CONTENT_SPACING)
        .with_child(image_block)
        .with_child(Expanded::new(1., source_link).finish())
        .finish()
}

fn render_mermaid_diagram_section<A: Action>(
    diagram: &AgentOutputMermaidDiagram,
    text_color: ColorU,
    section_index: usize,
    copy_action_factory: Option<CopyCodeActionFactory<A>>,
    lightbox_collection: &VisualMarkdownLightboxCollection,
    app: &AppContext,
) -> Box<dyn Element> {
    if !FeatureFlag::BlocklistMarkdownImages.is_enabled()
        || !FeatureFlag::MarkdownMermaid.is_enabled()
    {
        return render_visual_markdown_fallback(&diagram.markdown_source, text_color, app);
    }

    let asset_source = mermaid_asset_source(&diagram.source);
    let asset_state = AssetCache::as_ref(app).load_asset::<ImageType>(asset_source.clone());
    if matches!(asset_state, AssetState::FailedToLoad(_)) {
        return render_visual_markdown_fallback(&diagram.markdown_source, text_color, app);
    }
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let mermaid_block = render_visual_markdown_block(
        asset_source,
        diagram.markdown_source.clone(),
        VisualMarkdownBlockOptions {
            height: visual_section_height(app),
            width: None,
            copy_action_factory,
            max_width: visual_section_max_width(&asset_state, visual_section_height(app)),
            alignment: VisualMarkdownAlignment::Center,
            lightbox_trigger: lightbox_trigger_for_section(lightbox_collection, section_index),
            // Mermaid diagrams don't carry CommonMark image titles.
            tooltip: None,
            tooltip_mouse_state: None,
        },
        app,
    );
    let mermaid_canvas = Container::new(mermaid_block)
        .with_background(theme.foreground())
        .with_uniform_padding(MERMAID_CANVAS_PADDING)
        .finish();

    render_visual_card(
        "Mermaid diagram".to_string(),
        Icon::Dataflow,
        Container::new(mermaid_canvas)
            .with_background(theme.background())
            .finish(),
        app,
    )
}

fn render_visual_markdown_fallback(
    markdown_source: &str,
    text_color: ColorU,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    Text::new(
        markdown_source.to_owned(),
        appearance.ai_font_family(),
        appearance.monospace_font_size(),
    )
    .with_style(Properties::default().weight(appearance.monospace_font_weight()))
    .with_color(text_color)
    .finish()
}

fn render_visual_markdown_block<A: Action>(
    asset_source: AssetSource,
    markdown_source: String,
    options: VisualMarkdownBlockOptions<A>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let placeholder = Text::new(
        markdown_source.clone(),
        appearance.monospace_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(blended_colors::text_sub(theme, theme.surface_2()))
    .finish();

    let image = WarpImage::new(asset_source, CacheOption::BySize)
        .contain()
        .before_load(placeholder)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)));
    let mut content = ConstrainedBox::new(Box::new(image)).with_height(options.height);
    if let Some(width) = finite_positive_visual_size(options.width) {
        content = content.with_width(width);
    } else if let Some(max_width) = finite_positive_visual_size(options.max_width) {
        content = content.with_max_width(max_width);
    } else if let Some(fallback_width) = finite_positive_visual_size(Some(options.height)) {
        content = content.with_width(fallback_width);
    }
    let content = content.finish();
    let content = match options.alignment {
        VisualMarkdownAlignment::Left => Align::new(content).left().finish(),
        VisualMarkdownAlignment::Center => Align::new(content).finish(),
    };

    // Wrap the rendered image in the standard Warp tooltip when the source
    // carried a CommonMark `title`. Branching on `Some(non_empty)` here means
    // untitled images remain un-wrapped, matching `specs/GH849/product.md`
    // invariant 6 (no tooltip for empty or absent titles). The tooltip's
    // `Hoverable` needs a
    // `MouseStateHandle` that survives across frames (so `is_hovered()` can
    // latch); prefer the caller's pre-allocated handle from
    // `AIBlockStateHandles.image_section_tooltip_handles`. Fall back to a
    // fresh default only for callers that genuinely have no persistent handle
    // (e.g. collapsible reasoning sub-blocks) — in that case the tooltip may
    // not render.
    let content = if let Some(tooltip) = options.tooltip.clone() {
        let mouse_state = options.tooltip_mouse_state.clone().unwrap_or_default();
        appearance.ui_builder().tool_tip_on_element(
            tooltip,
            mouse_state,
            content,
            warpui::elements::ParentAnchor::TopMiddle,
            warpui::elements::ChildAnchor::BottomMiddle,
            // Small negative Y offset keeps a hairline gap between the
            // tooltip's bottom edge and the image's top edge without
            // floating noticeably above the image.
            vec2f(0., -2.),
        )
    } else {
        content
    };

    let lightbox_trigger = options.lightbox_trigger.clone();
    let copy_action_factory = options.copy_action_factory;
    if lightbox_trigger.is_none() && copy_action_factory.is_none() {
        return content;
    }

    let mut event_handler = EventHandler::new(content);

    if let Some(lightbox_trigger) = lightbox_trigger {
        event_handler = event_handler.on_left_mouse_down(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::OpenLightbox {
                images: lightbox_trigger.images.as_ref().clone(),
                initial_index: lightbox_trigger.initial_index,
            });
            DispatchEventResult::StopPropagation
        });
    }

    if let Some(copy_action_factory) = copy_action_factory {
        event_handler = event_handler.on_right_mouse_down(move |ctx, _, _| {
            ctx.dispatch_typed_action(copy_action_factory(markdown_source.clone()));
            DispatchEventResult::StopPropagation
        });
    }

    event_handler.finish()
}

fn render_visual_card(
    title: String,
    icon: Icon,
    body: Box<dyn Element>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let header_background = theme.surface_2();
    let header_text_color = blended_colors::text_main(theme, header_background);
    let header_icon = ConstrainedBox::new(icon.to_warpui_icon(header_text_color.into()).finish())
        .with_width(16.)
        .with_height(16.)
        .finish();
    let header = Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(12.)
            .with_child(header_icon)
            .with_child(
                Expanded::new(
                    1.,
                    Text::new(
                        title,
                        appearance.ui_font_family(),
                        appearance.monospace_font_size(),
                    )
                    .with_color(header_text_color)
                    .finish(),
                )
                .finish(),
            )
            .finish(),
    )
    .with_background(header_background)
    .with_horizontal_padding(VISUAL_CARD_HEADER_HORIZONTAL_PADDING)
    .with_vertical_padding(VISUAL_CARD_HEADER_VERTICAL_PADDING)
    .finish();

    Container::new(Flex::column().with_child(header).with_child(body).finish())
        .with_border(Border::all(1.).with_border_fill(theme.outline()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
            VISUAL_CARD_CORNER_RADIUS,
        )))
        .finish()
}

fn visual_section_max_width(asset_state: &AssetState<ImageType>, height: f32) -> Option<f32> {
    let height = finite_positive_visual_size(Some(height))?;
    let (width, height_px) = match asset_state {
        AssetState::Loaded { data } => match data.as_ref() {
            ImageType::Svg { svg } => (svg.size().width(), svg.size().height()),
            ImageType::StaticBitmap { image } => (image.width() as f32, image.height() as f32),
            ImageType::AnimatedBitmap { image } => {
                let frame = image.frames.first()?;
                (frame.image.width() as f32, frame.image.height() as f32)
            }
            ImageType::Unrecognized => return None,
        },
        AssetState::Loading { .. } | AssetState::Evicted | AssetState::FailedToLoad(_) => {
            return None;
        }
    };

    compute_visual_section_width(width, height_px, height)
}

fn compute_visual_section_width(width: f32, height_px: f32, height: f32) -> Option<f32> {
    let width = finite_positive_visual_size(Some(width))?;
    let height_px = finite_positive_visual_size(Some(height_px))?;
    finite_positive_visual_size(Some(height * width / height_px))
}

fn finite_positive_visual_size(size: Option<f32>) -> Option<f32> {
    size.filter(|size| size.is_finite() && *size > 0.)
}

fn is_supported_blocklist_image_source(source: &str) -> bool {
    if source.starts_with("http://") || source.starts_with("https://") {
        return false;
    }

    Path::new(source)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tiff" | "tif" | "webp" | "ico" | "svg"
            )
        })
        .unwrap_or(false)
}

fn visual_section_height(app: &AppContext) -> f32 {
    rich_text_styles(Appearance::as_ref(app), FontSettings::as_ref(app))
        .base_line_height()
        .as_f32()
        * BLOCKLIST_VISUAL_SECTION_HEIGHT_LINE_MULTIPLIER
}

const TABLE_BLOCK_CORNER_RADIUS: f32 = 8.0;
fn render_table_section(
    table: &AgentOutputTable,
    table_handles: TableSectionHandles,
    is_ai_input_enabled: bool,
    selectable: bool,
    section_index: usize,
    find_context: Option<FindContext<'_>>,
    app: &AppContext,
) -> Box<dyn Element> {
    if let AgentOutputTableRendering::Legacy { content } = &table.rendering {
        return render_legacy_table_section(
            content,
            table_handles.scroll_handle,
            is_ai_input_enabled,
            app,
        );
    }
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let table_appearance = markdown_table_appearance(appearance);
    let notebook_styles = rich_text_styles(appearance, FontSettings::as_ref(app));
    let table_font_family = appearance.ai_font_family();
    let table_font_size = appearance.monospace_font_size();
    let body_font_weight = appearance.monospace_font_weight();
    let header_text_color = table_appearance.header_text_color;
    let body_text_color = table_appearance.text_color;
    let border_color = table_appearance.border_color;
    let cell_padding = table_appearance.cell_padding;
    let selection_color = if is_ai_input_enabled {
        theme.text_selection_as_context_color().into_solid()
    } else {
        theme.text_selection_color().into_solid()
    };
    let inline_code_text_color = notebook_styles.inline_code_style.font_color;
    let inline_code_bg_color = notebook_styles.inline_code_style.background;
    let Some(structured_table) = table.structured_table() else {
        return Empty::new().finish();
    };
    let column_count = structured_table.headers.len();
    let alignments = structured_table.alignments.clone();
    let body_rows = structured_table.rows.clone();
    let header_highlights =
        build_table_row_cell_highlights(&structured_table.headers, section_index, 0, find_context);
    let body_highlights = body_rows
        .iter()
        .enumerate()
        .map(|(row_index, row)| {
            build_table_row_cell_highlights(row, section_index, row_index + 1, find_context)
        })
        .collect_vec();
    let table_element = Table::new(table_handles.state_handle.clone(), 0.0, 0.0)
        .with_headers(
            (0..column_count)
                .map(|column_index| {
                    TableHeader::new(render_table_cell(
                        TableCellProps {
                            cell: structured_table
                                .headers
                                .get(column_index)
                                .cloned()
                                .unwrap_or_default(),
                            alignment: alignments.get(column_index).copied().unwrap_or_default(),
                            font_family: table_font_family,
                            font_size: table_font_size,
                            font_weight: Weight::Bold,
                            text_color: header_text_color,
                            inline_code_text_color,
                            inline_code_bg_color,
                            selection_color,
                            selectable,
                            highlights: header_highlights
                                .get(column_index)
                                .cloned()
                                .unwrap_or_default(),
                        },
                        app,
                    ))
                    .with_width(TableColumnWidth::Intrinsic)
                })
                .collect(),
        )
        .with_row_count(body_rows.len())
        .with_row_render_fn(move |row_index, app| {
            (0..column_count)
                .map(|column_index| {
                    render_table_cell(
                        TableCellProps {
                            cell: body_rows
                                .get(row_index)
                                .and_then(|row| row.get(column_index))
                                .cloned()
                                .unwrap_or_default(),
                            alignment: alignments.get(column_index).copied().unwrap_or_default(),
                            font_family: table_font_family,
                            font_size: table_font_size,
                            font_weight: body_font_weight,
                            text_color: body_text_color,
                            inline_code_text_color,
                            inline_code_bg_color,
                            selection_color,
                            selectable,
                            highlights: body_highlights
                                .get(row_index)
                                .and_then(|row| row.get(column_index))
                                .cloned()
                                .unwrap_or_default(),
                        },
                        app,
                    )
                })
                .collect()
        })
        .with_config(TableConfig {
            border_width: 1.0,
            border_color,
            outer_border: table_appearance.outer_border,
            column_dividers: table_appearance.column_dividers,
            row_dividers: table_appearance.row_dividers,
            cell_padding,
            header_background: table_appearance.header_background,
            row_background: warpui::elements::RowBackground {
                primary: table_appearance.cell_background,
                alternating: table_appearance.alternate_row_background,
            },
            fixed_header: false,
            vertical_sizing: TableVerticalSizing::ExpandToContent,
            measure_body_cells_for_intrinsic_widths: true,
        });

    NewScrollable::horizontal(
        SingleAxisConfig::Clipped {
            handle: table_handles.scroll_handle,
            child: table_element.finish(),
        },
        theme.nonactive_ui_detail().into(),
        theme.active_ui_detail().into(),
        Fill::None,
    )
    .with_horizontal_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, true))
    .with_propagate_mousewheel_if_not_handled(true)
    .finish()
}

fn render_legacy_table_section(
    content: &str,
    scroll_handle: ClippedScrollStateHandle,
    is_ai_input_enabled: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let text_element = Text::new(
        content.to_owned(),
        appearance.monospace_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(blended_colors::text_main(theme, theme.surface_2()))
    .with_selection_color(if is_ai_input_enabled {
        theme.text_selection_as_context_color().into_solid()
    } else {
        theme.text_selection_color().into_solid()
    })
    .finish();

    let inner_content = Container::new(text_element)
        .with_vertical_padding(INLINE_ACTION_HEADER_VERTICAL_PADDING)
        .with_horizontal_padding(INLINE_ACTION_HORIZONTAL_PADDING)
        .finish();

    let scrollable_content = NewScrollable::horizontal(
        SingleAxisConfig::Clipped {
            handle: scroll_handle,
            child: inner_content,
        },
        theme.nonactive_ui_detail().into(),
        theme.active_ui_detail().into(),
        Fill::None,
    )
    .with_horizontal_scrollbar(ScrollableAppearance::new(ScrollbarWidth::Auto, true))
    .with_propagate_mousewheel_if_not_handled(true)
    .finish();

    Container::new(scrollable_content)
        .with_background(theme.surface_2())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
            TABLE_BLOCK_CORNER_RADIUS,
        )))
        .finish()
}

fn render_table_cell(props: TableCellProps, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let mut cell_element = FormattedTextElement::new(
        FormattedText::new([markdown_parser::FormattedTextLine::Line(props.cell)]),
        props.font_size,
        props.font_family,
        appearance.monospace_font_family(),
        props.text_color,
        Default::default(),
    )
    .with_alignment(match props.alignment {
        TableAlignment::Left => TextAlignment::Left,
        TableAlignment::Center => TextAlignment::Center,
        TableAlignment::Right => TextAlignment::Right,
    })
    .with_weight(props.font_weight)
    .with_line_height_ratio(1.2)
    .with_selection_color(props.selection_color)
    .with_inline_code_properties(
        Some(props.inline_code_text_color),
        Some(props.inline_code_bg_color),
    )
    .set_selectable(props.selectable)
    .register_default_click_handlers(|hyperlink, _, app| {
        app.open_url(&hyperlink.url);
    });
    cell_element.add_styles(0, props.highlights);
    cell_element.finish()
}

struct TableCellProps {
    cell: FormattedTextInline,
    alignment: TableAlignment,
    font_family: warpui::fonts::FamilyId,
    font_size: f32,
    font_weight: Weight,
    text_color: ColorU,
    inline_code_text_color: ColorU,
    inline_code_bg_color: ColorU,
    selection_color: ColorU,
    selectable: bool,
    highlights: Vec<HighlightedRange>,
}

fn build_table_row_cell_highlights(
    row: &[FormattedTextInline],
    section_index: usize,
    line_index: usize,
    find_context: Option<FindContext<'_>>,
) -> Vec<Vec<HighlightedRange>> {
    let cell_texts = row
        .iter()
        .map(|cell| cell.iter().map(|fragment| fragment.text.as_str()).collect())
        .collect_vec();
    (0..row.len())
        .map(|cell_index| {
            build_table_cell_highlights(
                &cell_texts,
                cell_index,
                section_index,
                line_index,
                find_context,
            )
        })
        .collect()
}

fn build_table_cell_highlights(
    cell_texts: &[String],
    cell_index: usize,
    section_index: usize,
    line_index: usize,
    find_context: Option<FindContext<'_>>,
) -> Vec<HighlightedRange> {
    let Some(find_context) = find_context else {
        return Vec::new();
    };
    let cell_start = cell_texts
        .iter()
        .take(cell_index)
        .map(|cell_text| cell_text.chars().count() + 1)
        .sum::<usize>();
    let cell_end = cell_start
        + cell_texts
            .get(cell_index)
            .map(|cell_text| cell_text.chars().count())
            .unwrap_or(0);
    get_highlight_ranges_for_find_matches(
        TextLocation::Output {
            section_index,
            line_index,
        },
        find_context.state,
        find_context.model,
    )
    .filter_map(|highlighted_range| {
        let highlight_indices = highlighted_range
            .highlight_indices
            .into_iter()
            .filter_map(|index| {
                if (cell_start..cell_end).contains(&index) {
                    Some(index - cell_start)
                } else {
                    None
                }
            })
            .collect_vec();
        if highlight_indices.is_empty() {
            None
        } else {
            Some(HighlightedRange {
                highlight: highlighted_range.highlight,
                highlight_indices,
            })
        }
    })
    .collect()
}

type OpenCodeBlockActionFactory<A> = &'static dyn Fn(CodeSource) -> A;
type CopyCodeActionFactory<A> = &'static dyn Fn(String) -> A;

pub struct CodeSectionProps<'a, A: 'static> {
    pub code_snippet: &'a str,
    pub editor_view: Option<&'a ViewHandle<CodeEditorView>>,
    pub button_handles: Option<&'a CodeSnippetButtonHandles>,
    pub language: Option<&'a ProgrammingLanguage>,
    pub source: Option<&'a CodeSource>,
    pub is_ai_input_enabled: bool,
    pub section_index: usize,
    pub find_context: Option<FindContext<'a>>,
    pub shell_launch_data: Option<&'a ShellLaunchData>,
    pub working_directory: Option<&'a String>,
    pub open_code_block_action_factory: Option<OpenCodeBlockActionFactory<A>>,
    pub copy_code_action_factory: Option<CopyCodeActionFactory<A>>,
    /// Whether the code block text should be selectable within the parent SelectableArea.
    pub selectable: bool,
    /// Pre-resolved code block file paths from the background detection task.
    /// Keyed by original path; value is the resolved absolute path (or None if unresolvable).
    #[cfg(feature = "local_fs")]
    pub resolved_code_block_paths: Option<&'a HashMap<PathBuf, Option<PathBuf>>>,
}

pub fn render_code_output_section<A: Action>(
    props: CodeSectionProps<'_, A>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    #[cfg(feature = "local_fs")]
    let source = if let Some(CodeSource::Link {
        path,
        range_start,
        range_end,
    }) = props.source
    {
        // Try the pre-resolved cache first; fall back to sync resolution on cache miss.
        let resolved = props
            .resolved_code_block_paths
            .and_then(|cache| cache.get(path).cloned())
            .unwrap_or_else(|| {
                dirs::home_dir().and_then(|home_dir| {
                    resolve_absolute_file_path(
                        path.to_owned(),
                        props.working_directory,
                        props.shell_launch_data,
                        home_dir,
                    )
                })
            });
        resolved.map(|path| CodeSource::Link {
            path,
            range_start: *range_start,
            range_end: *range_end,
        })
    } else {
        None
    };

    #[cfg(not(feature = "local_fs"))]
    let source = None;

    // Prefer the resolved (absolute, validated-to-exist) path when available.
    // If resolution fails, fall back to the original path parsed from the AI output.
    let open_source = source.as_ref().or(props.source).cloned();

    let file_path = match source.as_ref() {
        Some(CodeSource::Link { path, .. }) => match props.working_directory {
            // Attempt to convert the absolute path to a relative path from the current working directory.
            // If that fails, fall back to using the original absolute path.
            Some(working_directory) => {
                let is_wsl = matches!(props.shell_launch_data, Some(ShellLaunchData::WSL { .. }));
                to_relative_path(is_wsl, path.as_path(), Path::new(working_directory))
            }
            .unwrap_or_else(|| path.to_string_lossy().to_string()),
            None => path.to_string_lossy().to_string(),
        }
        .into(),
        _ => None,
    };

    let language_text = props.language.map(|language| {
        Text::new_inline(
            language.display_name(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(blended_colors::text_sub(theme, theme.surface_3()))
        .with_selection_color(if props.is_ai_input_enabled {
            theme.text_selection_as_context_color().into_solid()
        } else {
            theme.text_selection_color().into_solid()
        })
        .finish()
    });

    let allow_execution = props.language.is_none_or(|lang| lang.is_shell());

    match props.editor_view {
        Some(view) => render_code_block_with_warp_text(
            CodeBlockOptions {
                on_open: match (props.open_code_block_action_factory, open_source.clone()) {
                    #[allow(unused)]
                    (Some(action_factory), Some(source)) => {
                        #[cfg(feature = "local_fs")]
                        {
                            Some(Box::new(move |_, ctx| {
                                ctx.dispatch_typed_action(action_factory(source.clone()));
                            }))
                        }
                        #[cfg(not(feature = "local_fs"))]
                        None
                    }
                    _ => None,
                },
                on_execute: if allow_execution {
                    Some(Box::new(|code_snippet, ctx| {
                        ctx.dispatch_typed_action(WorkspaceAction::RunAISuggestedCommand(
                            code_snippet,
                        ));
                    }))
                } else {
                    None
                },
                on_copy: if let Some(action_factory) = props.copy_code_action_factory {
                    Some(Box::new(|code_snippet, ctx| {
                        ctx.dispatch_typed_action(action_factory(code_snippet));
                    }))
                } else {
                    None
                },
                on_insert: if source.is_some() {
                    Some(Box::new(move |insert_text, ctx| {
                        ctx.dispatch_typed_action(
                            crate::workspace::WorkspaceAction::InsertInInput {
                                content: insert_text,
                                replace_buffer: false,
                                ensure_agent_mode: false,
                            },
                        );
                    }))
                } else {
                    None
                },
                file_path,
                footer_element: language_text,
                mouse_handles: props.button_handles.cloned(),
            },
            view,
            app,
            source,
        ),
        None => {
            let find_highlight_ranges: Box<dyn Iterator<Item = HighlightedRange>> =
                if let Some(find_context) = props.find_context {
                    Box::new(get_highlight_ranges_for_find_matches(
                        TextLocation::Output {
                            section_index: props.section_index,
                            line_index: 0,
                        },
                        find_context.state,
                        find_context.model,
                    ))
                } else {
                    Box::new(iter::empty())
                };
            render_code_block_plain(
                props.code_snippet,
                find_highlight_ranges,
                CodeBlockOptions {
                    on_open: match (props.open_code_block_action_factory, open_source.clone()) {
                        #[allow(unused)]
                        (Some(action_factory), Some(source)) => {
                            #[cfg(feature = "local_fs")]
                            {
                                Some(Box::new(move |_, ctx| {
                                    ctx.dispatch_typed_action(action_factory(source.clone()));
                                }))
                            }
                            #[cfg(not(feature = "local_fs"))]
                            None
                        }
                        _ => None,
                    },
                    on_execute: if allow_execution {
                        Some(Box::new(|code_snippet, ctx| {
                            ctx.dispatch_typed_action(WorkspaceAction::RunAISuggestedCommand(
                                code_snippet,
                            ));
                        }))
                    } else {
                        None
                    },
                    on_copy: if let Some(action_factory) = props.copy_code_action_factory {
                        Some(Box::new(|code_snippet, ctx| {
                            ctx.dispatch_typed_action(action_factory(code_snippet));
                        }))
                    } else {
                        None
                    },
                    on_insert: if source.is_some() {
                        Some(Box::new(move |insert_text, ctx| {
                            ctx.dispatch_typed_action(
                                crate::workspace::WorkspaceAction::InsertInInput {
                                    content: insert_text,
                                    replace_buffer: false,
                                    ensure_agent_mode: false,
                                },
                            );
                        }))
                    } else {
                        None
                    },
                    file_path,
                    footer_element: language_text,
                    mouse_handles: props.button_handles.cloned(),
                },
                props.selectable,
                app,
                source,
            )
        }
    }
}

pub fn get_highlight_ranges_for_find_matches(
    location: TextLocation,
    find_state: &FindState,
    find_model: &TerminalFindModel,
) -> impl Iterator<Item = HighlightedRange> {
    let find_match_locations = find_state.matches_for_location(location);
    let focused_match_location = find_model
        .block_list_find_run()
        .and_then(|run| match run.focused_match() {
            Some(BlockListMatch::RichContent { match_id, .. }) => Some(match_id),
            _ => None,
        })
        .and_then(|match_id| find_state.location_for_match(*match_id));
    let mut highlighted_ranges = vec![];
    for find_match_location in find_match_locations {
        let is_focused_match =
            focused_match_location.is_some_and(|location| find_match_location == location);

        let highlight = Highlight::new().with_text_style(
            TextStyle::new()
                .with_background_color(if is_focused_match {
                    *FOCUSED_MATCH_COLOR
                } else {
                    *MATCH_COLOR
                })
                .with_foreground_color(ColorU::black()),
        );
        let highlight_indices = find_match_location.char_range.clone().collect_vec();
        if highlight_indices.is_empty() {
            continue;
        }
        highlighted_ranges.push(HighlightedRange {
            highlight,
            highlight_indices,
        })
    }
    HighlightedRange::merge_overlapping_ranges(highlighted_ranges)
        .into_iter()
        .sorted_by_key(|r| r.highlight_indices[0])
}

/// Attempts to resolve a parsed file path into a valid one. Returns `None` if resolution fails.
#[cfg(feature = "local_fs")]
pub(crate) fn resolve_absolute_file_path(
    path: PathBuf,
    working_directory: Option<&String>,
    shell_launch_data: Option<&ShellLaunchData>,
    home_dir: PathBuf,
) -> Option<PathBuf> {
    use warp_util::path::CleanPathResult;

    use crate::util::file::{absolute_path_if_valid, ShellPathType};

    let clean_path = CleanPathResult::with_line_and_column_number(&path.to_string_lossy());

    // First, we check if the raw file path is a valid absolute path.
    if let Some(resolved) = absolute_path_if_valid(
        &clean_path,
        ShellPathType::PlatformNative(home_dir.clone()),
        shell_launch_data,
    ) {
        return Some(resolved);
    }

    working_directory.and_then(|wd| {
        let joined_path = Path::new(wd).join(&path);
        let clean_joined_path =
            CleanPathResult::with_line_and_column_number(&joined_path.to_string_lossy());
        absolute_path_if_valid(
            &clean_joined_path,
            ShellPathType::PlatformNative(home_dir),
            shell_launch_data,
        )
    })
}

pub struct FailedOutputProps<'a> {
    pub error: &'a RenderableAIError,
    pub invalid_api_key_button_handle: &'a MouseStateHandle,
    pub aws_bedrock_credentials_error_view: Option<&'a ViewHandle<AwsBedrockCredentialsErrorView>>,
    pub is_ai_input_enabled: bool,
    pub icon_right_margin: f32,
}

pub fn render_failed_output(props: FailedOutputProps, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);

    let error_text = match props.error {
        RenderableAIError::QuotaLimit {
            user_display_message,
        } => {
            if let Some(message) = user_display_message {
                format!("{ERROR_APOLOGY_TEXT}\n\n{message}")
            } else {
                let ai_request_usage_model = AIRequestUsageModel::as_ref(app);
                let formatted_next_refresh_time = ai_request_usage_model
                    .next_refresh_time()
                    .format("%B %d")
                    .to_string();

                format!(
                    "{ERROR_APOLOGY_TEXT}\n\nYou've reached your credit limit. Your credit limit resets on {formatted_next_refresh_time}.",
                )
            }
        }
        RenderableAIError::ServerOverloaded => {
            "Warp is currently overloaded. Please try again later.".to_string()
        }
        RenderableAIError::InternalWarpError => {
            format!("{ERROR_APOLOGY_TEXT}\n\n{INTERNAL_WARP_ERROR}")
        }
        RenderableAIError::Other {
            error_message,
            will_attempt_resume,
            waiting_for_network,
        } => {
            if *will_attempt_resume {
                if *waiting_for_network {
                    format!(
                        "{error_message}\n\nWill resume conversation when network connectivity is restored..."
                    )
                } else {
                    format!("{error_message}\n\nAttempting to resume conversation...")
                }
            } else {
                format!("{ERROR_APOLOGY_TEXT}\n\n{error_message}")
            }
        }
        RenderableAIError::InvalidApiKey {
            provider,
            model_name,
        } => {
            return render_invalid_api_key_error(
                provider,
                model_name,
                props.invalid_api_key_button_handle,
                app,
            );
        }
        RenderableAIError::ContextWindowExceeded(error) => {
            // This is rendered in a different way, like a failed action.
            return RenderableAction::new(error.as_str(), app)
                .with_icon(inline_action_icons::cancelled_icon(appearance).finish())
                .render(app)
                .finish();
        }
        RenderableAIError::AwsBedrockCredentialsExpiredOrInvalid { model_name } => {
            // Use the rich stateful view if it exists, otherwise show a simple error message
            if let Some(view) = props.aws_bedrock_credentials_error_view {
                return ChildView::new(view).finish();
            }
            // Fallback for contexts that don't have the stateful view (e.g. CLI subagent)
            format!(
                "{ERROR_APOLOGY_TEXT}\n\nAWS credentials expired or missing for {model_name}. \
                 Please refresh your AWS credentials."
            )
        }
    };

    Flex::row()
        .with_child(
            Container::new(
                ConstrainedBox::new(
                    warpui::elements::Icon::new(
                        Icon::AlertTriangle.into(),
                        error_color(appearance.theme()),
                    )
                    .finish(),
                )
                .with_width(icon_size(app))
                .with_height(icon_size(app))
                .finish(),
            )
            .with_margin_right(props.icon_right_margin)
            .finish(),
        )
        .with_child(
            Shrinkable::new(
                1.,
                Text::new(
                    error_text,
                    appearance.monospace_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_color(blended_colors::text_sub(
                    appearance.theme(),
                    appearance.theme().surface_1(),
                ))
                .with_selection_color(if props.is_ai_input_enabled {
                    appearance
                        .theme()
                        .text_selection_as_context_color()
                        .into_solid()
                } else {
                    appearance.theme().text_selection_color().into_solid()
                })
                .finish(),
            )
            .finish(),
        )
        .finish()
}

fn render_invalid_api_key_error(
    provider: &str,
    model_name: &str,
    state_handle: &MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let alert_icon = ConstrainedBox::new(
        Icon::AlertTriangle
            .to_warpui_icon(error_color(appearance.theme()).into())
            .finish(),
    )
    .with_width(icon_size(app))
    .with_height(icon_size(app))
    .finish();

    let alert_text = Text::new(
        "Provided API key is not valid",
        appearance.ui_font_family(),
        14.,
    )
    .with_color(error_color(appearance.theme()))
    .with_selectable(false)
    .finish();

    let detail_text = Text::new(
        format!(
            "Failed to authenticate with {provider} when using {model_name}. \
                     Double-check that your API key is correct."
        ),
        appearance.ui_font_family(),
        14.,
    )
    .with_color(blended_colors::text_sub(
        appearance.theme(),
        appearance.theme().surface_1(),
    ))
    .with_selectable(false)
    .finish();

    let settings_button = appearance
        .ui_builder()
        .button(
            warpui::ui_components::button::ButtonVariant::Outlined,
            state_handle.clone(),
        )
        .with_style(UiComponentStyles {
            border_color: Some(internal_colors::neutral_4(theme).into()),
            ..Default::default()
        })
        .with_hovered_styles(UiComponentStyles {
            background: Some(internal_colors::fg_overlay_2(theme).into()),
            ..Default::default()
        })
        .with_clicked_styles(UiComponentStyles {
            background: Some(internal_colors::fg_overlay_3(theme).into()),
            ..Default::default()
        })
        .with_text_label("Edit API Keys".to_string())
        .with_cursor(Some(Cursor::PointingHand))
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(WorkspaceAction::ShowSettingsPageWithSearch {
                search_query: "api keys".to_string(),
                section: Some(SettingsSection::WarpAgent),
            });
        })
        .finish();

    Flex::column()
        .with_spacing(16.)
        .with_child(
            Flex::row()
                .with_spacing(8.)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(alert_icon)
                .with_child(alert_text)
                .finish(),
        )
        .with_child(
            Flex::row()
                .with_spacing(8.)
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Shrinkable::new(1., detail_text).finish())
                .with_child(settings_button)
                .finish(),
        )
        .finish()
}

pub fn render_informational_footer(app: &AppContext, text: String) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);

    Text::new_inline(
        text,
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(
        appearance
            .theme()
            .disabled_text_color(appearance.theme().background())
            .into(),
    )
    .with_selectable(false)
    .finish()
}

pub(crate) struct DebugFooterProps<'a, V: View> {
    pub conversation: Option<&'a AIConversation>,
    pub model: &'a dyn AIBlockModel<View = V>,
    pub debug_copy_button_handle: MouseStateHandle,
    pub submit_issue_button_handle: MouseStateHandle,
    pub should_render_feedback_below: bool,
}

pub(crate) fn render_debug_footer<V: View>(
    props: DebugFooterProps<'_, V>,
    on_copy_debug_id: impl Fn(String, &mut EventContext) + 'static,
    on_open_feedback: impl Fn(&mut EventContext) + 'static,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);

    // get debug info (similar to CopyExternalDebuggingId)
    let conversation_token = props.conversation.and_then(|conversation| {
        BlocklistAIHistoryModel::as_ref(app)
            .conversation(&conversation.id())
            .and_then(|convo| convo.server_conversation_token())
    });
    let Some(conversation_token) = conversation_token else {
        return Empty::new().finish();
    };
    let server_output_id = props.model.server_output_id(app);
    let debug_info = if let Some(request_id) = server_output_id {
        serde_json::json!({
            "request_id": request_id,
            "conversation_id": conversation_token.as_str()
        })
        .to_string()
    } else {
        serde_json::json!({
            "conversation_id": conversation_token.as_str()
        })
        .to_string()
    };

    // Check if we should show the submit button (hide for dogfood and enterprise users)
    let is_dogfood = ChannelState::channel().is_dogfood();
    let is_enterprise_user = UserWorkspaces::as_ref(app)
        .current_team()
        .is_some_and(|team| team.billing_metadata.customer_type == CustomerType::Enterprise);
    let submit_button = if !is_dogfood && !is_enterprise_user {
        let submit_button_style = UiComponentStyles {
            font_color: Some(
                appearance
                    .theme()
                    .sub_text_color(appearance.theme().background())
                    .into(),
            ),
            border_width: Some(1.),
            border_color: Some(
                appearance
                    .theme()
                    .sub_text_color(appearance.theme().background())
                    .into(),
            ),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
            padding: Some(Coords {
                top: 4.,
                bottom: 4.,
                left: 8.,
                right: 8.,
            }),
            font_size: Some(appearance.monospace_font_size()),
            font_family_id: Some(appearance.ui_font_family()),
            ..Default::default()
        };
        let submit_button_hover_style = UiComponentStyles {
            background: Some(blended_colors::neutral_4(appearance.theme()).into()),
            ..submit_button_style
        };
        Some(
            appearance
                .ui_builder()
                .button(
                    warpui::ui_components::button::ButtonVariant::Text,
                    props.submit_issue_button_handle,
                )
                .with_centered_text_label("Send Feedback".to_string())
                .with_style(submit_button_style)
                .with_hovered_styles(submit_button_hover_style)
                .with_clicked_styles(submit_button_hover_style)
                .build()
                .on_click(move |ctx, _, _| {
                    on_open_feedback(ctx);
                })
                .finish(),
        )
    } else {
        None
    };

    // render the conversation's debug id so screenshots automatically show the debug id
    let debug_text = Text::new(
        format!("Debug information: {debug_info}"),
        appearance.ui_font_family(),
        appearance.monospace_font_size(),
    )
    .with_color(
        appearance
            .theme()
            .disabled_text_color(appearance.theme().background())
            .into(),
    )
    .finish();

    // render a button to copy the debug id to the clipboard
    let copy_button_style = UiComponentStyles {
        font_color: Some(
            appearance
                .theme()
                .sub_text_color(appearance.theme().background())
                .into(),
        ),
        width: Some(icon_size(app) + 4.),
        height: Some(icon_size(app) + 4.),
        ..Default::default()
    };
    let copy_button_hover_style = UiComponentStyles {
        background: Some(blended_colors::neutral_4(appearance.theme()).into()),
        ..copy_button_style
    };
    let debug_info_for_copy = debug_info.clone();
    let copy_button = icon_button(
        appearance,
        Icon::Copy,
        false,
        props.debug_copy_button_handle.clone(),
    )
    .with_style(copy_button_style)
    .with_hovered_styles(copy_button_hover_style)
    .with_clicked_styles(copy_button_hover_style)
    .build()
    .on_click(move |ctx, _, _| {
        on_copy_debug_id(debug_info_for_copy.clone(), ctx);
    })
    .finish();
    let copy_button_with_tooltip = appearance.ui_builder().tool_tip_on_element(
        "Copy debug ID".to_string(),
        props.debug_copy_button_handle,
        copy_button,
        warpui::elements::ParentAnchor::TopRight,
        warpui::elements::ChildAnchor::BottomRight,
        vec2f(0., -8.),
    );

    let mut debug_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Max);

    // In narrow views, render the submit button in a separate row below.
    // Otherwise, place it inline in the debug row.
    let stacked_submit_button = if props.should_render_feedback_below {
        submit_button
    } else {
        if let Some(submit_button) = submit_button {
            debug_row.add_child(Container::new(submit_button).with_margin_right(8.).finish());
        }
        None
    };

    debug_row.add_child(
        Shrinkable::new(
            1.0,
            Container::new(debug_text).with_margin_right(8.).finish(),
        )
        .finish(),
    );
    debug_row.add_child(copy_button_with_tooltip);

    if let Some(submit_button) = stacked_submit_button {
        let mut column = Flex::column();
        column.add_child(Expanded::new(1.0, debug_row.finish()).finish());
        column.add_child(Container::new(submit_button).with_margin_top(8.).finish());
        column.finish()
    } else {
        Container::new(Expanded::new(1.0, debug_row.finish()).finish()).finish()
    }
}

#[derive(Copy, Clone, Debug)]
pub struct FindContext<'a> {
    pub model: &'a TerminalFindModel,
    pub state: &'a FindState,
}

/// Renders a user avatar with profile image or display name.
pub fn render_user_avatar(
    user_display_name: &str,
    profile_image_path: Option<&String>,
    avatar_color: Option<ColorU>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let background = avatar_color.unwrap_or_else(|| blended_colors::accent(theme).into());
    let avatar = Avatar::new(
        profile_image_path
            .map(|url| AvatarContent::Image {
                url: url.to_owned(),
                display_name: user_display_name.to_owned(),
            })
            .unwrap_or(AvatarContent::DisplayName(user_display_name.to_owned())),
        UiComponentStyles {
            width: Some(icon_size(app)),
            height: Some(icon_size(app)),
            font_family_id: Some(appearance.ui_font_family()),
            font_size: Some(appearance.monospace_font_size() - 2.),
            background: Some(background.into()),
            font_color: Some(blended_colors::text_main(theme, background)),
            border_radius: Some(CornerRadius::with_all(Radius::Percentage(50.))),
            ..Default::default()
        },
    );
    avatar.build().finish()
}

pub struct UserQueryProps<'a> {
    pub text: String,
    pub query_prefix_highlight_len: Option<usize>,
    pub detected_links_state: &'a DetectedLinksState,
    pub secret_redaction_state: &'a SecretRedactionState,
    pub input_index: usize,
    pub is_selecting: bool,
    pub is_ai_input_enabled: bool,
    pub find_context: Option<FindContext<'a>>,
    pub font_properties: &'a Properties,
}
pub(crate) fn user_query_mode_prefix_highlight_len(mode: UserQueryMode) -> Option<usize> {
    match mode {
        UserQueryMode::Normal => None,
        UserQueryMode::Plan => Some(commands::PLAN.name.len()),
        UserQueryMode::Orchestrate => Some(commands::ORCHESTRATE.name.len()),
    }
}

pub(super) fn query_prefix_highlight_len(
    input: &AIAgentInput,
    displayed_query: &str,
) -> Option<usize> {
    if let AIAgentInput::UserQuery {
        user_query_mode, ..
    } = input
    {
        if let Some(prefix_len) = user_query_mode_prefix_highlight_len(*user_query_mode) {
            return Some(prefix_len);
        }
    }

    if displayed_query.starts_with(commands::CREATE_ENVIRONMENT.name) {
        Some(commands::CREATE_ENVIRONMENT.name.len())
    } else if displayed_query.starts_with(commands::AGENT.name) {
        Some(commands::AGENT.name.len())
    } else if displayed_query.starts_with(commands::NEW.name) {
        Some(commands::NEW.name.len())
    } else {
        match input {
            AIAgentInput::InvokeSkill { skill, .. } => Some(1 + skill.name.len()),
            AIAgentInput::UserQuery { .. }
            | AIAgentInput::AutoCodeDiffQuery { .. }
            | AIAgentInput::ResumeConversation { .. }
            | AIAgentInput::InitProjectRules { .. }
            | AIAgentInput::CreateEnvironment { .. }
            | AIAgentInput::TriggerPassiveSuggestion { .. }
            | AIAgentInput::CreateNewProject { .. }
            | AIAgentInput::CloneRepository { .. }
            | AIAgentInput::CodeReview { .. }
            | AIAgentInput::FetchReviewComments { .. }
            | AIAgentInput::SummarizeConversation { .. }
            | AIAgentInput::StartFromAmbientRunPrompt { .. }
            | AIAgentInput::ActionResult { .. }
            | AIAgentInput::MessagesReceivedFromAgents { .. }
            | AIAgentInput::EventsFromAgents { .. }
            | AIAgentInput::PassiveSuggestionResult { .. }
            | AIAgentInput::OrchestrationConfigUpdate { .. } => None,
        }
    }
}

/// Renders query text with all interactive features: link detection, secret redaction, and highlights.
/// Returns a text element ready to be placed in a layout.
pub fn render_query_text(props: UserQueryProps<'_>, app: &AppContext) -> Text {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let location = TextLocation::Query {
        input_index: props.input_index,
    };

    let mut text_element = Text::new(
        props.text,
        appearance.monospace_font_family(),
        appearance.monospace_font_size(),
    )
    .with_style(*props.font_properties)
    .with_color(blended_colors::text_main(theme, theme.surface_1()))
    .with_selection_color(if props.is_ai_input_enabled {
        theme.text_selection_as_context_color().into_solid()
    } else {
        theme.text_selection_color().into_solid()
    });

    // Add link detection
    text_element = add_link_detection_mouse_interactions(
        text_element,
        props.detected_links_state,
        LinkActionConstructors::<AIBlockAction>::build_ai_block_action(),
        location,
    );

    // Add secret redaction
    let secret_redaction = get_secret_obfuscation_mode(app);
    if secret_redaction.should_redact_secret() {
        let should_hide = secret_redaction.is_visually_obfuscated();
        if let Some(secrets) = props.secret_redaction_state.secrets_for_location(&location) {
            text_element = redact_secrets_in_element(text_element, secrets, location, should_hide);
        }
    }

    // Add combined highlights (links, secrets, find-in-page, /plan)
    text_element = add_highlights_to_text(
        text_element,
        props.detected_links_state,
        props.secret_redaction_state,
        props.find_context,
        location,
        props.is_selecting,
        Some(*props.font_properties),
        props.query_prefix_highlight_len,
        app,
    );

    text_element
}

/// Renders a scrollable collapsible content area with auto-scroll-to-bottom
/// during streaming. Returns `None` if the state is collapsed.
///
/// Shared by reasoning/summarization blocks and orchestration blocks.
pub(crate) fn render_scrollable_collapsible_content(
    message_id: &MessageId,
    state: &CollapsibleElementState,
    body: Box<dyn Element>,
    is_streaming: bool,
    max_height: f32,
) -> Option<Box<dyn Element>> {
    let CollapsibleExpansionState::Expanded {
        scroll_pinned_to_bottom,
        ..
    } = state.expansion_state
    else {
        return None;
    };

    let message_id_str: &str = message_id;
    let bottom_id = format!("ai_collapsible_bottom_{message_id_str}");
    let content_with_anchor = Flex::column()
        .with_child(body)
        .with_child(
            SavePosition::new(
                ConstrainedBox::new(Empty::new().finish())
                    .with_height(1.)
                    .finish(),
                &bottom_id,
            )
            .finish(),
        )
        .finish();

    if is_streaming && scroll_pinned_to_bottom {
        state.scroll_state.scroll_to_position(ScrollTarget {
            position_id: bottom_id.clone(),
            mode: ScrollToPositionMode::FullyIntoView,
        });
    }

    let scrollable = NewScrollable::vertical(
        SingleAxisConfig::Clipped {
            handle: state.scroll_state.clone(),
            child: content_with_anchor,
        },
        Fill::None,
        Fill::None,
        Fill::None,
    )
    .with_propagate_mousewheel_if_not_handled(true)
    .finish();

    let clipped_scrollable = ConstrainedBox::new(scrollable)
        .with_max_height(max_height)
        .finish();

    let message_id_clone = message_id.clone();
    Some(
        EventHandler::new(clipped_scrollable)
            .on_scroll_wheel(move |ctx, _app, _, _| {
                ctx.dispatch_typed_action(AIBlockAction::SetCollapsibleBlockPinnedToBottom {
                    message_id: message_id_clone.clone(),
                    pinned_to_bottom: false,
                });
                DispatchEventResult::PropagateToParent
            })
            .finish(),
    )
}

#[cfg(test)]
#[path = "common_tests.rs"]
mod tests;
