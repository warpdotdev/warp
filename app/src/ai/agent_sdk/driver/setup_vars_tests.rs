use super::*;

#[test]
fn vars_file_uses_workspace_root_instead_of_a_repository_directory() {
    let root = tempfile::tempdir().unwrap();
    let repository = root.path().join("repo");

    assert_eq!(
        file_path(
            &repository,
            &HashMap::from([(
                OsString::from("OZ_WORKSPACE_ROOT"),
                root.path().as_os_str().to_owned(),
            )]),
        ),
        root.path().join(".warp.vars")
    );
    assert_eq!(
        file_path(
            &repository,
            &HashMap::from([(OsString::from("OZ_WORKSPACE_ROOT"), OsString::new())]),
        ),
        repository.join(".warp.vars")
    );
}

#[test]
#[cfg(windows)]
fn windows_keys_use_last_write_and_paths_deduplicate_without_case() {
    let values = parse(
        "Value=old\nVALUE=last\nPath=C:\\Tools;C:\\WINDOWS\nPATH=C:\\Tools;C:\\Windows\n",
        &HashMap::new(),
    );
    let mut environment = HashMap::from([(
        OsString::from("Path"),
        OsString::from("C:\\windows;C:\\System"),
    )]);

    merge(&values, &HashMap::new(), &mut environment);

    assert_eq!(environment[OsStr::new("VALUE")], "last");
    assert_eq!(
        environment[OsStr::new("Path")],
        "C:\\Tools;C:\\Windows;C:\\System"
    );
}

#[test]
#[cfg(unix)]
fn cold_setup_metadata_precedes_setup_and_file_values_reach_mcp_processes() {
    use warp_core::execution_mode::{AppExecutionMode, ExecutionMode};
    use warpui::{App, SingletonEntity as _};

    use super::super::AgentDriverOptions;
    use super::super::terminal::TerminalDriver;
    use crate::ai::mcp::{
        FileBasedMCPManager, FileMCPWatcher, ParsedTemplatableMCPServerResult,
        TemplatableMCPServerManager,
    };
    use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let sdk_mode = app.add_model(|ctx| AppExecutionMode::new(ExecutionMode::Sdk, true, ctx));
        AppExecutionMode::handle(&app).update(&mut app, |mode, ctx| {
            *mode = sdk_mode.as_ref(ctx).clone();
        });
        app.add_singleton_model(|_| simple_logger::manager::LogManager::new());
        app.add_singleton_model(|_| FileMCPWatcher::new_inert());
        app.add_singleton_model(FileBasedMCPManager::new);
        let root = tempfile::tempdir().unwrap();
        let (_, config) = super::super::split_run_options(AgentDriverOptions {
            working_dir: root.path().into(),
            environment_variables: HashMap::from([
                ("WARP_METADATA_NAME".into(), "cold-run".into()),
                (
                    "OZ_WORKSPACE_ROOT".into(),
                    root.path().to_string_lossy().into_owned(),
                ),
            ]),
            ..Default::default()
        })
        .unwrap();
        let terminal = add_window_with_terminal(&mut app, None);
        let driver = app.add_model(|ctx| {
            let terminal = TerminalDriver::create_from_existing_view(terminal, ctx);
            let mut driver = AgentDriver::new_for_test(root.path().into(), terminal, ctx);
            driver.apply_run_configuration(config, ctx);
            driver
        });
        let foreground = driver.update(&mut app, |_, ctx| ctx.spawner());

        AgentDriver::initialize_setup_vars(&foreground, true)
            .await
            .unwrap();
        let path = driver.read(&app, |driver, _| {
            PathBuf::from(&driver.resolved_env_vars[OsStr::new(VARS_FILE_ENV)])
        });
        assert_eq!(path, root.path().join(".warp.vars"));
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("\ncold-run\n")
        );
        writeln!(
            open_vars_file(&path).unwrap(),
            "CUSTOM_VALUE=from-cold-setup"
        )
        .unwrap();
        AgentDriver::read_setup_vars(&foreground, false, true)
            .await
            .unwrap();

        let output_path = root.path().join("mcp-environment");
        let json = serde_json::json!({
            "environment-probe": {
                "command": "sh",
                "args": [
                    "-c",
                    "printf '%s\\n%s' \"$CUSTOM_VALUE\" \"$WARP_VARS_FILE\" > \"$1\"",
                    "sh",
                    output_path,
                ]
            }
        });
        let installation = ParsedTemplatableMCPServerResult::from_user_json(&json.to_string())
            .unwrap()
            .pop()
            .unwrap()
            .templatable_mcp_server_installation
            .unwrap();
        let uuid = installation.uuid();
        TemplatableMCPServerManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.spawn_cli_ephemeral_server(installation, ctx);
        });
        let deadline = instant::Instant::now() + Duration::from_secs(5);
        let expected = format!("from-cold-setup\n{}", path.display());
        loop {
            if std::fs::read_to_string(&output_path).ok().as_ref() == Some(&expected) {
                break;
            }
            assert!(
                instant::Instant::now() < deadline,
                "MCP must inherit setup values: {:?}",
                TemplatableMCPServerManager::handle(&app).read(&app, |manager, _| {
                    manager.get_server_error_message(uuid).map(str::to_owned)
                })
            );
            warpui::r#async::Timer::after(Duration::from_millis(10)).await;
        }
        TemplatableMCPServerManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.shutdown_server(uuid, ctx);
        });
    });
}

