use markdown_parser::markdown_parser::RUNNABLE_BLOCK_MARKDOWN_LANG;
use markdown_parser::CodeBlockText;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::builder::AnimatedButtonOptions;
use warpui::clipboard::ClipboardContent;
use warpui::elements::{
    Align, Border, ChildAnchor, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, DispatchEventResult, EventHandler, Fill, Flex,
    FormattedTextElement, HyperlinkUrl, Icon, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentAnchor, ParentElement, Radius, SavePosition, ScrollbarWidth, Shrinkable, Stack, Text,
    Wrap,
};
use warpui::keymap::Keystroke;
use warpui::platform::Cursor;
use warpui::presenter::ChildView;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::units::{IntoPixels, Pixels};
use warpui::{
    AppContext, BlurContext, Element, Entity, FocusContext, ModelHandle, SingletonEntity,
    TypedActionView, View, ViewContext, ViewHandle, WeakViewHandle,
};

use super::panel::{HEADER_HEIGHT, HEXAGON_ALERT_SVG_PATH};
use super::requests::{RequestStatus, Requests};
use super::utils::{
    code_block_position_id, markdown_segments_from_text, render_prepared_response_button,
    render_request_limit_info, save_as_workflow_position_id, AssistantTranscriptPart,
    CodeBlockIndex, FormattedTranscriptMessage, MarkdownSegment, TranscriptPartSubType,
};
use super::AI_ASSISTANT_SVG_PATH;
use crate::appearance::Appearance;
use crate::editor::{EditorOptions, EditorView, PropagateAndNoOpNavigationKeys, TextOptions};
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::{SaveAsWorkflowModalSource, TelemetryEvent, WarpAIActionType};
use crate::ui_components::blended_colors;
use crate::workspaces::user_workspaces::UserWorkspaces;

const TRANSCRIPT_POSITION_ID: &str = "ai_assistant::transcript";

const TERMINAL_INPUT_SVG_PATH: &str = "bundled/svg/terminal-input.svg";
const USER_ICON_SVG_PATH: &str = "bundled/svg/user.svg";
const SAVE_WORKFLOW_ICON_PATH: &str = "bundled/svg/workflow.svg";
const PIN_ICON_PATH: &str = "bundled/svg/pin-01.svg";
const PIN_FILLED_ICON_PATH: &str = "bundled/svg/pin-filled.svg";

const BODY_FONT_SIZE: f32 = 13.;
const CODE_FONT_SIZE: f32 = 12.;
const WARNING_MESSAGE_FONT_SIZE: f32 = 10.;

const PANEL_LEFT_MARGIN: f32 = 15.;
const DETAILS_BOTTOM_MARGIN: f32 = 12.;

const COPY_BUTTON_SIZE: f32 = 14.;
const TERMINAL_INPUT_BUTTON_SIZE: f32 = 20.;
const SAVE_AS_WORKFLOW_BUTTON_SIZE: f32 = 20.;
const PIN_BUTTON_SIZE: f32 = 16.;
const MAX_NOTES_HEIGHT: f32 = 140.;

const HOW_DO_I_FIX_PROMPT: &str = "How do I fix this?";
const SHOW_EXAMPLES_PROMPT: &str = "Show examples.";
const WHAT_TO_DO_NEXT_PROMPT: &str = "What should I do next?";
const IN_FLIGHT_REQUEST_TEXT: &str = "Generating answer...";
const ACCURACY_NOTICE_TEXT: &str = "AI responses can be inaccurate.";
const MISSING_CONTEXT_NOTICE_TEXT: &str =
    "Warp AI might forget earlier answers as conversations get long.";
const NOTES_PLACEHOLDER_TEXT: &str = " Notes...";

lazy_static::lazy_static! {
    static ref SCROLL_BUFFER_OFFSET_PX: Pixels = (10.).into_pixels();
}

fn transcript_part_position_id(index: usize) -> String {
    format!("ai_assistant::transcript_part::{index}")
}

fn pinned_message_preview(text: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 120;

    let mut preview = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if preview.chars().count() > MAX_PREVIEW_CHARS {
        preview = preview
            .chars()
            .take(MAX_PREVIEW_CHARS)
            .collect::<String>()
            .trim_end()
            .to_string();
        preview.push('…');
    }
    preview
}

#[derive(Debug, Clone, Default)]
pub struct CodeBlockMouseStateHandles {
    pub play_button: MouseStateHandle,
    pub play_button_tooltip: MouseStateHandle,
    pub copy_button: MouseStateHandle,
    pub copy_button_tooltip: MouseStateHandle,
    pub save_as_workflow_button: MouseStateHandle,
    pub save_as_workflow_button_tooltip: MouseStateHandle,
}

#[derive(Default)]
struct MouseStateHandles {
    show_examples_button: MouseStateHandle,
    what_to_do_next_button: MouseStateHandle,
    how_do_i_fix_button: MouseStateHandle,
}

