use std::sync::atomic::AtomicBool;

use super::*;
use crate::App;

#[test]
fn test_keystroke_parse() -> anyhow::Result<()> {
    assert_eq!(
        Keystroke::parse("ctrl-p")?,
        Keystroke {
            key: "p".into(),
            ctrl: true,
            alt: false,
            shift: false,
            meta: false,
            cmd: false,
        }
    );

    assert_eq!(
        Keystroke::parse("alt-shift-down")?,
        Keystroke {
            key: "down".into(),
            ctrl: false,
            alt: true,
            shift: true,
            meta: false,
            cmd: false,
        }
    );

    assert_eq!(
        Keystroke::parse("shift-cmd--")?,
        Keystroke {
            key: "-".into(),
            ctrl: false,
            alt: false,
            shift: true,
            meta: false,
            cmd: true,
        }
    );

    assert_eq!(
        Keystroke::parse("shift-cmd-space")?,
        Keystroke {
            key: " ".into(),
            ctrl: false,
            alt: false,
            shift: true,
            meta: false,
            cmd: true,
        }
    );

    assert_eq!(
        Keystroke::parse("shift-cmd- ")?,
        Keystroke {
            key: " ".into(),
            ctrl: false,
            alt: false,
            shift: true,
            meta: false,
            cmd: true,
        }
    );

    assert_eq!(
        Keystroke::parse("enter")?,
        Keystroke {
            key: "enter".into(),
            ctrl: false,
            alt: false,
            shift: false,
            meta: false,
            cmd: false,
        }
    );

    Ok(())
}

#[test]
fn test_keystroke_normalized() -> anyhow::Result<()> {
    assert_eq!(Keystroke::parse("ctrl-p")?.normalized(), "ctrl-p");
    assert_eq!(Keystroke::parse("cmd-p")?.normalized(), "cmd-p");
    assert_eq!(Keystroke::parse("ctrl-alt-p")?.normalized(), "ctrl-alt-p");
    assert_eq!(
        Keystroke::parse("ctrl-alt-shift-P")?.normalized(),
        "ctrl-alt-shift-P"
    );

    assert_eq!(
        Keystroke::parse("ctrl-shift-P")?.normalized(),
        "ctrl-shift-P"
    );
    assert_eq!(
        Keystroke::parse("shift-ctrl-P")?.normalized(),
        "ctrl-shift-P"
    );

    assert_eq!(
        Keystroke::parse("shift-ctrl-space")?.normalized(),
        "ctrl-shift-space"
    );

    Ok(())
}

#[test]
#[should_panic]
fn test_keystroke_invalid_shift_lowercase() {
    let _ = Keystroke::parse("ctrl-shift-p");
}

#[test]
#[should_panic]
fn test_keystroke_invalid_no_shift_uppercase() {
    let _ = Keystroke::parse("ctrl-P");
}

