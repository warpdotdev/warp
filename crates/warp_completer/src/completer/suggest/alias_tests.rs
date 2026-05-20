use std::collections::HashMap;

use warp_util::path::ShellFamily;

use crate::completer::expand_command_aliases;
use crate::completer::testing::{FakeCompletionContext, MockGeneratorContext};
use crate::signatures::testing::{create_test_command_registry, test_signature};

#[test]
pub fn test_expand_command_aliases() {
    let registry = create_test_command_registry([test_signature()]);

    let generator_ctx = MockGeneratorContext::for_test_signature();
    let mut aliases = HashMap::new();
    aliases.insert("aliasForTest".into(), "test".to_owned());
    let ctx = FakeCompletionContext::new(registry)
        .with_generator_context(generator_ctx)
        .with_aliases(aliases);

    // Simple case: there's a command we don't have an alias for
    let result = warpui::r#async::block_on(expand_command_aliases(
        "normalCommandWithoutAlias ",
        false,
        &ctx,
    ));
    assert_eq!(result.expanded_command_line, "normalCommandWithoutAlias ");
    assert_eq!(
        result.tokens_from_command,
        vec!["normalCommandWithoutAlias"]
    );
    assert!(result.signature_for_completions.is_none());

    // We have a top-level "aliasForTest" which expands to "test".
    let result = warpui::r#async::block_on(expand_command_aliases("aliasForTest ", false, &ctx));
    assert_eq!(result.expanded_command_line, "test ");
    assert_eq!(result.tokens_from_command, vec!["test"]);
    #[cfg(not(feature = "v2"))]
    assert_eq!(
        result
            .signature_for_completions
            .expect("should have signature for completions")
            .signature
            .name(),
        "test"
    );

    #[cfg(not(feature = "v2"))]
    {
        // The test signature has an alias function, which expands subcommand "twelve" to "one".
        let result = warpui::r#async::block_on(expand_command_aliases("test twelve ", false, &ctx));
        assert_eq!(result.expanded_command_line, "test one ");
        assert_eq!(result.tokens_from_command, vec!["test", "one"]);
        // Should be using the subcommand signature for completions
        assert_eq!(
            result
                .signature_for_completions
                .expect("should have signature for completions")
                .signature
                .name(),
            "one"
        );

        // We have a top-level aliasForTest which expands to test, and then the test signature expands "twelve" to "one"
        let result =
            warpui::r#async::block_on(expand_command_aliases("aliasForTest twelve ", false, &ctx));
        assert_eq!(result.expanded_command_line, "test one ");
        assert_eq!(result.tokens_from_command, vec!["test", "one"]);
        // Should be using the subcommand signature for completions
        assert_eq!(
            result
                .signature_for_completions
                .expect("should have signature for completions")
                .signature
                .name(),
            "one"
        );
    }
}

