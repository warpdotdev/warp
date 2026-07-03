//! Defines the pure transition vocabulary and policy for terminal block lifecycle events.
//!
//! This module does not mutate terminal state. It reconciles a remembered lifecycle phase with
//! live block evidence and maps each accepted input to one action and one next phase.

use super::super::block::BlockState;

/// Represents the coordinator's current understanding of the active block lifecycle.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::terminal) enum LifecyclePhase {
    /// A completion created the active block, which has not received its prompt metadata yet.
    AwaitingPrecmd,
    /// The active block has received prompt metadata and is ready for a command start.
    AtPrompt,
    /// A command start was accepted, but `Preexec` has not been observed yet.
    Submitted,
    /// `Preexec` was observed and the active block is executing.
    Executing,
    /// The coordinator cannot prove a more specific phase from its current evidence.
    Unknown,
    /// The terminal has exited and later lifecycle inputs must not mutate it.
    Terminated,
}

/// Identifies the source and block semantics of a command-start intent.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::terminal) enum CommandStartKind {
    /// A user command or queued command should start the ordinary active block.
    UserOrQueued,
    /// A shared-session command should start the ordinary active block.
    SharedSession,
    /// An in-band command should start an in-band active block.
    InBand,
}

/// Classifies a completion-supplied next block ID against the current block list.
///
/// Completion and prompt evidence uses this classification to reject duplicate or colliding IDs
/// before phase-specific policy can authorize any mutation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::terminal) enum NextBlockIdDisposition {
    /// The supplied ID does not belong to any existing block.
    Novel,
    /// The supplied ID is already the active block's ID.
    ActiveDuplicate,
    /// The supplied ID belongs to an existing non-active block.
    ExistingCollision,
}

/// Classifies `Preexec` evidence relative to the active block's observed command.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::terminal) enum PreexecObservation {
    /// This is the first observed `Preexec` for the active block.
    First,
    /// The active block is executing and the observed command is unchanged.
    RepeatedSameCommand,
    /// The active block is executing but the observed command differs.
    RepeatedDifferentCommand,
}

/// Represents every lifecycle input that may affect the active block or terminal epoch.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::terminal) enum LifecycleInput {
    /// A caller intends to start command execution.
    StartCommand(CommandStartKind),
    /// The shell emitted `Preexec`.
    Preexec(PreexecObservation),
    /// The shell emitted the early `CommandFinished` completion signal.
    CommandFinished(NextBlockIdDisposition),
    /// The shell emitted `Precmd` with authoritative completion metadata.
    PrecmdWithCompletionMetadata(NextBlockIdDisposition),
    /// The shell emitted a prompt-only `Precmd` without completion metadata.
    PromptOnlyPrecmd,
    /// The shell initialized a new lifecycle epoch.
    InitShell,
    /// The terminal exited.
    Exit,
}

/// Identifies a lifecycle input without retaining its input-specific evidence.
///
/// Diagnostics use this bounded vocabulary so telemetry remains structured and non-UGC.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::terminal) enum LifecycleInputKind {
    StartCommand,
    Preexec,
    CommandFinished,
    PrecmdWithCompletionMetadata,
    PromptOnlyPrecmd,
    InitShell,
    Exit,
}

impl LifecycleInput {
    /// Returns the bounded input kind used to key and populate lifecycle diagnostics.
    pub(super) fn kind(self) -> LifecycleInputKind {
        match self {
            LifecycleInput::StartCommand(_) => LifecycleInputKind::StartCommand,
            LifecycleInput::Preexec(_) => LifecycleInputKind::Preexec,
            LifecycleInput::CommandFinished(_) => LifecycleInputKind::CommandFinished,
            LifecycleInput::PrecmdWithCompletionMetadata(_) => {
                LifecycleInputKind::PrecmdWithCompletionMetadata
            }
            LifecycleInput::PromptOnlyPrecmd => LifecycleInputKind::PromptOnlyPrecmd,
            LifecycleInput::InitShell => LifecycleInputKind::InitShell,
            LifecycleInput::Exit => LifecycleInputKind::Exit,
        }
    }
}

