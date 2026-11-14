//! Per-connection computer-use state shared across tool calls.

use computer_use::{
    Action, ActionResult, Actor, Options, Screenshot, ScreenshotParams, TargetedAction,
};

use crate::actions::{self, ActionPlan, Capture, PlanContext, ToolRequest};
use crate::coords::ScreenGeometry;
use crate::keys::KeyPlatform;
use crate::{ComputerUseMcpConfig, ToolError};

pub(crate) type ActorFactory = Box<dyn Fn() -> Box<dyn Actor> + Send + Sync>;

/// The result of a successful tool call.
#[derive(Debug)]
pub(crate) struct ToolOutput {
    pub text: String,
    pub image: Option<Screenshot>,
}

pub(crate) struct Session {
    config: ComputerUseMcpConfig,
    key_platform: KeyPlatform,
    actor_factory: ActorFactory,
    /// Created on first use and then kept, because actors carry input state across calls (such
    /// as a button held by `mouse_down`) that later calls rely on.
    actor: Option<Box<dyn Actor>>,
    /// The geometry of the most recent full screenshot, which defines the coordinate space of
    /// every tool's input.
    geometry: Option<ScreenGeometry>,
}

impl Session {
    pub fn new(config: ComputerUseMcpConfig, actor_factory: ActorFactory) -> Self {
        Self {
            config,
            key_platform: KeyPlatform::current(),
            actor_factory,
            actor: None,
            geometry: None,
        }
    }

    pub async fn handle(
        &mut self,
        request: ToolRequest,
        screenshot_after_input: bool,
    ) -> Result<ToolOutput, ToolError> {
        if request.needs_geometry() && self.geometry.is_none() {
            self.capture_full_screen().await?;
        }
        let plan = actions::plan(
            &request,
            &PlanContext {
                geometry: self.geometry.as_ref(),
                key_platform: self.key_platform,
                key_dialect: self.config.key_dialect,
                settle_delay: self.config.settle_delay,
                screenshot_after_input,
            },
        )?;
        let result = self.perform(&plan).await?;
        self.output(&request, plan, result)
    }

    async fn capture_full_screen(&mut self) -> Result<(), ToolError> {
        let plan = ActionPlan {
            actions: Vec::new(),
            summary: String::new(),
            capture: Capture::FullScreen,
        };
        let result = self.perform(&plan).await?;
        if result.screenshot.is_none() {
            return Err("The screen could not be captured.".into());
        }
        Ok(())
    }

    async fn perform(&mut self, plan: &ActionPlan) -> Result<ActionResult, ToolError> {
        let constraints = self.config.image_constraints;
        let screenshot_params = match plan.capture {
            Capture::Nothing => None,
            Capture::FullScreen => Some(constraints.screenshot_params(None)),
            Capture::Region(region) => Some(constraints.screenshot_params(Some(region))),
        };
        let actor = self.actor.get_or_insert_with(|| (self.actor_factory)());
        let targeted: Vec<_> = plan
            .actions
            .iter()
            .cloned()
            .map(TargetedAction::screen)
            .collect();
        let result = actor
            .perform_actions(&targeted, unattributed_options(screenshot_params))
            .await;
        match result {
            Ok(result) => {
                if plan.capture == Capture::FullScreen
                    && let Some(screenshot) = &result.screenshot
                {
                    self.geometry = Some(ScreenGeometry::from_screenshot(screenshot, constraints));
                }
                Ok(result)
            }
            Err(error) => {
                release_keys(&mut **actor, &plan.actions).await;
                // An actor without a platform could not connect to a display (for example because
                // one wasn't available yet), so build a fresh one on the next call.
                if actor.platform().is_none() {
                    self.actor = None;
                }
                Err(format!("Computer use failed: {error}").into())
            }
        }
    }

    fn output(
        &self,
        request: &ToolRequest,
        plan: ActionPlan,
        result: ActionResult,
    ) -> Result<ToolOutput, ToolError> {
        let text = match request {
            ToolRequest::Screenshot => self.describe_screenshot(),
            ToolRequest::Zoom { .. } => format!(
                "{}. This image is for inspection only: coordinates still refer to the last full \
                 screenshot.",
                plan.summary
            ),
            ToolRequest::CursorPosition => {
                let native = result
                    .cursor_position
                    .ok_or("The cursor position is unavailable on this platform.")?;
                let geometry = self
                    .geometry
                    .ok_or("Take a screenshot before using coordinates.")?;
                let point = geometry.to_model(native);
                serde_json::json!({ "x": point.x, "y": point.y }).to_string()
            }
            ToolRequest::Click { .. }
            | ToolRequest::MouseMove { .. }
            | ToolRequest::MouseDown { .. }
            | ToolRequest::MouseUp { .. }
            | ToolRequest::Drag { .. }
            | ToolRequest::Scroll { .. }
            | ToolRequest::TypeText { .. }
            | ToolRequest::Key { .. }
            | ToolRequest::Wait { .. } => format!("{}.", plan.summary),
        };
        Ok(ToolOutput {
            text,
            image: result.screenshot,
        })
    }

    fn describe_screenshot(&self) -> String {
        match self.geometry {
            Some(geometry) => {
                let (width, height) = geometry.image_size();
                let (native_width, native_height) = geometry.native_size();
                format!(
                    "Screenshot is {width}x{height} px (native screen {native_width}x\
                     {native_height} px). Use coordinates in this image for all actions."
                )
            }
            None => "Screenshot.".to_owned(),
        }
    }
}

/// Builds options for a whole-screen action batch that isn't part of a recording.
fn unattributed_options(screenshot_params: Option<ScreenshotParams>) -> Options {
    Options {
        screenshot_params,
        background_enabled: false,
        pointer_sink: None,
    }
}

/// Releases every key pressed by `actions`, so a batch that failed partway through doesn't leave
/// modifiers held for later input.
async fn release_keys(actor: &mut dyn Actor, actions: &[Action]) {
    let mut pressed = Vec::new();
    for action in actions {
        if let Action::KeyDown { key } = action
            && !pressed.contains(key)
        {
            pressed.push(key.clone());
        }
    }
    if pressed.is_empty() {
        return;
    }
    let releases: Vec<_> = pressed
        .into_iter()
        .rev()
        .map(|key| TargetedAction::screen(Action::KeyUp { key }))
        .collect();
    if let Err(error) = actor
        .perform_actions(&releases, unattributed_options(None))
        .await
    {
        log::warn!("Failed to release keys after a failed computer use action: {error}");
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
