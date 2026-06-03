//! Unit tests for the `coerce_integer_args` helper.

use serde_json::json;

use super::*;

fn obj(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    match value {
        serde_json::Value::Object(m) => m,
        _ => panic!("expected a JSON object"),
    }
}

/// Asserts that `args[path...]` serializes as the given JSON snippet. `path` is a
/// dotted lookup like `"a.b.c"`; numeric segments index into arrays.
fn assert_serialized_as(
    args: &serde_json::Map<String, serde_json::Value>,
    path: &str,
    expected: &str,
) {
    let mut current = &serde_json::Value::Object(args.clone());
    for segment in path.split('.') {
        current = match current {
            serde_json::Value::Object(map) => map
                .get(segment)
                .unwrap_or_else(|| panic!("missing key {segment} in {current}")),
            serde_json::Value::Array(arr) => {
                let idx: usize = segment.parse().unwrap();
                arr.get(idx)
                    .unwrap_or_else(|| panic!("missing index {idx} in {current}"))
            }
            _ => panic!("cannot descend into {current} via {segment}"),
        };
    }
    assert_eq!(serde_json::to_string(current).unwrap(), expected);
}

// ---------- Backward-compat (the original #6945 cases) ----------

#[test]
fn whole_float_is_coerced_when_schema_declares_integer() {
    let mut args = obj(json!({ "line": 5.0 }));
    let schema = obj(json!({
        "properties": { "line": { "type": "integer" } }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_eq!(serde_json::to_string(&args["line"]).unwrap(), "5");
    assert_eq!(args["line"].as_i64(), Some(5));
}

#[test]
fn no_coercion_when_not_typed_as_integer() {
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

// ---------- Nested object recursion ----------

#[test]
fn nested_object_integer_is_coerced() {
    let mut args = obj(json!({ "request_data": { "search_from": 0.0, "search_to": 100.0 } }));
    let schema = obj(json!({
        "type": "object",
        "properties": {
            "request_data": {
                "type": "object",
                "properties": {
                    "search_from": { "type": "integer" },
                    "search_to":   { "type": "integer" },
                }
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_serialized_as(&args, "request_data.search_from", "0");
    assert_serialized_as(&args, "request_data.search_to", "100");
}

// ---------- Array recursion ----------

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

    assert_serialized_as(&args, "values.0", "1");
    assert_serialized_as(&args, "values.1", "2");
    assert_serialized_as(&args, "values.2", "3");
}

#[test]
fn tuple_items_coerced_positionally() {
    let mut args = obj(json!({ "pair": [42.0, "label"] }));
    let schema = obj(json!({
        "properties": {
            "pair": {
                "type": "array",
                "items": [
                    { "type": "integer" },
                    { "type": "string" }
                ]
            }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_serialized_as(&args, "pair.0", "42");
    assert_serialized_as(&args, "pair.1", "\"label\"");
}

// ---------- Composition keywords ----------

#[test]
fn all_of_branches_apply_in_order() {
    // First branch declares the object shape, second branch declares an integer
    // field. Both must be applied to reach the integer coercion.
    let mut args = obj(json!({ "x": 7.0 }));
    let schema = obj(json!({
        "allOf": [
            { "type": "object" },
            { "properties": { "x": { "type": "integer" } } }
        ]
    }));

    coerce_integer_args(&mut args, &schema);

    assert_serialized_as(&args, "x", "7");
}

// ---------- Nullable / type-array forms ----------

#[test]
fn null_value_for_nullable_integer_passes_through() {
    let mut args = obj(json!({ "x": serde_json::Value::Null }));
    let schema = obj(json!({
        "properties": { "x": { "type": ["integer", "null"] } }
    }));

    coerce_integer_args(&mut args, &schema);

    assert!(args["x"].is_null());
}

// ---------- $ref resolution ----------

#[test]
fn internal_ref_resolves() {
    let mut args = obj(json!({ "ts": 1730419200000.0 }));
    let schema = obj(json!({
        "$defs": {
            "Timestamp": { "type": "integer" }
        },
        "properties": {
            "ts": { "$ref": "#/$defs/Timestamp" }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_serialized_as(&args, "ts", "1730419200000");
}

#[test]
fn internal_ref_cycle_terminates() {
    // A self-referencing schema with an integer leaf at each level. The cycle
    // detector must not block coercion as we descend into actual data depth.
    let mut args = obj(json!({
        "next": { "next": { "value": 7.0 }, "value": 6.0 },
        "value": 5.0
    }));
    let schema = obj(json!({
        "$defs": {
            "Node": {
                "properties": {
                    "next":  { "$ref": "#/$defs/Node" },
                    "value": { "type": "integer" }
                }
            }
        },
        "$ref": "#/$defs/Node"
    }));

    coerce_integer_args(&mut args, &schema);

    assert_serialized_as(&args, "value", "5");
    assert_serialized_as(&args, "next.value", "6");
    assert_serialized_as(&args, "next.next.value", "7");
}

#[test]
fn external_ref_is_skipped() {
    // An external `$ref` is left as an opaque schema (no recursion). Sibling
    // integer coercion at the same level still works.
    let mut args = obj(json!({ "a": 1.0, "b": 2.0 }));
    let schema = obj(json!({
        "properties": {
            "a": { "$ref": "https://example.com/Foo" },
            "b": { "type": "integer" }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_serialized_as(&args, "a", "1.0");
    assert_serialized_as(&args, "b", "2");
}

#[test]
fn pure_ref_cycle_does_not_loop() {
    // A→B→A schema cycle with no data depth must terminate (and is a no-op).
    let mut args = obj(json!({ "x": 3.0 }));
    let schema = obj(json!({
        "$defs": {
            "A": { "$ref": "#/$defs/B" },
            "B": { "$ref": "#/$defs/A" }
        },
        "properties": {
            "x": { "$ref": "#/$defs/A" }
        }
    }));

    coerce_integer_args(&mut args, &schema);

    // Cycle blocks ref resolution before reaching any concrete type — no coercion.
    assert_serialized_as(&args, "x", "3.0");
}

// ---------- additionalProperties ----------

#[test]
fn additional_properties_integer_is_coerced() {
    let mut args = obj(json!({ "a": 1.0, "b": 2.0 }));
    let schema = obj(json!({
        "type": "object",
        "additionalProperties": { "type": "integer" }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_serialized_as(&args, "a", "1");
    assert_serialized_as(&args, "b", "2");
}

#[test]
fn additional_properties_does_not_overwrite_known_properties() {
    let mut args = obj(json!({ "known": 1.0, "extra": 2.0 }));
    let schema = obj(json!({
        "type": "object",
        "properties": {
            "known": { "type": "number" }
        },
        "additionalProperties": { "type": "integer" }
    }));

    coerce_integer_args(&mut args, &schema);

    // `known` is governed by `properties` (type: number), so no coercion.
    assert_serialized_as(&args, "known", "1.0");
    // `extra` matches `additionalProperties` (type: integer), so coerced.
    assert_serialized_as(&args, "extra", "2");
}

#[test]
fn pattern_properties_keys_are_excluded_from_additional_properties() {
    // Per JSON Schema, a key that matches a `patternProperties` regex is
    // governed by that pattern's schema, not by `additionalProperties`. Our
    // walker doesn't (yet) coerce through `patternProperties`, but it must
    // still skip those keys when iterating `additionalProperties` — otherwise
    // a value governed by a pattern schema can be coerced by the wrong
    // schema.
    let mut args = obj(json!({ "_internal": 5.0, "regular": 7.0 }));
    let schema = obj(json!({
        "type": "object",
        "patternProperties": {
            "^_": { "type": "number" }
        },
        "additionalProperties": { "type": "integer" }
    }));

    coerce_integer_args(&mut args, &schema);

    // `_internal` matches the `^_` pattern → governed by patternProperties
    // (type: number) → must NOT be coerced to integer.
    assert_serialized_as(&args, "_internal", "5.0");
    // `regular` matches no pattern → falls under additionalProperties
    // (type: integer) → coerced.
    assert_serialized_as(&args, "regular", "7");
}

#[test]
fn uncompilable_pattern_skips_additional_properties_conservatively() {
    // JSON Schema's pattern grammar is ECMA-262; Rust's `regex` crate is a
    // strict subset and rejects valid patterns like lookaheads. We can't tell
    // "this key doesn't match the pattern" apart from "I couldn't compile the
    // pattern at all" — so if any pattern fails to compile we skip
    // `additionalProperties` entirely for this schema. A key that should have
    // been governed by the pattern won't be coerced with the wrong schema.
    let mut args = obj(json!({ "x": 3.0 }));
    let schema = obj(json!({
        "type": "object",
        "patternProperties": {
            "(": { "type": "number" }
        },
        "additionalProperties": { "type": "integer" }
    }));

    coerce_integer_args(&mut args, &schema);

    // `x` would normally be coerced via additionalProperties, but the
    // unparseable pattern forces us into the conservative skip.
    assert_serialized_as(&args, "x", "3.0");
}

#[test]
fn uncompilable_pattern_still_allows_properties_recursion() {
    // The conservative fallback only skips `additionalProperties` — it must
    // not poison `properties` recursion at the same level, since each
    // property's schema is fully specified and unaffected by the pattern.
    let mut args = obj(json!({ "known": 4.0, "extra": 5.0 }));
    let schema = obj(json!({
        "type": "object",
        "properties": {
            "known": { "type": "integer" }
        },
        "patternProperties": {
            "(": { "type": "number" }
        },
        "additionalProperties": { "type": "integer" }
    }));

    coerce_integer_args(&mut args, &schema);

    // `known` is governed by `properties` → coerced normally.
    assert_serialized_as(&args, "known", "4");
    // `extra` would normally fall under `additionalProperties`, but the
    // unparseable pattern forces the conservative skip.
    assert_serialized_as(&args, "extra", "5.0");
}

// ---------- Conservative cases ----------

#[test]
fn non_whole_float_is_not_truncated() {
    let mut args = obj(json!({ "x": 5.5 }));
    let schema = obj(json!({
        "properties": { "x": { "type": "integer" } }
    }));

    coerce_integer_args(&mut args, &schema);

    // Server will reject, but we don't lossily round.
    assert_serialized_as(&args, "x", "5.5");
}

#[test]
fn out_of_i64_range_is_not_coerced() {
    let mut args = obj(json!({ "x": 1e30 }));
    let schema = obj(json!({
        "properties": { "x": { "type": "integer" } }
    }));

    coerce_integer_args(&mut args, &schema);

    // Value is whole-number but outside i64 range — leave it as a float.
    assert!(args["x"].as_f64().unwrap() > 9.2e18);
    assert!(args["x"].as_i64().is_none());
}

#[test]
fn large_timestamp_is_coerced() {
    // The exact value from the issue: a Unix millisecond timestamp.
    let mut args = obj(json!({ "ts": 1730419200000.0 }));
    let schema = obj(json!({
        "properties": { "ts": { "type": "integer" } }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_serialized_as(&args, "ts", "1730419200000");
    assert_eq!(args["ts"].as_i64(), Some(1730419200000));
}

// ---------- Recursive composition guard ----------

#[test]
fn all_of_self_reference_terminates() {
    // An MCP server could advertise a schema where `allOf` recursively
    // references its own definition. `resolve_refs` breaks pure `$ref`
    // chains, but without a depth guard `coerce_recursive` → `allOf` →
    // `coerce_recursive` → `allOf` → ... loops indefinitely while preparing
    // a tool call. The depth limit must terminate this without panicking.
    let mut args = obj(json!({ "x": 3.0 }));
    let schema = obj(json!({
        "$defs": {
            "Loop": {
                "allOf": [{ "$ref": "#/$defs/Loop" }],
                "properties": { "x": { "type": "integer" } }
            }
        },
        "$ref": "#/$defs/Loop"
    }));

    coerce_integer_args(&mut args, &schema);

    // The loop unwinds before reaching the property recursion at the
    // bottom, so `x` does not get coerced. The important guarantee is
    // termination, not correctness on a malicious schema.
    assert!(
        args["x"].is_number(),
        "coercion call must terminate without panicking"
    );
}

#[test]
fn one_of_self_reference_terminates() {
    // Same shape via `oneOf` instead of `allOf`.
    let mut args = obj(json!({ "x": 3.0 }));
    let schema = obj(json!({
        "$defs": {
            "Loop": {
                "oneOf": [{ "$ref": "#/$defs/Loop" }],
                "properties": { "x": { "type": "integer" } }
            }
        },
        "$ref": "#/$defs/Loop"
    }));

    coerce_integer_args(&mut args, &schema);

    assert!(args["x"].is_number());
}

#[test]
fn fan_out_via_all_of_self_reference_terminates() {
    // `allOf: [{$ref: "#/Self"}, {$ref: "#/Self"}]` doubles work at each
    // level of recursion. With depth cap alone, the call still produces
    // 2^64 (~1.8e19) calls before any single chain reaches the cap. The
    // total op budget bounds work regardless of branching factor.
    let mut args = obj(json!({ "x": 3.0 }));
    let schema = obj(json!({
        "$defs": {
            "Loop": {
                "allOf": [
                    { "$ref": "#/$defs/Loop" },
                    { "$ref": "#/$defs/Loop" }
                ],
                "properties": { "x": { "type": "integer" } }
            }
        },
        "$ref": "#/$defs/Loop"
    }));

    coerce_integer_args(&mut args, &schema);

    // The cap bails before any meaningful coercion happens — the guarantee
    // is termination + bounded work, not correctness on a malicious schema.
    assert!(
        args["x"].is_number(),
        "coercion call must terminate without panicking"
    );
}

#[test]
fn deep_finite_nesting_under_limit_still_coerces() {
    // A schema with 10 levels of nested `properties` is well under the
    // depth limit and must still produce a coerced result.
    fn nest(remaining: usize) -> serde_json::Value {
        if remaining == 0 {
            json!({ "type": "integer" })
        } else {
            json!({
                "type": "object",
                "properties": { "next": nest(remaining - 1) }
            })
        }
    }
    fn nest_value(remaining: usize) -> serde_json::Value {
        if remaining == 0 {
            json!(5.0)
        } else {
            json!({ "next": nest_value(remaining - 1) })
        }
    }

    let mut args = obj(nest_value(10));
    let schema = obj(nest(10));

    coerce_integer_args(&mut args, &schema);

    // Walk down to the leaf and confirm it coerced.
    let mut current = &serde_json::Value::Object(args);
    for _ in 0..10 {
        current = &current["next"];
    }
    assert_eq!(serde_json::to_string(current).unwrap(), "5");
}

// ---------- End-to-end: the issue's repro schema ----------

#[test]
fn panw_audit_management_repro_coerces_all_integers() {
    let mut args = obj(json!({
        "request_data": {
            "search_from": 0.0,
            "search_to": 100.0,
            "filters": [
                { "field": "timestamp", "operator": "gte", "value": 1730419200000.0 }
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

    assert_serialized_as(&args, "request_data.search_from", "0");
    assert_serialized_as(&args, "request_data.search_to", "100");
    assert_serialized_as(&args, "request_data.filters.0.value", "1730419200000");
}

// ---------- oneOf/anyOf branch selection ----------

#[test]
fn tagged_union_does_not_coerce_through_non_matching_branch() {
    // Discriminated union: branch "a" types `val` as integer, branch "b" types
    // it as number. The instance carries `kind: "b"`, so the float `val` must
    // be left alone — coercing it via branch "a" would corrupt a legitimate
    // float the server asked for.
    let mut args = obj(json!({ "kind": "b", "val": 5.0 }));
    let schema = obj(json!({
        "oneOf": [
            {
                "properties": {
                    "kind": { "const": "a" },
                    "val": { "type": "integer" }
                }
            },
            {
                "properties": {
                    "kind": { "const": "b" },
                    "val": { "type": "number" }
                }
            }
        ]
    }));

    coerce_integer_args(&mut args, &schema);

    // Stayed a float — branch "a" was excluded by the `kind` discriminator.
    assert_serialized_as(&args, "val", "5.0");
}

#[test]
fn tagged_union_coerces_through_matching_branch() {
    // Same schema as above, but the instance selects branch "a", so `val`
    // should coerce to an integer.
    let mut args = obj(json!({ "kind": "a", "val": 5.0 }));
    let schema = obj(json!({
        "oneOf": [
            {
                "properties": {
                    "kind": { "const": "a" },
                    "val": { "type": "integer" }
                }
            },
            {
                "properties": {
                    "kind": { "const": "b" },
                    "val": { "type": "number" }
                }
            }
        ]
    }));

    coerce_integer_args(&mut args, &schema);

    assert_serialized_as(&args, "val", "5");
}

#[test]
fn enum_discriminator_excludes_non_matching_branch() {
    // `enum` discriminator instead of `const`.
    let mut args = obj(json!({ "tag": "float", "n": 9.0 }));
    let schema = obj(json!({
        "oneOf": [
            {
                "properties": {
                    "tag": { "enum": ["int"] },
                    "n": { "type": "integer" }
                }
            },
            {
                "properties": {
                    "tag": { "enum": ["float", "double"] },
                    "n": { "type": "number" }
                }
            }
        ]
    }));

    coerce_integer_args(&mut args, &schema);

    assert_serialized_as(&args, "n", "9.0");
}

#[test]
fn type_mismatched_branch_is_skipped() {
    // A branch whose top-level `type` the value can't satisfy is excluded, so
    // the object's integer field is coerced only via the matching object branch.
    let mut args = obj(json!({ "x": 3.0 }));
    let schema = obj(json!({
        "anyOf": [
            { "type": "string" },
            { "type": "object", "properties": { "x": { "type": "integer" } } }
        ]
    }));

    coerce_integer_args(&mut args, &schema);

    assert_serialized_as(&args, "x", "3");
}

#[test]
fn required_discriminator_excludes_branch_missing_keys() {
    // Branch "a" requires `a_only`; the instance lacks it, so that branch is
    // excluded and `shared` is coerced via branch "b".
    let mut args = obj(json!({ "b_only": true, "shared": 4.0 }));
    let schema = obj(json!({
        "oneOf": [
            {
                "required": ["a_only"],
                "properties": { "shared": { "type": "number" } }
            },
            {
                "required": ["b_only"],
                "properties": { "shared": { "type": "integer" } }
            }
        ]
    }));

    coerce_integer_args(&mut args, &schema);

    assert_serialized_as(&args, "shared", "4");
}

// ---------- patternProperties compile bounds ----------

#[test]
fn too_many_pattern_properties_skips_additional_coercion() {
    // More than MAX_PATTERN_PROPERTIES patterns ⇒ we can't safely reason about
    // which keys are pattern-governed, so additionalProperties coercion is
    // skipped and the float is left untouched (conservative fallback).
    let mut patterns = serde_json::Map::new();
    for i in 0..(MAX_PATTERN_PROPERTIES + 1) {
        patterns.insert(format!("^p{i}_"), json!({ "type": "string" }));
    }
    let mut args = obj(json!({ "free": 5.0 }));
    let schema = obj(json!({
        "type": "object",
        "patternProperties": patterns,
        "additionalProperties": { "type": "integer" }
    }));

    coerce_integer_args(&mut args, &schema);

    // Skipped — stayed a float.
    assert_serialized_as(&args, "free", "5.0");
}

#[test]
fn overlong_pattern_skips_additional_coercion() {
    let long_pattern = format!("^{}$", "a".repeat(MAX_PATTERN_LEN + 1));
    let mut args = obj(json!({ "free": 5.0 }));
    let schema = obj(json!({
        "type": "object",
        "patternProperties": { long_pattern: { "type": "string" } },
        "additionalProperties": { "type": "integer" }
    }));

    coerce_integer_args(&mut args, &schema);

    assert_serialized_as(&args, "free", "5.0");
}

#[test]
fn bounded_pattern_properties_still_coerce_non_matching_keys() {
    // A small, well-formed patternProperties set compiles fine: keys matching
    // the pattern are excluded, and other keys still coerce via
    // additionalProperties.
    let mut args = obj(json!({ "_internal": 1.0, "free": 2.0 }));
    let schema = obj(json!({
        "type": "object",
        "patternProperties": { "^_": { "type": "string" } },
        "additionalProperties": { "type": "integer" }
    }));

    coerce_integer_args(&mut args, &schema);

    // `_internal` is pattern-governed (left alone); `free` coerces.
    assert_serialized_as(&args, "_internal", "1.0");
    assert_serialized_as(&args, "free", "2");
}
