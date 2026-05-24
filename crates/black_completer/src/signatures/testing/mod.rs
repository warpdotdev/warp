use super::CommandRegistry;

cfg_if::cfg_if! {
    if #[cfg(feature = "v2")] {
        mod v2;
        pub use v2::*;

        pub fn create_test_command_registry(
            signatures: impl IntoIterator<Item = super::CommandSignature>,
        ) -> CommandRegistry {
            let registry = CommandRegistry::new();
            for signature in signatures.into_iter() {
                registry.register_signature(signature);
            }
            registry
        }
    } else if #[cfg(not(feature = "v2"))]{
        pub(crate) mod legacy;

        pub use legacy::*;

        pub fn create_test_command_registry(
            signatures: impl IntoIterator<Item = warp_command_signatures::Signature>,
        ) -> CommandRegistry {
            use std::collections::HashMap;

            let generators = HashMap::from([test_generators().into()]);
            CommandRegistry::new_for_test(signatures, generators)
        }

    }
}

pub(crate) const TEST_GENERATOR_1_COMMAND: &str = "echo 1";
pub(crate) const TEST_GENERATOR_2_COMMAND: &str = "echo 2";
pub(crate) const TEST_ALIAS_COMMAND: &str = "echo alias";
