//! Translation of tool requests into computer-use primitives.
//!
//! Ported from warp-server's provider-native computer-use tools
//! (`logic/ai/multi_agent/agents/computer_use/tools`).

use std::time::Duration;

use computer_use::{
    Action, Key, MouseButton, ScreenshotRegion, ScrollDirection, ScrollDistance, Vector2I,
};

use crate::ToolError;
use crate::coords::{Point, ScreenGeometry};
use crate::keys::{KeyDialect, KeyPlatform, parse_chord, return_key};

const MAX_CLICK_COUNT: u8 = 3;
const MAX_SCROLL_AMOUNT: u32 = 50;
const MAX_KEY_REPEAT: u32 = 100;
const MAX_WAIT_SECONDS: f64 = 30.0;

/// A tool invocation, with coordinates in the screenshot's pixel space.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ToolRequest {
    Screenshot,
    Click {
        at: Point,
        button: MouseButton,
        count: u8,
        modifiers: Option<String>,
    },
    MouseMove {
        to: Point,
    },
    MouseDown {
        at: Option<Point>,
        button: MouseButton,
    },
    MouseUp {
        button: MouseButton,
    },
    Drag {
        from: Point,
        to: Point,
        button: MouseButton,
        modifiers: Option<String>,
    },
    Scroll {
        at: Point,
        direction: ScrollDirection,
        amount: u32,
        modifiers: Option<String>,
    },
    TypeText {
        text: String,
    },
    Key {
        keys: String,
        repeat: u32,
    },
    Zoom {
        top_left: Point,
        bottom_right: Point,
    },
    Wait {
        seconds: f64,
    },
    CursorPosition,
}

impl ToolRequest {
    /// Whether planning this request needs the geometry of a full screenshot, to map coordinates
    /// between the screenshot and the native screen.
    pub fn needs_geometry(&self) -> bool {
        match self {
            Self::Click { .. }
            | Self::MouseMove { .. }
            | Self::Drag { .. }
            | Self::Scroll { .. }
            | Self::Zoom { .. }
            | Self::CursorPosition => true,
            Self::MouseDown { at, .. } => at.is_some(),
            Self::Screenshot
            | Self::MouseUp { .. }
            | Self::TypeText { .. }
            | Self::Key { .. }
            | Self::Wait { .. } => false,
        }
    }
}

/// What to capture after a plan's actions run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Capture {
    Nothing,
    FullScreen,
    Region(ScreenshotRegion),
}

/// The primitives that carry out a tool request.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ActionPlan {
    pub actions: Vec<Action>,
    /// A description of the request, with coordinates in the screenshot's pixel space.
    pub summary: String,
    pub capture: Capture,
}

/// Inputs to planning beyond the request itself.
pub(crate) struct PlanContext<'a> {
    pub geometry: Option<&'a ScreenGeometry>,
    pub key_platform: KeyPlatform,
    pub key_dialect: KeyDialect,
    pub settle_delay: Duration,
    /// Whether input requests should capture a screenshot after acting.
    pub screenshot_after_input: bool,
}

impl PlanContext<'_> {
    fn to_native(&self, point: Point) -> Result<Vector2I, ToolError> {
        self.geometry()?.to_native(point)
    }

    fn geometry(&self) -> Result<&ScreenGeometry, ToolError> {
        self.geometry
            .ok_or_else(|| "Take a screenshot before using coordinates.".into())
    }

    fn keys(&self, text: &str) -> Result<Vec<Key>, ToolError> {
        parse_chord(text, self.key_platform, self.key_dialect)
            .map_err(|error| error.to_string().into())
    }

    fn modifier_keys(&self, modifiers: Option<&str>) -> Result<Vec<Key>, ToolError> {
        match modifiers.map(str::trim) {
            Some(modifiers) if !modifiers.is_empty() => self.keys(modifiers),
            Some(_) | None => Ok(Vec::new()),
        }
    }
}

