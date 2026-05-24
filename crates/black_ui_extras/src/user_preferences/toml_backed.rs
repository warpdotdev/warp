//! Implementation of the [`UserPreferences`] trait using a TOML file for
//! persistence, with support for hierarchical sections and snake_case keys.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use toml_edit::{value, Array, DocumentMut, InlineTable, Item, Table, Value};

use super::Error;

/// Indentation used per nesting level when pretty-printing multi-line arrays
/// and inline tables.
const INDENT: &str = "  ";

/// Rough line-length budget before an inline container breaks across lines.
///
/// Arrays and inline tables whose default single-line rendering exceeds this
/// width get pretty-printed across multiple lines. This is measured against
/// the longest line of the container's current rendering, so already-broken
/// children don't defeat the check.
const MAX_INLINE_WIDTH: usize = 100;

/// An implementation of the [`UserPreferences`] trait using a TOML file for
/// persistence.
///
/// Settings are organized into hierarchical sections based on the `hierarchy`
/// metadata from the `Setting` trait. Keys are automatically converted from
/// CamelCase to snake_case for idiomatic TOML.
///
/// Values are stored as native TOML types (booleans, integers, floats, strings)
/// rather than JSON-encoded strings, making the file human-readable and
/// hand-editable.
pub struct TomlBackedUserPreferences {
    /// The path to the TOML file that backs this preferences store.
    file_path: PathBuf,

    /// The in-memory TOML document, preserving formatting and comments.
    document: RefCell<DocumentMut>,

    /// When `true`, writes are silently skipped to avoid overwriting a
    /// broken settings file with defaults. Set when the initial parse
    /// fails; cleared when [`reload_from_disk`](Self::reload_from_disk)
    /// succeeds.
    write_inhibited: Cell<bool>,

    /// Storage keys whose writes are individually inhibited because the
    /// value in the TOML file could not be deserialized into the expected
    /// type. Writes and removes for these keys are silently skipped to
    /// preserve the user's broken-but-fixable value.
    ///
    /// Cleared on successful [`reload_from_disk`](Self::reload_from_disk)
    /// and re-derived by the settings reload logic.
    write_inhibited_keys: RefCell<HashSet<String>>,
}

impl TomlBackedUserPreferences {
    /// Constructs a new TOML-backed user preferences store.
    ///
    /// If no file exists at the given path, an empty document will be used,
    /// and any modifications will trigger creation of the file (including
    /// any missing parent directories).
    ///
    /// If the file exists but contains invalid TOML, the store is created
    /// with an empty document (so all settings fall back to defaults) and
    /// the parse error is returned in the second tuple element. This
    /// ensures the caller always gets a functional preferences backend
    /// that can recover via [`reload_from_disk`](Self::reload_from_disk)
    /// when the user fixes the file.
    pub fn new(file_path: PathBuf) -> (Self, Option<Error>) {
        let (document, write_inhibited, error) = match Self::load_document(file_path.as_path()) {
            Ok(doc) => (doc, false, None),
            Err(err) => {
                log::warn!(
                    "Failed to parse settings file at {}: {err}; starting with empty defaults",
                    file_path.display(),
                );
                (DocumentMut::new(), true, Some(err))
            }
        };
        (
            Self {
                file_path,
                document: RefCell::new(document),
                write_inhibited: Cell::new(write_inhibited),
                write_inhibited_keys: RefCell::new(HashSet::new()),
            },
            error,
        )
    }

