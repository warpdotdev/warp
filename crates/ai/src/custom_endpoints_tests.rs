use indexmap::IndexMap;
use settings_value::SettingsValue;

use super::*;

fn model_file(name: &str, alias: Option<&str>) -> CustomEndpointModelDefinitionFile {
    CustomEndpointModelDefinitionFile {
        name: name.to_owned(),
        alias: alias.map(str::to_owned),
    }
}

fn endpoint_file(
    url: &str,
    models: Vec<CustomEndpointModelDefinitionFile>,
) -> CustomEndpointDefinitionFile {
    CustomEndpointDefinitionFile {
        url: url.to_owned(),
        schema: CustomEndpointSchema::OpenaiChatCompletions,
        models,
    }
}

type TestModelSpec<'a> = (&'a str, Option<&'a str>);
type TestEndpointSpec<'a> = (&'a str, &'a str, &'a [TestModelSpec<'a>]);

fn object_from_toml_like(
    entries: &[TestEndpointSpec<'_>],
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    for (name, url, models) in entries {
        let file = endpoint_file(
            url,
            models
                .iter()
                .map(|(name, alias)| model_file(name, *alias))
                .collect(),
        );
        map.insert(
            (*name).to_owned(),
            serde_json::to_value(file).expect("file value serializes"),
        );
    }
    map
}

// ── settings_custom_model_config_key ────────────────────────────

#[test]
fn config_key_is_deterministic() {
    let a = settings_custom_model_config_key("Acme Gateway", "gpt-4o");
    let b = settings_custom_model_config_key("Acme Gateway", "gpt-4o");
    assert_eq!(a, b);
    assert!(a.starts_with("custom-endpoint:v1:"));
}

#[test]
fn config_key_changes_with_endpoint_name() {
    let a = settings_custom_model_config_key("Acme Gateway", "gpt-4o");
    let b = settings_custom_model_config_key("Other Gateway", "gpt-4o");
    assert_ne!(a, b);
}

#[test]
fn config_key_changes_with_model_name() {
    let a = settings_custom_model_config_key("Acme Gateway", "gpt-4o");
    let b = settings_custom_model_config_key("Acme Gateway", "o3-mini");
    assert_ne!(a, b);
}

#[test]
fn config_key_is_tuple_boundary_safe() {
    // Without length-prefixing, ("ab", "c") and ("a", "bc") would hash the
    // same via naive concatenation. The length prefixes must prevent this.
    let a = settings_custom_model_config_key("ab", "c");
    let b = settings_custom_model_config_key("a", "bc");
    assert_ne!(a, b);
}

#[test]
fn config_key_stable_across_alias_url_schema_and_order_changes() {
    // Simulates editing everything except the endpoint/model names.
    let before = to_custom_endpoint(
        &validate_custom_endpoint_definition(
            "Acme Gateway",
            &endpoint_file(
                "https://a.example/v1",
                vec![
                    model_file("gpt-4o", Some("Fast")),
                    model_file("o3-mini", None),
                ],
            ),
        )
        .unwrap(),
        "key".to_owned(),
    );
    let after = to_custom_endpoint(
        &validate_custom_endpoint_definition(
            "Acme Gateway",
            &endpoint_file(
                "https://b.example/v2",
                // Order swapped, alias changed, schema irrelevant to config_key.
                vec![
                    model_file("o3-mini", None),
                    model_file("gpt-4o", Some("Renamed")),
                ],
            ),
        )
        .unwrap(),
        "different-key".to_owned(),
    );
    let before_gpt4o = before
        .models
        .iter()
        .find(|m| m.name == "gpt-4o")
        .unwrap()
        .config_key
        .clone();
    let after_gpt4o = after
        .models
        .iter()
        .find(|m| m.name == "gpt-4o")
        .unwrap()
        .config_key
        .clone();
    assert_eq!(before_gpt4o, after_gpt4o);
}

#[test]
fn config_key_changes_on_rename() {
    let renamed_endpoint = settings_custom_model_config_key("New Name", "gpt-4o");
    let original_endpoint = settings_custom_model_config_key("Acme Gateway", "gpt-4o");
    assert_ne!(renamed_endpoint, original_endpoint);

    let renamed_model = settings_custom_model_config_key("Acme Gateway", "gpt-4o-renamed");
    assert_ne!(renamed_model, original_endpoint);
}

// ── validate_custom_endpoint_url ─────────────────────────────────

#[test]
fn url_validation_rejects_non_https() {
    assert_eq!(
        validate_custom_endpoint_url("http://example.com"),
        Err(CustomEndpointUrlError::NotHttps)
    );
}

#[test]
fn url_validation_rejects_invalid_url() {
    assert_eq!(
        validate_custom_endpoint_url("not a url"),
        Err(CustomEndpointUrlError::Invalid)
    );
}

#[test]
fn url_validation_rejects_localhost() {
    assert_eq!(
        validate_custom_endpoint_url("https://localhost/v1"),
        Err(CustomEndpointUrlError::RestrictedHost)
    );
    assert_eq!(
        validate_custom_endpoint_url("https://LOCALHOST/v1"),
        Err(CustomEndpointUrlError::RestrictedHost)
    );
}

#[test]
fn url_validation_rejects_restricted_ipv4_literals() {
    for host in [
        "127.0.0.1",
        "0.0.0.0",
        "10.0.0.5",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.1.1",
    ] {
        assert_eq!(
            validate_custom_endpoint_url(&format!("https://{host}/v1")),
            Err(CustomEndpointUrlError::RestrictedHost),
            "expected {host} to be restricted"
        );
    }
}

#[test]
fn url_validation_rejects_restricted_ipv6_literals() {
    for host in ["[::1]", "[::]", "[fc00::1]", "[fe80::1]"] {
        assert_eq!(
            validate_custom_endpoint_url(&format!("https://{host}/v1")),
            Err(CustomEndpointUrlError::RestrictedHost),
            "expected {host} to be restricted"
        );
    }
    // IPv4-mapped loopback.
    assert_eq!(
        validate_custom_endpoint_url("https://[::ffff:127.0.0.1]/v1"),
        Err(CustomEndpointUrlError::RestrictedHost)
    );
}

#[test]
fn url_validation_accepts_valid_https_host() {
    assert_eq!(
        validate_custom_endpoint_url("https://llm.acme.example/v1"),
        Ok(())
    );
}

// ── validate_custom_endpoint_definition (fail-closed per endpoint) ──

#[test]
fn validation_accepts_product_spec_example() {
    let file = endpoint_file(
        "https://llm.acme.example/v1",
        vec![
            model_file("gpt-4o", Some("Acme GPT-4o")),
            model_file("o3-mini", None),
        ],
    );
    let definition = validate_custom_endpoint_definition("Acme Gateway", &file).unwrap();
    assert_eq!(definition.name, "Acme Gateway");
    assert_eq!(definition.models.len(), 2);
}

#[test]
fn validation_rejects_empty_name() {
    let file = endpoint_file("https://a.example", vec![model_file("m", None)]);
    assert!(validate_custom_endpoint_definition("", &file).is_err());
}

#[test]
fn validation_rejects_whitespace_padded_name() {
    let file = endpoint_file("https://a.example", vec![model_file("m", None)]);
    assert!(validate_custom_endpoint_definition(" Acme ", &file).is_err());
}

#[test]
fn validation_allows_name_with_internal_spaces() {
    let file = endpoint_file("https://a.example", vec![model_file("m", None)]);
    assert!(validate_custom_endpoint_definition("Acme Gateway", &file).is_ok());
}

#[test]
fn validation_rejects_empty_models_list() {
    let file = endpoint_file("https://a.example", vec![]);
    assert!(validate_custom_endpoint_definition("ep", &file).is_err());
}

#[test]
fn validation_rejects_duplicate_model_names_within_endpoint() {
    let file = endpoint_file(
        "https://a.example",
        vec![model_file("m", None), model_file("m", Some("alt"))],
    );
    assert!(validate_custom_endpoint_definition("ep", &file).is_err());
}

#[test]
fn validation_rejects_whitespace_padded_model_name() {
    let file = endpoint_file("https://a.example", vec![model_file(" m ", None)]);
    assert!(validate_custom_endpoint_definition("ep", &file).is_err());
}

#[test]
fn validation_rejects_whitespace_padded_alias() {
    let file = endpoint_file("https://a.example", vec![model_file("m", Some(" alias "))]);
    assert!(validate_custom_endpoint_definition("ep", &file).is_err());
}

#[test]
fn validation_accepts_empty_alias_falling_back_to_name() {
    let file = endpoint_file("https://a.example", vec![model_file("m", Some(""))]);
    assert!(validate_custom_endpoint_definition("ep", &file).is_ok());
}

#[test]
fn validation_rejects_restricted_url() {
    let file = endpoint_file("https://localhost/v1", vec![model_file("m", None)]);
    assert!(validate_custom_endpoint_definition("ep", &file).is_err());
}

#[test]
fn deny_unknown_fields_rejects_api_key_field() {
    let value = serde_json::json!({
        "url": "https://a.example",
        "api_key": "sk-should-not-be-here",
        "models": [{"name": "m"}],
    });
    let result: Result<CustomEndpointDefinitionFile, _> = serde_json::from_value(value);
    assert!(result.is_err());
}

#[test]
fn deny_unknown_fields_rejects_config_key_field() {
    let value = serde_json::json!({
        "name": "m",
        "config_key": "should-not-be-user-authored",
    });
    let result: Result<CustomEndpointModelDefinitionFile, _> = serde_json::from_value(value);
    assert!(result.is_err());
}

// ── CustomEndpointDefinitionsConfig::from_object (per-entry recovery) ──

#[test]
fn from_object_keeps_valid_and_diagnoses_invalid_independently() {
    let mut object = object_from_toml_like(&[("Good", "https://a.example", &[("m", None)])]);
    object.insert(
        "Bad".to_owned(),
        serde_json::json!({"url": "not-a-url", "models": [{"name": "m"}]}),
    );
    let config = CustomEndpointDefinitionsConfig::from_object(&object);
    assert_eq!(config.valid_len(), 1);
    assert!(config.get("Good").is_some());
    assert!(config.get("Bad").is_none());
    assert!(config.has_diagnostics());
    assert_eq!(config.invalid().count(), 1);
    assert_eq!(config.present_names().count(), 2);
}

#[test]
fn from_object_rejects_hand_authored_config_key_as_invalid_entry() {
    let mut object = serde_json::Map::new();
    object.insert(
        "ep".to_owned(),
        serde_json::json!({
            "url": "https://a.example",
            "models": [{"name": "m", "config_key": "hand-authored"}],
        }),
    );
    let config = CustomEndpointDefinitionsConfig::from_object(&object);
    assert_eq!(config.valid_len(), 0);
    assert_eq!(config.invalid().count(), 1);
}

#[test]
fn to_file_value_omits_invalid_entries_and_diagnostics() {
    let mut object = object_from_toml_like(&[("Good", "https://a.example", &[("m", None)])]);
    object.insert(
        "Bad".to_owned(),
        serde_json::json!({"url": "not-a-url", "models": []}),
    );
    let config = CustomEndpointDefinitionsConfig::from_object(&object);
    let file_value = config.to_file_value();
    let written = file_value.as_object().unwrap();
    assert_eq!(written.len(), 1);
    assert!(written.contains_key("Good"));
    assert!(!written.contains_key("Bad"));
}

#[test]
fn from_file_value_of_non_object_returns_none() {
    assert!(CustomEndpointDefinitionsConfig::from_file_value(&serde_json::json!("oops")).is_none());
}

// ── join_custom_endpoint_keys (settings+secret join and reconciliation) ──

fn keys(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, key)| ((*name).to_owned(), (*key).to_owned()))
        .collect()
}

