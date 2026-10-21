//! Agent hooks in `warp agent run` and Oz runs: the hook settings a run was launched with, the
//! explicit sources they name, and the messenger that writes hook notices and diagnostics to the
//! run's structured output.

use std::collections::HashSet;
use std::future::Future;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ai::hooks::{
    DiagnosticSeverity, ExplicitHookSource, HookDiagnostic, HookMessenger, HookNotice,
    HookNoticeKind, HookSourceId, HookSourceKind, HookSourceLocation, LogOnlyMessenger,
    RunMetadata, StaticTrustPolicy, TrustDecision, TrustPolicy as _, WARP_HOOK_DIRS_ENV,
    parse_hook_dirs_value, resolve_hook_dirs, sources_for_hook_dirs,
};
use anyhow::Context as _;
use futures::FutureExt as _;
use repo_metadata::repositories::{DetectedRepositories, RepoDetectionSource};
use warp_cli::agent::{HooksOnAsk, OutputFormat};
use warp_errors::report_if_error;
use warpui::{AppContext, ModelContext, SingletonEntity as _};

use super::{AgentDriver, AgentDriverError, output};
use crate::ai::agent::AIAgentActionId;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::hooks::strict_policy::StrictHookPolicy;
use crate::ai::blocklist::hooks::{
    HeadlessAskBehavior, HookRuntime, HookRuntimeEvent, RunIdentity, RunSurface,
};

/// The hook settings a run was launched with. The process environment's `WARP_HOOK_DIRS` is read
/// separately, when the driver is created.
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

/// The run's hook directories: the process's `WARP_HOOK_DIRS` entries, then `extra_dirs`.
/// Relative entries stay relative until the run's working directory resolves them.
pub(crate) fn hook_dirs(env_value: Option<&str>, extra_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = env_value.map(parse_hook_dirs_value).unwrap_or_default();
    dirs.extend(extra_dirs.iter().cloned());
    dirs
}

/// The process's `WARP_HOOK_DIRS` value, when it is set.
pub(crate) fn hook_dirs_env_value() -> Option<String> {
    std::env::var(WARP_HOOK_DIRS_ENV).ok()
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

/// The git repository `path` is inside, found by looking for a `.git` entry in `path` and each of
/// its parents. There is no stop at the home directory: a configuration entry inside any
/// repository is treated as that repository's, which is the cautious reading.
pub(crate) fn enclosing_repository(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .map(Path::to_path_buf)
}

/// Why a named hook source could not take part in the run. These are what a strict run refuses
/// to start with.
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
    /// Returns the detection of the working directory's repository, which must finish before
    /// the run pins its hooks so a project file there is discovered first.
    pub(super) fn prepare_hooks_for_run(
        &self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<impl Future<Output = ()> + use<>> {
        if !HookRuntime::is_installed(ctx) {
            return None;
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
        let messenger = Arc::new(OzHookMessenger::new(self.output_format));
        HookRuntime::handle(ctx).update(ctx, |runtime, _| {
            runtime.set_run(run);
            runtime.set_strict_policy(strict);
            runtime.set_headless_ask_behavior(ask_behavior);
            runtime.set_messenger(messenger);
        });
        if !ctx.has_singleton_model::<DetectedRepositories>() {
            return None;
        }
        let working_dir = self.working_dir.to_string_lossy().into_owned();
        let detection = DetectedRepositories::handle(ctx).update(ctx, |repositories, ctx| {
            repositories.detect_possible_local_git_repo(
                &working_dir,
                RepoDetectionSource::TerminalNavigation,
                ctx,
            )
        });
        Some(detection.map(|_| ()))
    }

    /// Loads every explicit hook source the run names and reports the ones that could not be
    /// loaded. A configuration file entry inside a git repository is that repository's code,
    /// so it activates only when a repository's own hook file would. In a strict run, any
    /// source that is missing, unreadable, invalid, or held back fails the run.
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
        if sources.is_empty() {
            return Ok(());
        }
        let configured = self
            .hook_options
            .configured_sources
            .iter()
            .map(|source| source.path.as_path())
            .collect::<HashSet<_>>();
        let terminal_view_id = self.terminal_driver.as_ref(ctx).terminal_view().id();
        let posture = HookRuntime::command_execution_posture(Some(terminal_view_id), ctx);
        let surface = HookRuntime::as_ref(ctx).surface(ctx);
        let mut problems = Vec::new();
        let mut loadable = Vec::new();
        for source in sources {
            let repository = configured
                .contains(source.path.as_path())
                .then(|| source.path.parent().and_then(enclosing_repository))
                .flatten();
            let Some(repository) = repository else {
                loadable.push(source);
                continue;
            };
            let as_project_file = HookSourceId {
                kind: HookSourceKind::ProjectCursorFile,
                location: HookSourceLocation::CanonicalPath(source.path.clone()),
            };
            match StaticTrustPolicy.may_activate(&as_project_file, surface, posture) {
                TrustDecision::Activate => loadable.push(source),
                TrustDecision::Inert { reason } => {
                    let message = format!(
                        "not loaded: this configuration entry is inside the repository at {}, \
                         so it follows the rule for a repository's own hooks: {reason}",
                        repository.display()
                    );
                    problems.push(unavailable_source(&source, message));
                }
            }
        }
        problems.extend(load_into_registry(&loadable, ctx));

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

    /// Fixes the run's hook set before its first request. The explicit sources and the
    /// repositories environment preparation registered are all known by now.
    pub(super) fn pin_hooks_for_run(&self, ctx: &mut ModelContext<Self>) {
        if !HookRuntime::is_installed(ctx) {
            return;
        }
        let terminal_view_id = self.terminal_driver.as_ref(ctx).terminal_view().id();
        HookRuntime::handle(ctx).update(ctx, |runtime, ctx| {
            runtime.pin_for_run(Some(terminal_view_id), ctx);
        });
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
    HookRegistry::change(&HookRegistry::handle(ctx), ctx, |registry| {
        failures = load_explicit_sources(sources, registry);
        registry.snapshot().revision
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
}

impl OzHookMessenger {
    pub(crate) fn new(output_format: OutputFormat) -> Self {
        Self { output_format }
    }
}

impl HookMessenger for OzHookMessenger {
    fn notify(&self, notice: HookNotice) {
        // Each record is written under one stdout lock, so it cannot interleave with the
        // conversation's own records.
        report_if_error!(
            output::with_stdout_buffered(|buf| write_notice(&notice, self.output_format, buf))
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

pub(crate) fn write_notice<W: Write>(
    notice: &HookNotice,
    format: OutputFormat,
    w: &mut W,
) -> io::Result<()> {
    let kind = notice_kind_name(notice.kind);
    match format {
        OutputFormat::Json | OutputFormat::Ndjson => output::json::hook_notice(
            kind,
            &notice.conversation_id.to_string(),
            notice.action_id.as_ref().map(|id| id.to_string()).as_deref(),
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
