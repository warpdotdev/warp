use crate::workspace::tab_settings::{TabPrimaryInfo, TabSecondaryInfo};

/// A tab showing the same information twice is never useful, so a secondary
/// choice that collides with the primary has to fall back to something else.
#[test]
fn secondary_info_never_duplicates_the_primary() {
    let primaries = [
        TabPrimaryInfo::AgentSession,
        TabPrimaryInfo::Command,
        TabPrimaryInfo::WorkingDirectory,
        TabPrimaryInfo::Branch,
    ];
    let secondaries = [
        TabSecondaryInfo::AgentSession,
        TabSecondaryInfo::Command,
        TabSecondaryInfo::WorkingDirectory,
        TabSecondaryInfo::Branch,
    ];

    for primary in primaries {
        for secondary in secondaries {
            let resolved = secondary.resolved_for(primary);
            let duplicates = matches!(
                (primary, resolved),
                (TabPrimaryInfo::AgentSession, TabSecondaryInfo::AgentSession)
                    | (TabPrimaryInfo::Command, TabSecondaryInfo::Command)
                    | (
                        TabPrimaryInfo::WorkingDirectory,
                        TabSecondaryInfo::WorkingDirectory
                    )
                    | (TabPrimaryInfo::Branch, TabSecondaryInfo::Branch)
            );
            assert!(
                !duplicates,
                "primary {primary:?} with secondary {secondary:?} resolved to {resolved:?}, \
                 which shows the same information on both lines"
            );
        }
    }
}

/// A non-conflicting choice must be respected exactly — conflict avoidance
/// should only ever intervene on an actual collision.
#[test]
fn secondary_info_is_preserved_when_it_does_not_conflict() {
    assert_eq!(
        TabSecondaryInfo::Command.resolved_for(TabPrimaryInfo::AgentSession),
        TabSecondaryInfo::Command
    );
    assert_eq!(
        TabSecondaryInfo::Branch.resolved_for(TabPrimaryInfo::Command),
        TabSecondaryInfo::Branch
    );
    assert_eq!(
        TabSecondaryInfo::AgentSession.resolved_for(TabPrimaryInfo::WorkingDirectory),
        TabSecondaryInfo::AgentSession
    );
}

/// The default pairing is the one most users see, so it must not collide.
#[test]
fn defaults_pair_agent_session_with_command() {
    let primary = TabPrimaryInfo::default();
    let secondary = TabSecondaryInfo::default().resolved_for(primary);
    assert_eq!(primary, TabPrimaryInfo::AgentSession);
    assert_eq!(secondary, TabSecondaryInfo::Command);
}
