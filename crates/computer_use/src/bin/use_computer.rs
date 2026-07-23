//! A CLI tool for manually testing computer use actions.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use computer_use::{
    Action, Key, MouseButton, Options, ScreenshotParams, ScreenshotRegion, Target, TargetedAction,
    Vector2I,
};

#[derive(Parser)]
#[command(name = "use_computer")]
#[command(about = "Manually test computer use actions")]
struct Cli {
    /// Experimental (macOS and Linux X11): target a specific background window/process instead
    /// of the screen, without moving the real cursor. On macOS events are delivered to this
    /// process ID; on Linux X11 delivery is addressed by `--window-id` and the pid is
    /// informational.
    #[arg(long, global = true)]
    pid: Option<i32>,

    /// Experimental (macOS and Linux X11): the platform window id to target (a CGWindowID on
    /// macOS, an X window id on Linux). Required when `--pid` is given. Use the `windows`
    /// subcommand to list window ids.
    #[arg(long, global = true)]
    window_id: Option<u32>,

    #[command(subcommand)]
    command: Command,
}

impl Cli {
    /// Resolves the per-action / screenshot target from the CLI flags. `--pid` plus
    /// `--window-id` selects a background window target; otherwise the legacy whole-screen
    /// target is used. `main` rejects lone flags up front, so a partial combination can never
    /// silently downgrade to screen targeting or produce a `0`-id window target.
    fn target(&self) -> Target {
        match (self.pid, self.window_id) {
            (Some(pid), Some(window_id)) => Target::Window { window_id, pid },
            (Some(_), None) | (None, Some(_)) | (None, None) => Target::Screen,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Perform a mouse click (mouse down + mouse up) at a position.
    Click {
        /// X coordinate.
        x: i32,
        /// Y coordinate.
        y: i32,
        /// Which mouse button to click.
        #[arg(short, long, default_value = "left")]
        button: Button,
    },
    /// Type text using the keyboard.
    Text {
        /// The text to type.
        text: String,
    },
    /// Take a screenshot and save it to a file.
    Screenshot {
        /// Output file path (PNG format).
        output: PathBuf,
        /// Optional region to capture as "x1,y1,x2,y2" (top-left and bottom-right coordinates).
        /// If not specified, captures the full display.
        #[arg(short, long, value_parser = parse_region)]
        region: Option<(i32, i32, i32, i32)>,
    },
    /// Press a key (key down + key up).
    Keypress {
        /// The key to press. Can be a single character (e.g., "a") or a keycode (e.g., "0x24" for Return on macOS).
        key: String,
    },
    /// Experimental (macOS and Linux X11): list on-screen windows with their window number,
    /// owner PID, owner name, and bounds, to help identify the right target PID/window.
    Windows,
}

#[derive(Clone, ValueEnum)]
enum Button {
    Left,
    Right,
    Middle,
}

impl From<Button> for MouseButton {
    fn from(button: Button) -> Self {
        match button {
            Button::Left => MouseButton::Left,
            Button::Right => MouseButton::Right,
            Button::Middle => MouseButton::Middle,
        }
    }
}

/// Parses a region string "x1,y1,x2,y2" into a tuple of coordinates.
fn parse_region(s: &str) -> Result<(i32, i32, i32, i32), String> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 4 {
        return Err("Region must be specified as 'x1,y1,x2,y2'".to_string());
    }
    let x1 = parts[0]
        .trim()
        .parse::<i32>()
        .map_err(|_| format!("Invalid x1: {}", parts[0]))?;
    let y1 = parts[1]
        .trim()
        .parse::<i32>()
        .map_err(|_| format!("Invalid y1: {}", parts[1]))?;
    let x2 = parts[2]
        .trim()
        .parse::<i32>()
        .map_err(|_| format!("Invalid x2: {}", parts[2]))?;
    let y2 = parts[3]
        .trim()
        .parse::<i32>()
        .map_err(|_| format!("Invalid y2: {}", parts[3]))?;
    Ok((x1, y1, x2, y2))
}

// The binary exits by returning an `ExitCode` rather than calling `std::process::exit`, which
// would skip `Drop` implementations: on Linux X11 the actor owns a server-global input device
// pair that must be removed when the actor is dropped.
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    // Window listing does not go through the actor's action model; handle it up front.
    if let Command::Windows = cli.command {
        return match computer_use::experimental_list_windows() {
            Ok(text) => {
                print!("{text}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Error: {e}");
                ExitCode::FAILURE
            }
        };
    }

