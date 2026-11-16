use super::{FormattedTranscriptMessage, TranscriptPart, TranscriptPartSubType};
use crate::ai_assistant::test_util::{
    default_assistant_transcript_part, default_code_block_segment, default_formatted_message,
    default_other_segment,
};
use crate::ai_assistant::utils::CodeBlockIndex;

// Mocked data to make it easy to test.
lazy_static::lazy_static! {
    static ref FIRST_USER_CODE_BLOCK_INDEX: CodeBlockIndex = CodeBlockIndex::new(0, TranscriptPartSubType::Question, 0);
    static ref SECOND_USER_CODE_BLOCK_INDEX: CodeBlockIndex = CodeBlockIndex::new(0, TranscriptPartSubType::Question, 1);
    static ref USER_FORMATTED_MESSAGE: FormattedTranscriptMessage = default_formatted_message(vec![
        default_code_block_segment(*FIRST_USER_CODE_BLOCK_INDEX),
        default_other_segment(),
        default_code_block_segment(*SECOND_USER_CODE_BLOCK_INDEX),
        default_other_segment(),
    ]);

    static ref FIRST_ASSISTANT_CODE_BLOCK_INDEX: CodeBlockIndex = CodeBlockIndex::new(0, TranscriptPartSubType::Answer, 0);
    static ref SECOND_ASSISTANT_CODE_BLOCK_INDEX: CodeBlockIndex = CodeBlockIndex::new(0, TranscriptPartSubType::Answer, 1);
    static ref ASSISTANT_FORMATTED_MESSAGE: FormattedTranscriptMessage = default_formatted_message(vec![
        default_other_segment(),
        default_other_segment(),
        default_code_block_segment(*FIRST_ASSISTANT_CODE_BLOCK_INDEX),
        default_code_block_segment(*SECOND_ASSISTANT_CODE_BLOCK_INDEX),
    ]);

    static ref TRANSCRIPT_PART: TranscriptPart =
        TranscriptPart {
            user: (*USER_FORMATTED_MESSAGE).clone(),
            assistant: default_assistant_transcript_part((*ASSISTANT_FORMATTED_MESSAGE).clone())
        };
}

#[test]
fn test_formatted_transcript_message_first_code_block() {
    assert_eq!(
        USER_FORMATTED_MESSAGE.first_code_block_index(),
        Some(*FIRST_USER_CODE_BLOCK_INDEX)
    );
    assert_eq!(
        ASSISTANT_FORMATTED_MESSAGE.first_code_block_index(),
        Some(*FIRST_ASSISTANT_CODE_BLOCK_INDEX)
    );
}

#[test]
fn test_formatted_transcript_message_last_code_block() {
    assert_eq!(
        USER_FORMATTED_MESSAGE.last_code_block_index(),
        Some(*SECOND_USER_CODE_BLOCK_INDEX)
    );
    assert_eq!(
        ASSISTANT_FORMATTED_MESSAGE.last_code_block_index(),
        Some(*SECOND_ASSISTANT_CODE_BLOCK_INDEX)
    );
}

#[test]
fn test_formatted_transcript_message_next_code_block() {
    assert_eq!(
        USER_FORMATTED_MESSAGE.next_code_block_index(0),
        Some(*SECOND_USER_CODE_BLOCK_INDEX)
    );
    assert_eq!(USER_FORMATTED_MESSAGE.next_code_block_index(1), None);
    assert_eq!(
        ASSISTANT_FORMATTED_MESSAGE.next_code_block_index(0),
        Some(*SECOND_ASSISTANT_CODE_BLOCK_INDEX)
    );
    assert_eq!(ASSISTANT_FORMATTED_MESSAGE.next_code_block_index(1), None);
}

#[test]
fn test_formatted_transcript_message_prev_code_block() {
    assert_eq!(USER_FORMATTED_MESSAGE.prev_code_block_index(0), None);
    assert_eq!(
        USER_FORMATTED_MESSAGE.prev_code_block_index(1),
        Some(*FIRST_USER_CODE_BLOCK_INDEX)
    );
    assert_eq!(ASSISTANT_FORMATTED_MESSAGE.prev_code_block_index(0), None);
    assert_eq!(
        ASSISTANT_FORMATTED_MESSAGE.prev_code_block_index(1),
        Some(*FIRST_ASSISTANT_CODE_BLOCK_INDEX)
    );
}

#[test]
fn test_transcript_part_first_code_block() {
    assert_eq!(
        TRANSCRIPT_PART.first_code_block_index(),
        Some(*FIRST_USER_CODE_BLOCK_INDEX)
    );
}

#[test]
fn test_transcript_part_last_code_block() {
    assert_eq!(
        TRANSCRIPT_PART.last_code_block_index(),
        Some(*SECOND_ASSISTANT_CODE_BLOCK_INDEX)
    );
}

#[test]
fn test_transcript_part_next_code_block() {
    assert_eq!(
        TRANSCRIPT_PART.next_code_block_index(*FIRST_USER_CODE_BLOCK_INDEX),
        Some(*SECOND_USER_CODE_BLOCK_INDEX)
    );
    assert_eq!(
        TRANSCRIPT_PART.next_code_block_index(*SECOND_USER_CODE_BLOCK_INDEX),
        Some(*FIRST_ASSISTANT_CODE_BLOCK_INDEX)
    );
    assert_eq!(
        TRANSCRIPT_PART.next_code_block_index(*FIRST_ASSISTANT_CODE_BLOCK_INDEX),
        Some(*SECOND_ASSISTANT_CODE_BLOCK_INDEX)
    );
    assert_eq!(
        TRANSCRIPT_PART.next_code_block_index(*SECOND_ASSISTANT_CODE_BLOCK_INDEX),
        None
    );
}

#[test]
fn test_transcript_part_prev_code_block() {
    assert_eq!(
        TRANSCRIPT_PART.prev_code_block_index(*FIRST_USER_CODE_BLOCK_INDEX),
        None
    );
    assert_eq!(
        TRANSCRIPT_PART.prev_code_block_index(*SECOND_USER_CODE_BLOCK_INDEX),
        Some(*FIRST_USER_CODE_BLOCK_INDEX)
    );
    assert_eq!(
        TRANSCRIPT_PART.prev_code_block_index(*FIRST_ASSISTANT_CODE_BLOCK_INDEX),
        Some(*SECOND_USER_CODE_BLOCK_INDEX)
    );
    assert_eq!(
        TRANSCRIPT_PART.prev_code_block_index(*SECOND_ASSISTANT_CODE_BLOCK_INDEX),
        Some(*FIRST_ASSISTANT_CODE_BLOCK_INDEX)
    );
}