/// A view to render a Q/A style transcript.
pub struct Transcript {
    view_handle: WeakViewHandle<Transcript>,

    requests_model: ModelHandle<Requests>,
    notes_editor: ViewHandle<EditorView>,
    selected_code_block: Option<CodeBlockIndex>,

    clipped_scroll_state: ClippedScrollStateHandle,
    mouse_state_handles: MouseStateHandles,
}

#[derive(Debug, Clone)]
pub enum TranscriptAction {
    CopyAnswerToClipboard {
        transcript_part_index: usize,
    },
    CopyCodeToClipboard {
        code_block_index: CodeBlockIndex,
    },
    PasteInTerminalInput {
        code_block_index: CodeBlockIndex,
    },
    OpenWorkflowModal(CodeBlockIndex),
    TogglePinnedAnswer {
        transcript_part_index: usize,
    },
    TogglePinnedAnswerCompleted {
        transcript_part_index: usize,
    },
    JumpToPinnedAnswer {
        transcript_part_index: usize,
    },
    ClickedCodeBlock {
        code_block_index: CodeBlockIndex,
    },
    ClickedUrl(HyperlinkUrl),
    Keydown(Keystroke),
    /// A mouse down event outside of the other clickable elements (e.g. buttons, code blocks, etc.)
    MouseDown,
}

pub enum TranscriptEvent {
    PasteInTerminalInput { code_block_index: CodeBlockIndex },
    FocusEditor,
    FocusTranscript,
    ClickedCodeBlock,
    OpenWorkflowModalWithCommand(String),
}

impl Entity for Transcript {
    type Event = TranscriptEvent;
}

impl TypedActionView for Transcript {
    type Action = TranscriptAction;

    fn handle_action(&mut self, action: &TranscriptAction, ctx: &mut ViewContext<Self>) {
        use TranscriptAction::*;

        match action {
            CopyAnswerToClipboard {
                transcript_part_index,
            } => {
                let answer = self
                    .requests_model
                    .as_ref(ctx)
                    .transcript()
                    .get(*transcript_part_index)
                    .map(|p| p.assistant.formatted_message.raw.clone());

                if let Some(answer) = answer {
                    ctx.clipboard().write(ClipboardContent::plain_text(answer));
                }
                send_telemetry_from_ctx!(
                    TelemetryEvent::WarpAIAction {
                        action_type: WarpAIActionType::CopyAnswer
                    },
                    ctx
                );
            }
            CopyCodeToClipboard { code_block_index } => {
                self.copy_code_to_clipboard(*code_block_index, ctx);
            }
            PasteInTerminalInput { code_block_index } => {
                self.paste_in_terminal_input(*code_block_index, ctx);
            }
            OpenWorkflowModal(code_block_index) => self.open_workflow_modal(*code_block_index, ctx),
            TogglePinnedAnswer {
                transcript_part_index,
            } => {
                self.requests_model.update(ctx, |requests, ctx| {
                    requests.toggle_pinned_assistant_message(*transcript_part_index, ctx);
                });
            }
            TogglePinnedAnswerCompleted {
                transcript_part_index,
            } => {
                self.requests_model.update(ctx, |requests, ctx| {
                    requests.toggle_pinned_assistant_message_completed(*transcript_part_index, ctx);
                });
            }
            JumpToPinnedAnswer {
                transcript_part_index,
            } => self.scroll_to_transcript_part(*transcript_part_index, ctx),
            ClickedUrl(url) => {
                ctx.open_url(&url.url);
            }
            ClickedCodeBlock { code_block_index } => {
                self.selected_code_block = Some(*code_block_index);
                ctx.emit(TranscriptEvent::ClickedCodeBlock);
                ctx.notify();
            }
            Keydown(keystroke) => self.handle_keydown(keystroke, ctx),
            MouseDown => {
                if self.selected_code_block.is_none() {
                    ctx.emit(TranscriptEvent::FocusEditor);
                } else {
                    ctx.emit(TranscriptEvent::FocusTranscript);
                }
                ctx.notify();
            }
        }
    }
}

