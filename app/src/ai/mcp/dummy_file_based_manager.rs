use crate::ai::mcp::templatable_installation::TemplatableMCPServerInstallation;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use super::MCPProvider;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

pub struct FileBasedMCPManager {}

impl FileBasedMCPManager {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {}
    }

    pub fn get_servers_for_working_directory(
        &self,
        _cwd: Option<&Path>,
        _app: &AppContext,
    ) -> Vec<&TemplatableMCPServerInstallation> {
        vec![]
    }

    pub fn file_based_servers(&self) -> Vec<&TemplatableMCPServerInstallation> {
        vec![]
    }

    pub fn get_installation_by_uuid(
        &self,
        _uuid: Uuid,
    ) -> Option<&TemplatableMCPServerInstallation> {
        None
    }

    pub fn directory_paths_for_installation_and_provider(
        &self,
        _uuid: Uuid,
        _provider: MCPProvider,
    ) -> Vec<PathBuf> {
        vec![]
    }
}

impl Entity for FileBasedMCPManager {
    type Event = ();
}

impl SingletonEntity for FileBasedMCPManager {}
