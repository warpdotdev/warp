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
