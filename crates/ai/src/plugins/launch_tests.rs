use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use tempfile::tempdir;

use super::*;

fn placeholders<'a>(root: &'a Path, data: &'a Path) -> PluginPlaceholders<'a> {
    PluginPlaceholders {
        plugin_root: root,
        plugin_data: data,
    }
}

fn expand(input: &str) -> String {
    expand_placeholders(
        input,
        &placeholders(Path::new("/plugins/devtools"), Path::new("/data/devtools")),
    )
}

#[test]
fn both_placeholders_expand_everywhere_they_appear() {
    assert_eq!(expand("${PLUGIN_ROOT}"), "/plugins/devtools");
    assert_eq!(expand("${PLUGIN_DATA}"), "/data/devtools");
    assert_eq!(
        expand("${PLUGIN_ROOT}/config.json"),
        "/plugins/devtools/config.json"
    );
    assert_eq!(
        expand("--root=${PLUGIN_ROOT} --data=${PLUGIN_DATA}"),
        "--root=/plugins/devtools --data=/data/devtools"
    );
    assert_eq!(
        expand("${PLUGIN_ROOT}${PLUGIN_ROOT}"),
        "/plugins/devtools/plugins/devtools"
    );
}

/// §9.2: unrecognized placeholder-like text stays literal, and Warp performs no other expansion.
#[test]
fn unrecognized_placeholders_stay_literal() {
    assert_eq!(expand("${HOME}/data"), "${HOME}/data");
    assert_eq!(expand("$PLUGIN_ROOT"), "$PLUGIN_ROOT");
    assert_eq!(expand("${plugin_root}"), "${plugin_root}");
    assert_eq!(expand("${PLUGIN_ROOT"), "${PLUGIN_ROOT");
    assert_eq!(expand("{{PLUGIN_ROOT}}"), "{{PLUGIN_ROOT}}");
    // A Warp-style template variable is not a plugin placeholder and must not be touched.
    assert_eq!(expand("{{api_key}}"), "{{api_key}}");
    // An unrecognized placeholder must not stop a later real one from expanding.
    assert_eq!(expand("${HOME}:${PLUGIN_DATA}"), "${HOME}:/data/devtools");
}

/// §9.2: expansion is a single pass. Text a replacement introduces is never rescanned, so a
/// plugin cannot chain one placeholder into another.
#[test]
fn expansion_is_single_pass_and_non_recursive() {
    let root = Path::new("/roots/${PLUGIN_DATA}");
    let data = Path::new("/data/real");
    assert_eq!(
        expand_placeholders("${PLUGIN_ROOT}/x", &placeholders(root, data)),
        "/roots/${PLUGIN_DATA}/x"
    );
}

#[test]
fn a_bare_command_is_left_for_the_platform_executable_search() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("plugin");
    let data = temp.path().join("data");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&data).unwrap();

    let plan = plan_stdio_launch(
        "npx",
        &[],
        &BTreeMap::new(),
        None,
        &placeholders(&root, &data),
    )
    .unwrap();
    assert_eq!(plan.command, ResolvedCommand::BareName("npx".to_owned()));
}

/// §7.2.1: an omitted `cwd` means the plugin root.
#[test]
fn an_omitted_cwd_defaults_to_the_plugin_root() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("plugin");
    let data = temp.path().join("data");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&data).unwrap();

    let plan = plan_stdio_launch(
        "server",
        &[],
        &BTreeMap::new(),
        None,
        &placeholders(&root, &data),
    )
    .unwrap();
    assert_eq!(plan.cwd, root);
}

