/// Common functionality used across different AI Assistant components.
use markdown_parser::{parse_markdown, CodeBlockText, FormattedText, FormattedTextLine};
use pathfinder_color::ColorU;
use warpui::{
    elements::{
        ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, HighlightedHyperlink,
        Icon, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Text,
    },
    platform::Cursor,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element, ModelHandle,
};

use crate::{appearance::Appearance, ui_components::blended_colors};

use super::{panel::AIAssistantAction, requests::Requests, transcript::CodeBlockMouseStateHandles};

const PREPARED_RESPONSE_FONT_SIZE: f32 = 11.;
const REQUEST_LIMIT_INFO_FONT_SIZE: f32 = 11.;

const SQUARE_ALERT_SVG_PATH: &str = "bundled/svg/alert-square.svg";
const TRIANGLE_ALERT_SVG_PATH: &str = "bundled/svg/alert-triangle.svg";

/// A transcript part is a question and answer _pair_. This is to enforce
/// the invariant that every question has an answer.
#[derive(Clone)]
pub struct TranscriptPart {
    pub user: FormattedTranscriptMessage,
    pub assistant: AssistantTranscriptPart,
}

/// The assistant part of a transcript part.
#[derive(Clone)]
pub struct AssistantTranscriptPart {
    pub is_error: bool,
    pub formatted_message: FormattedTranscriptMessage,
    pub copy_all_tooltip_and_button_mouse_handles: Option<(MouseStateHandle, MouseStateHandle)>,
}

/// The information needed to render a single transcript message (whether it be a question or answer).
#[derive(Clone)]
pub struct FormattedTranscriptMessage {
    /// If we can't parse the message as markdown, we can still
    /// use the `raw` field to display it. But we should try to render as markdown.
    pub markdown: Option<Vec<MarkdownSegment>>,
    pub raw: String,
}

impl FormattedTranscriptMessage {
    /// Finds the index of the first code block in the message, if there is one.
    fn first_code_block_index(&self) -> Option<CodeBlockIndex> {
        let segments = self.markdown.as_ref()?;
        segments.iter().find_map(|s| match s {
            MarkdownSegment::CodeBlock { index, .. } => Some(*index),
            _ => None,
        })
    }

    /// Finds the index of the last code block in the message, if there is one.
    fn last_code_block_index(&self) -> Option<CodeBlockIndex> {
        let segments = self.markdown.as_ref()?;
        segments.iter().rev().find_map(|s| match s {
            MarkdownSegment::CodeBlock { index, .. } => Some(*index),
            _ => None,
        })
    }

    /// Finds the index of the next code block after `code_block_index` in the message, if there is one.
    fn next_code_block_index(&self, code_block_index: usize) -> Option<CodeBlockIndex> {
        let segments = self.markdown.as_ref()?;
        segments.iter().find_map(|s| match s {
            MarkdownSegment::CodeBlock { index, .. } => {
                (index.code_block_index == code_block_index + 1).then_some(*index)
            }
            _ => None,
        })
    }

    /// Finds the index of the previous code block before `code_block_index` in the message, if there is one.
    fn prev_code_block_index(&self, code_block_index: usize) -> Option<CodeBlockIndex> {
        if code_block_index == 0 {
            return None;
        }

        let segments = self.markdown.as_ref()?;
        segments.iter().find_map(|s| match s {
            MarkdownSegment::CodeBlock { index, .. } => {
                (index.code_block_index == code_block_index - 1).then_some(*index)
            }
            _ => None,
        })
    }

    /// Returns the raw code block string for the given code block index.
    fn code_for_block(&self, code_block_index: usize) -> Option<&str> {
        let segments = self.markdown.as_ref()?;
        segments.iter().find_map(|s| match s {
            MarkdownSegment::CodeBlock { index, code, .. } => {
                (index.code_block_index == code_block_index).then_some(code.code.as_str())
            }
            _ => None,
        })
    }
}

/// A MarkdownSegment differs from a FormattedText in that we intentionally
/// separate out certain markdown elements.
/// For now, only code blocks are rendered differently.
#[derive(Clone)]
pub enum MarkdownSegment {
    CodeBlock {
        index: CodeBlockIndex,
        code: CodeBlockText,
        mouse_state_handles: CodeBlockMouseStateHandles,
    },
    Other {
        /// The formatted text does _not_ contain any of the other
        /// MarkdownSegment's.
        formatted_text: FormattedText,
        highlighted_hyperlink: HighlightedHyperlink,
    },
}

impl TranscriptPart {
    pub fn raw_user_prompt(&self) -> &str {
        self.user.raw.as_str()
    }

    pub fn raw_assistant_answer(&self) -> &str {
        self.assistant.formatted_message.raw.as_str()
    }

    /// Returns the index of the first code block in this transcript part, if there is one.
    pub fn first_code_block_index(&self) -> Option<CodeBlockIndex> {
        self.user
            .first_code_block_index()
            .or_else(|| self.assistant.formatted_message.first_code_block_index())
    }

