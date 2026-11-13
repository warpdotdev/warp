//! The MCP tool surface.

use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use computer_use::{MouseButton, Recorder, ScrollDirection};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::actions::ToolRequest;
use crate::coords::Point;
use crate::session::{ActorFactory, Session, SessionCommand, ToolOutput};
use crate::{ComputerUseMcpConfig, SERVER_NAME};

const INSTRUCTIONS: &str = "Controls this computer's mouse, keyboard, and screen. Take a \
    screenshot first: every x/y coordinate is measured in pixels of the most recent full \
    screenshot, and input tools return a fresh screenshot after acting. Use zoom to inspect small \
    details; it does not change the coordinate space.";

/// An MCP server that drives this computer's mouse, keyboard, and screen.
#[derive(Clone)]
pub struct ComputerUseMcpServer {
    // Tool calls run one at a time, since concurrent input would interleave unpredictably.
    session: Arc<Mutex<Session>>,
    tool_router: ToolRouter<Self>,
}

impl ComputerUseMcpServer {
    pub fn new(config: ComputerUseMcpConfig) -> Self {
        Self::with_platform(
            config,
            Box::new(computer_use::create_actor),
            computer_use::create_recorder(),
        )
    }

    pub(crate) fn with_platform(
        config: ComputerUseMcpConfig,
        actor_factory: ActorFactory,
        recorder: Box<dyn Recorder>,
    ) -> Self {
        let mut tool_router = Self::computer_tool_router();
        if config.recording.is_some() {
            tool_router += Self::recording_tool_router();
        }
        Self {
            session: Arc::new(Mutex::new(Session::new(config, actor_factory, recorder))),
            tool_router,
        }
    }

    /// Stops and publishes the recording in progress, if any, for when the session ends.
    pub(crate) async fn finish_recording(&self) {
        match self.session.lock().await.finish_recording().await {
            Some(Ok(summary)) => {
                log::info!(
                    "Published the recording that was in progress when the session ended: {summary}"
                );
            }
            Some(Err(error)) => {
                log::warn!(
                    "Failed to publish the recording that was in progress when the session ended: {error}"
                );
            }
            None => {}
        }
    }

    async fn use_computer(
        &self,
        request: ToolRequest,
        screenshot_after_input: bool,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(SessionCommand::UseComputer {
            request,
            screenshot_after_input,
        })
        .await
    }

    async fn run(&self, command: SessionCommand) -> Result<CallToolResult, ErrorData> {
        let session = self.session.clone();
        // Run on a separate task so a cancelled request can't abandon an input batch halfway
        // through, e.g. with modifier keys still held.
        let task = tokio::spawn(async move { session.lock().await.execute(command).await });
        match task.await {
            Ok(Ok(output)) => Ok(CallToolResult::success(content_blocks(output))),
            Ok(Err(error)) => Ok(CallToolResult::error(vec![ContentBlock::text(
                error.to_string(),
            )])),
            Err(error) => Err(ErrorData::internal_error(
                format!("Computer use task failed: {error}"),
                None,
            )),
        }
    }
}

