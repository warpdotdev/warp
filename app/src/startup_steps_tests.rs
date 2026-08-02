use std::cell::RefCell;

use anyhow::{Result, anyhow};
use warp_cli::CliCommand;

use super::*;
use crate::{LaunchMode, TuiEntryPoint};

/// Runs every guarded startup step through the same entry points startup uses,
/// recording which effects actually executed. This asserts observable startup
/// behavior rather than the value of a predicate: if a guard is removed,
/// inverted, or stops running its effect, the recorded list changes.
fn effects_that_run(launch_mode: &LaunchMode, autoupdate_enabled: bool) -> Vec<StartupEffect> {
    let recorded = RefCell::new(Vec::new());
    let record = |effect: StartupEffect| -> Result<()> {
        recorded.borrow_mut().push(effect);
        Ok(())
    };

    with_background_process_setup(launch_mode, || {
        record(StartupEffect::MarkProcessBackgroundOnly)
    })
    .expect("recording effect never fails");
    with_dock_and_menu_setup(launch_mode, || record(StartupEffect::ConfigureDockAndMenus))
        .expect("recording effect never fails");
    with_old_executable_cleanup(launch_mode, autoupdate_enabled, || {
        record(StartupEffect::RemoveOldExecutable)
    })
    .expect("recording effect never fails");

    recorded.into_inner()
}

/// A `LaunchMode::CommandLine` equivalent to running the bundled
/// `oz` / `oz-<channel>` wrapper for a headless command.
fn cli_launch_mode() -> LaunchMode {
    LaunchMode::CommandLine {
        command: CliCommand::Whoami,
        global_options: Default::default(),
        debug: false,
        is_sandboxed: false,
        computer_use_override: None,
    }
}

fn tui_launch_mode() -> LaunchMode {
    LaunchMode::Tui {
        entrypoint: TuiEntryPoint::Interactive {
            mount: Box::new(|_| {}),
            api_key: None,
        },
    }
}

fn gui_launch_mode() -> LaunchMode {
    LaunchMode::App {
        args: Default::default(),
        api_key: None,
    }
}

fn headless_launch_modes() -> Vec<LaunchMode> {
    vec![
        cli_launch_mode(),
        tui_launch_mode(),
        LaunchMode::RemoteServerProxy,
        LaunchMode::RemoteServerDaemon {
            identity_key: "test".to_owned(),
        },
    ]
}

/// The bundled CLI wrapper `exec`s the GUI executable from inside `Warp.app`,
/// so a headless launch must claim a background-only process type and must not
/// perform Dock-visible setup or touch the app bundle. Otherwise macOS shows a
/// Warp Dock tile that bounces for the whole command (APP-2946).
#[test]
fn headless_startup_only_claims_a_background_process_type() {
    for launch_mode in headless_launch_modes() {
        assert_eq!(
            effects_that_run(&launch_mode, /* autoupdate_enabled */ true),
            vec![StartupEffect::MarkProcessBackgroundOnly],
            "unexpected startup effects for {}",
            launch_mode.as_str_for_tracing()
        );
    }
}

/// The GUI app keeps its Dock presence and its own post-autoupdate cleanup, and
/// must not be demoted to a background process.
#[test]
fn gui_startup_configures_the_dock_and_cleans_up_its_bundle() {
    assert_eq!(
        effects_that_run(&gui_launch_mode(), /* autoupdate_enabled */ true),
        vec![
            StartupEffect::ConfigureDockAndMenus,
            StartupEffect::RemoveOldExecutable,
        ]
    );
}

/// The old-executable cleanup stays behind the autoupdate feature flag for the
/// GUI app too — the launch-mode guard is additional to it, not a replacement.
#[test]
fn gui_startup_skips_bundle_cleanup_when_autoupdate_is_disabled() {
    assert_eq!(
        effects_that_run(&gui_launch_mode(), /* autoupdate_enabled */ false),
        vec![StartupEffect::ConfigureDockAndMenus]
    );
}

/// A headless launch stays out of the app bundle regardless of the autoupdate
/// feature flag.
#[test]
fn headless_startup_never_touches_the_app_bundle() {
    for launch_mode in headless_launch_modes() {
        for autoupdate_enabled in [false, true] {
            assert!(
                !effects_that_run(&launch_mode, autoupdate_enabled)
                    .contains(&StartupEffect::RemoveOldExecutable),
                "{} must not remove the old executable from the app bundle (autoupdate_enabled={autoupdate_enabled})",
                launch_mode.as_str_for_tracing()
            );
        }
    }
}

/// Each step reports whether it ran and propagates a failure from the effect it
/// guards, so a caller cannot mistake "skipped" for "succeeded".
#[test]
fn each_step_reports_whether_it_ran_and_propagates_effect_failures() {
    let cli = cli_launch_mode();
    let gui = gui_launch_mode();

    assert!(
        with_background_process_setup(&cli, || Ok(())).unwrap(),
        "the CLI must run background-process setup"
    );
    assert!(
        !with_background_process_setup(&gui, || Ok(())).unwrap(),
        "the GUI must not run background-process setup"
    );
    assert!(
        with_background_process_setup(&cli, || Err(anyhow!("transform failed"))).is_err(),
        "a failing background-process setup must surface to the caller"
    );

    assert!(!with_dock_and_menu_setup(&cli, || Ok(())).unwrap());
    assert!(with_dock_and_menu_setup(&gui, || Ok(())).unwrap());
    assert!(with_dock_and_menu_setup(&gui, || Err(anyhow!("dock setup failed"))).is_err());

    assert!(!with_old_executable_cleanup(&cli, true, || Ok(())).unwrap());
    assert!(with_old_executable_cleanup(&gui, true, || Ok(())).unwrap());
    assert!(with_old_executable_cleanup(&gui, true, || Err(anyhow!("cleanup failed"))).is_err());
}