#[test]
fn parses_literal_assignments_with_bom_and_last_write_wins() {
    let values = parse(
        "\u{feff}# ignored\r\n\r\nFOO=old\r\nFOO=  \"$(cmd)\" `cmd`; '$HOME'=x  \r\n_EMPTY=\r\n",
        &HashMap::new(),
    );

    assert_eq!(
        values,
        HashMap::from([
            ("FOO".into(), "  \"$(cmd)\" `cmd`; '$HOME'=x  ".into()),
            ("_EMPTY".into(), String::new()),
        ])
    );
}

#[test]
#[cfg(windows)]
fn cold_powershell_assignments_preserve_unicode_and_literal_metacharacters() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(".warp.vars");
    std::fs::write(
        &path,
        "\u{feff}VALUE=’$(exit 41); ‘`exit 42` \"quoted\" = literal\r\n",
    )
    .unwrap();
    let values = parse(&std::fs::read_to_string(path).unwrap(), &HashMap::new())
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect();
    let (file, command) = private_assignment_script(&values, ShellType::PowerShell).unwrap();
    let output = command::blocking::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!(
            "$env:EXISTING='kept'; {command}; [Console]::OutputEncoding=[System.Text.Encoding]::UTF8; [Console]::Write($env:VALUE + \"`n\" + $env:EXISTING)"
        ))
        .output()
        .unwrap();
    drop(file);

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "’$(exit 41); ‘`exit 42` \"quoted\" = literal\nkept"
    );
}

#[test]
fn heredocs_preserve_blank_lines_and_comment_contents() {
    let values = parse(
        "TEXT<<END\r\nfirst=one\r\n\r\n# literal\r\n'$(cmd)'; `cmd`\r\nEND\r\nAFTER=ok\n",
        &HashMap::new(),
    );

    assert_eq!(
        values,
        HashMap::from([
            (
                "TEXT".into(),
                "first=one\n\n# literal\n'$(cmd)'; `cmd`".into()
            ),
            ("AFTER".into(), "ok".into()),
        ])
    );
}

#[test]
fn rejects_malformed_and_reserved_entries_without_losing_valid_entries() {
    let values = parse(
        "1BAD=value\nBAD-KEY=value\n=empty\nno assignment\nNULL=a\0b\n\
         WARP_API_KEY=blocked\nOZ_RUN_ID=blocked\nWARP_METADATA_NAME=forged\n\
         WARP_OTHER<<END\ninert\nEND\nGOOD=value\nUNFINISHED<<END\nunterminated\n",
        &HashMap::new(),
    );

    assert_eq!(values, HashMap::from([("GOOD".into(), "value".into())]));
}

#[test]
fn merging_preserves_runtime_and_secret_values() {
    let protected = HashMap::from([
        (OsString::from("RUN_VALUE"), OsString::from("runtime")),
        (OsString::from("SECRET_VALUE"), OsString::from("secret")),
    ]);
    let values = HashMap::from([
        ("RUN_VALUE".into(), "file".into()),
        ("SECRET_VALUE".into(), "file".into()),
        ("CUSTOM_VALUE".into(), "file".into()),
    ]);
    let mut environment = protected.clone();

    let applied = merge(&values, &protected, &mut environment);

    assert_eq!(
        applied,
        HashMap::from([(OsString::from("CUSTOM_VALUE"), OsString::from("file"))])
    );
    assert_eq!(environment[OsStr::new("RUN_VALUE")], "runtime");
    assert_eq!(environment[OsStr::new("SECRET_VALUE")], "secret");
}

#[test]
fn path_entries_prepend_and_deduplicate_without_dropping_existing_paths() {
    let file_path = std::env::join_paths(["tools", "shared", "tools"]).unwrap();
    let existing = std::env::join_paths(["system", "shared"]).unwrap();
    let values = HashMap::from([("PATH".into(), file_path.into_string().unwrap())]);
    let mut environment = HashMap::from([(OsString::from("PATH"), existing)]);
    let protected = environment.clone();

    merge(&values, &protected, &mut environment);

    assert_eq!(
        std::env::split_paths(&environment[OsStr::new("PATH")]).collect::<Vec<_>>(),
        [
            PathBuf::from("tools"),
            PathBuf::from("shared"),
            PathBuf::from("system")
        ]
    );
}

