use super::CommandRegistry;
pub(crate) mod legacy;

pub use legacy::*;

pub fn create_test_command_registry(
    signatures: impl IntoIterator<Item = warp_command_signatures::Signature>,
) -> CommandRegistry {
    use std::collections::HashMap;

    let generators = HashMap::from([test_generators().into()]);
    CommandRegistry::new_for_test(signatures, generators)
}

pub(crate) const TEST_GENERATOR_1_COMMAND: &str = "echo 1";
pub(crate) const TEST_GENERATOR_2_COMMAND: &str = "echo 2";
pub(crate) const TEST_ALIAS_COMMAND: &str = "echo alias";
