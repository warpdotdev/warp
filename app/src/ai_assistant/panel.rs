use std::sync::Arc;
use std::time::Duration;

use chrono::Local;

use pathfinder_geometry::vector::{vec2f, Vector2F};
use warp_editor::editor::NavigationKey;
use warpui::clipboard::ClipboardContent;
use warpui::elements::{
    resizable_state_handle, Align, Border, ChildAnchor, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, DispatchEventResult, DragBarSide, Empty, EventHandler, Fill, Flex,
    HyperlinkUrl, Icon, MainAxisAlignment, MainAxisSize, OffsetPositioning, ParentAnchor,
    PositionedElementAnchor, PositionedElementOffsetBounds, Radius, SavePosition, Shrinkable,
    Stack, Text,
};
use warpui::fonts::Properties;
use warpui::keymap::{EditableBinding, FixedBinding};
use warpui::platform::Cursor;
use warpui::presenter::ChildView;
use warpui::r#async::Timer;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{elements::Element, AppContext, Entity, TypedActionView, View, ViewContext};
use warpui::{FocusContext, ModelHandle, SingletonEntity, ViewHandle};

use crate::appearance::Appearance;
use crate::editor::{
    EditorOptions, EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, TextOptions,
};
use crate::input_suggestions::{Event as InputSuggestionsEvent, InputSuggestions};

use crate::send_telemetry_from_ctx;
use crate::server::server_api::ai::AIClient;
use crate::server::server_api::ServerApi;
use crate::server::telemetry::{TelemetryEvent, WarpAIActionType};
use crate::terminal::resizable_data::{ModalType, ResizableData, DEFAULT_WARP_AI_WIDTH};
use crate::ui_components::blended_colors;
use crate::workspaces::user_workspaces::UserWorkspaces;

use crate::ui_components::buttons::icon_button;
use crate::workspace::{ActiveSession, TAB_BAR_HEIGHT};

use crate::util::bindings::{cmd_or_ctrl_shift, CustomAction};
use warpui::elements::MouseStateHandle;
use warpui::elements::ParentElement;
use warpui::elements::Resizable;
use warpui::elements::ResizableStateHandle;

use super::execution_context::WarpAiExecutionContext;
use super::requests::{Event as RequestsEvent, RequestStatus, Requests};
use super::transcript::{Transcript, TranscriptEvent};
use super::utils::{render_prepared_response_button, render_request_limit_info, TranscriptPart};
use super::{
    AskAIType, AI_ASSISTANT_FEATURE_NAME, AI_ASSISTANT_LOGO_COLOR, AI_ASSISTANT_SVG_PATH,
    ASK_AI_ASSISTANT_TEXT, PROMPT_CHARACTER_LIMIT,
};

const INFO_ICON_SVG_PATH: &str = "bundled/svg/info.svg";
pub const HEXAGON_ALERT_SVG_PATH: &str = "bundled/svg/alert-hexagon.svg";

const EDITOR_SAVE_POSITION_ID: &str = "ai_assistant::editor";

const MIN_PANEL_WIDTH: f32 = 300.;
const MIN_REMAINING_WINDOW_SIZE: f32 = 200.;
const MAX_EDITOR_HEIGHT: f32 = 300.;
const MAX_INPUT_SUGGESTIONS_HEIGHT: f32 = 200.;

pub(super) const HEADER_HEIGHT: f32 = TAB_BAR_HEIGHT;
pub(super) const HEADER_VERTICAL_PADDING: f32 = 5.;
const PANEL_HORIZONTAL_PADDING: f32 = 6.;
const EDITOR_MARGIN: f32 = 16.;
const LOGO_SIZE: f32 = 20.;

const BODY_FONT_SIZE: f32 = 13.;
const TITLE_FONT_SIZE: f32 = 16.;
const ZERO_STATE_HELP_TEXT_FONT_SIZE: f32 = 12.;

const ZERO_STATE_HELP_TEXT: &str = "Shift + ctrl + space a block or text selection to ask Warp AI.";
const SCRIPT_ZERO_STATE_PROMPT: &str = "Write a script to connect to an AWS EC2 instance.";
const GIT_ZERO_STATE_PROMPT: &str = "How do I undo the most recent commits in git?";
const FILES_ZERO_STATE_PROMPT: &str = "How do I find all files containing specific text?";

// The placeholder texts are prepended with a space to give them cushion from the cursor.
const INIT_PLACEHOLDER_TEXT: &str = " Ask a question...";
const FOLLOWUP_PLACEHOLDER_TEXT: &str = " Type a response or click one above...";
const RESTART_BUTTON_TEXT: &str = "Restart";

const ASK_AI_BLOCK_INPUT_LIMIT: usize = 100;

#[derive(Default)]
struct MouseStateHandles {
    close_panel_state: MouseStateHandle,
    reset_context_button: MouseStateHandle,
    copy_transcript_button: MouseStateHandle,

