//! Deferred PTY-less terminal ownership for a TUI cloud-child session.
//!
//! This is the TUI counterpart of the GUI shared-session viewer's
//! `TerminalManager::new_deferred`: it retains a real terminal model and
//! stable surface identity before a shared-session transport is available.
use std::any::Any;
use std::sync::Arc;

use parking_lot::FairMutex;
use pathfinder_geometry::vector::Vector2F;
use warp::tui_export::{
    BlockSpacing, TerminalManagerTrait, TerminalModel, TerminalSurfaceInit,
    initialize_tui_cloud_viewer_terminal,
};
use warpui_core::AppContext;

/// Retains the PTY-less terminal model for a deferred cloud session.
pub(crate) struct TuiCloudTerminalManager {
    model: Arc<FairMutex<TerminalModel>>,
    _inactive_pty_reads_rx: async_broadcast::InactiveReceiver<Arc<Vec<u8>>>,
}

impl TuiCloudTerminalManager {
    /// Creates the manager and surface inputs before a shared session exists.
    pub(crate) fn new(
        initial_size: Vector2F,
        block_spacing: BlockSpacing,
        ctx: &mut AppContext,
    ) -> (Self, TerminalSurfaceInit) {
        let surface_init = initialize_tui_cloud_viewer_terminal(initial_size, block_spacing, ctx);
        let manager = Self {
            model: surface_init.model.clone(),
            _inactive_pty_reads_rx: surface_init.inactive_pty_reads_rx.clone(),
        };
        (manager, surface_init)
    }
}

impl TerminalManagerTrait for TuiCloudTerminalManager {
    fn model(&self) -> Arc<FairMutex<TerminalModel>> {
        self.model.clone()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
