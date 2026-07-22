use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use warp_graphql::mutations::share_block::DisplaySetting as GqlDisplaySetting;

use crate::terminal::model::ObfuscateSecrets;
use crate::terminal::model::block::{Block as ClientBlock, BlockTime};
use crate::terminal::model::grid::Dimensions as _;

// These are pixel heights of various parts of an embedded block.
pub const TITLE_HEIGHT: u32 = 34;
pub const HEADER_PADDING: u32 = 30;
pub const OUTPUT_PADDING: u32 = 32;
pub const LINE_HEIGHT: u32 = 19;
pub const PROMPT_LINE_HEIGHT: u32 = 16;
pub const OUTPUT_CELL_WIDTH: u32 = 10;
pub const EMBED_FOOTER_HEIGHT: u32 = 38;
pub const EXTRA_PADDING: u32 = 35;

/// This enum is a replica of the `share_block::DisplaySetting` struct auto-generated from the GraphQL Schema.
/// We cannot derive traits on the auto-generated structs because any rust attributes
/// will be rewritten.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum DisplaySetting {
    Command,
    Output,
    CommandAndOutput,
    Other(String),
}

impl From<DisplaySetting> for GqlDisplaySetting {
    fn from(value: DisplaySetting) -> Self {
        match value {
            DisplaySetting::Command => GqlDisplaySetting::Command,
            DisplaySetting::Output => GqlDisplaySetting::Output,
            DisplaySetting::CommandAndOutput => GqlDisplaySetting::CommandAndOutput,
            DisplaySetting::Other(s) => GqlDisplaySetting::Other(s),
        }
    }
}

/// A representation of a Block for the server.
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Block {
    pub id: Option<String>,

    /// The input lines for a block.
    pub command: Option<String>,

    /// The output lines for a block.
    pub output: Option<String>,

    /// The input lines with their corresponding escape sequences so it can be rendered outside of
    /// the terminal.
    pub stylized_command: Option<String>,

    /// The output lines with their corresponding escape sequences so it can be rendered outside of
    /// the terminal.
    pub stylized_output: Option<String>,

    /// The prompt lines with their corresponding escape sequences so it can be rendered outside of
    /// the terminal.
    pub stylized_prompt: Option<String>,

    /// The prompt and command (combined) lines with their corresponding escape sequences so it can
    /// be rendered outside of the terminal. Only non-null if using PS1 with the combined grid.
    pub stylized_prompt_and_command: Option<String>,

    /// The current working directory of the block.
    pub pwd: Option<String>,

    /// The terminal's timestamp of block completion.
    pub time_started_term: DateTime<FixedOffset>,

    /// The terminal's timestamp of block completion.
    pub time_completed_term: DateTime<FixedOffset>,
}

/// A helper struct to organize the block's contents.
struct BlockContents {
    command: Option<String>,
    stylized_command: Option<String>,
    output: Option<String>,
    stylized_output: Option<String>,
    stylized_prompt: Option<String>,
    stylized_prompt_and_command: Option<String>,
}

