use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ai::agent::action::FileEdit;
use ai::hooks::{
    DenyCause, DispatchOutcome, HookBoundary, HookExecutionHost, HookNotice, HookNoticeKind,
    HookPermissionPolicy, HookReason, HookVerdict, MergedEffects, PermissionDecision,
    PermissionFloors, RunFacts, WarpPermissionResult,
};
use futures::FutureExt;
use futures::future::BoxFuture;
use warp_util::path::ShellFamily;
use warpui::{AppContext, EntityId, ModelHandle, SingletonEntity};

use super::envelope::{EnvelopeSite, envelope_for_tool_pre};
use super::{HeadlessAskBehavior, HookRuntime, HookRuntimeEvent};
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{AIAgentAction, AIAgentActionId, AIAgentActionType};
use crate::ai::blocklist::BlocklistAIPermissions;
use crate::ai::blocklist::permissions::{
    CommandExecutionPermission, CommandExecutionPermissionDeniedReason, FileWritePermission,
    FileWritePermissionDeniedReason,
};
use crate::ai::execution_profiles::ActionPermission;
#[cfg(not(target_family = "wasm"))]
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::ai::paths::host_native_absolute_path;
use crate::terminal::model::session::active_session::ActiveSession;
use crate::workspaces::user_workspaces::TeamScope;

/// How many times one action's hook check may start over because the facts it was keyed on
/// changed underneath it. A hook configuration that keeps changing (a file written in a loop)
/// would otherwise re-run the hooks forever; past this many restarts the action is denied.
const MAX_RESTARTS_PER_ACTION: u32 = 3;

/// Reason an action is denied with when its hook check restarted too many times.
pub(crate) const CONFIGURATION_UNSTABLE_DENY_REASON: &str = "The hook configuration kept changing while this action waited for its hooks, so the action \
     was not run.";

/// What a cached verdict is keyed by. A change to any part of it makes the verdict describe a
/// different operation, so the hooks run again.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct VerdictKey {
    /// Tool call ids are only unique within a conversation, and the cache is shared by every
    /// conversation in the process.
    pub conversation_id: AIConversationId,
    pub action_id: AIAgentActionId,
    /// A hash of the action's arguments as they were when the hooks ran.
    pub argument_revision: u64,
    /// The working directory the operation would run in.
    pub cwd: Option<PathBuf>,
    /// The hook configuration the verdict was judged under.
    pub registry_revision: u64,
}

impl VerdictKey {
    fn action_key(&self) -> ActionKey {
        ActionKey {
            conversation_id: self.conversation_id,
            action_id: self.action_id.clone(),
        }
    }
}

/// The part of a [`VerdictKey`] that names the action. One entry is kept per action; the rest
/// of the key says which operation the entry's verdict describes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ActionKey {
    conversation_id: AIConversationId,
    action_id: AIAgentActionId,
}

/// A stable digest of an action's arguments. The derived `Debug` form covers every field, so a
/// change the hooks should see again changes the digest; a field that prints differently for
/// the same value can only cause an extra hook run, never a stale verdict.
pub(crate) fn argument_revision(action: &AIAgentActionType) -> u64 {
    let mut hasher = DefaultHasher::new();
    format!("{action:?}").hash(&mut hasher);
    hasher.finish()
}

#[derive(Debug, Clone)]
pub(crate) enum PreToolHookState {
    Pending,
    Resolved(Box<MergedEffects>),
}

#[derive(Debug)]
struct VerdictEntry {
    key: VerdictKey,
    state: PreToolHookState,
    /// Set once any caller asked for this action with user approval, so the resumed attempt
    /// keeps that approval. The approval covers the operation the key describes; it is dropped
    /// with the entry when the key changes.
    is_user_initiated: bool,
    cancel: Arc<AtomicBool>,
    /// How many times this action's hook check has started over.
    restarts: u32,
    /// Set once the user was told a hook asked for confirmation, so the notice goes out once
    /// however many times the waiting action is tried.
    confirmation_noticed: bool,
}

impl VerdictEntry {
    /// Whether the entry waits for a verdict that will never come: its dispatch future was
    /// dropped before it finished, which set the cancel flag without resolving anything.
    fn dispatch_died(&self) -> bool {
        matches!(self.state, PreToolHookState::Pending) && self.cancel.load(Ordering::Acquire)
    }
}

