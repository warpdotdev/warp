use chrono::TimeZone;

use super::*;

fn utc(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, 0, 0, 0).unwrap()
}

fn summary(start: DateTime<Utc>, end: DateTime<Utc>) -> BillingCycleUsageSummary {
    BillingCycleUsageSummary {
        period_start: start,
        period_end: end,
        entries: vec![],
    }
}

fn item_icon(item: &MenuItem<BillingCycleUsageAction>) -> Option<Icon> {
    match item {
        MenuItem::Item(fields) => fields.icon(),
        _ => None,
    }
}

fn selected_period_ends(items: &[MenuItem<BillingCycleUsageAction>]) -> Vec<DateTime<Utc>> {
    items
        .iter()
        .filter(|item| item_icon(item) == Some(Icon::Check))
        .filter_map(|item| match item.item_on_select_action() {
            Some(BillingCycleUsageAction::SelectPeriod(end)) => *end,
            _ => None,
        })
        .collect()
}

fn sample_summaries() -> Vec<BillingCycleUsageSummary> {
    // Newest cycle first, matching the server ordering the UI relies on.
    vec![
        summary(utc(2026, 6, 27), utc(2026, 7, 27)),
        summary(utc(2026, 5, 27), utc(2026, 6, 27)),
        summary(utc(2026, 4, 27), utc(2026, 5, 27)),
    ]
}

/// With no explicit selection, the most recent cycle (the one shown in the
/// header by default) is the one marked with a check.
#[test]
fn checks_most_recent_period_when_none_selected() {
    let summaries = sample_summaries();
    let items = build_period_menu_items(&summaries, None);

    assert_eq!(items.len(), summaries.len());
    assert_eq!(selected_period_ends(&items), vec![utc(2026, 7, 27)]);
    // Exactly one item is checked; the rest are indented (no icon).
    assert_eq!(item_icon(&items[0]), Some(Icon::Check));
    assert_eq!(item_icon(&items[1]), None);
    assert_eq!(item_icon(&items[2]), None);
}

/// Selecting an older period moves the check to that period only.
#[test]
fn checks_explicitly_selected_period() {
    let summaries = sample_summaries();
    let items = build_period_menu_items(&summaries, Some(utc(2026, 6, 27)));

    assert_eq!(selected_period_ends(&items), vec![utc(2026, 6, 27)]);
    assert_eq!(item_icon(&items[0]), None);
    assert_eq!(item_icon(&items[1]), Some(Icon::Check));
    assert_eq!(item_icon(&items[2]), None);
}

/// A `selected_period_end` that no longer exists in the data leaves nothing
/// checked rather than falsely highlighting an unrelated row.
#[test]
fn checks_nothing_when_selection_absent() {
    let summaries = sample_summaries();
    let items = build_period_menu_items(&summaries, Some(utc(1999, 1, 1)));

    assert!(selected_period_ends(&items).is_empty());
    assert!(items.iter().all(|item| item_icon(item).is_none()));
}

/// Every row still carries its `SelectPeriod` action for the matching cycle,
/// so highlighting doesn't change selection behavior.
#[test]
fn every_item_keeps_its_select_action() {
    let summaries = sample_summaries();
    let items = build_period_menu_items(&summaries, None);

    for (item, summary) in items.iter().zip(summaries.iter()) {
        match item.item_on_select_action() {
            Some(BillingCycleUsageAction::SelectPeriod(Some(end))) => {
                assert_eq!(*end, summary.period_end);
            }
            other => panic!("expected SelectPeriod action, got {other:?}"),
        }
    }
}
