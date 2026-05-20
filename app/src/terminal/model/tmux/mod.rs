pub mod commands;
pub mod parser;
use crate::terminal::event::ExecutedExecutorCommandEvent;
use crate::util::parse_ascii_u32;
use lazy_static::lazy_static;
use regex::bytes::{Regex, RegexBuilder};

pub enum ControlModeEvent {
    /// This event is sent when the control mode has started
    /// we don't have enough info from the tmux output to know
    /// the primary pane or window yet.
    Starting,
    /// Control Mode will inform us of the primary pane and window,
    /// at which point we can safely direct input to the appropriate
    /// panel.
    ControlModeReady {
        primary_window: u32,
        primary_pane: u32,
    },
    /// This event is sent when Control Mode informs us of pane output
    /// that is coming from a pane which is not the primary pane.
    BackgroundPaneOutput { pane: u32, byte: u8 },
    /// This event is sent when Control Mode has been exited.
    Exited,
}

pub fn format_input(pane: u32, input: &[u8]) -> String {
    let mut formatted = String::new();

    for chunk in input.chunks(1000) {
        formatted.push_str(&format!("send-keys -Ht %{pane}"));
        for byte in chunk {
            formatted.push_str(&format!(" {byte:X}"));
        }
        formatted.push('\n');
    }
    formatted
}

pub fn parse_generator_output(input: &[u8]) -> Option<ExecutedExecutorCommandEvent> {
    lazy_static! {
        static ref GENERATOR_OUTPUT_REGEX: Regex =
            RegexBuilder::new(r"\^\^\^(.+?)\|\|\|(.*?)\|\|\|(\d+)\$\$\$")
                .dot_matches_new_line(true)
                .unicode(false)
                .build()
                .unwrap();
        /// tmux adds a carriage return to newlines that it prints, so remove that here.
        static ref NEWLINE_REGEX: Regex = Regex::new(r"\r\n").expect("Invalid regex");
    }

    GENERATOR_OUTPUT_REGEX.captures(input).and_then(|caps| {
        let command_id = std::str::from_utf8(&caps[1]).ok()?.to_string();
        let output = NEWLINE_REGEX.replace_all(&caps[2], b"\n").to_vec();
        let exit_code = parse_ascii_u32(&caps[3])? as usize;

        Some(ExecutedExecutorCommandEvent {
            command_id,
            output,
            exit_code,
        })
    })
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