    /// Loads the TOML document from disk, or returns an empty document if
    /// the file doesn't exist.
    fn load_document(file_path: &Path) -> Result<DocumentMut, Error> {
        let file_contents = match std::fs::read_to_string(file_path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DocumentMut::new());
            }
            Err(err) => return Err(err.into()),
        };

        if file_contents.trim().is_empty() {
            return Ok(DocumentMut::new());
        }

        file_contents
            .parse::<DocumentMut>()
            .map_err(|err| Error::Unknown(anyhow::anyhow!(err)))
    }

    /// Hashes the settings file content on disk.
    ///
    /// Returns `None` if the file is missing, empty/whitespace-only, or
    /// unreadable. These cases are all treated as "no local state" rather
    /// than "local state that should win" — the caller's startup
    /// comparison logic treats a `None` result as "no differing local
    /// state" so that cloud can restore rather than wiping cloud with
    /// local defaults.
    ///
    /// Uses SHA-256 so that persisted hashes are stable across Rust
    /// toolchain upgrades and crate version bumps (unlike `SipHasher`
    /// or `DefaultHasher`, whose output is not guaranteed to be stable).
    pub fn file_content_hash(file_path: &Path) -> Option<String> {
        let contents = match std::fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
            Err(err) => {
                log::warn!(
                    "Failed to read settings file at {}: {err}",
                    file_path.display()
                );
                return None;
            }
        };
        // An empty/whitespace-only file is semantically equivalent to a
        // missing file — no settings are defined. Treating them the
        // same way avoids wiping cloud with defaults if the user
        // empties the file to reset.
        if contents.trim().is_empty() {
            return None;
        }
        let digest = Sha256::digest(contents.as_bytes());
        Some(format!("{digest:x}"))
    }

    /// Reloads the TOML document from disk, replacing the in-memory contents.
    ///
    /// On parse failure (e.g. the user introduced a syntax error), the old
    /// document is kept and an error is returned.
    pub fn reload_from_disk(&self) -> Result<(), Error> {
        match Self::load_document(self.file_path.as_path()) {
            Ok(doc) => {
                *self.document.borrow_mut() = doc;
                // The file is now valid — allow writes again.
                self.write_inhibited.set(false);
                // Clear per-key inhibitions; they will be re-derived by
                // the settings reload logic for keys that still fail.
                self.write_inhibited_keys.borrow_mut().clear();
                Ok(())
            }
            Err(err) => {
                log::warn!(
                    "Failed to reload settings file at {}: {err}; keeping previous state",
                    self.file_path.display(),
                );
                Err(err)
            }
        }
    }

    /// Builds a compound key from a storage key and optional hierarchy.
    ///
    /// For example, `("FontSize", Some("font"))` → `"font.FontSize"`.
    fn compound_key(key: &str, hierarchy: Option<&str>) -> String {
        match hierarchy {
            Some(h) => format!("{h}.{key}"),
            None => key.to_owned(),
        }
    }

    /// Returns `true` if writes for the given key are individually inhibited.
    fn is_key_write_inhibited(&self, key: &str, hierarchy: Option<&str>) -> bool {
        let compound = Self::compound_key(key, hierarchy);
        self.write_inhibited_keys.borrow().contains(&compound)
    }

    /// Flushes the in-memory TOML document to disk.
    ///
    /// When writes are inhibited (because the initial parse failed), this
    /// is a silent no-op to avoid overwriting the user's broken-but-fixable
    /// file with empty defaults.
    fn flush(&self) -> Result<(), Error> {
        if self.write_inhibited.get() {
            return Ok(());
        }
        let parent_dir = self
            .file_path
            .parent()
            .expect("absolute path to file should have parent");
        std::fs::create_dir_all(parent_dir)?;

        let data = self.document.borrow().to_string();
        std::fs::write(&self.file_path, data)?;
        Ok(())
    }

    /// Navigates to or creates the table for the given hierarchy path.
    ///
    /// For example, `"font.display"` will ensure that `[font.display]` exists
    /// and return a mutable reference to that table.
    fn get_or_create_table<'a>(table: &'a mut Table, hierarchy: &str) -> &'a mut Table {
        let mut current = table;
        for segment in hierarchy.split('.') {
            if !current.contains_key(segment) || !current[segment].is_table() {
                current[segment] = Item::Table(Table::new());
            }
            current = current[segment]
                .as_table_mut()
                .expect("just ensured this is a table");
        }
        current
    }

    /// Navigates to the table for the given hierarchy path, returning `None`
    /// if any segment along the path doesn't exist or isn't a table.
    fn get_table<'a>(table: &'a Table, hierarchy: &str) -> Option<&'a Table> {
        let mut current = table;
        for segment in hierarchy.split('.') {
            current = current.get(segment)?.as_table()?;
        }
        Some(current)
    }

    /// Converts a JSON-serialized value string into a native TOML [`Item`].
    ///
    /// Primitives become native TOML types, JSON objects become TOML tables,
    /// JSON arrays become TOML arrays (with inline tables for object elements),
    /// and JSON null is omitted (`Item::None`).
    ///
    /// `remaining_depth` controls how deeply nested objects are rendered as
    /// section tables before switching to inline tables:
    /// - `None` — unlimited depth (all section tables)
    /// - `Some(0)` — fully inline (`{ key = value }`)
    /// - `Some(n)` — `n` levels of section tables, then inline
    fn json_value_to_toml_item(json_str: &str, remaining_depth: Option<u32>) -> Item {
        if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(json_str) {
            if remaining_depth == Some(0) {
                match Self::json_to_toml_value(&json_value) {
                    Some(v) => Item::Value(v),
                    None => Item::None,
                }
            } else {
                Self::json_to_toml_item(&json_value, remaining_depth)
            }
        } else {
            // If it's not valid JSON, store as a plain string.
            value(json_str)
        }
    }

    /// Recursively converts a parsed JSON value into a TOML [`Item`].
    ///
    /// JSON objects become `Item::Table` (rendered as `[section]` headers),
    /// which is more readable than inline tables for struct-valued settings.
    ///
    /// `remaining_depth` controls how many more levels of section tables to
    /// allow before switching to inline. `None` means unlimited.
    fn json_to_toml_item(json: &serde_json::Value, remaining_depth: Option<u32>) -> Item {
        match json {
            serde_json::Value::Null => Item::None,
            serde_json::Value::Bool(b) => value(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    value(i)
                } else if let Some(f) = n.as_f64() {
                    value(f)
                } else {
                    value(n.to_string())
                }
            }
            serde_json::Value::String(s) => value(s.as_str()),
            serde_json::Value::Array(arr) => {
                let mut toml_arr = toml_edit::Array::new();
                for elem in arr {
                    if let Some(v) = Self::json_to_toml_value(elem) {
                        toml_arr.push(v);
                    }
                }
                value(toml_arr)
            }
            serde_json::Value::Object(obj) => {
                let child_depth = remaining_depth.map(|d| d.saturating_sub(1));
                if child_depth == Some(0) {
                    // Children should be inline — convert each value inline.
                    let mut table = Table::new();
                    for (k, v) in obj {
                        let item = match Self::json_to_toml_value(v) {
                            Some(v) => Item::Value(v),
                            None => Item::None,
                        };
                        if !matches!(item, Item::None) {
                            table[k.as_str()] = item;
                        }
                    }
                    Item::Table(table)
                } else {
                    let mut table = Table::new();
                    for (k, v) in obj {
                        let item = Self::json_to_toml_item(v, child_depth);
                        if !matches!(item, Item::None) {
                            table[k.as_str()] = item;
                        }
                    }
                    Item::Table(table)
                }
            }
        }
    }

    /// Recursively pretty-prints an [`Item`] tree in place.
    ///
    /// Inline containers (arrays and inline tables) whose single-line
    /// rendering exceeds [`MAX_INLINE_WIDTH`], or which contain a child that
    /// itself was made multi-line, get broken across lines using [`INDENT`]
    /// per nesting level. Other items are left untouched.
    ///
    /// This is only called on freshly-built items produced by
    /// [`Self::json_value_to_toml_item`], so there's no risk of trampling
    /// over user-authored formatting for other entries in the file.
    fn prettify_item(item: &mut Item, indent_level: usize) {
        match item {
            Item::None => {}
            Item::Value(v) => Self::prettify_value(v, indent_level),
            Item::Table(t) => Self::prettify_table(t, indent_level),
            Item::ArrayOfTables(arr) => {
                for table in arr.iter_mut() {
                    Self::prettify_table(table, indent_level);
                }
            }
        }
    }

    fn prettify_value(v: &mut Value, indent_level: usize) {
        match v {
            Value::Array(arr) => Self::prettify_array(arr, indent_level),
            Value::InlineTable(t) => Self::prettify_inline_table(t, indent_level),
            Value::String(_)
            | Value::Integer(_)
            | Value::Float(_)
            | Value::Boolean(_)
            | Value::Datetime(_) => {}
        }
    }

    fn prettify_table(t: &mut Table, indent_level: usize) {
        // A section table's header sits at column 0 regardless of how deeply
        // nested it is logically, and so do its `key = value` lines. So the
        // effective indent for its children is the same as its own.
        for (_, item) in t.iter_mut() {
            Self::prettify_item(item, indent_level);
        }
    }

    fn prettify_array(arr: &mut Array, indent_level: usize) {
        // Recurse into children first so their multi-line decisions are
        // final before we look at the parent.
        for v in arr.iter_mut() {
            Self::prettify_value(v, indent_level + 1);
        }
        if arr.is_empty() {
            return;
        }
        let needs_multiline = arr.iter().any(Self::value_rendering_is_multiline)
            || Self::longest_line(&arr.to_string()) > MAX_INLINE_WIDTH;
        if !needs_multiline {
            return;
        }
        let child_indent = INDENT.repeat(indent_level + 1);
        let outer_indent = INDENT.repeat(indent_level);
        let child_prefix = format!("\n{child_indent}");
        for v in arr.iter_mut() {
            v.decor_mut().set_prefix(child_prefix.clone());
            v.decor_mut().set_suffix("");
        }
        arr.set_trailing_comma(true);
        arr.set_trailing(format!("\n{outer_indent}"));
    }

    fn prettify_inline_table(t: &mut InlineTable, indent_level: usize) {
        for (_, v) in t.iter_mut() {
            Self::prettify_value(v, indent_level + 1);
        }
        if t.is_empty() {
            return;
        }
        let needs_multiline = t.iter().any(|(_, v)| Self::value_rendering_is_multiline(v))
            || Self::longest_line(&t.to_string()) > MAX_INLINE_WIDTH;
        if !needs_multiline {
            return;
        }
        let child_indent = INDENT.repeat(indent_level + 1);
        let outer_indent = INDENT.repeat(indent_level);
        let child_prefix = format!("\n{child_indent}");
        for (mut key, v) in t.iter_mut() {
            key.leaf_decor_mut().set_prefix(child_prefix.clone());
            key.leaf_decor_mut().set_suffix(" ");
            v.decor_mut().set_prefix(" ");
            v.decor_mut().set_suffix("");
        }
        t.set_trailing_comma(true);
        t.set_trailing(format!("\n{outer_indent}"));
    }

    /// Whether a value's current rendering spans multiple lines.
    ///
    /// Used for the propagation rule: if any child container was already
    /// expanded, the parent must be expanded too to avoid ugly output like
    /// `[{\n  a = 1\n}, ...]`.
    fn value_rendering_is_multiline(v: &Value) -> bool {
        match v {
            Value::Array(_) | Value::InlineTable(_) => v.to_string().contains('\n'),
            Value::String(_)
            | Value::Integer(_)
            | Value::Float(_)
            | Value::Boolean(_)
            | Value::Datetime(_) => false,
        }
    }

    /// Returns the length (in chars) of the longest line in `s`.
    fn longest_line(s: &str) -> usize {
        s.lines()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0)
    }

    /// Converts a parsed JSON value into a TOML [`Value`](toml_edit::Value).
    ///
    /// Objects become inline tables (`key = { ... }`), keeping setting values
    /// on a single line rather than creating separate `[section]` headers.
    fn json_to_toml_value(json: &serde_json::Value) -> Option<toml_edit::Value> {
        match json {
            serde_json::Value::Null => None,
            serde_json::Value::Bool(b) => Some((*b).into()),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Some(i.into())
                } else if let Some(f) = n.as_f64() {
                    Some(f.into())
                } else {
                    Some(n.to_string().into())
                }
            }
            serde_json::Value::String(s) => Some(s.as_str().into()),
            serde_json::Value::Array(arr) => {
                let mut toml_arr = toml_edit::Array::new();
                for elem in arr {
                    if let Some(v) = Self::json_to_toml_value(elem) {
                        toml_arr.push(v);
                    }
                }
                Some(toml_edit::Value::Array(toml_arr))
            }
            serde_json::Value::Object(obj) => {
                let mut inline_table = toml_edit::InlineTable::new();
                for (k, v) in obj {
                    if let Some(toml_v) = Self::json_to_toml_value(v) {
                        inline_table.insert(k, toml_v);
                    }
                }
                Some(toml_edit::Value::InlineTable(inline_table))
            }
        }
    }

    /// Converts a TOML [`Item`] back into a JSON-serialized string that
    /// `serde_json::from_str` can parse.
    fn toml_item_to_json_string(item: &Item) -> Option<String> {
        match item {
            Item::None => None,
            Item::Value(v) => Self::toml_value_to_json(v),
            Item::Table(t) => Some(Self::toml_table_to_json(t)),
            Item::ArrayOfTables(arr) => {
                let parts: Vec<String> = arr.iter().map(Self::toml_table_to_json).collect();
                Some(format!("[{}]", parts.join(",")))
            }
        }
    }

    /// Converts a TOML [`Value`](toml_edit::Value) into a JSON string.
    fn toml_value_to_json(v: &toml_edit::Value) -> Option<String> {
        match v {
            toml_edit::Value::Boolean(b) => {
                Some(serde_json::to_string(b.value()).unwrap_or_default())
            }
            toml_edit::Value::Integer(i) => {
                Some(serde_json::to_string(i.value()).unwrap_or_default())
            }
            toml_edit::Value::Float(f) => {
                Some(serde_json::to_string(f.value()).unwrap_or_default())
            }
            toml_edit::Value::String(s) => {
                // Always JSON-encode the string value.
                Some(serde_json::to_string(s.value()).unwrap_or_default())
            }
            toml_edit::Value::Array(arr) => {
                let parts: Vec<String> = arr.iter().filter_map(Self::toml_value_to_json).collect();
                Some(format!("[{}]", parts.join(",")))
            }
            toml_edit::Value::InlineTable(t) => {
                let parts: Vec<String> = t
                    .iter()
                    .filter_map(|(k, v)| {
                        Self::toml_value_to_json(v).map(|json_v| {
                            format!(
                                "{}:{}",
                                serde_json::to_string(k).unwrap_or_default(),
                                json_v
                            )
                        })
                    })
                    .collect();
                Some(format!("{{{}}}", parts.join(",")))
            }
            _ => None,
        }
    }

    /// Converts a TOML [`Table`] into a JSON object string.
    fn toml_table_to_json(table: &Table) -> String {
        let parts: Vec<String> = table
            .iter()
            .filter_map(|(k, item)| {
                Self::toml_item_to_json_string(item).map(|json_v| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        json_v
                    )
                })
            })
            .collect();
        format!("{{{}}}", parts.join(","))
    }
}

