use ai::hooks::test_util::{ScriptedRunner, test_binding, test_source_id};
use ai::hooks::{HookBoundary, HookVerdict};
use futures::executor::block_on;
use warpui::App;

use super::*;
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

#[test]
fn lookup_is_inactive_when_the_flag_is_off() {
    App::test((), |mut app| async move {
        let harness = hook_test_app(&mut app);
        install_shell_deny(&harness, &mut app, "no");
        let action = shell_action("a1", "ls");
        let lookup = app.update(|ctx| {
            begin_or_lookup(
                &action,
                harness.conversation_id,
                false,
                &harness.active_session,
                ctx,
            )
        });
        assert!(matches!(lookup, PreToolLookup::Inactive));
        assert_eq!(harness.hooks.adapter.encoded_count(), 0);
        assert!(app.read(|ctx| cached_state(&action.id, ctx)).is_none());
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
        let lookup = app.update(|ctx| {
            begin_or_lookup(
                &action,
                harness.conversation_id,
                false,
                &harness.active_session,
                ctx,
            )
        });
        assert!(matches!(lookup, PreToolLookup::Inactive));
        assert!(app.read(|ctx| cached_state(&action.id, ctx)).is_none());
    });
}

#[test]
fn first_lookup_starts_a_dispatch_and_later_lookups_wait_on_it() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let harness = hook_test_app(&mut app);
        install_shell_deny(&harness, &mut app, "blocked by policy");
        let action = shell_action("a1", "ls");

        let first = app.update(|ctx| {
            begin_or_lookup(
                &action,
                harness.conversation_id,
                false,
                &harness.active_session,
                ctx,
            )
        });
        let PreToolLookup::Pending {
            dispatch: Some(dispatch),
        } = first
        else {
            panic!("the first lookup must hand back a dispatch");
        };
        assert_eq!(dispatch.action_id, action.id);
        assert!(matches!(
            app.read(|ctx| cached_state(&action.id, ctx)),
            Some(PreToolHookState::Pending)
        ));

        let second = app.update(|ctx| {
            begin_or_lookup(
                &action,
                harness.conversation_id,
                true,
                &harness.active_session,
                ctx,
            )
        });
        assert!(matches!(second, PreToolLookup::Pending { dispatch: None }));

        let outcome = block_on(dispatch.future);
        assert_eq!(outcome.effects.verdict, HookVerdict::Deny);
        let resumption = app.update(|ctx| on_resolved(&action.id, outcome, ctx));
        assert_eq!(
            resumption,
            Some(Resumption {
                conversation_id: harness.conversation_id,
                is_user_initiated: true,
            }),
            "user approval seen while pending sticks to the resumption"
        );

        let third = app.update(|ctx| {
            begin_or_lookup(
                &action,
                harness.conversation_id,
                false,
                &harness.active_session,
                ctx,
            )
        });
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
        let PreToolLookup::Pending {
            dispatch: Some(dispatch),
        } = app.update(|ctx| {
            begin_or_lookup(
                &action,
                harness.conversation_id,
                false,
                &harness.active_session,
                ctx,
            )
        })
        else {
            panic!("expected a dispatch");
        };
        let cancel = app
            .read(|ctx| cancel_flag(&action.id, ctx))
            .expect("pending entry has a cancel flag");

        app.update(|ctx| invalidate(&action.id, ctx));
        assert!(cancel.load(Ordering::Acquire));
        assert!(app.read(|ctx| cached_state(&action.id, ctx)).is_none());

        let outcome = block_on(dispatch.future);
        assert!(
            app.update(|ctx| on_resolved(&action.id, outcome, ctx))
                .is_none()
        );
        assert!(app.read(|ctx| cached_state(&action.id, ctx)).is_none());
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
            let lookup = app.update(|ctx| {
                begin_or_lookup(action, conversation_id, false, &harness.active_session, ctx)
            });
            assert!(matches!(lookup, PreToolLookup::Pending { .. }));
        }
        let mine_cancel = app.read(|ctx| cancel_flag(&mine.id, ctx)).expect("pending");

        app.update(|ctx| cancel_all_for_conversation(harness.conversation_id, ctx));

        assert!(mine_cancel.load(Ordering::Acquire));
        assert!(app.read(|ctx| cached_state(&mine.id, ctx)).is_none());
        assert!(matches!(
            app.read(|ctx| cached_state(&theirs.id, ctx)),
            Some(PreToolHookState::Pending)
        ));
    });
}
