//! Rubric literals for agentic judges, kept separate from the eval tests that
//! invoke them so a rubric can be reused across eval variants.

pub mod warpctrl_first_slice;

use super::agent_judge;
pub use warpctrl_first_slice::WARPCTRL_FIRST_SLICE;