    script_zero_state_prompt: MouseStateHandle,
    git_zero_state_prompt: MouseStateHandle,
    files_zero_state_prompt: MouseStateHandle,
}

pub enum AIAssistantPanelEvent {
    ClosePanel,
    PasteInTerminalInput(Arc<String>),
    FocusTerminalInput,
    OpenWorkflowModalWithCommand(String),
}

/// Which child view is currently focused. It must be exactly one of these.
#[derive(Copy, Clone)]
enum PanelFocusState {
    Editor,
    Transcript,
}

enum InputSuggestionsMode {
    Open { origin_buffer_text: String },
    Closed,
}

/// The panel view is responsible for the various components that make up the panel
/// (e.g. header, transcript, editor).
/// TODO: we should eventually refactor this and other panels into a more
/// general Panel view.
pub struct AIAssistantPanelView {
    editor: ViewHandle<EditorView>,
    transcript_view: ViewHandle<Transcript>,
    input_suggestions_view: ViewHandle<InputSuggestions>,
    input_suggestions_mode: InputSuggestionsMode,
    requests_model: ModelHandle<Requests>,
    focus_state: PanelFocusState,

    resizable_state_handle: ResizableStateHandle,
    mouse_state_handles: MouseStateHandles,
}

#[derive(Debug, Clone)]
pub enum AIAssistantAction {
    ClosePanel,
    ResetContext,
    CopyTranscript,
    PreparedPrompt(&'static str),
    ClickedUrl(HyperlinkUrl),
    CopyAnswerToClipboard(Arc<String>),
    FocusTerminalInput,
    FocusEditor,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::custom(
        CustomAction::CloseCurrentSession,
        AIAssistantAction::ClosePanel,
        "Close Warp AI",
        id!("AIAssistantPanel"),
    )]);

    app.register_editable_bindings([
        EditableBinding::new(
            "ai_assistant_panel:focus_terminal_input",
            "Focus Terminal Input From Warp AI",
            AIAssistantAction::FocusTerminalInput,
        )
        .with_context_predicate(id!("AIAssistantPanel"))
        .with_key_binding(cmd_or_ctrl_shift("l")),
        EditableBinding::new(
            "ai_assistant_panel:reset_context",
            "Restart Warp AI",
            AIAssistantAction::ResetContext,
        )
        .with_context_predicate(id!("AIAssistantPanel"))
        .with_key_binding("ctrl-l"),
        EditableBinding::new(
            "ai_assistant_panel:reset_context",
            "Restart Warp AI",
            AIAssistantAction::ResetContext,
        )
        .with_context_predicate(id!("AIAssistantPanel"))
        .with_key_binding(cmd_or_ctrl_shift("k")),
    ]);
}

