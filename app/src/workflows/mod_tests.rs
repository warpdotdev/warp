use warpui::App;

use crate::server::ids::SyncId;

use super::workflow::{Argument, Workflow};

#[test]
fn test_serialize_cloud_workflow() {
    App::test((), |_app| async move {
        let sample_workflow = Workflow::new("Test name", "Command name");
        assert_eq!(
            serde_json::from_str::<Workflow>(
                serde_json::to_string(&sample_workflow)
                    .expect("Serialized workflow.")
                    .as_str()
            )
            .expect("Deserialized workflow."),
            sample_workflow
        );

        let arguments = vec![Argument {
            name: "Argument".to_string(),
            description: Some("no".to_string()),
            default_value: None,
            arg_type: Default::default(),
        }];
        let arguments_workflow = sample_workflow.clone().with_arguments(arguments);
        assert_eq!(
            serde_json::from_str::<Workflow>(
                serde_json::to_string(&arguments_workflow)
                    .expect("Serialized workflow.")
                    .as_str()
            )
            .expect("Deserialized workflow."),
            arguments_workflow
        );

        let description_workflow = sample_workflow.with_description("cool description".to_string());
        assert_eq!(
            serde_json::from_str::<Workflow>(
                serde_json::to_string(&description_workflow)
                    .expect("Serialized workflow.")
                    .as_str()
            )
            .expect("Deserialized workflow."),
            description_workflow
        );

        let workflow_with_additional_fields = Workflow::Command {
            name: "Test".to_string(),
            command: "Command".to_string(),
            tags: vec![],
            description: None,
            arguments: vec![],
            source_url: Some("url".to_string()),
            author: Some("author_name".to_string()),
            author_url: None,
            shells: vec![],
            environment_variables: Some(SyncId::ServerId(123.into())),
        };
        assert_eq!(
            serde_json::from_str::<Workflow>(
                serde_json::to_string(&workflow_with_additional_fields)
                    .expect("Serialized workflow.")
                    .as_str()
            )
            .expect("Deserialized workflow."),
            workflow_with_additional_fields
        );
    });
}
