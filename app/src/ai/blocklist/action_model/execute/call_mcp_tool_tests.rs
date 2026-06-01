//! Unit tests for the `coerce_integer_args` helper.

use serde_json::json;

use super::*;

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
fn nested_object_integer_is_coerced() {
    let mut args = obj(json!({ "outer": { "inner": 5.0 } }));
    let schema = obj(json!({
        "properties": {
            "outer": {
                "type": "object",
                "properties": { "inner": { "type": "integer" } }
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["outer"]["inner"].as_i64(), Some(5));
    assert_eq!(serde_json::to_string(&args["outer"]["inner"]).unwrap(), "5");
}

#[test]
fn array_items_integer_is_coerced() {
    let mut args = obj(json!({ "values": [1.0, 2.0, 3.0] }));
    let schema = obj(json!({
        "properties": {
            "values": {
                "type": "array",
                "items": { "type": "integer" }
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(serde_json::to_string(&args["values"]).unwrap(), "[1,2,3]");
}

#[test]
fn tuple_array_items_integer_is_coerced_positionally() {
    // `items` as an array (tuple validation) applies each subschema by index.
    let mut args = obj(json!({ "pair": [1.0, 2.5] }));
    let schema = obj(json!({
        "properties": {
            "pair": {
                "type": "array",
                "items": [
                    { "type": "integer" },
                    { "type": "number" }
                ]
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["pair"][0].as_i64(), Some(1));
    assert_eq!(args["pair"][1].as_f64(), Some(2.5));
}

#[test]
fn oneof_integer_branch_is_coerced() {
    // Mirrors the failing case from issue #10596: a filter object whose `value`
    // can be either an array of strings or an integer timestamp.
    let mut args = obj(json!({
        "filter": { "field": "timestamp", "value": 1_730_419_200_000.0 }
    }));
    let schema = obj(json!({
        "properties": {
            "filter": {
                "oneOf": [
                    { "properties": { "value": { "type": "array", "items": { "type": "string" } } } },
                    { "properties": { "value": { "type": "integer" } } }
                ]
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["filter"]["value"].as_i64(), Some(1_730_419_200_000));
}

#[test]
fn anyof_integer_branch_is_coerced() {
    let mut args = obj(json!({ "x": 7.0 }));
    let schema = obj(json!({
        "properties": {
            "x": {
                "anyOf": [
                    { "type": "string" },
                    { "type": "integer" }
                ]
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["x"].as_i64(), Some(7));
}

#[test]
fn allof_integer_is_coerced() {
    let mut args = obj(json!({ "x": 9.0 }));
    let schema = obj(json!({
        "properties": {
            "x": {
                "allOf": [
                    { "type": "integer" },
                    { "minimum": 0 }
                ]
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["x"].as_i64(), Some(9));
}

#[test]
fn nullable_integer_type_array_is_coerced() {
    // OpenAPI / JSON Schema allow `"type": ["integer", "null"]` for a nullable
    // integer; the integer-typed value should still be coerced.
    let mut args = obj(json!({ "maybe": 42.0 }));
    let schema = obj(json!({
        "properties": {
            "maybe": { "type": ["integer", "null"] }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["maybe"].as_i64(), Some(42));
}

#[test]
fn fractional_floats_in_nested_shapes_are_preserved() {
    // Regression: even when the schema declares integer, a value with a
    // fractional part must not be silently truncated. Leave the float intact
    // so the server's schema validation can reject it explicitly.
    let mut args = obj(json!({
        "outer": { "inner": 5.25 },
        "values": [1.5, 2.5],
    }));
    let schema = obj(json!({
        "properties": {
            "outer": {
                "properties": { "inner": { "type": "integer" } }
            },
            "values": {
                "type": "array",
                "items": { "type": "integer" }
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["outer"]["inner"].as_f64(), Some(5.25));
    assert_eq!(args["values"][0].as_f64(), Some(1.5));
    assert_eq!(args["values"][1].as_f64(), Some(2.5));
}

#[test]
fn nested_non_integer_fields_are_left_alone() {
    let mut args = obj(json!({
        "outer": { "ratio": 0.5, "label": "x" }
    }));
    let schema = obj(json!({
        "properties": {
            "outer": {
                "properties": {
                    "ratio": { "type": "number" },
                    "label": { "type": "string" }
                }
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["outer"]["ratio"].as_f64(), Some(0.5));
    assert_eq!(args["outer"]["label"].as_str(), Some("x"));
}

#[test]
fn top_level_composition_is_walked() {
    // Some OpenAPI-derived MCP schemas put `oneOf` / `anyOf` at the root rather
    // than inside a property — make sure the entry point walks composition at
    // the top level too, not just inside properties.
    let mut args = obj(json!({ "x": 5.0 }));
    let schema = obj(json!({
        "oneOf": [
            { "properties": { "x": { "type": "integer" } } },
            { "properties": { "y": { "type": "string" } } }
        ]
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(args["x"].as_i64(), Some(5));
}

#[test]
fn audit_management_log_request_matches_reporter_payload() {
    // End-to-end repro from issue #10596 using the reporter's exact schema and
    // payload. Verifies every integer field serializes without a `.0` suffix.
    let mut args = obj(json!({
        "request_data": {
            "search_from": 0.0,
            "search_to": 100.0,
            "filters": [
                { "field": "timestamp", "operator": "gte", "value": 1_730_419_200_000.0 }
            ]
        }
    }));
    let schema = obj(json!({
        "type": "object",
        "properties": {
            "request_data": {
                "type": "object",
                "properties": {
                    "search_from": { "type": "integer" },
                    "search_to":   { "type": "integer" },
                    "filters": {
                        "type": "array",
                        "items": {
                            "oneOf": [
                                { "properties": { "value": { "type": "array", "items": { "type": "string" } } } },
                                { "properties": { "value": { "type": "integer" } } }
                            ]
                        }
                    }
                }
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    // The core of the bug: no whole-number float suffixes should remain in the
    // serialized payload, regardless of object key ordering.
    let serialized = serde_json::to_string(&args).unwrap();
    assert!(
        !serialized.contains(".0"),
        "expected no \".0\" suffixes after coercion, got: {serialized}"
    );

    // Round-trip through `Value` to compare structurally (key order in the raw
    // string is implementation-defined; content equality is what matters).
    let actual: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    let expected = json!({
        "request_data": {
            "search_from": 0,
            "search_to": 100,
            "filters": [
                { "field": "timestamp", "operator": "gte", "value": 1_730_419_200_000_i64 }
            ]
        }
    });
    assert_eq!(actual, expected);
}
