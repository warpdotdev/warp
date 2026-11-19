//! Per-connection computer-use state shared across tool calls.

use computer_use::{
    Action, ActionResult, Actor, Key, MouseButton, Options, Recorder, RecordingConfig,
    RecordingFinalizeReason, Screenshot, TargetedAction, is_meaningful_action_group,
    overlay_labels_for,
};
use serde_json::{Value, json};

use crate::actions::{self, ActionPlan, Capture, PlanContext, ToolRequest};
use crate::coords::ScreenGeometry;
use crate::keys::KeyPlatform;
use crate::recording::{self, ActiveRecording};
use crate::{ComputerUseMcpConfig, ToolError};

pub(crate) type ActorFactory = Box<dyn Fn() -> Box<dyn Actor> + Send + Sync>;

/// A tool call for the session to carry out.
#[derive(Debug)]
pub(crate) enum SessionCommand {
    UseComputer {
        request: ToolRequest,
        screenshot_after_input: bool,
    },
    StartRecording {
        title: Option<String>,
        description: Option<String>,
    },
    StopRecording {
        recording_id: String,
        discard: bool,
    },
}

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
    /// Input that `actor` has pressed and not yet released, to release when the session ends.
    held: HeldInput,
    /// The geometry of the most recent full screenshot, which defines the coordinate space of
    /// every tool's input.
    geometry: Option<ScreenGeometry>,
    recorder: Box<dyn Recorder>,
    recording: Option<ActiveRecording>,
    recordings_started: u32,
}

impl Session {
    pub fn new(
        config: ComputerUseMcpConfig,
        actor_factory: ActorFactory,
        recorder: Box<dyn Recorder>,
    ) -> Self {
        Self {
            config,
            key_platform: KeyPlatform::current(),
            actor_factory,
            actor: None,
            held: HeldInput::default(),
            geometry: None,
            recorder,
            recording: None,
            recordings_started: 0,
        }
    }

