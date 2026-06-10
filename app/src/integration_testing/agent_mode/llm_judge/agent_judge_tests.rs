use super::*;

static TEST_RUBRIC: RubricSpec = RubricSpec {
    name: "test_rubric",
    items: &[
        RubricSpecItem {
            id: "A-one",
            description: "First behavior.",
            spec_section: "TECH.md §1",
        },
        RubricSpecItem {
            id: "B-two",
            description: "Second behavior.",
            spec_section: "TECH.md §2",
        },
    ],
};

fn result_json(score_one: &str, score_two: &str) -> String {
    format!(
        r#"{{
            "items": [
                {{"id": "A-one", "score": "{score_one}", "evidence": "src/a.rs:1..10"}},
                {{"id": "B-two", "score": "{score_two}", "evidence": "src/b.rs:5..20"}}
            ],
            "abstract_dimensions": {{"completeness": 4, "correctness": 5, "scope_discipline": 3}},
            "overall_critique": "Looks good."
        }}"#
    )
}

#[test]
fn parses_fenced_json_with_language_tag() {
    let message = format!(
        "Here is my verdict:\n```json\n{}\n```\nDone.",
        result_json("pass", "not_implemented")
    );
    let result = parse_agent_judge_result(&message).unwrap();
    assert_eq!(result.items.len(), 2);
    assert_eq!(result.items[0].score, RubricScore::Pass);
    assert_eq!(result.items[1].score, RubricScore::NotImplemented);
    assert_eq!(result.abstract_dimensions.completeness, 4);
    assert_eq!(result.overall_critique, "Looks good.");
}

#[test]
fn parses_last_valid_fenced_block() {
    let message = format!(
        "```\nnot json at all\n```\nsome text\n```json\n{}\n```",
        result_json("partial", "fail")
    );
    let result = parse_agent_judge_result(&message).unwrap();
    assert_eq!(result.items[0].score, RubricScore::Partial);
    assert_eq!(result.items[1].score, RubricScore::Fail);
}

#[test]
fn parses_bare_json_message() {
    let result = parse_agent_judge_result(&result_json("pass", "pass")).unwrap();
    assert_eq!(result.items.len(), 2);
}

#[test]
fn missing_json_is_an_error() {
    assert!(parse_agent_judge_result("no json here").is_err());
    assert!(parse_agent_judge_result("```json\n{\"items\": 7}\n```").is_err());
}

#[test]
fn all_pass_gate_passes_when_every_item_passes() {
    let result = parse_agent_judge_result(&result_json("pass", "pass")).unwrap();
    let gate =
        evaluate_rubric_result(&TEST_RUBRIC, RubricExpectations::all_pass(), &result).unwrap();
    assert!(gate.overall_pass);
    assert!(gate.failures.is_empty());
}

#[test]
fn all_pass_gate_fails_on_non_pass_score() {
    let result = parse_agent_judge_result(&result_json("pass", "partial")).unwrap();
    let gate =
        evaluate_rubric_result(&TEST_RUBRIC, RubricExpectations::all_pass(), &result).unwrap();
    assert!(!gate.overall_pass);
    assert_eq!(gate.failures.len(), 1);
    assert!(gate.failures[0].contains("B-two"));
}

#[test]
fn expectation_overrides_apply_per_item() {
    let result = parse_agent_judge_result(&result_json("pass", "fail")).unwrap();
    let expectations = RubricExpectations {
        default_score: Some(RubricScore::Pass),
        overrides: &[("B-two", RubricScore::Fail)],
    };
    let gate = evaluate_rubric_result(&TEST_RUBRIC, expectations, &result).unwrap();
    assert!(gate.overall_pass);
}

#[test]
fn unconstrained_default_leaves_items_ungated() {
    let result = parse_agent_judge_result(&result_json("pass", "fail")).unwrap();
    let expectations = RubricExpectations {
        default_score: None,
        overrides: &[("A-one", RubricScore::Pass)],
    };
    let gate = evaluate_rubric_result(&TEST_RUBRIC, expectations, &result).unwrap();
    assert!(gate.overall_pass);
}

#[test]
fn missing_rubric_id_is_an_error() {
    let message = r#"{
        "items": [{"id": "A-one", "score": "pass", "evidence": ""}],
        "abstract_dimensions": {"completeness": 1, "correctness": 1, "scope_discipline": 1}
    }"#;
    let result = parse_agent_judge_result(message).unwrap();
    let err =
        evaluate_rubric_result(&TEST_RUBRIC, RubricExpectations::all_pass(), &result).unwrap_err();
    assert!(err.contains("B-two"));
}

#[test]
fn duplicate_rubric_id_is_an_error() {
    let message = r#"{
        "items": [
            {"id": "A-one", "score": "pass", "evidence": ""},
            {"id": "A-one", "score": "fail", "evidence": ""},
            {"id": "B-two", "score": "pass", "evidence": ""}
        ],
        "abstract_dimensions": {"completeness": 1, "correctness": 1, "scope_discipline": 1}
    }"#;
    let result = parse_agent_judge_result(message).unwrap();
    let err =
        evaluate_rubric_result(&TEST_RUBRIC, RubricExpectations::all_pass(), &result).unwrap_err();
    assert!(err.contains("more than once"));
}

#[test]
fn judge_prompt_renders_rubric_items_and_no_placeholders() {
    let prompt = judge_user_prompt(&TEST_RUBRIC);
    assert!(prompt.contains("1. A-one: First behavior. (TECH.md §1)"));
    assert!(prompt.contains("2. B-two: Second behavior. (TECH.md §2)"));
    assert!(!prompt.contains("{rubric_items}"));
    assert!(!prompt.contains("{spec_excerpt}"));
}