impl Transcript {
    pub fn new(requests_model: &ModelHandle<Requests>, ctx: &mut ViewContext<Self>) -> Self {
        ctx.observe(requests_model, |_, _, ctx| ctx.notify());
        let notes_editor = {
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
        notes_editor.update(ctx, |editor, ctx| {
            editor.set_placeholder_text(NOTES_PLACEHOLDER_TEXT, ctx)
        });

        Self {
            view_handle: ctx.handle(),
            requests_model: requests_model.to_owned(),
            notes_editor,
            selected_code_block: None,
            clipped_scroll_state: Default::default(),
            mouse_state_handles: Default::default(),
        }
    }

    fn copy_code_to_clipboard(
        &mut self,
        code_block_index: CodeBlockIndex,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(code) = self.code_for_index(code_block_index, ctx) {
            ctx.clipboard().write(ClipboardContent::plain_text(code));
        }

        send_telemetry_from_ctx!(
            TelemetryEvent::WarpAIAction {
                action_type: WarpAIActionType::CopyCode
            },
            ctx
        );
    }

    fn paste_in_terminal_input(
        &mut self,
        code_block_index: CodeBlockIndex,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.emit(TranscriptEvent::PasteInTerminalInput { code_block_index });
        send_telemetry_from_ctx!(
            TelemetryEvent::WarpAIAction {
                action_type: WarpAIActionType::InsertIntoInput
            },
            ctx
        );
    }

    fn open_workflow_modal(
        &mut self,
        code_block_index: CodeBlockIndex,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(code) = self.code_for_index(code_block_index, ctx) {
            ctx.emit(TranscriptEvent::OpenWorkflowModalWithCommand(code));
        }

        send_telemetry_from_ctx!(
            TelemetryEvent::SaveAsWorkflowModal {
                source: SaveAsWorkflowModalSource::WarpAIPanel
            },
            ctx
        );
    }

    fn handle_keydown(&mut self, keystroke: &Keystroke, ctx: &mut ViewContext<Self>) {
        let Some(selected_block_index) = self.selected_code_block else {
            return;
        };

        if keystroke.key == "down" {
            let new_index = self.next_code_block_index(ctx);
            if new_index.is_some() {
                self.selected_code_block = new_index;
            } else if keystroke.cmd {
                self.selected_code_block = None;
                self.scroll_to_bottom_of_transcript(ctx);
                ctx.emit(TranscriptEvent::FocusEditor);
            }
            ctx.notify();
        } else if keystroke.key == "up" {
            let new_index = self.previous_code_block_index(ctx);
            if new_index.is_some() {
                self.selected_code_block = new_index;
                ctx.notify();
            }
        } else if keystroke.cmd && keystroke.key == "c" {
            self.copy_code_to_clipboard(selected_block_index, ctx);
        } else if keystroke.cmd && keystroke.key == "enter" {
            self.paste_in_terminal_input(selected_block_index, ctx);
        } else if keystroke.cmd && keystroke.key == "s" {
            self.open_workflow_modal(selected_block_index, ctx);
        } else if keystroke.key == "escape" {
            self.selected_code_block = None;
            ctx.emit(TranscriptEvent::FocusEditor);
            ctx.notify();
        }

        // If we took an action on a code block or changed code blocks, let's scroll to it
        // so the user knows what's going on.
        if let Some(selected_code_block) = self.selected_code_block {
            self.scroll_to_code_block(selected_code_block, ctx);
        }
    }

    /// Only scrolls to the code block if it isn't already in the viewport.
    fn scroll_to_code_block(
        &mut self,
        code_block_index: CodeBlockIndex,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(transcript_pos) = ctx.element_position_by_id(TRANSCRIPT_POSITION_ID) else {
            return;
        };
        let Some(code_block_pos) =
            ctx.element_position_by_id(code_block_position_id(code_block_index))
        else {
            return;
        };

        let current_scroll_top_px = self.clipped_scroll_state.scroll_start();
        let viewable_transcript_height_px = transcript_pos.height().into_pixels();
        let code_block_start_y_px =
            code_block_pos.origin_y().into_pixels() - transcript_pos.origin_y().into_pixels();
        let code_block_end_y_px =
            code_block_pos.origin_y().into_pixels() + code_block_pos.height().into_pixels();

        // We only need to scroll if either the start of the code block is cut off or the end is cut off.
        if code_block_start_y_px < Pixels::zero()
            || code_block_end_y_px > viewable_transcript_height_px
        {
            // In the case that the new scroll top exceeds the max scroll top, the after_layout
            // of clipped scrollable will re-adjust accordingly, so this is safe.
            self.clipped_scroll_state.scroll_to(
                current_scroll_top_px + code_block_start_y_px - *SCROLL_BUFFER_OFFSET_PX,
            );
            ctx.notify();
        }
    }

    fn scroll_to_transcript_part(
        &mut self,
        transcript_part_index: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(transcript_pos) = ctx.element_position_by_id(TRANSCRIPT_POSITION_ID) else {
            return;
        };
        let Some(part_pos) =
            ctx.element_position_by_id(transcript_part_position_id(transcript_part_index))
        else {
            return;
        };

        let current_scroll_top_px = self.clipped_scroll_state.scroll_start();
        let part_start_y_px =
            part_pos.origin_y().into_pixels() - transcript_pos.origin_y().into_pixels();

        self.clipped_scroll_state
            .scroll_to(current_scroll_top_px + part_start_y_px - *SCROLL_BUFFER_OFFSET_PX);
        ctx.notify();
    }

    pub fn scroll_to_bottom_of_transcript(&mut self, ctx: &mut ViewContext<Self>) {
        // This relies on the fact that the clipped scrollable will recompute
        // the scroll_top in after_layout if it exceeds the true max.
        self.clipped_scroll_state.scroll_to(f32::MAX.into_pixels());
        ctx.notify();
    }

    fn previous_code_block_index(&self, ctx: &mut ViewContext<Self>) -> Option<CodeBlockIndex> {
        let transcript = self.requests_model.as_ref(ctx).transcript();
        let selected_code_block_index = self.selected_code_block?;
        let transcript_index = selected_code_block_index.transcript_index();

        // Try to find the prev code block in the current part.
        let found = transcript
            .get(transcript_index)
            .and_then(|p| p.prev_code_block_index(selected_code_block_index));

        // If it's not in the current part, then take the last code block from the closest
        // transcript part to this one (in reverse).
        found.or_else(|| {
            transcript
                .get(..transcript_index)?
                .iter()
                .rev()
                .find_map(|part| part.last_code_block_index())
        })
    }

    fn next_code_block_index(&self, ctx: &mut ViewContext<Self>) -> Option<CodeBlockIndex> {
        let transcript = self.requests_model.as_ref(ctx).transcript();
        let selected_code_block_index = self.selected_code_block?;
        let transcript_index = selected_code_block_index.transcript_index();

        // Try to find the next code block in the current part.
        let found = transcript
            .get(transcript_index)
            .and_then(|p| p.next_code_block_index(selected_code_block_index));

        // If it's not in the current part, then take the first code block from the closest
        // transcript part to this one (in sequence).
        found.or_else(|| {
            transcript
                .get(transcript_index + 1..)?
                .iter()
                .find_map(|part| part.first_code_block_index())
        })
    }

    pub fn select_last_code_block(&mut self, ctx: &mut ViewContext<Self>) {
        let transcript = self.requests_model.as_ref(ctx).transcript();
        // The last code block will be the last code block in the first transcript part starting from the end.
        let code_block_index = transcript
            .iter()
            .rev()
            .find_map(|p| p.last_code_block_index());
        self.selected_code_block = code_block_index;

        if let Some(new_code_block) = self.selected_code_block {
            self.scroll_to_code_block(new_code_block, ctx);
            ctx.emit(TranscriptEvent::ClickedCodeBlock);
        }
        ctx.notify();
    }

    pub fn clear_selected_block(&mut self, ctx: &mut ViewContext<Self>) {
        self.selected_code_block = None;
        ctx.notify();
    }

    pub fn code_for_index(
        &self,
        code_block_index: CodeBlockIndex,
        app: &AppContext,
    ) -> Option<String> {
        let transcript = self.requests_model.as_ref(app).transcript();
        transcript
            .get(code_block_index.transcript_index())
            .and_then(|p| p.code_for_block(code_block_index).map(ToOwned::to_owned))
    }

    pub fn reset(&mut self, ctx: &mut ViewContext<Self>) {
        self.selected_code_block = None;
        ctx.notify();
    }
}

/// Rendering-related implementation.
impl Transcript {
    fn render_code_block_actions(
        &self,
        code_block_index: CodeBlockIndex,
        appearance: &Appearance,
        code_block_info: &CodeBlockText,
        mouse_state_handles: &CodeBlockMouseStateHandles,
    ) -> Box<dyn Element> {
        let mut buttons = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::End)
            .with_main_axis_size(MainAxisSize::Max);

