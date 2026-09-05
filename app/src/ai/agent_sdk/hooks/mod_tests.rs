use super::HookEventName;

#[test]
fn oz_hooks_config_event_names_are_stable() {
    assert_eq!(
        HookEventName::ALL.map(HookEventName::as_str),
        [
            "SessionStart",
            "SessionEnd",
            "UserPromptSubmit",
            "Stop",
            "PreToolUse",
            "PostToolUse",
            "PreCompact",
        ]
    );
}
