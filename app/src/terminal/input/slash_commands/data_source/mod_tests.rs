use super::commands::{DISABLE_NATURAL_LANGUAGE_DETECTION, ENABLE_NATURAL_LANGUAGE_DETECTION, MCP};
use super::{nld_toggle_command_is_visible, prefix_match_bonus};
use crate::search::slash_command_menu::fuzzy_match::SlashCommandFuzzyMatchResult;

#[test]
fn exact_match_returns_full_bonus() {
    // Query "new" exactly matches the name "/new" (after stripping '/').
    let bonus = prefix_match_bonus("new", "/new");
    assert!((bonus - 100.0).abs() < f64::EPSILON);
}

#[test]
fn partial_prefix_returns_proportional_bonus() {
    // "for" is a prefix of "fork" → coverage 3/4 = 75.
    let bonus = prefix_match_bonus("for", "/fork");
    assert!((bonus - 75.0).abs() < f64::EPSILON);
}

#[test]
fn non_prefix_returns_zero() {
    // "new" is NOT a prefix of "create-new-project".
    let bonus = prefix_match_bonus("new", "/create-new-project");
    assert!((bonus - 0.0).abs() < f64::EPSILON);
}

#[test]
fn case_insensitive() {
    let bonus = prefix_match_bonus("new", "/New");
    assert!((bonus - 100.0).abs() < f64::EPSILON);
}

#[test]
fn name_without_slash_prefix() {
    // Skills don't have the '/' prefix in their name.
    let bonus = prefix_match_bonus("figma", "figma-create-new-file");
    let coverage = 5.0 / 21.0 * 100.0;
    assert!((bonus - coverage).abs() < f64::EPSILON);
}

#[test]
fn short_prefix_match_ranks_above_longer_fuzzy_match() {
    // Simulates the reported issue: query "new" should give /new a much
    // higher combined score than /figma-create-new-file.
    let short_match = SlashCommandFuzzyMatchResult::try_match("new", "/new", None).unwrap();
    let long_match =
        SlashCommandFuzzyMatchResult::try_match("new", "/figma-create-new-file", None).unwrap();

    const SCORE_MULTIPLIER: f64 = 1000.0;

    let short_score = short_match.score() * SCORE_MULTIPLIER
        + prefix_match_bonus("new", "/new") * SCORE_MULTIPLIER
        + 1.0 / "/new".len() as f64;
    let long_score = long_match.score() * SCORE_MULTIPLIER
        + prefix_match_bonus("new", "/figma-create-new-file") * SCORE_MULTIPLIER
        + 1.0 / "/figma-create-new-file".len() as f64;

    assert!(
        short_score > long_score,
        "/new score ({short_score}) should be greater than /figma-create-new-file score ({long_score})"
    );
}

#[test]
fn nld_enable_command_only_visible_when_autodetection_is_off() {
    // When NLD is off, /enable is shown and /disable is hidden.
    assert!(nld_toggle_command_is_visible(
        ENABLE_NATURAL_LANGUAGE_DETECTION.name,
        false
    ));
    assert!(!nld_toggle_command_is_visible(
        DISABLE_NATURAL_LANGUAGE_DETECTION.name,
        false
    ));
}

#[test]
fn nld_disable_command_only_visible_when_autodetection_is_on() {
    // When NLD is on, /disable is shown and /enable is hidden.
    assert!(nld_toggle_command_is_visible(
        DISABLE_NATURAL_LANGUAGE_DETECTION.name,
        true
    ));
    assert!(!nld_toggle_command_is_visible(
        ENABLE_NATURAL_LANGUAGE_DETECTION.name,
        true
    ));
}

#[test]
fn nld_toggle_visibility_leaves_other_commands_unaffected() {
    // A non-NLD command is visible regardless of the autodetection state.
    assert!(nld_toggle_command_is_visible(MCP.name, false));
    assert!(nld_toggle_command_is_visible(MCP.name, true));
}
