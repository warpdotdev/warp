//! Inline selector for choosing among multiple `devcontainer.json` configs
//! discovered in the current workspace for the `/devcontainer` command.
mod data_source;
mod search_item;
mod view;

use std::path::PathBuf;

pub use view::{InlineDevContainerConfigSelectorEvent, InlineDevContainerConfigSelectorView};

use crate::terminal::input::inline_menu::{InlineMenuAction, InlineMenuType};

/// Action emitted when a Dev Container config is chosen from the inline menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectDevContainerConfig {
    pub config_path: PathBuf,
}

impl InlineMenuAction for SelectDevContainerConfig {
    const MENU_TYPE: InlineMenuType = InlineMenuType::DevContainerConfigSelector;
}
