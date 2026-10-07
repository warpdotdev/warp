//! Defines the [`SettingsValue`] trait for controlling how setting value
//! types are serialized to and deserialized from the user-visible TOML settings
//! file.
//!
//! The trait provides a parallel serialization path to serde: types implement
//! `to_file_value` / `from_file_value` to produce a human-friendly JSON
//! representation that the TOML backend converts to TOML. The default
//! implementation delegates to serde, so types without custom file formatting
//! need only an empty `impl SettingsValue for T {}`.
//!
//! Cloud sync and platform-native stores (UserDefaults, registry) continue
//! using serde directly — this trait is only consulted when writing to or
//! reading from the settings file.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use instant::Duration;
use serde::Serialize;
use serde::de::DeserializeOwned;
// Re-export the derive macro when available.
use serde_json::Value;
#[cfg(feature = "derive")]
pub use settings_value_derive::SettingsValue;

/// Defines how a type is represented in the user-visible settings file.
///
/// The default implementation delegates to serde (`serde_json::to_value` /
/// `serde_json::from_value`), so types that don't need a custom file
/// representation can use an empty impl:
///
/// ```ignore
/// impl SettingsValue for MyType {}
/// ```
///
/// Types that want a different representation override the methods.  For
/// example, `Duration` serializes as an integer (seconds) rather than the
/// serde `{ secs, nanos }` object.
///
/// # Choosing an implementation strategy
///
/// **`#[derive(SettingsValue)]`** — the default choice for most types.
/// For enums, the derive converts variant names to snake_case and
/// recursively serializes inner data via `SettingsValue`.  For structs,
/// it recursively calls `to_file_value`/`from_file_value` on each field.
/// The derive **bypasses serde entirely** — it does not call
/// `serde_json::to_value`.
///
/// **Empty `impl SettingsValue for T {}`** (serde passthrough) — use when
/// the type has custom `Serialize`/`Deserialize` impls that already
/// produce the desired file format (e.g. `StartupShell` serializes as
/// `Option<String>`, `SyncId` flattens to a plain string).  The
/// passthrough delegates to serde, so those custom impls are respected.
/// Also use this for types in external crates where you cannot add a
/// derive attribute (orphan rule).
///
/// **Manual impl with overridden methods** — use when neither the derive
/// nor the serde output is suitable.  For example,
/// `AgentModeCommandExecutionPredicate` serializes as a plain regex
/// string in the file, which neither the derive nor serde would produce.
pub trait SettingsValue: Serialize + DeserializeOwned {
    /// Converts this value to a JSON representation for the settings file.
    fn to_file_value(&self) -> Value {
        serde_json::to_value(self).expect("serializable type should convert to Value")
    }

    /// Reconstructs this value from a JSON representation read from the
    /// settings file.  Returns `None` if the value cannot be parsed.
    fn from_file_value(value: &Value) -> Option<Self>
    where
        Self: Sized,
    {
        serde_json::from_value(value.clone()).ok()
    }

    /// Returns the JSON Schema describing the file representation of this type.
    ///
    /// The default delegates to `schemars::JsonSchema`, which is correct for
    /// passthrough types.  Override when `to_file_value` produces a different
    /// shape than serde (e.g. Duration → integer seconds).
    fn file_schema(sgen: &mut schemars::SchemaGenerator) -> schemars::Schema
    where
        Self: schemars::JsonSchema,
    {
        sgen.subschema_for::<Self>()
    }
}

// ---------------------------------------------------------------------------
// Convenience macro for external types
// ---------------------------------------------------------------------------

/// Implements `SettingsValue` as a serde passthrough for types outside this
/// crate.  The listed types should have appropriate `#[serde(rename_all = …)]`
/// attributes on their definitions to ensure the desired serialization format
/// in the settings file.
#[macro_export]
macro_rules! impl_snake_case {
    ($($ty:ty),* $(,)?) => {
        $(impl $crate::SettingsValue for $ty {})*
    };
}

// ---------------------------------------------------------------------------
// Primitive impls (serde passthrough)
// ---------------------------------------------------------------------------

macro_rules! impl_default_file_format {
    ($($ty:ty),* $(,)?) => {
        $(impl SettingsValue for $ty {})*
    };
}

impl_default_file_format!(
    bool,
    u8,
    u16,
    u32,
    u64,
    usize,
    i8,
    i16,
    i32,
    i64,
    f32,
    f64,
    String,
    PathBuf,
    DateTime<Utc>,
);

// ---------------------------------------------------------------------------
// Generic collection impls (recursive)
// ---------------------------------------------------------------------------

impl<T: SettingsValue> SettingsValue for Vec<T> {
    fn to_file_value(&self) -> Value {
        Value::Array(self.iter().map(T::to_file_value).collect())
    }

    fn from_file_value(value: &Value) -> Option<Self> {
        value.as_array()?.iter().map(T::from_file_value).collect()
    }
}

impl<T: SettingsValue> SettingsValue for Option<T> {
    fn to_file_value(&self) -> Value {
        match self {
            Some(v) => v.to_file_value(),
            None => Value::Null,
        }
    }

    fn from_file_value(value: &Value) -> Option<Self> {
        if value.is_null() {
            Some(None)
        } else {
            Some(Some(T::from_file_value(value)?))
        }
    }
}

impl<T> SettingsValue for HashSet<T>
where
    T: SettingsValue + Eq + Hash,
{
    fn to_file_value(&self) -> Value {
        Value::Array(self.iter().map(T::to_file_value).collect())
    }

    fn from_file_value(value: &Value) -> Option<Self> {
        value.as_array()?.iter().map(T::from_file_value).collect()
    }
}

impl<K, V> SettingsValue for HashMap<K, V>
where
    K: SettingsValue + Eq + Hash,
    V: SettingsValue,
{
    fn to_file_value(&self) -> Value {
        let mut obj = serde_json::Map::new();
        for (k, v) in self {
            let key_str = match k.to_file_value() {
                Value::String(s) => s,
                other => other.to_string(),
            };
            obj.insert(key_str, v.to_file_value());
        }
        Value::Object(obj)
    }

    fn from_file_value(value: &Value) -> Option<Self> {
        let obj = value.as_object()?;
        let mut map = HashMap::new();
        for (key_str, val) in obj {
            let k = K::from_file_value(&Value::String(key_str.clone()))?;
            let v = V::from_file_value(val)?;
            map.insert(k, v);
        }
        Some(map)
    }
}

// ---------------------------------------------------------------------------
// Duration — serialize as integer seconds
// ---------------------------------------------------------------------------

impl SettingsValue for Duration {
    fn to_file_value(&self) -> Value {
        Value::Number(self.as_secs().into())
    }

    fn from_file_value(value: &Value) -> Option<Self> {
        value.as_u64().map(Duration::from_secs)
    }

    fn file_schema(sgen: &mut schemars::SchemaGenerator) -> schemars::Schema
    where
        Self: schemars::JsonSchema,
    {
        sgen.subschema_for::<u64>()
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
