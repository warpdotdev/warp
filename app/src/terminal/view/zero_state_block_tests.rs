use std::sync::Arc;

use warpui::App;

use super::*;
use crate::terminal::event::{BlockCompletedEvent, UserBlockCompleted};
use crate::terminal::model::block::{BlockId, SerializedBlock};
use crate::terminal::model::terminal_model::BlockIndex;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn completed_block(was_warp_authored: bool) -> BlockType {
    BlockType::User(UserBlockCompleted::new_for_test(
        BlockIndex::zero(),
        Arc::new(SerializedBlock::new_for_test(b"claude".to_vec(), vec![])),
        "claude".to_owned(),
        "claude".to_owned(),
        String::new(),
        String::new(),
        false,
        was_warp_authored,
        None,
        0,
        0,
    ))
}

/// AE14/R24: the pane's zero-state affordance is spent by the user's first block. A resume is
/// Warp filling the pane in, not the user starting to work in it.
#[test]
fn zero_state_affordance_survives_a_resume_block() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal = add_window_with_terminal(&mut app, None);
        let (zero_state, dispatcher) = terminal.update(&mut app, |view, ctx| {
            let controller = view.agent_view_controller.clone();
            let dispatcher = view.model_event_dispatcher().clone();
            let zero_state =
                ctx.add_view(|ctx| TerminalViewZeroStateBlock::new(&controller, &dispatcher, ctx));
            (zero_state, dispatcher)
        });

        let emit = |app: &mut App, block_type: BlockType| {
            dispatcher.update(app, |_, ctx| {
                ctx.emit(ModelEvent::BlockCompleted(BlockCompletedEvent {
                    block_type,
                    num_secrets_obfuscated: 0,
                    block_index: BlockIndex::zero(),
                    block_id: BlockId::new(),
                    session_id: None,
                    restored_block_was_local: None,
                }));
            });
        };

        emit(&mut app, completed_block(/*was_warp_authored=*/ true));
        zero_state.read(&app, |zero_state, _| {
            assert!(
                !zero_state.should_hide,
                "a resume must not spend the pane's zero-state affordance"
            );
        });

        emit(&mut app, completed_block(/*was_warp_authored=*/ false));
        zero_state.read(&app, |zero_state, _| {
            assert!(
                zero_state.should_hide,
                "the user's own first block still hides it"
            );
        });
    });
}