#[test]
fn test_keymap_bindings_list() {
    use crate::keymap::macros::*;

    #[derive(Debug)]
    enum TypedAction {
        Open,
        Close,
        Idle,
        Copy,
        Paste,
    }

    let mut map = Keymap::default();
    map.register_editable_bindings([EditableBinding::new(
        "open",
        "Typed action for Open",
        TypedAction::Open,
    )
    .with_key_binding("cmd-o")]);
    map.register_fixed_bindings([FixedBinding::new("cmd-c", TypedAction::Copy, always!())]);
    map.register_editable_bindings([EditableBinding::new(
        "close",
        "Typed action for Close",
        TypedAction::Close,
    )
    .with_key_binding("cmd-w")]);
    map.register_fixed_bindings([FixedBinding::new("cmd-v", TypedAction::Paste, always!())]);
    map.register_editable_bindings([EditableBinding::new(
        "idle",
        "Typed action for Idle",
        TypedAction::Idle,
    )]);

    let mut ordered = map.bindings();

    // All bindings should be returned.
    // The precedence order for the bindings should be:
    // 1. Editable bindings in the reverse order they were registered (LIFO)
    // 2. Fixed bindings in the reverse order they were added

    // Given the above, we should have 5 items (in order): Idle, Close, Open, Paste, Copy
    let first = ordered.next().unwrap();
    assert_eq!(
        first
            .description
            .unwrap()
            .in_context(DescriptionContext::Default),
        "Typed Action for Idle"
    );
    assert!(first.trigger == &Trigger::Empty);
    let typed = first
        .action
        .as_ref()
        .as_any()
        .downcast_ref::<TypedAction>()
        .unwrap();
    assert!(matches!(typed, TypedAction::Idle));

    let second = ordered.next().unwrap();
    assert_eq!(
        second
            .description
            .unwrap()
            .in_context(DescriptionContext::Default),
        "Typed Action for Close"
    );
    assert!(second.trigger == &Trigger::Keystrokes(vec![Keystroke::parse("cmd-w").unwrap()]));
    let typed = second
        .action
        .as_ref()
        .as_any()
        .downcast_ref::<TypedAction>()
        .unwrap();
    assert!(matches!(typed, TypedAction::Close));

    let third = ordered.next().unwrap();
    assert_eq!(
        third
            .description
            .unwrap()
            .in_context(DescriptionContext::Default),
        "Typed Action for Open"
    );
    assert!(third.trigger == &Trigger::Keystrokes(vec![Keystroke::parse("cmd-o").unwrap()]));
    let typed = third
        .action
        .as_ref()
        .as_any()
        .downcast_ref::<TypedAction>()
        .unwrap();
    assert!(matches!(typed, TypedAction::Open));

    let fourth = ordered.next().unwrap();
    assert!(fourth.description.is_none());
    assert!(fourth.trigger == &Trigger::Keystrokes(vec![Keystroke::parse("cmd-v").unwrap()]));
    let typed = fourth
        .action
        .as_ref()
        .as_any()
        .downcast_ref::<TypedAction>()
        .unwrap();
    assert!(matches!(typed, TypedAction::Paste));

    let fifth = ordered.next().unwrap();
    assert!(fifth.description.is_none());
    assert!(fifth.trigger == &Trigger::Keystrokes(vec![Keystroke::parse("cmd-c").unwrap()]));
    let typed = fifth
        .action
        .as_ref()
        .as_any()
        .downcast_ref::<TypedAction>()
        .unwrap();
    assert!(matches!(typed, TypedAction::Copy));

    assert!(ordered.next().is_none());
}

#[test]
fn test_binding_description_preserves_case() {
    let desc = BindingDescription::new_preserve_case("/add-mcp");
    assert_eq!(desc.in_context(DescriptionContext::Default), "/add-mcp");

    let desc = BindingDescription::new_preserve_case("Add new MCP server");
    assert_eq!(
        desc.in_context(DescriptionContext::Default),
        "Add new MCP server"
    );
}