/// One entry per action; the entry's key says which operation its verdict describes.
#[derive(Debug, Default)]
pub(crate) struct PreToolVerdictCache {
    entries: HashMap<ActionKey, VerdictEntry>,
}

impl PreToolVerdictCache {
    /// What the cache holds for `key`'s action. An entry for an earlier revision of the
    /// operation, or one whose dispatch died, is removed and its dispatch told to stop.
    fn consult(&mut self, key: &VerdictKey, is_user_initiated: bool) -> Cached {
        let action_key = key.action_key();
        match self.entries.get_mut(&action_key) {
            Some(entry) if entry.key == *key && !entry.dispatch_died() => {
                entry.is_user_initiated |= is_user_initiated;
                Cached::Current(entry.state.clone())
            }
            Some(_) => {
                let entry = self
                    .entries
                    .remove(&action_key)
                    .expect("the entry was just found");
                entry.cancel.store(true, Ordering::Release);
                let same_operation = entry.key == *key;
                log::info!(
                    "hook verdict for {} {}; running its hooks again (restart {})",
                    entry.key.action_id,
                    if same_operation {
                        "was never going to arrive"
                    } else {
                        "no longer describes the action"
                    },
                    entry.restarts + 1
                );
                Cached::Outdated {
                    restarts: entry.restarts + 1,
                    // Approval covers the operation the user saw under the hook configuration
                    // of that moment. Different arguments are a different operation, and a
                    // changed configuration may ask for a confirmation the user never saw, so
                    // approval carries over only when the key is unchanged.
                    is_user_initiated: entry.is_user_initiated && same_operation,
                }
            }
            None => Cached::Nothing,
        }
    }
}

/// Names one dispatch of one action. A later dispatch for the same action gets a new ticket, so
/// a verdict arriving from an earlier dispatch is recognized as stale and dropped.
#[derive(Debug, Clone)]
pub(crate) struct DispatchTicket {
    pub conversation_id: AIConversationId,
    pub action_id: AIAgentActionId,
    cancel: Arc<AtomicBool>,
}

impl DispatchTicket {
    fn action_key(&self) -> ActionKey {
        ActionKey {
            conversation_id: self.conversation_id,
            action_id: self.action_id.clone(),
        }
    }
}

/// A dispatch the caller must spawn. The verdict arrives through [`on_resolved`].
pub(crate) struct PendingDispatch {
    pub ticket: DispatchTicket,
    pub future: BoxFuture<'static, DispatchOutcome>,
}

pub(crate) enum PreToolLookup {
    /// Hooks are off, or no binding applies to this action. Proceed as if hooks did not exist.
    Inactive,
    /// A verdict is being computed. `dispatch` is present the first time an action is seen.
    Pending {
        dispatch: Option<PendingDispatch>,
    },
    Resolved(Box<MergedEffects>),
}

/// Everything the executor needs to resume an action once its verdict is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Resumption {
    pub conversation_id: AIConversationId,
    pub is_user_initiated: bool,
}

/// Sets the cancel flag if the dispatch future is dropped before it finishes, so a runner that
/// polls the flag stops even when nobody is left to wait for the verdict.
struct CancelOnDrop(Option<Arc<AtomicBool>>);

impl CancelOnDrop {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(cancel) = &self.0 {
            cancel.store(true, Ordering::Release);
        }
    }
}

/// What the cache held for an action when a lookup arrived.
enum Cached {
    /// The entry describes the operation being looked up.
    Current(PreToolHookState),
    /// The entry described an earlier revision of the operation, or its dispatch died before
    /// answering; it was dropped and its dispatch told to stop.
    Outdated {
        restarts: u32,
        is_user_initiated: bool,
    },
    Nothing,
}

