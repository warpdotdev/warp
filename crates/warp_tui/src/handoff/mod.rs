//! TUI local-to-cloud handoff state, presentation, and session integration.

mod block;
mod model;

pub(crate) use block::{TuiHandoffBlock, TuiHandoffBlockEvent, init};
pub(crate) use model::{TuiHandoffModel, TuiHandoffModelEvent};
