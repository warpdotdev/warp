//! Agent hooks in `warp agent run` and Oz runs: the hook settings a run was launched with, the
//! explicit sources they name, and the messenger that writes hook notices and diagnostics to the
//! run's structured output.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ai::hooks::{
    DiagnosticSeverity, ExplicitHookSource, HookDiagnostic, HookMessenger, HookNotice,
    HookNoticeKind, LogOnlyMessenger, RunMetadata, WARP_HOOK_DIRS_ENV, parse_hook_dirs_value,
    resolve_hook_dirs, sources_for_hook_dirs,
};
#[cfg(not(feature = "local_fs"))]
use ai::hooks::{HookSourceId, HookSourceLocation};
use anyhow::Context as _;
use warp_cli::agent::{HooksOnAsk, OutputFormat};
use warp_errors::report_if_error;
use warpui::{AppContext, ModelContext, SingletonEntity as _};

use super::{AgentDriver, AgentDriverError, output};
use crate::ai::agent::AIAgentActionId;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::hooks::strict_policy::StrictHookPolicy;
use crate::ai::blocklist::hooks::{
    HeadlessAskBehavior, HookRuntime, HookRuntimeEvent, RunIdentity, RunSurface,
};
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};

/// The hook settings a run was launched with. The run's `WARP_HOOK_DIRS` is read separately, when
/// the driver is created.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookRunOptions {
    /// Sources the configuration file named, with absolute paths.
    pub configured_sources: Vec<ExplicitHookSource>,
    /// Directories from `--hook-dir`, added after the `WARP_HOOK_DIRS` entries.
    pub extra_dirs: Vec<PathBuf>,
    pub strict: bool,
    pub on_ask: HooksOnAsk,
    pub surface: RunSurface,
}

/// Whether the server dispatched the run (`--task-id` was passed), which makes it an Oz run. A
/// `warp agent run` without it is a command-line run, even though it creates a task of its own.
pub(crate) fn run_surface_for(dispatched_task: bool) -> RunSurface {
    if dispatched_task {
        RunSurface::Oz
    } else {
        RunSurface::CommandLine
    }
}

pub(crate) fn headless_ask_behavior(on_ask: HooksOnAsk) -> HeadlessAskBehavior {
    match on_ask {
        HooksOnAsk::Deny => HeadlessAskBehavior::DenyWithReason,
        HooksOnAsk::FailRun => HeadlessAskBehavior::FailRun,
    }
}

/// The run's hook directories: the `WARP_HOOK_DIRS` entries, then `extra_dirs`.
/// Relative entries stay relative until the run's working directory resolves them.
pub(crate) fn hook_dirs(env_value: Option<&str>, extra_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = env_value.map(parse_hook_dirs_value).unwrap_or_default();
    dirs.extend(extra_dirs.iter().cloned());
    dirs
}

/// The run's `WARP_HOOK_DIRS` value: the one the run hands its terminal session (the execution's
/// environment variables, managed secrets, and task variables) when it sets one, else the
/// process environment's.
pub(crate) fn hook_dirs_env_value(session_env: &HashMap<OsString, OsString>) -> Option<String> {
    session_env
        .get(OsStr::new(WARP_HOOK_DIRS_ENV))
        .map(|value| value.to_string_lossy().into_owned())
        .or_else(|| std::env::var(WARP_HOOK_DIRS_ENV).ok())
}

/// Every explicit source the run names: one `hooks.json` per hook directory, resolved against
/// `working_dir`, then the configuration file's sources.
pub(crate) fn explicit_sources(
    working_dir: &Path,
    hook_dirs: &[PathBuf],
    configured: &[ExplicitHookSource],
) -> Vec<ExplicitHookSource> {
    let mut sources = sources_for_hook_dirs(&resolve_hook_dirs(working_dir, hook_dirs.to_vec()));
    sources.extend(configured.iter().cloned());
    sources
}

/// The server token of each conversation in this process, by local id, so a record names a
/// conversation the way `conversation_started` does. Filled in as tokens are assigned.
#[derive(Clone, Default)]
pub(crate) struct ConversationTokens(Arc<Mutex<HashMap<AIConversationId, String>>>);

impl ConversationTokens {
    fn insert(&self, conversation_id: AIConversationId, token: String) {
        if let Ok(mut tokens) = self.0.lock() {
            tokens.insert(conversation_id, token);
        }
    }

    fn get(&self, conversation_id: &AIConversationId) -> Option<String> {
        self.0.lock().ok()?.get(conversation_id).cloned()
    }

