use super::*;

fn row(uid: i64, name: &str, is_active: bool) -> TuiTeamMenuRow {
    TuiTeamMenuRow {
        uid: uid.into(),
        title: name.to_owned(),
        is_active,
    }
}

#[test]
fn the_active_team_is_marked_and_every_team_stays_selectable() {
    let active = snapshot_row(&row(123, "Platform", true));
    assert_eq!(active.title, "Platform");
    assert_eq!(active.state_suffix.as_deref(), Some("(active)"));
    assert!(
        active.is_selectable,
        "re-selecting the active team should be a harmless no-op, not a disabled row"
    );

    let inactive = snapshot_row(&row(456, "Security", false));
    assert_eq!(inactive.state_suffix, None);
    assert!(inactive.is_selectable);
}

#[test]
fn an_empty_query_preselects_the_active_team() {
    let rows = [
        row(123, "Platform", false),
        row(456, "Security", true),
        row(789, "Growth", false),
    ];

    assert_eq!(preferred_row_index(&rows), Some(1));
}

/// Searching filters the active team out, and the selection has to move with it. Without a
/// fallback the list is left with nothing selected, so typing a name and pressing enter --
/// the whole point of a searchable picker -- silently does nothing.
#[test]
fn filtering_out_the_active_team_still_preselects_a_row() {
    let rows = [row(123, "Platform", false), row(789, "Growth", false)];

    assert_eq!(preferred_row_index(&rows), Some(0));
}

#[test]
fn no_rows_preselects_nothing() {
    assert_eq!(preferred_row_index(&[]), None);
}
