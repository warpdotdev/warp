use super::fuzzy_match::SlashCommandFuzzyMatchResult;

#[test]
fn test_try_match_with_name_match() {
    let result = SlashCommandFuzzyMatchResult::try_match("test", "testing", None);

    assert!(result.is_some());
    let result = result.unwrap();
    assert!(result.name_match_result.is_some());
    assert!(result.description_match_result.is_none());
}

#[test]
fn test_try_match_with_description_match() {
    let result =
        SlashCommandFuzzyMatchResult::try_match("run", "build", Some("run the build process"));

    assert!(result.is_some());
    let result = result.unwrap();
    assert!(result.name_match_result.is_none());
    assert!(result.description_match_result.is_some());
}

#[test]
fn test_match_two_commands_with_same_name_but_different_description() {
    let result1 = SlashCommandFuzzyMatchResult::try_match("test", "test", Some("tests"));
    let result2 = SlashCommandFuzzyMatchResult::try_match("test", "test", Some("faketests"));
    assert!(result1.is_some());
    assert!(result2.is_some());
    let result1 = result1.unwrap();
    let result2 = result2.unwrap();
    assert!(result1.name_match_result.is_some());
    assert!(result2.name_match_result.is_some());
    assert!(result1.description_match_result.is_some());
    assert!(result2.description_match_result.is_some());
    assert!(result1.score() > result2.score());
}

#[test]
fn test_match_two_commands_with_different_name_and_same_description() {
    let result1 = SlashCommandFuzzyMatchResult::try_match("test", "test", Some("run tests"));
    let result2 = SlashCommandFuzzyMatchResult::try_match("test", "faketest", Some("run tests"));
    assert!(result1.is_some());
    assert!(result2.is_some());
    let result1 = result1.unwrap();
    let result2 = result2.unwrap();
    assert!(result1.name_match_result.is_some());
    assert!(result2.name_match_result.is_some());
    assert!(result1.description_match_result.is_some());
    assert!(result2.description_match_result.is_some());
    assert!(result1.score() > result2.score());
}

#[test]
fn test_score_returns_no_match_when_both_none() {
    let result = SlashCommandFuzzyMatchResult {
        name_match_result: None,
        description_match_result: None,
    };

    let best = result.score();
    assert_eq!(best, 0.0);
}

#[test]
fn test_case_insensitive_matching() {
    let result1 = SlashCommandFuzzyMatchResult::try_match("TEST", "test-command", None);
    let result2 = SlashCommandFuzzyMatchResult::try_match("test", "TEST-COMMAND", None);

    assert!(result1.is_some());
    assert!(result2.is_some());

    let result1 = result1.unwrap();
    let result2 = result2.unwrap();

    assert!(result1.name_match_result.is_some());
    assert!(result2.name_match_result.is_some());
}