        let copy_button = appearance
            .ui_builder()
            .copy_button(COPY_BUTTON_SIZE, mouse_state_handles.copy_button.clone())
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(TranscriptAction::CopyCodeToClipboard {
                    code_block_index,
                });
            })
            .finish();

        buttons.add_child(appearance.ui_builder().tool_tip_on_element(
            "Copy code to clipboard [Cmd + C]".to_string(),
            mouse_state_handles.copy_button_tooltip.clone(),
            copy_button,
            ParentAnchor::TopRight,
            ChildAnchor::BottomRight,
            vec2f(0., -5.),
        ));

        if code_block_info.lang.ends_with("sh")
            || code_block_info.lang == RUNNABLE_BLOCK_MARKDOWN_LANG
        {
            let insert_button = appearance
                .ui_builder()
                .animated_button(
                    mouse_state_handles.play_button.clone(),
                    TERMINAL_INPUT_SVG_PATH,
                    AnimatedButtonOptions {
                        size: TERMINAL_INPUT_BUTTON_SIZE,
                        padding: Some(4.),
                        color: None,
                        with_accent_animations: true,
                        circular: true,
                    },
                )
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(TranscriptAction::PasteInTerminalInput {
                        code_block_index,
                    });
                })
                .with_cursor(Cursor::PointingHand)
                .finish();

            buttons.add_child(
                Container::new(appearance.ui_builder().tool_tip_on_element(
                    "Insert code into terminal input [Cmd + Enter]".to_string(),
                    mouse_state_handles.play_button_tooltip.clone(),
                    insert_button,
                    ParentAnchor::TopRight,
                    ChildAnchor::BottomRight,
                    vec2f(0., -5.),
                ))
                .with_margin_left(10.)
                .with_margin_bottom(-4.)
                .finish(),
            );

            let save_as_workflow_button = appearance
                .ui_builder()
                .animated_button(
                    mouse_state_handles.save_as_workflow_button.clone(),
                    SAVE_WORKFLOW_ICON_PATH,
                    AnimatedButtonOptions {
                        size: SAVE_AS_WORKFLOW_BUTTON_SIZE,
                        padding: Some(4.),
                        color: None,
                        with_accent_animations: true,
                        circular: true,
                    },
                )
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(TranscriptAction::OpenWorkflowModal(code_block_index))
                })
                .with_cursor(Cursor::PointingHand)
                .finish();

            buttons.add_child(
                SavePosition::new(
                    Container::new(appearance.ui_builder().tool_tip_on_element(
                        "Save as workflow [Cmd + S]".to_string(),
                        mouse_state_handles.save_as_workflow_button_tooltip.clone(),
                        save_as_workflow_button,
                        ParentAnchor::TopRight,
                        ChildAnchor::BottomRight,
                        vec2f(0., -5.),
                    ))
                    .with_margin_left(2.)
                    .with_margin_bottom(-4.)
                    .finish(),
                    &save_as_workflow_position_id(code_block_index),
                )
                .finish(),
            );
        }
        buttons.finish()
    }

    fn render_assistant_answer(
        &self,
        transcript_part_index: usize,
        part: &AssistantTranscriptPart,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        let background_color = theme.surface_2().into_solid();
        let icon = if part.is_error {
            ConstrainedBox::new(
                Icon::new(HEXAGON_ALERT_SVG_PATH, appearance.theme().ui_error_color()).finish(),
            )
            .with_height(18.)
            .with_width(18.)
            .finish()
        } else {
            ConstrainedBox::new(
                Icon::new(
                    AI_ASSISTANT_SVG_PATH,
                    theme.main_text_color(background_color.into()),
                )
                .finish(),
            )
            .with_height(16.)
            .with_width(16.)
            .finish()
        };

        let mut buttons = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        if let Some((tooltip_handle, button_handle)) =
            part.copy_all_tooltip_and_button_mouse_handles.clone()
        {
            let copy_button = appearance
                .ui_builder()
                .copy_button(16., button_handle)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(TranscriptAction::CopyAnswerToClipboard {
                        transcript_part_index,
                    })
                })
                .with_cursor(Cursor::PointingHand)
                .finish();

            buttons.add_child(appearance.ui_builder().tool_tip_on_element(
                "Copy answer to clipboard".to_string(),
                tooltip_handle,
                copy_button,
                ParentAnchor::TopRight,
                ChildAnchor::BottomRight,
                vec2f(0., -5.),
            ));
        }

        if let Some((tooltip_handle, button_handle)) =
            part.pin_tooltip_and_button_mouse_handles.clone()
        {
            let is_pinned = self
                .requests_model
                .as_ref(app)
                .is_assistant_message_pinned(transcript_part_index);
            let icon_path = if is_pinned {
                PIN_FILLED_ICON_PATH
            } else {
                PIN_ICON_PATH
            };
            let tooltip_text = if is_pinned {
                "Unpin answer"
            } else {
                "Pin answer"
            };
            let pin_button = appearance
                .ui_builder()
                .animated_button(
                    button_handle,
                    icon_path,
                    AnimatedButtonOptions {
                        size: PIN_BUTTON_SIZE,
                        padding: Some(3.),
                        color: None,
                        with_accent_animations: true,
                        circular: true,
                    },
                )
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(TranscriptAction::TogglePinnedAnswer {
                        transcript_part_index,
                    })
                })
                .with_cursor(Cursor::PointingHand)
                .finish();

            buttons.add_child(
                Container::new(appearance.ui_builder().tool_tip_on_element(
                    tooltip_text.to_string(),
                    tooltip_handle,
                    pin_button,
                    ParentAnchor::TopRight,
                    ChildAnchor::BottomRight,
                    vec2f(0., -5.),
                ))
                .with_margin_left(10.)
                .finish(),
            );
        }
        let bottom_right_element = Some(buttons.finish());

        self.render_message(
            &part.formatted_message,
            background_color,
            icon,
            bottom_right_element,
            appearance,
        )
    }

    fn render_user_prompt(
        &self,
        dialogue: &FormattedTranscriptMessage,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background_color = theme.surface_1().into_solid();
        let icon = ConstrainedBox::new(
            Icon::new(
                USER_ICON_SVG_PATH,
                theme.main_text_color(background_color.into()),
            )
            .finish(),
        )
        .with_height(16.)
        .with_width(16.)
        .finish();
        self.render_message(dialogue, background_color, icon, None, appearance)
    }

    fn render_notes_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(
            Flex::column()
                .with_child(
                    Text::new_inline("Notes", appearance.ui_font_family(), BODY_FONT_SIZE)
                        .with_color(theme.active_ui_text_color().into())
                        .finish(),
                )
                .with_child(
                    ConstrainedBox::new(
                        Container::new(ChildView::new(&self.notes_editor).finish())
                            .with_background_color(theme.surface_2().into_solid())
                            .with_border(Border::all(1.).with_border_fill(theme.outline()))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                            .with_uniform_padding(8.)
                            .with_margin_top(8.)
                            .finish(),
                    )
                    .with_max_height(MAX_NOTES_HEIGHT)
                    .finish(),
                )
                .finish(),
        )
        .with_padding_left(PANEL_LEFT_MARGIN)
        .with_padding_right(20.)
        .with_padding_top(16.)
        .with_padding_bottom(16.)
        .finish()
    }

    fn render_pinned_messages(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let requests = self.requests_model.as_ref(app);
        let transcript = requests.transcript();
        let mut column = Flex::column().with_child(
            Text::new_inline("Pinned", appearance.ui_font_family(), BODY_FONT_SIZE)
                .with_color(theme.active_ui_text_color().into())
                .finish(),
        );

        for pinned in requests.pinned_assistant_messages() {
            let Some(part) = transcript.get(pinned.transcript_part_index) else {
                continue;
            };
            let transcript_part_index = pinned.transcript_part_index;
            let checkbox = if pinned.completed { "☑" } else { "☐" };
            let checkbox = EventHandler::new(
                Container::new(
                    Text::new_inline(checkbox, appearance.ui_font_family(), BODY_FONT_SIZE)
                        .with_color(theme.active_ui_text_color().into())
                        .finish(),
                )
                .with_margin_right(8.)
                .finish(),
            )
            .on_left_mouse_down(move |ctx, _, _| {
                ctx.dispatch_typed_action(TranscriptAction::TogglePinnedAnswerCompleted {
                    transcript_part_index,
                });
                DispatchEventResult::StopPropagation
            })
            .finish();

            let row = EventHandler::new(
                Container::new(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(checkbox)
                        .with_child(
                            Shrinkable::new(
                                1.,
                                appearance
                                    .ui_builder()
                                    .wrappable_text(
                                        pinned_message_preview(part.raw_assistant_answer()),
                                        true,
                                    )
                                    .with_style(UiComponentStyles {
                                        font_size: Some(BODY_FONT_SIZE),
                                        font_family_id: Some(appearance.ui_font_family()),
                                        font_color: Some(
                                            theme.main_text_color(theme.surface_2()).into(),
                                        ),
                                        ..Default::default()
                                    })
                                    .build()
                                    .finish(),
                            )
                            .finish(),
                        )
                        .finish(),
                )
                .with_background_color(theme.surface_2().into_solid())
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                .with_uniform_padding(8.)
                .with_margin_top(8.)
                .finish(),
            )
            .on_left_mouse_down(move |ctx, _, _| {
                ctx.dispatch_typed_action(TranscriptAction::JumpToPinnedAnswer {
                    transcript_part_index,
                });
                DispatchEventResult::StopPropagation
            })
            .finish();

            column.add_child(row);
        }

        Container::new(column.finish())
            .with_padding_left(PANEL_LEFT_MARGIN)
            .with_padding_right(20.)
            .with_padding_bottom(16.)
            .finish()
    }

    /// Renders a single message (whether that be a user's prompt or assistant's answer).
    fn render_message(
        &self,
        dialogue: &FormattedTranscriptMessage,
        background_color: ColorU,
        icon: Box<dyn Element>,
        bottom_right_element: Option<Box<dyn Element>>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let inline_code_bg_color = appearance.theme().surface_3().into_solid();

        let body = if let Some(parts) = &dialogue.markdown {
            let mut column = Flex::column();
            for part in parts {
                let column_part = match part {
                    MarkdownSegment::Other {
                        formatted_text,
                        highlighted_hyperlink,
                    } => FormattedTextElement::new(
                        formatted_text.to_owned(),
                        BODY_FONT_SIZE,
                        appearance.ui_font_family(),
                        appearance.monospace_font_family(),
                        theme.main_text_color(theme.surface_2()).into_solid(),
                        highlighted_hyperlink.clone(),
                    )
                    .with_inline_code_properties(
                        Some(theme.nonactive_ui_text_color().into()),
                        Some(inline_code_bg_color),
                    )
                    .register_default_click_handlers(move |url, ctx, _| {
                        ctx.dispatch_typed_action(TranscriptAction::ClickedUrl(url));
                    })
                    .finish(),
                    MarkdownSegment::CodeBlock {
                        index,
                        code,
                        mouse_state_handles,
                    } => {
                        let actions = self.render_code_block_actions(
                            *index,
                            appearance,
                            code,
                            mouse_state_handles,
                        );
                        let code = code.code.clone();

                        let (border_fill, border_width, padding) =
                            if self.selected_code_block == Some(*index) {
                                (appearance.theme().accent(), 1.5, 11.5)
                            } else {
                                (appearance.theme().outline(), 1., 12.)
                            };

                        let code_block_index = *index;

                        EventHandler::new(
                            Container::new(
                                SavePosition::new(
                                    Container::new(
                                        Flex::column()
                                            .with_child(
                                                appearance
                                                    .ui_builder()
                                                    .wrappable_text(code, true)
                                                    .with_style(UiComponentStyles {
                                                        font_family_id: Some(
                                                            appearance.monospace_font_family(),
                                                        ),
                                                        font_size: Some(CODE_FONT_SIZE),
                                                        ..Default::default()
                                                    })
                                                    .build()
                                                    .with_margin_bottom(10.)
                                                    .finish(),
                                            )
                                            .with_child(actions)
                                            .finish(),
                                    )
                                    .with_uniform_padding(padding)
                                    .with_border(
                                        Border::all(border_width).with_border_fill(border_fill),
                                    )
                                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                                    .finish(),
                                    &code_block_position_id(code_block_index),
                                )
                                .finish(),
                            )
                            .with_margin_top(10.)
                            .with_margin_bottom(10.)
                            .finish(),
                        )
                        .on_left_mouse_down(move |ctx, _, _| {
                            ctx.dispatch_typed_action(TranscriptAction::ClickedCodeBlock {
                                code_block_index,
                            });
                            DispatchEventResult::StopPropagation
                        })
                        .finish()
                    }
                };

                column.add_child(column_part);
            }

            column.finish()
        } else {
            // If we don't have the markdown representation, just render it as basic text.
            appearance
                .ui_builder()
                .wrappable_text(dialogue.raw.to_owned(), true)
                .with_style(UiComponentStyles {
                    font_size: Some(BODY_FONT_SIZE),
                    ..Default::default()
                })
                .build()
                .finish()
        };

        let mut final_col = Flex::column().with_child(body);

        if let Some(bottom_right_element) = bottom_right_element {
            final_col.add_child(
                Align::new(
                    Container::new(bottom_right_element)
                        .with_margin_top(16.)
                        .finish(),
                )
                .right()
                .finish(),
            );
        }

        let row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(
                Container::new(icon)
                    .with_margin_right(12.)
                    .with_margin_top(3.)
                    .finish(),
            )
            .with_child(Shrinkable::new(1., Container::new(final_col.finish()).finish()).finish());

        Container::new(row.finish())
            .with_background_color(background_color)
            .with_padding_left(PANEL_LEFT_MARGIN)
            .with_padding_top(16.)
            .with_padding_bottom(16.)
            .with_padding_right(20.)
            .finish()
    }

    fn render_prepared_responses(&self, appearance: &Appearance) -> Box<dyn Element> {
        Wrap::row()
            .with_run_spacing(10.)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_child(render_prepared_response_button(
                appearance,
                self.mouse_state_handles.what_to_do_next_button.clone(),
                None,
                Some(8.),
                WHAT_TO_DO_NEXT_PROMPT,
            ))
            .with_child(
                Container::new(render_prepared_response_button(
                    appearance,
                    self.mouse_state_handles.show_examples_button.clone(),
                    None,
                    Some(8.),
                    SHOW_EXAMPLES_PROMPT,
                ))
                .with_margin_left(10.)
                .with_margin_right(10.)
                .finish(),
            )
            .with_child(render_prepared_response_button(
                appearance,
                self.mouse_state_handles.how_do_i_fix_button.clone(),
                None,
                Some(8.),
                HOW_DO_I_FIX_PROMPT,
            ))
            .finish()
    }

    fn render_warning_message(&self, message: String, appearance: &Appearance) -> Box<dyn Element> {
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_child(
                Text::new_inline(
                    message,
                    appearance.ui_font_family(),
                    WARNING_MESSAGE_FONT_SIZE,
                )
                .with_color(blended_colors::text_sub(
                    appearance.theme(),
                    appearance.theme().background(),
                ))
                .finish(),
            )
            .finish()
    }
}

