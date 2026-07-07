use std::collections::HashSet;
use std::sync::Arc;

use smol_str::SmolStr;
use warp_completer::completer::{GeneratorContext, PathCompletionContext};
use warp_completer::signatures::CommandRegistry;

/// An implementation of `CompletionContext` for testing purposes.
pub struct CompletionContext {
    command_registry: Arc<CommandRegistry>,
}

impl CompletionContext {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            command_registry: CommandRegistry::global_instance(),
        }
    }
}

impl warp_completer::completer::CompletionContext for CompletionContext {
    fn top_level_commands(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        Box::new(self.command_registry.registered_commands())
    }

    fn command_registry(&self) -> &CommandRegistry {
        &self.command_registry
    }

    fn environment_variable_names(&self) -> Option<&HashSet<SmolStr>> {
        None
    }

    fn shell_supports_autocd(&self) -> Option<bool> {
        None
    }

    fn path_completion_context(&self) -> Option<&dyn PathCompletionContext> {
        None
    }

    fn generator_context(&self) -> Option<&dyn GeneratorContext> {
        None
    }
}