#[test]
fn join_attaches_key_by_exact_name() {
    let object = object_from_toml_like(&[("Acme Gateway", "https://a.example", &[("m", None)])]);
    let config = CustomEndpointDefinitionsConfig::from_object(&object);
    let result = join_custom_endpoint_keys(&config, &keys(&[("Acme Gateway", "sk-1")]));
    assert_eq!(result.endpoints.len(), 1);
    assert_eq!(result.endpoints[0].api_key, "sk-1");
    assert!(result.orphaned_keys.is_empty());
}

#[test]
fn join_leaves_unkeyed_endpoint_with_empty_key() {
    let object = object_from_toml_like(&[("Acme Gateway", "https://a.example", &[("m", None)])]);
    let config = CustomEndpointDefinitionsConfig::from_object(&object);
    let result = join_custom_endpoint_keys(&config, &IndexMap::new());
    assert_eq!(result.endpoints[0].api_key, "");
}

#[test]
fn join_classifies_key_for_deleted_endpoint_as_orphaned() {
    let object = object_from_toml_like(&[("Kept", "https://a.example", &[("m", None)])]);
    let config = CustomEndpointDefinitionsConfig::from_object(&object);
    let result =
        join_custom_endpoint_keys(&config, &keys(&[("Kept", "sk-1"), ("Deleted", "sk-2")]));
    assert_eq!(result.orphaned_keys, vec!["Deleted".to_owned()]);
}

