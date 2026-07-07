use super::FuzzyMatchWorkflowResult;
use crate::workflows::workflow::Workflow;

#[test]
fn test_exact_command_before_exact_title() {
    // Regression test to ensure that exact command matches are surfaced above title/description
    // matches.

    let goose_workflow = Workflow::new("Run migration", "goose up")
        .with_description("Migrates the local database".to_string());

    let test_workflow = Workflow::new("Use goose up", "echo test");

    let query = "goose";
    let goose_match =
        FuzzyMatchWorkflowResult::try_match(query, &goose_workflow, "Some / Path").unwrap();
    let test_match =
        FuzzyMatchWorkflowResult::try_match(query, &test_workflow, "Some / Path").unwrap();

    assert!(goose_match.score() > test_match.score());
}

#[test]
fn test_exact_command_before_fuzzy_command() {
    let exact_workflow = Workflow::new("Update database", "echo working");
    let fuzzy_workflow = Workflow::new(
        "Do All The Appropriate Things",
        "echo fuzzy match for search",
    );

    let query = "data";

    let exact_match = FuzzyMatchWorkflowResult::try_match(query, &exact_workflow, "").unwrap();
    let fuzzy_match = FuzzyMatchWorkflowResult::try_match(query, &fuzzy_workflow, "").unwrap();

    // Both match in the name, but not the command.
    assert!(exact_match.score() > fuzzy_match.score());
}

#[ignore = "Weighting doesn't prioritize this case well"]
#[test]
fn test_exact_title_before_fuzzy_command() {
    // Even though the command is weighted more, an exact title match _should_ score higher.
    let exact_workflow = Workflow::new("Build code", "cargo package");
    let fuzzy_workflow = Workflow::new("JavaScript Workflow", "BUndle IsLanD");

    let query = "build";
    let exact_match = FuzzyMatchWorkflowResult::try_match(query, &exact_workflow, "").unwrap();
    let fuzzy_match = FuzzyMatchWorkflowResult::try_match(query, &fuzzy_workflow, "").unwrap();

    println!(
        "exact score = {}, fuzzy score = {}",
        exact_match.score(),
        fuzzy_match.score()
    );
    assert!(exact_match.score() > fuzzy_match.score());
}

#[test]
fn test_title_tiebreak() {
    let workflow1 = Workflow::new("Run tests", "cargo test");
    let workflow2 = Workflow::new("Something else", "cargo test");

    let query = "test";
    let match1 = FuzzyMatchWorkflowResult::try_match(query, &workflow1, "Path").unwrap();
    let match2 = FuzzyMatchWorkflowResult::try_match(query, &workflow2, "Path").unwrap();

    // Both have the same command, so the title should cause workflow1 to score higher.
    assert!(match1.score() > match2.score());
}
