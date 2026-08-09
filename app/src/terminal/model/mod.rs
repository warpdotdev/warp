pub use grid::GridStorage;
pub use terminal_model::TerminalModel;

#[cfg(test)]
#[macro_export]
macro_rules! assert_lines_approx_eq {
    ($actual:expr_2021, $expected:expr_2021) => {{
        float_cmp::assert_approx_eq!(
            warpui::units::Lines,
            $actual,
            warpui::units::IntoLines::into_lines($expected)
        )
    }};
}

pub mod alt_screen;
pub mod block;
pub mod blocks;
pub mod bootstrap;
pub mod header_grid;
pub mod rich_content;
pub mod secrets;

pub mod early_output;
pub mod index;
pub(in crate::terminal) mod lifecycle;
pub mod session;
pub mod terminal_model;
#[cfg(any(test, feature = "test-util"))]
pub mod test_utils;

pub use lifecycle::{LifecycleRecoveryRecord, StartCommandOutcome};
pub use warp_terminal::model::grid::cell;
pub use warp_terminal::model::secrets::{
    ObfuscateSecrets, RespectObfuscatedSecrets, Secret, SecretHandle,
    set_user_and_enterprise_secret_regexes,
};
pub use warp_terminal::model::{
    BlockId, ansi, blockgrid, char_or_str, completions, escape_sequences, find, grid, image_map,
    iterm_image, kitty, mouse, selection,
};
