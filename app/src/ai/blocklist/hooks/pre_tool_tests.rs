use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use ai::hooks::test_util::{ScriptedRunner, test_binding, test_binding_id, test_source_id};
use ai::hooks::{HookAsker, HookBoundary, HookNoticeKind, HookOnAsk, HookReason, HookVerdict};
use futures::executor::block_on;
use warp_core::execution_mode::ExecutionMode;
use warpui::App;

use super::*;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::hooks::test_utils::{
    HookTestApp, add_child_conversation, hook_test_app, install_test_hooks, set_working_directory,
    shell_action,
};
use crate::test_util::settings::initialize_settings_for_tests_with_mode;

fn install_shell_deny(harness: &HookTestApp, app: &mut App, reason: &str) {
    let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, Some("Shell"));
    harness
        .hooks
        .runner
        .script(binding.id.clone(), ScriptedRunner::deny_with(reason));
    app.update(|ctx| harness.hooks.install_bindings(vec![binding], ctx));
}

fn lookup(
    app: &mut App,
    harness: &HookTestApp,
    action: &AIAgentAction,
    conversation_id: AIConversationId,
    is_user_initiated: bool,
) -> PreToolLookup {
    app.update(|ctx| {
        begin_or_lookup(
            action,
            conversation_id,
            is_user_initiated,
            &harness.active_session,
            ctx,
        )
    })
}

fn expect_dispatch(lookup: PreToolLookup) -> PendingDispatch {
    let PreToolLookup::Pending {
        dispatch: Some(dispatch),
    } = lookup
    else {
        panic!("expected the first lookup of an action to hand back a dispatch");
    };
    dispatch
}

#[test]
fn lookup_is_inactive_before_the_registry_is_finalized() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, Some("Shell"));
        harness
            .hooks
            .runner
            .script(binding.id.clone(), ScriptedRunner::deny_with("no"));
        app.update(|ctx| harness.hooks.stage_bindings(vec![binding], ctx));
        let action = shell_action("a1", "ls");
        let result = lookup(&mut app, &harness, &action, harness.conversation_id, false);
        assert!(matches!(result, PreToolLookup::Inactive));
        assert_eq!(harness.hooks.adapter.encoded_count(), 0);
        assert!(
            app.read(|ctx| cached_state(harness.conversation_id, &action.id, ctx))
                .is_none()
        );
    });
}

#[test]
fn lookup_is_inactive_when_no_binding_matches_the_action() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        let binding = test_binding(test_source_id("s"), HookBoundary::ToolPre, 0, Some("Read"));
        app.update(|ctx| harness.hooks.install_bindings(vec![binding], ctx));
        let action = shell_action("a1", "ls");
        let result = lookup(&mut app, &harness, &action, harness.conversation_id, false);
        assert!(matches!(result, PreToolLookup::Inactive));
        assert!(
            app.read(|ctx| cached_state(harness.conversation_id, &action.id, ctx))
                .is_none()
        );
    });
}

#[test]
fn lookup_skips_the_envelope_when_no_pre_tool_binding_exists() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        let binding = test_binding(test_source_id("s"), HookBoundary::ToolResult, 0, None);
        app.update(|ctx| harness.hooks.install_bindings(vec![binding], ctx));
        let action = shell_action("a1", "ls");
        // Inside a history-model update, building the envelope would re-borrow the model and
        // panic, so reaching `Inactive` here proves the envelope was never built.
        let result = app.update(|ctx| {
            BlocklistAIHistoryModel::handle(ctx).update(ctx, |_, ctx| {
                begin_or_lookup(
                    &action,
                    harness.conversation_id,
                    false,
                    &harness.active_session,
                    ctx,
                )
            })
        });
        assert!(matches!(result, PreToolLookup::Inactive));
        assert_eq!(harness.hooks.adapter.encoded_count(), 0);
    });
}