/// Looks up the verdict for `action`, starting a dispatch if this is the first time the action
/// is seen in its current form.
pub(crate) fn begin_or_lookup(
    action: &AIAgentAction,
    conversation_id: AIConversationId,
    is_user_initiated: bool,
    active_session: &ModelHandle<ActiveSession>,
    ctx: &mut AppContext,
) -> PreToolLookup {
    if !HookRuntime::is_active(ctx) {
        return PreToolLookup::Inactive;
    }
    let runtime = HookRuntime::handle(ctx);
    let snapshot = runtime.as_ref(ctx).snapshot(ctx);
    // The key hashes the whole action, so the cheap check comes first. An entry left behind
    // when the last pre-tool binding went away is dropped when the action finishes.
    if !snapshot.has_bindings_for(HookBoundary::ToolPre) {
        return PreToolLookup::Inactive;
    }
    let key = VerdictKey {
        conversation_id,
        action_id: action.id.clone(),
        argument_revision: argument_revision(&action.action),
        cwd: active_session
            .as_ref(ctx)
            .current_working_directory()
            .cloned()
            .map(PathBuf::from),
        registry_revision: snapshot.revision,
    };
    let action_key = key.action_key();
    let cached = runtime.update(ctx, |runtime, _| {
        runtime.pre_tool_verdicts.consult(&key, is_user_initiated)
    });
    let (restarts, is_user_initiated) = match cached {
        Cached::Current(PreToolHookState::Pending) => {
            return PreToolLookup::Pending { dispatch: None };
        }
        Cached::Current(PreToolHookState::Resolved(merged)) => {
            return PreToolLookup::Resolved(merged);
        }
        Cached::Outdated {
            restarts,
            is_user_initiated: earlier_approval,
        } => (restarts, is_user_initiated || earlier_approval),
        Cached::Nothing => (0, is_user_initiated),
    };
    if restarts > MAX_RESTARTS_PER_ACTION {
        let denied = Box::new(unstable_configuration_denial());
        runtime.update(ctx, |runtime, _| {
            runtime.pre_tool_verdicts.entries.insert(
                action_key,
                VerdictEntry {
                    key,
                    state: PreToolHookState::Resolved(denied.clone()),
                    is_user_initiated,
                    cancel: Arc::new(AtomicBool::new(false)),
                    restarts,
                    confirmation_noticed: false,
                },
            );
        });
        return PreToolLookup::Resolved(denied);
    }

    let site = EnvelopeSite::new(conversation_id, active_session);
    let envelope = envelope_for_tool_pre(action, &site, ctx);
    let has_matching_binding = snapshot
        .bindings_for(
            HookBoundary::ToolPre,
            envelope.cwd.as_deref(),
            HookExecutionHost::Local,
        )
        .iter()
        .any(|active| active.adapter.matches(&active.binding, &envelope));
    if !has_matching_binding {
        return PreToolLookup::Inactive;
    }

    let ticket = DispatchTicket {
        conversation_id,
        action_id: action.id.clone(),
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let dispatch = runtime.update(ctx, |runtime, _| {
        runtime.pre_tool_verdicts.entries.insert(
            action_key,
            VerdictEntry {
                key,
                state: PreToolHookState::Pending,
                is_user_initiated,
                cancel: ticket.cancel.clone(),
                restarts,
                confirmation_noticed: false,
            },
        );
        runtime.dispatcher().dispatch(
            envelope,
            snapshot,
            runtime.dispatch_policy(),
            ticket.cancel.clone(),
        )
    });
    let mut guard = CancelOnDrop(Some(ticket.cancel.clone()));
    let future = async move {
        let outcome = dispatch.await;
        guard.disarm();
        outcome
    }
    .boxed();
    PreToolLookup::Pending {
        dispatch: Some(PendingDispatch { ticket, future }),
    }
}

fn unstable_configuration_denial() -> MergedEffects {
    MergedEffects {
        verdict: HookVerdict::Deny,
        reasons: vec![HookReason {
            text: CONFIGURATION_UNSTABLE_DENY_REASON.to_owned(),
            binding: None,
        }],
        ..Default::default()
    }
}

/// Stores a finished dispatch's verdict. Returns `None`, and drops the verdict, when the dispatch
/// was cancelled or replaced while it ran. A verdict judged under a hook configuration that has
/// since changed is dropped too, but the action is still resumed, without the approval it may
/// have carried: its next lookup sees the entry's key no longer matches and runs the hooks again
/// against the new configuration, which may ask for a confirmation the user has not seen.
pub(crate) fn on_resolved(
    ticket: &DispatchTicket,
    outcome: DispatchOutcome,
    ctx: &mut AppContext,
) -> Option<Resumption> {
    if outcome.cancelled || ticket.cancel.load(Ordering::Acquire) || !HookRuntime::is_installed(ctx)
    {
        return None;
    }
    let action_key = ticket.action_key();
    let current_revision = HookRuntime::as_ref(ctx).snapshot(ctx).revision;
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        let entry = runtime.pre_tool_verdicts.entries.get_mut(&action_key)?;
        if !Arc::ptr_eq(&entry.cancel, &ticket.cancel) {
            return None;
        }
        if outcome.registry_revision != current_revision {
            log::info!(
                "hook verdict for {} was judged under hook configuration revision {} but the \
                 configuration is now at revision {current_revision}; it was dropped",
                ticket.action_id,
                outcome.registry_revision
            );
            entry.is_user_initiated = false;
        } else {
            entry.state = PreToolHookState::Resolved(Box::new(outcome.effects));
        }
        Some(Resumption {
            conversation_id: ticket.conversation_id,
            is_user_initiated: entry.is_user_initiated,
        })
    })
}