    pub async fn execute(&mut self, command: SessionCommand) -> Result<ToolOutput, ToolError> {
        match command {
            SessionCommand::UseComputer {
                request,
                screenshot_after_input,
            } => self.handle(request, screenshot_after_input).await,
            SessionCommand::StartRecording { title, description } => {
                self.start_recording(title, description).await
            }
            SessionCommand::StopRecording {
                recording_id,
                discard,
            } => self.stop_recording(&recording_id, discard).await,
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
        let targeted: Vec<_> = plan
            .actions
            .iter()
            .cloned()
            .map(TargetedAction::screen)
            .collect();
        let (recorded_group, pointer_sink) = self
            .recording
            .as_ref()
            .filter(|_| is_meaningful_action_group(&targeted))
            .map(|recording| {
                recording
                    .action_log
                    .begin_group(overlay_labels_for(&targeted, &plan.summary))
            })
            .unzip();
        let options = Options {
            screenshot_params,
            background_enabled: false,
            pointer_sink,
        };
        let actor = self.actor.get_or_insert_with(|| (self.actor_factory)());
        let result = actor.perform_actions(&targeted, options).await;
        if let (Some(group), Some(recording)) = (recorded_group, self.recording.as_mut()) {
            match &result {
                Ok(_) => recording.action_log.commit_group(group),
                Err(_) => recording.action_log.discard_group(group),
            }
        }
        match result {
            Ok(result) => {
                self.held.apply(&plan.actions);
                if plan.capture == Capture::FullScreen
                    && let Some(screenshot) = &result.screenshot
                {
                    self.geometry = Some(ScreenGeometry::from_screenshot(screenshot, constraints));
                }
                Ok(result)
            }
            Err(error) => {
                // There's no telling how far the batch got, so release everything it may have
                // pressed rather than leave a modifier or mouse button held for later input.
                let pressed = HeldInput::pressed_by(&plan.actions);
                release(&mut **actor, &pressed).await;
                self.held.remove(&pressed);
                // An actor without a platform could not connect to a display (for example because
                // one wasn't available yet), so build a fresh one on the next call.
                if actor.platform().is_none() {
                    self.actor = None;
                    self.held = HeldInput::default();
                }
                Err(format!("Computer use failed: {error}").into())
            }
        }
    }

    /// Releases every key and mouse button that input has left pressed, such as a button held by
    /// `mouse_down`.
    pub async fn release_held_input(&mut self) {
        let held = std::mem::take(&mut self.held);
        if let Some(actor) = self.actor.as_mut() {
            release(&mut **actor, &held).await;
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

    async fn start_recording(
        &mut self,
        title: Option<String>,
        description: Option<String>,
    ) -> Result<ToolOutput, ToolError> {
        if self.config.recording.is_none() {
            return Err("Recording is disabled.".into());
        }
        if let Some(recording) = &self.recording {
            return Err(format!(
                "Recording {:?} is already in progress. Stop it with stop_recording before \
                 starting another.",
                recording.id()
            )
            .into());
        }
        let config = RecordingConfig::default();
        let frame_rate = config.frame_rate;
        let handle = self
            .recorder
            .start(config)
            .await
            .map_err(|error| format!("Failed to start recording: {error}"))?;
        self.recordings_started += 1;
        let recording_id = format!("recording-{}", self.recordings_started);
        let text = json!({
            "recording_id": recording_id,
            "width_px": handle.width(),
            "height_px": handle.height(),
        })
        .to_string();
        self.recording = Some(ActiveRecording::new(
            recording_id,
            handle,
            frame_rate,
            title,
            description,
        ));
        Ok(ToolOutput { text, image: None })
    }

    async fn stop_recording(
        &mut self,
        recording_id: &str,
        discard: bool,
    ) -> Result<ToolOutput, ToolError> {
        let Some(recording) = self
            .recording
            .take_if(|recording| recording.id() == recording_id)
        else {
            return Err(match &self.recording {
                Some(recording) => format!(
                    "No recording has ID {recording_id:?}; the recording in progress is {:?}.",
                    recording.id()
                ),
                None => "No recording is in progress.".to_owned(),
            }
            .into());
        };
        let summary = if discard {
            // Dropping the recording stops capture and deletes the partial video.
            drop(recording);
            json!({ "status": "discarded" })
        } else {
            self.finish(recording, RecordingFinalizeReason::StoppedByAgent)
                .await?
        };
        Ok(ToolOutput {
            text: summary.to_string(),
            image: None,
        })
    }

    /// Stops and publishes the recording in progress, if any, for when the session ends.
    pub async fn finish_recording(&mut self) -> Option<Result<Value, ToolError>> {
        let recording = self.recording.take()?;
        Some(
            self.finish(recording, RecordingFinalizeReason::RunEnded)
                .await,
        )
    }

    async fn finish(
        &self,
        recording: ActiveRecording,
        reason: RecordingFinalizeReason,
    ) -> Result<Value, ToolError> {
        let destination = self
            .config
            .recording
            .as_ref()
            .ok_or("Recording is disabled.")?;
        recording::finish(self.recorder.as_ref(), recording, destination, reason).await
    }

    #[cfg(test)]
    pub fn active_recording(&self) -> Option<&ActiveRecording> {
        self.recording.as_ref()
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

/// Keys and mouse buttons that are pressed, in the order they were pressed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct HeldInput {
    keys: Vec<Key>,
    buttons: Vec<MouseButton>,
}

impl HeldInput {
    /// Every key and button that `actions` press, including those they release again.
    fn pressed_by(actions: &[Action]) -> Self {
        let mut pressed = Self::default();
        for action in actions {
            match action {
                Action::KeyDown { key } => pressed.press_key(key),
                Action::MouseDown { button, .. } => pressed.press_button(*button),
                Action::Wait(_)
                | Action::MouseUp { .. }
                | Action::MouseMove { .. }
                | Action::MouseWheel { .. }
                | Action::TypeText { .. }
                | Action::KeyUp { .. } => {}
            }
        }
        pressed
    }

    /// Updates the held input for `actions` having been performed.
    fn apply(&mut self, actions: &[Action]) {
        for action in actions {
            match action {
                Action::KeyDown { key } => self.press_key(key),
                Action::KeyUp { key } => self.keys.retain(|held| held != key),
                Action::MouseDown { button, .. } => self.press_button(*button),
                Action::MouseUp { button } => self.buttons.retain(|held| held != button),
                Action::Wait(_)
                | Action::MouseMove { .. }
                | Action::MouseWheel { .. }
                | Action::TypeText { .. } => {}
            }
        }
    }

    fn remove(&mut self, released: &HeldInput) {
        self.keys.retain(|key| !released.keys.contains(key));
        self.buttons
            .retain(|button| !released.buttons.contains(button));
    }

    fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.buttons.is_empty()
    }

    /// Actions that release everything held: buttons first, so a modified drag ends with its
    /// modifiers still applied, then keys in reverse press order.
    fn release_actions(&self) -> Vec<TargetedAction> {
        self.buttons
            .iter()
            .map(|&button| Action::MouseUp { button })
            .chain(
                self.keys
                    .iter()
                    .rev()
                    .map(|key| Action::KeyUp { key: key.clone() }),
            )
            .map(TargetedAction::screen)
            .collect()
    }

    fn press_key(&mut self, key: &Key) {
        if !self.keys.contains(key) {
            self.keys.push(key.clone());
        }
    }

    fn press_button(&mut self, button: MouseButton) {
        if !self.buttons.contains(&button) {
            self.buttons.push(button);
        }
    }
}

async fn release(actor: &mut dyn Actor, input: &HeldInput) {
    if input.is_empty() {
        return;
    }
    if let Err(error) = actor
        .perform_actions(
            &input.release_actions(),
            Options {
                screenshot_params: None,
                background_enabled: false,
                pointer_sink: None,
            },
        )
        .await
    {
        log::warn!("Failed to release held computer use input: {error}");
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
