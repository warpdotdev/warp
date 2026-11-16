use warpui::{Entity, ModelContext, SingletonEntity};

pub struct FileMCPWatcher {}

impl FileMCPWatcher {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {}
    }
}

impl Entity for FileMCPWatcher {
    type Event = ();
}

impl SingletonEntity for FileMCPWatcher {}
