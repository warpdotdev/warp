use std::collections::HashMap;

use warp_cli::agent::Harness;
use warpui::{Entity, ModelContext, SingletonEntity};

pub(crate) struct LocalAcpSessionStore {
    sessions_by_harness: HashMap<Harness, String>,
}

impl LocalAcpSessionStore {
    pub(crate) fn new(_: &mut ModelContext<Self>) -> Self {
        Self {
            sessions_by_harness: HashMap::new(),
        }
    }

    pub(crate) fn session_id(&self, harness: Harness) -> Option<&str> {
        self.sessions_by_harness
            .get(&harness)
            .map(std::string::String::as_str)
    }

    pub(crate) fn set_session_id(&mut self, harness: Harness, session_id: String) {
        self.sessions_by_harness.insert(harness, session_id);
    }
}

impl Entity for LocalAcpSessionStore {
    type Event = ();
}

impl SingletonEntity for LocalAcpSessionStore {}
