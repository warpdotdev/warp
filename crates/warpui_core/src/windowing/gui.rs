//! GUI-backend windowing items: scene-building is presenter/GPU machinery
//! that has no TUI analog.

use std::rc::Rc;

use super::WindowCallbackDispatcher;
use crate::platform::WindowContext;
use crate::{AppContext, Scene};

pub(crate) type BuildSceneCallback = Box<dyn Fn(&dyn WindowContext, &mut AppContext) -> Rc<Scene>>;

impl WindowCallbackDispatcher<'_> {
    pub fn build_scene(&mut self, window: &dyn WindowContext) -> Rc<Scene> {
        (self.callbacks.build_scene_callback)(window, &mut self.ctx)
    }
}
