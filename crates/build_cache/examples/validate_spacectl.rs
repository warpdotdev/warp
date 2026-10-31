use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::{env, fs};

use build_cache::{
    CacheScope, RepoIdentity, RepositoryCacheSource, default_run_command, global_cache_modes,
    setup_cache,
};
use command::r#async::Command;
use serde_json::Value;

struct Fixture {
    name: &'static str,
    files: &'static [(&'static str, &'static str)],
    /// Modes of the repository's root configuration, which is kept only when it mounts paths
    /// inside the repository.
    repository_modes: &'static [&'static str],
    /// Modes the global configuration must include for this repository.
    global_modes: &'static [&'static str],
}
const CARGO_TOML: &str =
    "[package]\nname = \"cache-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
const GO_MOD: &str = "module example.com/cache-fixture\n\ngo 1.22\n";
const PACKAGE_JSON: &str =
    "{\"name\":\"cache-fixture\",\"version\":\"1.0.0\",\"lockfileVersion\":3}\n";
const PACKAGE_LOCK: &str =
    "{\"name\":\"cache-fixture\",\"version\":\"1.0.0\",\"lockfileVersion\":3,\"packages\":{}}\n";
const WORKSPACE_CARGO_TOML: &str =
    "[workspace]\nmembers = [\"crates/a\", \"crates/b\"]\nresolver = \"2\"\n";
const MEMBER_A_CARGO_TOML: &str =
    "[package]\nname = \"member-a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
const MEMBER_B_CARGO_TOML: &str =
    "[package]\nname = \"member-b\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";

const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "rust",
        files: &[("Cargo.toml", CARGO_TOML)],
        repository_modes: &["rust"],
        global_modes: &["rust"],
    },
    Fixture {
        name: "rust-secondary",
        files: &[("Cargo.toml", CARGO_TOML)],
        repository_modes: &["rust"],
        global_modes: &["rust"],
    },
    Fixture {
        name: "go",
        files: &[("go.mod", GO_MOD)],
        repository_modes: &[],
        global_modes: &["go"],
    },
    Fixture {
        name: "go-secondary",
        files: &[("go.mod", GO_MOD)],
        repository_modes: &[],
        global_modes: &["go"],
    },
    Fixture {
        name: "rust-go",
        files: &[("Cargo.toml", CARGO_TOML), ("go.mod", GO_MOD)],
        repository_modes: &["go", "rust"],
        global_modes: &["go", "rust"],
    },
    Fixture {
        name: "nested",
        files: &[
            ("Cargo.toml", CARGO_TOML),
            ("frontend/package.json", PACKAGE_JSON),
            ("frontend/package-lock.json", PACKAGE_LOCK),
            ("backend/go.mod", GO_MOD),
        ],
        repository_modes: &["rust"],
        global_modes: &["go", "npm", "rust"],
    },
    Fixture {
        name: "workspace",
        files: &[
            ("Cargo.toml", WORKSPACE_CARGO_TOML),
            ("crates/a/Cargo.toml", MEMBER_A_CARGO_TOML),
            ("crates/a/src/lib.rs", ""),
            ("crates/b/Cargo.toml", MEMBER_B_CARGO_TOML),
            ("crates/b/src/lib.rs", ""),
        ],
        repository_modes: &["rust"],
        global_modes: &["rust"],
    },
    Fixture {
        name: "node",
        files: &[(
            "package.json",
            "{\"name\":\"cache-fixture\",\"version\":\"1.0.0\"}\n",
        )],
        repository_modes: &[],
        global_modes: &[],
    },
    Fixture {
        name: "empty",
        files: &[],
        repository_modes: &[],
        global_modes: &[],
    },
];