pub(crate) fn plan(request: &ToolRequest, ctx: &PlanContext<'_>) -> Result<ActionPlan, ToolError> {
    let (mut actions, summary) = match request {
        ToolRequest::Screenshot => {
            return Ok(ActionPlan {
                actions: Vec::new(),
                summary: "Screenshot".to_owned(),
                capture: Capture::FullScreen,
            });
        }
        ToolRequest::CursorPosition => {
            return Ok(ActionPlan {
                actions: Vec::new(),
                summary: "Cursor position".to_owned(),
                capture: Capture::Nothing,
            });
        }
        ToolRequest::Zoom {
            top_left,
            bottom_right,
        } => return plan_zoom(*top_left, *bottom_right, ctx),
        ToolRequest::Wait { seconds } => {
            let actions = vec![Action::Wait(wait_duration(*seconds)?)];
            let summary = format!("Wait {seconds:.2}s");
            return Ok(ActionPlan {
                actions,
                summary,
                capture: capture_after_input(ctx),
            });
        }
        ToolRequest::Click {
            at,
            button,
            count,
            modifiers,
        } => {
            if !(1..=MAX_CLICK_COUNT).contains(count) {
                return Err(format!("count must be between 1 and {MAX_CLICK_COUNT}.").into());
            }
            let native = ctx.to_native(*at)?;
            let clicks = (0..*count)
                .flat_map(|_| {
                    [
                        Action::MouseDown {
                            button: *button,
                            at: Some(native),
                        },
                        Action::MouseUp { button: *button },
                    ]
                })
                .collect();
            let actions = with_modifiers(clicks, ctx.modifier_keys(modifiers.as_deref())?);
            let summary = format!("{} at ({}, {})", click_label(*button, *count), at.x, at.y);
            (actions, summary)
        }
        ToolRequest::MouseMove { to } => {
            let actions = vec![Action::MouseMove {
                to: ctx.to_native(*to)?,
            }];
            (actions, format!("Mouse moved to ({}, {})", to.x, to.y))
        }
        ToolRequest::MouseDown { at, button } => {
            let native = at.map(|at| ctx.to_native(at)).transpose()?;
            let actions = vec![Action::MouseDown {
                button: *button,
                at: native,
            }];
            let location = match at {
                Some(at) => format!("at ({}, {})", at.x, at.y),
                None => "at cursor".to_owned(),
            };
            (
                actions,
                format!("{} mouse down {location}", button_name(*button)),
            )
        }
        ToolRequest::MouseUp { button } => (
            vec![Action::MouseUp { button: *button }],
            format!("{} mouse up", button_name(*button)),
        ),
        ToolRequest::Drag {
            from,
            to,
            button,
            modifiers,
        } => {
            let drag = vec![
                Action::MouseDown {
                    button: *button,
                    at: Some(ctx.to_native(*from)?),
                },
                Action::MouseMove {
                    to: ctx.to_native(*to)?,
                },
                Action::MouseUp { button: *button },
            ];
            let actions = with_modifiers(drag, ctx.modifier_keys(modifiers.as_deref())?);
            let summary = format!(
                "{} click drag from ({}, {}) to ({}, {})",
                button_name(*button),
                from.x,
                from.y,
                to.x,
                to.y
            );
            (actions, summary)
        }
        ToolRequest::Scroll {
            at,
            direction,
            amount,
            modifiers,
        } => {
            if !(1..=MAX_SCROLL_AMOUNT).contains(amount) {
                return Err(format!("amount must be between 1 and {MAX_SCROLL_AMOUNT}.").into());
            }
            let scroll = vec![Action::MouseWheel {
                at: ctx.to_native(*at)?,
                direction: *direction,
                distance: ScrollDistance::Clicks(*amount as i32),
            }];
            let actions = with_modifiers(scroll, ctx.modifier_keys(modifiers.as_deref())?);
            let summary = format!(
                "Scroll {} {amount} clicks at ({}, {})",
                direction_name(*direction),
                at.x,
                at.y
            );
            (actions, summary)
        }
        ToolRequest::TypeText { text } => {
            if text.is_empty() {
                return Err("text must not be empty.".into());
            }
            (
                type_text_actions(text, ctx.key_platform),
                format!("Type {text:?}"),
            )
        }
        ToolRequest::Key { keys, repeat } => {
            if !(1..=MAX_KEY_REPEAT).contains(repeat) {
                return Err(format!("repeat must be between 1 and {MAX_KEY_REPEAT}.").into());
            }
            let chord = key_chord(ctx.keys(keys)?);
            let actions = chord
                .iter()
                .cycle()
                .take(chord.len() * *repeat as usize)
                .cloned()
                .collect();
            let summary = if *repeat > 1 {
                format!("Key {keys:?} x{repeat}")
            } else {
                format!("Key {keys:?}")
            };
            (actions, summary)
        }
    };
    let capture = capture_after_input(ctx);
    if capture == Capture::FullScreen && !ctx.settle_delay.is_zero() {
        actions.push(Action::Wait(ctx.settle_delay));
    }
    Ok(ActionPlan {
        actions,
        summary,
        capture,
    })
}

