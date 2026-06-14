use warpui::platform::WindowStyle;
use warpui::App;

use super::Transcript;
use crate::ai_assistant::requests::Requests;
use crate::ai_assistant::test_util::{
    default_assistant_transcript_part, default_code_block_segment, default_formatted_message,
    default_other_segment,
};
use crate::ai_assistant::utils::{CodeBlockIndex, TranscriptPart, TranscriptPartSubType};
use crate::appearance;
use crate::auth::AuthStateProvider;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::user_workspaces::UserWorkspaces;

// Mocked data to make it easy to test.
lazy_static::lazy_static! {
    static ref TRANSCRIPT: Vec<TranscriptPart> = vec![
        TranscriptPart {
            user: default_formatted_message(vec![
                default_other_segment(),
                default_code_block_segment(CodeBlockIndex::new(0, TranscriptPartSubType::Question, 0)),
                default_other_segment(),
                default_code_block_segment(CodeBlockIndex::new(0, TranscriptPartSubType::Question, 1)),

            ]),
            assistant: default_assistant_transcript_part(default_formatted_message(vec![
                default_code_block_segment(CodeBlockIndex::new(0, TranscriptPartSubType::Answer, 0)),
            ])),
        },
        TranscriptPart {
            user: default_formatted_message(vec![
                default_code_block_segment(CodeBlockIndex::new(1, TranscriptPartSubType::Question, 0)),
                default_code_block_segment(CodeBlockIndex::new(1, TranscriptPartSubType::Question, 1)),

            ]),
            assistant: default_assistant_transcript_part(default_formatted_message(vec![
                default_other_segment(),
                default_other_segment(),
            ])),
        },
    ];
}

fn initialize_app(app: &mut App) {
    initialize_settings_for_tests(app);
    appearance::register(app);
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(UserWorkspaces::default_mock);
}

#[test]
fn test_next_code_block() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let requests_model = app.add_model(|_| Requests::new_with_transcript(TRANSCRIPT.clone()));
        let (_, transcript_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            Transcript::new(&requests_model, ctx)
        });
        transcript_view.update(&mut app, |view, ctx| {
            // Starting point
            view.selected_code_block =
                Some(CodeBlockIndex::new(0, TranscriptPartSubType::Question, 0));

            let next_code_block = view.next_code_block_index(ctx);
            assert_eq!(
                next_code_block,
                Some(CodeBlockIndex::new(0, TranscriptPartSubType::Question, 1))
            );
            view.selected_code_block = next_code_block;

            let next_code_block = view.next_code_block_index(ctx);
            assert_eq!(
                next_code_block,
                Some(CodeBlockIndex::new(0, TranscriptPartSubType::Answer, 0))
            );
            view.selected_code_block = next_code_block;

            let next_code_block = view.next_code_block_index(ctx);
            assert_eq!(
                next_code_block,
                Some(CodeBlockIndex::new(1, TranscriptPartSubType::Question, 0))
            );
            view.selected_code_block = next_code_block;

            let next_code_block = view.next_code_block_index(ctx);
            assert_eq!(
                next_code_block,
                Some(CodeBlockIndex::new(1, TranscriptPartSubType::Question, 1))
            );
            view.selected_code_block = next_code_block;

            let next_code_block = view.next_code_block_index(ctx);
            assert_eq!(next_code_block, None)
        });
    });
}

#[test]
fn test_prev_code_block() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let requests_model = app.add_model(|_| Requests::new_with_transcript(TRANSCRIPT.clone()));
        let (_, transcript_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            Transcript::new(&requests_model, ctx)
        });
        transcript_view.update(&mut app, |view, ctx| {
            // Starting point
            view.selected_code_block =
                Some(CodeBlockIndex::new(1, TranscriptPartSubType::Question, 1));

            let prev_code_block = view.previous_code_block_index(ctx);
            assert_eq!(
                prev_code_block,
                Some(CodeBlockIndex::new(1, TranscriptPartSubType::Question, 0))
            );
            view.selected_code_block = prev_code_block;

            let prev_code_block = view.previous_code_block_index(ctx);
            assert_eq!(
                prev_code_block,
                Some(CodeBlockIndex::new(0, TranscriptPartSubType::Answer, 0))
            );
            view.selected_code_block = prev_code_block;

            let prev_code_block = view.previous_code_block_index(ctx);
            assert_eq!(
                prev_code_block,
                Some(CodeBlockIndex::new(0, TranscriptPartSubType::Question, 1))
            );
            view.selected_code_block = prev_code_block;

            let prev_code_block = view.previous_code_block_index(ctx);
            assert_eq!(
                prev_code_block,
                Some(CodeBlockIndex::new(0, TranscriptPartSubType::Question, 0))
            );
            view.selected_code_block = prev_code_block;

            let prev_code_block = view.previous_code_block_index(ctx);
            assert_eq!(prev_code_block, None);
        });
    });
}

#[test]
fn test_last_code_block() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let requests_model = app.add_model(|_| Requests::new_with_transcript(TRANSCRIPT.clone()));
        let (_, transcript_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            Transcript::new(&requests_model, ctx)
        });
        transcript_view.update(&mut app, |view, ctx| {
            view.select_last_code_block(ctx);
            assert_eq!(
                view.selected_code_block,
                Some(CodeBlockIndex::new(1, TranscriptPartSubType::Question, 1))
            );
        });
    });
}