#[test]
fn first_lookup_starts_a_dispatch_and_later_lookups_wait_on_it() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        install_shell_deny(&harness, &mut app, "blocked by policy");
        let action = shell_action("a1", "ls");

        let dispatch = expect_dispatch(lookup(
            &mut app,
            &harness,
            &action,
            harness.conversation_id,
            false,
        ));
        assert_eq!(dispatch.ticket.action_id, action.id);
        assert_eq!(dispatch.ticket.conversation_id, harness.conversation_id);
        assert!(matches!(
            app.read(|ctx| cached_state(harness.conversation_id, &action.id, ctx)),
            Some(PreToolHookState::Pending)
        ));

        let second = lookup(&mut app, &harness, &action, harness.conversation_id, true);
        assert!(matches!(second, PreToolLookup::Pending { dispatch: None }));

        let outcome = block_on(dispatch.future);
        assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
        let resumption = app.update(|ctx| on_resolved(&dispatch.ticket, outcome, ctx));
        assert_eq!(
            resumption,
            Some(Resumption {
                conversation_id: harness.conversation_id,
                is_user_initiated: true,
            }),
            "user approval seen while pending sticks to the resumption"
        );

        let third = lookup(&mut app, &harness, &action, harness.conversation_id, false);
        let PreToolLookup::Resolved(merged) = third else {
            panic!("expected the cached verdict");
        };
        assert_eq!(merged.verdict, HookVerdict::Deny);
        assert_eq!(merged.reasons[0].text, "blocked by policy");
        assert_eq!(
            harness.hooks.adapter.encoded_count(),
            1,
            "one dispatch only"
        );
    });
}

#[test]
fn invalidation_stops_the_dispatch_and_drops_a_late_verdict() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        install_shell_deny(&harness, &mut app, "no");
        let action = shell_action("a1", "ls");
        let dispatch = expect_dispatch(lookup(
            &mut app,
            &harness,
            &action,
            harness.conversation_id,
            false,
        ));
        let cancel = app
            .read(|ctx| cancel_flag(harness.conversation_id, &action.id, ctx))
            .expect("pending entry has a cancel flag");

        app.update(|ctx| invalidate(harness.conversation_id, &action.id, ctx));
        assert!(cancel.load(Ordering::Acquire));
        assert!(
            app.read(|ctx| cached_state(harness.conversation_id, &action.id, ctx))
                .is_none()
        );

        let outcome = block_on(dispatch.future);
        assert!(
            app.update(|ctx| on_resolved(&dispatch.ticket, outcome, ctx))
                .is_none()
        );
        assert!(
            app.read(|ctx| cached_state(harness.conversation_id, &action.id, ctx))
                .is_none()
        );
    });
}

#[test]
fn a_replaced_dispatch_cannot_resolve_the_entry_that_replaced_it() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        install_shell_deny(&harness, &mut app, "no");
        let hold = harness.hooks.runner.hold();
        let action = shell_action("a1", "ls");

        let first = expect_dispatch(lookup(
            &mut app,
            &harness,
            &action,
            harness.conversation_id,
            false,
        ));
        app.update(|ctx| invalidate(harness.conversation_id, &action.id, ctx));
        let second = expect_dispatch(lookup(
            &mut app,
            &harness,
            &action,
            harness.conversation_id,
            false,
        ));
        hold.release();

        let first_outcome = block_on(first.future);
        assert!(
            app.update(|ctx| on_resolved(&first.ticket, first_outcome, ctx))
                .is_none(),
            "the first dispatch's verdict must not land on the second entry"
        );
        assert!(matches!(
            app.read(|ctx| cached_state(harness.conversation_id, &action.id, ctx)),
            Some(PreToolHookState::Pending)
        ));

        let second_outcome = block_on(second.future);
        assert_eq!(
            app.update(|ctx| on_resolved(&second.ticket, second_outcome, ctx)),
            Some(Resumption {
                conversation_id: harness.conversation_id,
                is_user_initiated: false,
            })
        );
        assert!(matches!(
            app.read(|ctx| cached_state(harness.conversation_id, &action.id, ctx)),
            Some(PreToolHookState::Resolved(_))
        ));
    });
}

