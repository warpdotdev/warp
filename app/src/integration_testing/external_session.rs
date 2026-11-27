use std::ffi::OsString;
use std::path::PathBuf;

use warp_cli::ExternalCommandSpec;
use warpui::App;

use crate::external_session::{ExternalSessionLaunchRequest, open_external_session};

#[derive(Debug, Default, Clone)]
pub struct TestExternalSessionOptions {
    pub working_directory: Option<PathBuf>,
    pub command: Option<ExternalCommandSpec>,
    pub env_vars: Vec<(OsString, OsString)>,
    pub title: Option<String>,
    pub new_window: bool,
    pub wait_after_command: Option<bool>,
}

pub fn open_external_session_for_test(options: TestExternalSessionOptions, app: &mut App) {
    let request = ExternalSessionLaunchRequest {
        working_directory: options.working_directory,
        command: options.command,
        env_vars: options.env_vars,
        title: options.title,
        new_window: options.new_window,
        wait_after_command: options.wait_after_command,
    };
    app.update(|ctx| open_external_session(request, ctx));
}