#[test]
fn test_custom_triggers() {
    #[derive(Debug)]
    enum TypedAction {
        First,
        Second,
    }

    let mut map = Keymap::default();
    let first_default_binding = Keystroke::parse("cmd-1").unwrap();
    let second_default_binding = Keystroke::parse("cmd-2").unwrap();
    let first_custom_trigger = Keystroke::parse("cmd-a").unwrap();

    map.register_editable_bindings([
        EditableBinding::new("first", "First editable binding", TypedAction::First)
            .with_key_binding("cmd-1"),
        EditableBinding::new("second", "Second editable binding", TypedAction::Second)
            .with_key_binding("cmd-2"),
    ]);

    map.update_custom_trigger(
        "first",
        Some(Trigger::Keystrokes(vec![first_custom_trigger.clone()])),
    );

    {
        let mut bindings = map.bindings();
        let second = bindings.next().unwrap();
        let first = bindings.next().unwrap();
        assert!(bindings.next().is_none());

        match first.trigger {
            Trigger::Keystrokes(keystrokes) => {
                assert_eq!(keystrokes, std::slice::from_ref(&first_custom_trigger));
            }
            _ => panic!("Expected keystroke trigger"),
        }

        match second.trigger {
            Trigger::Keystrokes(keystrokes) => {
                assert_eq!(keystrokes, std::slice::from_ref(&second_default_binding));
            }
            _ => panic!("Expected keystroke trigger"),
        }
    }

    {
        let mut editable_bindings = map.editable_bindings();
        let second = editable_bindings.next().unwrap();
        let first = editable_bindings.next().unwrap();
        assert!(editable_bindings.next().is_none());

        match first.trigger {
            Trigger::Keystrokes(keystrokes) => {
                assert_eq!(keystrokes, &[first_custom_trigger]);
            }
            _ => panic!("Expected keystroke trigger"),
        }

        match second.trigger {
            Trigger::Keystrokes(keystrokes) => {
                assert_eq!(keystrokes, std::slice::from_ref(&second_default_binding));
            }
            _ => panic!("Expected keystroke trigger"),
        }
    }

    map.update_custom_trigger("first", None);

    {
        let mut bindings = map.bindings();
        let second = bindings.next().unwrap();
        let first = bindings.next().unwrap();
        assert!(bindings.next().is_none());

        match first.trigger {
            Trigger::Keystrokes(keystrokes) => {
                assert_eq!(keystrokes, &[first_default_binding]);
            }
            _ => panic!("Expected keystroke trigger"),
        }

        match second.trigger {
            Trigger::Keystrokes(keystrokes) => {
                assert_eq!(keystrokes, &[second_default_binding]);
            }
            _ => panic!("Expected keystroke trigger"),
        }
    }
}

#[test]
fn test_disabled_bindings() {
    #[derive(Debug)]
    enum TypedAction {
        AlwaysAvailable,
        Enableable,
    }

    static TOGGLE: AtomicBool = AtomicBool::new(true);

    let mut map = Keymap::default();

    map.register_editable_bindings([
        EditableBinding::new("always", "First Binding", TypedAction::AlwaysAvailable)
            .with_key_binding("cmd-1"),
        EditableBinding::new("toggle", "Second Binding", TypedAction::Enableable)
            .with_key_binding("cmd-2")
            .with_enabled(|| TOGGLE.load(Ordering::Relaxed)),
    ]);

    // Since `TOGGLE` is `true`, both bindings should be listed.
    {
        let mut bindings = map.bindings();
        let second = bindings.next().unwrap();
        let first = bindings.next().unwrap();

        assert_eq!(
            first
                .description
                .unwrap()
                .in_context(DescriptionContext::Default),
            "First Binding"
        );

        assert_eq!(
            second
                .description
                .unwrap()
                .in_context(DescriptionContext::Default),
            "Second Binding"
        );
    }

    // If the binding is toggled off, it should no longer be listed.
    TOGGLE.store(false, Ordering::Relaxed);

    {
        let mut bindings = map.bindings();

        let first = bindings.next().expect("First binding should exist");
        assert_eq!(
            first
                .description
                .unwrap()
                .in_context(DescriptionContext::Default),
            "First Binding"
        );

        assert!(bindings.next().is_none());
    }
}

#[test]
fn test_binding_description_has_dynamic_override() {
    let plain = BindingDescription::new("static");
    assert!(!plain.has_dynamic_override());

    let dynamic =
        BindingDescription::new("static").with_dynamic_override(|_| Some("dynamic".into()));
    assert!(dynamic.has_dynamic_override());
}

#[test]
fn test_binding_description_in_context_ignores_dynamic_override() {
    let desc = BindingDescription::new("static").with_dynamic_override(|_| Some("dynamic".into()));
    assert_eq!(desc.in_context(DescriptionContext::Default), "Static");
}

