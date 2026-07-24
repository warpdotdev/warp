use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::{env, fs};

use build_cache::{
    CacheScope, RepoIdentity, RepositoryCacheSource, default_run_command, global_cache_modes,
    setup_cache,
};
use command::r#async::Command;
use futures_lite::future;
use serde_json::Value;

struct Fixture {
    name: &'static str,
    files: &'static [(&'static str, &'static str)],
    expected_modes: &'static [&'static str],
}
const CARGO_TOML: &str =
    "[package]\nname = \"cache-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
const GO_MOD: &str = "module example.com/cache-fixture\n\ngo 1.22\n";

const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "rust",
        files: &[("Cargo.toml", CARGO_TOML)],
        expected_modes: &["rust"],
    },
    Fixture {
        name: "rust-secondary",
        files: &[("Cargo.toml", CARGO_TOML)],
        expected_modes: &["rust"],
    },
    Fixture {
        name: "go",
        files: &[("go.mod", GO_MOD)],
        expected_modes: &["go"],
    },
    Fixture {
        name: "go-secondary",
        files: &[("go.mod", GO_MOD)],
        expected_modes: &["go"],
    },
    Fixture {
        name: "rust-go",
        files: &[("Cargo.toml", CARGO_TOML), ("go.mod", GO_MOD)],
        expected_modes: &["go", "rust"],
    },
    Fixture {
        name: "node",
        files: &[(
            "package.json",
            "{\"name\":\"cache-fixture\",\"version\":\"1.0.0\"}\n",
        )],
        expected_modes: &[],
    },
    Fixture {
        name: "empty",
        files: &[],
        expected_modes: &[],
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

    let responses = Rc::new(RefCell::new(Vec::new()));
    let report = future::block_on(setup_cache(
        cache_root,
        repositories,
        additional_global_modes,
        {
            let responses = Rc::clone(&responses);
            move |mut command| {
                let cwd = command
                    .get_current_dir()
                    .map(ToOwned::to_owned)
                    .unwrap_or_default();
                let dry_run = command
                    .get_args()
                    .any(|argument| argument == OsStr::new("--dry_run=true"));
                configure_isolated_environment(&mut command, &isolated_home, &command_path);
                let responses = Rc::clone(&responses);
                async move {
                    let bytes = default_run_command(command).await?;
                    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                    responses.borrow_mut().push(CapturedResponse {
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
    let repository_cache_root_failures = validate_repository_cache_roots(&report);
    let mount_failures = validate_mounts(&responses.borrow());
    let degradation_count = report.degradations().count();
    let missing_plan = usize::from(report.plan.is_none());
    let failure_count = expected_mode_failures
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

fn configure_isolated_environment(command: &mut Command, home: &Path, path: &OsStr) {
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
        let expected_modes = fixture.expected_modes;
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
