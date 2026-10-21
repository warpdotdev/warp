use ai::hooks::test_util::{ScriptedRunner, test_binding, test_source_id};
use ai::hooks::{HookBoundary, HookVerdict};
use futures::executor::block_on;
use warpui::App;

use super::*;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::ai::blocklist::hooks::testing::{HookTestApp, hook_test_app, shell_action};
use crate::features::FeatureFlag;

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
fn lookup_is_inactive_when_the_flag_is_off() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        install_shell_deny(&harness, &mut app, "no");
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
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
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
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
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
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
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
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
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
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        install_shell_deny(&harness, &mut app, "no");
        let gate = harness.hooks.runner.hold();
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
        gate.release();

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
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        install_shell_deny(&harness, &mut app, "no");
        let gate = harness.hooks.runner.hold();
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
        gate.release();
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
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
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

#[test]
fn cancelling_a_conversation_clears_only_its_entries() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
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
