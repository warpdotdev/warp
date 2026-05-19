use chrono::{TimeZone as _, Utc};

use super::*;

fn agent(uid: &str, name: &str, created_at_seconds: i64) -> AgentResponse {
    agent_with_available(uid, name, created_at_seconds, true)
}

fn agent_with_available(
    uid: &str,
    name: &str,
    created_at_seconds: i64,
    available: bool,
) -> AgentResponse {
    AgentResponse {
        uid: uid.to_string(),
        name: name.to_string(),
        description: None,
        available,
        created_at: Utc
            .timestamp_opt(created_at_seconds, 0)
            .single()
            .expect("valid timestamp"),
        secrets: vec![],
        skills: vec![],
        base_model: None,
        environment_id: None,
    }
}

#[test]
fn table_format_does_not_include_available_column() {
    let header = AgentResponse::header()
        .into_iter()
        .map(|cell| cell.content().to_string())
        .collect::<Vec<_>>();
    let row = agent("1", "agent", 1).row();

    assert_eq!(
        header,
        [
            "UID",
            "Name",
            "Created",
            "Description",
            "Secrets",
            "Skills",
            "Base model",
            "Environment",
        ]
    );
    assert_eq!(row.len(), header.len());
}

#[test]
fn visible_agents_and_hidden_count_filters_disabled_agents() {
    let agents = vec![
        agent_with_available("1", "enabled", 1, true),
        agent_with_available("2", "disabled", 2, false),
    ];

    let (visible_agents, hidden_count) = visible_agents_and_hidden_count(&agents);

    assert_eq!(visible_agents.len(), 1);
    assert_eq!(visible_agents[0].name, "enabled");
    assert_eq!(hidden_count, 1);
}
#[test]
fn sort_agents_defaults_to_name_ascending() {
    let mut agents = vec![agent("2", "zeta", 2), agent("1", "alpha", 1)];

    sort_agents(&mut agents, None, None);

    assert_eq!(agents[0].name, "alpha");
    assert_eq!(agents[1].name, "zeta");
}

#[test]
fn sort_agents_defaults_created_at_to_descending() {
    let mut agents = vec![agent("1", "old", 1), agent("2", "new", 2)];

    sort_agents(&mut agents, Some(AgentSortByArg::CreatedAt), None);

    assert_eq!(agents[0].name, "new");
    assert_eq!(agents[1].name, "old");
}

#[test]
fn sort_agents_respects_explicit_sort_order_without_sort_field() {
    let mut agents = vec![agent("1", "alpha", 1), agent("2", "zeta", 2)];

    sort_agents(&mut agents, None, Some(SortOrderArg::Desc));

    assert_eq!(agents[0].name, "zeta");
    assert_eq!(agents[1].name, "alpha");
}

#[test]
fn update_request_omits_unset_fields_and_serializes_clears() {
    let request = UpdateAgentRequest {
        description: Some(String::new()),
        secrets: Some(vec![]),
        base_model: Some(String::new()),
        ..Default::default()
    };

    let json = serde_json::to_value(request).expect("request serializes");

    assert_eq!(
        json,
        serde_json::json!({
            "description": "",
            "secrets": [],
            "base_model": "",
        })
    );
}

#[test]
fn rejects_sort_for_json_output() {
    let args = AgentListArgs {
        sort_by: Some(AgentSortByArg::Name),
        sort_order: None,
        json_output: JsonOutput { filter: None },
    };

    let err = ensure_json_sort_is_not_requested(OutputFormat::Json, &args.json_output, &args)
        .unwrap_err();

    assert!(err.to_string().contains("not supported with JSON output"));
}

#[test]
fn apply_string_deltas_removes_and_appends_without_duplicates() {
    let values = apply_string_deltas(
        &["old".to_string(), "keep".to_string()],
        vec!["new".to_string(), "keep".to_string()],
        vec!["old".to_string()],
    );

    assert_eq!(values, ["keep", "new"]);
}

#[test]
fn apply_secret_deltas_uses_secret_names() {
    let values = apply_secret_deltas(
        &[
            SecretRef {
                name: "OLD_TOKEN".to_string(),
            },
            SecretRef {
                name: "KEEP_TOKEN".to_string(),
            },
        ],
        vec!["NEW_TOKEN".to_string()],
        vec!["OLD_TOKEN".to_string()],
    );

    assert_eq!(
        values,
        [
            SecretRef {
                name: "KEEP_TOKEN".to_string()
            },
            SecretRef {
                name: "NEW_TOKEN".to_string()
            },
        ]
    );
}
