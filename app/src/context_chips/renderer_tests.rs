use pathfinder_color::ColorU;
use warpui::fonts::Properties;

use crate::context_chips::{ChipAvailability, ChipDisabledReason, ContextChipKind};

use super::{Renderer, RendererStyles};

#[test]
fn test_constructor_availability_updates_disabled_state_and_tooltip_override() {
    let kind = ContextChipKind::ShellGitBranch;
    let chip = kind.to_chip().expect("chip definition should exist");
    let renderer = Renderer::new(
        kind,
        chip,
        crate::context_chips::ChipValue::Text("main".to_string()),
        RendererStyles::new(
            ColorU {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
            Properties::default(),
        ),
        ChipAvailability::Disabled(ChipDisabledReason::RequiresExecutable {
            command: "gh".to_string(),
        }),
    );

    assert!(renderer.is_disabled);
    assert_eq!(
        renderer.tooltip_override_text.as_deref(),
        Some("Requires the GitHub CLI")
    );
}

#[test]
fn test_constructor_availability_enabled_has_no_disabled_state_or_tooltip_override() {
    let kind = ContextChipKind::ShellGitBranch;
    let chip = kind.to_chip().expect("chip definition should exist");
    let renderer = Renderer::new(
        kind,
        chip,
        crate::context_chips::ChipValue::Text("main".to_string()),
        RendererStyles::new(
            ColorU {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
            Properties::default(),
        ),
        ChipAvailability::Enabled,
    );

    assert!(!renderer.is_disabled);
    assert_eq!(renderer.tooltip_override_text, None);
}
