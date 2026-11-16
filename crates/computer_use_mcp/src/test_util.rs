use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use computer_use::{
    ActionResult, Actor, Options, Platform, Screenshot, ScreenshotParams, TargetedAction, Vector2I,
};

use crate::session::ActorFactory;
use crate::{ComputerUseMcpConfig, ImageConstraints, KeyDialect};

pub(crate) const NATIVE_SIZE: (u32, u32) = (2940, 1912);

pub(crate) fn test_config() -> ComputerUseMcpConfig {
    ComputerUseMcpConfig {
        image_constraints: ImageConstraints::ANTHROPIC,
        key_dialect: KeyDialect::Anthropic,
        settle_delay: Duration::from_millis(500),
    }
}

/// A batch of actions the fake actor was asked to perform.
#[derive(Debug, Clone)]
pub(crate) struct RecordedBatch {
    pub actions: Vec<TargetedAction>,
    pub screenshot_params: Option<ScreenshotParams>,
}

/// Shared view of everything performed by the actors a [`FakeActorFactory`] creates.
#[derive(Clone, Default)]
pub(crate) struct FakeActorLog {
    pub batches: Arc<Mutex<Vec<RecordedBatch>>>,
    pub actors_created: Arc<Mutex<usize>>,
    /// When set, the next batch with input actions fails with this error.
    pub fail_next_input: Arc<Mutex<Option<String>>>,
}

impl FakeActorLog {
    pub fn batches(&self) -> Vec<RecordedBatch> {
        self.batches.lock().unwrap().clone()
    }

    pub fn actors_created(&self) -> usize {
        *self.actors_created.lock().unwrap()
    }

    pub fn fail_next_input(&self, error: &str) {
        *self.fail_next_input.lock().unwrap() = Some(error.to_owned());
    }

    pub fn factory(&self) -> ActorFactory {
        let log = self.clone();
        Box::new(move || {
            *log.actors_created.lock().unwrap() += 1;
            Box::new(FakeActor { log: log.clone() })
        })
    }
}

struct FakeActor {
    log: FakeActorLog,
}

#[async_trait]
impl Actor for FakeActor {
    fn platform(&self) -> Option<Platform> {
        Some(Platform::LinuxX11)
    }

    async fn perform_actions(
        &mut self,
        actions: &[TargetedAction],
        options: Options,
    ) -> Result<ActionResult, String> {
        self.log.batches.lock().unwrap().push(RecordedBatch {
            actions: actions.to_vec(),
            screenshot_params: options.screenshot_params,
        });
        if !actions.is_empty()
            && let Some(error) = self.log.fail_next_input.lock().unwrap().take()
        {
            return Err(error);
        }
        let screenshot = options.screenshot_params.map(synthetic_screenshot);
        Ok(ActionResult::legacy(
            screenshot,
            Some(Vector2I::new(1470, 956)),
        ))
    }
}

/// Builds a screenshot with the dimensions the real actors would produce for `params`.
pub(crate) fn synthetic_screenshot(params: ScreenshotParams) -> Screenshot {
    let (native_width, native_height) = match params.region {
        Some(region) => {
            let size = region.bottom_right - region.top_left;
            (size.x() as u32, size.y() as u32)
        }
        None => NATIVE_SIZE,
    };
    let scale = computer_use::screenshot_scale_factor(
        native_width,
        native_height,
        params.max_long_edge_px,
        params.max_total_px,
    );
    let scaled = |value: u32| (value as f64 * scale).max(1.0).round() as usize;
    Screenshot {
        width: scaled(native_width),
        height: scaled(native_height),
        original_width: native_width as usize,
        original_height: native_height as usize,
        data: b"\x89PNG fake".to_vec(),
        mime_type: "image/png".into(),
    }
}
