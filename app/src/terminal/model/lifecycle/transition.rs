use super::super::block::BlockState;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::terminal) enum LifecyclePhase {
    AwaitingPrecmd,
    AtPrompt,
    Submitted,
    Executing,
    Unknown,
    Terminated,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::terminal) enum CommandStartKind {
    UserOrQueued,
    SharedSession,
    InBand,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::terminal) enum NextBlockIdDisposition {
    Novel,
    ActiveDuplicate,
    ExistingCollision,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::terminal) enum PreexecObservation {
    First,
    RepeatedSameCommand,
    RepeatedDifferentCommand,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::terminal) enum LifecycleInput {
    StartCommand(CommandStartKind),
    Preexec(PreexecObservation),
    CommandFinished(NextBlockIdDisposition),
    PrecmdWithCompletionMetadata(NextBlockIdDisposition),
    PromptOnlyPrecmd,
    InitShell,
    Exit,
}

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
}

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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[allow(dead_code)]
pub(in crate::terminal) enum LifecycleAction {
    StartActiveBlock,
    ApplyPreexec,
    AcceptCommandFinished,
    ReconcileCompletionThenApplyPrecmd,
    ApplyPrecmd,
    RefreshPrecmd,
    BeginEpoch,
    Terminate,
    Ignore(IgnoreReason),
}

impl LifecycleAction {
    pub(super) fn is_ignored(self) -> bool {
        matches!(self, LifecycleAction::Ignore(_))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::terminal) struct LifecycleTransition {
    pub previous_phase: LifecyclePhase,
    pub next_phase: LifecyclePhase,
    pub action: LifecycleAction,
    pub recovery_record: Option<super::LifecycleRecoveryRecord>,
}

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

pub(super) fn plan(
    previous_phase: LifecyclePhase,
    input: LifecycleInput,
    snapshot: &LifecycleSnapshot,
) -> (LifecyclePhase, LifecycleAction) {
    use IgnoreReason::*;
    use LifecycleAction::*;
    use LifecycleInput::*;
    use LifecyclePhase::*;
    use NextBlockIdDisposition::*;
    use PreexecObservation::*;

    match input {
        Exit if previous_phase != Terminated => (Terminated, Terminate),
        Exit => (Terminated, Ignore(IgnoredTerminated)),
        InitShell if previous_phase != Terminated => (Submitted, BeginEpoch),
        InitShell => (Terminated, Ignore(IgnoredTerminated)),
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
        CommandFinished(ActiveDuplicate) => (previous_phase, Ignore(DuplicateCompletion)),
        CommandFinished(ExistingCollision) => (previous_phase, Ignore(CollidingCompletion)),
        CommandFinished(Novel) => match previous_phase {
            AtPrompt if !snapshot.is_bootstrap_done => (AwaitingPrecmd, AcceptCommandFinished),
            Submitted | Executing => (AwaitingPrecmd, AcceptCommandFinished),
            AwaitingPrecmd | AtPrompt | Unknown => (previous_phase, Ignore(RecoveryDisabled)),
            Terminated => (Terminated, Ignore(IgnoredTerminated)),
        },
        PrecmdWithCompletionMetadata(ExistingCollision) => {
            (previous_phase, Ignore(CollidingCompletion))
        }
        PrecmdWithCompletionMetadata(Novel) => match previous_phase {
            Terminated => (Terminated, Ignore(IgnoredTerminated)),
            AwaitingPrecmd | AtPrompt | Submitted | Executing | Unknown => {
                (previous_phase, Ignore(RecoveryDisabled))
            }
        },
        PrecmdWithCompletionMetadata(ActiveDuplicate) => match previous_phase {
            AwaitingPrecmd => (AtPrompt, ApplyPrecmd),
            AtPrompt => (AtPrompt, Ignore(RepeatedPrecmd)),
            Submitted | Executing | Unknown => (previous_phase, Ignore(RecoveryDisabled)),
            Terminated => (Terminated, Ignore(IgnoredTerminated)),
        },
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