    // Window targeting needs both flags: the window id addresses the window, and a lone
    // `--window-id` must not silently downgrade to screen targeting (nor a lone `--pid`
    // produce the ambiguous `0` window-id sentinel).
    match (cli.pid.is_some(), cli.window_id.is_some()) {
        (true, false) => {
            eprintln!(
                "--window-id is required when --pid is given. Use the `windows` subcommand to \
                 list window ids."
            );
            return ExitCode::FAILURE;
        }
        (false, true) => {
            eprintln!(
                "--pid is required when --window-id is given (on Linux X11 the pid is \
                 informational, but both flags select window targeting together). Use the \
                 `windows` subcommand to list window ids and pids."
            );
            return ExitCode::FAILURE;
        }
        (true, true) | (false, false) => {}
    }

    let target = cli.target();

    let (actions, screenshot_params, output_path) = match cli.command {
        Command::Click { x, y, button } => {
            let pos = Vector2I::new(x, y);
            let button: MouseButton = button.into();
            (
                vec![
                    Action::MouseDown {
                        button: button.clone(),
                        at: pos,
                    },
                    Action::MouseUp { button },
                ],
                None,
                None,
            )
        }
        Command::Text { text } => (vec![Action::TypeText { text }], None, None),
        Command::Screenshot { output, region } => {
            let region = region.map(|(x1, y1, x2, y2)| ScreenshotRegion {
                top_left: Vector2I::new(x1, y1),
                bottom_right: Vector2I::new(x2, y2),
            });
            (
                vec![],
                Some(ScreenshotParams {
                    max_long_edge_px: None,
                    max_total_px: None,
                    region,
                    target,
                }),
                Some(output),
            )
        }
        Command::Keypress { key } => {
            let key = match parse_key(&key) {
                Ok(key) => key,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::FAILURE;
                }
            };
            (
                vec![Action::KeyDown { key: key.clone() }, Action::KeyUp { key }],
                None,
                None,
            )
        }
        // Handled up front, above.
        Command::Windows => unreachable!(),
    };

    // Pair every action with the resolved target before handing off to the actor.
    let actions: Vec<TargetedAction> = actions
        .into_iter()
        .map(|action| TargetedAction { action, target })
        .collect();
    // The CLI is a developer tool for exercising window targeting, so background per-window
    // control is always enabled here.
    let options = Options {
        screenshot_params,
        background_enabled: true,
        pointer_sink: None,
    };

    let mut actor = computer_use::create_actor();
    match actor.perform_actions(&actions, options).await {
        Ok(result) => {
            if let Some(pos) = result.cursor_position {
                println!("Cursor position: ({}, {})", pos.x(), pos.y());
            }
            if let Some(screenshot) = result.screenshot
                && let Some(path) = output_path
            {
                if let Err(e) = std::fs::write(&path, &screenshot.data) {
                    eprintln!("Failed to write screenshot: {e}");
                    return ExitCode::FAILURE;
                }
                println!(
                    "Screenshot saved to {} ({}x{})",
                    path.display(),
                    screenshot.width,
                    screenshot.height
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Parses a key argument: a "0x"-prefixed platform keycode, or a single character.
fn parse_key(key: &str) -> Result<Key, String> {
    if key.starts_with("0x") || key.starts_with("0X") {
        let keycode =
            i32::from_str_radix(&key[2..], 16).map_err(|_| format!("Invalid keycode: {key}"))?;
        return Ok(Key::Keycode(keycode));
    }
    let mut chars = key.chars();
    let ch = chars
        .next()
        .ok_or_else(|| "Key cannot be empty".to_string())?;
    if chars.next().is_some() {
        return Err(format!("Key must be a single character, got: {key}"));
    }
    Ok(Key::Char(ch))
}
