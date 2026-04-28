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
        match self.push_keystroke(Keystroke::parse(keystroke).unwrap(), None, view_id, ctx) {
            MatchResult::Action(action) => Some(action),
            _ => None,
        }
    }

    /// Like `test_keystroke`, but also supplies a physical key code - simulates
    /// what `convert_keyboard_input_event` produces for a real OS keypress.
    fn test_keystroke_with_physical(
        &mut self,
        keystroke: &str,
        physical_code: &str,
        view_id: EntityId,
        ctx: &Context,
    ) -> Option<Arc<dyn Action>> {
        match self.push_keystroke(
            Keystroke::parse(keystroke).unwrap(),
            Some(physical_code),
            view_id,
            ctx,
        ) {
            MatchResult::Action(action) => Some(action),
            _ => None,
        }
    }
}

#[test]
fn test_explicit_physical_binding_matches_any_layout() -> anyhow::Result<()> {
    // A binding that explicitly opts into physical-key matching should fire
    // regardless of the active keyboard layout - this is the "I know what I'm
    // doing, force layout-independent matching" path.
    #[derive(Debug, PartialEq)]
    enum Action {
        Copy,
    }

    let mut keymap = Keymap::default();
    keymap.register_editable_bindings([
        EditableBinding::new("editor:copy", "Copy", Action::Copy)
            .with_key_binding("cmd-Code(KeyC)")
            .with_context_predicate(id!("editor")),
    ]);

    let mut ctx = Context::default();
    ctx.set.insert("editor");

    let mut matcher = Matcher::new(keymap);
    let view_id = EntityId::new();

    // Russian layout: physical KeyC produces logical "с" (Cyrillic). The
    // explicit Physical binding still matches.
    assert_eq!(
        matcher
            .test_keystroke_with_physical("cmd-с", "KeyC", view_id, &ctx)
            .map(|a| a.as_action::<Action>().clone()),
        Some(Action::Copy),
    );

    // English layout: same binding matches the equivalent logical event.
    assert_eq!(
        matcher
            .test_keystroke_with_physical("cmd-c", "KeyC", view_id, &ctx)
            .map(|a| a.as_action::<Action>().clone()),
        Some(Action::Copy),
    );

    // Wrong physical key on the same layout - should not match (sanity check).
    assert!(matcher
        .test_keystroke_with_physical("cmd-v", "KeyV", view_id, &ctx)
        .is_none());

    Ok(())
}

#[test]
fn test_smart_binding_promotes_alphanumeric_logical() -> anyhow::Result<()> {
    // The master "Smart layout-aware shortcuts" toggle: a plain Logical
    // `cmd-c` binding starts matching by physical KeyC when the toggle is on.
    // This is the one-click "fix copy/paste under RU" path.
    #[derive(Debug, PartialEq, Clone)]
    enum Action {
        Copy,
    }

    let mut keymap = Keymap::default();
    keymap.register_editable_bindings([
        EditableBinding::new("editor:copy", "Copy", Action::Copy)
            .with_key_binding("cmd-c")
            .with_context_predicate(id!("editor")),
    ]);

    let mut ctx = Context::default();
    ctx.set.insert("editor");

    let mut matcher = Matcher::new(keymap);
    let view_id = EntityId::new();

    // Smart binding OFF + RU layout: doesn't match (this is the bug we
    // started with - documented here as a regression-guard test).
    assert!(matcher
        .test_keystroke_with_physical("cmd-с", "KeyC", view_id, &ctx)
        .is_none());

    // Smart binding ON + RU layout: matches via physical normalization.
    matcher.set_smart_binding_enabled(true);
    assert_eq!(
        matcher
            .test_keystroke_with_physical("cmd-с", "KeyC", view_id, &ctx)
            .map(|a| a.as_action::<Action>().clone()),
        Some(Action::Copy),
    );

    // Smart binding ON + EN layout: still matches (no regression).
    assert_eq!(
        matcher
            .test_keystroke_with_physical("cmd-c", "KeyC", view_id, &ctx)
            .map(|a| a.as_action::<Action>().clone()),
        Some(Action::Copy),
    );

    Ok(())
}

#[test]
fn test_smart_binding_does_not_promote_symbol_bindings() -> anyhow::Result<()> {
    // Smart binding only applies to letter/digit physical keys. Symbol
    // bindings like `cmd-/` continue to match by logical character - that's
    // what the user expects (the slash symbol lives in different physical
    // positions across layouts and forcing it to one would be wrong).
    #[derive(Debug, PartialEq, Clone)]
    enum Action {
        Find,
    }

    let mut keymap = Keymap::default();
    keymap.register_editable_bindings([
        EditableBinding::new("editor:find", "Find", Action::Find)
            .with_key_binding("cmd-/")
            .with_context_predicate(id!("editor")),
    ]);

    let mut ctx = Context::default();
    ctx.set.insert("editor");

    let mut matcher = Matcher::new(keymap);
    matcher.set_smart_binding_enabled(true);
    let view_id = EntityId::new();

    // Logical `/` matches.
    assert_eq!(
        matcher
            .test_keystroke_with_physical("cmd-/", "Slash", view_id, &ctx)
            .map(|a| a.as_action::<Action>().clone()),
        Some(Action::Find),
    );

    // A different layout's physical "Slash" without a `/` logical key should
    // *not* fire `cmd-/` - the Smart binding promotion stays off for symbols,
    // which is the right behavior.
    assert!(matcher
        .test_keystroke_with_physical("cmd-7", "Slash", view_id, &ctx)
        .is_none());

    Ok(())
}

#[test]
fn test_smart_binding_suppressed_during_ime_composition() -> anyhow::Result<()> {
    // While the user is composing CJK input (IME), the matcher must not fire
    // hotkeys via physical-key promotion - the user is typing kanji, not
    // pressing copy.
    #[derive(Debug, PartialEq, Clone)]
    enum Action {
        Copy,
    }

    let mut keymap = Keymap::default();
    keymap.register_editable_bindings([
        EditableBinding::new("editor:copy", "Copy", Action::Copy)
            .with_key_binding("cmd-c")
            .with_context_predicate(id!("editor")),
    ]);

    let mut ctx = Context::default();
    ctx.set.insert("editor");
    ctx.set.insert("IMEOpen");

    let mut matcher = Matcher::new(keymap);
    matcher.set_smart_binding_enabled(true);
    let view_id = EntityId::new();

    // Even with smart binding on, IMEOpen blocks the physical-key shortcut.
    assert!(matcher
        .test_keystroke_with_physical("cmd-с", "KeyC", view_id, &ctx)
        .is_none());

    Ok(())
}

trait AsAction {
    fn as_action<A: Action>(&self) -> &A;
}

impl AsAction for Arc<dyn Action> {
    fn as_action<A: Action>(&self) -> &A {
        self.as_ref().as_any().downcast_ref::<A>().unwrap()
    }
}
