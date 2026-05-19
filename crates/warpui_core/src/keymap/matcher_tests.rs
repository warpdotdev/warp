use crate::keymap::macros::*;

use super::*;

#[test]
fn test_matcher() -> anyhow::Result<()> {
    #[derive(Debug, PartialEq)]
    enum Action {
        A(String),
        B,
        AB,
    }

    let keymap = Keymap::new(vec![
        FixedBinding::new("a", Action::A("b".into()), id!("a")),
        FixedBinding::new("b", Action::B, id!("a")),
        FixedBinding::new("a b", Action::AB, id!("a") | id!("b")),
    ]);

    let mut ctx_a = Context::default();
    ctx_a.set.insert("a");

    let mut ctx_b = Context::default();
    ctx_b.set.insert("b");

    let mut matcher = Matcher::new(keymap);

    let view_id = EntityId::new();

    // Basic match
    assert_eq!(
        matcher
            .test_keystroke("a", view_id, &ctx_a)
            .unwrap()
            .as_action::<Action>(),
        &Action::A("b".into())
    );

    // Multi-keystroke match
    assert!(matcher.test_keystroke("a", view_id, &ctx_b).is_none());
    assert_eq!(
        matcher
            .test_keystroke("b", view_id, &ctx_b)
            .unwrap()
            .as_action::<Action>(),
        &Action::AB
    );

    // Failed matches don't interfere with matching subsequent keys
    assert!(matcher.test_keystroke("x", view_id, &ctx_a).is_none());
    assert_eq!(
        matcher
            .test_keystroke("a", view_id, &ctx_a)
            .unwrap()
            .as_action::<Action>(),
        &Action::A("b".into())
    );

    // Pending keystrokes are cleared when the context changes
    assert!(matcher.test_keystroke("a", view_id, &ctx_b).is_none());
    assert_eq!(
        matcher
            .test_keystroke("b", view_id, &ctx_a)
            .unwrap()
            .as_action::<Action>(),
        &Action::B
    );

    let mut ctx_c = Context::default();
    ctx_c.set.insert("c");

    // Pending keystrokes are maintained per-view
    let view_id1 = EntityId::new();
    let view_id2 = EntityId::new();
    assert_ne!(view_id1, view_id2);
    assert!(matcher.test_keystroke("a", view_id1, &ctx_b).is_none());
    assert!(matcher.test_keystroke("a", view_id2, &ctx_c).is_none());
    assert_eq!(
        matcher
            .test_keystroke("b", view_id1, &ctx_b)
            .unwrap()
            .as_action::<Action>(),
        &Action::AB
    );

    Ok(())
}