fn plan_zoom(
    top_left: Point,
    bottom_right: Point,
    ctx: &PlanContext<'_>,
) -> Result<ActionPlan, ToolError> {
    let geometry = ctx.geometry()?;
    let region = ScreenshotRegion {
        top_left: geometry.corner_to_native(top_left)?,
        bottom_right: geometry.corner_to_native(bottom_right)?,
    };
    region.validate()?;
    Ok(ActionPlan {
        actions: Vec::new(),
        summary: format!(
            "Zoom into region ({}, {}) to ({}, {})",
            top_left.x, top_left.y, bottom_right.x, bottom_right.y
        ),
        capture: Capture::Region(region),
    })
}

fn capture_after_input(ctx: &PlanContext<'_>) -> Capture {
    if ctx.screenshot_after_input {
        Capture::FullScreen
    } else {
        Capture::Nothing
    }
}

fn wait_duration(seconds: f64) -> Result<Duration, ToolError> {
    if seconds > 0.0 && seconds <= MAX_WAIT_SECONDS {
        Ok(Duration::from_secs_f64(seconds))
    } else {
        Err(format!("seconds must be greater than 0 and at most {MAX_WAIT_SECONDS}.").into())
    }
}

/// Presses every key in order, then releases them in reverse order.
fn key_chord(keys: Vec<Key>) -> Vec<Action> {
    let releases: Vec<_> = keys
        .iter()
        .rev()
        .map(|key| Action::KeyUp { key: key.clone() })
        .collect();
    keys.into_iter()
        .map(|key| Action::KeyDown { key })
        .chain(releases)
        .collect()
}

/// Holds `modifiers` down for the duration of `actions`.
fn with_modifiers(actions: Vec<Action>, modifiers: Vec<Key>) -> Vec<Action> {
    let releases = modifiers
        .iter()
        .rev()
        .map(|key| Action::KeyUp { key: key.clone() })
        .collect::<Vec<_>>();
    modifiers
        .into_iter()
        .map(|key| Action::KeyDown { key })
        .chain(actions)
        .chain(releases)
        .collect()
}

/// Types text segment by segment, pressing Return between lines, because not every platform can
/// synthesize a newline keystroke (X11 has no keysym for it).
fn type_text_actions(text: &str, platform: KeyPlatform) -> Vec<Action> {
    let text = text.replace("\r\n", "\n");
    let return_key = return_key(platform);
    let mut actions = Vec::new();
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            actions.push(Action::KeyDown {
                key: return_key.clone(),
            });
            actions.push(Action::KeyUp {
                key: return_key.clone(),
            });
        }
        if !line.is_empty() {
            actions.push(Action::TypeText {
                text: line.to_owned(),
            });
        }
    }
    actions
}

fn click_label(button: MouseButton, count: u8) -> String {
    let button = button_name(button);
    match count {
        1 => format!("{button} click"),
        2 => format!("{button} double click"),
        _ => format!("{button} triple click"),
    }
}

fn button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "Left",
        MouseButton::Right => "Right",
        MouseButton::Middle => "Middle",
        MouseButton::Back => "Back",
        MouseButton::Forward => "Forward",
    }
}

fn direction_name(direction: ScrollDirection) -> &'static str {
    match direction {
        ScrollDirection::Up => "up",
        ScrollDirection::Down => "down",
        ScrollDirection::Left => "left",
        ScrollDirection::Right => "right",
    }
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
