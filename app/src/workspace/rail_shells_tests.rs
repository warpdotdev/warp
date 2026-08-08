use super::{RailLiveRow, RailShellFilter, hidden_shells_label, visible_live_rows};

fn shell(tab_index: usize) -> RailLiveRow {
    RailLiveRow {
        tab_index,
        has_agent: false,
    }
}

fn agent(tab_index: usize) -> RailLiveRow {
    RailLiveRow {
        tab_index,
        has_agent: true,
    }
}

/// The setting off is today's rail exactly: every row, nothing counted as
/// hidden.
#[test]
fn filter_off_keeps_every_row() {
    let rows = [agent(0), shell(1), shell(2)];
    let view = visible_live_rows(
        &rows,
        RailShellFilter {
            hide_shells: false,
            active_tab: Some(0),
            fallback_row: Some(0),
        },
    );

    assert_eq!(view.visible, vec![0, 1, 2]);
    assert_eq!(view.hidden_shells, 0);
}

/// The point of the feature: agent rows stay, agent-less shells go, and the
/// count of what went is exact.
#[test]
fn agentless_shells_are_hidden_and_counted() {
    let rows = [agent(0), shell(1), shell(2), agent(3)];
    let view = visible_live_rows(
        &rows,
        RailShellFilter {
            hide_shells: true,
            active_tab: None,
            fallback_row: None,
        },
    );

    assert_eq!(view.visible, vec![0, 3]);
    assert_eq!(view.hidden_shells, 2);
}

/// Hiding the row of the tab the user is looking at would leave the rail
/// disagreeing with the terminal on screen.
#[test]
fn the_active_tab_is_never_hidden() {
    let rows = [shell(0), shell(1), agent(2)];
    let view = visible_live_rows(
        &rows,
        RailShellFilter {
            hide_shells: true,
            active_tab: Some(1),
            fallback_row: None,
        },
    );

    assert_eq!(view.visible, vec![1, 2]);
    assert_eq!(view.hidden_shells, 1);
}

/// The selected project must not collapse to a bare header, so its
/// most-recently-used tab survives when nothing else would.
#[test]
fn the_selected_projects_last_row_survives() {
    let rows = [shell(4), shell(5), shell(6)];
    let view = visible_live_rows(
        &rows,
        RailShellFilter {
            hide_shells: true,
            active_tab: None,
            fallback_row: Some(5),
        },
    );

    assert_eq!(view.visible, vec![5]);
    assert_eq!(view.hidden_shells, 2);
}

/// The fallback only applies when the filter would otherwise empty the
/// project: an agent row already keeps it populated, so the shells still go.
#[test]
fn the_fallback_does_not_fire_while_a_row_survives() {
    let rows = [shell(0), agent(1), shell(2)];
    let view = visible_live_rows(
        &rows,
        RailShellFilter {
            hide_shells: true,
            active_tab: None,
            fallback_row: Some(0),
        },
    );

    assert_eq!(view.visible, vec![1]);
    assert_eq!(view.hidden_shells, 2);
}

/// The fallback names a tab by index, and an index from another project must
/// never pull a foreign row into this one.
#[test]
fn a_fallback_outside_the_project_is_ignored() {
    let rows = [shell(7), shell(8)];
    let view = visible_live_rows(
        &rows,
        RailShellFilter {
            hide_shells: true,
            active_tab: None,
            fallback_row: Some(2),
        },
    );

    assert!(view.visible.is_empty());
    assert_eq!(view.hidden_shells, 2);
}

/// An unselected, agent-less project collapses to its header — that is the
/// noise the setting exists to remove.
#[test]
fn an_unselected_project_can_hide_every_row() {
    let rows = [shell(0), shell(1)];
    let view = visible_live_rows(
        &rows,
        RailShellFilter {
            hide_shells: true,
            active_tab: Some(9),
            fallback_row: None,
        },
    );

    assert!(view.visible.is_empty());
    assert_eq!(view.hidden_shells, 2);
}

/// Rows only ever leave, never move: the rail's spatial memory has to survive
/// toggling the setting.
#[test]
fn row_order_is_preserved() {
    let rows = [agent(9), shell(3), agent(1)];
    let view = visible_live_rows(
        &rows,
        RailShellFilter {
            hide_shells: true,
            active_tab: None,
            fallback_row: None,
        },
    );

    assert_eq!(view.visible, vec![9, 1]);
}

/// The summary row is a sentence, not a counter, and it is absent entirely
/// when nothing was hidden.
#[test]
fn the_summary_row_reads_as_a_sentence() {
    assert_eq!(hidden_shells_label(0), None);
    assert_eq!(hidden_shells_label(1).as_deref(), Some("1 shell"));
    assert_eq!(hidden_shells_label(4).as_deref(), Some("4 shells"));
}
