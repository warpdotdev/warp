use markdown_parser::{CodeBlockText, FormattedText};

use crate::ai_assistant::utils::{
    AssistantTranscriptPart, CodeBlockIndex, FormattedTranscriptMessage, MarkdownSegment,
};

pub fn default_code_block_segment(code_block_index: CodeBlockIndex) -> MarkdownSegment {
    MarkdownSegment::CodeBlock {
        index: code_block_index,
        code: CodeBlockText {
            lang: String::from(""),
            code: String::from(""),
        },
        mouse_state_handles: Default::default(),
    }
}

pub fn default_other_segment() -> MarkdownSegment {
    MarkdownSegment::Other {
        formatted_text: FormattedText::new(vec![]),
        highlighted_hyperlink: Default::default(),
    }
}

pub fn default_formatted_message(segments: Vec<MarkdownSegment>) -> FormattedTranscriptMessage {
    FormattedTranscriptMessage {
        markdown: Some(segments),
        raw: String::from(""),
    }
}

pub fn default_assistant_transcript_part(
    formatted_transcript_message: FormattedTranscriptMessage,
) -> AssistantTranscriptPart {
    AssistantTranscriptPart {
        is_error: false,
        formatted_message: formatted_transcript_message,
        copy_all_tooltip_and_button_mouse_handles: None,
        pin_tooltip_and_button_mouse_handles: None,
    }
}