    /// Returns the index of the last code block in this transcript part, if there is one.
    pub fn last_code_block_index(&self) -> Option<CodeBlockIndex> {
        self.assistant
            .formatted_message
            .last_code_block_index()
            .or_else(|| self.user.last_code_block_index())
    }

    /// Returns the index of the next code block after the given code block index in this transcript part, if there is one.
    pub fn next_code_block_index(
        &self,
        code_block_index: CodeBlockIndex,
    ) -> Option<CodeBlockIndex> {
        match code_block_index.transcript_part_type {
            // Since a transcript part is question -> answer, check if there's a next code block in the question part,
            // otherwise get the first code block in the answer part.
            TranscriptPartSubType::Question => self
                .user
                .next_code_block_index(code_block_index.code_block_index)
                .or_else(|| self.assistant.formatted_message.first_code_block_index()),
            TranscriptPartSubType::Answer => self
                .assistant
                .formatted_message
                .next_code_block_index(code_block_index.code_block_index),
        }
    }

    /// Returns the index of the previous code block before the given code block index in this transcript part, if there is one.
    pub fn prev_code_block_index(
        &self,
        code_block_index: CodeBlockIndex,
    ) -> Option<CodeBlockIndex> {
        match code_block_index.transcript_part_type {
            TranscriptPartSubType::Question => self
                .user
                .prev_code_block_index(code_block_index.code_block_index),
            // Since a transcript part is question -> answer, check if there's a previous code block in the answer part,
            // otherwise get the last code block from the question part.
            TranscriptPartSubType::Answer => self
                .assistant
                .formatted_message
                .prev_code_block_index(code_block_index.code_block_index)
                .or_else(|| self.user.last_code_block_index()),
        }
    }

    pub fn code_for_block(&self, code_block_index: CodeBlockIndex) -> Option<&str> {
        match code_block_index.transcript_part_type {
            TranscriptPartSubType::Question => {
                self.user.code_for_block(code_block_index.code_block_index)
            }
            TranscriptPartSubType::Answer => self
                .assistant
                .formatted_message
                .code_for_block(code_block_index.code_block_index),
        }
    }
}

/// Since a transcript part consists of two sub parts (question and answer),
/// this enum is used to identify which of the two we're referring to.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum TranscriptPartSubType {
    Question,
    Answer,
}

impl TranscriptPartSubType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Question => "question",
            Self::Answer => "answer",
        }
    }
}

/// A CodeBlockIndex is used to uniquely identify a code block in a transcript.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CodeBlockIndex {
    /// The index into the `trancripts` list.
    transcript_part_index: usize,

    /// Since each transcript part consists of two sub-parts (question & answer),
    /// we need to distinguish which of these sub-parts the code block is in.
    transcript_part_type: TranscriptPartSubType,

    /// A subpart can have > 1 code blocks, so this specifies the exact one.
    code_block_index: usize,
}

impl CodeBlockIndex {
    pub fn new(
        transcript_part_index: usize,
        transcript_part_type: TranscriptPartSubType,
        code_block_index: usize,
    ) -> Self {
        Self {
            transcript_part_index,
            transcript_part_type,
            code_block_index,
        }
    }

    pub fn as_id_str(&self) -> String {
        format!(
            "{}_{}_{}",
            self.transcript_part_index,
            self.transcript_part_type.as_str(),
            self.code_block_index
        )
    }

    pub fn transcript_index(&self) -> usize {
        self.transcript_part_index
    }
}

pub fn render_prepared_response_button(
    appearance: &Appearance,
    mouse_state_handle: MouseStateHandle,
    width: Option<f32>,
    right_left_padding: Option<f32>,
    prompt: &'static str,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let default_button_styles = UiComponentStyles {
        width,
        font_size: Some(PREPARED_RESPONSE_FONT_SIZE),
        font_family_id: Some(appearance.ui_font_family()),
        font_color: Some(
            appearance
                .theme()
                .main_text_color(appearance.theme().background())
                .into(),
        ),
        border_radius: Some(CornerRadius::with_all(Radius::Percentage(50.))),
        border_color: Some(theme.accent().into()),
        border_width: Some(1.),
        padding: Some(Coords {
            top: 5.,
            bottom: 5.,
            left: right_left_padding.unwrap_or(0.),
            right: right_left_padding.unwrap_or(0.),
        }),
        ..Default::default()
    };
    let hovered_and_clicked_styles = UiComponentStyles {
        background: Some(theme.accent().into()),
        font_color: Some(theme.background().into()),
        ..default_button_styles
    };
    appearance
        .ui_builder()
        .button_with_custom_styles(
            ButtonVariant::Text,
            mouse_state_handle,
            default_button_styles,
            Some(hovered_and_clicked_styles),
            Some(hovered_and_clicked_styles),
            Some(hovered_and_clicked_styles),
        )
        .with_centered_text_label(prompt.to_string())
        .build()
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(AIAssistantAction::PreparedPrompt(prompt))
        })
        .finish()
}