#[test]
fn join_classifies_key_for_renamed_endpoint_as_orphaned() {
    // Rename is delete-plus-add: the old name's key is orphaned even though a
    // "new" endpoint now exists under a different name.
    let object = object_from_toml_like(&[("New Name", "https://a.example", &[("m", None)])]);
    let config = CustomEndpointDefinitionsConfig::from_object(&object);
    let result = join_custom_endpoint_keys(&config, &keys(&[("Old Name", "sk-1")]));
    assert_eq!(result.orphaned_keys, vec!["Old Name".to_owned()]);
    assert_eq!(result.endpoints[0].api_key, "");
}

#[test]
fn join_preserves_key_for_present_but_invalid_endpoint() {
    let mut object = serde_json::Map::new();
    object.insert(
        "Typo".to_owned(),
        serde_json::json!({"url": "not-a-url", "models": [{"name": "m"}]}),
    );
    let config = CustomEndpointDefinitionsConfig::from_object(&object);
    assert!(config.has_diagnostics());
    let result = join_custom_endpoint_keys(&config, &keys(&[("Typo", "sk-1")]));
    // The invalid entry contributes no effective endpoint, but its key is not
    // orphaned — fixing the typo should reconnect it.
    assert!(result.endpoints.is_empty());
    assert!(result.orphaned_keys.is_empty());
}

