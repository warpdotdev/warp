use warp_command_signatures::{Priority, Signature};
use warp_core::channel::Channel;

use crate::completer::testing::FakeCompletionContext;
use crate::completer::{CompletionContext, TopLevelCommandCaseSensitivity};
use crate::signatures::registry::{MAX_CACHEABLE_COMMAND_LEN, SignatureResult};
use crate::signatures::testing::{create_test_command_registry, test_signature};

/// A minimal signature with the given `name`, for exercising `SignatureCache` boundary
/// conditions that don't care about arguments, subcommands, or options.
fn signature_with_name(name: &str) -> Signature {
    Signature {
        name: name.to_string(),
        alias_generator: None,
        description: None,
        arguments: None,
        subcommands: None,
        options: None,
        priority: Priority::default(),
        parser_directives: Default::default(),
    }
}

/// Recursively finds the longest `Signature::name` in `signature` or any of its (possibly
/// nested) subcommands, updating `longest` in place if a longer one is found.
fn track_longest_name(signature: &Signature, longest: &mut (usize, String)) {
    if signature.name.len() > longest.0 {
        *longest = (signature.name.len(), signature.name.clone());
    }
    for subcommand in signature.subcommands() {
        track_longest_name(subcommand, longest);
    }
}

#[test]
fn test_all_known_signature_names_are_within_the_length_cap() {
    let mut longest = (0, String::new());

    for signature in warp_command_signatures::commands() {
        track_longest_name(&signature, &mut longest);
    }

    for channel in [Channel::Stable, Channel::Preview, Channel::Dev] {
        let mut clap_cmd = <warp_cli::Args as clap::CommandFactory>::command();
        let signature = crate::signatures::clap::signature_from_clap_command(
            &mut clap_cmd,
            channel.cli_command_name(),
        );
        track_longest_name(&signature, &mut longest);
    }

    let (max_len, longest_name) = longest;
    assert!(
        max_len <= MAX_CACHEABLE_COMMAND_LEN,
        "found a command/subcommand name of length {max_len} ({longest_name:?}), longer than \
         MAX_CACHEABLE_COMMAND_LEN ({MAX_CACHEABLE_COMMAND_LEN}); SignatureCache::get's \
         oversized-token fast path assumes no such name exists and would make this one \
         unresolvable -- raise MAX_CACHEABLE_COMMAND_LEN or add back a fallback lookup path for \
         oversized tokens"
    );
}

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

    let result =
        warpui_core::r#async::block_on(ctx.command_registry().signature_with_alias_expansion(
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
fn test_oversized_command_is_not_cached_and_resolves_to_none() {
    let registry = create_test_command_registry([test_signature()]);
    let positive_len_before = registry.signatures.signatures.len();
    let negative_len_before = registry.signatures.misses.len();

    let oversized_command = "a".repeat(MAX_CACHEABLE_COMMAND_LEN + 1);
    assert_eq!(registry.signature(&oversized_command), None);
    assert_eq!(
        registry.signatures.signatures.len(),
        positive_len_before,
        "looking up an oversized command should not add an entry to the positive cache"
    );
    assert_eq!(
        registry.signatures.misses.len(),
        negative_len_before,
        "looking up an oversized command should not add an entry to the negative cache either -- \
         that would just move the leak this cap exists to fix into the negative cache"
    );
}

#[test]
fn test_misses_are_never_cached_in_the_positive_cache() {
    let registry = create_test_command_registry([test_signature()]);
    let len_before = registry.signatures.signatures.len();

    assert_eq!(registry.signature("not-a-real-command"), None);
    assert_eq!(registry.signature("not-a-real-command"), None);
    assert_eq!(registry.signatures.signatures.len(), len_before);
}