impl super::UserPreferences for TomlBackedUserPreferences {
    fn is_settings_file(&self) -> bool {
        true
    }

    fn reload_from_disk(&self) -> Result<(), super::Error> {
        self.reload_from_disk()
    }

    fn write_value(&self, key: &str, val: String) -> Result<(), Error> {
        self.write_value_with_hierarchy(key, val, None, None)
    }

    fn read_value(&self, key: &str) -> Result<Option<String>, Error> {
        self.read_value_with_hierarchy(key, None)
    }

    fn remove_value(&self, key: &str) -> Result<(), Error> {
        self.remove_value_with_hierarchy(key, None)
    }

    fn inhibit_writes_for_key(&self, key: &str, hierarchy: Option<&str>) {
        let compound = Self::compound_key(key, hierarchy);
        log::info!("Inhibiting writes for setting key {compound}");
        self.write_inhibited_keys.borrow_mut().insert(compound);
    }

    fn clear_all_write_inhibitions(&self) {
        self.write_inhibited_keys.borrow_mut().clear();
    }

    fn write_value_with_hierarchy(
        &self,
        key: &str,
        val: String,
        hierarchy: Option<&str>,
        max_table_depth: Option<u32>,
    ) -> Result<(), Error> {
        if self.is_key_write_inhibited(key, hierarchy) {
            return Ok(());
        }

        let mut item = Self::json_value_to_toml_item(&val, max_table_depth);
        // Apply pretty-printing before inserting. The assignment always
        // lands at the top of a section table (either the root or the
        // `[hierarchy]` table), so the value's own indent level is 0 and
        // nested containers get +1 per level.
        Self::prettify_item(&mut item, 0);

        let mut doc = self.document.borrow_mut();
        let table = match hierarchy {
            Some(h) => Self::get_or_create_table(doc.as_table_mut(), h),
            None => doc.as_table_mut(),
        };
        table[key] = item;
        drop(doc);

        self.flush()
    }