/// Applies the permission policy to a resolved verdict and reports what it decided. A denial is
/// delivered to the user as a notice; a denial because nobody could answer a hook's `ask` is
/// also announced to the run's owner when the run asked to end on it. A confirmation step a
/// hook asked for is announced once, with the hook's reason, so the user knows why the card
/// appeared.
pub(crate) fn decide(
    merged: &MergedEffects,
    warp: WarpPermissionResult,
    floors: PermissionFloors,
    action: &AIAgentAction,
    conversation_id: AIConversationId,
    ctx: &mut AppContext,
) -> PermissionDecision {
    let runtime = HookRuntime::as_ref(ctx);
    let run = RunFacts {
        is_headless: runtime.is_headless_for(conversation_id, ctx),
        is_strict: runtime.strict_policy().enabled,
    };
    let headless_ask_behavior = runtime.headless_ask_behavior();
    let messenger = runtime.messenger();

    let decision = HookPermissionPolicy::compose(merged, warp, &floors, run);
    match &decision {
        PermissionDecision::Deny { reason, cause } => {
            messenger.notify(HookNotice {
                conversation_id,
                action_id: Some(action.id.clone()),
                kind: HookNoticeKind::Denied,
                text: reason.text.clone(),
            });
            let ends_the_run = match (cause, headless_ask_behavior) {
                (DenyCause::ApprovalUnavailable, HeadlessAskBehavior::FailRun) => true,
                (DenyCause::ApprovalUnavailable, HeadlessAskBehavior::DenyWithReason)
                | (
                    DenyCause::Hook
                    | DenyCause::RewriteUnsupported
                    | DenyCause::DeferralUnsupported,
                    HeadlessAskBehavior::DenyWithReason | HeadlessAskBehavior::FailRun,
                ) => false,
            };
            if ends_the_run {
                let reason = reason.text.clone();
                let action_id = action.id.clone();
                HookRuntime::handle(ctx).update(ctx, |_, ctx| {
                    ctx.emit(HookRuntimeEvent::ApprovalUnavailable {
                        conversation_id,
                        action_id,
                        reason,
                    });
                });
            }
        }
        PermissionDecision::RequireConfirmation {
            reason: Some(reason),
        } => {
            if first_confirmation_notice(conversation_id, &action.id, ctx) {
                messenger.notify(HookNotice {
                    conversation_id,
                    action_id: Some(action.id.clone()),
                    kind: HookNoticeKind::Warning,
                    text: reason.text.clone(),
                });
            }
        }
        PermissionDecision::RequireConfirmation { reason: None } | PermissionDecision::Execute => {}
    }
    decision
}