#[test]
pub fn test_expand_command_aliases_env_vars() {
    let registry = create_test_command_registry([test_signature()]);

    let generator_ctx = MockGeneratorContext::for_test_signature();
    let mut aliases = HashMap::new();
    aliases.insert("aliasForTest".into(), "test".to_owned());
    let ctx = FakeCompletionContext::new(registry)
        .with_generator_context(generator_ctx)
        .with_aliases(aliases);

    // We have a top-level "aliasForTest" which expands to "test".
    let result = warpui::r#async::block_on(expand_command_aliases(
        "ENV1=VAL1 ENV2=VAL2 aliasForTest ",
        false,
        &ctx,
    ));
    assert_eq!(result.expanded_command_line, "ENV1=VAL1 ENV2=VAL2 test ");
    // Tokens should not include env vars
    assert_eq!(result.tokens_from_command, vec!["test"]);
    // Should have env vars in classified command.
    assert_eq!(
        result
            .classified_command
            .expect("should have classified command")
            .env_vars,
        vec!["ENV1=VAL1", "ENV2=VAL2"]
    );
    #[cfg(not(feature = "v2"))]
    assert_eq!(
        result
            .signature_for_completions
            .expect("should have signature for completions")
            .signature
            .name(),
        "test"
    );

    #[cfg(not(feature = "v2"))]
    {
        // The test signature has an alias function, which expands subcommand "twelve" to "one".
        let result = warpui::r#async::block_on(expand_command_aliases(
            "ENV1=VAL1 ENV2=VAL2 test twelve ",
            false,
            &ctx,
        ));
        assert_eq!(
            result.expanded_command_line,
            "ENV1=VAL1 ENV2=VAL2 test one "
        );
        // Tokens should not include env vars
        assert_eq!(result.tokens_from_command, vec!["test", "one"]);
        // Should have env vars in classified command.
        assert_eq!(
            result
                .classified_command
                .expect("should have classified command")
                .env_vars,
            vec!["ENV1=VAL1", "ENV2=VAL2"]
        );
        // Should be using the subcommand signature for completions
        assert_eq!(
            result
                .signature_for_completions
                .expect("should have signature for completions")
                .signature
                .name(),
            "one"
        );

        // We have a top-level aliasForTest which expands to test, and then the test signature expands "twelve" to "one"
        let result = warpui::r#async::block_on(expand_command_aliases(
            "ENV1=VAL1 ENV2=VAL2 aliasForTest twelve ",
            false,
            &ctx,
        ));
        assert_eq!(
            result.expanded_command_line,
            "ENV1=VAL1 ENV2=VAL2 test one "
        );
        // Tokens should not include env vars
        assert_eq!(result.tokens_from_command, vec!["test", "one"]);
        // Should have env vars in classified command.
        assert_eq!(
            result
                .classified_command
                .expect("should have classified command")
                .env_vars,
            vec!["ENV1=VAL1", "ENV2=VAL2"]
        );
        // Should be using the subcommand signature for completions
        assert_eq!(
            result
                .signature_for_completions
                .expect("should have signature for completions")
                .signature
                .name(),
            "one"
        );
    }
}

#[test]
pub fn test_expand_command_aliases_should_not_expand_if_no_space_after_alias() {
    let registry = create_test_command_registry([test_signature()]);

    let generator_ctx = MockGeneratorContext::for_test_signature();
    let mut aliases = HashMap::new();
    aliases.insert("aliasForTest".into(), "test".to_owned());
    let ctx = FakeCompletionContext::new(registry)
        .with_generator_context(generator_ctx)
        .with_aliases(aliases);

    // We have a top-level "aliasForTest" which expands to "test", but there's no trailing space so we shouldn't expand.
    let result = warpui::r#async::block_on(expand_command_aliases("aliasForTest", false, &ctx));
    assert_eq!(result.expanded_command_line, "aliasForTest");
    assert_eq!(result.tokens_from_command, vec!["aliasForTest"]);
    assert!(result.signature_for_completions.is_none());

    // The test signature has an alias function which expands subcommand "twelve" to "one", but there's no trailing space so we shouldn't expand.
    let result = warpui::r#async::block_on(expand_command_aliases("test twelve", false, &ctx));
    assert_eq!(result.expanded_command_line, "test twelve");
    assert_eq!(result.tokens_from_command, vec!["test", "twelve"]);
    // "twelve" isn't a valid subcommand, so we should use the "test" signature.
    #[cfg(not(feature = "v2"))]
    assert_eq!(
        result
            .signature_for_completions
            .expect("should have signature for completions")
            .signature
            .name(),
        "test"
    );

    // We have a top-level aliasForTest which expands to test. But the test signature does not expand "twelve" to "one" because there's no trailing space.
    let result =
        warpui::r#async::block_on(expand_command_aliases("aliasForTest twelve", false, &ctx));
    assert_eq!(result.expanded_command_line, "test twelve");
    assert_eq!(result.tokens_from_command, vec!["test", "twelve"]);
    // "twelve" isn't a valid subcommand, so we should use the "test" signature.
    #[cfg(not(feature = "v2"))]
    assert_eq!(
        result
            .signature_for_completions
            .expect("should have signature for completions")
            .signature
            .name(),
        "test"
    );
}

