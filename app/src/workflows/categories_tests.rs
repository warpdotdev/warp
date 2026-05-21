use crate::workflows::categories::{CategoriesView, WorkflowMatchType};
use crate::workflows::workflow::Workflow;
use crate::workflows::WorkflowType;
use std::sync::Arc;

#[test]
fn test_workflow_matches() {
    let workflow = Arc::new(WorkflowType::Local(Workflow::Command {
        name: "g workflow_name it ".into(),
        command: "command_name git".to_string(),
        tags: vec!["foo".into(), "bar".into()],
        description: None,
        arguments: vec![],
        source_url: None,
        author: None,
        author_url: None,
        shells: vec![],
        environment_variables: None,
    }));

    assert_eq!(
        CategoriesView::matches_workflow(&workflow, "foo"),
        WorkflowMatchType::Tag
    );
    assert_eq!(
        CategoriesView::matches_workflow(&workflow, "bar"),
        WorkflowMatchType::Tag
    );

    // The Workflow name has higher precedence than the command.
    assert!(matches!(
        CategoriesView::matches_workflow(&workflow, "name"),
        WorkflowMatchType::Name { .. }
    ));

    // Git matches both the name and the command, but fuzzy matches command with a higher score.
    assert!(matches!(
        CategoriesView::matches_workflow(&workflow, "git"),
        WorkflowMatchType::Command { .. }
    ));

    assert!(matches!(
        CategoriesView::matches_workflow(&workflow, "command"),
        WorkflowMatchType::Command { .. }
    ));

    assert!(matches!(
        CategoriesView::matches_workflow(&workflow, "command"),
        WorkflowMatchType::Command { .. }
    ));

    assert_eq!(
        CategoriesView::matches_workflow(&workflow, "gibberish"),
        WorkflowMatchType::Unmatched
    );
}
