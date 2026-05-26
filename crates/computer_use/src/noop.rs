use async_trait::async_trait;

use crate::ActionResult;

pub fn is_supported_on_current_platform() -> bool {
    false
}

pub struct Actor;

impl Actor {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl super::Actor for Actor {
    fn platform(&self) -> Option<super::Platform> {
        None
    }

    async fn perform_actions(
        &mut self,
        _actions: &[super::TargetedAction],
        _options: super::Options,
    ) -> Result<ActionResult, String> {
        Ok(ActionResult::legacy(None, None))
    }
}
