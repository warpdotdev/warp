use crate::{
    cloud_object::model::generic_string_model::GenericStringObjectId,
    server::ids::{ClientId, HashableId, ServerId, SyncId},
    workflows::workflow::{Argument, ArgumentType, Workflow},
};

#[test]
fn test_workflow_serialization_with_enum_params() {
    let workflow = Workflow::Command {
        name: "name".to_string(),
        command: "command".to_string(),
        arguments: vec![
            Argument {
                name: "text".to_string(),
                arg_type: ArgumentType::Text,
                description: None,
                default_value: Some("default".to_string()),
            },
            Argument {
                name: "server id enum".to_string(),
                arg_type: ArgumentType::Enum {
                    enum_id: SyncId::from(GenericStringObjectId::from(ServerId::from(123))),
                },
                description: Some("description".to_string()),
                default_value: None,
            },
            Argument {
                name: "client id enum".to_string(),
                arg_type: ArgumentType::Enum {
                    enum_id: SyncId::ClientId(
                        ClientId::from_hash("Client-06d26381-ac61-4a4a-8a23-a3431f1d340c")
                            .expect("should be able to construct ClientId from hash"),
                    ),
                },
                description: Some("description".to_string()),
                default_value: None,
            },
        ],
        description: None,
        source_url: None,
        author: None,
        author_url: None,
        shells: vec![],
        tags: vec![],
        environment_variables: None,
    };

    let serialized = serde_json::to_string(&workflow).expect("failed to serialize");
    let correct_serialized = r#"{"name":"name","command":"command","tags":[],"description":null,"arguments":[{"name":"text","arg_type":"Text","description":null,"default_value":"default"},{"name":"server id enum","arg_type":"Enum","enum_id":"test_uid00000000000123","description":"description","default_value":null},{"name":"client id enum","arg_type":"Enum","enum_id":"Client-06d26381-ac61-4a4a-8a23-a3431f1d340c","description":"description","default_value":null}],"source_url":null,"author":null,"author_url":null,"shells":[],"environment_variables":null}"#;

    assert_eq!(
        serialized, correct_serialized,
        "Workflow should serialize correctly"
    );

    let deserialized: Workflow =
        serde_json::from_str(serialized.as_str()).expect("failed to deserialized");

    assert_eq!(deserialized, workflow);
}

#[test]
fn test_agent_mode_workflow_serialization() {
    let workflow = Workflow::AgentMode {
        name: "name".to_string(),
        query: "query {{text}}".to_string(),
        arguments: vec![Argument {
            name: "text".to_string(),
            arg_type: ArgumentType::Text,
            description: None,
            default_value: Some("default".to_string()),
        }],
        description: None,
    };

    let serialized = serde_json::to_string(&workflow).expect("failed to serialize");
    let correct_serialized = r#"{"type":"agent_mode","name":"name","query":"query {{text}}","arguments":[{"name":"text","arg_type":"Text","description":null,"default_value":"default"}]}"#;

    assert_eq!(
        serialized, correct_serialized,
        "Workflow should serialize correctly"
    );

    let deserialized: Workflow =
        serde_json::from_str(serialized.as_str()).expect("failed to deserialized");

    assert_eq!(deserialized, workflow);
}
