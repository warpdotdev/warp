use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;

pub(crate) fn local_child_harness_disabled_message(harness: Harness) -> Option<&'static str> {
    if FeatureFlag::LocalClaudeCodexChildHarnesses.is_enabled() {
        return None;
    }

    match harness {
        Harness::Claude => Some("Local Claude Code child agents are temporarily disabled."),
        Harness::Codex => Some("Local Codex child agents are temporarily disabled."),
        Harness::Oz | Harness::OpenCode | Harness::Gemini | Harness::Unknown => None,
    }
}

pub(crate) fn local_child_harness_is_enabled(harness: Harness) -> bool {
    local_child_harness_disabled_message(harness).is_none()
}
