// Vendored wasm-logger.
// MIT License: Copyright (c) 2018 Limira
use anyhow::Result;
use log::{Level, Log, Metadata, Record};
use wasm_bindgen::prelude::*;
use web_sys::console;

use crate::LogConfig;

/// Initializes the global logger for the application.
/// Note: On WASM, `config` is ignored since we always log to the browser console.
pub fn init(_config: LogConfig) -> Result<()> {
    // Set a panic hook that captures panic stack traces and writes them out
    // via console.error().
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));

    // Increase the limit on number of frames in a printed stack trace.  This
    // typically defaults to 10, which is too short to be useful for Rust.
    // js-sys doesn't have bindings for `Error.stackTraceLimit`, so we set
    // the value using eval().
    let _ = js_sys::eval("Error.stackTraceLimit = 100");

    // Configure log entries to be written to the JS console.
    init_logger(Config::new(log::Level::Info).module_prefix("warp"));

    Ok(())
}

/// Specify what to be logged
struct Config {
    level: Level,
    module_prefix: Option<String>,
    message_location: MessageLocation,
}

/// Specify where the message will be logged.
enum MessageLocation {
    /// The message will be on the same line as other info (level, path...)
    SameLine,
    /// The message will be on its own line, a new after other info.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    NewLine,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            level: Level::Debug,
            module_prefix: None,
            message_location: MessageLocation::SameLine,
        }
    }
}

impl Config {
    /// Specify the maximum level you want to log
    pub fn new(level: Level) -> Self {
        Self {
            level,
            module_prefix: None,
            message_location: MessageLocation::SameLine,
        }
    }

    /// Configure the `target` of the logger. If specified, the logger
    /// only output for `log`s in module that its path starts with
    /// `module_prefix`. wasm-logger only supports single prefix. Only
    /// the last call to `module_prefix` has effect if you call it multiple times.
    pub fn module_prefix(mut self, module_prefix: &str) -> Self {
        self.module_prefix = Some(module_prefix.to_string());
        self
    }

    /// Put the message on a new line, separated from other information
    /// such as level, file path, line number.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    pub fn message_on_new_line(mut self) -> Self {
        self.message_location = MessageLocation::NewLine;
        self
    }
}

/// The log styles
struct Style {
    lvl_trace: String,
    lvl_debug: String,
    lvl_info: String,
    lvl_warn: String,
    lvl_error: String,
    tgt: String,
    args: String,
}

impl Style {
    fn new() -> Style {
        let base = String::from("color: white; padding: 0 3px; background:");
        Style {
            lvl_trace: format!("{base} gray;"),
            lvl_debug: format!("{base} blue;"),
            lvl_info: format!("{base} green;"),
            lvl_warn: format!("{base} orange;"),
            lvl_error: format!("{base} darkred;"),
            tgt: String::from("font-weight: bold; color: inherit"),
            args: String::from("background: inherit; color: inherit"),
        }
    }
}

/// The logger
struct WasmLogger {
    config: Config,
    style: Style,
}

impl Log for WasmLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        if let Some(ref prefix) = self.config.module_prefix {
            metadata.target().starts_with(prefix)
        } else {
            true
        }
    }

    fn log(&self, record: &Record<'_>) {
        if self.enabled(record.metadata()) {
            let style = &self.style;
            let message_separator = match self.config.message_location {
                MessageLocation::NewLine => "\n",
                MessageLocation::SameLine => " ",
            };
            let s = format!(
                "%c{}%c {}:{}%c{}{}",
                record.level(),
                record.file().unwrap_or_else(|| record.target()),
                record
                    .line()
                    .map_or_else(|| "[Unknown]".to_string(), |line| line.to_string()),
                message_separator,
                record.args(),
            );
            let s = JsValue::from_str(&s);
            let tgt_style = JsValue::from_str(&style.tgt);
            let args_style = JsValue::from_str(&style.args);

            match record.level() {
                Level::Trace => console::debug_4(
                    &s,
                    &JsValue::from(&style.lvl_trace),
                    &tgt_style,
                    &args_style,
                ),
                Level::Debug => console::log_4(
                    &s,
                    &JsValue::from(&style.lvl_debug),
                    &tgt_style,
                    &args_style,
                ),
                Level::Info => {
                    console::info_4(&s, &JsValue::from(&style.lvl_info), &tgt_style, &args_style)
                }
                Level::Warn => {
                    console::warn_4(&s, &JsValue::from(&style.lvl_warn), &tgt_style, &args_style)
                }
                Level::Error => {
                    if record.target() == warp_errors::LOG_TARGET {
                        let error = format!(
                            "ERROR: {}\n\n{}:{}",
                            record.args(),
                            record.file().unwrap_or_else(|| record.target()),
                            record
                                .line()
                                .map_or_else(|| "[Unknown]".to_string(), |line| line.to_string()),
                        );
                        warp_web_event_bus::emit_event(
                            warp_web_event_bus::WarpEvent::ErrorLogged { error },
                        );
                    }

                    console::error_4(
                        &s,
                        &JsValue::from(&style.lvl_error),
                        &tgt_style,
                        &args_style,
                    )
                }
            }
        }
    }

    fn flush(&self) {}
}

/// Initialize the logger with the given config. If initialization fails, it will log a message to the browser console.
fn init_logger(config: Config) {
    let max_level = config.level;
    let wl = WasmLogger {
        config,
        style: Style::new(),
    };

    match log::set_boxed_logger(Box::new(wl)) {
        Ok(_) => log::set_max_level(max_level.to_level_filter()),
        Err(e) => console::error_1(&JsValue::from(e.to_string())),
    }
}