#[cfg_attr(not(unix), allow(dead_code))]
struct CapturedResponse {
    cwd: PathBuf,
    dry_run: bool,
    value: Value,
}
#[cfg(unix)]
struct ExpectedMount {
    cache_path: PathBuf,
    mode: String,
}
struct Options {
    reset: bool,
    fixture_names: Vec<String>,
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("validation setup failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool, String> {
    let options = parse_options()?;
    let fixtures = selected_fixtures(&options.fixture_names)?;
    let validation_root = validation_root(options.reset)?;
    let repositories_root = validation_root.join("repositories");
    let cache_root = validation_root.join("cache");
    let isolated_home = validation_root.join("home");
    let shim_directory = create_sudo_shim(&validation_root)?;
    let command_path = command_path(&shim_directory)?;
    let rustup_home = rustup_home();

    reset_directory(&repositories_root)?;
    fs::create_dir_all(&cache_root).map_err(|error| error.to_string())?;
    reset_directory(&isolated_home)?;

    let repositories = create_repositories(&repositories_root, &fixtures)?;
    let additional_global_modes = global_cache_modes();

    println!("validation root: {}", validation_root.display());
    println!("cache root: {}", cache_root.display());
    println!("isolated HOME: {}", isolated_home.display());
    println!("command shim directory: {}", shim_directory.display());
    println!(
        "selected fixtures: {}",
        fixtures
            .iter()
            .map(|fixture| fixture.name)
            .collect::<Vec<_>>()
            .join(",")
    );
    println!(
        "additional global modes: {}",
        display_modes(&additional_global_modes)
    );
    println!("generated repositories:");
    for repository in &repositories {
        println!("  {}: {}", repository.name, repository.cwd.display());
    }

    let responses = Arc::new(Mutex::new(Vec::new()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| error.to_string())?;
    let report = runtime.block_on(setup_cache(
        cache_root.clone(),
        repositories,
        additional_global_modes,
        {
            let responses = Arc::clone(&responses);
            move |mut command| {
                let cwd = command
                    .get_current_dir()
                    .map(ToOwned::to_owned)
                    .unwrap_or_default();
                let dry_run = command
                    .get_args()
                    .any(|argument| argument == OsStr::new("--dry_run=true"));
                configure_isolated_environment(
                    &mut command,
                    &isolated_home,
                    rustup_home.as_deref(),
                    &command_path,
                );
                let responses = Arc::clone(&responses);
                async move {
                    let bytes = default_run_command(command).await?;
                    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                    responses.lock().unwrap().push(CapturedResponse {
                        cwd,
                        dry_run,
                        value,
                    });
                    Ok(bytes)
                }
            }
        },
    ));

    print_plan(&report);
    print_invocations(&report);
    print_environment(&report);

    let expected_mode_failures = validate_fixture_modes(&report, &fixtures);
    let global_mode_failures = validate_global_modes(&report, &fixtures);
    let nested_fixture_failures = validate_nested_fixture(&report, &fixtures);
    let workspace_fixture_failures = validate_workspace_fixture(
        &report,
        &fixtures,
        &responses.lock().unwrap(),
        &cache_root,
        &repositories_root.join("workspace"),
    );
    let repository_cache_root_failures = validate_repository_cache_roots(&report);
    let mount_failures = validate_mounts(&responses.lock().unwrap());
    let degradation_count = report.degradations().count();
    let missing_plan = usize::from(report.plan.is_none());
    let failure_count = expected_mode_failures
        + global_mode_failures
        + nested_fixture_failures
        + workspace_fixture_failures
        + repository_cache_root_failures
        + mount_failures
        + degradation_count
        + missing_plan;
    println!();
    if failure_count == 0 {
        println!("validation passed");
        Ok(true)
    } else {
        println!(
            "validation failed: {degradation_count} degraded invocation(s), \
             {expected_mode_failures} mode mismatch(es), \
             {global_mode_failures} global mode mismatch(es), \
             {nested_fixture_failures} nested fixture mismatch(es), \
             {workspace_fixture_failures} workspace fixture mismatch(es), \
             {repository_cache_root_failures} duplicate repository cache root(s), \
             {mount_failures} mount mismatch(es), \
             {missing_plan} missing plan(s)"
        );
        Ok(false)
    }
}

fn parse_options() -> Result<Options, String> {
    let mut reset = false;
    let mut fixture_names = Vec::new();
    for argument in env::args().skip(1) {
        if argument == "--reset" {
            reset = true;
        } else if argument.starts_with('-') {
            return Err(format!("unknown option {argument}"));
        } else if fixture_names.contains(&argument) {
            return Err(format!("fixture {argument} was selected more than once"));
        } else {
            fixture_names.push(argument);
        }
    }
    Ok(Options {
        reset,
        fixture_names,
    })
}

fn selected_fixtures(names: &[String]) -> Result<Vec<&'static Fixture>, String> {
    if names.is_empty() {
        return Ok(FIXTURES.iter().collect());
    }
    names
        .iter()
        .map(|name| {
            FIXTURES
                .iter()
                .find(|fixture| fixture.name == name)
                .ok_or_else(|| {
                    format!(
                        "unknown fixture {name}; available fixtures: {}",
                        FIXTURES
                            .iter()
                            .map(|fixture| fixture.name)
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                })
        })
        .collect()
}

fn validation_root(reset: bool) -> Result<PathBuf, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/build-cache-validation");
    if reset && root.exists() {
        fs::remove_dir_all(&root).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    root.canonicalize().map_err(|error| error.to_string())
}

fn reset_directory(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(path).map_err(|error| error.to_string())
}

fn create_repositories(
    root: &Path,
    fixtures: &[&Fixture],
) -> Result<Vec<RepositoryCacheSource>, String> {
    fixtures
        .iter()
        .copied()
        .map(|fixture| {
            let cwd = root.join(fixture.name);
            fs::create_dir_all(&cwd).map_err(|error| error.to_string())?;
            for (relative_path, contents) in fixture.files {
                let path = cwd.join(relative_path);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                fs::write(path, contents).map_err(|error| error.to_string())?;
            }
            Ok(RepositoryCacheSource {
                name: fixture.name.to_owned(),
                identity: RepoIdentity::new(
                    "local.invalid",
                    "build-cache-validation",
                    fixture.name,
                ),
                cwd,
            })
        })
        .collect()
}

/// Create a no-op `sudo` shim for use on macOS, where cache setup doesn't actually require elevated privileges.
#[cfg(unix)]
fn create_sudo_shim(root: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = root.join("bin");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let shim = directory.join("sudo");
    fs::write(&shim, "#!/bin/sh\nexec \"$@\"\n").map_err(|error| error.to_string())?;
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755))
        .map_err(|error| error.to_string())?;
    Ok(directory)
}