#[test]
pub fn test_expand_command_aliases_case_insensitive_for_powershell() {
    let registry = create_test_command_registry([test_signature()]);

    let generator_ctx = MockGeneratorContext::for_test_signature();
    let mut aliases = HashMap::new();
    aliases.insert("aliasForTest".into(), "test".to_owned());
    let ctx = FakeCompletionContext::new(registry)
        .with_generator_context(generator_ctx)
        .with_aliases(aliases)
        .with_shell_family(ShellFamily::PowerShell);

    let result = warpui::r#async::block_on(expand_command_aliases("ALIASFORTEST ", false, &ctx));
    assert_eq!(result.expanded_command_line, "test ");

    let result = warpui::r#async::block_on(expand_command_aliases("aliasfortest ", false, &ctx));
    assert_eq!(result.expanded_command_line, "test ");

    let result = warpui::r#async::block_on(expand_command_aliases("ALIASFORTEST", false, &ctx));
    assert_eq!(result.expanded_command_line, "ALIASFORTEST");
}

#[test]
pub fn test_expand_command_aliases_case_sensitive_for_posix() {
    let registry = create_test_command_registry([test_signature()]);

    let generator_ctx = MockGeneratorContext::for_test_signature();
    let mut aliases = HashMap::new();
    aliases.insert("aliasForTest".into(), "test".to_owned());
    let ctx = FakeCompletionContext::new(registry)
        .with_generator_context(generator_ctx)
        .with_aliases(aliases)
        .with_shell_family(ShellFamily::Posix);

    let result = warpui::r#async::block_on(expand_command_aliases("ALIASFORTEST ", false, &ctx));
    assert_eq!(result.expanded_command_line, "ALIASFORTEST ");

    let result = warpui::r#async::block_on(expand_command_aliases("aliasForTest ", false, &ctx));
    assert_eq!(result.expanded_command_line, "test ");
}

#[test]
pub fn test_expand_command_aliases_multiple_commands() {
    let registry = create_test_command_registry([test_signature()]);

    let generator_ctx = MockGeneratorContext::for_test_signature();
    let mut aliases = HashMap::new();
    aliases.insert("aliasForTest".into(), "test".to_owned());
    let ctx = FakeCompletionContext::new(registry)
        .with_generator_context(generator_ctx)
        .with_aliases(aliases);

    // We have a top-level "aliasForTest" which expands to "test".
    let result = warpui::r#async::block_on(expand_command_aliases(
        "kubectl get pod && ENV1=VAL1 ENV2=VAL2 aliasForTest ",
        false,
        &ctx,
    ));
    assert_eq!(
        result.expanded_command_line,
        "kubectl get pod && ENV1=VAL1 ENV2=VAL2 test "
    );

    #[cfg(not(feature = "v2"))]
    {
        // The test signature has an alias function, which expands subcommand "twelve" to "one".
        let result = warpui::r#async::block_on(expand_command_aliases(
            "kubectl get pod && ENV1=VAL1 ENV2=VAL2 test twelve ",
            false,
            &ctx,
        ));
        assert_eq!(
            result.expanded_command_line,
            "kubectl get pod && ENV1=VAL1 ENV2=VAL2 test one "
        );

        // Multiple commands should all have their aliases expanded.
        // It is a known issue that only the last command is expanded currently.
        // TODO(INT-830): fix this case, it should expand to "ENV1=VAL1 ENV2=VAL2 test && ENV3=VAL3 ENV3=VAL3 test "
        let result = warpui::r#async::block_on(expand_command_aliases(
            "ENV1=VAL1 ENV2=VAL2 aliasForTest && ENV3=VAL3 ENV3=VAL3 aliasForTest ",
            false,
            &ctx,
        ));
        assert_eq!(
            result.expanded_command_line,
            "ENV1=VAL1 ENV2=VAL2 aliasForTest && ENV3=VAL3 ENV3=VAL3 test "
        );
    }
}
