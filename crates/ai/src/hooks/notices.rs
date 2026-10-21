use ai_types::{AIAgentActionId, AIConversationId};

use crate::hooks::model::HookDiagnostic;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookNoticeKind {
    Denied,
    Warning,
    Info,
}

/// A message for the user about something hooks did during a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookNotice {
    pub conversation_id: AIConversationId,
    pub action_id: Option<AIAgentActionId>,
    pub kind: HookNoticeKind,
    pub text: String,
}

/// Delivers hook notices and diagnostics to whatever the current surface uses to talk to the user.
pub trait HookMessenger: Send + Sync {
    fn notify(&self, notice: HookNotice);
    fn diagnostic(&self, diagnostic: HookDiagnostic);
}

/// Writes every notice and diagnostic to the log and nowhere else.
pub struct LogOnlyMessenger;

impl HookMessenger for LogOnlyMessenger {
    fn notify(&self, notice: HookNotice) {
        log::info!(
            "hook notice: conversation={} action={:?} kind={:?} text={}",
            notice.conversation_id,
            notice.action_id,
            notice.kind,
            notice.text
        );
    }

    fn diagnostic(&self, diagnostic: HookDiagnostic) {
        log::info!(
            "hook diagnostic: source={} severity={:?} message={}",
            diagnostic.source,
            diagnostic.severity,
            diagnostic.message
        );
    }
}