#[test]
fn test_oversized_later_token_does_not_bypass_the_length_guard() {
    let sudo = warp_command_signatures::signature_by_name("sudo")
        .expect("global command signatures should include 'sudo'");
    let registry = create_test_command_registry([sudo]);
    let positive_len_before = registry.signatures.signatures.len();
    let negative_len_before = registry.signatures.misses.len();

    let oversized_token = "a".repeat(MAX_CACHEABLE_COMMAND_LEN + 1);
    let tokens = ["sudo", oversized_token.as_str()];

    let found_signature = registry
        .signature_from_tokens(
            &tokens,
            false,
            TopLevelCommandCaseSensitivity::CaseSensitive,
        )
        .expect("sudo signature from tokens should exist");
    assert_eq!(
        found_signature.signature.name(),
        "sudo",
        "an oversized later token shouldn't resolve to a replacement signature"
    );
    assert_eq!(
        registry.signatures.signatures.len(),
        positive_len_before,
        "looking up an oversized later token should not add an entry to the positive cache"
    );
    assert_eq!(
        registry.signatures.misses.len(),
        negative_len_before,
        "looking up an oversized later token should not add an entry to the negative cache"
    );

    let sudo = warp_command_signatures::signature_by_name("sudo")
        .expect("global command signatures should include 'sudo'");
    let ctx =
        FakeCompletionContext::new(create_test_command_registry([sudo])).with_case_sensitivity();
    let result = warpui_core::r#async::block_on(
        ctx.command_registry()
            .signature_with_alias_expansion(&tokens, false, &ctx),
    );
    let SignatureResult::Success(found_signature) = result else {
        panic!("expected SignatureResult::Success");
    };
    assert_eq!(found_signature.signature.name(), "sudo");
}

#[test]
fn test_ordinary_commands_still_resolve_and_are_cached() {
    let registry = create_test_command_registry([test_signature()]);

    let found = registry.signature("TEST");
    assert_eq!(found.map(|s| s.name.as_str()), Some("test"));

    let len_after_first_lookup = registry.signatures.signatures.len();
    assert_eq!(registry.signature("test"), found);
    assert_eq!(registry.signatures.signatures.len(), len_after_first_lookup);
}

#[test]
fn test_registered_signature_longer_than_the_cap_is_unresolvable() {
    let long_name = "a".repeat(MAX_CACHEABLE_COMMAND_LEN + 1);
    let registry = create_test_command_registry([signature_with_name(&long_name)]);

    assert_eq!(registry.signature(&long_name), None);
}

#[cfg(windows)]
#[test]
fn test_exe_suffix_is_trimmed_before_the_length_check() {
    let max_length_name = "a".repeat(MAX_CACHEABLE_COMMAND_LEN);
    let registry = create_test_command_registry([signature_with_name(&max_length_name)]);

    let token = format!("{max_length_name}.exe");
    assert_eq!(
        registry.signature(&token).map(|s| s.name.as_str()),
        Some(max_length_name.as_str())
    );
}

#[test]
fn test_registered_commands_unaffected_by_oversized_lookups() {
    let registry = create_test_command_registry([test_signature()]);

    let oversized_command = "a".repeat(MAX_CACHEABLE_COMMAND_LEN + 1);
    assert_eq!(registry.signature(&oversized_command), None);
    assert_eq!(registry.signature("not-a-real-command"), None);

    let registered = registry.registered_commands().collect::<Vec<_>>();
    assert_eq!(registered, vec!["test"]);
}

#[test]
fn test_alias_expansion_path_skips_multiple_flags_before_subcommand() {
    // Exercises signature_with_alias_expansion with multiple flags (valued and
    // switch) placed before the subcommand.
    let kubectl = warp_command_signatures::signature_by_name("kubectl")
        .expect("global command signatures should include 'kubectl'");

    let registry = create_test_command_registry([kubectl]);
    let ctx = FakeCompletionContext::new(registry).with_case_sensitivity();

    let result =
        warpui_core::r#async::block_on(ctx.command_registry().signature_with_alias_expansion(
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
