#[cfg(test)]
use std::sync::Arc;
#[cfg(not(test))]
use std::sync::OnceLock;
use std::sync::mpsc;
#[cfg(not(target_family = "wasm"))]
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    path::PathBuf,
};

use chrono::{Local, SecondsFormat};
#[cfg(test)]
use parking_lot::Mutex;
use warp_completer::completer::{CommandExitStatus, CommandOutput};
#[cfg(not(target_family = "wasm"))]
use warp_errors::report_error;

use super::ContextChipKind;
use crate::terminal::shell::ShellType;

const EMPTY_VALUE: &str = "<empty>";
const MISSING_VALUE: &str = "<none>";

pub(crate) struct ChipCommandLogEntry<'a> {
    pub chip_kind: &'a ContextChipKind,
    pub chip_title: &'a str,
    pub phase: PromptChipExecutionPhase,
    pub shell_type: ShellType,
    pub working_directory: Option<&'a str>,
    pub command: &'a str,
    pub output: Option<&'a CommandOutput>,
    pub timed_out: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum PromptChipExecutionPhase {
    Value,
    OnClick,
}

impl PromptChipExecutionPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Value => "value",
            Self::OnClick => "on_click",
        }
    }
}

#[derive(Clone)]
pub(crate) enum PromptChipLogger {
    Disabled,
    Runtime {
        sender: mpsc::Sender<String>,
    },
    #[cfg(test)]
    TestBuffer {
        entries: Arc<Mutex<Vec<String>>>,
    },
}

impl Default for PromptChipLogger {
    fn default() -> Self {
        Self::shared()
    }
}

impl PromptChipLogger {
    pub(crate) fn shared() -> Self {
        cfg_if::cfg_if! {
            if #[cfg(test)] {
                Self::Disabled
            } else {
                static SHARED_LOGGER: OnceLock<PromptChipLogger> = OnceLock::new();
                SHARED_LOGGER.get_or_init(Self::init_runtime).clone()
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_buffer(entries: Arc<Mutex<Vec<String>>>) -> Self {
        Self::TestBuffer { entries }
    }

    pub(crate) fn log_shell_command(&self, entry: &ChipCommandLogEntry<'_>) {
        let formatted = format_log_entry(entry);

        match self {
            Self::Disabled => {}
            Self::Runtime { sender } => {
                let _ = sender.send(formatted);
            }
            #[cfg(test)]
            Self::TestBuffer { entries } => {
                entries.lock().push(formatted);
            }
        }
    }

    #[cfg(not(target_family = "wasm"))]
    fn init_runtime() -> Self {
        if !warp_core::channel::ChannelState::enable_debug_features() {
            return Self::Disabled;
        }

        let log_path = match log_file_path() {
            Ok(log_path) => log_path,
            Err(err) => {
                log::warn!("Failed to determine prompt chip log file path: {err:#}");
                return Self::Disabled;
            }
        };

        match spawn_log_writer(log_path.clone()) {
            Ok(sender) => Self::Runtime { sender },
            Err(err) => {
                log::warn!(
                    "Failed to initialize prompt chip log writer at {}: {err:#}",
                    log_path.display()
                );
                Self::Disabled
            }
        }
    }

    #[cfg(target_family = "wasm")]
    fn init_runtime() -> Self {
        Self::Disabled
    }
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn log_file_path() -> anyhow::Result<PathBuf> {
    let log_directory = warp_logging::log_directory()?;
    let channel_logfile_name = warp_core::channel::ChannelState::logfile_name();
    Ok(log_directory.join(prompt_chip_log_filename(&channel_logfile_name)))
}

#[cfg(not(target_family = "wasm"))]
fn spawn_log_writer(log_path: PathBuf) -> io::Result<mpsc::Sender<String>> {
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)?;
    let (tx, rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("prompt-chip-log-writer".to_string())
        .spawn(move || write_log_entries(file, rx, log_path))
        .map_err(io::Error::other)?;

    Ok(tx)
}

#[cfg(not(target_family = "wasm"))]
fn write_log_entries(mut file: File, rx: mpsc::Receiver<String>, log_path: PathBuf) {
    while let Ok(entry) = rx.recv() {
        if let Err(err) = file.write_all(entry.as_bytes()).and_then(|_| file.flush()) {
            report_error!(
                anyhow::Error::new(err).context("Failed to write prompt chip log entry"),
                extra: { "log_path" => %log_path.display() }
            );
            return;
        }
    }
}

fn prompt_chip_log_filename(channel_logfile_name: &str) -> String {
    let channel_logfile_stem = channel_logfile_name
        .strip_suffix(".log")
        .unwrap_or(channel_logfile_name);
    format!("{channel_logfile_stem}.prompt_chips.log")
}

fn format_log_entry(entry: &ChipCommandLogEntry<'_>) -> String {
    let timestamp = Local::now().to_rfc3339_opts(SecondsFormat::Millis, false);
    let status = if entry.timed_out {
        "timed_out"
    } else if entry
        .output
        .is_some_and(|output| output.status == CommandExitStatus::Success)
    {
        "success"
    } else {
        "failure"
    };
    let exit_code = entry
        .output
        .and_then(CommandOutput::exit_code)
        .map(|exit_code| exit_code.to_string())
        .unwrap_or_else(|| MISSING_VALUE.to_string());
    let stdout = entry
        .output
        .map_or(&[][..], |output| output.stdout.as_slice());
    let stderr = entry
        .output
        .map_or(&[][..], |output| output.stderr.as_slice());

    format!(
        "\
===== PROMPT CHIP EXECUTION BEGIN =====
timestamp: {timestamp}
chip_kind: {:?}
chip_title: {}
phase: {}
shell_type: {:?}
working_directory: {}
status: {status}
timed_out: {}
exit_code: {exit_code}
{}
{}
{}
===== PROMPT CHIP EXECUTION END =====

",
        entry.chip_kind,
        entry.chip_title,
        entry.phase.as_str(),
        entry.shell_type,
        format_scalar_field(entry.working_directory),
        entry.timed_out,
        format_text_block("command", "COMMAND", entry.command),
        format_bytes_block("stdout", "STDOUT", stdout),
        format_bytes_block("stderr", "STDERR", stderr),
    )
}

fn format_scalar_field(value: Option<&str>) -> &str {
    value.unwrap_or(MISSING_VALUE)
}

fn format_text_block(label: &str, marker: &str, content: &str) -> String {
    format_block(label, marker, content)
}

fn format_bytes_block(label: &str, marker: &str, content: &[u8]) -> String {
    let content = if content.is_empty() {
        EMPTY_VALUE.to_string()
    } else {
        String::from_utf8_lossy(content).into_owned()
    };

    format_block(label, marker, &content)
}

fn format_block(label: &str, marker: &str, content: &str) -> String {
    let content = if content.is_empty() {
        EMPTY_VALUE
    } else {
        content
    };
    let mut output = String::new();
    output.push_str(label);
    output.push_str(":\n<<<");
    output.push_str(marker);
    output.push('\n');
    output.push_str(content);
    if !content.ends_with('\n') {
        output.push('\n');
    }
    output.push_str(">>>");
    output.push_str(marker);
    output.push('\n');
    output
}

#[cfg(test)]
#[path = "logging_tests.rs"]
mod tests;