impl View for Transcript {
    fn ui_name() -> &'static str {
        "AIAssistantTranscript"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let transcript = self.requests_model.as_ref(app).transcript();
        let request_status = self.requests_model.as_ref(app).request_status();
        let num_remaining_reqs = self.requests_model.as_ref(app).num_remaining_reqs();

        let mut blocks = Flex::column();
        blocks.add_child(self.render_notes_section(appearance));
        if !self
            .requests_model
            .as_ref(app)
            .pinned_assistant_messages()
            .is_empty()
        {
            blocks.add_child(self.render_pinned_messages(appearance, app));
        }
        for (index, part) in transcript.iter().enumerate() {
            blocks.add_child(self.render_user_prompt(&part.user, appearance));
            blocks.add_child(
                SavePosition::new(
                    self.render_assistant_answer(index, &part.assistant, appearance, app),
                    &transcript_part_position_id(index),
                )
                .finish(),
            );
        }

        if let RequestStatus::InFlight { request, .. } = request_status {
            blocks.add_child(self.render_user_prompt(request, appearance));

            let transcript_part_index = transcript.len();
            let in_flight_request_markdown = markdown_segments_from_text(
                transcript_part_index,
                TranscriptPartSubType::Answer,
                IN_FLIGHT_REQUEST_TEXT,
            );
            blocks.add_child(self.render_assistant_answer(
                transcript_part_index,
                &AssistantTranscriptPart {
                    is_error: false,
                    copy_all_tooltip_and_button_mouse_handles: None,
                    pin_tooltip_and_button_mouse_handles: None,
                    formatted_message: FormattedTranscriptMessage {
                        markdown: in_flight_request_markdown,
                        raw: IN_FLIGHT_REQUEST_TEXT.to_owned(),
                    },
                },
                appearance,
                app,
            ));
        }