#[test]
fn the_same_action_id_in_two_conversations_gets_two_entries() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        install_shell_deny(&harness, &mut app, "no");
        let hold = harness.hooks.runner.hold();
        let other_conversation = AIConversationId::new();
        let mine = shell_action("call_1", "ls");
        let theirs = shell_action("call_1", "rm -rf x");

        let my_dispatch = expect_dispatch(lookup(
            &mut app,
            &harness,
            &mine,
            harness.conversation_id,
            false,
        ));
        let their_dispatch = expect_dispatch(lookup(
            &mut app,
            &harness,
            &theirs,
            other_conversation,
            false,
        ));
        hold.release();
        let my_outcome = block_on(my_dispatch.future);
        let their_outcome = block_on(their_dispatch.future);
        assert_eq!(harness.hooks.runner.invocation_count(), 2);

        // Approval in the other conversation must not leak into this one.
        let again = lookup(&mut app, &harness, &theirs, other_conversation, true);
        assert!(matches!(again, PreToolLookup::Pending { dispatch: None }));

        assert_eq!(
            app.update(|ctx| on_resolved(&my_dispatch.ticket, my_outcome, ctx)),
            Some(Resumption {
                conversation_id: harness.conversation_id,
                is_user_initiated: false,
            })
        );
        assert_eq!(
            app.update(|ctx| on_resolved(&their_dispatch.ticket, their_outcome, ctx)),
            Some(Resumption {
                conversation_id: other_conversation,
                is_user_initiated: true,
            })
        );

        app.update(|ctx| invalidate(other_conversation, &theirs.id, ctx));
        assert!(matches!(
            app.read(|ctx| cached_state(harness.conversation_id, &mine.id, ctx)),
            Some(PreToolHookState::Resolved(_))
        ));
        assert!(
            app.read(|ctx| cached_state(other_conversation, &theirs.id, ctx))
                .is_none()
        );
    });
}

#[test]
fn dropping_an_unfinished_dispatch_sets_its_cancel_flag() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        install_shell_deny(&harness, &mut app, "no");
        let action = shell_action("a1", "ls");

        let dropped = expect_dispatch(lookup(
            &mut app,
            &harness,
            &action,
            harness.conversation_id,
            false,
        ));
        let cancel = app
            .read(|ctx| cancel_flag(harness.conversation_id, &action.id, ctx))
            .expect("pending entry has a cancel flag");
        drop(dropped.future);
        assert!(cancel.load(Ordering::Acquire));

        app.update(|ctx| invalidate(harness.conversation_id, &action.id, ctx));
        let finished = expect_dispatch(lookup(
            &mut app,
            &harness,
            &action,
            harness.conversation_id,
            false,
        ));
        let cancel = app
            .read(|ctx| cancel_flag(harness.conversation_id, &action.id, ctx))
            .expect("pending entry has a cancel flag");
        drop(block_on(finished.future));
        assert!(
            !cancel.load(Ordering::Acquire),
            "a dispatch that ran to completion is not cancelled"
        );
    });
}

/// A dispatch dropped before it finished leaves an entry that nothing will resolve. The next
/// lookup notices and starts over instead of waiting forever; the approval it carried stays,
/// since the operation and the configuration are unchanged.
#[test]
fn a_lookup_after_a_dropped_dispatch_starts_over() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        install_shell_deny(&harness, &mut app, "no");
        let action = shell_action("a1", "ls");

        let dropped = expect_dispatch(lookup(
            &mut app,
            &harness,
            &action,
            harness.conversation_id,
            true,
        ));
        drop(dropped.future);
        assert!(matches!(
            app.read(|ctx| cached_state(harness.conversation_id, &action.id, ctx)),
            Some(PreToolHookState::Pending)
        ));

        let restarted = expect_dispatch(lookup(
            &mut app,
            &harness,
            &action,
            harness.conversation_id,
            false,
        ));
        assert!(
            !Arc::ptr_eq(&restarted.ticket.cancel, &dropped.ticket.cancel),
            "a fresh dispatch, not the dead one"
        );
        let outcome = block_on(restarted.future);
        assert_eq!(
            app.update(|ctx| on_resolved(&restarted.ticket, outcome, ctx)),
            Some(Resumption {
                conversation_id: harness.conversation_id,
                is_user_initiated: true,
            })
        );
        assert_eq!(
            harness.hooks.adapter.encoded_count(),
            1,
            "the dropped dispatch never ran; the restarted one did"
        );
    });
}

