use std::path::{Path, PathBuf};

use warpui_core::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity};

use crate::model::LanguageServerId;
use crate::supported_servers::LSPServerType;
use crate::{LspServerConfig, LspServerModel};

#[derive(Debug)]
pub enum LspManagerModelEvent {
    ServerStarted(PathBuf),
    ServerStopped(PathBuf),
    ServerRemoved {
        workspace_root: PathBuf,
        server_type: LSPServerType,
        server_id: LanguageServerId,
    },
}

#[derive(Default)]
pub struct LspManagerModel;

impl LspManagerModel {
    pub fn new() -> Self {
        Self
    }

    pub fn workspace_roots(&self) -> impl Iterator<Item = &PathBuf> {
        std::iter::empty()
    }

    pub fn servers_for_workspace(&self, _path: &Path) -> Option<&Vec<ModelHandle<LspServerModel>>> {
        None
    }

    pub fn server_registered(
        &self,
        _path: &Path,
        _server_type: LSPServerType,
        _ctx: &AppContext,
    ) -> bool {
        false
    }

    pub fn server_registered_and_started(
        &self,
        _path: &Path,
        _server_type: LSPServerType,
        _ctx: &AppContext,
    ) -> bool {
        false
    }

    pub fn server_for_path(
        &self,
        _path: &Path,
        _ctx: &AppContext,
    ) -> Option<ModelHandle<LspServerModel>> {
        None
    }

    pub fn maybe_register_external_file(&mut self, _path: &Path, _server_id: LanguageServerId) {}

    pub fn server_by_id(
        &self,
        _id: LanguageServerId,
        _ctx: &AppContext,
    ) -> Option<ModelHandle<LspServerModel>> {
        None
    }

    pub fn register(
        &mut self,
        _path: PathBuf,
        _config: LspServerConfig,
        _ctx: &mut ModelContext<Self>,
    ) -> bool {
        false
    }

    pub fn start_all(&mut self, _path: PathBuf, _ctx: &mut ModelContext<Self>) {}

    pub fn stop_all(&mut self, _path: PathBuf, _ctx: &mut ModelContext<Self>) {}

    pub fn remove_server(
        &mut self,
        _workspace_root: &Path,
        _server_type: LSPServerType,
        _ctx: &mut ModelContext<Self>,
    ) {
    }

    pub fn terminate(&mut self, _ctx: &mut ModelContext<Self>) {}

    pub fn lsp_model_for_path(&self, _path: &Path) -> Option<&[ModelHandle<LspServerModel>]> {
        None
    }

    pub fn repo_path_for_path(_path: &Path, _ctx: &AppContext) -> Option<PathBuf> {
        None
    }
}

impl Entity for LspManagerModel {
    type Event = LspManagerModelEvent;
}

impl SingletonEntity for LspManagerModel {}
