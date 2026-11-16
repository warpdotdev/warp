use warpui::{Entity, ModelContext};

use crate::editor::InteractionState;

pub struct InteractionStateModel {
    state: InteractionState,
    is_block_selected: bool,
}

impl InteractionStateModel {
    pub fn new(initial_state: InteractionState) -> Self {
        Self {
            state: initial_state,
            is_block_selected: false, // refers to whether any block in the given notebook is selected
        }
    }

    pub fn set_interaction_state(
        &mut self,
        new_state: InteractionState,
        ctx: &mut ModelContext<Self>,
    ) {
        self.state = new_state;
        ctx.emit(InteractionStateModelEvent::InteractionStateChanged { new_state });
    }

    pub fn interaction_state(&self) -> InteractionState {
        self.state
    }

    pub fn is_block_selected(&self) -> bool {
        self.is_block_selected
    }

    pub fn set_is_block_selected(&mut self, is_selected: bool, ctx: &mut ModelContext<Self>) {
        self.is_block_selected = is_selected;
        ctx.notify();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionStateModelEvent {
    InteractionStateChanged { new_state: InteractionState },
}

impl Entity for InteractionStateModel {
    type Event = InteractionStateModelEvent;
}
