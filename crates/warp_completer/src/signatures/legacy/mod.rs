use std::sync::{Arc, OnceLock};

mod miss_cache;
pub mod registry;

pub use registry::CommandRegistry;
#[cfg(feature = "test-util")]
use warp_command_signatures::Signature;

static GLOBAL_REGISTRY: OnceLock<Arc<CommandRegistry>> = OnceLock::new();

impl CommandRegistry {
    /// Returns a reference to a single global instance of the command registry.
    ///
    /// The registry is a read-only store of information used to provide smart
    /// suggestions and completions, and as such, only one instance is required
    /// across the application.  The registry itself can be quite large, so use
    /// of a single global instance avoids unnecessary memory allocations and
    /// usage.
    pub fn global_instance() -> Arc<Self> {
        GLOBAL_REGISTRY
            .get_or_init(|| {
                // TODO(wasm): Determine how to asynchronously load command signatures on wasm.
                Arc::new(CommandRegistry::new_with_embedded_signatures())
            })
            .clone()
    }

    /// Returns a new [`CommandRegistry`] that looks up commands in the embedded
    /// set of command signatures.
    fn new_with_embedded_signatures() -> Self {
        CommandRegistry::new(
            |command| {
                let start = instant::Instant::now();
                let signature = warp_command_signatures::signature_by_name(command);
                log::debug!(
                    "Lazily loaded command signature for {command} in {}s",
                    start.elapsed().as_secs_f32()
                );
                signature
            },
            warp_command_signatures::dynamic_command_signature_data(),
        )
    }

    /// Returns an empty [`CommandRegistry`] that contains no signatures nor
    /// generators.
    pub fn empty() -> Self {
        CommandRegistry::new(|_| None, std::collections::HashMap::new())
    }

    /// Returns a [`CommandRegistry`] that uses the provided set of signatures
    /// and generators.  This does not utilize any data from the
    /// warp-command-signatures crate.
    #[cfg(feature = "test-util")]
    pub fn new_for_test(
        signatures: impl IntoIterator<Item = Signature>,
        generators: std::collections::HashMap<
            String,
            warp_command_signatures::DynamicCompletionData,
        >,
    ) -> Self {
        let registry = CommandRegistry::new(|_| None, generators);
        signatures
            .into_iter()
            .for_each(|signature| registry.register_signature(signature));
        registry
    }
}

// We only implement Default for this in tests, as in production, we should
// always use the shared instance, but in tests, we might want to configure
// instances differently.
#[cfg(feature = "test-util")]
impl Default for CommandRegistry {
    fn default() -> Self {
        CommandRegistry::new_with_embedded_signatures()
    }
}
