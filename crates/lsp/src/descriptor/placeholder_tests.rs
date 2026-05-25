use super::*;
use serial_test::serial;
use std::path::PathBuf;

fn ctx() -> LspPlaceholderContext {
    LspPlaceholderContext::new(
        PathBuf::from("/work/project"),
        "abcdef0123456789".to_string(),
        PathBuf::from("/cache/lsp/some-server"),
    )
}

#[test]
fn substitutes_workspace_root() {
    let out = expand("--root={{workspace_root}}", &ctx());
    assert_eq!(out, "--root=/work/project");
}

#[test]
fn substitutes_workspace_slug() {
    let out = expand("data/{{workspace_slug}}/state", &ctx());
    assert_eq!(out, "data/abcdef0123456789/state");
}

#[test]
fn substitutes_cache_dir() {
    let out = expand("-data {{cache_dir}}/workspaces", &ctx());
    assert_eq!(out, "-data /cache/lsp/some-server/workspaces");
}

#[test]
#[serial]
fn substitutes_env_var() {
    unsafe {
        std::env::set_var("LSP_DESCRIPTOR_TEST_PLACEHOLDER_VAR", "hello");
    }
    let out = expand("{{env_LSP_DESCRIPTOR_TEST_PLACEHOLDER_VAR}}", &ctx());
    assert_eq!(out, "hello");
    unsafe {
        std::env::remove_var("LSP_DESCRIPTOR_TEST_PLACEHOLDER_VAR");
    }
}

#[test]
#[serial]
fn missing_env_var_expands_to_empty_string() {
    unsafe {
        std::env::remove_var("LSP_DESCRIPTOR_TEST_PLACEHOLDER_DEFINITELY_UNSET");
    }
    let out = expand(
        "before:{{env_LSP_DESCRIPTOR_TEST_PLACEHOLDER_DEFINITELY_UNSET}}:after",
        &ctx(),
    );
    assert_eq!(out, "before::after");
}

#[test]
fn unknown_placeholder_passes_through_verbatim() {
    let out = expand("plain {{nope}} text", &ctx());
    assert_eq!(out, "plain {{nope}} text");
}

#[test]
fn named_placeholder_whitelist_is_enforced() {
    // Guard test: only the documented placeholders resolve to context values.
    // Names that look plausible (`data_dir`, `config_dir`, `home`,
    // `app_version`, etc.) must NOT resolve unless explicitly added to the
    // whitelist.
    for name in [
        "data_dir",
        "config_dir",
        "state_dir",
        "home",
        "app_version",
        "app_name",
        "channel",
        "os",
        "arch",
        "user",
    ] {
        let template = format!("X{{{{{name}}}}}Y");
        let out = expand(&template, &ctx());
        assert_eq!(
            out, template,
            "placeholder `{{{{{name}}}}}` unexpectedly resolved; \
             named-placeholder whitelist now has an extra entry",
        );
    }
}

#[test]
#[serial]
fn substitution_is_single_pass() {
    // If the substituted value contains `{{...}}` syntax, it is not
    // re-expanded. The Handlebars engine renders against the substitution
    // map exactly once.
    unsafe {
        std::env::set_var("LSP_DESCRIPTOR_TEST_SINGLE_PASS", "{{workspace_root}}");
    }
    let out = expand("{{env_LSP_DESCRIPTOR_TEST_SINGLE_PASS}}", &ctx());
    assert_eq!(out, "{{workspace_root}}");
    unsafe {
        std::env::remove_var("LSP_DESCRIPTOR_TEST_SINGLE_PASS");
    }
}

#[test]
fn multiple_placeholders_in_one_string() {
    let out = expand("{{workspace_root}}/sub/{{workspace_slug}}", &ctx());
    assert_eq!(out, "/work/project/sub/abcdef0123456789");
}

#[test]
fn leading_tilde_is_expanded() {
    // Detailed home-expansion behavior is covered by `warp_util` tests;
    // here we only confirm that `expand` actually invokes the home-prefix
    // helper (i.e. the leading `~` does not survive verbatim when a home
    // dir is available).
    let out = expand("~/bin/lsp-server", &ctx());
    if let Some(home) = std::env::var_os("HOME") {
        let expected = format!("{}/bin/lsp-server", home.to_string_lossy());
        assert_eq!(out, expected);
    }
}

#[test]
fn embedded_tilde_not_expanded() {
    let out = expand("/opt/~/bin", &ctx());
    assert_eq!(out, "/opt/~/bin");
}

#[test]
fn expand_json_substitutes_string_leaves_only() {
    let input = serde_json::json!({
        "java": {
            "import": { "gradle": { "enabled": true } },
            "home": "{{workspace_root}}/jdk",
            "args": ["-Xmx1G", "-Dfoo={{workspace_slug}}"],
            "timeout": 30,
        }
    });
    let out = expand_json(&input, &ctx());
    let expected = serde_json::json!({
        "java": {
            "import": { "gradle": { "enabled": true } },
            "home": "/work/project/jdk",
            "args": ["-Xmx1G", "-Dfoo=abcdef0123456789"],
            "timeout": 30,
        }
    });
    assert_eq!(out, expected);
}

#[test]
fn expand_json_passes_through_non_strings() {
    let input = serde_json::json!({ "a": 1, "b": false, "c": null, "d": [1, 2, 3] });
    let out = expand_json(&input, &ctx());
    assert_eq!(out, input);
}

#[test]
fn unknown_placeholder_dedupe_within_same_context() {
    let context = ctx();
    let _ = expand("{{weird}} {{weird}}", &context);
    let _ = expand("{{weird}}", &context);
    let warned = context.warned.lock().unwrap();
    assert_eq!(warned.len(), 1);
    assert!(warned.contains("weird"));
}
