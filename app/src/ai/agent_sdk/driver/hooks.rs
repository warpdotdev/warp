//! Agent hooks in Oz runs: the hook sources a run's execution configuration selected, how they
//! are found on disk and loaded once, and the messenger that writes hook notices and diagnostics
//! to the run's structured output.

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use ai::hooks::{
    DiagnosticSeverity, ExplicitHookSource, HookDiagnostic, HookDirectory, HookFormat,
    HookMessenger, HookNotice, HookNoticeKind, HookSourceId, HookSourceKind, HookSourceLocation,
    HookSourcePolicy, LogOnlyMessenger, RunMetadata, SourceLoadFailure,
};
use anyhow::Context as _;
use warp_cli::agent::OutputFormat;
use warp_errors::report_if_error;
use warpui::{AppContext, ModelContext, SingletonEntity as _};

use super::{AgentDriver, AgentDriverError, output};
use crate::ai::agent::AIAgentActionId;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::hooks::{HookRuntime, HookRuntimeEvent, RunIdentity, RunSurface};
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};
use crate::ai::cloud_environments::SourceRepo;

/// The hooks a run's execution configuration selected, and the kind of run it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RunHooks {
    /// Agent entries first, then factory entries, each in declared order.
    pub sources: Vec<ConfiguredHookSource>,
    pub surface: RunSurface,
}

/// One hook directory the execution configuration selected, before it is found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfiguredHookSource {
    pub format: HookFormat,
    pub location: ConfiguredHookLocation,
    pub kind: HookSourceKind,
    pub policy: HookSourcePolicy,
}

/// Where a selected hook directory is, as the server described it. The path is checked when the
/// source is found on disk, so a malformed one is a failure of that entry alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConfiguredHookLocation {
    /// A directory relative to the root of one of the run's repositories.
    Repository { repo: SourceRepo, path: String },
    /// An absolute directory on the worker.
    Worker { path: String },
}

impl ConfiguredHookLocation {
    fn describe(&self) -> String {
        match self {
            Self::Repository { repo, path } => format!("{}/{}:{path}", repo.owner, repo.repo),
            Self::Worker { path } => path.clone(),
        }
    }
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

/// Finds each selected hook directory on disk. A repository source is looked up among the
/// repositories the run cloned, each at `working_dir/<name>`; a source whose repository was not
/// cloned, or whose path is malformed, becomes a failure of that entry under its own `strict`.
pub(crate) fn resolve_sources(
    configured: &[ConfiguredHookSource],
    working_dir: &Path,
    repositories: &[SourceRepo],
) -> (Vec<ExplicitHookSource>, Vec<SourceLoadFailure>) {
    let mut sources = Vec::with_capacity(configured.len());
    let mut failures = Vec::new();
    for source in configured {
        match hook_directory(&source.location, working_dir, repositories) {
            Ok(directory) => sources.push(ExplicitHookSource::from_directory(
                source.format,
                &directory,
                source.kind,
                source.policy,
            )),
            Err(error) => failures.push(SourceLoadFailure {
                diagnostic: HookDiagnostic {
                    source: HookSourceId {
                        kind: source.kind,
                        location: HookSourceLocation::Named(source.location.describe()),
                    },
                    severity: DiagnosticSeverity::Error,
                    message: format!("could not find configured hook source: {error:#}"),
                },
                strict: source.policy.strict,
            }),
        }
    }
    (sources, failures)
}

fn hook_directory(
    location: &ConfiguredHookLocation,
    working_dir: &Path,
    repositories: &[SourceRepo],
) -> anyhow::Result<HookDirectory> {
    match location {
        ConfiguredHookLocation::Repository { repo, path } => {
            let relative = crate::ai::agent_sdk::execution_config::hook_repository_path(path)?;
            let cloned = repositories
                .iter()
                .find(|candidate| same_repository(candidate, repo))
                .with_context(|| {
                    format!(
                        "repository {}/{} is not checked out for this run",
                        repo.owner, repo.repo
                    )
                })?;
            let root = working_dir.join(&cloned.repo);
            Ok(HookDirectory::Repository {
                directory: root.join(relative),
                root,
            })
        }
        ConfiguredHookLocation::Worker { path } => Ok(HookDirectory::Worker {
            directory: crate::ai::agent_sdk::execution_config::hook_worker_path(path)?,
        }),
    }
}

/// Repository identity as environment preparation compares it: the forge, and the owner and name
/// without regard to case.
fn same_repository(a: &SourceRepo, b: &SourceRepo) -> bool {
    a.code_forge == b.code_forge
        && a.owner.eq_ignore_ascii_case(&b.owner)
        && a.repo.eq_ignore_ascii_case(&b.repo)
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

impl AgentDriver {
    /// Records the run this process belongs to and installs the messenger that writes to the
    /// run's output.
    pub(super) fn prepare_hooks_for_run(&self, ctx: &mut ModelContext<Self>) {
        // A run with no hooks has nothing to record or report.
        if self.hooks.sources.is_empty() || !HookRuntime::is_installed(ctx) {
            return;
        }
        let run = self.task_id.map(|task_id| RunIdentity {
            metadata: RunMetadata {
                run_id: task_id.to_string(),
                parent_run_id: self.parent_run_id.clone(),
                task_id: Some(task_id.to_string()),
            },
            surface: self.hooks.surface,
        });
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
            runtime.set_messenger(messenger);
        });
    }

