use super::DiffSetSearchItem;
use crate::code_review::diff_state::DiffMode;
use crate::search::item::SearchItem;

#[test]
fn diffset_has_higher_priority_tier() {
    let match_result =
        fuzzy_match::match_indices_case_insensitive("uncommitted changes", "uncommitted")
            .expect("query should match");

    let item = DiffSetSearchItem {
        diff_mode: DiffMode::Head,
        match_result,
    };

    assert_eq!(item.priority_tier(), 1);
}