impl AIAssistantPanelView {
    pub fn new(
        server_api: Arc<ServerApi>,
        ai_client: Arc<dyn AIClient>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let editor = {
            ctx.add_typed_action_view(|ctx| {
                let appearance = Appearance::as_ref(ctx);
                let options = EditorOptions {
                    text: TextOptions::ui_text(Some(BODY_FONT_SIZE), appearance),
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    autogrow: true,
                    soft_wrap: true,
                    supports_vim_mode: true,
                    ..Default::default()
                };

                EditorView::new(options, ctx)
            })
        };
        editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text(INIT_PLACEHOLDER_TEXT, ctx)
        });
        ctx.subscribe_to_view(&editor, |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        let active_session_model = ActiveSession::handle(ctx);
        ctx.observe(&active_session_model, Self::on_active_session_change);

        let requests_model =
            ctx.add_model(|ctx| Requests::new(server_api.clone(), ai_client.clone(), ctx));
        ctx.subscribe_to_model(&requests_model, move |me, _, event, ctx| {
            me.handle_requests_model_event(event, ctx);
        });
        ctx.observe(&requests_model, |_, _, ctx| ctx.notify());

        let transcript_view =
            ctx.add_typed_action_view(|ctx| Transcript::new(&requests_model, ctx));
        ctx.subscribe_to_view(&transcript_view, |me, _, event, ctx| {
            me.handle_transcript_event(event, ctx);
        });

        let input_suggestions_view = ctx.add_typed_action_view(InputSuggestions::new);
        ctx.subscribe_to_view(&input_suggestions_view, move |me, _, event, ctx| {
            me.handle_input_suggestions_event(event, ctx);
        });

        let resizable_data_handle = ResizableData::handle(ctx);
        let resizable_state_handle = match resizable_data_handle
            .as_ref(ctx)
            .get_handle(ctx.window_id(), ModalType::WarpAIWidth)
        {
            Some(handle) => handle,
            None => {
                log::error!("Couldn't retrieve warp ai resizable state handle.");
                resizable_state_handle(DEFAULT_WARP_AI_WIDTH)
            }
        };

        let mut panel = Self {
            editor,
            transcript_view,
            input_suggestions_view,
            input_suggestions_mode: InputSuggestionsMode::Closed,
            requests_model,
            focus_state: PanelFocusState::Editor,

            resizable_state_handle,
            mouse_state_handles: Default::default(),
        };

        panel.tick(ctx);
        panel.on_active_session_change(active_session_model, ctx);
        panel
    }

    fn on_active_session_change(
        &mut self,
        active_session_handle: ModelHandle<ActiveSession>,
        ctx: &mut ViewContext<Self>,
    ) {
        let window_id = ctx.window_id();
        let ai_execution_context = active_session_handle
            .as_ref(ctx)
            .session(window_id)
            .as_ref()
            .map(WarpAiExecutionContext::new);
        self.requests_model.update(ctx, |requests, _| {
            requests.update_ai_execution_context(ai_execution_context);
        });
    }

    // Every minute, we re-render to make the next refresh time tick.
    fn tick(&self, ctx: &mut ViewContext<Self>) {
        ctx.spawn(
            async move { Timer::after(Duration::from_secs(60)).await },
            |view, _, ctx| {
                view.transcript_view.update(ctx, |_, ctx| {
                    ctx.notify();
                });
                ctx.notify();
                view.tick(ctx);
            },
        );
    }

    fn format_as_code_block(&self, content: &str) -> String {
        // Intentionally choose a language that won't be interpreted as a shell language
        // i.e. (*sh)
        format!("```warp\n{}\n```", content.trim())
    }

    // TODO: reconsider if we should be doing all the formatting in here as opposed
    // to doing the formatting at source and passing down the prompt to render as is.
    pub fn ask_ai(&mut self, ask_type: &AskAIType, ctx: &mut ViewContext<Self>) {
        match ask_type {
            AskAIType::FromTextSelection {
                text,
                populate_input_box,
            } => {
                if *populate_input_box {
                    let prefix = "Explain the following:\n";
                    let code_block_formatting_len = self.format_as_code_block("").len();
                    let truncated =
                        if text.chars().count() + prefix.len() + code_block_formatting_len
                            > PROMPT_CHARACTER_LIMIT
                        {
                            // Take the first k characters of the text selection, where k is the
                            // remaining length after we limit the prompt and add formatting to it.
                            let truncated: String = text
                                .chars()
                                // Take 3 for the ellipsis
                                .take(
                                    PROMPT_CHARACTER_LIMIT
                                        - prefix.len()
                                        - code_block_formatting_len
                                        - 3,
                                )
                                .collect();
                            format!("{truncated}...")
                        } else {
                            text.to_string()
                        };

                    self.editor.update(ctx, |editor, ctx| {
                        editor.set_buffer_text(
                            &format!(
                                "{}{}",
                                prefix,
                                self.format_as_code_block(truncated.as_str())
                            ),
                            ctx,
                        );
                    });
                    ctx.notify();
                }
            }
            AskAIType::FromBlock {
                input,
                output,
                exit_code,
                ..
            } => {
                let block_successful = exit_code.was_successful();

                // Formatting strings.
                let question = if block_successful {
                    "\nWhat should I do next?"
                } else {
                    "\nHow do I fix this?"
                };
                let prefix = "I ran the command: `";
                let suffix = "` and got the following output:\n";
                let code_block_formatting_len = self.format_as_code_block("").len();
                let non_input_output_len =
                    prefix.len() + suffix.len() + question.len() + code_block_formatting_len;

                let input_len = input.chars().count();
                let output_len = output.chars().count();

                // If the input and output are longer than can be and the input is particularly large, try to
                // shave the input down to a fixed number of chars.
                let truncated_input = if input_len + output_len + non_input_output_len
                    > PROMPT_CHARACTER_LIMIT
                    && input_len > ASK_AI_BLOCK_INPUT_LIMIT
                {
                    let truncated: String = input.chars().take(ASK_AI_BLOCK_INPUT_LIMIT).collect();
                    format!("{truncated}...")
                } else {
                    input.to_string()
                };
                let truncated_input_len = truncated_input.chars().count();

                // If the truncated input and raw output are still longer than
                // the allowed size, trim down the output.
                let truncated_output = if truncated_input_len + output_len + non_input_output_len
                    > PROMPT_CHARACTER_LIMIT
                {
                    // Take the last k characters of the block's output, where k is the
                    // remaining length after we limit the prompt and add formatting to it.
                    // + 3 for the ellipsis.
                    let output_starting_index =
                        output_len + truncated_input_len + non_input_output_len + 3
                            - PROMPT_CHARACTER_LIMIT;
                    let truncated: String = output.chars().skip(output_starting_index).collect();
                    format!("...{truncated}")
                } else {
                    output.to_string()
                };

                // Insert the truncated strings (with the formatting around them) into the editor.
                self.editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(
                        &format!(
                            "{prefix}{}{suffix}{}{question}",
                            truncated_input,
                            self.format_as_code_block(truncated_output.as_str())
                        ),
                        ctx,
                    );
                });
                ctx.notify();
            }
            AskAIType::FromAICommandSearch { query } => {
                let truncated = if query.chars().count() > PROMPT_CHARACTER_LIMIT {
                    // Reserve 3 for the ellpisis
                    let truncated: String =
                        query.chars().take(PROMPT_CHARACTER_LIMIT - 3).collect();
                    format!("{truncated}...")
                } else {
                    query.to_string()
                };
                self.editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(truncated.as_str(), ctx);
                });
            }
            // Not supported by the AI Assistant. Only supported by blocklist AI.
            AskAIType::FromBlocks { .. } => (),
        }

        send_telemetry_from_ctx!(
            TelemetryEvent::OpenedWarpAI {
                source: ask_type.into()
            },
            ctx
        );
    }

    fn is_prompt_too_long(&self, prompt: &str) -> bool {
        prompt.chars().count() > PROMPT_CHARACTER_LIMIT
    }

    fn is_prompt_empty(&self, prompt: &str) -> bool {
        prompt.chars().count() == 0
    }

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Enter => {
                self.input_suggestions_mode = InputSuggestionsMode::Closed;
                let buffer_text = self.editor.as_ref(ctx).buffer_text(ctx);
                if !self.is_prompt_too_long(buffer_text.as_str())
                    && !self.is_prompt_empty(buffer_text.as_str())
                {
                    self.issue_request(buffer_text, ctx);
                } else {
                    // Only send this event if the user tried to execute with a longer than permitted prompt.
                    send_telemetry_from_ctx!(TelemetryEvent::WarpAICharacterLimitExceeded, ctx);
                }
                ctx.notify();
            }
            EditorEvent::Edited(_) => {
                // Force a re-render so we can show the character limit warning.
                ctx.notify();

                if let Some(selected_text) = self
                    .input_suggestions_view
                    .as_ref(ctx)
                    .get_selected_item_text()
                {
                    if selected_text == self.editor.as_ref(ctx).buffer_text(ctx) {
                        return;
                    }
                }
                self.input_suggestions_mode = InputSuggestionsMode::Closed;
            }
            EditorEvent::CmdUpOnFirstRow => {
                self.transcript_view.update(ctx, |transcript_view, ctx| {
                    transcript_view.select_last_code_block(ctx);
                });
                ctx.notify();
            }
            EditorEvent::Activate => {
                self.focus_state = PanelFocusState::Editor;
                ctx.focus_self();
            }
            EditorEvent::Escape => {
                self.input_suggestions_view
                    .update(ctx, |input_suggestions, ctx| {
                        input_suggestions.exit(true, ctx);
                    });
                ctx.notify();
            }
            EditorEvent::Navigate(NavigationKey::Up) => {
                if matches!(self.input_suggestions_mode, InputSuggestionsMode::Closed) {
                    let editor = self.editor.as_ref(ctx);
                    if editor.single_cursor_on_first_row(ctx) {
                        let buffer_text = editor.buffer_text(ctx);
                        let all_past_prompts = self
                            .requests_model
                            .as_ref(ctx)
                            .all_past_transcript_prompts();
                        self.input_suggestions_view
                            .update(ctx, |input_suggestions, ctx| {
                                input_suggestions.fuzzy_substring_search(
                                    buffer_text.clone(),
                                    all_past_prompts,
                                    ctx,
                                );
                            });
                        self.input_suggestions_mode = InputSuggestionsMode::Open {
                            origin_buffer_text: buffer_text,
                        };
                    } else {
                        self.editor.update(ctx, |editor, ctx| editor.move_up(ctx));
                    }
                } else {
                    self.input_suggestions_view
                        .update(ctx, |input_suggestions, ctx| {
                            input_suggestions.select_prev(ctx);
                        });
                }
                ctx.notify();
            }
            EditorEvent::Navigate(NavigationKey::Down) => {
                if matches!(
                    self.input_suggestions_mode,
                    InputSuggestionsMode::Open { .. }
                ) {
                    self.input_suggestions_view
                        .update(ctx, |input_suggestions, ctx| {
                            if input_suggestions.is_empty() {
                                input_suggestions.exit(true, ctx);
                            } else {
                                input_suggestions.select_next(ctx);
                            }
                        });
                } else {
                    self.editor.update(ctx, |editor, ctx| editor.move_down(ctx));
                }
                ctx.notify();
            }
            _ => {}
        }
    }

    fn handle_transcript_event(&mut self, event: &TranscriptEvent, ctx: &mut ViewContext<Self>) {
        match event {
            TranscriptEvent::PasteInTerminalInput { code_block_index } => {
                let code = self.transcript_view.read(ctx, |transcript_view, ctx| {
                    transcript_view.code_for_index(*code_block_index, ctx)
                });
                if let Some(code) = code {
                    ctx.emit(AIAssistantPanelEvent::PasteInTerminalInput(Arc::new(code)));
                }
            }
            TranscriptEvent::FocusEditor => {
                self.focus_state = PanelFocusState::Editor;
                ctx.focus_self();
            }
            TranscriptEvent::ClickedCodeBlock | TranscriptEvent::FocusTranscript => {
                self.focus_state = PanelFocusState::Transcript;
                ctx.focus_self();
            }
            TranscriptEvent::OpenWorkflowModalWithCommand(command) => {
                ctx.emit(AIAssistantPanelEvent::OpenWorkflowModalWithCommand(
                    command.clone(),
                ));
            }
        }
    }

    fn handle_requests_model_event(&mut self, event: &RequestsEvent, ctx: &mut ViewContext<Self>) {
        match event {
            RequestsEvent::RequestFinished { .. } => {
                self.editor.update(ctx, |editor, ctx| {
                    editor.clear_buffer_and_reset_undo_stack(ctx);
                    editor.set_placeholder_text(FOLLOWUP_PLACEHOLDER_TEXT, ctx);
                });
                self.transcript_view.update(ctx, |transcript_view, ctx| {
                    transcript_view.scroll_to_bottom_of_transcript(ctx);
                });
                ctx.notify();
            }
        }
    }

    fn handle_input_suggestions_event(
        &mut self,
        event: &InputSuggestionsEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            InputSuggestionsEvent::CloseSuggestion { .. } => {
                if let InputSuggestionsMode::Open { origin_buffer_text } =
                    &self.input_suggestions_mode
                {
                    self.editor.update(ctx, |editor, ctx| {
                        editor.set_buffer_text(origin_buffer_text, ctx);
                    });
                }
                self.input_suggestions_mode = InputSuggestionsMode::Closed;
            }
            InputSuggestionsEvent::Select(item) => {
                self.editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(item.text(), ctx);
                });
            }
            InputSuggestionsEvent::ConfirmSuggestion { suggestion, .. } => {
                self.editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text(suggestion, ctx);
                });
                self.input_suggestions_mode = InputSuggestionsMode::Closed;
            }
            InputSuggestionsEvent::ConfirmAndExecuteSuggestion { suggestion, .. } => {
                self.issue_request(suggestion.to_string(), ctx);
                self.input_suggestions_mode = InputSuggestionsMode::Closed;
            }
            InputSuggestionsEvent::IgnoreItem { .. } => {
                // not worth implementing; feature is hardly used
            }
        }
        ctx.notify();
    }

    fn issue_request(&mut self, request: String, ctx: &mut ViewContext<Self>) {
        self.requests_model.update(ctx, |requests_model, ctx| {
            requests_model.issue_request(request, ctx);
        });
        self.transcript_view.update(ctx, |transcript_view, ctx| {
            transcript_view.clear_selected_block(ctx);
            transcript_view.scroll_to_bottom_of_transcript(ctx);
        });
        ctx.notify();
    }

    fn reset_context(&mut self, ctx: &mut ViewContext<Self>) {
        let request_status = self.requests_model.as_ref(ctx).request_status();
        if matches!(request_status, &RequestStatus::InFlight { .. }) {
            self.editor.update(ctx, |editor, ctx| {
                editor.clear_buffer_and_reset_undo_stack(ctx);
            });
        }

        self.editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text(INIT_PLACEHOLDER_TEXT, ctx);
        });

        self.requests_model.update(ctx, |requests_model, ctx| {
            requests_model.reset(ctx);
        });

        self.transcript_view.update(ctx, |transcript_view, ctx| {
            transcript_view.reset(ctx);
        });

        self.focus_state = PanelFocusState::Editor;

        ctx.focus_self();
        ctx.notify();
    }

    fn copy_transcript(&mut self, ctx: &mut ViewContext<Self>) {
        let transcript = self.transcript(ctx);
        let mut result = String::new();
        let time_now = Local::now();

        result.push_str(&format!(
            "## Warp AI Transcript ({})\n\n",
            time_now.format("%x %l:%M %p")
        ));

        for part in transcript {
            result.push_str(&format!("Prompt: {}\n\n", part.raw_user_prompt().trim()));
            result.push_str(&format!(
                "Warp AI: {}\n\n",
                part.raw_assistant_answer().trim()
            ));
        }

        ctx.clipboard()
            .write(ClipboardContent::plain_text(result.trim().to_string()));
    }

    fn should_render_zero_state(&self, app: &AppContext) -> bool {
        self.transcript(app).is_empty()
            && matches!(self.request_status(app), RequestStatus::NotInFlight)
    }

    fn transcript<'a>(&self, app: &'a AppContext) -> &'a [TranscriptPart] {
        self.requests_model.as_ref(app).transcript()
    }

    fn request_status<'a>(&self, app: &'a AppContext) -> &'a RequestStatus {
        self.requests_model.as_ref(app).request_status()
    }

    fn num_remaining_reqs(&self, app: &AppContext) -> usize {
        self.requests_model.as_ref(app).num_remaining_reqs()
    }

    #[cfg(feature = "integration_tests")]
    pub fn editor(&self) -> &ViewHandle<EditorView> {
        &self.editor
    }
}

