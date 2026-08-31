use super::*;

#[test]
fn history_search_ranking_v2_cargo_feature_bridges_to_the_flag() {
    // `history_search_ranking_v2` is the compile-time on-ramp for eventually promoting
    // `FeatureFlag::HistorySearchRankingV2` to Stable (see the `promote-feature` skill):
    // `enabled_features()` must include the flag exactly when the Cargo feature is compiled in.
    // Run once with the feature off (the default) and once with `--features
    // history_search_ranking_v2` to exercise both branches.
    assert_eq!(
        enabled_features().contains(&FeatureFlag::HistorySearchRankingV2),
        cfg!(feature = "history_search_ranking_v2"),
    );
}