        if !transcript.is_empty() && matches!(request_status, RequestStatus::NotInFlight) {
            // Only show the prepared responses if the last response wasn't an error
            // and the user still has remaining requests.
            if !transcript.last().is_none_or(|p| p.assistant.is_error) && num_remaining_reqs > 0 {
                blocks.add_child(
                    Container::new(self.render_prepared_responses(appearance))
                        .with_margin_top(15.)
                        .finish(),
                );
            }

            let is_custom_llm_enabled: bool = UserWorkspaces::as_ref(app)
                .current_team()
                .is_some_and(|team| team.is_custom_llm_enabled());

            if !is_custom_llm_enabled {
                blocks.add_child(
                    Container::new(render_request_limit_info(
                        &self.requests_model,
                        app,
                        appearance,
                    ))
                    .with_margin_top(15.)
                    .finish(),
                );
            }

            let current_transcript_summarized = self
                .requests_model
                .as_ref(app)
                .current_transcript_summarized();

            blocks.add_child(
                Container::new(
                    self.render_warning_message(ACCURACY_NOTICE_TEXT.to_string(), appearance),
                )
                .with_margin_top(DETAILS_BOTTOM_MARGIN)
                .with_margin_bottom(if current_transcript_summarized {
                    DETAILS_BOTTOM_MARGIN / 2.
                } else {
                    DETAILS_BOTTOM_MARGIN
                })
                .finish(),
            );

            if current_transcript_summarized {
                blocks.add_child(
                    Container::new(self.render_warning_message(
                        MISSING_CONTEXT_NOTICE_TEXT.to_string(),
                        appearance,
                    ))
                    .with_margin_bottom(DETAILS_BOTTOM_MARGIN)
                    .finish(),
                );
            }
        }

