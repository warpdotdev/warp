use std::collections::BTreeSet;

use instant::Instant;
use warp_core::features::FeatureFlag;
use warp_core::telemetry::TelemetryEvent;

use super::telemetry::{
    LifecycleRecoveryRecord, LifecycleTelemetryEvent, LifecycleTelemetryLimiter,
};
use super::transition::{
    plan, reconcile_phase, IgnoreReason, LifecycleAction, LifecycleInput, LifecycleInputKind,
    LifecyclePhase, LifecycleSnapshot, NextBlockIdDisposition,
};
use crate::terminal::model::block::BlockState;

#[test]
fn transition_matrix_preserves_normal_flow_and_plans_safe_completion_recovery() {
    use LifecycleAction::*;
    use LifecycleInput::*;
    use LifecyclePhase::*;
    use NextBlockIdDisposition::*;

    let cases = [
        (
            AwaitingPrecmd,
            StartCommand(super::CommandStartKind::UserOrQueued),
            Submitted,
            StartActiveBlock,
        ),
        (
            AtPrompt,
            StartCommand(super::CommandStartKind::UserOrQueued),
            Submitted,
            StartActiveBlock,
        ),
        (
            Submitted,
            StartCommand(super::CommandStartKind::UserOrQueued),
            Submitted,
            Ignore(IgnoreReason::CoalescedStart),
        ),
        (
            Executing,
            StartCommand(super::CommandStartKind::UserOrQueued),
            Executing,
            Ignore(IgnoreReason::RejectedExecuting),
        ),
        (
            Unknown,
            StartCommand(super::CommandStartKind::UserOrQueued),
            Submitted,
            StartActiveBlock,
        ),
        (
            Terminated,
            StartCommand(super::CommandStartKind::UserOrQueued),
            Terminated,
            Ignore(IgnoreReason::IgnoredTerminated),
        ),
        (
            AwaitingPrecmd,
            Preexec(super::PreexecObservation::First),
            Executing,
            ApplyPreexec,
        ),
        (
            AtPrompt,
            Preexec(super::PreexecObservation::First),
            Executing,
            ApplyPreexec,
        ),
        (
            Submitted,
            Preexec(super::PreexecObservation::First),
            Executing,
            ApplyPreexec,
        ),
        (
            Executing,
            Preexec(super::PreexecObservation::First),
            Executing,
            Ignore(IgnoreReason::RepeatedPreexec),
        ),
        (
            Unknown,
            Preexec(super::PreexecObservation::First),
            Executing,
            ApplyPreexec,
        ),
        (
            Terminated,
            Preexec(super::PreexecObservation::First),
            Terminated,
            Ignore(IgnoreReason::IgnoredTerminated),
        ),
        (
            AwaitingPrecmd,
            CommandFinished(Novel),
            AwaitingPrecmd,
            AcceptCommandFinished,
        ),
        (
            AtPrompt,
            CommandFinished(Novel),
            AwaitingPrecmd,
            AcceptCommandFinished,
        ),
        (
            Submitted,
            CommandFinished(Novel),
            AwaitingPrecmd,
            AcceptCommandFinished,
        ),
        (
            Executing,
            CommandFinished(Novel),
            AwaitingPrecmd,
            AcceptCommandFinished,
        ),
        (
            Unknown,
            CommandFinished(Novel),
            AwaitingPrecmd,
            AcceptCommandFinished,
        ),
        (
            Terminated,
            CommandFinished(Novel),
            Terminated,
            Ignore(IgnoreReason::IgnoredTerminated),
        ),
        (
            AwaitingPrecmd,
            PrecmdWithCompletionMetadata(ActiveDuplicate),
            AtPrompt,
            ApplyPrecmd,
        ),
        (
            AtPrompt,
            PrecmdWithCompletionMetadata(ActiveDuplicate),
            AtPrompt,
            Ignore(IgnoreReason::RepeatedPrecmd),
        ),
        (
            Submitted,
            PrecmdWithCompletionMetadata(ActiveDuplicate),
            Submitted,
            Ignore(IgnoreReason::RecoveryDisabled),
        ),
        (
            Executing,
            PrecmdWithCompletionMetadata(ActiveDuplicate),
            Executing,
            Ignore(IgnoreReason::RecoveryDisabled),
        ),
        (
            Unknown,
            PrecmdWithCompletionMetadata(ActiveDuplicate),
            Unknown,
            Ignore(IgnoreReason::RecoveryDisabled),
        ),
        (
            Terminated,
            PrecmdWithCompletionMetadata(ActiveDuplicate),
            Terminated,
            Ignore(IgnoreReason::IgnoredTerminated),
        ),
        (AwaitingPrecmd, PromptOnlyPrecmd, AtPrompt, ApplyPrecmd),
        (
            AtPrompt,
            PromptOnlyPrecmd,
            AtPrompt,
            Ignore(IgnoreReason::RepeatedPrecmd),
        ),
        (
            Submitted,
            PromptOnlyPrecmd,
            Submitted,
            Ignore(IgnoreReason::UnsupportedPromptOnlyPrecmd),
        ),
        (
            Executing,
            PromptOnlyPrecmd,
            Executing,
            Ignore(IgnoreReason::UnsupportedPromptOnlyPrecmd),
        ),
        (
            Unknown,
            PromptOnlyPrecmd,
            Unknown,
            Ignore(IgnoreReason::UnsupportedPromptOnlyPrecmd),
        ),
        (
            Terminated,
            PromptOnlyPrecmd,
            Terminated,
            Ignore(IgnoreReason::IgnoredTerminated),
        ),
        (AwaitingPrecmd, InitShell, Submitted, BeginEpoch),
        (AtPrompt, InitShell, Submitted, BeginEpoch),
        (Submitted, InitShell, Submitted, BeginEpoch),
        (Executing, InitShell, Submitted, BeginEpoch),
        (Unknown, InitShell, Submitted, BeginEpoch),
        (
            Terminated,
            InitShell,
            Terminated,
            Ignore(IgnoreReason::IgnoredTerminated),
        ),
        (AwaitingPrecmd, Exit, Terminated, Terminate),
        (AtPrompt, Exit, Terminated, Terminate),
        (Submitted, Exit, Terminated, Terminate),
        (Executing, Exit, Terminated, Terminate),
        (Unknown, Exit, Terminated, Terminate),
        (
            Terminated,
            Exit,
            Terminated,
            Ignore(IgnoreReason::IgnoredTerminated),
        ),
    ];

    for (phase, input, expected_phase, expected_action) in cases {
        assert_eq!(plan(phase, input), (expected_phase, expected_action));
    }

    assert_eq!(
        plan(AtPrompt, CommandFinished(Novel)),
        (AwaitingPrecmd, AcceptCommandFinished)
    );
    assert_eq!(
        plan(
            Executing,
            Preexec(super::PreexecObservation::RepeatedDifferentCommand),
        ),
        (
            Executing,
            Ignore(IgnoreReason::RepeatedPreexecDifferentCommand),
        )
    );
    for phase in [
        AwaitingPrecmd,
        AtPrompt,
        Submitted,
        Executing,
        Unknown,
        Terminated,
    ] {
        assert_eq!(
            plan(phase, CommandFinished(ActiveDuplicate)),
            (phase, Ignore(IgnoreReason::DuplicateCompletion))
        );
        assert_eq!(
            plan(phase, CommandFinished(ExistingCollision)),
            (phase, Ignore(IgnoreReason::CollidingCompletion))
        );
        let expected_novel_precmd = match phase {
            Terminated => (Terminated, Ignore(IgnoreReason::IgnoredTerminated)),
            AwaitingPrecmd | AtPrompt | Submitted | Executing | Unknown => {
                (AtPrompt, ReconcileCompletionThenApplyPrecmd)
            }
        };
        assert_eq!(
            plan(phase, PrecmdWithCompletionMetadata(Novel)),
            expected_novel_precmd
        );
    }
    for phase in [
        AwaitingPrecmd,
        AtPrompt,
        Submitted,
        Executing,
        Unknown,
        Terminated,
    ] {
        let expected = plan(phase, StartCommand(super::CommandStartKind::UserOrQueued));
        for kind in [
            super::CommandStartKind::SharedSession,
            super::CommandStartKind::InBand,
        ] {
            assert_eq!(plan(phase, StartCommand(kind)), expected);
        }
        if phase != Executing {
            let expected = plan(phase, Preexec(super::PreexecObservation::First));
            for observation in [
                super::PreexecObservation::RepeatedSameCommand,
                super::PreexecObservation::RepeatedDifferentCommand,
            ] {
                assert_eq!(plan(phase, Preexec(observation)), expected);
            }
        }
        assert_eq!(
            plan(phase, PrecmdWithCompletionMetadata(ExistingCollision)),
            (phase, Ignore(IgnoreReason::CollidingCompletion))
        );
    }
    assert_eq!(
        plan(
            Executing,
            Preexec(super::PreexecObservation::RepeatedSameCommand),
        ),
        (Executing, Ignore(IgnoreReason::RepeatedPreexec))
    );
}