fn content_blocks(output: ToolOutput) -> Vec<ContentBlock> {
    let mut blocks = vec![ContentBlock::text(output.text)];
    if let Some(image) = output.image {
        blocks.push(ContentBlock::image(
            STANDARD.encode(&image.data),
            image.mime_type,
        ));
    }
    blocks
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Button {
    #[default]
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

impl From<Button> for MouseButton {
    fn from(button: Button) -> Self {
        match button {
            Button::Left => MouseButton::Left,
            Button::Right => MouseButton::Right,
            Button::Middle => MouseButton::Middle,
            Button::Back => MouseButton::Back,
            Button::Forward => MouseButton::Forward,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl From<Direction> for ScrollDirection {
    fn from(direction: Direction) -> Self {
        match direction {
            Direction::Up => ScrollDirection::Up,
            Direction::Down => ScrollDirection::Down,
            Direction::Left => ScrollDirection::Left,
            Direction::Right => ScrollDirection::Right,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ClickArgs {
    /// X coordinate in the most recent screenshot.
    x: i32,
    /// Y coordinate in the most recent screenshot.
    y: i32,
    /// The mouse button to click. Defaults to "left".
    button: Option<Button>,
    /// 1 for a single click, 2 for a double click, or 3 for a triple click. Defaults to 1.
    count: Option<u8>,
    /// Modifier keys to hold while clicking, e.g. "shift" or "ctrl+shift".
    modifiers: Option<String>,
    /// Whether to return a screenshot taken after acting. Defaults to true.
    screenshot: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct MouseMoveArgs {
    /// X coordinate in the most recent screenshot.
    x: i32,
    /// Y coordinate in the most recent screenshot.
    y: i32,
    /// Whether to return a screenshot taken after acting. Defaults to true.
    screenshot: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct MouseDownArgs {
    /// X coordinate in the most recent screenshot. Omit both x and y to press at the pointer's
    /// current position.
    x: Option<i32>,
    /// Y coordinate in the most recent screenshot. Omit both x and y to press at the pointer's
    /// current position.
    y: Option<i32>,
    /// The mouse button to press. Defaults to "left".
    button: Option<Button>,
    /// Whether to return a screenshot taken after acting. Defaults to true.
    screenshot: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct MouseUpArgs {
    /// The mouse button to release. Defaults to "left".
    button: Option<Button>,
    /// Whether to return a screenshot taken after acting. Defaults to true.
    screenshot: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DragArgs {
    /// Where to press the button, in the most recent screenshot.
    from: Point,
    /// Where to release the button, in the most recent screenshot.
    to: Point,
    /// The mouse button to drag with. Defaults to "left".
    button: Option<Button>,
    /// Modifier keys to hold while dragging, e.g. "alt".
    modifiers: Option<String>,
    /// Whether to return a screenshot taken after acting. Defaults to true.
    screenshot: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ScrollArgs {
    /// X coordinate to scroll at, in the most recent screenshot.
    x: i32,
    /// Y coordinate to scroll at, in the most recent screenshot.
    y: i32,
    /// The direction to scroll.
    direction: Direction,
    /// How far to scroll, in mouse wheel clicks (1 to 50). Defaults to 3.
    amount: Option<u32>,
    /// Modifier keys to hold while scrolling, e.g. "shift".
    modifiers: Option<String>,
    /// Whether to return a screenshot taken after acting. Defaults to true.
    screenshot: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct TypeTextArgs {
    /// The text to type. Newlines press Return.
    text: String,
    /// Whether to return a screenshot taken after acting. Defaults to true.
    screenshot: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct KeyArgs {
    /// A key or "+"-separated key combination, e.g. "enter", "ctrl+c", or "cmd+shift+t". Write
    /// "plus" for the "+" key.
    keys: String,
    /// How many times to press the combination (1 to 100). Defaults to 1.
    repeat: Option<u32>,
    /// Whether to return a screenshot taken after acting. Defaults to true.
    screenshot: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ZoomArgs {
    /// Left edge of the region, in the most recent full screenshot.
    x1: i32,
    /// Top edge of the region, in the most recent full screenshot.
    y1: i32,
    /// Right edge of the region, in the most recent full screenshot.
    x2: i32,
    /// Bottom edge of the region, in the most recent full screenshot.
    y2: i32,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WaitArgs {
    /// How long to wait, in seconds (greater than 0, at most 30).
    seconds: f64,
    /// Whether to return a screenshot taken after waiting. Defaults to true.
    screenshot: Option<bool>,
}

const DEFAULT_SCROLL_AMOUNT: u32 = 3;

#[tool_router(router = computer_tool_router)]
impl ComputerUseMcpServer {
    #[tool(
        description = "Capture the screen. Every other tool takes coordinates in pixels of the \
                       most recent screenshot, so take one first and again whenever the screen \
                       may have changed.",
        annotations(read_only_hint = true)
    )]
    async fn screenshot(&self) -> Result<CallToolResult, ErrorData> {
        self.use_computer(ToolRequest::Screenshot, true).await
    }

    #[tool(description = "Click a mouse button at a point on the screen.")]
    async fn click(
        &self,
        Parameters(args): Parameters<ClickArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = ToolRequest::Click {
            at: Point {
                x: args.x,
                y: args.y,
            },
            button: args.button.unwrap_or_default().into(),
            count: args.count.unwrap_or(1),
            modifiers: args.modifiers,
        };
        self.use_computer(request, args.screenshot.unwrap_or(true))
            .await
    }

    #[tool(description = "Move the mouse pointer to a point on the screen.")]
    async fn mouse_move(
        &self,
        Parameters(args): Parameters<MouseMoveArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = ToolRequest::MouseMove {
            to: Point {
                x: args.x,
                y: args.y,
            },
        };
        self.use_computer(request, args.screenshot.unwrap_or(true))
            .await
    }

    #[tool(
        description = "Press and hold a mouse button, at a point or at the pointer's current \
                       position. Release it with mouse_up."
    )]
    async fn mouse_down(
        &self,
        Parameters(args): Parameters<MouseDownArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let at = match (args.x, args.y) {
            (Some(x), Some(y)) => Some(Point { x, y }),
            (None, None) => None,
            (Some(_), None) | (None, Some(_)) => {
                return Ok(CallToolResult::error(vec![ContentBlock::text(
                    "Provide both x and y, or neither to press at the pointer's position.",
                )]));
            }
        };
        let request = ToolRequest::MouseDown {
            at,
            button: args.button.unwrap_or_default().into(),
        };
        self.use_computer(request, args.screenshot.unwrap_or(true))
            .await
    }

    #[tool(description = "Release a mouse button held by mouse_down.")]
    async fn mouse_up(
        &self,
        Parameters(args): Parameters<MouseUpArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = ToolRequest::MouseUp {
            button: args.button.unwrap_or_default().into(),
        };
        self.use_computer(request, args.screenshot.unwrap_or(true))
            .await
    }

    #[tool(description = "Press a mouse button at one point, move to another, and release it.")]
    async fn drag(
        &self,
        Parameters(args): Parameters<DragArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = ToolRequest::Drag {
            from: args.from,
            to: args.to,
            button: args.button.unwrap_or_default().into(),
            modifiers: args.modifiers,
        };
        self.use_computer(request, args.screenshot.unwrap_or(true))
            .await
    }

    #[tool(description = "Scroll the mouse wheel at a point on the screen.")]
    async fn scroll(
        &self,
        Parameters(args): Parameters<ScrollArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = ToolRequest::Scroll {
            at: Point {
                x: args.x,
                y: args.y,
            },
            direction: args.direction.into(),
            amount: args.amount.unwrap_or(DEFAULT_SCROLL_AMOUNT),
            modifiers: args.modifiers,
        };
        self.use_computer(request, args.screenshot.unwrap_or(true))
            .await
    }

    #[tool(description = "Type text with the keyboard into the focused element.")]
    async fn type_text(
        &self,
        Parameters(args): Parameters<TypeTextArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = ToolRequest::TypeText { text: args.text };
        self.use_computer(request, args.screenshot.unwrap_or(true))
            .await
    }

    #[tool(
        description = "Press a key or key combination, e.g. \"enter\", \"ctrl+c\", or \
                       \"cmd+shift+t\"."
    )]
    async fn key(
        &self,
        Parameters(args): Parameters<KeyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = ToolRequest::Key {
            keys: args.keys,
            repeat: args.repeat.unwrap_or(1),
        };
        self.use_computer(request, args.screenshot.unwrap_or(true))
            .await
    }

    #[tool(
        description = "Capture a region of the screen at higher resolution to inspect small \
                       details. Coordinates still refer to the last full screenshot afterwards.",
        annotations(read_only_hint = true)
    )]
    async fn zoom(
        &self,
        Parameters(args): Parameters<ZoomArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = ToolRequest::Zoom {
            top_left: Point {
                x: args.x1,
                y: args.y1,
            },
            bottom_right: Point {
                x: args.x2,
                y: args.y2,
            },
        };
        self.use_computer(request, false).await
    }

    #[tool(
        description = "Wait, e.g. for a page to load, then return a screenshot.",
        annotations(read_only_hint = true)
    )]
    async fn wait(
        &self,
        Parameters(args): Parameters<WaitArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let request = ToolRequest::Wait {
            seconds: args.seconds,
        };
        self.use_computer(request, args.screenshot.unwrap_or(true))
            .await
    }

    #[tool(
        description = "Get the mouse pointer's position, in coordinates of the most recent \
                       screenshot.",
        annotations(read_only_hint = true)
    )]
    async fn cursor_position(&self) -> Result<CallToolResult, ErrorData> {
        self.use_computer(ToolRequest::CursorPosition, false).await
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StartRecordingArgs {
    /// A short title for the video, e.g. "Sign-up flow".
    title: Option<String>,
    /// What the video shows.
    description: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct StopRecordingArgs {
    /// The ID returned by start_recording.
    recording_id: String,
    /// Whether to throw the video away instead of publishing it. Defaults to false.
    discard: Option<bool>,
}

#[tool_router(router = recording_tool_router)]
impl ComputerUseMcpServer {
    #[tool(
        description = "Start recording a video of the screen, e.g. to demonstrate a change. Only \
                       one recording can be in progress at a time. Stop it with stop_recording, \
                       which publishes the video."
    )]
    async fn start_recording(
        &self,
        Parameters(args): Parameters<StartRecordingArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(SessionCommand::StartRecording {
            title: args.title,
            description: args.description,
        })
        .await
    }

    #[tool(
        description = "Stop the recording in progress and publish its video, or discard it. \
                       Publishing can take a minute."
    )]
    async fn stop_recording(
        &self,
        Parameters(args): Parameters<StopRecordingArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.run(SessionCommand::StopRecording {
            recording_id: args.recording_id,
            discard: args.discard.unwrap_or(false),
        })
        .await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ComputerUseMcpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