#[test]
fn metadata_is_available_before_setup_and_user_entries_remain_untrusted() {
    let root = tempfile::tempdir().unwrap();
    let mut vars = SetupVars::create(
        root.path(),
        HashMap::from([("WARP_METADATA_NAME".into(), "first\nsecond".into())]),
    )
    .unwrap();
    let contents = std::fs::read_to_string(&vars.path).unwrap();
    assert!(contents.contains("WARP_METADATA_NAME<<"));
    assert!(contents.contains("\nfirst\nsecond\n"));
    writeln!(
        open_vars_file(&vars.path).unwrap(),
        "FOO=setup\nWARP_METADATA_NAME=forged"
    )
    .unwrap();

    vars.read();

    assert_eq!(vars.values, HashMap::from([("FOO".into(), "setup".into())]));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&vars.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn standby_values_survive_assignment_and_metadata_is_written_without_rereading() {
    use warpui::App;

    use super::super::AgentDriverOptions;
    use super::super::terminal::TerminalDriver;
    use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let root = tempfile::tempdir().unwrap();
        let mut vars = SetupVars::create(root.path(), HashMap::new()).unwrap();
        writeln!(
            open_vars_file(&vars.path).unwrap(),
            "CUSTOM_VALUE=from-standby\nRUN_VALUE=blocked\nSECRET_VALUE=blocked"
        )
        .unwrap();
        vars.read();
        let runtime = HashMap::from([
            ("RUN_VALUE".into(), "assigned".into()),
            ("WARP_METADATA_NAME".into(), "assigned-metadata".into()),
            (
                "OZ_WORKSPACE_ROOT".into(),
                root.path().to_string_lossy().into_owned(),
            ),
        ]);
        vars.write_assignment_metadata(&runtime).unwrap();
        writeln!(open_vars_file(&vars.path).unwrap(), "CUSTOM_VALUE=too-late").unwrap();
        let terminal = add_window_with_terminal(&mut app, None);
        let driver = app.add_model(|ctx| {
            let terminal = TerminalDriver::create_from_existing_view(terminal, ctx);
            AgentDriver::new_for_test(root.path().to_owned(), terminal, ctx)
        });
        let (_, config) = super::super::split_run_options(AgentDriverOptions {
            working_dir: root.path().to_owned(),
            environment_variables: runtime.clone(),
            secrets: HashMap::from([(
                "SECRET_VALUE".into(),
                warp_managed_secrets::ManagedSecretValue::raw_value("managed"),
            )]),
            ..Default::default()
        })
        .unwrap();
        let mut mcp = runtime;

        driver.update(&mut app, |driver, ctx| {
            driver.setup_vars = Some(vars);
            driver.apply_run_configuration(config, ctx);
            driver.merge_assignment_setup_vars(&mut mcp);

            assert_eq!(
                driver.resolved_env_vars[OsStr::new("CUSTOM_VALUE")],
                "from-standby"
            );
            assert_eq!(
                driver.resolved_env_vars[OsStr::new("RUN_VALUE")],
                "assigned"
            );
            assert_eq!(
                driver.resolved_env_vars[OsStr::new("SECRET_VALUE")],
                "managed"
            );
            assert_eq!(mcp["CUSTOM_VALUE"], "from-standby");
            assert_eq!(mcp["RUN_VALUE"], "assigned");
            assert!(!mcp.contains_key("SECRET_VALUE"));
            assert_eq!(mcp["WARP_METADATA_NAME"], "assigned-metadata");
            assert_eq!(
                PathBuf::from(&mcp[VARS_FILE_ENV]),
                root.path().join(".warp.vars")
            );
        });
        let contents = std::fs::read_to_string(root.path().join(".warp.vars")).unwrap();
        assert!(contents.contains("\nassigned-metadata\n"));
    });
}

#[test]
fn assignment_scripts_quote_metacharacters_for_each_shell() {
    let values = HashMap::from([(
        OsString::from("VALUE"),
        OsString::from("$(cmd) `cmd` \"double\" 'single'; \\\nnext"),
    )]);

    assert_eq!(
        assignment_script(&values, ShellType::Bash),
        "export VALUE='$(cmd) `cmd` \"double\" '\\''single'\\''; \\\nnext'\n"
    );
    assert_eq!(
        assignment_script(&values, ShellType::Zsh),
        "export VALUE='$(cmd) `cmd` \"double\" '\\''single'\\''; \\\nnext'\n"
    );
    assert_eq!(
        assignment_script(&values, ShellType::Fish),
        "set -gx VALUE '$(cmd) `cmd` \"double\" \\'single\\'; \\\\\nnext'\n"
    );
}

#[test]
#[cfg(unix)]
fn cold_assignment_script_preserves_literal_values_in_the_existing_shell() {
    let values = HashMap::from([(
        OsString::from("VALUE"),
        OsString::from("$(exit 41) `exit 42` \"double\" 'single'; \\\nnext"),
    )]);
    let (file, command) = private_assignment_script(&values, ShellType::Bash).unwrap();
    assert!(!command.contains("exit"));
    let output = command::blocking::Command::new("bash")
        .args(["--noprofile", "--norc", "-c"])
        .arg(format!(
            "export EXISTING=kept; {command}; printf '%s\\n%s' \"$VALUE\" \"$EXISTING\""
        ))
        .output()
        .unwrap();
    drop(file);

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "$(exit 41) `exit 42` \"double\" 'single'; \\\nnext\nkept"
    );
}
