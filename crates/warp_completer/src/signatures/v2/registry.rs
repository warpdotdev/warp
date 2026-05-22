//! `CommandRegistry` contains all registered `CommandSignature`s that are eligible for use in
//! completion suggestion generation.
//!
//! Completion engine callers must supply a `CommandRegistry` in their `CompletionContext`
//! implementation to generate suggestions for registered commands.
use std::sync::{Arc, OnceLock};

use memo_map::MemoMap;

use super::CommandSignature;

static GLOBAL_REGISTRY: OnceLock<Arc<CommandRegistry>> = OnceLock::new();

#[derive(Debug, Default, Clone)]
pub struct CommandRegistry {
    signatures: MemoMap<String, CommandSignature>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            signatures: MemoMap::default(),
        }
    }

    pub fn get_signature(&self, command: impl AsRef<str>) -> Option<&CommandSignature> {
        self.signatures.get(command.as_ref())
    }

    pub fn register_signature(&self, signature: CommandSignature) {
        self.signatures
            .insert(signature.command.name.clone(), signature);
    }

    pub fn registered_commands(&self) -> impl Iterator<Item = &str> {
        self.signatures.keys().map(|key| key.as_str())
    }

    pub fn global_instance() -> Arc<Self> {
        GLOBAL_REGISTRY
            .get_or_init(|| {
                // TODO(wasm): Determine how to asynchronously load command signatures on wasm.
                Arc::new(CommandRegistry::new())
            })
            .clone()
    }

    pub fn empty() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod test;