#[test]
fn rename_produces_different_model_identity() {
    let before_object =
        object_from_toml_like(&[("Old Name", "https://a.example", &[("gpt-4o", None)])]);
    let before_config = CustomEndpointDefinitionsConfig::from_object(&before_object);
    let before = join_custom_endpoint_keys(&before_config, &keys(&[("Old Name", "sk-1")]));

    let after_object =
        object_from_toml_like(&[("New Name", "https://a.example", &[("gpt-4o", None)])]);
    let after_config = CustomEndpointDefinitionsConfig::from_object(&after_object);
    let after = join_custom_endpoint_keys(&after_config, &IndexMap::new());

    assert_ne!(
        before.endpoints[0].models[0].config_key,
        after.endpoints[0].models[0].config_key
    );
}

#[test]
fn case_only_rename_is_treated_as_a_different_endpoint() {
    let object = object_from_toml_like(&[("acme", "https://a.example", &[("m", None)])]);
    let config = CustomEndpointDefinitionsConfig::from_object(&object);
    let result = join_custom_endpoint_keys(&config, &keys(&[("Acme", "sk-1")]));
    assert_eq!(result.orphaned_keys, vec!["Acme".to_owned()]);
    assert_eq!(result.endpoints[0].api_key, "");
}

#[test]
fn deleting_one_model_does_not_affect_other_models_identity() {
    let object = object_from_toml_like(&[(
        "ep",
        "https://a.example",
        &[("gpt-4o", None), ("o3-mini", None)],
    )]);
    let config = CustomEndpointDefinitionsConfig::from_object(&object);
    let before = join_custom_endpoint_keys(&config, &IndexMap::new());
    let gpt4o_before = before.endpoints[0]
        .models
        .iter()
        .find(|m| m.name == "gpt-4o")
        .unwrap()
        .config_key
        .clone();

    let object = object_from_toml_like(&[("ep", "https://a.example", &[("gpt-4o", None)])]);
    let config = CustomEndpointDefinitionsConfig::from_object(&object);
    let after = join_custom_endpoint_keys(&config, &IndexMap::new());
    let gpt4o_after = after.endpoints[0].models[0].config_key.clone();

    assert_eq!(gpt4o_before, gpt4o_after);
}

// ── legacy/settings equivalence ──────────────────────────────────

#[test]
fn legacy_and_settings_validators_agree_on_valid_definition() {
    let settings_result = validate_custom_endpoint_definition(
        "Acme Gateway",
        &endpoint_file(
            "https://llm.acme.example/v1",
            vec![
                model_file("gpt-4o", Some("Acme GPT-4o")),
                model_file("o3-mini", None),
            ],
        ),
    );
    let legacy_result = validate_legacy_custom_endpoint_definition(
        "Acme Gateway",
        "https://llm.acme.example/v1",
        CustomEndpointSchema::OpenaiChatCompletions,
        &[
            ("gpt-4o".to_owned(), Some("Acme GPT-4o".to_owned())),
            ("o3-mini".to_owned(), None),
        ],
    );
    assert_eq!(settings_result.is_ok(), legacy_result.is_ok());
    assert_eq!(settings_result.unwrap(), legacy_result.unwrap());
}

#[test]
fn legacy_and_settings_validators_agree_on_restricted_url() {
    let settings_result = validate_custom_endpoint_definition(
        "ep",
        &endpoint_file("https://localhost/v1", vec![model_file("m", None)]),
    );
    let legacy_result = validate_legacy_custom_endpoint_definition(
        "ep",
        "https://localhost/v1",
        CustomEndpointSchema::OpenaiChatCompletions,
        &[("m".to_owned(), None)],
    );
    assert!(settings_result.is_err());
    assert!(legacy_result.is_err());
    assert_eq!(settings_result.unwrap_err(), legacy_result.unwrap_err());
}

#[test]
fn legacy_and_settings_validators_agree_on_duplicate_model_names() {
    let settings_result = validate_custom_endpoint_definition(
        "ep",
        &endpoint_file(
            "https://a.example",
            vec![model_file("m", None), model_file("m", None)],
        ),
    );
    let legacy_result = validate_legacy_custom_endpoint_definition(
        "ep",
        "https://a.example",
        CustomEndpointSchema::OpenaiChatCompletions,
        &[("m".to_owned(), None), ("m".to_owned(), None)],
    );
    assert!(settings_result.is_err());
    assert!(legacy_result.is_err());
}