/// All rendering related capabilities.
impl AIAssistantPanelView {
    fn render_title_bar(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let mut header = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        Icon::new(AI_ASSISTANT_SVG_PATH, *AI_ASSISTANT_LOGO_COLOR).finish(),
                    )
                    .with_height(LOGO_SIZE)
                    .with_width(LOGO_SIZE)
                    .finish(),
                )
                .with_padding_right(4.)
                .finish(),
            )
            .with_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .wrappable_text(AI_ASSISTANT_FEATURE_NAME.to_string(), false)
                        .with_style(UiComponentStyles {
                            font_family_id: Some(appearance.ui_font_family()),
                            font_size: Some(TITLE_FONT_SIZE),
                            font_weight: Some(warpui::fonts::Weight::Semibold),
                            font_color: Some(appearance.theme().active_ui_text_color().into()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .finish(),
            )
            .with_child(Shrinkable::new(1., Empty::new().finish()).finish());

        // Add the copy and restart buttons iff the transcript is non-empty or there's a request in flight;
        if !self.transcript(app).is_empty()
            || matches!(self.request_status(app), RequestStatus::InFlight { .. })
        {
            header.add_child(
                Container::new(Align::new(self.render_restart_button(appearance)).finish())
                    .with_margin_right(4.)
                    .finish(),
            );

            header.add_child(
                Container::new(self.render_copy_transcript_button(appearance, app))
                    .with_margin_right(4.)
                    .finish(),
            );
        }

        // Add the close button
        header.add_child(
            Container::new(
                icon_button(
                    appearance,
                    crate::ui_components::icons::Icon::X,
                    false,
                    self.mouse_state_handles.close_panel_state.clone(),
                )
                .build()
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(AIAssistantAction::ClosePanel))
                .with_cursor(Cursor::PointingHand)
                .finish(),
            )
            .finish(),
        );

        header.finish()
    }

    fn render_copy_transcript_button(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let tooltip_background = appearance.theme().surface_1().into_solid();
        let ui_builder = appearance.ui_builder().clone();
        let tooltip = crate::i18n::tr_static(app, "Copy transcript to clipboard").to_owned();
        icon_button(
            appearance,
            crate::ui_components::icons::Icon::Copy,
            false,
            self.mouse_state_handles.copy_transcript_button.clone(),
        )
        .with_tooltip(move || {
            let tool_tip_style = UiComponentStyles {
                background: Some(Fill::Solid(tooltip_background)),
                ..Default::default()
            };
            ui_builder
                .tool_tip(tooltip.clone())
                .with_style(tool_tip_style)
                .build()
                .finish()
        })
        .build()
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(AIAssistantAction::CopyTranscript))
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    fn render_restart_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let default_styles = UiComponentStyles {
            border_width: None,
            font_color: Some(appearance.theme().active_ui_text_color().into()),
            font_size: Some(12.),
            font_family_id: Some(appearance.ui_font_family()),
            padding: Some(Coords {
                top: 4.,
                bottom: 4.,
                left: 8.,
                right: 8.,
            }),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
            ..Default::default()
        };

        let hover_style = UiComponentStyles {
            background: Some(appearance.theme().surface_3().into()),
            ..default_styles
        };

        appearance
            .ui_builder()
            .button_with_custom_styles(
                ButtonVariant::Text,
                self.mouse_state_handles.reset_context_button.clone(),
                default_styles,
                Some(hover_style),
                Some(hover_style),
                Some(hover_style),
            )
            .with_text_label(RESTART_BUTTON_TEXT.to_owned())
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(AIAssistantAction::ResetContext))
            .with_cursor(Cursor::PointingHand)
            .finish()
    }

    fn render_editor_size_warning(
        &self,
        appearance: &Appearance,
        buffer_len: usize,
    ) -> Box<dyn Element> {
        Flex::row()
            .with_children([
                Container::new(
                    Text::new_inline(
                        "Character limit exceeded.",
                        appearance.ui_font_family(),
                        BODY_FONT_SIZE,
                    )
                    .with_style(Properties {
                        weight: warpui::fonts::Weight::Bold,
                        ..Default::default()
                    })
                    .with_color(appearance.theme().ui_error_color())
                    .finish(),
                )
                .with_margin_right(10.)
                .finish(),
                Text::new_inline(
                    format!("{buffer_len} / {PROMPT_CHARACTER_LIMIT}"),
                    appearance.ui_font_family(),
                    BODY_FONT_SIZE,
                )
                .with_color(appearance.theme().ui_error_color())
                .finish(),
            ])
            .finish()
    }

    fn render_editor(&self) -> Box<dyn Element> {
        SavePosition::new(
            ConstrainedBox::new(ChildView::new(&self.editor).finish())
                .with_max_height(MAX_EDITOR_HEIGHT)
                .finish(),
            EDITOR_SAVE_POSITION_ID,
        )
        .finish()
    }

    fn render_input_suggestions_menu(&self, appearance: &Appearance) -> Box<dyn Element> {
        ConstrainedBox::new(
            Container::new(ChildView::new(&self.input_suggestions_view).finish())
                .with_uniform_margin(10.)
                .with_background(appearance.theme().surface_2())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_border(Border::all(1.0).with_border_fill(appearance.theme().outline()))
                .finish(),
        )
        .with_max_height(MAX_INPUT_SUGGESTIONS_HEIGHT)
        .finish()
    }

    fn render_zero_state(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();

        // TODO: fill should be Copy. it's cheap (a few ColorU's at most).
        let sub_text_color = blended_colors::text_sub(theme, theme.surface_2());
        let thick_overlay_color = theme.surface_3();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                ConstrainedBox::new(Icon::new(AI_ASSISTANT_SVG_PATH, thick_overlay_color).finish())
                    .with_height(44.)
                    .with_width(44.)
                    .finish(),
            )
            .with_child(
                Container::new(
                    Text::new_inline(ASK_AI_ASSISTANT_TEXT, appearance.ui_font_family(), 14.)
                        .with_color(sub_text_color)
                        .finish(),
                )
                .with_margin_top(8.)
                .finish(),
            );

        if self.num_remaining_reqs(app) > 0 {
            column.add_children([
                Container::new(render_prepared_response_button(
                    appearance,
                    self.mouse_state_handles.git_zero_state_prompt.clone(),
                    Some(300.),
                    None,
                    GIT_ZERO_STATE_PROMPT,
                ))
                .with_margin_top(20.)
                .with_margin_bottom(10.)
                .finish(),
                Container::new(render_prepared_response_button(
                    appearance,
                    self.mouse_state_handles.files_zero_state_prompt.clone(),
                    Some(300.),
                    None,
                    FILES_ZERO_STATE_PROMPT,
                ))
                .with_margin_bottom(10.)
                .finish(),
                Container::new(render_prepared_response_button(
                    appearance,
                    self.mouse_state_handles.script_zero_state_prompt.clone(),
                    Some(300.),
                    None,
                    SCRIPT_ZERO_STATE_PROMPT,
                ))
                .finish(),
            ]);
        }

        column.add_child(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_child(
                        Container::new(
                            ConstrainedBox::new(
                                Icon::new(INFO_ICON_SVG_PATH, theme.active_ui_text_color())
                                    .finish(),
                            )
                            .with_height(15.)
                            .with_width(15.)
                            .finish(),
                        )
                        .with_margin_right(8.)
                        .finish(),
                    )
                    .with_child(
                        Shrinkable::new(
                            1.,
                            appearance
                                .ui_builder()
                                .wrappable_text(ZERO_STATE_HELP_TEXT.to_string(), true)
                                .with_style(UiComponentStyles {
                                    font_family_id: Some(appearance.ui_font_family()),
                                    font_size: Some(ZERO_STATE_HELP_TEXT_FONT_SIZE),
                                    font_color: Some(theme.active_ui_text_color().into()),
                                    ..Default::default()
                                })
                                .build()
                                .finish(),
                        )
                        .finish(),
                    )
                    .finish(),
            )
            .with_margin_top(25.)
            .finish(),
        );

        let is_custom_llm_enabled: bool = UserWorkspaces::as_ref(app)
            .current_team()
            .is_some_and(|team| team.is_custom_llm_enabled());

        if !is_custom_llm_enabled {
            column.add_child(
                Container::new(render_request_limit_info(
                    &self.requests_model,
                    app,
                    appearance,
                ))
                .with_margin_top(18.)
                .finish(),
            );
        }

        Container::new(column.finish())
            .with_margin_left(12.)
            .with_margin_right(12.)
            .with_padding_top(50.)
            .finish()
    }
}

