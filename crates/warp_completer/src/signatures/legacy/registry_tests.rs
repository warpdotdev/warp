use crate::completer::testing::FakeCompletionContext;
use crate::completer::CompletionContext;
use crate::completer::TopLevelCommandCaseSensitivity;
use crate::signatures::registry::SignatureResult;
use crate::signatures::testing::{create_test_command_registry, test_signature};

#[test]
fn test_find_command_from_a_top_level_signature() {
    let bundle = warp_command_signatures::signature_by_name("bundle")
        .expect("global command signatures should include 'bundle'");

    let registry = create_test_command_registry([bundle.clone(), test_signature()]);

    let found_signature = registry
        .signature_from_line(
            "bundle exec ",
            TopLevelCommandCaseSensitivity::CaseSensitive,
        )
        .map(|c| c.signature);
    let signature = bundle;
    let exec_subcommand = signature
        .subcommands()
        .iter()
        .find(|sig| sig.name() == "exec");
    assert_eq!(found_signature, exec_subcommand);
}

#[test]
fn test_find_subcommand_signature_with_flags() {
    let kubectl = warp_command_signatures::signature_by_name("kubectl")
        .expect("global command signatures should include 'kubectl'");

    let registry = create_test_command_registry([kubectl.clone(), test_signature()]);

    let found_signature = registry
        .signature_from_line(
            "kubectl -n default get ",
            TopLevelCommandCaseSensitivity::CaseSensitive,
        )
        .expect("kubectl signature from line should exist");
    // Should parse this as entering the "get" subcommand even though there's a top level -n default flag.
    assert_eq!(found_signature.signature.name(), "get");
    assert_eq!(found_signature.token_index, 3);
}

#[test]
fn test_find_option_by_name_exact_match_does_not_match_substring() {
    // Regression test: "-n" should match the "-n"/"--namespace" option, NOT
    // "--no-headers" (which contains the substring "-n"). The fix uses exact
    // equality instead of `contains`.
    let kubectl = warp_command_signatures::signature_by_name("kubectl")
        .expect("global command signatures should include 'kubectl'");

    let registry = create_test_command_registry([kubectl, test_signature()]);

    // "kubectl -n default api-resources " should resolve to "api-resources",
    // which has a "--no-headers" option. If "-n" incorrectly matched
    // "--no-headers" via substring, the parser would skip "default" as the
    // flag argument and never reach the "api-resources" subcommand.
    let found_signature = registry
        .signature_from_line(
            "kubectl -n default api-resources ",
            TopLevelCommandCaseSensitivity::CaseSensitive,
        )
        .expect("kubectl signature from line should exist");
    assert_eq!(found_signature.signature.name(), "api-resources");
}

#[test]
fn test_flag_arg_consumes_token_matching_subcommand_name() {
    // When a recognized flag takes a required argument, the next token should be consumed
    // as that flag's argument even if it happens to match a subcommand name.
    // Here, --not-long takes 1 argument, so "one" is consumed as that argument
    // rather than being resolved as the "one" subcommand.
    let registry = create_test_command_registry([test_signature()]);

    let found_signature = registry
        .signature_from_line(
            "test --not-long one foo ",
            TopLevelCommandCaseSensitivity::CaseSensitive,
        )
        .expect("test signature from line should exist");
    assert_eq!(found_signature.signature.name(), "test");
    assert_eq!(found_signature.token_index, 0);
}

#[test]
fn test_multiple_switches_before_subcommand() {
    // Multiple switch flags (no arguments) before a subcommand should all be
    // skipped, allowing the subcommand to be discovered.
    let registry = create_test_command_registry([test_signature()]);

    let found_signature = registry
        .signature_from_line(
            "test -r -V one foo ",
            TopLevelCommandCaseSensitivity::CaseSensitive,
        )
        .expect("test signature from line should exist");
    assert_eq!(found_signature.signature.name(), "one");
    assert_eq!(found_signature.token_index, 3);
}

#[test]
fn test_unrecognized_flag_skipped_before_subcommand() {
    // Unrecognized flags (tokens starting with '-' not in the spec) should be
    // skipped so the parser can still discover subcommands after them.
    let registry = create_test_command_registry([test_signature()]);

    let found_signature = registry
        .signature_from_line(
            "test --unknown-flag one foo ",
            TopLevelCommandCaseSensitivity::CaseSensitive,
        )
        .expect("test signature from line should exist");
    assert_eq!(found_signature.signature.name(), "one");
    assert_eq!(found_signature.token_index, 2);
}