/// Runs one lookup to its verdict so the entry is `Resolved`.
fn resolve(app: &mut App, harness: &HookTestApp, action: &AIAgentAction) {
    let dispatch = expect_dispatch(lookup(app, harness, action, harness.conversation_id, false));
    let outcome = block_on(dispatch.future);
    assert!(
        app.update(|ctx| on_resolved(&dispatch.ticket, outcome, ctx))
            .is_some()
    );
    assert!(matches!(
        app.read(|ctx| cached_state(harness.conversation_id, &action.id, ctx)),
        Some(PreToolHookState::Resolved(_))
    ));
}

#[test]
fn a_changed_command_gets_a_new_dispatch_without_an_explicit_invalidate() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        install_shell_deny(&harness, &mut app, "no");
        resolve(&mut app, &harness, &shell_action("a1", "ls"));
        let old_cancel = app
            .read(|ctx| cancel_flag(harness.conversation_id, &shell_action("a1", "ls").id, ctx))
            .expect("entry");

        // Same action id, different text: the cached verdict describes another command.
        let edited = shell_action("a1", "ls -la");
        let dispatch = expect_dispatch(lookup(
            &mut app,
            &harness,
            &edited,
            harness.conversation_id,
            true,
        ));
        assert!(
            old_cancel.load(Ordering::Acquire),
            "the old entry is stopped"
        );
        let key = app
            .read(|ctx| cached_key(harness.conversation_id, &edited.id, ctx))
            .expect("entry");
        assert_eq!(key.argument_revision, argument_revision(&edited.action));
        assert_ne!(
            key.argument_revision,
            argument_revision(&shell_action("a1", "ls").action)
        );
        drop(block_on(dispatch.future));
        assert_eq!(harness.hooks.adapter.encoded_count(), 2);
    });
}

#[test]
fn a_changed_working_directory_gets_a_new_dispatch() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        install_shell_deny(&harness, &mut app, "no");
        let action = shell_action("a1", "ls");
        set_working_directory(&mut app, &harness, Path::new("/work/one"));
        resolve(&mut app, &harness, &action);
        assert_eq!(
            app.read(|ctx| cached_key(harness.conversation_id, &action.id, ctx))
                .and_then(|key| key.cwd),
            Some(PathBuf::from("/work/one"))
        );

        set_working_directory(&mut app, &harness, Path::new("/work/two"));
        let dispatch = expect_dispatch(lookup(
            &mut app,
            &harness,
            &action,
            harness.conversation_id,
            false,
        ));
        assert_eq!(
            app.read(|ctx| cached_key(harness.conversation_id, &action.id, ctx))
                .and_then(|key| key.cwd),
            Some(PathBuf::from("/work/two"))
        );
        drop(block_on(dispatch.future));
        assert_eq!(harness.hooks.adapter.encoded_count(), 2);

        // Back in the first directory the key is new again; nothing is kept per directory.
        set_working_directory(&mut app, &harness, Path::new("/work/one"));
        let third = lookup(&mut app, &harness, &action, harness.conversation_id, false);
        assert!(matches!(
            third,
            PreToolLookup::Pending { dispatch: Some(_) }
        ));
    });
}