#[test]
fn lifecycle_phase_reconciliation_requires_compatible_live_evidence() {
    use LifecyclePhase::*;

    let before_execution = LifecycleSnapshot {
        active_block_id: "active".to_owned(),
        active_session_id: Some(1),
        supplied_next_block_id: None,
        hook_session_id: None,
        block_state: BlockState::BeforeExecution,
        started: false,
        finished: false,
        received_precmd: false,
        is_in_band: false,
        is_bootstrapped: true,
        is_bootstrap_done: true,
        is_alt_screen_active: false,
        completion_mismatch: false,
    };
    assert_eq!(
        reconcile_phase(AwaitingPrecmd, &before_execution),
        AwaitingPrecmd
    );
    assert_eq!(
        reconcile_phase(
            AtPrompt,
            &LifecycleSnapshot {
                received_precmd: true,
                ..before_execution.clone()
            }
        ),
        AtPrompt
    );
    assert_eq!(
        reconcile_phase(
            Submitted,
            &LifecycleSnapshot {
                started: true,
                ..before_execution.clone()
            }
        ),
        Submitted
    );
    assert_eq!(
        reconcile_phase(
            Executing,
            &LifecycleSnapshot {
                block_state: BlockState::Executing,
                started: true,
                ..before_execution.clone()
            }
        ),
        Executing
    );
    assert_eq!(reconcile_phase(AtPrompt, &before_execution), Unknown);
    assert_eq!(reconcile_phase(Unknown, &before_execution), Unknown);
    assert_eq!(reconcile_phase(Terminated, &before_execution), Terminated);
}

