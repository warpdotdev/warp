//! Applies Warp's agent permission policy to the requests an ACP agent makes of its client:
//! `session/request_permission` and the `fs/*` methods.
//!
//! Runs are unattended, so anything the native executor would *ask* the user about is allowed
//! here, and the policy's verdict is logged instead. Only what the native executor refuses
//! outright is refused: commands on the user's or organization's execution denylist, and writes
//! to protected files. Surfacing a blocked action for a shared-session viewer to approve is a
//! separate piece of work.
use std::path::PathBuf;

use warp_util::path::EscapeChar;
use warpui::{AppContext, EntityId, SingletonEntity};

use super::mapping::{command_from_raw_input, first_diff, path_from_raw_input};
use super::protocol::{ToolCallFields, ToolKind};
use crate::ai::blocklist::{
    BlocklistAIController, BlocklistAIPermissions, CommandExecutionPermission,
    CommandExecutionPermissionDeniedReason, FileWritePermission, FileWritePermissionDeniedReason,
};

/// Something the agent wants to do that Warp's permission policy has an opinion on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PolicyRequest {
    Execute { command: String },
    WriteFiles { paths: Vec<PathBuf> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PolicyDecision {
    Allow,
    Deny { reason: String },
}

impl PolicyRequest {
    /// The policy-relevant part of a tool call the agent asked permission for, if any. The
    /// agent chooses which fields to include, so paths are gathered from every place it may
    /// have put them.
    pub(super) fn for_tool_call(call: &ToolCallFields) -> Option<Self> {
        match call.kind? {
            ToolKind::Execute => {
                let title = call.title.as_deref().unwrap_or_default();
                command_from_raw_input(call.raw_input.as_ref(), call.kind, title)
                    .map(|command| Self::Execute { command })
            }
            ToolKind::Edit | ToolKind::Delete | ToolKind::Move => {
                let mut paths: Vec<PathBuf> = call
                    .locations
                    .iter()
                    .flatten()
                    .map(|location| PathBuf::from(&location.path))
                    .collect();
                if let Some((path, _, _)) = first_diff(call.content.as_deref()) {
                    paths.push(PathBuf::from(path));
                }
                if let Some(path) = path_from_raw_input(call.raw_input.as_ref()) {
                    paths.push(PathBuf::from(path));
                }
                paths.retain(|path| !path.as_os_str().is_empty());
                paths.dedup();
                (!paths.is_empty()).then_some(Self::WriteFiles { paths })
            }
            ToolKind::Read
            | ToolKind::Search
            | ToolKind::Think
            | ToolKind::Fetch
            | ToolKind::SwitchMode
            | ToolKind::Other
            | ToolKind::Unknown => None,
        }
    }

    pub(super) fn write(path: &str) -> Self {
        Self::WriteFiles {
            paths: vec![PathBuf::from(path)],
        }
    }

    /// Evaluates against the permissions that apply to `controller`'s bound native
    /// conversation, which is the one the agent's turn is rendered through.
    pub(super) fn evaluate(
        &self,
        controller: &BlocklistAIController,
        terminal_view_id: EntityId,
        escape_char: EscapeChar,
        ctx: &AppContext,
    ) -> PolicyDecision {
        let Some(conversation_id) = controller.native_prompt_conversation_id() else {
            log::warn!("No native conversation bound while checking an ACP request; allowing");
            return PolicyDecision::Allow;
        };
        let scope = controller.team_context(ctx);
        let permissions = BlocklistAIPermissions::as_ref(ctx);
        match self {
            Self::Execute { command } => match permissions.can_autoexecute_command(
                &conversation_id,
                command,
                escape_char,
                false,
                None,
                Some(terminal_view_id),
                &scope,
                ctx,
            ) {
                CommandExecutionPermission::Denied(
                    CommandExecutionPermissionDeniedReason::ExplicitlyDenylisted,
                ) => PolicyDecision::Deny {
                    reason: "the command matches the execution denylist".to_owned(),
                },
                CommandExecutionPermission::Denied(reason) => {
                    log::info!(
                        "ACP agent command would need approval in an attended session \
                         (reason={reason:?}); allowing in an unattended run"
                    );
                    PolicyDecision::Allow
                }
                CommandExecutionPermission::Allowed(_) => PolicyDecision::Allow,
            },
            Self::WriteFiles { paths } => match permissions.can_write_files(
                &conversation_id,
                paths,
                Some(terminal_view_id),
                &scope,
                ctx,
            ) {
                FileWritePermission::Denied(FileWritePermissionDeniedReason::ProtectedPath) => {
                    PolicyDecision::Deny {
                        reason: "the path is a protected configuration file".to_owned(),
                    }
                }
                FileWritePermission::Denied(reason) => {
                    log::info!(
                        "ACP agent file write would need approval in an attended session \
                         (reason={reason:?}); allowing in an unattended run"
                    );
                    PolicyDecision::Allow
                }
                FileWritePermission::Allowed(_) => PolicyDecision::Allow,
            },
        }
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