/// The user approved a command, and its hook check had to start over. Approval carries over to
/// the restarted check only when the operation is the same: same arguments in the same working
/// directory.
#[test]
fn approval_carries_over_only_for_the_same_arguments_in_the_same_working_directory() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        install_shell_deny(&harness, &mut app, "no");
        set_working_directory(&mut app, &harness, Path::new("/work/one"));

        // The approved check died before answering; the same operation keeps the approval.
        let same = shell_action("a1", "ls");
        drop(expect_dispatch(lookup(
            &mut app,
            &harness,
            &same,
            harness.conversation_id,
            true,
        )));
        let restarted = expect_dispatch(lookup(
            &mut app,
            &harness,
            &same,
            harness.conversation_id,
            false,
        ));
        let outcome = block_on(restarted.future);
        assert_eq!(
            app.update(|ctx| on_resolved(&restarted.ticket, outcome, ctx)),
            Some(Resumption {
                conversation_id: harness.conversation_id,
                is_user_initiated: true,
            })
        );

        // The shell moved before the verdict arrived; the same command elsewhere is a new
        // operation and needs a new approval.
        let moved = shell_action("a2", "ls");
        let first = expect_dispatch(lookup(
            &mut app,
            &harness,
            &moved,
            harness.conversation_id,
            true,
        ));
        set_working_directory(&mut app, &harness, Path::new("/work/two"));
        let restarted = expect_dispatch(lookup(
            &mut app,
            &harness,
            &moved,
            harness.conversation_id,
            false,
        ));
        assert!(
            first.ticket.cancel.load(Ordering::Acquire),
            "the old check is stopped"
        );
        let outcome = block_on(restarted.future);
        assert_eq!(
            app.update(|ctx| on_resolved(&restarted.ticket, outcome, ctx)),
            Some(Resumption {
                conversation_id: harness.conversation_id,
                is_user_initiated: false,
            })
        );

        // The command changed before the verdict arrived; it needs a new approval too.
        let approved = shell_action("a3", "ls");
        drop(expect_dispatch(lookup(
            &mut app,
            &harness,
            &approved,
            harness.conversation_id,
            true,
        )));
        let edited = shell_action("a3", "ls -la");
        let restarted = expect_dispatch(lookup(
            &mut app,
            &harness,
            &edited,
            harness.conversation_id,
            false,
        ));
        let outcome = block_on(restarted.future);
        assert_eq!(
            app.update(|ctx| on_resolved(&restarted.ticket, outcome, ctx)),
            Some(Resumption {
                conversation_id: harness.conversation_id,
                is_user_initiated: false,
            })
        );
    });
}

fn ask_effects(reason: &str) -> MergedEffects {
    let binding = test_binding_id(test_source_id("s"), "tool.pre", 0);
    MergedEffects {
        verdict: HookVerdict::Ask,
        verdict_binding: Some(binding.clone()),
        reasons: vec![HookReason {
            text: reason.to_owned(),
            binding: Some(binding),
        }],
        ..Default::default()
    }
}

fn warp_would_run() -> WarpPermissionResult {
    WarpPermissionResult {
        can_auto_execute: true,
        is_user_initiated: false,
        is_autonomous_shell_exception: false,
    }
}

/// Every `ApprovalUnavailable` event the runtime emitted.
fn record_run_events(app: &mut App) -> Rc<RefCell<Vec<(AIAgentActionId, String)>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let log = events.clone();
    app.update(|ctx| {
        ctx.subscribe_to_model(&HookRuntime::handle(ctx), move |_, event, _| {
            let HookRuntimeEvent::ApprovalUnavailable {
                action_id, reason, ..
            } = event;
            log.borrow_mut().push((action_id.clone(), reason.clone()));
        });
    });
    events
}

#[test]
fn an_ask_on_an_interactive_surface_requires_confirmation() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        let action = shell_action("a1", "ls");
        let decision = app.update(|ctx| {
            decide(
                &ask_effects("check first"),
                warp_would_run(),
                PermissionFloors::default(),
                &action,
                harness.conversation_id,
                ctx,
            )
        });
        let PermissionDecision::RequireConfirmation {
            reason: Some(reason),
        } = decision
        else {
            panic!("expected a confirmation step, got {decision:?}");
        };
        assert_eq!(reason.text, "check first");
        let notices = harness.hooks.messenger.notices();
        assert_eq!(notices.len(), 1, "the user is told why the card appeared");
        assert_eq!(notices[0].kind, HookNoticeKind::Warning);
        assert_eq!(notices[0].text, "check first");
        assert_eq!(notices[0].action_id, Some(action.id.clone()));
    });
}