#[test]
fn test_editable_binding_matching() {
    #[derive(Debug, PartialEq)]
    enum Action {
        A(&'static str),
        B,
        AOrB,
    }

    let mut keymap = Keymap::default();
    use crate::keymap::macros::*;
    keymap.register_editable_bindings([
        EditableBinding::new("a", "Action for A", Action::A("b"))
            .with_key_binding("a")
            .with_context_predicate(id!("a")),
        EditableBinding::new("b", "Action for B", Action::B)
            .with_key_binding("b")
            .with_context_predicate(id!("a")),
        EditableBinding::new("a_or_b", "Action for A or B", Action::AOrB)
            .with_key_binding("a b")
            .with_context_predicate(id!("a") | id!("b")),
    ]);

    let mut ctx_a = Context::default();
    ctx_a.set.insert("a");

    let mut ctx_b = Context::default();
    ctx_b.set.insert("b");

    let mut matcher = Matcher::new(keymap);

    let view_id = EntityId::new();

    // Basic match
    assert_eq!(
        matcher
            .test_keystroke("a", view_id, &ctx_a)
            .unwrap()
            .as_action::<Action>(),
        &Action::A("b"),
    );

    // Multi-keystroke match
    assert!(matcher.test_keystroke("a", view_id, &ctx_b).is_none());
    assert_eq!(
        matcher
            .test_keystroke("b", view_id, &ctx_b)
            .unwrap()
            .as_action::<Action>(),
        &Action::AOrB
    );

    // Failed matches don't interfere with matching subsequent keys
    assert!(matcher.test_keystroke("x", view_id, &ctx_a).is_none());
    assert_eq!(
        matcher
            .test_keystroke("a", view_id, &ctx_a)
            .unwrap()
            .as_action::<Action>(),
        &Action::A("b")
    );

    // Pending keystrokes are cleared when the context changes
    assert!(matcher.test_keystroke("a", view_id, &ctx_b).is_none());
    assert_eq!(
        matcher
            .test_keystroke("b", view_id, &ctx_a)
            .unwrap()
            .as_action::<Action>(),
        &Action::B
    );

    let mut ctx_c = Context::default();
    ctx_c.set.insert("c");

    // Pending keystrokes are maintained per-view
    let view_id1 = EntityId::new();
    let view_id2 = EntityId::new();
    assert_ne!(view_id1, view_id2);
    assert!(matcher.test_keystroke("a", view_id1, &ctx_b).is_none());
    assert!(matcher.test_keystroke("a", view_id2, &ctx_c).is_none());
    assert_eq!(
        matcher
            .test_keystroke("b", view_id1, &ctx_b)
            .unwrap()
            .as_action::<Action>(),
        &Action::AOrB
    );
}

/// Regression test for https://github.com/warpdotdev/warp/issues/9128.
///
/// When the user explicitly rebinds an editable action to a keystroke that another editable
/// action already uses by default, the user's binding must win precedence regardless of
/// registration order. Previously, the iteration order followed only registration order
/// (most-recently-registered wins), so default bindings registered *after* the user's
/// rebound action would silently shadow the user override.
#[test]
fn test_user_override_wins_over_default_with_same_keystroke() {
    #[derive(Debug, PartialEq)]
    enum Action {
        Copy,
        SplitPaneDown,
    }

    let mut keymap = Keymap::default();
    use crate::keymap::macros::*;

    // Simulate the real-world ordering from #9128: an action that the user wants to bind
    // (`split_pane_down`) is registered BEFORE the action whose default keystroke conflicts
    // with the user override (`copy`). In the buggy precedence model, `copy` is iterated
    // first because it was registered later, so the user's override never fires.
    keymap.register_editable_bindings([EditableBinding::new(
        "split_pane_down",
        "Split pane down",
        Action::SplitPaneDown,
    )
    .with_key_binding("ctrl-shift-E")
    .with_context_predicate(id!("PaneGroup"))]);
    keymap.register_editable_bindings([EditableBinding::new("copy", "Copy", Action::Copy)
        .with_key_binding("ctrl-shift-C")
        .with_context_predicate(id!("Terminal"))]);

    // The user rebinds Split Pane Down to ctrl-shift-C, which collides with the default
    // Copy binding above.
    keymap.update_custom_trigger(
        "split_pane_down",
        Some(Trigger::Keystrokes(vec![
            Keystroke::parse("ctrl-shift-C").unwrap()
        ])),
    );

    // Build a context that satisfies BOTH bindings' predicates — this is the real-world
    // case: the terminal lives inside the pane group, so the active context contains
    // both `Terminal` and `PaneGroup` tags.
    let mut ctx = Context::default();
    ctx.set.insert("Terminal");
    ctx.set.insert("PaneGroup");

    let mut matcher = Matcher::new(keymap);
    let view_id = EntityId::new();
    let action = matcher
        .test_keystroke("ctrl-shift-C", view_id, &ctx)
        .expect("user-overridden binding should fire");
    assert_eq!(
        action.as_action::<Action>(),
        &Action::SplitPaneDown,
        "user-customized binding should take precedence over a default that uses the same keystroke"
    );
}

#[test]
fn test_bindings_for_context() {
    #[derive(Debug)]
    enum Action {
        A,
        B,
        C,
    }
    let keymap = Keymap::new(vec![
        FixedBinding::new("a", Action::A, id!("a")),
        FixedBinding::new("b", Action::B, id!("b")),
        FixedBinding::new("c", Action::C, id!("b")),
    ]);
    let matcher = Matcher::new(keymap);

    let mut ctx_a = Context::default();
    ctx_a.set.insert("a");

    let mut ctx_b = Context::default();
    ctx_b.set.insert("b");

    // Getting bindings for the 'a' context returns a single result
    let ctx_a_bindings = matcher
        .bindings_for_context(ctx_a)
        .filter_map(|bind| match bind.trigger {
            Trigger::Keystrokes(keys) => {
                assert_eq!(keys.len(), 1);
                Some(keys[0].normalized())
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(ctx_a_bindings.len(), 1);
    assert_eq!(ctx_a_bindings, vec!["a"]);

    // Getting bindings for the 'b' context returns two results, in the reverse order they
    // added, so the "c" binding first followed by the "b" binding
    let ctx_b_bindings = matcher
        .bindings_for_context(ctx_b)
        .filter_map(|bind| match bind.trigger {
            Trigger::Keystrokes(keys) => {
                assert_eq!(keys.len(), 1);
                Some(keys[0].normalized())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(ctx_b_bindings, vec!["c", "b"]);
}

impl Matcher {
    fn test_keystroke(
        &mut self,
        keystroke: &str,
        view_id: EntityId,
        ctx: &Context,
    ) -> Option<Arc<dyn Action>> {
        match self.push_keystroke(Keystroke::parse(keystroke).unwrap(), view_id, ctx) {
            MatchResult::Action(action) => Some(action),
            _ => None,
        }
    }
}

trait AsAction {
    fn as_action<A: Action>(&self) -> &A;
}

impl AsAction for Arc<dyn Action> {
    fn as_action<A: Action>(&self) -> &A {
        self.as_ref().as_any().downcast_ref::<A>().unwrap()
    }
}