#[cfg(not(unix))]
fn create_sudo_shim(root: &Path) -> Result<PathBuf, String> {
    Ok(root.join("bin"))
}

fn command_path(shim_directory: &Path) -> Result<std::ffi::OsString, String> {
    let mut paths = vec![shim_directory.to_owned()];
    if let Some(path) = env::var_os("PATH") {
        paths.extend(env::split_paths(&path));
    }
    env::join_paths(paths).map_err(|error| error.to_string())
}

/// The caller's rustup home. Toolchains are shared with the isolated environment so that
/// `cargo metadata` resolves a toolchain instead of failing and falling back to `./target`.
fn rustup_home() -> Option<PathBuf> {
    env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".rustup")))
}

fn configure_isolated_environment(
    command: &mut Command,
    home: &Path,
    rustup_home: Option<&Path>,
    path: &OsStr,
) {
    if let Some(rustup_home) = rustup_home {
        command.env("RUSTUP_HOME", rustup_home);
    }
    command.env_remove("CARGO_TARGET_DIR");
    command.envs([
        ("HOME", home.to_path_buf()),
        ("CARGO_HOME", home.join(".cargo")),
        ("GOPATH", home.join("go")),
        ("GOCACHE", home.join("Library/Caches/go-build")),
        ("GOMODCACHE", home.join("go/pkg/mod")),
        ("HOMEBREW_CACHE", home.join("Library/Caches/Homebrew")),
        ("XDG_CACHE_HOME", home.join(".cache")),
    ]);
    command.env("PATH", path);
}

fn print_plan(report: &build_cache::CacheSetupReport) {
    println!();
    println!("plan:");
    let Some(plan) = &report.plan else {
        println!("  none");
        return;
    };
    for configuration in &plan.configurations {
        println!(
            "  {} cwd={} cache={} modes={}",
            scope_name(&configuration.scope),
            configuration.cwd.display(),
            configuration.relative_cache_dir.display(),
            display_modes(&configuration.modes),
        );
    }
}

fn print_invocations(report: &build_cache::CacheSetupReport) {
    println!();
    println!("invocations:");
    for invocation in &report.invocations {
        let outcome = match &invocation.error {
            Some(error) => format!("error={} ({})", error.kind(), error),
            None => "ok".to_owned(),
        };
        println!(
            "  {} cache={} modes={} {outcome}",
            scope_name(&invocation.scope),
            invocation.relative_cache_dir.display(),
            display_modes(&invocation.modes),
        );
        for (mode, stats) in &invocation.mode_stats {
            println!(
                "    {mode}: hits={} misses={}",
                stats.cache_hits, stats.cache_misses
            );
        }
    }
}

fn print_environment(report: &build_cache::CacheSetupReport) {
    println!();
    println!("environment:");
    if report.add_envs.is_empty() {
        println!("  none");
    } else {
        for (name, value) in &report.add_envs {
            println!("  {name}={value}");
        }
    }
}