impl Entity for AIAssistantPanelView {
    type Event = AIAssistantPanelEvent;
}

impl TypedActionView for AIAssistantPanelView {
    type Action = AIAssistantAction;

    fn handle_action(&mut self, action: &AIAssistantAction, ctx: &mut ViewContext<Self>) {
        use AIAssistantAction::*;

        match action {
            ResetContext => {
                self.reset_context(ctx);
                send_telemetry_from_ctx!(
                    TelemetryEvent::WarpAIAction {
                        action_type: WarpAIActionType::Restart
                    },
                    ctx
                );
            }
            CopyTranscript => {
                self.copy_transcript(ctx);
                send_telemetry_from_ctx!(
                    TelemetryEvent::WarpAIAction {
                        action_type: WarpAIActionType::CopyTranscript
                    },
                    ctx
                );
            }
            ClosePanel => {
                ctx.emit(AIAssistantPanelEvent::ClosePanel);
            }
            PreparedPrompt(prompt) => {
                self.issue_request(prompt.to_string(), ctx);
                send_telemetry_from_ctx!(TelemetryEvent::UsedWarpAIPreparedPrompt { prompt }, ctx);
            }
            ClickedUrl(url) => {
                ctx.open_url(&url.url);
            }
            CopyAnswerToClipboard(content) => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(content.to_string()));
                send_telemetry_from_ctx!(
                    TelemetryEvent::WarpAIAction {
                        action_type: WarpAIActionType::CopyAnswer
                    },
                    ctx
                );
            }
            FocusTerminalInput => ctx.emit(AIAssistantPanelEvent::FocusTerminalInput),
            FocusEditor => {
                self.focus_state = PanelFocusState::Editor;
                ctx.focus_self();
            }
        }
    }
}