#[test]
fn lifecycle_coordinator_records_only_conservative_or_recovery_transitions() {
    use LifecycleAction::*;
    use LifecycleInput::*;

    let before_execution = LifecycleSnapshot {
        active_block_id: "active".to_owned(),
        active_session_id: Some(1),
        supplied_next_block_id: None,
        hook_session_id: None,
        block_state: BlockState::BeforeExecution,
        started: false,
        finished: false,
        received_precmd: false,
        is_in_band: false,
        is_bootstrapped: true,
        is_bootstrap_done: true,
        is_alt_screen_active: false,
        completion_mismatch: false,
    };
    let mut coordinator = super::BlockLifecycleCoordinator::default();

    let transition = coordinator.plan(&before_execution, InitShell);
    assert_eq!(transition.action, BeginEpoch);
    assert!(transition.recovery_record.is_none());
    coordinator.commit(&transition);

    let submitted = LifecycleSnapshot {
        started: true,
        ..before_execution.clone()
    };
    let transition = coordinator.plan(&submitted, Preexec(super::PreexecObservation::First));
    assert_eq!(transition.action, ApplyPreexec);
    assert!(transition.recovery_record.is_none());
    coordinator.commit(&transition);

    let executing = LifecycleSnapshot {
        block_state: BlockState::Executing,
        started: true,
        ..before_execution.clone()
    };
    let transition = coordinator.plan(&executing, CommandFinished(NextBlockIdDisposition::Novel));
    assert_eq!(transition.action, AcceptCommandFinished);
    assert!(transition.recovery_record.is_none());
    coordinator.commit(&transition);

    let transition = coordinator.plan(
        &before_execution,
        PrecmdWithCompletionMetadata(NextBlockIdDisposition::ActiveDuplicate),
    );
    assert_eq!(transition.action, ApplyPrecmd);
    assert!(transition.recovery_record.is_none());
    coordinator.commit(&transition);

    let at_prompt = LifecycleSnapshot {
        received_precmd: true,
        ..before_execution
    };
    let transition = coordinator.plan(
        &at_prompt,
        PrecmdWithCompletionMetadata(NextBlockIdDisposition::ActiveDuplicate),
    );
    assert_eq!(transition.action, Ignore(IgnoreReason::RepeatedPrecmd));
    assert!(transition.recovery_record.is_some());
}

#[test]
fn lifecycle_coordinator_gates_novel_completion_recovery() {
    let _recovery_disabled = FeatureFlag::TerminalLifecycleRecovery.override_enabled(false);
    let snapshot = LifecycleSnapshot {
        active_block_id: "active".to_owned(),
        active_session_id: Some(1),
        supplied_next_block_id: Some("next".to_owned()),
        hook_session_id: Some(1),
        block_state: BlockState::BeforeExecution,
        started: false,
        finished: false,
        received_precmd: true,
        is_in_band: false,
        is_bootstrapped: true,
        is_bootstrap_done: true,
        is_alt_screen_active: false,
        completion_mismatch: false,
    };
    let mut coordinator = super::BlockLifecycleCoordinator {
        phase: LifecyclePhase::AtPrompt,
        ..Default::default()
    };

    for input in [
        LifecycleInput::CommandFinished(NextBlockIdDisposition::Novel),
        LifecycleInput::PrecmdWithCompletionMetadata(NextBlockIdDisposition::Novel),
    ] {
        let transition = coordinator.plan(&snapshot, input);
        assert_eq!(
            transition.action,
            LifecycleAction::Ignore(IgnoreReason::RecoveryDisabled)
        );
        assert_eq!(transition.next_phase, LifecyclePhase::AtPrompt);
        assert!(transition.recovery_record.is_some());
    }
}