fn validate_fixture_modes(report: &build_cache::CacheSetupReport, fixtures: &[&Fixture]) -> usize {
    println!();
    println!("fixture mode checks:");
    let actual = report
        .plan
        .iter()
        .flat_map(|plan| &plan.configurations)
        .filter_map(|configuration| match &configuration.scope {
            CacheScope::Repository { name, .. } => {
                Some((name.as_str(), configuration.modes.as_slice()))
            }
            CacheScope::Global => None,
        })
        .collect::<BTreeMap<_, _>>();

    let mut failures = 0;
    for fixture in fixtures {
        let actual_modes = actual.get(fixture.name).copied().unwrap_or_default();
        let expected_modes = fixture.repository_modes;
        if actual_modes == expected_modes {
            println!("  ok {}: {}", fixture.name, display_modes(actual_modes));
        } else {
            println!(
                "  mismatch {}: expected {}, got {}",
                fixture.name,
                display_modes(expected_modes),
                display_modes(actual_modes)
            );
            failures += 1;
        }
    }
    failures
}

fn validate_global_modes(report: &build_cache::CacheSetupReport, fixtures: &[&Fixture]) -> usize {
    println!();
    println!("global mode checks:");
    let global_modes = report
        .plan
        .iter()
        .flat_map(|plan| &plan.configurations)
        .filter(|configuration| matches!(configuration.scope, CacheScope::Global))
        .flat_map(|configuration| &configuration.modes)
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut failures = 0;
    for fixture in fixtures {
        let missing = fixture
            .global_modes
            .iter()
            .filter(|mode| !global_modes.contains(*mode))
            .collect::<Vec<_>>();
        if missing.is_empty() {
            println!(
                "  ok {}: {}",
                fixture.name,
                display_modes(fixture.global_modes)
            );
        } else {
            println!(
                "  mismatch {}: global scope is missing {}",
                fixture.name,
                display_modes(&missing)
            );
            failures += 1;
        }
    }
    failures
}

fn repository_configurations<'a>(
    report: &'a build_cache::CacheSetupReport,
    repository: &str,
) -> Vec<&'a build_cache::CacheConfiguration> {
    report
        .plan
        .iter()
        .flat_map(|plan| &plan.configurations)
        .filter(|configuration| {
            matches!(
                &configuration.scope,
                CacheScope::Repository { name, .. } if name == repository
            )
        })
        .collect()
}

fn validate_nested_fixture(report: &build_cache::CacheSetupReport, fixtures: &[&Fixture]) -> usize {
    if !fixtures.iter().any(|fixture| fixture.name == "nested") {
        return 0;
    }

    println!();
    println!("nested fixture checks:");
    if report.plan.is_none() {
        println!("  mismatch: no cache plan");
        return 1;
    }
    let configurations = repository_configurations(report, "nested");
    let mut failures = 0;
    let root_kept = configurations.iter().any(|configuration| {
        configuration.cwd.ends_with("nested")
            && configuration.modes.iter().any(|mode| mode == "rust")
    });
    if root_kept {
        println!("  ok nested: root kept for its rust target");
    } else {
        println!("  mismatch nested: expected the root configuration with rust");
        failures += 1;
    }
    // Go and npm only mount global paths, which the global configuration already covers.
    for suffix in ["nested/backend", "nested/frontend"] {
        if configurations
            .iter()
            .any(|configuration| configuration.cwd.ends_with(suffix))
        {
            println!("  mismatch {suffix}: expected no repository configuration");
            failures += 1;
        } else {
            println!("  ok {suffix}: no repository configuration");
        }
    }
    failures
}