impl View for AIAssistantPanelView {
    fn ui_name() -> &'static str {
        "AIAssistantPanel"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            match &self.focus_state {
                PanelFocusState::Editor => {
                    self.transcript_view.update(ctx, |transcript_view, ctx| {
                        transcript_view.clear_selected_block(ctx)
                    });
                    ctx.focus(&self.editor);
                }
                PanelFocusState::Transcript => ctx.focus(&self.transcript_view),
            }
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut panel = Flex::column().with_main_axis_size(MainAxisSize::Max);

        let should_render_zero_state = self.should_render_zero_state(app);
        let body = if should_render_zero_state {
            Align::new(self.render_zero_state(appearance, app)).finish()
        } else {
            Align::new(ChildView::new(&self.transcript_view).finish())
                .top_center()
                .finish()
        };
        panel.add_child(Shrinkable::new(1., body).finish());

        if matches!(self.request_status(app), RequestStatus::NotInFlight) {
            let buffer_text = self.editor.as_ref(app).buffer_text(app);
            if self.is_prompt_too_long(buffer_text.as_str()) {
                panel.add_child(
                    Container::new(
                        self.render_editor_size_warning(appearance, buffer_text.chars().count()),
                    )
                    .with_padding_left(PANEL_HORIZONTAL_PADDING)
                    .with_padding_bottom(5.)
                    .with_padding_top(10.)
                    .finish(),
                );
            }

            panel.add_child(
                Container::new(self.render_editor())
                    .with_uniform_padding(EDITOR_MARGIN)
                    .finish(),
            );
        }

