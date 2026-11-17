use std::ffi::OsString;

use warpui::{Entity, SingletonEntity};

/// The process environment at the start of application initialization.
pub struct InitialProcessEnvironment {
    variables: Vec<(OsString, OsString)>,
}

impl InitialProcessEnvironment {
    pub fn capture() -> Self {
        Self {
            variables: std::env::vars_os().collect(),
        }
    }

    pub fn variables(&self) -> &[(OsString, OsString)] {
        &self.variables
    }
}

impl Entity for InitialProcessEnvironment {
    type Event = ();
}

impl SingletonEntity for InitialProcessEnvironment {}
