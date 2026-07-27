use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::sync_channel;

use ai::workspace::WorkspaceMetadata;
use chrono::Utc;
use lsp::supported_servers::LSPServerType;

use super::{EnablementState, PersistedWorkspace, Workspace};
use crate::persistence::ModelEvent;

#[test]
fn remove_workspace_language_servers_preserves_workspace_and_requests_persisted_deletion() {
    let path = PathBuf::from("/tmp/lsp-only-repo");
    let (sender, receiver) = sync_channel(1);
    let mut persisted_workspace = PersistedWorkspace {
        workspaces: HashMap::from([(
            path.clone(),
            Workspace {
                metadata: WorkspaceMetadata {
                    path: path.clone(),
                    navigated_ts: None,
                    modified_ts: Some(Utc::now()),
                    queried_ts: None,
                },
                language_servers: HashMap::from([(
                    LSPServerType::RustAnalyzer,
                    EnablementState::No,
                )]),
            },
        )]),
        model_event_sender: Some(sender),
        #[cfg(feature = "local_fs")]
        lsp_installation_status: HashMap::new(),
    };

    persisted_workspace.remove_workspace_language_servers(&path);

    assert_eq!(
        persisted_workspace.root_for_workspace(&path),
        Some(path.as_path())
    );
    assert_eq!(persisted_workspace.workspaces().count(), 1);
    assert_eq!(
        persisted_workspace
            .all_lsp_servers(&path, true)
            .expect("workspace should remain")
            .count(),
        0
    );
    assert!(matches!(
        receiver.try_recv(),
        Ok(ModelEvent::DeleteWorkspaceLanguageServers { workspace_path })
            if workspace_path == path
    ));
}

#[test]
fn remove_workspace_language_servers_is_a_noop_for_unknown_path() {
    let path = PathBuf::from("/tmp/not-tracked");
    let (sender, receiver) = sync_channel(1);
    let mut persisted_workspace = PersistedWorkspace {
        workspaces: HashMap::new(),
        model_event_sender: Some(sender),
        #[cfg(feature = "local_fs")]
        lsp_installation_status: HashMap::new(),
    };

    persisted_workspace.remove_workspace_language_servers(&path);

    assert!(receiver.try_recv().is_err());
}