    /// Finds the run's selected hook sources among `repositories` (the ones environment
    /// preparation cloned) and on the worker, then loads and finalizes them in one call. Every
    /// failure and every load warning is reported as a diagnostic; a failure of a strict source
    /// also fails the run. A build that cannot run hooks fails a run that has any, so the hooks
    /// are never silently skipped.
    pub(super) fn load_hook_sources_for_run(
        &self,
        repositories: &[SourceRepo],
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), AgentDriverError> {
        // Without `local_fs` no hook file can be read, so such a build cannot run hooks either.
        if !HookRuntime::is_installed(ctx) || !cfg!(feature = "local_fs") {
            return if self.hooks.sources.is_empty() {
                Ok(())
            } else {
                Err(AgentDriverError::HooksUnsupportedByThisBuild)
            };
        }
        let (sources, mut failures) =
            resolve_sources(&self.hooks.sources, &self.working_dir, repositories);
        failures.extend(load_into_registry(&sources, ctx)?);

        let messenger = HookRuntime::as_ref(ctx).messenger();
        for failure in &failures {
            messenger.diagnostic(failure.diagnostic.clone());
        }
        // Errors were just reported as failures. Warnings describe something that already changed
        // what a source does (entries that disagree on where one file's hooks run, a definition
        // the adapter skipped) and go to the run's output too. Info diagnostics only predict what
        // a hook might emit, so they stay in the log.
        for diagnostic in HookRuntime::as_ref(ctx).snapshot(ctx).diagnostics.iter() {
            match diagnostic.severity {
                DiagnosticSeverity::Error => {}
                DiagnosticSeverity::Warning => messenger.diagnostic(diagnostic.clone()),
                DiagnosticSeverity::Info => {
                    log::info!("hook source {}: {}", diagnostic.source, diagnostic.message)
                }
            }
        }
        let strict_failures = failures
            .iter()
            .filter(|failure| failure.strict)
            .map(|failure| {
                format!(
                    "{}: {}",
                    failure.diagnostic.source, failure.diagnostic.message
                )
            })
            .collect::<Vec<_>>();
        if !strict_failures.is_empty() {
            return Err(AgentDriverError::HookSourcesUnavailable {
                details: strict_failures,
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

    /// Fails the conversation where the hook denied an action, or the run's own conversation if
    /// the denial happened on a hidden child's surface.
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

/// Loads `sources` into the registry, finalizing it, and returns the sources that failed.
#[cfg(feature = "local_fs")]
fn load_into_registry(
    sources: &[ExplicitHookSource],
    ctx: &mut AppContext,
) -> Result<Vec<SourceLoadFailure>, AgentDriverError> {
    use ai::hooks::{HookRegistry, load_explicit_sources};

    HookRegistry::handle(ctx)
        .update(ctx, |registry, _| load_explicit_sources(sources, registry))
        .map_err(|error| {
            log::error!("agent hook sources were loaded twice in one run: {error}");
            AgentDriverError::InvalidRuntimeState
        })
}

#[cfg(not(feature = "local_fs"))]
fn load_into_registry(
    sources: &[ExplicitHookSource],
    _: &mut AppContext,
) -> Result<Vec<SourceLoadFailure>, AgentDriverError> {
    Ok(sources
        .iter()
        .map(|source| SourceLoadFailure {
            diagnostic: HookDiagnostic {
                source: HookSourceId {
                    kind: source.kind,
                    location: HookSourceLocation::Named(source.config_file.display().to_string()),
                },
                severity: DiagnosticSeverity::Error,
                message: "hook files cannot be read in this build".to_owned(),
            },
            strict: source.policy.strict,
        })
        .collect())
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
            output::text::hook_diagnostic(&source, severity, &diagnostic.message, w)
        }
    }
}

#[cfg(all(test, unix, feature = "local_fs"))]
impl AgentDriver {
    /// Gives the driver `hooks` and loads them, as a claimed run does after environment
    /// preparation.
    pub(crate) fn load_hooks_for_test(
        &mut self,
        hooks: RunHooks,
        repositories: &[SourceRepo],
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), AgentDriverError> {
        self.hooks = hooks;
        self.load_hook_sources_for_run(repositories, ctx)
    }

    /// Loads `hooks` against `repositories` and starts the run with no initial turn, as the run
    /// loop does after environment preparation, for tests outside this module. The runtime keeps
    /// whatever messenger the test installed.
    pub(crate) fn start_run_with_hooks_for_test(
        &mut self,
        hooks: RunHooks,
        repositories: &[SourceRepo],
        ctx: &mut ModelContext<Self>,
    ) -> Result<
        futures::channel::oneshot::Receiver<super::SDKConversationOutputStatus>,
        AgentDriverError,
    > {
        self.skip_initial_turn = true;
        self.idle_on_complete = None;
        self.load_hooks_for_test(hooks, repositories, ctx)?;
        Ok(self.execute_run(super::AgentRunPrompt::Local(String::new()), ctx))
    }
}

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