#[test]
fn lifecycle_coordinator_accepts_novel_completion_recovery_when_enabled() {
    let _recovery_enabled = FeatureFlag::TerminalLifecycleRecovery.override_enabled(true);
    let snapshot = LifecycleSnapshot {
        active_block_id: "active".to_owned(),
        active_session_id: Some(1),
        supplied_next_block_id: Some("next".to_owned()),
        hook_session_id: Some(1),
        block_state: BlockState::BeforeExecution,
        started: false,
        finished: false,
        received_precmd: true,
        is_in_band: false,
        is_bootstrapped: true,
        is_bootstrap_done: true,
        is_alt_screen_active: false,
        completion_mismatch: false,
    };
    let mut coordinator = super::BlockLifecycleCoordinator {
        phase: LifecyclePhase::AtPrompt,
        ..Default::default()
    };

    let command_finished = coordinator.plan(
        &snapshot,
        LifecycleInput::CommandFinished(NextBlockIdDisposition::Novel),
    );
    assert_eq!(
        command_finished.action,
        LifecycleAction::AcceptCommandFinished
    );
    assert_eq!(command_finished.next_phase, LifecyclePhase::AwaitingPrecmd);
    assert!(command_finished.recovery_record.is_some());

    let precmd_with_completion_metadata = coordinator.plan(
        &snapshot,
        LifecycleInput::PrecmdWithCompletionMetadata(NextBlockIdDisposition::Novel),
    );
    assert_eq!(
        precmd_with_completion_metadata.action,
        LifecycleAction::ReconcileCompletionThenApplyPrecmd
    );
    assert_eq!(
        precmd_with_completion_metadata.next_phase,
        LifecyclePhase::AtPrompt
    );
    assert!(precmd_with_completion_metadata.recovery_record.is_some());
}

#[test]
fn lifecycle_telemetry_is_rate_limited_per_transition_key() {
    let mut limiter = LifecycleTelemetryLimiter::default();
    let now = Instant::now();
    let record = LifecycleRecoveryRecord::new(
        LifecyclePhase::Executing,
        LifecyclePhase::Executing,
        LifecycleInputKind::StartCommand,
        LifecycleAction::Ignore(IgnoreReason::RejectedExecuting),
        &LifecycleSnapshot {
            active_block_id: "active".to_owned(),
            active_session_id: Some(1),
            supplied_next_block_id: None,
            hook_session_id: None,
            block_state: BlockState::Executing,
            started: true,
            finished: false,
            received_precmd: true,
            is_in_band: false,
            is_bootstrapped: true,
            is_bootstrap_done: true,
            is_alt_screen_active: false,
            completion_mismatch: false,
        },
    );

    assert!(limiter.record_at(record.clone(), now).is_some());
    assert!(limiter
        .record_at(record.clone(), now + std::time::Duration::from_secs(1))
        .is_none());
    let emitted = limiter
        .record_at(record, now + std::time::Duration::from_secs(61))
        .expect("The rate-limit interval should emit an aggregate.");
    assert_eq!(emitted.suppressed_repeats, 1);
}

#[test]
fn lifecycle_telemetry_payload_is_allowlisted_and_non_ugc() {
    let record = LifecycleRecoveryRecord::new(
        LifecyclePhase::Executing,
        LifecyclePhase::Executing,
        LifecycleInputKind::StartCommand,
        LifecycleAction::Ignore(IgnoreReason::RejectedExecuting),
        &LifecycleSnapshot {
            active_block_id: "active".to_owned(),
            active_session_id: Some(1),
            supplied_next_block_id: Some("next".to_owned()),
            hook_session_id: Some(2),
            block_state: BlockState::Executing,
            started: true,
            finished: false,
            received_precmd: true,
            is_in_band: false,
            is_bootstrapped: true,
            is_bootstrap_done: true,
            is_alt_screen_active: false,
            completion_mismatch: false,
        },
    );
    let event = LifecycleTelemetryEvent::Recovery(record);
    assert!(!event.contains_ugc());
    let payload = event
        .payload()
        .expect("Lifecycle telemetry should have a payload.");
    let fields = payload
        .as_object()
        .expect("Lifecycle telemetry should be a JSON object.")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        fields,
        BTreeSet::from([
            "action",
            "active_block_id",
            "active_session_id",
            "block_state",
            "completion_mismatch",
            "finished",
            "hook_session_id",
            "input_kind",
            "is_alt_screen_active",
            "is_bootstrap_done",
            "is_bootstrapped",
            "is_in_band",
            "next_phase",
            "previous_phase",
            "received_precmd",
            "started",
            "supplied_next_block_id",
            "suppressed_repeats",
        ])
    );
}
