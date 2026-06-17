//! Periodically shuts down idle LSP servers.
//!
//! Every `SCAN_INTERVAL` this singleton scans for workspaces that have a running LSP server
//! but no associated:
//!   1. Active local `TerminalView` whose repo root matches the workspace root.
//!   2. Open `LocalCodeEditorView` with a file that belongs to that workspace root.
//!
//! For those workspaces, we call `LspManagerModel::stop` to tear down the LSP server.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::stream::AbortHandle;
use lsp::LspManagerModel;
use warpui::r#async::Timer;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::code::local_code_editor::LocalCodeEditorView;
use crate::terminal::TerminalView;

const SCAN_INTERVAL: Duration = Duration::from_secs(10);

pub struct LanguageServerShutdownManager {
    in_progress_scan: Option<AbortHandle>,
}

impl LanguageServerShutdownManager {
    pub fn new() -> Self {
        Self {
            in_progress_scan: None,
        }
    }

    pub fn has_in_progress_scan(&self) -> bool {
        self.in_progress_scan.is_some()
    }

    pub fn schedule_next_scan(&mut self, ctx: &mut ModelContext<Self>) {
        if let Some(scan) = self.in_progress_scan.take() {
            scan.abort();
        }

        self.in_progress_scan = Some(
            ctx.spawn(
                async {
                    Timer::after(SCAN_INTERVAL).await;
                },
                |me, _, ctx| {
                    if me.scan_and_shutdown_unused_servers(ctx) {
                        me.schedule_next_scan(ctx);
                    } else {
                        me.in_progress_scan = None;
                    }
                },
            )
            .abort_handle(),
        );
    }

    /// Scans for unused LSP servers and shuts them down.
    ///
    /// Returns `true` if there are still active workspace roots remaining (indicating more scans
    /// may be needed), or `false` if all workspace roots were shut down or there were no roots.
    fn scan_and_shutdown_unused_servers(&self, ctx: &mut ModelContext<Self>) -> bool {
        let lsp_manager_handle = LspManagerModel::handle(ctx);
        let workspace_roots: Vec<PathBuf> = lsp_manager_handle
            .as_ref(ctx)
            .workspace_roots()
            .cloned()
            .collect();

        if workspace_roots.is_empty() {
            return false;
        }

        let mut unused_roots = Vec::new();
        for root in &workspace_roots {
            if !workspace_root_in_use(root, ctx) {
                unused_roots.push(root);
            }
        }

        // There will be remaining active roots if not all of the workspace roots are unused.
        let has_active_roots = unused_roots.len() < workspace_roots.len();

        if unused_roots.is_empty() {
            return has_active_roots;
        }

        // Stop servers for all workspaces that are no longer in use.
        lsp_manager_handle.update(ctx, |manager, m_ctx| {
            for root in unused_roots {
                log::info!("Stopping unused LSP for workspace {}", root.display());
                manager.stop_all(root.clone(), m_ctx);
            }
        });

        has_active_roots
    }
}

fn workspace_root_in_use(root: &Path, app: &AppContext) -> bool {
    has_terminal_for_workspace(root, app) || has_open_file_for_workspace(root, app)
}

fn has_terminal_for_workspace(root: &Path, app: &AppContext) -> bool {
    for window_id in app.window_ids() {
        if let Some(terminals) = app.views_of_type::<TerminalView>(window_id) {
            for terminal in terminals {
                let Some(pwd) = terminal.as_ref(app).canonical_session_pwd_if_local(app) else {
                    continue;
                };

                if pwd.as_path().starts_with(root) {
                    return true;
                }
            }
        }
    }

    false
}

fn has_open_file_for_workspace(root: &Path, app: &AppContext) -> bool {
    for window_id in app.window_ids() {
        if let Some(editors) = app.views_of_type::<LocalCodeEditorView>(window_id) {
            for editor in editors {
                let editor_ref = editor.as_ref(app);

                if !editor_ref.language_server_enabled() {
                    continue;
                }

                let Some(path) = editor_ref.file_path() else {
                    continue;
                };

                if path.starts_with(root) {
                    return true;
                }
            }
        }
    }

    false
}

impl Entity for LanguageServerShutdownManager {
    type Event = ();
}

impl SingletonEntity for LanguageServerShutdownManager {}
