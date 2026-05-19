//! Unit tests for the `coerce_integer_args` helper.

use super::*;
use serde_json::json;

fn obj(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    match value {
        serde_json::Value::Object(m) => m,
        _ => panic!("expected a JSON object"),
    }
}

#[test]
fn whole_float_is_coerced_when_schema_declares_integer() {
    let mut args = obj(json!({ "line": 5.0 }));
    let schema = obj(json!({
        "properties": { "line": { "type": "integer" } }
    }));

    coerce_integer_args(&mut args, &schema);

    // Serialized as "5", not "5.0", and round-trips as i64.
    assert_eq!(serde_json::to_string(&args["line"]).unwrap(), "5");
    assert_eq!(args["line"].as_i64(), Some(5));
}

#[test]
fn no_coercion_when_not_typed_as_integer() {
    // Three scenarios that should all preserve the original float value:
    //   * schema declares `"type": "number"` (explicit float)
    //   * schema has no `properties` at all
    //   * schema property lacks a `"type"` key
    let cases = [
        json!({ "properties": { "x": { "type": "number" } } }),
        json!({}),
        json!({ "properties": { "x": { "description": "no type" } } }),
    ];

    for schema_value in cases {
        let mut args = obj(json!({ "x": 1.0 }));
        let schema = obj(schema_value);

        coerce_integer_args(&mut args, &schema);

        assert_eq!(args["x"].as_f64(), Some(1.0));
        assert_eq!(serde_json::to_string(&args["x"]).unwrap(), "1.0");
    }
}

#[test]
fn nested_integer_fields_are_coerced() {
    let mut args = obj(json!({
        "request_data": {
            "search_from": 0.0,
            "search_to": 100.0,
            "ratio": 1.5
        }
    }));
    let schema = obj(json!({
        "properties": {
            "request_data": {
                "type": "object",
                "properties": {
                    "search_from": { "type": "integer" },
                    "search_to": { "type": "integer" },
                    "ratio": { "type": "number" }
                }
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(
        serde_json::to_string(&args["request_data"]["search_from"]).unwrap(),
        "0"
    );
    assert_eq!(
        serde_json::to_string(&args["request_data"]["search_to"]).unwrap(),
        "100"
    );
    assert_eq!(
        serde_json::to_string(&args["request_data"]["ratio"]).unwrap(),
        "1.5"
    );
}

#[test]
fn array_items_and_one_of_branches_are_coerced() {
    let mut args = obj(json!({
        "filters": [
            { "field": "timestamp", "value": 1730419200000.0 },
            { "field": "names", "value": ["a", "b"] }
        ]
    }));
    let schema = obj(json!({
        "properties": {
            "filters": {
                "type": "array",
                "items": {
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {
                                "value": { "type": "array", "items": { "type": "string" } }
                            }
                        },
                        {
                            "type": "object",
                            "properties": {
                                "value": { "type": "integer" }
                            }
                        }
                    ]
                }
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(
        serde_json::to_string(&args["filters"][0]["value"]).unwrap(),
        "1730419200000"
    );
    assert_eq!(args["filters"][1]["value"], json!(["a", "b"]));
}

#[test]
fn one_of_uses_first_matching_branch_before_coercing() {
    let mut args = obj(json!({ "value": 1.0 }));
    let schema = obj(json!({
        "properties": {
            "value": {
                "oneOf": [
                    { "type": "number" },
                    { "type": "integer" }
                ]
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(serde_json::to_string(&args["value"]).unwrap(), "1.0");
}

#[test]
fn additional_properties_skip_declared_properties() {
    let mut args = obj(json!({
        "price": 1.0,
        "quantity": 2.0
    }));
    let schema = obj(json!({
        "properties": {
            "price": { "type": "number" }
        },
        "additionalProperties": { "type": "integer" }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(serde_json::to_string(&args["price"]).unwrap(), "1.0");
    assert_eq!(serde_json::to_string(&args["quantity"]).unwrap(), "2");
}

#[test]
fn integer_coercion_rejects_f64_i64_upper_rounding_boundary() {
    let mut args = obj(json!({ "id": 9_223_372_036_854_775_808.0 }));
    let schema = obj(json!({
        "properties": { "id": { "type": "integer" } }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["id"].as_f64(), Some(9_223_372_036_854_775_808.0));
}

#[test]
fn refs_and_nullable_integer_types_are_coerced() {
    let mut args = obj(json!({
        "limit": 5.0,
        "cursor": null,
        "range": { "start": 10.0, "end": 12.0 }
    }));
    let schema = obj(json!({
        "$defs": {
            "nullable_integer": { "type": ["integer", "null"] },
            "range": {
                "type": "object",
                "properties": {
                    "start": { "$ref": "#/$defs/nullable_integer" },
                    "end": { "type": "integer" }
                }
            }
        },
        "properties": {
            "limit": { "$ref": "#/$defs/nullable_integer" },
            "cursor": { "$ref": "#/$defs/nullable_integer" },
            "range": { "$ref": "#/$defs/range" }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(serde_json::to_string(&args["limit"]).unwrap(), "5");
    assert_eq!(args["cursor"], serde_json::Value::Null);
    assert_eq!(
        serde_json::to_string(&args["range"]["start"]).unwrap(),
        "10"
    );
    assert_eq!(serde_json::to_string(&args["range"]["end"]).unwrap(), "12");
}
