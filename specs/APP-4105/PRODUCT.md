# APP-4105: Fix MCP Tool Call Integer Parameter Coercion

## Summary

MCP tool calls that include integer-typed parameters fail on strict MCP servers because Warp serializes those values as floats (e.g. `5.0`) instead of integers (e.g. `5`) in JSON. The fix coerces float-valued arguments back to integers at the client before dispatching, using the tool's own [JSON Schema `"type"` keyword](https://json-schema.org/understanding-json-schema/reference/type) to decide which fields need coercion.

## Problem

When a user invokes an MCP tool that has integer parameters (e.g. `line` and `column` in a code navigation tool), some MCP servers — including the GoLand MCP server — reject the call with a parsing error like:

```
Failed to parse literal '5.0' as an int value
```

This makes those tools completely unusable from Warp, even though the model generates semantically correct arguments. The failure is invisible to the user: the tool call is dispatched, but the MCP server returns an error.

The root cause is that the LLM's original JSON string (e.g. `{"line": 5}`) is parsed into a `google.protobuf.Struct` on the server. `Struct`'s `NumberValue` stores all numbers as `float64`, erasing the integer/float distinction. When the Rust client re-serializes to JSON via `serde_json::Number::from_f64`, whole-number floats become `5.0` on the wire instead of `5`. JSON Schema deserializers in strict-typed languages (Kotlin/Java, strict Python, Rust with `i64`) reject `5.0` for integer-typed fields.

## Goals

- MCP tool calls with integer-typed parameters are accepted by strict MCP servers that require `5` rather than `5.0` in JSON.
- MCP tool calls with float-typed parameters (e.g. `temperature: 0.7`) continue to work correctly.
- MCP tool calls with mixed or string-only parameters are unaffected.
- No server-side or proto changes — the fix is entirely client-side.

## Non-goals

- Fixing the underlying loss of type information in the `structpb.NumberValue` wire format. The lossy server-side encoding remains; the fix restores the integer literal at dispatch time using the tool's schema.
- Preserving full 64-bit integer precision for values above 2⁵³. Those have already been rounded by the `float64` round-trip before the client sees them; this fix can restore the integer *literal form* but not the lost bits. In practice this is only a concern for very large IDs and timestamps; typical MCP tools use small integers.
- Walking nested objects, arrays, or JSON Schema combinators (`anyOf`/`oneOf`/`$ref`). Only top-level properties with `"type": "integer"` are coerced. No known MCP tool requires more than this today.
- Changing server-side behavior or the LLM prompt.

## Figma

Figma: none provided (no UI changes).

## User Experience

### Normal case — integer parameters

Given a tool `get_symbol_info` with schema:
```json
{ "properties": { "line": { "type": "integer" }, "column": { "type": "integer" }, "filePath": { "type": "string" } } }
```

When the model generates `{"line": 5, "column": 1, "filePath": "server.go"}`, the MCP server receives:
```json
{"line": 5, "column": 1, "filePath": "server.go"}
```
(integers, not `5.0`/`1.0`).

### Normal case — number (float) parameters

Given a tool with schema `{ "properties": { "temperature": { "type": "number" } } }` and model input `{"temperature": 0.7}`, the MCP server receives `{"temperature": 0.7}` unchanged. Floats are never touched.

If the model happens to generate a whole-number value for a `number`-typed field (e.g. `{"temperature": 1.0}`), this is left as-is and serialized as a float — which is correct, since the schema declared the field as `number`.

### Schema lookup failure

If the tool's `input_schema` cannot be located (server disconnected mid-call, tool name mismatch, no active server matches), the call proceeds with the arguments unchanged. The failure mode is no worse than today.

### Non-integer-valued floats in integer fields

If the LLM generates a non-whole float like `{"line": 5.5}` for an integer-typed field, the value is left unchanged (not coerced). The MCP server will reject it as invalid — this is correct behavior; the LLM generated bad input, and silent truncation would be worse than a clear error.

### MCP tool call detail in the blocklist

The rendered MCP tool call detail (shown in each AI block alongside the tool name) displays the same arguments that will actually be dispatched. An integer-typed parameter is rendered as `5`, not `5.0`. This keeps the UI honest about what the MCP server will receive — a user reading the block can trust that the literal form they see matches what's sent over the wire.

The same coercion step that runs at dispatch time also runs when building the block-detail display string, so render and dispatch stay in lock-step. If the schema lookup fails for either path, that path falls back to the raw (uncoerced) values independently — there is no assumption of coupling between the two.

## Success Criteria

- Calling a GoLand MCP tool with integer-typed parameters (e.g. `get_symbol_info` with `line: 5`) succeeds and returns a result, rather than returning a parse error.
- A tool with `number`-typed parameters (e.g. `temperature: 0.7`) continues to work correctly.
- A tool with no integer parameters in its schema is unaffected.
- A tool whose schema cannot be found at dispatch time still dispatches (with uncoerced arguments) rather than erroring out of the coercion path.
- The MCP tool call detail rendered in the blocklist shows integer-typed fields as integers (e.g. `5`), matching what the MCP server will receive.
- Existing MCP tool call tests continue to pass.

## Validation

1. **Manual**: Connect the GoLand MCP server to Warp and invoke `get_symbol_info` at a valid line/column. Confirm a valid result is returned rather than a `Failed to parse literal '5.0'` error.
2. **Unit tests** on the coercion helper:
   - `integer`-typed field with whole-number float value → coerced to integer
   - `integer`-typed field with non-whole float value → unchanged
   - `number`-typed field with whole-number float value → unchanged
   - `string`-typed field → unchanged
   - missing `properties` key in schema → no-op
   - schema absent entirely → no-op
3. **Regression**: Run existing MCP-related tests and confirm no failures.

## Open Questions

- None at this time.