/// The facts behind Warp's own permission answer for `action` that no hook may loosen. An action
/// Warp auto-executes has nothing to loosen, so it has no floors; neither has any family other
/// than the shell, file edit, and MCP ones. The floors are read from the same permission
/// functions the per-family auto-execute checks call, so a denylist bypass or a sandbox exemption
/// is applied exactly as Warp applies it. The functions are called again here rather than read
/// from the executor that ran them; where that executor answered before reaching them, the
/// floors still describe the settings and lists as they stand.
pub(crate) fn permission_floors(
    action: &AIAgentActionType,
    conversation_id: &AIConversationId,
    can_auto_execute: bool,
    active_session: &ModelHandle<ActiveSession>,
    terminal_view_id: EntityId,
    scope: &impl TeamScope,
    ctx: &AppContext,
) -> PermissionFloors {
    if can_auto_execute {
        return PermissionFloors::default();
    }
    match action {
        AIAgentActionType::RequestCommandOutput {
            command,
            is_read_only,
            is_risky,
            ..
        } => shell_floors(
            conversation_id,
            command,
            *is_read_only,
            *is_risky,
            active_session,
            terminal_view_id,
            scope,
            ctx,
        ),
        AIAgentActionType::RequestFileEdits { file_edits, .. } => file_write_floors(
            conversation_id,
            file_edits,
            active_session,
            terminal_view_id,
            scope,
            ctx,
        ),
        AIAgentActionType::CallMCPTool {
            server_id, name, ..
        } => mcp_floors(
            server_id.as_ref(),
            McpLookup::Tool { name },
            terminal_view_id,
            ctx,
        ),
        AIAgentActionType::ReadMCPResource {
            server_id,
            name,
            uri,
        } => mcp_floors(
            server_id.as_ref(),
            McpLookup::Resource {
                name,
                uri: uri.as_deref(),
            },
            terminal_view_id,
            ctx,
        ),
        AIAgentActionType::WriteToLongRunningShellCommand { .. }
        | AIAgentActionType::ReadShellCommandOutput { .. }
        | AIAgentActionType::TransferShellCommandControlToUser { .. }
        | AIAgentActionType::ReadFiles(_)
        | AIAgentActionType::UploadArtifact(_)
        | AIAgentActionType::SearchCodebase(_)
        | AIAgentActionType::Grep { .. }
        | AIAgentActionType::FileGlob { .. }
        | AIAgentActionType::FileGlobV2 { .. }
        | AIAgentActionType::SuggestNewConversation { .. }
        | AIAgentActionType::SuggestPrompt(_)
        | AIAgentActionType::InitProject
        | AIAgentActionType::OpenCodeReview
        | AIAgentActionType::ReadDocuments(_)
        | AIAgentActionType::EditDocuments(_)
        | AIAgentActionType::CreateDocuments(_)
        | AIAgentActionType::UseComputer(_)
        | AIAgentActionType::InsertCodeReviewComments { .. }
        | AIAgentActionType::RequestComputerUse(_)
        | AIAgentActionType::StartRecording { .. }
        | AIAgentActionType::StopRecording { .. }
        | AIAgentActionType::ReadSkill(_)
        | AIAgentActionType::FetchConversation { .. }
        | AIAgentActionType::SendMessageToAgent { .. }
        | AIAgentActionType::AskUserQuestion { .. }
        | AIAgentActionType::RunAgents(_)
        | AIAgentActionType::WaitForEvents { .. } => PermissionFloors::default(),
    }
}