/// Captures live block and terminal evidence at the point an input is planned.
///
/// The snapshot both guards phase reconciliation and supplies an allowlisted, non-UGC diagnostic
/// record when the transition is ignored or requires recovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::terminal) struct LifecycleSnapshot {
    pub active_block_id: String,
    pub active_session_id: Option<u64>,
    pub supplied_next_block_id: Option<String>,
    pub hook_session_id: Option<u64>,
    pub block_state: BlockState,
    pub started: bool,
    pub finished: bool,
    pub received_precmd: bool,
    pub is_in_band: bool,
    pub is_bootstrapped: bool,
    pub is_bootstrap_done: bool,
    pub is_alt_screen_active: bool,
    pub completion_mismatch: bool,
}

/// Explains why an input was conservatively ignored instead of mutating terminal state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::terminal) enum IgnoreReason {
    CoalescedStart,
    RejectedExecuting,
    IgnoredTerminated,
    DuplicateCompletion,
    CollidingCompletion,
    RepeatedPreexec,
    RepeatedPreexecDifferentCommand,
    RepeatedPrecmd,
    UnsupportedPromptOnlyPrecmd,
    RecoveryDisabled,
}

/// Describes the single mutation, epoch change, or no-op selected by transition policy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::terminal) enum LifecycleAction {
    StartActiveBlock,
    ApplyPreexec,
    AcceptCommandFinished,
    ReconcileCompletionThenApplyPrecmd,
    ApplyPrecmd,
    BeginEpoch,
    Terminate,
    Ignore(IgnoreReason),
}

impl LifecycleAction {
    /// Returns whether the selected action intentionally avoids lifecycle mutation.
    pub(super) fn is_ignored(self) -> bool {
        matches!(self, LifecycleAction::Ignore(_))
    }
}

/// Contains the complete plan for handling one lifecycle input.
///
/// Callers apply `action`, emit any attached diagnostic, and then commit `next_phase`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::terminal) struct LifecycleTransition {
    pub previous_phase: LifecyclePhase,
    pub next_phase: LifecyclePhase,
    pub action: LifecycleAction,
    pub recovery_record: Option<super::LifecycleRecoveryRecord>,
}

/// Reconciles a remembered phase against the active block's live state.
///
/// A specific remembered phase is preserved only when its required block-state, start, and prompt
/// evidence still holds. Any mismatch falls back to [`LifecyclePhase::Unknown`] so subsequent
/// policy cannot authorize a mutation from stale coordinator state. `Unknown` remains unknown
/// until a lifecycle input re-establishes a phase, while `Terminated` is absorbing.
pub(super) fn reconcile_phase(
    phase: LifecyclePhase,
    snapshot: &LifecycleSnapshot,
) -> LifecyclePhase {
    match phase {
        LifecyclePhase::Unknown | LifecyclePhase::Terminated => phase,
        LifecyclePhase::AwaitingPrecmd
            if snapshot.block_state == BlockState::BeforeExecution
                && (!snapshot.started || !snapshot.is_bootstrap_done)
                && !snapshot.received_precmd =>
        {
            phase
        }
        LifecyclePhase::AtPrompt
            if snapshot.block_state == BlockState::BeforeExecution
                && (!snapshot.started || !snapshot.is_bootstrap_done)
                && snapshot.received_precmd =>
        {
            phase
        }
        LifecyclePhase::Submitted
            if snapshot.block_state == BlockState::BeforeExecution && snapshot.started =>
        {
            phase
        }
        LifecyclePhase::Executing if snapshot.block_state == BlockState::Executing => phase,
        LifecyclePhase::AwaitingPrecmd
        | LifecyclePhase::AtPrompt
        | LifecyclePhase::Submitted
        | LifecyclePhase::Executing => LifecyclePhase::Unknown,
    }
}

