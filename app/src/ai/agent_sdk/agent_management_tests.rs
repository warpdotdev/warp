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
        prompt: None,
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
            "Prompt",
        ]
    );
    assert_eq!(row.len(), header.len());
}

#[test]
fn table_format_truncates_prompt_to_sixty_characters() {
    let mut agent = agent("1", "agent", 1);
    agent.prompt = Some("a".repeat(PROMPT_DISPLAY_MAX_CHARS + 1));

    let prompt = agent
        .row()
        .last()
        .expect("prompt cell")
        .content()
        .to_string();

    assert_eq!(prompt.chars().count(), PROMPT_DISPLAY_MAX_CHARS);
    assert_eq!(
        prompt,
        format!("{}…", "a".repeat(PROMPT_DISPLAY_MAX_CHARS - 1))
    );
}

#[test]
fn table_format_preserves_short_prompt_and_flattens_newlines() {
    let mut agent = agent("1", "agent", 1);
    agent.prompt = Some("first line\nsecond line".to_string());

    let prompt = agent
        .row()
        .last()
        .expect("prompt cell")
        .content()
        .to_string();

    assert_eq!(prompt, "first line second line");
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

fn create_args(name: &str) -> AgentCreateArgs {
    AgentCreateArgs {
        name: name.to_string(),
        description: None,
        prompt: None,
        secrets: vec![],
        skills: vec![],
        base_model: None,
        environment: None,
        json_output: JsonOutput { filter: None },
    }
}

fn update_args(uid: &str) -> AgentUpdateArgs {
    AgentUpdateArgs {
        uid: uid.to_string(),
        name: None,
        description: None,
        remove_description: false,
        add_secrets: vec![],
        remove_secrets: vec![],
        remove_all_secrets: false,
        add_skills: vec![],
        remove_skills: vec![],
        remove_all_skills: false,
        base_model: None,
        remove_base_model: false,
        environment: None,
        remove_environment: false,
        prompt: None,
        remove_prompt: false,
        json_output: JsonOutput { filter: None },
    }
}

#[test]
fn build_create_request_forwards_prompt() {
    let args = AgentCreateArgs {
        prompt: Some("base prompt".to_string()),
        ..create_args("agent")
    };
    let request = build_create_request(args);
    assert_eq!(request.name, "agent");
    assert_eq!(request.prompt.as_deref(), Some("base prompt"));
}

#[test]
fn build_create_request_omits_prompt_when_unset() {
    let request = build_create_request(create_args("agent"));
    assert!(request.prompt.is_none());
}

#[test]
fn build_update_request_replaces_prompt() {
    let args = AgentUpdateArgs {
        prompt: Some("new prompt".to_string()),
        ..update_args("uid")
    };
    let request = build_update_request(args, None);
    assert_eq!(request.prompt.as_deref(), Some("new prompt"));
}

#[test]
fn build_update_request_remove_prompt_clears_via_empty_string() {
    let args = AgentUpdateArgs {
        remove_prompt: true,
        ..update_args("uid")
    };
    let request = build_update_request(args, None);
    // The public API's PATCH clear-via-empty semantics: an empty string clears.
    assert_eq!(request.prompt, Some(String::new()));
}

#[test]
fn build_update_request_leaves_prompt_unchanged_when_neither_flag_set() {
    let request = build_update_request(update_args("uid"), None);
    assert!(request.prompt.is_none());
}

#[test]
fn request_is_empty_treats_prompt_as_an_update() {
    let request = UpdateAgentRequest {
        prompt: Some("new prompt".to_string()),
        ..Default::default()
    };
    assert!(!request_is_empty(&request));
}

#[test]
fn request_is_empty_clears_prompt_still_counts_as_an_update() {
    let request = UpdateAgentRequest {
        prompt: Some(String::new()),
        ..Default::default()
    };
    assert!(!request_is_empty(&request));
}

#[test]
fn create_agent_request_omits_prompt_when_none_and_serializes_when_set() {
    let none = build_create_request(create_args("agent"));
    let none_json = serde_json::to_value(&none).expect("request serializes");
    assert!(
        none_json.get("prompt").is_none(),
        "unset prompt must be omitted"
    );

    let set = build_create_request(AgentCreateArgs {
        prompt: Some("base prompt".to_string()),
        ..create_args("agent")
    });
    let set_json = serde_json::to_value(&set).expect("request serializes");
    assert_eq!(set_json["prompt"], serde_json::json!("base prompt"));
}

#[test]
fn update_request_serializes_prompt_clear_as_empty_string_and_omits_none() {
    let none = UpdateAgentRequest {
        prompt: None,
        ..Default::default()
    };
    let none_json = serde_json::to_value(&none).expect("serializes");
    assert!(
        none_json.get("prompt").is_none(),
        "unset prompt must be omitted"
    );

    let clear = UpdateAgentRequest {
        prompt: Some(String::new()),
        ..Default::default()
    };
    let clear_json = serde_json::to_value(&clear).expect("serializes");
    assert_eq!(clear_json["prompt"], serde_json::json!(""));
}

fn agent_response_json() -> serde_json::Value {
    serde_json::json!({
        "uid": "1",
        "name": "agent",
        "description": null,
        "available": true,
        "created_at": "2024-01-01T00:00:00Z",
        "secrets": [],
        "skills": [],
        "base_model": null,
        "environment_id": "",
    })
}

#[test]
fn agent_response_deserializes_prompt() {
    let mut json = agent_response_json();
    json["prompt"] = serde_json::json!("base prompt");
    let response: AgentResponse = serde_json::from_value(json).expect("response deserializes");
    assert_eq!(response.prompt.as_deref(), Some("base prompt"));
}

#[test]
fn agent_response_defaults_prompt_to_none_when_absent() {
    let response: AgentResponse =
        serde_json::from_value(agent_response_json()).expect("response deserializes");
    assert!(response.prompt.is_none());
}

#[test]
fn agent_response_deserializes_null_prompt_as_none() {
    let mut json = agent_response_json();
    json["prompt"] = serde_json::Value::Null;
    let response: AgentResponse = serde_json::from_value(json).expect("response deserializes");
    assert!(response.prompt.is_none());
}