pub fn render_request_limit_info(
    request_model: &ModelHandle<Requests>,
    app: &AppContext,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let text_color: ColorU =
        blended_colors::text_sub(appearance.theme(), appearance.theme().background());

    let num_requests_used = request_model.as_ref(app).num_requests_used();
    let num_requests_remaining = request_model.as_ref(app).num_remaining_reqs();
    let request_limit = request_model.as_ref(app).request_limit();
    let next_refresh_time = request_model.as_ref(app).serialized_time_until_refresh();

    // Always show the remaining requests count.
    let mut row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Text::new_inline(
                format!("Credits used: {num_requests_used} / {request_limit}.",),
                appearance.ui_font_family(),
                REQUEST_LIMIT_INFO_FONT_SIZE,
            )
            .with_color(text_color)
            .finish(),
        );

    // Add the warning icon if necessary.
    let icon = if num_requests_remaining == 0 {
        Some(Icon::new(
            TRIANGLE_ALERT_SVG_PATH,
            appearance.theme().ui_error_color(),
        ))
    } else if num_requests_remaining <= 10 {
        Some(Icon::new(
            SQUARE_ALERT_SVG_PATH,
            appearance.theme().ui_warning_color(),
        ))
    } else {
        None
    };

    if let Some(icon) = icon {
        row.add_child(
            Container::new(
                ConstrainedBox::new(icon.finish())
                    .with_height(16.)
                    .with_width(16.)
                    .finish(),
            )
            .with_margin_left(5.)
            .finish(),
        );
    }

    // Show the next refresh time if it's valid.
    if let Some(next_refresh_time) = next_refresh_time {
        row.add_child(
            Container::new(
                Text::new_inline(
                    format!("{next_refresh_time} until refresh."),
                    appearance.ui_font_family(),
                    REQUEST_LIMIT_INFO_FONT_SIZE,
                )
                .with_color(text_color)
                .finish(),
            )
            .with_margin_left(5.)
            .finish(),
        );
    }

    row.finish()
}

pub fn code_block_position_id(code_block_index: CodeBlockIndex) -> String {
    format!("code_block_id_{}", code_block_index.as_id_str(),)
}

pub fn save_as_workflow_position_id(code_block_index: CodeBlockIndex) -> String {
    format!(
        "{}_save_as_workflow",
        code_block_position_id(code_block_index)
    )
}

pub fn markdown_segments_from_text(
    transcript_part_index: usize,
    transcript_part_type: TranscriptPartSubType,
    text: &str,
) -> Option<Vec<MarkdownSegment>> {
    let parsed = parse_markdown(text).ok();
    parsed.map(|p| {
        translate_formatted_text_into_markdown_segments(
            transcript_part_index,
            transcript_part_type,
            p,
        )
    })
}

fn translate_formatted_text_into_markdown_segments(
    transcript_part_index: usize,
    transcript_part_type: TranscriptPartSubType,
    formatted_text: FormattedText,
) -> Vec<MarkdownSegment> {
    // At a high-level, we want to go through the FormattedText and extract
    // all the code-blocks separately from contiguous non-code blocks. We want
    // to do this so that we can render the code-blocks specially. The final
    // result is a set of markdown_segments.
    let mut markdown_segments = vec![];

    // The running non-code block is a contiguous sequence of FormattedTextLine's
    // that _do not_ contain any code blocks.
    let mut running_non_code_block = vec![];

    let mut curr_code_block_index = 0;

    for part in formatted_text.lines {
        match part {
            FormattedTextLine::CodeBlock(mut code) => {
                // If we found a code block, flush out the running non-code-block
                // contiguous sequence into a single markdown segment.
                if !running_non_code_block.is_empty() {
                    markdown_segments.push(MarkdownSegment::Other {
                        formatted_text: FormattedText::new_trimmed(running_non_code_block),
                        highlighted_hyperlink: Default::default(),
                    });
                }

                code.code = code.code.trim().to_string();
                markdown_segments.push(MarkdownSegment::CodeBlock {
                    index: CodeBlockIndex::new(
                        transcript_part_index,
                        transcript_part_type,
                        curr_code_block_index,
                    ),
                    code,
                    mouse_state_handles: Default::default(),
                });
                curr_code_block_index += 1;
                running_non_code_block = vec![];
            }
            _ => {
                // If this is anything other than a code block, tack it onto
                // our running sequence.
                running_non_code_block.push(part);
            }
        }
    }

    // If we had a non-code block sequence that we haven't flushed yet by the end,
    // flush it now.
    if !running_non_code_block.is_empty() {
        markdown_segments.push(MarkdownSegment::Other {
            formatted_text: FormattedText::new_trimmed(running_non_code_block),
            highlighted_hyperlink: Default::default(),
        });
    }

    markdown_segments
}

#[cfg(test)]
#[path = "utils_tests.rs"]
mod utils_tests;