/// The complete launch plan: argv is expanded and stays separate from the one-token command, and
/// the authoritative variables are applied last so a package cannot displace them.
#[test]
fn launch_plan_expands_argv_and_sets_authoritative_variables_last() {
    let temp = tempdir().unwrap();
    let root = dunce::canonicalize(temp.path()).unwrap().join("plugin");
    let data = dunce::canonicalize(temp.path()).unwrap().join("data");
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::write(root.join("bin").join("validator"), "#!/bin/sh\n").unwrap();
    fs::create_dir_all(&data).unwrap();

    let mut env = BTreeMap::new();
    env.insert("CONFIG".to_owned(), "${PLUGIN_ROOT}/config.json".to_owned());
    env.insert("CACHE".to_owned(), "${PLUGIN_DATA}/cache".to_owned());
    env.insert("LITERAL".to_owned(), "${HOME}/x".to_owned());

    let plan = plan_stdio_launch(
        "./bin/validator",
        &[
            "--data".to_owned(),
            "${PLUGIN_DATA}/validator".to_owned(),
            "--verbatim".to_owned(),
        ],
        &env,
        Some("${PLUGIN_ROOT}"),
        &placeholders(&root, &data),
    )
    .unwrap();

    assert_eq!(
        plan.command,
        ResolvedCommand::PluginRelative(root.join("bin").join("validator"))
    );
    assert_eq!(
        plan.args,
        vec![
            "--data".to_owned(),
            format!("{}/validator", data.display()),
            "--verbatim".to_owned(),
        ]
    );
    assert_eq!(plan.cwd, root);

    // The two authoritative variables must be the final entries, in this order, so that they
    // overwrite anything the configured environment set.
    let names: Vec<&str> = plan.env.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        vec!["CACHE", "CONFIG", "LITERAL", "PLUGIN_ROOT", "PLUGIN_DATA"]
    );
    let values: BTreeMap<&str, &str> = plan
        .env
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(values["CONFIG"], format!("{}/config.json", root.display()));
    assert_eq!(values["LITERAL"], "${HOME}/x");
    assert_eq!(values["PLUGIN_ROOT"], root.to_string_lossy());
    assert_eq!(values["PLUGIN_DATA"], data.to_string_lossy());
}

/// §7.2.1: a `${PLUGIN_DATA}`-rooted `cwd` is contained in the data directory, not the package.
#[test]
fn a_plugin_data_rooted_cwd_is_contained_in_the_data_directory() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("plugin");
    let data = temp.path().join("data");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&data).unwrap();

    let plan = plan_stdio_launch(
        "server",
        &[],
        &BTreeMap::new(),
        Some("${PLUGIN_DATA}/work"),
        &placeholders(&root, &data),
    )
    .unwrap();
    assert_eq!(plan.cwd, dunce::canonicalize(&data).unwrap().join("work"));
}

#[cfg(unix)]
#[test]
fn a_cwd_that_escapes_its_containment_root_is_rejected() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("plugin");
    let data = temp.path().join("data");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&data).unwrap();
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
    std::os::unix::fs::symlink(&outside, data.join("escape")).unwrap();

    for cwd in ["./escape", "${PLUGIN_ROOT}/escape", "${PLUGIN_DATA}/escape"] {
        let diagnostic = plan_stdio_launch(
            "server",
            &[],
            &BTreeMap::new(),
            Some(cwd),
            &placeholders(&root, &data),
        )
        .unwrap_err();
        assert_eq!(
            diagnostic.code,
            crate::plugins::PluginDiagnosticCode::PathEscapesPluginRoot,
            "'{cwd}' should be rejected"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_command_symlinked_out_of_the_plugin_root_is_rejected() {
    let temp = tempdir().unwrap();
    let root = temp.path().join("plugin");
    let data = temp.path().join("data");
    let outside = temp.path().join("outside");
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::create_dir_all(&data).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("evil"), "#!/bin/sh\n").unwrap();
    std::os::unix::fs::symlink(outside.join("evil"), root.join("bin").join("server")).unwrap();

    let diagnostic = plan_stdio_launch(
        "./bin/server",
        &[],
        &BTreeMap::new(),
        None,
        &placeholders(&root, &data),
    )
    .unwrap_err();
    assert_eq!(
        diagnostic.code,
        crate::plugins::PluginDiagnosticCode::PathEscapesPluginRoot
    );
}