        let mut stack = Stack::new().with_child(panel.finish());

        stack.add_positioned_overlay_child(
            ConstrainedBox::new(
                Container::new(self.render_title_bar(appearance, app))
                    .with_padding_top(HEADER_VERTICAL_PADDING)
                    .with_padding_bottom(HEADER_VERTICAL_PADDING)
                    .with_padding_left(PANEL_HORIZONTAL_PADDING)
                    .with_padding_right(PANEL_HORIZONTAL_PADDING)
                    .finish(),
            )
            .with_height(HEADER_HEIGHT)
            .finish(),
            OffsetPositioning::offset_from_parent(
                vec2f(0., HEADER_HEIGHT),
                warpui::elements::ParentOffsetBounds::Unbounded,
                ParentAnchor::TopLeft,
                ChildAnchor::BottomLeft,
            ),
        );

        if matches!(
            self.input_suggestions_mode,
            InputSuggestionsMode::Open { .. }
        ) {
            stack.add_positioned_overlay_child(
                self.render_input_suggestions_menu(appearance),
                OffsetPositioning::offset_from_save_position_element(
                    EDITOR_SAVE_POSITION_ID,
                    Vector2F::new(0., -10.),
                    PositionedElementOffsetBounds::ParentByPosition,
                    PositionedElementAnchor::TopLeft,
                    ChildAnchor::BottomLeft,
                ),
            );
        }

        let styled_panel = Container::new(stack.finish())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)));

        let clickable_panel =
            EventHandler::new(styled_panel.finish()).on_left_mouse_down(|ctx, _, _| {
                ctx.dispatch_typed_action(AIAssistantAction::FocusEditor);
                DispatchEventResult::StopPropagation
            });

        Resizable::new(
            self.resizable_state_handle.clone(),
            clickable_panel.finish(),
        )
        .on_resize(move |ctx, _| ctx.notify())
        .with_dragbar_side(DragBarSide::Left)
        .with_bounds_callback(Box::new(|window_bounds| {
            (
                MIN_PANEL_WIDTH,
                (window_bounds.x() - MIN_REMAINING_WINDOW_SIZE).max(MIN_PANEL_WIDTH),
            )
        }))
        .finish()
    }
}
