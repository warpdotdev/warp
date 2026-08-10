use tempfile::tempdir;

use super::*;
use crate::plugins::identity::{
    PluginScopeId, PluginSourceId, PluginSourceKind, filesystem_safe_segment,
};

fn instance(
    scope: PluginScopeId,
    kind: PluginSourceKind,
    identity: &str,
    name: &str,
) -> PluginInstanceId {
    PluginInstanceId::new(scope, PluginSourceId::new(kind, identity), name)
}

fn user_instance(name: &str) -> PluginInstanceId {
    instance(
        PluginScopeId::User,
        PluginSourceKind::AgentsDirectory,
        "/home/alex/.agents",
        name,
    )
}

#[test]
fn the_data_directory_is_outside_the_package_and_under_the_locator_root() {
    let locator = LocalPluginDataLocator::new("/data", PluginFrontend::Gui);
    let dir = locator.data_dir(&user_instance("devtools"));
    assert!(dir.starts_with("/data/plugins/data"));
    assert_eq!(dir.parent().unwrap(), locator.root());
}

/// §9.1: the directory is dedicated to one instance and survives package changes, so the key must
/// depend on identity that does not change with the package's contents or version.
#[test]
fn the_key_is_stable_for_one_instance() {
    let first = plugin_data_instance_key(PluginFrontend::Gui, &user_instance("devtools"));
    let second = plugin_data_instance_key(PluginFrontend::Gui, &user_instance("devtools"));
    assert_eq!(first, second);
    assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn the_key_separates_instances_that_must_not_share_data() {
    let baseline = plugin_data_instance_key(PluginFrontend::Gui, &user_instance("devtools"));

    // A different front-end must not share writable state or running processes.
    let other_frontend = plugin_data_instance_key(PluginFrontend::Tui, &user_instance("devtools"));
    // A different plugin name.
    let other_name = plugin_data_instance_key(PluginFrontend::Gui, &user_instance("other"));
    // The same name in a repository rather than the user's home.
    let other_scope = plugin_data_instance_key(
        PluginFrontend::Gui,
        &instance(
            PluginScopeId::Repository,
            PluginSourceKind::AgentsDirectory,
            "/repos/one",
            "devtools",
        ),
    );
    // The same name in a different repository.
    let other_repository = plugin_data_instance_key(
        PluginFrontend::Gui,
        &instance(
            PluginScopeId::Repository,
            PluginSourceKind::AgentsDirectory,
            "/repos/two",
            "devtools",
        ),
    );
    // The same repository, but the `.warp` provider rather than `.agents`.
    let other_provider = plugin_data_instance_key(
        PluginFrontend::Gui,
        &instance(
            PluginScopeId::Repository,
            PluginSourceKind::WarpDirectory,
            "/repos/one",
            "devtools",
        ),
    );

    let keys = [
        baseline,
        other_frontend,
        other_name,
        other_scope,
        other_repository,
        other_provider,
    ];
    for (index, key) in keys.iter().enumerate() {
        for (other_index, other) in keys.iter().enumerate() {
            if index != other_index {
                assert_ne!(key, other, "keys {index} and {other_index} must differ");
            }
        }
    }
}

/// Two field values that concatenate to the same bytes must still produce different keys.
#[test]
fn field_boundaries_cannot_be_confused() {
    let first = plugin_data_instance_key(
        PluginFrontend::Gui,
        &instance(
            PluginScopeId::Agent {
                name: "a".to_owned(),
            },
            PluginSourceKind::FactoryRepository,
            "/factory",
            "b",
        ),
    );
    let second = plugin_data_instance_key(
        PluginFrontend::Gui,
        &instance(
            PluginScopeId::Agent {
                name: "a/b".to_owned(),
            },
            PluginSourceKind::FactoryRepository,
            "/factory",
            "",
        ),
    );
    assert_ne!(first, second);
}

#[test]
fn ensure_data_dir_creates_the_directory() {
    let temp = tempdir().unwrap();
    let locator = LocalPluginDataLocator::new(temp.path(), PluginFrontend::Gui);
    let instance = user_instance("devtools");

    assert!(!locator.data_dir(&instance).exists());
    let created = locator.ensure_data_dir(&instance).unwrap();
    assert!(created.is_dir());
    assert_eq!(created, locator.data_dir(&instance));
}

// ---------------------------------------------------------------------------
// Factory persistent-data contract.
//
// The composed path shape and the segment sanitization below are a cross-repo
// contract shared with warp-server. The worked examples are duplicated on the
// Go side; a divergence shows up as these tests failing rather than as plugin
// data quietly landing in the wrong place.
// ---------------------------------------------------------------------------

/// The exact shape the worker contract fixes: `<WARP_PLUGIN_DATA_ROOT>/<scope>/<plugin-key>`.
///
/// The root already carries the Factory UID, so nothing below it mentions one.
#[test]
fn the_factory_path_is_the_root_plus_scope_and_plugin_key() {
    let locator = FactoryPluginDataLocator::new(
        "/cache/warp/plugin-data/fac_01HZY",
        Some("fac_01HZY".to_owned()),
    );

    let cases = [
        (
            PluginScopeId::Factory,
            "acme-tools",
            "/cache/warp/plugin-data/fac_01HZY/factory/acme-tools",
        ),
        (
            PluginScopeId::Agent {
                name: "release".to_owned(),
            },
            "acme-tools",
            "/cache/warp/plugin-data/fac_01HZY/agent-release/acme-tools",
        ),
        (
            PluginScopeId::Automation {
                name: "nightly".to_owned(),
            },
            "release.tools",
            "/cache/warp/plugin-data/fac_01HZY/automation-nightly/release.tools",
        ),
    ];

    for (scope, name, expected) in cases {
        let instance = instance(
            scope,
            PluginSourceKind::FactoryRepository,
            "/checkout",
            name,
        );
        assert_eq!(
            locator.data_dir(&instance),
            std::path::PathBuf::from(expected)
        );
    }
}

/// The UID is recorded but never composed. Appending it again would build a second layout
/// underneath the one the worker already created.
#[test]
fn the_factory_uid_never_enters_the_path() {
    let locator = FactoryPluginDataLocator::new(
        "/durable/plugin-data/fac_01HZY",
        Some("fac_01HZY".to_owned()),
    );
    let instance = instance(
        PluginScopeId::Factory,
        PluginSourceKind::FactoryRepository,
        "/checkout",
        "acme-tools",
    );

    assert_eq!(locator.factory_uid(), Some("fac_01HZY"));
    assert_eq!(
        locator.data_dir(&instance),
        std::path::PathBuf::from("/durable/plugin-data/fac_01HZY/factory/acme-tools")
    );
    // Exactly one occurrence: the one the server put in the root.
    assert_eq!(
        locator
            .data_dir(&instance)
            .to_string_lossy()
            .matches("fac_01HZY")
            .count(),
        1
    );
}

/// The Factory layout must not nest the local one, which is the defect this type exists to stop.
#[test]
fn the_factory_layout_is_not_the_local_layout() {
    let factory = FactoryPluginDataLocator::new("/durable/plugin-data/fac_01HZY", None);
    let factory_instance = instance(
        PluginScopeId::Factory,
        PluginSourceKind::FactoryRepository,
        "/checkout",
        "acme-tools",
    );
    let path = factory
        .data_dir(&factory_instance)
        .to_string_lossy()
        .into_owned();
    assert!(
        !path.contains("plugins/data"),
        "the local layout must not appear under a Factory root: {path}"
    );

    // Local plugins keep the hashed layout, unchanged.
    let local = LocalPluginDataLocator::new("/data", PluginFrontend::Gui);
    assert!(
        local
            .data_dir(&user_instance("acme-tools"))
            .starts_with("/data/plugins/data")
    );
}

/// An agent name comes from a repository, so it must not be able to climb out of the root.
#[test]
fn an_author_controlled_scope_name_cannot_escape_the_root() {
    let locator = FactoryPluginDataLocator::new("/durable/plugin-data/fac_01HZY", None);

    for hostile in ["../../etc", "..", ".", "a/b", "a\\b", "", "Mixed Case"] {
        let instance = instance(
            PluginScopeId::Agent {
                name: hostile.to_owned(),
            },
            PluginSourceKind::FactoryRepository,
            "/checkout",
            "acme-tools",
        );
        let path = locator.data_dir(&instance);
        let below_root = path
            .strip_prefix("/durable/plugin-data/fac_01HZY")
            .unwrap_or_else(|_| panic!("'{hostile}' escaped: {}", path.display()));
        // Exactly the flat scope segment and the plugin key below the root, and no component
        // of it is a parent reference.
        assert_eq!(
            below_root.components().count(),
            2,
            "'{hostile}' produced an unexpected depth: {}",
            path.display()
        );
        assert!(
            !below_root
                .components()
                .any(|c| c.as_os_str() == std::ffi::OsStr::new("..")),
            "'{hostile}' produced a parent reference: {}",
            path.display()
        );
    }
}

/// Two hostile names that reduce to the same visible text must still get separate directories.
#[test]
fn distinct_names_that_sanitize_alike_do_not_collide() {
    let locator = FactoryPluginDataLocator::new("/durable/plugin-data/fac_01HZY", None);
    let path_for = |name: &str| {
        locator.data_dir(&instance(
            PluginScopeId::Agent {
                name: name.to_owned(),
            },
            PluginSourceKind::FactoryRepository,
            "/checkout",
            "acme-tools",
        ))
    };
    assert_ne!(path_for("a/b"), path_for("a\\b"));
    assert_ne!(path_for(".."), path_for("."));
}

/// The sanitization rule itself, stated as worked examples.
///
/// A conformant name passes through untouched so real paths stay legible; anything else is
/// reduced and given a digest suffix that keeps it distinct.
#[test]
fn the_sanitization_rule() {
    // Unchanged: already a safe segment.
    for clean in [
        "acme-tools",
        "release.tools",
        "a",
        "lint3r",
        "with_underscore",
    ] {
        assert_eq!(filesystem_safe_segment(clean), clean);
    }

    // Transformed: reduced, then suffixed with the first four bytes of the SHA-256 of the input.
    assert_eq!(filesystem_safe_segment("a/b"), "a-b-c14cddc0");
    assert_eq!(filesystem_safe_segment("Mixed Case"), "mixed-case-0962903a");

    // Periods survive step 1, so a traversal attempt keeps them and is distinguished only by
    // the digest. This case was hand-written wrong once; it is executed here so it cannot be
    // again.
    assert_eq!(filesystem_safe_segment("../../etc"), "..-..-etc-74ccf3c5");
    assert_eq!(filesystem_safe_segment("..-..-etc"), "..-..-etc");

    // Reserved or empty: the digest alone, since there is no safe text to keep.
    assert_eq!(filesystem_safe_segment(""), "e3b0c442");
    assert_eq!(filesystem_safe_segment("."), "cdb4ee2a");
    assert_eq!(filesystem_safe_segment(".."), "5ec1f7e7");
}

/// Drives the composition through the vendored cross-repo contract's own worked examples.
///
/// This is the consuming half of the assertion warp-server makes against the same file. The
/// examples include a hostile agent name, so the sanitization is exercised here rather than only
/// in this crate's own tests.
#[test]
fn the_vendored_contract_examples_compose_identically() {
    const CONTRACT: &str = include_str!("contract/factory_plugin_runtime_contract.json");
    let contract: serde_json::Value = serde_json::from_str(CONTRACT).unwrap();

    // The variable names this client reads must be the ones the server declares it produces.
    let declared: Vec<&str> = contract["environment_variables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    for required in [
        PLUGIN_DATA_ROOT_ENV,
        FACTORY_UID_ENV,
        crate::plugins::PLUGIN_DIRS_ENV,
        crate::plugins::FACTORY_MCP_FILES_ENV,
    ] {
        assert!(
            declared.contains(&required),
            "the contract no longer declares '{required}'; it declares {declared:?}"
        );
    }

    let path_contract = &contract["plugin_data_path"];
    assert_eq!(
        path_contract["composed"].as_str().unwrap(),
        "${WARP_PLUGIN_DATA_ROOT}/<scope-segment>/<plugin-key>"
    );
    let segments_below_root = path_contract["segments_below_root"].as_u64().unwrap() as usize;

    for example in contract["examples"].as_array().unwrap() {
        let root = example["server_exports"].as_str().unwrap();
        let scope_segment = example["scope"].as_str().unwrap();
        let plugin_key = example["plugin_key"].as_str().unwrap();
        let expected = example["composed"].as_str().unwrap();

        // Rebuild the scope the contract names. The segment is flat, and the portion after the
        // prefix is already sanitized — sanitization is idempotent on a safe string — so this
        // round-trips through the real `path_segment` rather than restating it.
        let scope = if scope_segment == "factory" {
            PluginScopeId::Factory
        } else if let Some(name) = scope_segment.strip_prefix("agent-") {
            PluginScopeId::Agent {
                name: name.to_owned(),
            }
        } else if let Some(name) = scope_segment.strip_prefix("automation-") {
            PluginScopeId::Automation {
                name: name.to_owned(),
            }
        } else {
            panic!("unhandled contract scope segment '{scope_segment}'");
        };
        assert_eq!(
            scope.path_segment(),
            scope_segment,
            "the contract's scope segment did not round-trip through path_segment"
        );

        let locator =
            FactoryPluginDataLocator::new(root, example["factory_uid"].as_str().map(str::to_owned));
        let instance = instance(
            scope,
            PluginSourceKind::FactoryRepository,
            "/checkout",
            plugin_key,
        );
        let composed = locator.data_dir(&instance);
        assert_eq!(
            composed,
            std::path::PathBuf::from(expected),
            "contract example for scope '{scope_segment}' did not compose as declared"
        );
        assert_eq!(
            composed
                .strip_prefix(root)
                .expect("the composed path stays under the exported root")
                .components()
                .count(),
            segments_below_root,
            "contract example for scope '{scope_segment}' broke the segments-below-root invariant"
        );
    }
}