/// Selects the next phase and action for a reconciled phase and lifecycle input.
///
/// This is the authoritative transition table. Its ordering encodes global safety rules before
/// phase-specific handling, so duplicate or colliding completion evidence never reaches a mutation
/// path and terminated terminals remain absorbing.
pub(super) fn plan(
    previous_phase: LifecyclePhase,
    input: LifecycleInput,
) -> (LifecyclePhase, LifecycleAction) {
    use IgnoreReason::*;
    use LifecycleAction::*;
    use LifecycleInput::*;
    use LifecyclePhase::*;
    use NextBlockIdDisposition::*;
    use PreexecObservation::*;

    match input {
        // Terminal-wide inputs either begin a new shell epoch or enter the absorbing terminal
        // phase.
        Exit if previous_phase != Terminated => (Terminated, Terminate),
        Exit => (Terminated, Ignore(IgnoredTerminated)),
        InitShell if previous_phase != Terminated => (Submitted, BeginEpoch),
        InitShell => (Terminated, Ignore(IgnoredTerminated)),
        // Start and `Preexec` inputs advance ordinary command execution while repeated or
        // impossible starts are ignored.
        StartCommand(_) => match previous_phase {
            AwaitingPrecmd | AtPrompt | Unknown => (Submitted, StartActiveBlock),
            Submitted => (Submitted, Ignore(CoalescedStart)),
            Executing => (Executing, Ignore(RejectedExecuting)),
            Terminated => (Terminated, Ignore(IgnoredTerminated)),
        },
        Preexec(observation) => match previous_phase {
            AwaitingPrecmd | AtPrompt | Submitted | Unknown => (Executing, ApplyPreexec),
            Executing => match observation {
                First | RepeatedSameCommand => (Executing, Ignore(RepeatedPreexec)),
                RepeatedDifferentCommand => (Executing, Ignore(RepeatedPreexecDifferentCommand)),
            },
            Terminated => (Terminated, Ignore(IgnoredTerminated)),
        },
        // `CommandFinished` first applies global next-block-ID safety, then accepts only phases
        // where the completion can safely advance the active block.
        CommandFinished(ActiveDuplicate) => (previous_phase, Ignore(DuplicateCompletion)),
        CommandFinished(ExistingCollision) => (previous_phase, Ignore(CollidingCompletion)),
        CommandFinished(Novel) => match previous_phase {
            AwaitingPrecmd | AtPrompt | Submitted | Executing | Unknown => {
                (AwaitingPrecmd, AcceptCommandFinished)
            }
            Terminated => (Terminated, Ignore(IgnoredTerminated)),
        },
        // A `Precmd` with completion metadata can apply a prompt after an already accepted
        // completion. A novel next-block ID selects recovery, which the coordinator feature-gates
        // before application.
        PrecmdWithCompletionMetadata(ExistingCollision) => {
            (previous_phase, Ignore(CollidingCompletion))
        }
        PrecmdWithCompletionMetadata(Novel) => match previous_phase {
            Terminated => (Terminated, Ignore(IgnoredTerminated)),
            AwaitingPrecmd | AtPrompt | Submitted | Executing | Unknown => {
                (AtPrompt, ReconcileCompletionThenApplyPrecmd)
            }
        },
        PrecmdWithCompletionMetadata(ActiveDuplicate) => match previous_phase {
            AwaitingPrecmd => (AtPrompt, ApplyPrecmd),
            AtPrompt => (AtPrompt, Ignore(RepeatedPrecmd)),
            Submitted | Executing | Unknown => (previous_phase, Ignore(RecoveryDisabled)),
            Terminated => (Terminated, Ignore(IgnoredTerminated)),
        },
        // A prompt-only `Precmd` may apply prompt metadata after a proven completion, but it
        // cannot prove completion or advance the block list by itself.
        PromptOnlyPrecmd => match previous_phase {
            AwaitingPrecmd => (AtPrompt, ApplyPrecmd),
            AtPrompt => (AtPrompt, Ignore(RepeatedPrecmd)),
            Submitted | Executing | Unknown => {
                (previous_phase, Ignore(UnsupportedPromptOnlyPrecmd))
            }
            Terminated => (Terminated, Ignore(IgnoredTerminated)),
        },
    }
}
