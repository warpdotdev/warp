use crate::signatures::{Command, Priority};

use super::*;

fn test_signature() -> CommandSignature {
    CommandSignature {
        command: Command {
            name: "test".to_owned(),
            alias: vec![],
            description: None,
            arguments: vec![],
            subcommands: vec![],
            options: vec![],
            priority: Priority::default(),
        },
    }
}

#[test]
fn test_command_registry_registers_signature() {
    let registry = CommandRegistry::new();
    registry.register_signature(test_signature());

    let signature = registry
        .get_signature("test")
        .expect("Signature is registered.");
    assert_eq!(signature.command.name, "test");
}
