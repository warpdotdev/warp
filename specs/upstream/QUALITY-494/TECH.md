# SettingsValue: Custom Settings File Serialization

## Problem
Settings values are serialized to the TOML settings file using their serde representations, which aren't always human-friendly. Duration values render as `{ nanos = 0, secs = 30 }` instead of `30`, and `AgentModeCommandExecutionPredicate` values render as opaque regex objects instead of plain strings. The current `file_serialize`/`file_deserialize` hooks on the settings macros address this per-setting, but the approach doesn't compose — nested types like Duration inside a struct require the parent to manually handle the transformation.

## Changes

### New trait: `SettingsValue`

A trait that setting value types implement to define their TOML file representation:

```rust
pub trait SettingsValue: Serialize + DeserializeOwned {
    fn to_file_value(&self) -> serde_json::Value { ... }      // default: serde passthrough
    fn from_file_value(value: &Value) -> Option<Self> { ... }  // default: serde passthrough
    fn file_schema(gen: &mut SchemaGenerator) -> Schema         // default: schemars passthrough
        where Self: JsonSchema { ... }
}
```

- `to_file_value` / `from_file_value` — parallel serialization path to serde for the settings file. Defaults delegate to `serde_json::to_value`/`from_value`.
- `file_schema` — returns the JSON Schema describing the file representation. Defaults to `gen.subschema_for::<Self>()`. Override when the file representation differs from serde (e.g. Duration → u64 schema instead of `{secs, nanos}` object).

`SettingsValue` is a supertrait bound on `Setting::Value`.

### Derive macro: `#[derive(SettingsValue)]`

A proc-macro crate (`settings_value_derive`) generates impls:

- **Named-field structs**: Recursive serialization — each field calls `to_file_value()`/`from_file_value()` on the field type. Respects `#[serde(rename)]`, `#[serde(skip)]`, `#[serde(default)]`, and `Option<T>` fields.
- **Newtype structs** (`struct Foo(T)`): Delegates to the inner type's impl.
- **Enums**: Serde passthrough (equivalent to empty `impl SettingsValue for T {}`). Recursive data-carrying variant support deferred to branch 2.

The derive is gated behind a `derive` feature on the `settings_value` crate and re-exported as `settings_value::SettingsValue`.

### Crate structure

Two new crates:

- **`settings_value`** — trait definition, primitive impls, collection impls, Duration impl. Dependencies: `serde`, `serde_json`, `instant`, `schemars`. Optional `derive` feature.
- **`settings_value_derive`** — proc-macro crate for `#[derive(SettingsValue)]`. Dependencies: `proc-macro2`, `quote`, `syn`.

The `settings` crate depends on `settings_value` (with `derive` feature). The `app` crate depends on it for custom impls. `warpui_core` gets impls behind an optional `settings_value` feature flag.

### Impls by category

**Primitives and standard types** — shipped in `settings_value` with serde passthrough impls:
`bool`, `u8`, `u16`, `u32`, `u64`, `usize`, `i8`, `i16`, `i32`, `i64`, `f32`, `f64`, `String`, `PathBuf`.

**Generic collections** — recursive impls in `settings_value`:
`Vec<T>`, `Option<T>`, `HashSet<T>` recursively call `to_file_value`/`from_file_value` on elements. `HashMap<K, V>` uses serde passthrough.

**Duration** — manual impl in `settings_value`:
`to_file_value` returns integer seconds, `from_file_value` parses integer to `Duration::from_secs`, `file_schema` returns `u64` schema.

**`AgentModeCommandExecutionPredicate`** — manual impl in `app/`:
`to_file_value` returns the regex as a JSON string, `from_file_value` parses via `new_regex`. The `Vec<T>` impl then makes `Vec<AgentModeCommandExecutionPredicate>` serialize as an array of strings automatically.

**Structs with custom serialization needs** (e.g. `NotificationsSettings`) — use the derive. The struct derive recursively calls trait methods on each field, so Duration fields automatically serialize as integers without manual patching. The `#[schemars(with = "u64")]` annotation on Duration fields ensures schema correctness.

**Most enum and struct types** — use `#[derive(SettingsValue)]` at the type definition site. ~40 types across the app crate.

**Exceptions requiring manual passthrough impls:**
- `CycleInfo`, `AIRequestQuotaInfo` — contain `DateTime<Utc>` (foreign type, orphan rule prevents implementing `SettingsValue` for it)
- `warpui_core` types (`Weight`, `ThinStrokes`, `GraphicsBackend`, `AccessibilityVerbosity`, `DisplayIdx`) — kept in a centralized `cfg`-gated module due to optional feature dependency

### Integration with settings macros

The `file_serialize`, `file_deserialize`, and `file_value_type` macro parameters are removed from `define_setting!`, `implement_setting_for_enum!`, and `define_settings_group!`.

The `submit_schema_entry!` macro uses `SettingsValue::file_schema` instead of `JsonSchema::json_schema` for schema generation, ensuring the schema reflects the file representation rather than the serde representation.

### Schema validation test

A test in the app crate (`app/src/settings/schema_validation_tests.rs`) validates that every registered public setting's `file_default_value_fn` output conforms to its `file_schema`. This catches mismatches like the Duration case (schema says `{secs, nanos}` object but file value is integer `30`). The test uses `jsonschema::draft202012` for validation and covers all ~3000 settings linked via `inventory` in the app crate.

### Behavioral changes to TOML output

- `Duration` values change from `{ nanos = 0, secs = 30 }` to `30` (integer seconds)
- `AgentModeCommandExecutionPredicate` values change from opaque regex objects to plain strings
- All other types are unchanged (serde-delegating impls produce the same output as before)

## Follow-ups
- Snake_case serialization for enum variants in the TOML file (e.g. `ShowAndCollapse` → `show_and_collapse`) via the derive macro's enum arm — branch 2 (`daniel/snake-case-enums`).
- Recursive data-carrying variant support in the enum derive (requires all inner types to impl `SettingsValue`) — also branch 2.