/// Where someone can answer, a hook that asked from a `fail_run` source still gets a
/// confirmation step, and nothing tells the run to end.
#[test]
fn a_fail_run_asker_with_someone_to_answer_requires_confirmation_and_ends_nothing() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        let events = record_run_events(&mut app);
        let action = shell_action("a1", "ls");
        let mut ask = ask_effects("check first");
        ask.askers = vec![HookAsker {
            binding: test_binding_id(test_source_id("s"), "tool.pre", 0),
            on_ask: HookOnAsk::FailRun,
        }];
        let decision = app.update(|ctx| {
            decide(
                &ask,
                warp_would_run(),
                PermissionFloors::default(),
                &action,
                harness.conversation_id,
                ctx,
            )
        });
        assert!(
            matches!(
                decision,
                PermissionDecision::RequireConfirmation { reason: Some(_) }
            ),
            "{decision:?}"
        );
        assert!(events.borrow().is_empty());
    });
}

/// A waiting action is tried more than once; the user hears about the hook's `ask` once.
#[test]
fn the_ask_notice_goes_out_once_per_verdict() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        install_shell_deny(&harness, &mut app, "unused");
        let action = shell_action("a1", "ls");
        resolve(&mut app, &harness, &action);

        for _ in 0..3 {
            let decision = app.update(|ctx| {
                decide(
                    &ask_effects("check first"),
                    warp_would_run(),
                    PermissionFloors::default(),
                    &action,
                    harness.conversation_id,
                    ctx,
                )
            });
            assert!(matches!(
                decision,
                PermissionDecision::RequireConfirmation { .. }
            ));
        }
        let notices = harness.hooks.messenger.notices();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].kind, HookNoticeKind::Warning);
    });
}

#[test]
fn an_ask_for_a_local_child_requires_confirmation() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        let child = add_child_conversation(&mut app, &harness);
        let events = record_run_events(&mut app);
        let action = shell_action("a1", "ls");
        let decision = app.update(|ctx| {
            decide(
                &ask_effects("check first"),
                warp_would_run(),
                PermissionFloors::default(),
                &action,
                child,
                ctx,
            )
        });
        let PermissionDecision::RequireConfirmation {
            reason: Some(reason),
        } = decision
        else {
            panic!("expected a confirmation step, got {decision:?}");
        };
        assert_eq!(reason.text, "check first");

        let notices = harness.hooks.messenger.notices();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].kind, HookNoticeKind::Warning);
        assert_eq!(notices[0].conversation_id, child);
        assert_eq!(notices[0].text, reason.text);
        assert!(
            events.borrow().is_empty(),
            "requiring confirmation does not announce anything to the run"
        );
    });
}

#[test]
fn an_asker_set_to_fail_the_run_announces_an_unanswerable_ask_and_still_denies() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests_with_mode(&mut app, ExecutionMode::Sdk, false);
        let hooks = app.update(install_test_hooks);
        let conversation_id = AIConversationId::new();
        let events = record_run_events(&mut app);
        let action = shell_action("a1", "ls");
        let mut ask = ask_effects("check first");
        ask.askers = vec![HookAsker {
            binding: test_binding_id(test_source_id("s"), "tool.pre", 0),
            on_ask: HookOnAsk::FailRun,
        }];
        let decision = app.update(|ctx| {
            decide(
                &ask,
                warp_would_run(),
                PermissionFloors::default(),
                &action,
                conversation_id,
                ctx,
            )
        });
        let PermissionDecision::Deny { reason, cause } = decision else {
            panic!("expected a denial, got {decision:?}");
        };
        assert_eq!(cause, DenyCause::ApprovalUnavailable);
        assert_eq!(
            events.borrow().as_slice(),
            &[(action.id.clone(), reason.text.clone())]
        );
        let notices = hooks.messenger.notices();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].kind, HookNoticeKind::Denied);

        // An outright deny outranks the ask, so the run is not told to end even though a hook
        // that asked wanted that.
        let mut deny = ask.clone();
        deny.verdict = HookVerdict::Deny;
        app.update(|ctx| {
            decide(
                &deny,
                warp_would_run(),
                PermissionFloors::default(),
                &action,
                conversation_id,
                ctx,
            )
        });
        assert_eq!(events.borrow().len(), 1);
    });
}

