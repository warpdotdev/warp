//! Inline history menu for up-arrow history when `FeatureFlag::AgentView` is enabled.
//!
//! Shows both live conversations for the terminal view and command history in the terminal
//! view, and prompts and command history in the agent view.
mod data_source;
mod search_item;
mod view;

pub use data_source::{AcceptHistoryItem, InlineHistoryMenuDataSource};
pub use view::{HistoryTab, InlineHistoryMenuEvent, InlineHistoryMenuView};