    /// Records the token of `conversation_id` when it has one.
    fn record(&self, conversation_id: &AIConversationId, ctx: &AppContext) {
        if let Some(token) = BlocklistAIHistoryModel::as_ref(ctx)
            .conversation(conversation_id)
            .and_then(|conversation| conversation.server_conversation_token())
        {
            self.insert(*conversation_id, token.as_str().to_owned());
        }
    }
}

/// Why a named hook source could not take part in the run. These are what a strict run refuses
/// to start with.
#[cfg(not(feature = "local_fs"))]
fn unavailable_source(source: &ExplicitHookSource, message: String) -> HookDiagnostic {
    HookDiagnostic {
        source: HookSourceId {
            kind: source.scope.source_kind(),
            location: HookSourceLocation::Named(source.path.display().to_string()),
        },
        severity: DiagnosticSeverity::Error,
        message,
    }
}

impl AgentDriver {
    /// Records how hooks behave in this run (its identity and surface, the strict policy, what
    /// an unanswerable `ask` does) and installs the messenger that writes to the run's output.
    pub(super) fn prepare_hooks_for_run(&self, ctx: &mut ModelContext<Self>) {
        if !HookRuntime::is_installed(ctx) {
            return;
        }
        let run = self.task_id.map(|task_id| RunIdentity {
            metadata: RunMetadata {
                run_id: task_id.to_string(),
                parent_run_id: self.parent_run_id.clone(),
                task_id: Some(task_id.to_string()),
            },
            surface: self.hook_options.surface,
        });
        let strict = StrictHookPolicy {
            enabled: self.hook_options.strict,
        };
        let ask_behavior = headless_ask_behavior(self.hook_options.on_ask);
        let tokens = ConversationTokens::default();
        let history = BlocklistAIHistoryModel::handle(ctx);
        for (_, conversation) in history.as_ref(ctx).all_live_conversations() {
            tokens.record(&conversation.id(), ctx);
        }
        ctx.subscribe_to_model(&history, {
            let tokens = tokens.clone();
            move |_, _, event, ctx| {
                if let BlocklistAIHistoryEvent::ConversationServerTokenAssigned {
                    conversation_id,
                    ..
                } = event
                {
                    tokens.record(conversation_id, ctx);
                }
            }
        });
        let messenger = Arc::new(OzHookMessenger::new(self.output_format, tokens));
        HookRuntime::handle(ctx).update(ctx, |runtime, _| {
            runtime.set_run(run);
            runtime.set_strict_policy(strict);
            runtime.set_headless_ask_behavior(ask_behavior);
            runtime.set_messenger(messenger);
        });
    }

    /// Loads and finalizes every explicit hook source the run names. In a strict run, any source
    /// that is missing, unreadable, or invalid fails the run.
    pub(super) fn load_hook_sources_for_run(
        &self,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), AgentDriverError> {
        if !HookRuntime::is_installed(ctx) {
            return Ok(());
        }
        let sources = explicit_sources(
            &self.working_dir,
            &self.hook_dirs,
            &self.hook_options.configured_sources,
        );
        HookRuntime::handle(ctx).update(ctx, |runtime, _| {
            runtime.set_delivered_sources(sources.clone());
        });
        let problems = load_into_registry(&sources, ctx);

        let messenger = HookRuntime::as_ref(ctx).messenger();
        for problem in &problems {
            messenger.diagnostic(problem.clone());
        }
        if self.hook_options.strict && !problems.is_empty() {
            return Err(AgentDriverError::HookSourcesUnavailable {
                details: problems
                    .iter()
                    .map(|problem| format!("{}: {}", problem.source, problem.message))
                    .collect(),
            });
        }
        Ok(())
    }

    /// Ends the run as a failure when a hook asks for confirmation and the run is configured to
    /// fail when nobody can answer.
    pub(super) fn subscribe_to_hook_runtime_events(&self, ctx: &mut ModelContext<Self>) {
        if !HookRuntime::is_installed(ctx) {
            return;
        }
        ctx.subscribe_to_model(&HookRuntime::handle(ctx), |me, _, event, ctx| match event {
            HookRuntimeEvent::ApprovalUnavailable {
                conversation_id,
                action_id,
                reason,
            } => me.fail_run_for_hook(*conversation_id, action_id, reason, ctx),
        });
    }