#[test]
fn test_flag_with_missing_value_at_end_of_input() {
    // When a flag that takes a required argument appears at the end of input
    // with no value provided, the resolved signature stays on the parent command.
    let registry = create_test_command_registry([test_signature()]);

    let found_signature = registry
        .signature_from_line(
            "test --not-long ",
            TopLevelCommandCaseSensitivity::CaseSensitive,
        )
        .expect("test signature from line should exist");
    assert_eq!(found_signature.signature.name(), "test");
    assert_eq!(found_signature.token_index, 0);
}

#[test]
fn test_multiple_flags_with_values_before_subcommand() {
    // Multiple valued flags before a subcommand should all be skipped,
    // allowing the subcommand to be discovered.
    let registry = create_test_command_registry([test_signature()]);

    // --not-long takes 1 required arg ("val"), -r is a switch.
    let found_signature = registry
        .signature_from_line(
            "test --not-long val -r one foo ",
            TopLevelCommandCaseSensitivity::CaseSensitive,
        )
        .expect("test signature from line should exist");
    assert_eq!(found_signature.signature.name(), "one");
    assert_eq!(found_signature.token_index, 4);
}

#[test]
fn test_only_flags_no_subcommand() {
    // When only flags appear after the command with no following subcommand,
    // the parent command should be returned.
    let registry = create_test_command_registry([test_signature()]);

    let found_signature = registry
        .signature_from_line(
            "test --not-long val ",
            TopLevelCommandCaseSensitivity::CaseSensitive,
        )
        .expect("test signature from line should exist");
    assert_eq!(found_signature.signature.name(), "test");
    assert_eq!(found_signature.token_index, 0);
}

#[test]
fn test_optional_flag_arg_does_not_consume_subcommand() {
    // --required-and-optional-args has 1 required arg + 1 optional arg.
    // The parser should only skip the required arg, so "one" is found as a
    // subcommand rather than being consumed as the optional arg.
    let registry = create_test_command_registry([test_signature()]);

    let found_signature = registry
        .signature_from_line(
            "test --required-and-optional-args val one foo ",
            TopLevelCommandCaseSensitivity::CaseSensitive,
        )
        .expect("test signature from line should exist");
    assert_eq!(found_signature.signature.name(), "one");
    assert_eq!(found_signature.token_index, 3);
}

#[test]
fn test_alias_expansion_path_skips_flag_with_value_before_subcommand() {
    // Exercises signature_with_alias_expansion (not just signature_from_tokens)
    // to ensure the alias-expansion code path also skips flags before subcommands.
    let kubectl = warp_command_signatures::signature_by_name("kubectl")
        .expect("global command signatures should include 'kubectl'");

    let registry = create_test_command_registry([kubectl]);
    let ctx = FakeCompletionContext::new(registry).with_case_sensitivity();

    let result = warpui::r#async::block_on(ctx.command_registry().signature_with_alias_expansion(
        &["kubectl", "-n", "default", "get"],
        true,
        &ctx,
    ));
    let SignatureResult::Success(found_signature) = result else {
        panic!("expected SignatureResult::Success");
    };
    assert_eq!(found_signature.signature.name(), "get");
    assert_eq!(found_signature.token_index, 3);
}

#[test]
fn test_alias_expansion_path_skips_multiple_flags_before_subcommand() {
    // Exercises signature_with_alias_expansion with multiple flags (valued and
    // switch) placed before the subcommand.
    let kubectl = warp_command_signatures::signature_by_name("kubectl")
        .expect("global command signatures should include 'kubectl'");

    let registry = create_test_command_registry([kubectl]);
    let ctx = FakeCompletionContext::new(registry).with_case_sensitivity();

    let result = warpui::r#async::block_on(ctx.command_registry().signature_with_alias_expansion(
        &[
            "kubectl",
            "--context",
            "staging-cluster",
            "-n",
            "project1",
            "get",
        ],
        true,
        &ctx,
    ));
    let SignatureResult::Success(found_signature) = result else {
        panic!("expected SignatureResult::Success");
    };
    assert_eq!(found_signature.signature.name(), "get");
    assert_eq!(found_signature.token_index, 5);
}
