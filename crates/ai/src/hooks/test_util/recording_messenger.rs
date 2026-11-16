use std::sync::Mutex;

use crate::hooks::model::HookDiagnostic;
use crate::hooks::notices::{HookMessenger, HookNotice};

#[derive(Default)]
pub struct RecordingMessenger {
    notices: Mutex<Vec<HookNotice>>,
    diagnostics: Mutex<Vec<HookDiagnostic>>,
}

impl RecordingMessenger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn notices(&self) -> Vec<HookNotice> {
        self.notices.lock().expect("messenger lock").clone()
    }

    pub fn diagnostics(&self) -> Vec<HookDiagnostic> {
        self.diagnostics.lock().expect("messenger lock").clone()
    }
}

impl HookMessenger for RecordingMessenger {
    fn notify(&self, notice: HookNotice) {
        self.notices.lock().expect("messenger lock").push(notice);
    }

    fn diagnostic(&self, diagnostic: HookDiagnostic) {
        self.diagnostics
            .lock()
            .expect("messenger lock")
            .push(diagnostic);
    }
}
