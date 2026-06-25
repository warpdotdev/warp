#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ProjectOnboardingSettings {
    #[default]
    NoProject,
    Project {
        selected_local_folder: String,
        initialize_projects_automatically: bool,
    },
}

impl ProjectOnboardingSettings {
    pub fn from_path(path: Option<String>) -> Self {
        match path {
            None => ProjectOnboardingSettings::NoProject,
            Some(path) => ProjectOnboardingSettings::Project {
                selected_local_folder: path,
                initialize_projects_automatically: true,
            },
        }
    }
}
