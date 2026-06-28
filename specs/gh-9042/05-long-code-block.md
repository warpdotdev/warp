# Long code block — scroll test

Scroll down until the ` ```rust ` opening line is **off the top of the screen**, then
confirm the Rust code still highlights (keywords, strings, types, comments). That
exercises the viewport-scoping / boundary fix: the block must color even when its
language declaration isn't on screen.

```rust
//! A small in-memory key/value store used to exercise syntax highlighting.

use std::collections::HashMap;
use std::fmt;

/// Errors the store can return.
#[derive(Debug)]
pub enum StoreError {
    NotFound(String),
    Capacity { limit: usize },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::NotFound(key) => write!(f, "no entry for key: {key}"),
            StoreError::Capacity { limit } => write!(f, "store is full (limit {limit})"),
        }
    }
}

/// A bounded key/value store.
pub struct Store {
    entries: HashMap<String, String>,
    limit: usize,
}

impl Store {
    /// Create a store that holds at most `limit` entries.
    pub fn with_capacity(limit: usize) -> Self {
        Self {
            entries: HashMap::new(),
            limit,
        }
    }

    /// Insert a value, returning an error if the store is at capacity.
    pub fn insert(&mut self, key: &str, value: &str) -> Result<(), StoreError> {
        if self.entries.len() >= self.limit && !self.entries.contains_key(key) {
            return Err(StoreError::Capacity { limit: self.limit });
        }
        self.entries.insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    /// Fetch a value or report that it is missing.
    pub fn get(&self, key: &str) -> Result<&str, StoreError> {
        self.entries
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| StoreError::NotFound(key.to_owned()))
    }

    /// Number of stored entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn main() {
    let mut store = Store::with_capacity(2);
    store.insert("greeting", "hello").unwrap();
    store.insert("target", "world").unwrap();

    match store.get("greeting") {
        Ok(value) => println!("greeting = {value}"),
        Err(err) => eprintln!("lookup failed: {err}"),
    }

    // This third insert should exceed the capacity of 2.
    if let Err(err) = store.insert("extra", "boom") {
        println!("expected capacity error: {err}");
    }

    println!("store holds {} entries", store.len());
}
```

End of file — the heading above and this line are plain Markdown for contrast.