    /// Fails the conversation the hook denied an action in, through the same path a shell exit
    /// takes, so the run reports a failure with the hook's reason. A conversation on another
    /// surface (a hidden child) fails the run's own conversation instead.
    fn fail_run_for_hook(
        &self,
        conversation_id: AIConversationId,
        action_id: &AIAgentActionId,
        reason: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        let terminal_view = self.terminal_driver.as_ref(ctx).terminal_view().clone();
        let terminal_view_id = terminal_view.id();
        let on_this_surface = BlocklistAIHistoryModel::as_ref(ctx)
            .terminal_surface_id_for_conversation(&conversation_id)
            == Some(terminal_view_id);
        let Some(target) = on_this_surface
            .then_some(conversation_id)
            .or(self.run_conversation_id)
        else {
            log::warn!(
                "an agent hook asked to end the run, but the run has no conversation to fail"
            );
            return;
        };
        let message = format!(
            "The run was ended because an agent hook asked for confirmation on action \
             {action_id}, and this run is set to fail when nobody can answer. {reason}"
        );
        let controller = terminal_view.as_ref(ctx).ai_controller().clone();
        controller.update(ctx, |controller, ctx| {
            controller.fail_conversation_due_to_hook(target, message, ctx);
        });
    }
}

/// Loads `sources` into the registry and returns the errors it recorded for them.
#[cfg(feature = "local_fs")]
fn load_into_registry(sources: &[ExplicitHookSource], ctx: &mut AppContext) -> Vec<HookDiagnostic> {
    use ai::hooks::{HookRegistry, load_explicit_sources};

    let mut failures = Vec::new();
    HookRegistry::handle(ctx).update(ctx, |registry, _| {
        failures = load_explicit_sources(sources, registry);
    });
    failures
}

#[cfg(not(feature = "local_fs"))]
fn load_into_registry(sources: &[ExplicitHookSource], _: &mut AppContext) -> Vec<HookDiagnostic> {
    sources
        .iter()
        .map(|source| {
            unavailable_source(source, "hook files cannot be read in this build".to_owned())
        })
        .collect()
}

/// Writes hook notices and diagnostics to the run's standard output in the run's output format,
/// one record per message, and also to the log.
pub(crate) struct OzHookMessenger {
    output_format: OutputFormat,
    tokens: ConversationTokens,
}

impl OzHookMessenger {
    pub(crate) fn new(output_format: OutputFormat, tokens: ConversationTokens) -> Self {
        Self {
            output_format,
            tokens,
        }
    }
}

impl HookMessenger for OzHookMessenger {
    fn notify(&self, notice: HookNotice) {
        let server_token = self.tokens.get(&notice.conversation_id);
        // Each record is written under one stdout lock, so it cannot interleave with the
        // conversation's own records.
        report_if_error!(
            output::with_stdout_buffered(|buf| write_notice(
                &notice,
                server_token.as_deref(),
                self.output_format,
                buf
            ))
            .context("Failed to write a hook notice")
        );
        LogOnlyMessenger.notify(notice);
    }

    fn diagnostic(&self, diagnostic: HookDiagnostic) {
        report_if_error!(
            output::with_stdout_buffered(|buf| write_diagnostic(
                &diagnostic,
                self.output_format,
                buf
            ))
            .context("Failed to write a hook diagnostic")
        );
        LogOnlyMessenger.diagnostic(diagnostic);
    }
}

fn notice_kind_name(kind: HookNoticeKind) -> &'static str {
    match kind {
        HookNoticeKind::Denied => "denied",
        HookNoticeKind::Warning => "warning",
        HookNoticeKind::Info => "info",
    }
}

fn severity_name(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Error => "error",
        DiagnosticSeverity::Warning => "warning",
        DiagnosticSeverity::Info => "info",
    }
}

/// Writes `notice` as one record. `server_token` is the conversation's server token, which the
/// JSON record reports as `conversation_id` like `conversation_started` does; before the server
/// has assigned one it is null, and the local id is always in `local_conversation_id`.
pub(crate) fn write_notice<W: Write>(
    notice: &HookNotice,
    server_token: Option<&str>,
    format: OutputFormat,
    w: &mut W,
) -> io::Result<()> {
    let kind = notice_kind_name(notice.kind);
    match format {
        OutputFormat::Json | OutputFormat::Ndjson => output::json::hook_notice(
            kind,
            server_token,
            &notice.conversation_id.to_string(),
            notice
                .action_id
                .as_ref()
                .map(|id| id.to_string())
                .as_deref(),
            &notice.text,
            w,
        ),
        OutputFormat::Text | OutputFormat::Pretty => {
            output::text::hook_notice(kind, &notice.text, w)
        }
    }
}

pub(crate) fn write_diagnostic<W: Write>(
    diagnostic: &HookDiagnostic,
    format: OutputFormat,
    w: &mut W,
) -> io::Result<()> {
    let severity = severity_name(diagnostic.severity);
    let source = diagnostic.source.to_string();
    match format {
        OutputFormat::Json | OutputFormat::Ndjson => {
            output::json::hook_diagnostic(&source, severity, &diagnostic.message, w)
        }
        OutputFormat::Text | OutputFormat::Pretty => {
            output::text::hook_diagnostic(severity, &source, &diagnostic.message, w)
        }
    }
}

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