/// The floors for a shell command, from `can_autoexecute_command`'s reason. Without a shell type
/// the command cannot be taken apart for the denylist and Warp does not auto-execute it; the
/// setting is still a fact.
#[allow(clippy::too_many_arguments)]
fn shell_floors(
    conversation_id: &AIConversationId,
    command: &str,
    is_read_only: Option<bool>,
    is_risky: Option<bool>,
    active_session: &ModelHandle<ActiveSession>,
    terminal_view_id: EntityId,
    scope: &impl TeamScope,
    ctx: &AppContext,
) -> PermissionFloors {
    let permissions = BlocklistAIPermissions::as_ref(ctx);
    let terminal_view_id = Some(terminal_view_id);
    let Some(escape_char) = active_session
        .as_ref(ctx)
        .shell_type(ctx)
        .map(|shell| ShellFamily::from(shell).escape_char())
    else {
        return PermissionFloors {
            always_ask_setting: permissions.get_execute_commands_setting(
                terminal_view_id,
                scope,
                ctx,
            ) == ActionPermission::AlwaysAsk,
            ..Default::default()
        };
    };
    match permissions.can_autoexecute_command(
        conversation_id,
        command,
        escape_char,
        is_read_only.unwrap_or(false),
        is_risky,
        terminal_view_id,
        scope,
        ctx,
    ) {
        CommandExecutionPermission::Denied(
            CommandExecutionPermissionDeniedReason::ExplicitlyDenylisted,
        ) => PermissionFloors {
            command_denylist_hit: true,
            ..Default::default()
        },
        CommandExecutionPermission::Denied(
            CommandExecutionPermissionDeniedReason::AlwaysAskEnabled,
        ) => PermissionFloors {
            always_ask_setting: true,
            ..Default::default()
        },
        CommandExecutionPermission::Denied(
            CommandExecutionPermissionDeniedReason::AutonomyForceDisabled
            | CommandExecutionPermissionDeniedReason::ContainsRedirection
            | CommandExecutionPermissionDeniedReason::Inconclusive
            | CommandExecutionPermissionDeniedReason::AgentDecided,
        )
        | CommandExecutionPermission::Allowed(_) => PermissionFloors::default(),
    }
}

/// The floors for a file edit, from `can_write_files`'s reason. The model writes paths relative
/// to the session's working directory and the protected paths are absolute, so the paths are
/// resolved the way the file-edit executor resolves them before its own check.
fn file_write_floors(
    conversation_id: &AIConversationId,
    file_edits: &[FileEdit],
    active_session: &ModelHandle<ActiveSession>,
    terminal_view_id: EntityId,
    scope: &impl TeamScope,
    ctx: &AppContext,
) -> PermissionFloors {
    let session = active_session.as_ref(ctx);
    let current_working_directory = session.current_working_directory().cloned();
    let shell = session.shell_launch_data(ctx);
    let paths = file_edits
        .iter()
        .filter_map(|edit| edit.file())
        .map(|file| {
            PathBuf::from(host_native_absolute_path(
                file,
                &shell,
                &current_working_directory,
            ))
        })
        .collect::<Vec<_>>();
    match BlocklistAIPermissions::as_ref(ctx).can_write_files(
        conversation_id,
        &paths,
        Some(terminal_view_id),
        scope,
        ctx,
    ) {
        FileWritePermission::Denied(FileWritePermissionDeniedReason::ProtectedPath) => {
            PermissionFloors {
                protected_write_path: true,
                ..Default::default()
            }
        }
        FileWritePermission::Denied(FileWritePermissionDeniedReason::AlwaysAskEnabled) => {
            PermissionFloors {
                always_ask_setting: true,
                ..Default::default()
            }
        }
        FileWritePermission::Denied(
            FileWritePermissionDeniedReason::AutonomyForceDisabled
            | FileWritePermissionDeniedReason::Inconclusive
            | FileWritePermissionDeniedReason::AgentDecided,
        )
        | FileWritePermission::Allowed(_) => PermissionFloors::default(),
    }
}

/// How an MCP action names its server when it carries no installation id.
enum McpLookup<'a> {
    Tool { name: &'a str },
    Resource { name: &'a str, uri: Option<&'a str> },
}

/// The floors for an MCP call. The server is found the way `can_call_mcp_tool` and
/// `can_read_mcp_resource` find it (by installation id, else by the tool or resource name),
/// because those checks answer only yes or no and do not say whether the denylist was why.
#[cfg(not(target_family = "wasm"))]
fn mcp_floors(
    server_id: Option<&uuid::Uuid>,
    lookup: McpLookup<'_>,
    terminal_view_id: EntityId,
    ctx: &AppContext,
) -> PermissionFloors {
    let permissions = BlocklistAIPermissions::as_ref(ctx);
    let manager = TemplatableMCPServerManager::as_ref(ctx);
    let terminal_view_id = Some(terminal_view_id);
    let template = server_id
        .and_then(|id| manager.get_template_uuid(*id))
        .or_else(|| {
            let installation = match lookup {
                McpLookup::Tool { name } => manager.server_from_tool(name.to_owned()),
                McpLookup::Resource { name, uri } => manager.server_from_resource(name, uri),
            };
            installation
                .copied()
                .and_then(|id| manager.get_template_uuid(id))
        });
    let mcp_server_denylisted = template.is_some_and(|template| {
        permissions
            .get_mcp_denylist(ctx, terminal_view_id)
            .contains(&template)
    });
    let always_ask_setting = permissions.get_mcp_permissions_setting(ctx, terminal_view_id)
        == ActionPermission::AlwaysAsk;
    PermissionFloors {
        mcp_server_denylisted,
        always_ask_setting,
        ..Default::default()
    }
}