#[test]
fn test_binding_description_eq_ignores_dynamic_override() {
    let plain = BindingDescription::new("static");
    let with_dynamic_override =
        BindingDescription::new("static").with_dynamic_override(|_| Some("dynamic".into()));
    let with_different_override =
        BindingDescription::new("static").with_dynamic_override(|_| Some("other".into()));

    assert_eq!(plain, with_dynamic_override);
    assert_eq!(with_dynamic_override, with_different_override);

    let different_static = BindingDescription::new("different");
    assert_ne!(plain, different_static);
}

#[cfg(feature = "settings_value")]
mod settings_value_tests {
    use super::*;
    use settings_value::SettingsValue;

    #[test]
    fn test_keystroke_to_file_value_is_normalized_string() {
        let keystroke = Keystroke::parse("alt-b").unwrap();
        assert_eq!(
            keystroke.to_file_value(),
            serde_json::Value::String("alt-b".to_string())
        );

        let keystroke = Keystroke::parse("ctrl-shift-P").unwrap();
        assert_eq!(
            keystroke.to_file_value(),
            serde_json::Value::String("ctrl-shift-P".to_string())
        );
    }

    #[test]
    fn test_keystroke_from_file_value_parses_string() {
        let value = serde_json::Value::String("cmd-shift-A".to_string());
        let keystroke = Keystroke::from_file_value(&value).unwrap();
        assert_eq!(keystroke, Keystroke::parse("cmd-shift-A").unwrap());
    }

    #[test]
    fn test_keystroke_round_trip_through_file_value() {
        for source in ["alt-b", "cmd-p", "ctrl-alt-shift-P", "shift-cmd-space"] {
            let keystroke = Keystroke::parse(source).unwrap();
            let round_tripped = Keystroke::from_file_value(&keystroke.to_file_value()).unwrap();
            assert_eq!(round_tripped, keystroke, "round-trip failed for {source}");
        }
    }

    #[test]
    fn test_keystroke_from_file_value_rejects_unparseable_string() {
        let value = serde_json::Value::String("not a valid keystroke".to_string());
        assert!(Keystroke::from_file_value(&value).is_none());
    }

    #[test]
    fn test_option_keystroke_round_trip() {
        let some = Some(Keystroke::parse("alt-b").unwrap());
        let round_tripped = Option::<Keystroke>::from_file_value(&some.to_file_value()).unwrap();
        assert_eq!(round_tripped, some);

        let none: Option<Keystroke> = None;
        assert_eq!(none.to_file_value(), serde_json::Value::Null);
        let round_tripped = Option::<Keystroke>::from_file_value(&serde_json::Value::Null).unwrap();
        assert_eq!(round_tripped, none);
    }
}

#[test]
fn test_binding_description_resolve_static() {
    App::test((), |app| async move {
        let resolved = app.read(|ctx| {
            BindingDescription::new("static")
                .resolve(ctx, DescriptionContext::Default)
                .into_owned()
        });
        assert_eq!(resolved, "Static");
    });
}

#[test]
fn test_binding_description_resolve_dynamic_override() {
    App::test((), |app| async move {
        let resolved = app.read(|ctx| {
            BindingDescription::new("static")
                .with_dynamic_override(|_| Some("dynamic".into()))
                .resolve(ctx, DescriptionContext::Default)
                .into_owned()
        });
        assert_eq!(resolved, "Dynamic");
    });
}

#[test]
fn test_binding_description_resolve_dynamic_override_falls_back_to_custom_context() {
    App::test((), |app| async move {
        let resolved = app.read(|ctx| {
            BindingDescription::new("static")
                .with_custom_description(DescriptionContext::Custom("menu"), "menu-static")
                .with_dynamic_override(|_| None)
                .resolve(ctx, DescriptionContext::Custom("menu"))
                .into_owned()
        });
        assert_eq!(resolved, "menu-static");
    });
}
