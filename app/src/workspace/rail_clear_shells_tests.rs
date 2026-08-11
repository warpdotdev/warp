use super::*;

/// A row with every exemption cleared: a lone idle terminal with no agent on
/// it. Each test flips exactly one field, so the assertion names the reason.
fn plain_shell(tab_index: usize) -> RailShellRow {
    RailShellRow {
        tab_index,
        has_agent: false,
        is_busy: false,
        is_lone_terminal: true,
        is_shared: false,
        project: "warp".to_owned(),
    }
}

#[test]
fn an_idle_shell_with_no_agent_is_cleared() {
    let rows = vec![plain_shell(0), plain_shell(1), plain_shell(2)];

    assert_eq!(shells_to_clear(&rows, Some(9)), vec![0, 1, 2]);
}

/// `has_agent` is one bool here, but at the call site it is
/// [`tab_title::pane_has_agent`](crate::workspace::tab_title::pane_has_agent),
/// which is `true` for a live CLI agent session, a non-passive Agent Mode
/// conversation, *or* a stored session handle. This covers all three at once:
/// whichever source said "agent", the row survives.
///
/// Scanned sessions need no case: they have no open tab, so they are dormant
/// rows and never reach this list at all.
#[test]
fn anything_agent_backed_survives() {
    let rows = vec![
        RailShellRow {
            has_agent: true,
            ..plain_shell(0)
        },
        plain_shell(1),
    ];

    assert_eq!(
        shells_to_clear(&rows, None),
        vec![1],
        "a live agent, a stored handle or an Agent Mode conversation all keep a row"
    );
}

/// Closing the tab the user is looking at yanks the terminal out from under
/// them, and it is also the one tab they demonstrably still want.
#[test]
fn the_active_tab_is_never_cleared() {
    let rows = vec![plain_shell(0), plain_shell(1), plain_shell(2)];

    assert_eq!(shells_to_clear(&rows, Some(1)), vec![0, 2]);
}

/// The active tab is exempt even when it is the *only* candidate — the action
/// then finds nothing and, per `an_empty_selection_clears_nothing`, does not
/// confirm.
#[test]
fn the_active_tab_alone_leaves_nothing_to_clear() {
    let rows = vec![plain_shell(4)];

    assert!(shells_to_clear(&rows, Some(4)).is_empty());
}

/// A long-running command is work in progress; ending it is the user's call,
/// made deliberately, not a side effect of tidying the rail.
#[test]
fn a_pane_running_a_command_is_never_cleared() {
    let rows = vec![
        RailShellRow {
            is_busy: true,
            ..plain_shell(0)
        },
        plain_shell(1),
    ];

    assert_eq!(shells_to_clear(&rows, None), vec![1]);
}

/// A split, a code pane, or an off-tree child agent pane makes the tab more
/// than one terminal — and closing it would take a sibling with it that no
/// other exemption has vouched for.
#[test]
fn a_tab_that_is_more_than_one_terminal_is_never_cleared() {
    let rows = vec![
        RailShellRow {
            is_lone_terminal: false,
            ..plain_shell(0)
        },
        plain_shell(1),
    ];

    assert_eq!(shells_to_clear(&rows, None), vec![1]);
}

/// Closing a shared session ends it for the other participant too.
#[test]
fn a_shared_session_is_never_cleared() {
    let rows = vec![
        RailShellRow {
            is_shared: true,
            ..plain_shell(0)
        },
        plain_shell(1),
    ];

    assert_eq!(shells_to_clear(&rows, None), vec![1]);
}

/// The caller keys off an empty selection to skip the dialog entirely: a
/// confirmation for "close 0 shells" is a dead end the user has to dismiss.
#[test]
fn an_empty_selection_clears_nothing() {
    let rows = vec![
        RailShellRow {
            has_agent: true,
            ..plain_shell(0)
        },
        RailShellRow {
            is_busy: true,
            ..plain_shell(1)
        },
        plain_shell(2),
    ];

    assert!(
        shells_to_clear(&rows, Some(2)).is_empty(),
        "every row is exempt, so there is nothing to confirm"
    );
    assert!(shells_to_clear(&[], None).is_empty());
}

/// Rail order is preserved so the dialog's project list reads against the list
/// the user is looking at, and each project is named once.
#[test]
fn projects_are_listed_once_in_rail_order() {
    let rows = vec![
        RailShellRow {
            project: "warp".to_owned(),
            ..plain_shell(0)
        },
        RailShellRow {
            project: "docs".to_owned(),
            ..plain_shell(1)
        },
        RailShellRow {
            project: "warp".to_owned(),
            ..plain_shell(2)
        },
        RailShellRow {
            project: "server".to_owned(),
            ..plain_shell(3)
        },
    ];
    let selection = shells_to_clear(&rows, None);

    assert_eq!(projects_of(&rows, &selection), ["warp", "docs", "server"]);
    assert_eq!(
        projects_of(&rows, &[3]),
        ["server"],
        "only the projects that actually lose a tab are named"
    );
}

#[test]
fn the_prompt_counts_one_shell_as_one() {
    assert_eq!(clear_shells_prompt(1), "Close 1 shell with no agent?");
    assert_eq!(clear_shells_prompt(2), "Close 2 shells with no agent?");
    assert_eq!(clear_shells_prompt(23), "Close 23 shells with no agent?");
}

#[test]
fn the_toast_counts_one_shell_as_one() {
    assert_eq!(cleared_shells_label(1), "Closed 1 shell");
    assert_eq!(cleared_shells_label(23), "Closed 23 shells");
}

/// Past three projects the list stops being something you can take in at a
/// glance, so it becomes a count instead.
#[test]
fn the_detail_line_names_at_most_three_projects() {
    let named = |projects: &[&str]| {
        clear_shells_detail(&projects.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>())
    };

    assert!(named(&["warp"]).starts_with("In warp. "));
    assert!(named(&["warp", "docs", "server"]).starts_with("In warp, docs, server. "));
    assert!(
        named(&["warp", "docs", "server", "cli", "web"])
            .starts_with("In warp, docs, server and 2 more. ")
    );
    assert!(
        !named(&[]).contains("In "),
        "with no project to name the line is just the promise"
    );
    for projects in [
        &[][..],
        &["warp"][..],
        &["warp", "docs", "server", "cli"][..],
    ] {
        assert!(
            named(projects).contains(
                "The active tab, anything with an agent, and anything running a command stay open."
            ),
            "the exemptions are always spelled out"
        );
    }
}