    fn read_value_with_hierarchy(
        &self,
        key: &str,
        hierarchy: Option<&str>,
    ) -> Result<Option<String>, Error> {
        let doc = self.document.borrow();
        let table = match hierarchy {
            Some(h) => match Self::get_table(doc.as_table(), h) {
                Some(t) => t,
                None => return Ok(None),
            },
            None => doc.as_table(),
        };

        match table.get(key) {
            Some(item) => Ok(Self::toml_item_to_json_string(item)),
            None => Ok(None),
        }
    }

    fn remove_value_with_hierarchy(&self, key: &str, hierarchy: Option<&str>) -> Result<(), Error> {
        if self.is_key_write_inhibited(key, hierarchy) {
            return Ok(());
        }

        let mut doc = self.document.borrow_mut();
        let table = match hierarchy {
            Some(h) => {
                // Navigate to the parent table; if it doesn't exist, nothing to remove.
                let mut current = doc.as_table_mut();
                for segment in h.split('.') {
                    if !current.contains_key(segment) || !current[segment].is_table() {
                        return Ok(());
                    }
                    current = current[segment].as_table_mut().ok_or_else(|| {
                        Error::Unknown(anyhow::anyhow!(
                            "expected table at segment '{segment}' in hierarchy '{h}'"
                        ))
                    })?;
                }
                current
            }
            None => doc.as_table_mut(),
        };
        table.remove(key);
        drop(doc);

        self.flush()
    }
}

#[cfg(test)]
#[path = "toml_backed_tests.rs"]
mod tests;