#[test]
fn cancelling_a_conversation_clears_only_its_entries() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        install_shell_deny(&harness, &mut app, "no");
        let mine = shell_action("mine", "ls");
        let other_conversation = AIConversationId::new();
        let theirs = shell_action("theirs", "ls");
        for (action, conversation_id) in [
            (&mine, harness.conversation_id),
            (&theirs, other_conversation),
        ] {
            let result = lookup(&mut app, &harness, action, conversation_id, false);
            assert!(matches!(result, PreToolLookup::Pending { .. }));
        }
        let mine_cancel = app
            .read(|ctx| cancel_flag(harness.conversation_id, &mine.id, ctx))
            .expect("pending");

        app.update(|ctx| cancel_all_for_conversation(harness.conversation_id, ctx));

        assert!(mine_cancel.load(Ordering::Acquire));
        assert!(
            app.read(|ctx| cached_state(harness.conversation_id, &mine.id, ctx))
                .is_none()
        );
        assert!(matches!(
            app.read(|ctx| cached_state(other_conversation, &theirs.id, ctx)),
            Some(PreToolHookState::Pending)
        ));
    });
}

/// The protected-path floor is read against resolved paths, the way the file-edit executor
/// checks them, so an edit the model writes relative to the session's working directory
/// carries the floor when it touches a protected file.
#[cfg(feature = "local_fs")]
mod floors {
    use std::path::PathBuf;

    use ai::agent::action::FileEdit;
    use ai::hooks::PermissionFloors;
    use tempfile::TempDir;
    use warpui::App;

    use super::*;
    use crate::ai::blocklist::hooks::test_utils::{
        HookTestApp, hook_test_app, set_working_directory,
    };
    use crate::workspaces::user_workspaces::UserWorkspaces;

    fn project(app: &mut App, harness: &HookTestApp) -> (TempDir, PathBuf) {
        let dir = TempDir::new().expect("tempdir");
        let root = dunce::canonicalize(dir.path()).expect("canonical");
        set_working_directory(app, harness, &root);
        (dir, root)
    }

    fn edit_of(file: &str) -> AIAgentActionType {
        AIAgentActionType::RequestFileEdits {
            file_edits: vec![FileEdit::Create {
                file: Some(file.to_owned()),
                content: Some(String::new()),
                allow_overwrite: true,
            }],
            title: None,
        }
    }

    fn floors_for(app: &mut App, harness: &HookTestApp, file: &str) -> PermissionFloors {
        let action = edit_of(file);
        app.read(|ctx| {
            let scope = UserWorkspaces::teamless_context_resolver_for_test()(ctx);
            permission_floors(
                &action,
                &harness.conversation_id,
                false,
                &harness.active_session,
                harness.terminal_view_id,
                &scope,
                ctx,
            )
        })
    }

    #[test]
    fn a_relative_path_to_a_protected_file_carries_the_protected_write_floor() {
        App::test((), |mut app| async move {
            let harness = hook_test_app(&mut app);
            let (_project, root) = project(&mut app, &harness);

            assert!(
                floors_for(&mut app, &harness, ".mcp.json").protected_write_path,
                "the model's relative spelling resolves to the protected file"
            );
            assert!(
                floors_for(
                    &mut app,
                    &harness,
                    &root.join(".mcp.json").display().to_string()
                )
                .protected_write_path,
                "the absolute spelling does too"
            );
            assert!(
                !floors_for(&mut app, &harness, "README.md").protected_write_path,
                "an ordinary project file is not protected"
            );
        });
    }
}