/// MCP calls never run on wasm, so the check that would establish a floor never runs either.
#[cfg(target_family = "wasm")]
fn mcp_floors(
    _server_id: Option<&uuid::Uuid>,
    _lookup: McpLookup<'_>,
    _terminal_view_id: EntityId,
    _ctx: &AppContext,
) -> PermissionFloors {
    PermissionFloors::default()
}

/// Marks the action's entry as having announced its confirmation step and says whether this is
/// the first time. Without an entry to remember it on, every call is the first.
fn first_confirmation_notice(
    conversation_id: AIConversationId,
    action_id: &AIAgentActionId,
    ctx: &mut AppContext,
) -> bool {
    let action_key = ActionKey {
        conversation_id,
        action_id: action_id.clone(),
    };
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        match runtime.pre_tool_verdicts.entries.get_mut(&action_key) {
            Some(entry) => !std::mem::replace(&mut entry.confirmation_noticed, true),
            None => true,
        }
    })
}

/// Forgets whatever is known about `action_id` in `conversation_id`. An in-flight dispatch is
/// told to stop and its verdict is dropped when it arrives.
pub(crate) fn invalidate(
    conversation_id: AIConversationId,
    action_id: &AIAgentActionId,
    ctx: &mut AppContext,
) {
    if !HookRuntime::is_installed(ctx) {
        return;
    }
    let action_key = ActionKey {
        conversation_id,
        action_id: action_id.clone(),
    };
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        if let Some(entry) = runtime.pre_tool_verdicts.entries.remove(&action_key) {
            entry.cancel.store(true, Ordering::Release);
        }
    });
}

/// Forgets every action of `conversation_id` and stops its in-flight dispatches.
pub(crate) fn cancel_all_for_conversation(conversation_id: AIConversationId, ctx: &mut AppContext) {
    if !HookRuntime::is_installed(ctx) {
        return;
    }
    HookRuntime::handle(ctx).update(ctx, |runtime, _| {
        runtime.pre_tool_verdicts.entries.retain(|key, entry| {
            if key.conversation_id == conversation_id {
                entry.cancel.store(true, Ordering::Release);
                false
            } else {
                true
            }
        });
    });
}

#[cfg(test)]
fn entry<'a>(
    conversation_id: AIConversationId,
    action_id: &AIAgentActionId,
    ctx: &'a AppContext,
) -> Option<&'a VerdictEntry> {
    HookRuntime::as_ref(ctx)
        .pre_tool_verdicts
        .entries
        .get(&ActionKey {
            conversation_id,
            action_id: action_id.clone(),
        })
}

#[cfg(test)]
pub(crate) fn cached_state(
    conversation_id: AIConversationId,
    action_id: &AIAgentActionId,
    ctx: &AppContext,
) -> Option<PreToolHookState> {
    entry(conversation_id, action_id, ctx).map(|entry| entry.state.clone())
}

#[cfg(test)]
pub(crate) fn cached_key(
    conversation_id: AIConversationId,
    action_id: &AIAgentActionId,
    ctx: &AppContext,
) -> Option<VerdictKey> {
    entry(conversation_id, action_id, ctx).map(|entry| entry.key.clone())
}

#[cfg(test)]
pub(crate) fn cancel_flag(
    conversation_id: AIConversationId,
    action_id: &AIAgentActionId,
    ctx: &AppContext,
) -> Option<Arc<AtomicBool>> {
    entry(conversation_id, action_id, ctx).map(|entry| entry.cancel.clone())
}

#[cfg(test)]
#[path = "pre_tool_tests.rs"]
mod tests;