/// Checks that the workspace's shared `target` directory is mounted once, from the root
/// configuration's cache directory, rather than once per member crate.
fn validate_workspace_fixture(
    report: &build_cache::CacheSetupReport,
    fixtures: &[&Fixture],
    responses: &[CapturedResponse],
    cache_root: &Path,
    workspace_root: &Path,
) -> usize {
    if !fixtures.iter().any(|fixture| fixture.name == "workspace") {
        return 0;
    }

    println!();
    println!("workspace fixture checks:");
    let configurations = repository_configurations(report, "workspace");
    let mut failures = 0;
    let root_configuration = match configurations.as_slice() {
        [configuration] if configuration.cwd == workspace_root => {
            println!("  ok: only the workspace root has a repository configuration");
            Some(*configuration)
        }
        _ => {
            println!(
                "  mismatch: expected only the workspace root, got {}",
                configurations
                    .iter()
                    .map(|configuration| configuration.cwd.display().to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            failures += 1;
            configurations
                .iter()
                .find(|configuration| configuration.cwd == workspace_root)
                .copied()
        }
    };

    let target = workspace_root.join("target");
    let target_mounts = responses
        .iter()
        .filter(|response| !response.dry_run)
        .flat_map(|response| {
            response
                .value
                .get("output")
                .and_then(|output| output.get("mounts"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|mount| {
                    let mount_path = mount.get("mount_path").and_then(Value::as_str)?;
                    let cache_path = mount.get("cache_path").and_then(Value::as_str)?;
                    (lexical_absolute(&response.cwd, Path::new(mount_path)) == target)
                        .then(|| lexical_absolute(&response.cwd, Path::new(cache_path)))
                })
        })
        .collect::<Vec<_>>();
    let expected_cache =
        root_configuration.map(|configuration| cache_root.join(&configuration.relative_cache_dir));
    match (target_mounts.as_slice(), expected_cache) {
        ([cache_path], Some(expected)) if cache_path.starts_with(&expected) => {
            println!("  ok: target mounted once, from {}", cache_path.display());
        }
        (mounts, _) => {
            println!(
                "  mismatch: expected one target mount from the root cache, got {} ({})",
                mounts.len(),
                mounts
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            failures += 1;
        }
    }
    failures
}

/// Resolves `path` against `cwd` and removes `.` and `..` components without touching the
/// filesystem.
fn lexical_absolute(cwd: &Path, path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in cwd.join(path).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other),
        }
    }
    resolved
}
fn validate_repository_cache_roots(report: &build_cache::CacheSetupReport) -> usize {
    println!();
    println!("repository cache root checks:");
    let mut owners = BTreeMap::<&Path, Vec<&str>>::new();
    if let Some(plan) = &report.plan {
        for configuration in &plan.configurations {
            if let CacheScope::Repository { name, .. } = &configuration.scope {
                owners
                    .entry(&configuration.relative_cache_dir)
                    .or_default()
                    .push(name);
            }
        }
    }

    let mut failures = 0;
    for (cache_root, names) in owners {
        if names.len() == 1 {
            println!("  ok {}: {}", names[0], cache_root.display());
        } else {
            println!("  duplicate {}: {}", names.join(","), cache_root.display());
            failures += 1;
        }
    }
    failures
}

#[cfg(unix)]
fn validate_mounts(responses: &[CapturedResponse]) -> usize {
    let mut expected_mounts = BTreeMap::new();
    for response in responses.iter().filter(|response| !response.dry_run) {
        let Some(mounts) = response
            .value
            .get("output")
            .and_then(|output| output.get("mounts"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for mount in mounts {
            let Some(cache_path) = mount.get("cache_path").and_then(Value::as_str) else {
                continue;
            };
            let Some(mount_path) = mount.get("mount_path").and_then(Value::as_str) else {
                continue;
            };
            let mode = mount
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned();
            expected_mounts.insert(
                absolute_from(&response.cwd, Path::new(mount_path)),
                ExpectedMount {
                    cache_path: absolute_from(&response.cwd, Path::new(cache_path)),
                    mode,
                },
            );
        }
    }

    println!();
    println!("final symlink checks:");
    let mut failures = 0;
    for (mount_path, expected) in expected_mounts {
        let result = fs::symlink_metadata(&mount_path)
            .map_err(|error| error.to_string())
            .and_then(|metadata| {
                if !metadata.file_type().is_symlink() {
                    return Err("not a symlink".to_owned());
                }
                let target = fs::read_link(&mount_path).map_err(|error| error.to_string())?;
                let resolved_target = absolute_from(
                    mount_path.parent().unwrap_or_else(|| Path::new("/")),
                    &target,
                );
                let resolved_target = resolved_target
                    .canonicalize()
                    .map_err(|error| error.to_string())?;
                let expected_target = expected
                    .cache_path
                    .canonicalize()
                    .map_err(|error| error.to_string())?;
                if resolved_target == expected_target {
                    Ok(target)
                } else {
                    Err(format!(
                        "points to {}, expected {}",
                        resolved_target.display(),
                        expected_target.display()
                    ))
                }
            });
        match result {
            Ok(target) => println!(
                "  ok [{}] {} -> {}",
                expected.mode,
                mount_path.display(),
                target.display()
            ),
            Err(error) => {
                println!(
                    "  mismatch [{}] {}: {error}",
                    expected.mode,
                    mount_path.display()
                );
                failures += 1;
            }
        }
    }
    failures
}

#[cfg(not(unix))]
fn validate_mounts(_responses: &[CapturedResponse]) -> usize {
    println!();
    println!("final symlink checks: skipped on non-Unix platform");
    0
}

#[cfg(unix)]
fn absolute_from(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        cwd.join(path)
    }
}

fn scope_name(scope: &CacheScope) -> &str {
    match scope {
        CacheScope::Repository { name, .. } => name,
        CacheScope::Global => "global",
    }
}

fn display_modes<T: AsRef<str>>(modes: &[T]) -> String {
    if modes.is_empty() {
        "none".to_owned()
    } else {
        modes
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>()
            .join(",")
    }
}
