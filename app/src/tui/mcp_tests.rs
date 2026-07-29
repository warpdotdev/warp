use uuid::Uuid;

use super::{
    TuiMcpFileScope, TuiMcpFileSource, TuiMcpServerId, TuiMcpServerSnapshot, TuiMcpServerSource,
    TuiMcpServerStatus, TuiMcpTemplateVariable, TuiMcpVariableValue, server_priority, sort_servers,
    validate_variable_values,
};

fn server(id: TuiMcpServerId, name: &str, status: TuiMcpServerStatus) -> TuiMcpServerSnapshot {
    TuiMcpServerSnapshot {
        id,
        installation_uuid: None,
        name: name.to_owned(),
        description: None,
        source: TuiMcpServerSource::SyncedTemplate,
        transport: None,
        status,
        tool_count: 0,
        resource_count: 0,
        can_log_out: false,
        authorization_url: None,
    }
}

#[test]
fn source_aware_ids_do_not_alias() {
    let uuid = Uuid::from_u128(7);
    let ids = [
        TuiMcpServerId::FileBased(7),
        TuiMcpServerId::Installation(uuid),
        TuiMcpServerId::SyncedTemplate(uuid),
        TuiMcpServerId::Gallery(uuid),
    ];

    for (index, id) in ids.iter().enumerate() {
        assert!(!ids[index + 1..].contains(id));
    }
}

#[test]
fn installed_rows_sort_before_available_rows_then_by_name_and_id() {
    let mut servers = vec![
        server(
            TuiMcpServerId::Gallery(Uuid::from_u128(1)),
            "Alpha",
            TuiMcpServerStatus::Available,
        ),
        server(
            TuiMcpServerId::Installation(Uuid::from_u128(3)),
            "Zulu",
            TuiMcpServerStatus::Running,
        ),
        server(
            TuiMcpServerId::FileBased(2),
            "alpha",
            TuiMcpServerStatus::Offline,
        ),
    ];

    sort_servers(&mut servers);

    assert_eq!(
        servers.iter().map(|server| server.id).collect::<Vec<_>>(),
        vec![
            TuiMcpServerId::FileBased(2),
            TuiMcpServerId::Installation(Uuid::from_u128(3)),
            TuiMcpServerId::Gallery(Uuid::from_u128(1)),
        ]
    );
    assert_eq!(server_priority(&servers[0]), 0);
    assert_eq!(server_priority(&servers[2]), 1);
}

#[test]
fn file_sources_keep_distinct_provider_scope_and_root_provenance() {
    let sources = vec![
        TuiMcpFileSource {
            provider: "Claude".to_owned(),
            root_path: "/repo".into(),
            scope: TuiMcpFileScope::Project,
        },
        TuiMcpFileSource {
            provider: "Claude".to_owned(),
            root_path: "/home/user".into(),
            scope: TuiMcpFileScope::Global,
        },
    ];

    let source = TuiMcpServerSource::FileBased {
        sources: sources.clone(),
    };

    assert_eq!(
        source,
        TuiMcpServerSource::FileBased { sources },
        "same-named definitions retain every source"
    );
}

#[test]
fn variable_validation_requires_exact_keys_and_allowed_values() {
    let variables = vec![
        TuiMcpTemplateVariable {
            key: "TOKEN".to_owned(),
            allowed_values: None,
        },
        TuiMcpTemplateVariable {
            key: "REGION".to_owned(),
            allowed_values: Some(vec!["us".to_owned(), "eu".to_owned()]),
        },
    ];

    let values = validate_variable_values(
        &variables,
        vec![
            TuiMcpVariableValue {
                key: "TOKEN".to_owned(),
                value: "secret".to_owned(),
            },
            TuiMcpVariableValue {
                key: "REGION".to_owned(),
                value: "eu".to_owned(),
            },
        ],
    )
    .expect("valid values should resolve");
    assert_eq!(values.len(), 2);

    assert!(
        validate_variable_values(
            &variables,
            vec![
                TuiMcpVariableValue {
                    key: "TOKEN".to_owned(),
                    value: "secret".to_owned(),
                },
                TuiMcpVariableValue {
                    key: "REGION".to_owned(),
                    value: "invalid".to_owned(),
                },
            ],
        )
        .is_err()
    );
}

#[test]
fn zero_variable_confirmation_accepts_no_values() {
    assert!(validate_variable_values(&[], Vec::new()).is_ok());
}
