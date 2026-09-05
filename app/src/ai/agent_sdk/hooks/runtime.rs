use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use instant::Instant;
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};
use tokio::process::Command;
use tokio::sync::{Mutex as AsyncMutex, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::config::{ConfiguredHook, HookConfigSnapshot};
use super::payload::HookPayloadTemplate;
use super::redaction::truncate_utf8;
use super::{
    FailureMode, HookConfigSource, HookEventName, MAX_DENIAL_REASON_BYTES, MAX_OUTPUT_BYTES,
};

const SESSION_END_TOTAL_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug)]
pub(crate) struct OzHookEvent {
    pub(crate) invocation_id: String,
    pub(crate) tool_use_id: Option<String>,
    pub(crate) payload: HookPayloadTemplate,
}

enum ReaderFailure {
    Overflow,
    Io,
}

#[derive(Clone, Debug)]
pub(crate) struct OzPreToolUseEvent(OzHookEvent);

impl OzPreToolUseEvent {
    pub(crate) fn new(event: OzHookEvent) -> Result<Self, HookRuntimeError> {
        if event.payload.event_name() != HookEventName::PreToolUse {
            return Err(HookRuntimeError::WrongEvent);
        }
        Ok(Self(event))
    }
}

#[derive(Clone, Debug)]
pub(crate) enum OzHookCancellationScope {
    Session,
    Invocation(String),
    #[allow(dead_code)]
    Tool(String),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct OzHookObservation {
    pub(crate) diagnostics: Vec<HookInvocationDiagnostic>,
}

#[derive(Clone, Debug)]
pub(crate) enum OzPreToolUseDecision {
    Continue {
        diagnostics: Vec<HookInvocationDiagnostic>,
    },
    Deny {
        reason: String,
        source: HookConfigSource,
        diagnostics: Vec<HookInvocationDiagnostic>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HookInvocationResult {
    Succeeded,
    Continued,
    Denied,
    Failed,
    TimedOut,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HookFailureCategory {
    Spawn,
    Stdin,
    Timeout,
    Cancelled,
    OutputOverflow,
    OutputRead,
    InvalidUtf8,
    NonZeroExit,
    InvalidDecision,
    Payload,
}

#[derive(Clone, Debug)]
pub(crate) struct HookInvocationDiagnostic {
    pub(crate) event: HookEventName,
    pub(crate) source: HookConfigSource,
    pub(crate) config_path: std::path::PathBuf,
    pub(crate) definition_hash: String,
    pub(crate) matcher: Option<String>,
    pub(crate) started_at: SystemTime,
    pub(crate) finished_at: SystemTime,
    pub(crate) duration: Duration,
    pub(crate) result: HookInvocationResult,
    pub(crate) exit_code: Option<i32>,
    pub(crate) output_truncated: bool,
    pub(crate) failure_category: Option<HookFailureCategory>,
}

#[async_trait]
pub(crate) trait OzHookRuntime: Send + Sync {
    async fn observe(&self, event: OzHookEvent) -> OzHookObservation;
    async fn pre_tool_use(&self, event: OzPreToolUseEvent) -> OzPreToolUseDecision;
    fn cancel(&self, scope: OzHookCancellationScope);
}

pub(crate) struct OzHookRuntimeService {
    config: HookConfigSnapshot,
    queue: AsyncMutex<()>,
    cancellation: Mutex<RuntimeCancellation>,
}

struct RuntimeCancellation {
    session: CancellationToken,
    invocations: HashMap<String, PendingInvocation>,
}

struct PendingInvocation {
    tool_use_id: Option<String>,
    token: CancellationToken,
}

impl OzHookRuntimeService {
    pub(crate) fn new(config: HookConfigSnapshot) -> Self {
        Self {
            config,
            queue: AsyncMutex::new(()),
            cancellation: Mutex::new(RuntimeCancellation {
                session: CancellationToken::new(),
                invocations: HashMap::new(),
            }),
        }
    }

    async fn run_event(&self, event: OzHookEvent, pre_tool: bool) -> EventOutcome {
        let token = {
            let mut cancellation = self.cancellation.lock().unwrap();
            let token = cancellation.session.child_token();
            cancellation.invocations.insert(
                event.invocation_id.clone(),
                PendingInvocation {
                    tool_use_id: event.tool_use_id.clone(),
                    token: token.clone(),
                },
            );
            token
        };
        let _queue = tokio::select! {
            guard = self.queue.lock() => guard,
            () = token.cancelled() => {
                self.remove_pending(&event.invocation_id);
                return EventOutcome::default();
            }
        };

        let event_name = event.payload.event_name();
        let session_end_deadline = (event_name == HookEventName::SessionEnd)
            .then(|| Instant::now() + SESSION_END_TOTAL_TIMEOUT);
        let mut outcome = EventOutcome::default();
        for handler in self
            .config
            .matching_handlers(event_name, event.payload.matcher_subject())
        {
            if token.is_cancelled() {
                break;
            }
            let Some(timeout) = effective_timeout(handler.timeout, session_end_deadline) else {
                break;
            };
            let started_at = SystemTime::now();
            let started = Instant::now();
            let result = match event.payload.serialize_for_source(handler.source) {
                Ok(payload) => {
                    run_command(handler, &event.payload, &payload, timeout, token.clone()).await
                }
                Err(_) => Err(CommandFailure {
                    category: HookFailureCategory::Payload,
                    exit_code: None,
                }),
            };
            let mut diagnostic = HookInvocationDiagnostic {
                event: event_name,
                source: handler.source,
                config_path: handler.config_path.clone(),
                definition_hash: handler.definition_hash.clone(),
                matcher: handler.matcher_text.clone(),
                started_at,
                finished_at: SystemTime::now(),
                duration: started.elapsed(),
                result: HookInvocationResult::Succeeded,
                exit_code: None,
                output_truncated: false,
                failure_category: None,
            };
            match result {
                Ok(CommandOutcome::Continue { exit_code }) => {
                    diagnostic.exit_code = exit_code;
                }
                Ok(CommandOutcome::Deny { reason, exit_code }) if pre_tool => {
                    diagnostic.result = HookInvocationResult::Denied;
                    diagnostic.exit_code = exit_code;
                    outcome.diagnostics.push(diagnostic);
                    outcome.denial = Some((reason, handler.source));
                    break;
                }
                Ok(CommandOutcome::Deny { exit_code, .. }) => {
                    diagnostic.result = HookInvocationResult::Failed;
                    diagnostic.exit_code = exit_code;
                    diagnostic.failure_category = Some(HookFailureCategory::InvalidDecision);
                }
                Err(failure) => {
                    diagnostic.exit_code = failure.exit_code;
                    diagnostic.failure_category = Some(failure.category);
                    diagnostic.output_truncated =
                        failure.category == HookFailureCategory::OutputOverflow;
                    diagnostic.result = match failure.category {
                        HookFailureCategory::Timeout => HookInvocationResult::TimedOut,
                        HookFailureCategory::Cancelled => HookInvocationResult::Cancelled,
                        HookFailureCategory::Spawn
                        | HookFailureCategory::Stdin
                        | HookFailureCategory::OutputOverflow
                        | HookFailureCategory::OutputRead
                        | HookFailureCategory::InvalidUtf8
                        | HookFailureCategory::NonZeroExit
                        | HookFailureCategory::InvalidDecision
                        | HookFailureCategory::Payload => {
                            if pre_tool && handler.on_failure == FailureMode::Deny {
                                HookInvocationResult::Denied
                            } else {
                                HookInvocationResult::Continued
                            }
                        }
                    };
                    if pre_tool
                        && handler.on_failure == FailureMode::Deny
                        && failure.category != HookFailureCategory::Cancelled
                    {
                        outcome.diagnostics.push(diagnostic);
                        outcome.denial = Some((
                            "An Oz hook failed closed and denied this tool.".into(),
                            handler.source,
                        ));
                        break;
                    }
                }
            }
            log::info!(
                "Oz hook invocation: event={} source={} config_path={} definition_hash={} \
                 matcher_present={} started_at={:?} finished_at={:?} duration_ms={} result={:?} \
                 exit_code={:?} output_truncated={} failure_category={:?}",
                diagnostic.event,
                diagnostic.source.as_str(),
                diagnostic.config_path.display(),
                diagnostic.definition_hash,
                diagnostic.matcher.is_some(),
                diagnostic.started_at,
                diagnostic.finished_at,
                diagnostic.duration.as_millis(),
                diagnostic.result,
                diagnostic.exit_code,
                diagnostic.output_truncated,
                diagnostic.failure_category
            );
            outcome.diagnostics.push(diagnostic);
        }
        self.remove_pending(&event.invocation_id);
        outcome
    }

    fn remove_pending(&self, invocation_id: &str) {
        self.cancellation
            .lock()
            .unwrap()
            .invocations
            .remove(invocation_id);
    }
}

#[async_trait]
impl OzHookRuntime for OzHookRuntimeService {
    async fn observe(&self, event: OzHookEvent) -> OzHookObservation {
        OzHookObservation {
            diagnostics: self.run_event(event, false).await.diagnostics,
        }
    }

    async fn pre_tool_use(&self, event: OzPreToolUseEvent) -> OzPreToolUseDecision {
        let outcome = self.run_event(event.0, true).await;
        match outcome.denial {
            Some((reason, source)) => OzPreToolUseDecision::Deny {
                reason,
                source,
                diagnostics: outcome.diagnostics,
            },
            None => OzPreToolUseDecision::Continue {
                diagnostics: outcome.diagnostics,
            },
        }
    }

    fn cancel(&self, scope: OzHookCancellationScope) {
        let cancellation = self.cancellation.lock().unwrap();
        match scope {
            OzHookCancellationScope::Session => cancellation.session.cancel(),
            OzHookCancellationScope::Invocation(invocation_id) => {
                if let Some(invocation) = cancellation.invocations.get(&invocation_id) {
                    invocation.token.cancel();
                }
            }
            OzHookCancellationScope::Tool(tool_use_id) => {
                for invocation in cancellation.invocations.values() {
                    if invocation.tool_use_id.as_deref() == Some(&tool_use_id) {
                        invocation.token.cancel();
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct EventOutcome {
    diagnostics: Vec<HookInvocationDiagnostic>,
    denial: Option<(String, HookConfigSource)>,
}

fn effective_timeout(
    configured: Duration,
    session_end_deadline: Option<Instant>,
) -> Option<Duration> {
    let Some(deadline) = session_end_deadline else {
        return Some(configured);
    };
    deadline
        .checked_duration_since(Instant::now())
        .map(|remaining| remaining.min(configured))
        .filter(|remaining| !remaining.is_zero())
}

enum CommandOutcome {
    Continue {
        exit_code: Option<i32>,
    },
    Deny {
        reason: String,
        exit_code: Option<i32>,
    },
}

struct CommandFailure {
    category: HookFailureCategory,
    exit_code: Option<i32>,
}

async fn run_command(
    handler: &ConfiguredHook,
    payload: &HookPayloadTemplate,
    stdin_payload: &[u8],
    timeout: Duration,
    cancellation: CancellationToken,
) -> Result<CommandOutcome, CommandFailure> {
    let mut command = hook_command(handler);
    command
        .current_dir(Path::new(&payload.context.cwd))
        .env_clear()
        .envs(hook_environment(payload))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_process_group(&mut command);
    let mut child = command.spawn().map_err(|_| CommandFailure {
        category: HookFailureCategory::Spawn,
        exit_code: None,
    })?;
    let process_id = child.id();
    let mut stdin = child.stdin.take().unwrap();
    let stdin_payload = stdin_payload.to_vec();
    let stdin_task = tokio::spawn(async move {
        stdin.write_all(&stdin_payload).await?;
        stdin.shutdown().await
    });
    let (overflow_tx, mut overflow_rx) = mpsc::unbounded_channel();
    let stdout_task = spawn_bounded_reader(child.stdout.take().unwrap(), overflow_tx.clone());
    let stderr_task = spawn_bounded_reader(child.stderr.take().unwrap(), overflow_tx);

    enum Completion {
        Exited(std::io::Result<std::process::ExitStatus>),
        Timeout,
        Cancelled,
        Overflow,
    }
    let completion = tokio::select! {
        status = child.wait() => Completion::Exited(status),
        () = tokio::time::sleep(timeout) => Completion::Timeout,
        () = cancellation.cancelled() => Completion::Cancelled,
        Some(()) = overflow_rx.recv() => Completion::Overflow,
    };
    let status = match completion {
        Completion::Exited(status) => status.map_err(|_| CommandFailure {
            category: HookFailureCategory::NonZeroExit,
            exit_code: None,
        })?,
        Completion::Timeout => {
            kill_process_tree(process_id, &mut child).await;
            return Err(CommandFailure {
                category: HookFailureCategory::Timeout,
                exit_code: None,
            });
        }
        Completion::Cancelled => {
            kill_process_tree(process_id, &mut child).await;
            return Err(CommandFailure {
                category: HookFailureCategory::Cancelled,
                exit_code: None,
            });
        }
        Completion::Overflow => {
            kill_process_tree(process_id, &mut child).await;
            return Err(CommandFailure {
                category: HookFailureCategory::OutputOverflow,
                exit_code: None,
            });
        }
    };
    match stdin_task.await {
        Ok(Ok(())) => {}
        Ok(Err(_)) | Err(_) => {
            return Err(CommandFailure {
                category: HookFailureCategory::Stdin,
                exit_code: status.code(),
            });
        }
    }
    let stdout = join_output(stdout_task, status.code()).await?;
    let stderr = join_output(stderr_task, status.code()).await?;
    parse_command_result(payload.event_name(), status.code(), stdout, stderr)
}

fn hook_command(handler: &ConfiguredHook) -> Command {
    #[cfg(windows)]
    {
        let shell = std::env::var_os("COMSPEC").unwrap_or_else(|| OsString::from("cmd.exe"));
        let selected = handler
            .command_windows
            .as_deref()
            .unwrap_or(&handler.command);
        let mut command = Command::new(shell);
        command.arg("/C").arg(selected);
        command
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var_os("SHELL").unwrap_or_else(|| OsString::from("/bin/sh"));
        let mut command = Command::new(shell);
        command.arg("-c").arg(&handler.command);
        command
    }
}

fn hook_environment(payload: &HookPayloadTemplate) -> HashMap<OsString, OsString> {
    #[cfg(windows)]
    const ALLOWED: &[&str] = &[
        "USERPROFILE",
        "PATH",
        "COMSPEC",
        "SystemRoot",
        "TMP",
        "TEMP",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
    ];
    #[cfg(not(windows))]
    const ALLOWED: &[&str] = &[
        "HOME", "PATH", "SHELL", "TMPDIR", "TMP", "TEMP", "LANG", "LC_ALL", "LC_CTYPE",
    ];

    let mut environment = ALLOWED
        .iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (OsString::from(key), value)))
        .collect::<HashMap<_, _>>();
    environment.insert(
        "WARP_HOOK_EVENT_NAME".into(),
        payload.event_name().as_str().into(),
    );
    environment.insert("WARP_RUN_ID".into(), payload.context.run_id.clone().into());
    environment.insert(
        "WARP_CONVERSATION_ID".into(),
        payload.context.conversation_id.clone().into(),
    );
    environment
}

fn spawn_bounded_reader(
    mut reader: impl AsyncRead + Unpin + Send + 'static,
    overflow: mpsc::UnboundedSender<()>,
) -> JoinHandle<Result<Vec<u8>, ReaderFailure>> {
    tokio::spawn(async move {
        let mut output = Vec::new();
        let mut buffer = [0_u8; 8192];
        loop {
            let read = reader
                .read(&mut buffer)
                .await
                .map_err(|_| ReaderFailure::Io)?;
            if read == 0 {
                return Ok(output);
            }
            if output.len() + read > MAX_OUTPUT_BYTES {
                let _ = overflow.send(());
                return Err(ReaderFailure::Overflow);
            }
            output.extend_from_slice(&buffer[..read]);
        }
    })
}

async fn join_output(
    task: JoinHandle<Result<Vec<u8>, ReaderFailure>>,
    exit_code: Option<i32>,
) -> Result<String, CommandFailure> {
    let bytes = task
        .await
        .map_err(|_| CommandFailure {
            category: HookFailureCategory::OutputOverflow,
            exit_code,
        })?
        .map_err(|failure| CommandFailure {
            category: match failure {
                ReaderFailure::Overflow => HookFailureCategory::OutputOverflow,
                ReaderFailure::Io => HookFailureCategory::OutputRead,
            },
            exit_code,
        })?;
    String::from_utf8(bytes).map_err(|_| CommandFailure {
        category: HookFailureCategory::InvalidUtf8,
        exit_code,
    })
}

fn parse_command_result(
    event: HookEventName,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
) -> Result<CommandOutcome, CommandFailure> {
    match (event, exit_code) {
        (HookEventName::PreToolUse, Some(2)) if !stderr.is_empty() => Ok(CommandOutcome::Deny {
            reason: truncate_utf8(&stderr, MAX_DENIAL_REASON_BYTES),
            exit_code,
        }),
        (HookEventName::PreToolUse, Some(2)) => Err(CommandFailure {
            category: HookFailureCategory::InvalidDecision,
            exit_code,
        }),
        (HookEventName::PreToolUse, Some(0)) => parse_pre_tool_stdout(&stdout, exit_code),
        (_, Some(0)) => Ok(CommandOutcome::Continue { exit_code }),
        _ => Err(CommandFailure {
            category: HookFailureCategory::NonZeroExit,
            exit_code,
        }),
    }
}

fn parse_pre_tool_stdout(
    stdout: &str,
    exit_code: Option<i32>,
) -> Result<CommandOutcome, CommandFailure> {
    if stdout.trim().is_empty() {
        return Ok(CommandOutcome::Continue { exit_code });
    }
    let output: PreToolOutput = serde_json::from_str(stdout).map_err(|_| CommandFailure {
        category: HookFailureCategory::InvalidDecision,
        exit_code,
    })?;
    let Some(specific) = output.hook_specific_output else {
        return Ok(CommandOutcome::Continue { exit_code });
    };
    if specific.hook_event_name != HookEventName::PreToolUse
        || specific.permission_decision != PermissionDecision::Deny
        || specific.permission_decision_reason.is_empty()
    {
        return Err(CommandFailure {
            category: HookFailureCategory::InvalidDecision,
            exit_code,
        });
    }
    Ok(CommandOutcome::Deny {
        reason: truncate_utf8(
            &specific.permission_decision_reason,
            MAX_DENIAL_REASON_BYTES,
        ),
        exit_code,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreToolOutput {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: Option<PreToolSpecificOutput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreToolSpecificOutput {
    #[serde(rename = "hookEventName")]
    hook_event_name: HookEventName,
    #[serde(rename = "permissionDecision")]
    permission_decision: PermissionDecision,
    #[serde(rename = "permissionDecisionReason")]
    permission_decision_reason: String,
}

#[derive(Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum PermissionDecision {
    Deny,
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    command.as_std_mut().process_group(0);
}

#[cfg(windows)]
fn configure_process_group(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;
    command.creation_flags(windows::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP.0);
}

#[cfg(unix)]
async fn kill_process_tree(process_id: Option<u32>, child: &mut tokio::process::Child) {
    if let Some(process_id) = process_id {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(process_id as i32),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}

#[cfg(windows)]
async fn kill_process_tree(_process_id: Option<u32>, child: &mut tokio::process::Child) {
    let _ = child.start_kill();
    let _ = child.wait().await;
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum HookRuntimeError {
    #[error("expected a PreToolUse event")]
    WrongEvent,
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
