use crate::ai::agent::{
    SuggestedAgentModeWorkflow, SuggestedLoggingId, SuggestedRule, Suggestions,
};

#[test]
fn test_extend_suggestions() {
    // Create base suggestions
    let mut base_suggestions = Suggestions {
        rules: vec![
            SuggestedRule {
                name: "rule1".into(),
                content: "content1".into(),
                logging_id: SuggestedLoggingId::from("id1".to_string()),
            },
            SuggestedRule {
                name: "rule2".into(),
                content: "content2".into(),
                logging_id: SuggestedLoggingId::from("id2".to_string()),
            },
        ],
        agent_mode_workflows: vec![
            SuggestedAgentModeWorkflow {
                name: "workflow1".into(),
                prompt: "prompt1".into(),
                logging_id: SuggestedLoggingId::from("wid1".to_string()),
            },
            SuggestedAgentModeWorkflow {
                name: "workflow2".into(),
                prompt: "prompt2".into(),
                logging_id: SuggestedLoggingId::from("wid2".to_string()),
            },
        ],
    };

    // Create additional suggestions with both unique and duplicate logging_ids
    let additional_suggestions = Suggestions {
        rules: vec![
            // Duplicate logging_id but different name/content
            SuggestedRule {
                name: "rule1_modified".into(),
                content: "content1_modified".into(),
                logging_id: SuggestedLoggingId::from("id1".to_string()),
            },
            // New unique rule
            SuggestedRule {
                name: "rule3".into(),
                content: "content3".into(),
                logging_id: SuggestedLoggingId::from("id3".to_string()),
            },
            // Another new unique rule
            SuggestedRule {
                name: "rule4".into(),
                content: "content4".into(),
                logging_id: SuggestedLoggingId::from("id4".to_string()),
            },
        ],
        agent_mode_workflows: vec![
            // Duplicate workflow logging_id but different name/prompt
            SuggestedAgentModeWorkflow {
                name: "workflow1_modified".into(),
                prompt: "prompt1_modified".into(),
                logging_id: SuggestedLoggingId::from("wid1".to_string()),
            },
            // New unique workflow
            SuggestedAgentModeWorkflow {
                name: "workflow3".into(),
                prompt: "prompt3".into(),
                logging_id: SuggestedLoggingId::from("wid3".to_string()),
            },
            // Another new unique workflow
            SuggestedAgentModeWorkflow {
                name: "workflow4".into(),
                prompt: "prompt4".into(),
                logging_id: SuggestedLoggingId::from("wid4".to_string()),
            },
        ],
    };

    // Extend base suggestions with additional ones
    base_suggestions.extend(&additional_suggestions);

    // Verify rules

    // Verify the length (should be 4 because one was a duplicate)
    assert_eq!(base_suggestions.rules.len(), 4);

    // Verify that original rules with id1 and id2 are still present and unchanged
    assert!(base_suggestions
        .rules
        .iter()
        .any(|r| r.logging_id.to_string() == "id1"
            && r.name == "rule1"
            && r.content == "content1"));
    assert!(base_suggestions
        .rules
        .iter()
        .any(|r| r.logging_id.to_string() == "id2"
            && r.name == "rule2"
            && r.content == "content2"));

    // Verify that new unique rules (id3 and id4) were added
    assert!(base_suggestions
        .rules
        .iter()
        .any(|r| r.logging_id.to_string() == "id3"
            && r.name == "rule3"
            && r.content == "content3"));
    assert!(base_suggestions
        .rules
        .iter()
        .any(|r| r.logging_id.to_string() == "id4"
            && r.name == "rule4"
            && r.content == "content4"));

    // Verify that the modified version of id1 was not added (deduplication worked)
    assert!(!base_suggestions
        .rules
        .iter()
        .any(|r| r.logging_id.to_string() == "id1" && r.name == "rule1_modified"));

    // Verify workflows

    // Verify the length (should be 4 because one was a duplicate)
    assert_eq!(base_suggestions.agent_mode_workflows.len(), 4);

    // Verify that original workflows with wid1 and wid2 are still present and unchanged
    assert!(base_suggestions
        .agent_mode_workflows
        .iter()
        .any(|w| w.logging_id.to_string() == "wid1"
            && w.name == "workflow1"
            && w.prompt == "prompt1"));
    assert!(base_suggestions
        .agent_mode_workflows
        .iter()
        .any(|w| w.logging_id.to_string() == "wid2"
            && w.name == "workflow2"
            && w.prompt == "prompt2"));

    // Verify that new unique workflows (wid3 and wid4) were added
    assert!(base_suggestions
        .agent_mode_workflows
        .iter()
        .any(|w| w.logging_id.to_string() == "wid3"
            && w.name == "workflow3"
            && w.prompt == "prompt3"));
    assert!(base_suggestions
        .agent_mode_workflows
        .iter()
        .any(|w| w.logging_id.to_string() == "wid4"
            && w.name == "workflow4"
            && w.prompt == "prompt4"));

    // Verify that the modified version of wid1 was not added (deduplication worked)
    assert!(!base_suggestions
        .agent_mode_workflows
        .iter()
        .any(|w| w.logging_id.to_string() == "wid1" && w.name == "workflow1_modified"));
}
