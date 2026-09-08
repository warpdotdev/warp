use super::{
    ANNOTATED_DEFAULT, AgentTabBadgeSize, AgentTabColor, AgentTabStyleLayer, AgentTabStyles,
};

#[test]
fn agent_tab_styles_config_contract() {
    let defaults = AgentTabStyles::default();
    assert_eq!(
        AgentTabStyles::parse(ANNOTATED_DEFAULT),
        Ok(defaults.clone())
    );

    let partial = AgentTabStyles::parse("version: 1\nstates:\n  success:\n    color: cyan\n")
        .expect("partial config should inherit defaults");
    assert_eq!(partial.states.success.color, AgentTabColor::Cyan);
    assert_eq!(partial.states.success.badge_size, AgentTabBadgeSize::Big);
    assert_eq!(
        partial.states.success.layers,
        [
            AgentTabStyleLayer::TabBg,
            AgentTabStyleLayer::TabText,
            AgentTabStyleLayer::BadgeIcon
        ]
    );
    assert_eq!(partial.states.processing, defaults.states.processing);

    for invalid in [
        "version: 2",
        "version: 1\nunknown: true",
        "version: 1\nstates:\n  idle:\n    color: orange",
        "version: 1\nstates:\n  idle:\n    badge_size: huge",
        "version: 1\nstates:\n  idle:\n    layers: [tab_bg, unknown]",
        "version: 1\nstates:\n  idle:\n    layers: [tab_bg, tab_bg]",
        "version: 1\ngroup_outline:\n  colors: [blue, blue]",
    ] {
        assert!(
            AgentTabStyles::parse(invalid).is_err(),
            "accepted {invalid}"
        );
    }
}
