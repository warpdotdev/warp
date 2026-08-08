use super::{SHELL_LABEL_FLOOR, non_blank, resolve_rail_task_label};
use crate::workspace::tab_settings::{TabPrimaryInfo, TabSecondaryInfo};

/// Every shape of "nothing", as the sources actually produce them: an empty
/// string from a title that was never set, and whitespace from one that was
/// set to whitespace (which `is_empty` checks upstream let through).
const BLANKS: [&str; 4] = ["", " ", "   ", "\n\t "];

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

/// The guarantee the rail depends on: whatever blanks the four sources hand
/// over, the row still has something to draw. A blank label renders as a
/// highlighted row with no text in it, which is what this exists to prevent.
#[test]
fn a_live_tab_row_is_never_labelled_blank() {
    for configured in BLANKS {
        for stored in BLANKS {
            for tab in BLANKS {
                for shell in BLANKS {
                    let label = resolve_rail_task_label(
                        Some(configured.to_owned()),
                        || Some(stored.to_owned()),
                        || tab.to_owned(),
                        || Some(shell.to_owned()),
                    );
                    assert_eq!(
                        label, SHELL_LABEL_FLOOR,
                        "blank sources ({configured:?}, {stored:?}, {tab:?}, {shell:?}) \
                         resolved to {label:?}"
                    );
                }
            }
        }
    }
}

/// Sources missing entirely (no agent, no handle, no terminal session at all)
/// land on the same floor as blank ones.
#[test]
fn absent_sources_land_on_the_shell_floor() {
    let label = resolve_rail_task_label(None, || None, String::new, || None);
    assert_eq!(label, SHELL_LABEL_FLOOR);
}

/// The shell name is the floor above the constant: a plain `zsh` tab reads as
/// its shell rather than as "Shell".
#[test]
fn a_shell_tab_falls_back_to_the_shell_name() {
    for blank in BLANKS {
        let label = resolve_rail_task_label(
            Some(blank.to_owned()),
            || Some(blank.to_owned()),
            || blank.to_owned(),
            || Some("zsh".to_owned()),
        );
        assert_eq!(label, "zsh");
    }
}

/// Each source in turn is the only one with anything to say, and each in turn
/// wins — the order is the point, not just the non-blankness.
#[test]
fn the_first_non_blank_source_wins() {
    assert_eq!(
        resolve_rail_task_label(
            Some("Fix the parser".to_owned()),
            || Some("cached".to_owned()),
            || "tab".to_owned(),
            || Some("zsh".to_owned()),
        ),
        "Fix the parser"
    );
    assert_eq!(
        resolve_rail_task_label(
            Some(" ".to_owned()),
            || Some("cached".to_owned()),
            || "tab".to_owned(),
            || Some("zsh".to_owned()),
        ),
        "cached"
    );
    assert_eq!(
        resolve_rail_task_label(
            None,
            || Some("\n".to_owned()),
            || "tab".to_owned(),
            || Some("zsh".to_owned()),
        ),
        "tab"
    );
}

/// Titles arrive with whatever padding their source had; the row shows the
/// text, not the padding.
#[test]
fn labels_are_trimmed() {
    assert_eq!(
        resolve_rail_task_label(
            Some("  Fix the parser \n".to_owned()),
            || None,
            String::new,
            || None,
        ),
        "Fix the parser"
    );
}

/// The one gate every label passes through, checked directly.
#[test]
fn non_blank_rejects_every_blank_shape() {
    for blank in BLANKS {
        assert_eq!(non_blank(blank), None, "{blank:?} should not be a label");
    }
    assert_eq!(non_blank(" name ").as_deref(), Some("name"));
}