        // Note: we don't render a scrollbar because the gutter makes the segmented transcript
        // look "broken".
        let transcript = SavePosition::new(
            ClippedScrollable::vertical(
                self.clipped_scroll_state.clone(),
                blocks.finish(),
                ScrollbarWidth::None,
                theme.disabled_text_color(theme.background()).into(),
                theme.main_text_color(theme.background()).into(),
                Fill::None,
            )
            .with_padding_end(0.)
            .with_padding_start(0.)
            .finish(),
            TRANSCRIPT_POSITION_ID,
        )
        .finish();

        let mut navigatable_transcript =
            EventHandler::new(transcript).on_left_mouse_down(|ctx, _, _| {
                ctx.dispatch_typed_action(TranscriptAction::MouseDown);
                DispatchEventResult::StopPropagation
            });

        // Only handle keydown events when a code block is selected and the transcript is focused.
        let is_focused = self
            .view_handle
            .upgrade(app)
            .is_some_and(|v| v.is_focused(app));
        if self.selected_code_block.is_some() && is_focused {
            navigatable_transcript = navigatable_transcript.on_keydown(|ctx, _, keystroke| {
                ctx.dispatch_typed_action(TranscriptAction::Keydown(keystroke.to_owned()));
                DispatchEventResult::StopPropagation
            });
        }

        let mut stack = Stack::new();
        stack.add_child(
            Container::new(navigatable_transcript.finish())
                .with_padding_top(HEADER_HEIGHT)
                .finish(),
        );

        stack.finish()
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            // Force a re-render to reflect the fact that this view is now focused.
            ctx.notify();
        }
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            // Force a re-render to reflect the fact that this view is now blurred.
            ctx.notify();
        }
    }
}

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod transcript_tests;
