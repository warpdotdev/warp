//! A CLI tool for manually testing computer use actions.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use computer_use::{
    Action, Key, MouseButton, Options, ScreenshotParams, ScreenshotRegion, Target, TargetedAction,
    Vector2I,
};

#[derive(Parser)]
#[command(name = "use_computer")]
#[command(about = "Manually test computer use actions")]
struct Cli {
    /// Experimental (macOS only): target a specific background window/process instead of the
    /// screen. Deliver events directly to this process ID (and `--window-id`, if given) without
    /// moving the real cursor or raising the window.
    #[arg(long, global = true)]
    pid: Option<i32>,

    /// Experimental (macOS only): the CGWindowID of the window to target, used with `--pid`.
    /// When omitted, the window is resolved from the action's coordinates. Use the `windows`
    /// subcommand to list window ids.
    #[arg(long, global = true)]
    window_id: Option<u32>,

    #[command(subcommand)]
    command: Command,
}

impl Cli {
    /// Resolves the per-action / screenshot target from the CLI flags. A `--pid` selects a
    /// background window target; otherwise the legacy whole-screen target is used.
    fn target(&self) -> Target {
        match self.pid {
            Some(pid) => Target::Window {
                window_id: self.window_id.unwrap_or(0),
                pid,
            },
            None => Target::Screen,
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
    /// Experimental (macOS only): list on-screen windows with their window number, owner PID,
    /// owner name, layer, and bounds, to help identify the right target PID/window.
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

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = Cli::parse();

    // Window listing does not go through the actor's action model; handle it up front.
    if let Command::Windows = cli.command {
        match computer_use::experimental_list_windows() {
            Ok(text) => print!("{text}"),
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let target = cli.target();
    let mut actor = computer_use::create_actor();

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
            // Parse key: if it starts with "0x", treat as keycode; otherwise as character
            let key = if key.starts_with("0x") || key.starts_with("0X") {
                let keycode = i32::from_str_radix(&key[2..], 16).unwrap_or_else(|_| {
                    eprintln!("Invalid keycode: {key}");
                    std::process::exit(1);
                });
                Key::Keycode(keycode)
            } else {
                let mut chars = key.chars();
                let ch = chars.next().unwrap_or_else(|| {
                    eprintln!("Key cannot be empty");
                    std::process::exit(1);
                });
                if chars.next().is_some() {
                    eprintln!("Key must be a single character, got: {key}");
                    std::process::exit(1);
                }
                Key::Char(ch)
            };
            (
                vec![Action::KeyDown { key: key.clone() }, Action::KeyUp { key }],
                None,
                None,
            )
        }
        // Handled before the actor is created, above.
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
    };

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
                    std::process::exit(1);
                }
                println!(
                    "Screenshot saved to {} ({}x{})",
                    path.display(),
                    screenshot.width,
                    screenshot.height
                );
            }
        }
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
