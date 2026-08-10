use std::collections::BTreeMap;
use std::path::PathBuf;

use serial_test::serial;

use super::*;

/// Sets the three contract variables for the duration of `body`, then restores the previous
/// values. `from_env` reads the real process environment, so these tests are serialized.
fn with_env<T>(vars: &[(&str, Option<&str>)], body: impl FnOnce() -> T) -> T {
    let previous: Vec<(String, Option<String>)> = vars
        .iter()
        .map(|(name, _)| ((*name).to_owned(), std::env::var(name).ok()))
        .collect();
    for (name, value) in vars {
        match value {
            // SAFETY: the `serial` attribute keeps these tests off other threads that would
            // otherwise read the environment concurrently.
            Some(value) => unsafe { std::env::set_var(name, value) },
            None => unsafe { std::env::remove_var(name) },
        }
    }
    let result = body();
    for (name, value) in previous {
        match value {
            Some(value) => unsafe { std::env::set_var(&name, value) },
            None => unsafe { std::env::remove_var(&name) },
        }
    }
    result
}

fn stdio_server(name: &str) -> PluginMcpServer {
    PluginMcpServer {
        name: name.to_owned(),
        transport: PluginMcpTransport::Stdio {
            command: "server".to_owned(),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: None,
        },
    }
}

fn http_server(name: &str) -> PluginMcpServer {
    PluginMcpServer {
        name: name.to_owned(),
        transport: PluginMcpTransport::StreamableHttp {
            url: "https://example.com/mcp".to_owned(),
            headers: Vec::new(),
        },
    }
}

#[test]
#[serial]
fn the_contract_preserves_worker_ordering_and_resolves_against_the_working_directory() {
    let runtime = with_env(
        &[
            (
                PLUGIN_DIRS_ENV,
                Some("repo/automations/nightly/plugins,repo/agents/release/plugins,repo/plugins"),
            ),
            (
                FACTORY_MCP_FILES_ENV,
                Some("repo/automations/nightly/mcp.json,/abs/repo/mcp.json"),
            ),
            (PLUGIN_DATA_ROOT_ENV, Some("/durable/plugin-data")),
        ],
        || FactoryPluginRuntime::from_env(Path::new("/work")),
    );

    // Most specific first, exactly as the worker ordered them.
    assert_eq!(
        runtime.plugin_collection_dirs,
        vec![
            PathBuf::from("/work/repo/automations/nightly/plugins"),
            PathBuf::from("/work/repo/agents/release/plugins"),
            PathBuf::from("/work/repo/plugins"),
        ]
    );
    // An absolute entry passes through; a relative one is anchored to the working directory.
    assert_eq!(
        runtime.factory_mcp_files,
        vec![
            PathBuf::from("/work/repo/automations/nightly/mcp.json"),
            PathBuf::from("/abs/repo/mcp.json"),
        ]
    );
    assert_eq!(
        runtime.plugin_data_root,
        Some(PathBuf::from("/durable/plugin-data"))
    );
    assert!(runtime.allows_stdio_plugin_servers());
}

#[test]
#[serial]
fn an_absent_contract_yields_an_empty_runtime() {
    let runtime = with_env(
        &[
            (PLUGIN_DIRS_ENV, None),
            (FACTORY_MCP_FILES_ENV, None),
            (PLUGIN_DATA_ROOT_ENV, None),
        ],
        || FactoryPluginRuntime::from_env(Path::new("/work")),
    );
    assert_eq!(runtime, FactoryPluginRuntime::default());
    assert!(!runtime.allows_stdio_plugin_servers());
}

#[test]
#[serial]
fn blank_and_whitespace_entries_are_dropped() {
    let runtime = with_env(
        &[
            (PLUGIN_DIRS_ENV, Some(" a , ,b, ")),
            (FACTORY_MCP_FILES_ENV, Some("")),
            (PLUGIN_DATA_ROOT_ENV, None),
        ],
        || FactoryPluginRuntime::from_env(Path::new("/work")),
    );
    assert_eq!(
        runtime.plugin_collection_dirs,
        vec![PathBuf::from("/work/a"), PathBuf::from("/work/b")]
    );
    assert!(runtime.factory_mcp_files.is_empty());
}

/// A relative data root is not durable in any meaningful sense, so it is treated as absent
/// rather than silently anchored somewhere.
#[test]
#[serial]
fn a_relative_plugin_data_root_is_rejected() {
    let runtime = with_env(
        &[
            (PLUGIN_DIRS_ENV, None),
            (FACTORY_MCP_FILES_ENV, None),
            (PLUGIN_DATA_ROOT_ENV, Some("relative/data")),
        ],
        || FactoryPluginRuntime::from_env(Path::new("/work")),
    );
    assert_eq!(runtime.plugin_data_root, None);
    assert!(!runtime.allows_stdio_plugin_servers());
}

/// The rule the server cannot enforce: with no durable data root, a plugin's stdio servers must
/// not start. Nothing is spawned, and the refusal says why.
#[test]
fn without_a_data_root_no_stdio_server_may_start() {
    let runtime = FactoryPluginRuntime {
        plugin_collection_dirs: vec![PathBuf::from("/repo/plugins")],
        factory_mcp_files: Vec::new(),
        plugin_data_root: None,
        factory_uid: None,
    };
    let servers = vec![stdio_server("validator")];

    let (startable, refused) = runtime.partition_startable("acme-tools", &servers);

    assert!(
        startable.is_empty(),
        "a stdio server must not start without a durable plugin data root"
    );
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].plugin.as_deref(), Some("acme-tools"));
    assert_eq!(refused[0].component.as_deref(), Some("validator"));
    assert!(refused[0].reason.contains(PLUGIN_DATA_ROOT_ENV));
}

/// A remote-transport server needs no plugin data, so it is unaffected by the same absence.
#[test]
fn without_a_data_root_remote_servers_still_start() {
    let runtime = FactoryPluginRuntime::default();
    let servers = vec![stdio_server("validator"), http_server("issues")];

    let (startable, refused) = runtime.partition_startable("acme-tools", &servers);

    assert_eq!(
        startable
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        vec!["issues"]
    );
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].component.as_deref(), Some("validator"));
}

#[test]
fn with_a_data_root_every_transport_is_startable() {
    let runtime = FactoryPluginRuntime {
        plugin_collection_dirs: Vec::new(),
        factory_mcp_files: Vec::new(),
        plugin_data_root: Some(PathBuf::from("/durable")),
        factory_uid: None,
    };
    let servers = vec![stdio_server("validator"), http_server("issues")];

    let (startable, refused) = runtime.partition_startable("acme-tools", &servers);

    assert_eq!(startable.len(), 2);
    assert!(refused.is_empty());
}