impl Block {
    pub fn new(
        block: &ClientBlock,
        show_prompt: bool,
        display_setting: &DisplaySetting,
        obfuscate_secrets: ObfuscateSecrets,
    ) -> Self {
        let block_time = BlockTime::new(DateTime::from(Utc::now()), DateTime::from(Utc::now()));

        let block_contents =
            if obfuscate_secrets.is_visually_obfuscated() {
                let (command, stylized_command) = match display_setting {
                    DisplaySetting::Command | DisplaySetting::CommandAndOutput => (
                        Some(block.command_with_secrets_obfuscated(
                            false, /*include_escape_sequences*/
                        )),
                        Some(block.command_with_secrets_obfuscated(
                            true, /*include_escape_sequences*/
                        )),
                    ),
                    _ => (None, None),
                };
                let (output, stylized_output) = match display_setting {
                    DisplaySetting::Output | DisplaySetting::CommandAndOutput => (
                        Some(
                            block
                                .output_grid()
                                .contents_to_string_force_secrets_obfuscated(
                                    false, /*include_escape_sequences*/
                                    None,  /*max_rows*/
                                ),
                        ),
                        Some(
                            block
                                .output_grid()
                                .contents_to_string_force_secrets_obfuscated(
                                    true, /*include_escape_sequences*/
                                    None, /*max_rows*/
                                ),
                        ),
                    ),
                    _ => (None, None),
                };

                let stylized_prompt = show_prompt.then_some(if block.honor_ps1() {
                    block.prompt_with_secrets_obfuscated(true)
                } else {
                    Self::native_prompt_for_server(block)
                });
                let stylized_prompt_and_command = (show_prompt && block.honor_ps1())
                    .then(|| block.prompt_and_command_with_secrets_obfuscated(true));
                BlockContents {
                    command,
                    stylized_command,
                    output,
                    stylized_output,
                    stylized_prompt,
                    stylized_prompt_and_command,
                }
            } else {
                let (command, stylized_command) = match display_setting {
                    DisplaySetting::Command | DisplaySetting::CommandAndOutput => (
                        Some(block.command_with_secrets_unobfuscated(
                            false, /*include_escape_sequences*/
                        )),
                        Some(block.command_with_secrets_unobfuscated(
                            true, /*include_escape_sequences*/
                        )),
                    ),
                    _ => (None, None),
                };
                let (output, stylized_output) = match display_setting {
                    DisplaySetting::Output | DisplaySetting::CommandAndOutput => (
                        Some(
                            block
                                .output_grid()
                                .contents_to_string_with_secrets_unobfuscated(
                                    false, /*include_escape_sequences*/
                                    None,  /*max_rows*/
                                ),
                        ),
                        Some(
                            block
                                .output_grid()
                                .contents_to_string_with_secrets_unobfuscated(
                                    true, /*include_escape_sequences*/
                                    None, /*max_rows*/
                                ),
                        ),
                    ),
                    _ => (None, None),
                };

                let stylized_prompt = show_prompt.then_some(if block.honor_ps1() {
                    block.prompt_with_secrets_unobfuscated(true)
                } else {
                    Self::native_prompt_for_server(block)
                });
                let stylized_prompt_and_command = (show_prompt && block.honor_ps1())
                    .then(|| block.prompt_and_command_with_secrets_unobfuscated(true));
                BlockContents {
                    command,
                    stylized_command,
                    output,
                    stylized_output,
                    stylized_prompt,
                    stylized_prompt_and_command,
                }
            };

        Block {
            id: None,
            command: block_contents.command,
            output: block_contents.output,
            stylized_command: block_contents.stylized_command,
            stylized_output: block_contents.stylized_output,
            stylized_prompt: block_contents.stylized_prompt,
            stylized_prompt_and_command: block_contents.stylized_prompt_and_command,
            pwd: block.pwd().map(String::from),
            time_started_term: block_time.time_started_term,
            time_completed_term: block_time.time_completed_term,
        }
    }

    pub fn native_prompt_for_server(block: &ClientBlock) -> String {
        if let Some(prompt_snapshot) = block.prompt_snapshot() {
            prompt_snapshot.to_string()
        } else {
            let mut stylized_prompt = String::new();
            if let Some(conda_env) = block.conda_env() {
                stylized_prompt.push_str(format!("({conda_env}) ").as_str());
            }
            if let Some(virtual_env) = block.virtual_env_short_name() {
                stylized_prompt.push_str(format!("({virtual_env}) ").as_str());
            }
            if let Some(pwd) = block.server_pwd().to_owned() {
                stylized_prompt.push_str(format!("{pwd} ").as_str());
            }
            if let Some(git_branch) = block.git_branch() {
                stylized_prompt.push_str(format!("git:({git_branch})").as_str());
            }
            stylized_prompt
        }
    }

    pub fn embed_pixel_height(
        block: &ClientBlock,
        show_prompt: bool,
        display_setting: &DisplaySetting,
    ) -> u32 {
        let mut height = TITLE_HEIGHT;
        height += HEADER_PADDING;

        if show_prompt {
            if block.honor_ps1() && !block.render_prompt_on_same_line() {
                height += block.prompt_number_of_rows() as u32 * PROMPT_LINE_HEIGHT;
            } else if !block.honor_ps1() {
                height += PROMPT_LINE_HEIGHT;
            }
        }

        match display_setting {
            DisplaySetting::Command => {
                height += block.prompt_and_command_number_of_rows() as u32 * LINE_HEIGHT;
            }
            DisplaySetting::Output => {
                height += LINE_HEIGHT; // The command is blank, but space is still rendered inside the sticky header.
                height += block.output_grid().len() as u32 * LINE_HEIGHT;
                height += OUTPUT_PADDING;
            }
            _ => {
                height += block.prompt_and_command_number_of_rows() as u32 * LINE_HEIGHT;
                height += block.output_grid().len() as u32 * LINE_HEIGHT;
                height += OUTPUT_PADDING;
            }
        }

        height += EMBED_FOOTER_HEIGHT;
        height += EXTRA_PADDING;
        height
    }

    pub fn embed_pixel_width(block: &ClientBlock) -> u32 {
        (block.output_grid().grid_handler().columns() as u32 * OUTPUT_CELL_WIDTH) + OUTPUT_PADDING
    }
}
