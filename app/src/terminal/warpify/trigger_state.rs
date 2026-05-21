use parking_lot::FairMutex;
use warp_core::ui::appearance::Appearance;
use warpui::{
    r#async::SpawnedFutureHandle, AppContext, EntityId, SingletonEntity as _, ViewContext,
    ViewHandle,
};

use crate::terminal::{
    model::{
        ansi::SystemDetails,
        block::BlockId,
        session::SessionId,
        terminal_model::{SubshellInitializationInfo, TmuxInstallationState},
    },
    settings::TerminalSettings,
    shell::ShellType,
    ssh::{error::SshErrorBlock, install_tmux::SshInstallTmuxBlock, warpify::SshWarpifyBlock},
    TerminalModel, TerminalView,
};
use std::{collections::HashMap, sync::Arc};

use super::success_block::WarpifySuccessBlock;

/// A unique identifier for a subshell separator.
pub type SeparatorId = usize;

/// These are elements in the BlockList which are similar to inline banners but are smaller, and
/// only meant to render in compact mode when their in-padding flag counterparts don't have enough
/// space in the padding to render.
#[derive(Default)]
struct SubshellSeparatorState {
    /// The ID for the next separator to be created.
    next_separator_id: SeparatorId,

    /// These are for rendering above the first block of a subshell session.
    separators: HashMap<SeparatorId, String>,
}

impl SubshellSeparatorState {
    /// Returns the ID to assign to the next separator
    fn next_separator_id(&mut self) -> SeparatorId {
        let next_id = self.next_separator_id;
        self.next_separator_id += 1;
        next_id
    }
}

#[derive(Debug)]
pub enum SshBlockState {
    Warpifying {
        handle: ViewHandle<SshWarpifyBlock>,
    },
    WarpifySuccess {
        handle: ViewHandle<WarpifySuccessBlock>,
    },
    InstallTmux {
        handle: ViewHandle<SshInstallTmuxBlock>,
    },
    Error {
        handle: ViewHandle<SshErrorBlock>,
    },
}

impl SshBlockState {
    pub fn should_prevent_input(&self) -> bool {
        !matches!(self, SshBlockState::InstallTmux { .. })
    }

    pub fn get_block_view_id(&self) -> EntityId {
        match self {
            SshBlockState::Warpifying { handle, .. } => handle.id(),
            SshBlockState::WarpifySuccess { handle, .. } => handle.id(),
            SshBlockState::InstallTmux { handle, .. } => handle.id(),
            SshBlockState::Error { handle } => handle.id(),
        }
    }

    /// Returns `true` if the script was previously visible and is now collapsed.
    pub fn collapse_script(&mut self, ctx: &mut ViewContext<TerminalView>) -> bool {
        match self {
            SshBlockState::InstallTmux { handle, .. } => {
                handle.update(ctx, |block, ctx| block.collapse_script(ctx))
            }
            SshBlockState::Warpifying { .. } => false,
            SshBlockState::WarpifySuccess { .. } => false,
            SshBlockState::Error { .. } => false,
        }
    }

    pub fn focus(&mut self, ctx: &mut ViewContext<TerminalView>) {
        match self {
            SshBlockState::Warpifying { handle } => {
                handle.update(ctx, |block, ctx| block.focus(ctx));
            }
            SshBlockState::WarpifySuccess { .. } => {}
            SshBlockState::InstallTmux { handle } => {
                handle.update(ctx, |block, ctx| block.focus(ctx));
            }
            SshBlockState::Error { handle } => {
                handle.update(ctx, |block, ctx| block.focus(ctx));
            }
        }
    }

    pub fn get_system_details(&self, app: &AppContext) -> Option<SystemDetails> {
        match self {
            SshBlockState::InstallTmux { handle, .. } => {
                Some(handle.read(app, |view, _| view.system_details()))
            }
            SshBlockState::Warpifying { .. } => None,
            SshBlockState::WarpifySuccess { .. } => None,
            SshBlockState::Error { .. } => None,
        }
    }

    pub fn on_warpified_session_complete(
        &self,
        ctx: &mut ViewContext<TerminalView>,
    ) -> Option<EntityId> {
        match self {
            SshBlockState::InstallTmux { .. } | SshBlockState::Warpifying { .. } => {
                let block_id = self.get_block_view_id();
                return Some(block_id);
            }
            SshBlockState::WarpifySuccess { handle } => {
                handle.update(ctx, |block, ctx| {
                    block.on_warpified_session_complete(ctx);
                });
            }
            SshBlockState::Error { .. } => {}
        }
        None
    }
}

/// Temporary state used to trigger Warpification.
#[derive(Default)]
struct WarpifyTriggerState {
    block_id: Option<BlockId>,

    /// Lets us abort an attempt to auto warpify if the subshell command
    /// hasn't completed.
    auto_warpify_abort_handle: Option<SpawnedFutureHandle>,

    /// The subshell banner waits 1s before showing. This is to see that the command stays running
    /// for a while without exiting. We store the abort handle here so that the
    /// TerminalEvent::BlockCompleted event can abort the banner.
    subshell_banner_abort_handle: Option<SpawnedFutureHandle>,

    /// The command which may trigger ssh Warpification
    pending_command: Option<String>,
    /// The Host which may trigger ssh Warpification
    pending_warpify_ssh_host: Option<String>,

    /// Which, if any, SSH block is currently added to the blocklist.
    ssh_block_state: Option<SshBlockState>,

    ssh_warpify_timeout_handle: Option<SpawnedFutureHandle>,

    shell_type: Option<ShellType>,

    is_shell_detection_in_progress: bool,

    tmux_installation: TmuxInstallationState,
}

#[derive(Default)]
pub struct WarpifyState {
    session_id: Option<SessionId>,

    pending_state: Option<WarpifyTriggerState>,
    /// Stores the metadata needed to render any separators above the first block of a subshell.
    subshell_separator_state: SubshellSeparatorState,
    /// A unique-enough ID that is used to validate that a timeout is still valid.
    timeout_id: u8,
}

impl WarpifyState {
    pub fn delete_state(&mut self) {
        self.pending_state.take();
    }

    pub fn is_shell_detection_in_progress(&self) -> bool {
        self.pending_state
            .as_ref()
            .map(|state| state.is_shell_detection_in_progress)
            .unwrap_or_default()
    }

    pub fn set_shell_detection_in_progress(&mut self) {
        if let Some(ref mut pending_state) = self.pending_state.as_mut() {
            pending_state.is_shell_detection_in_progress = true;
        }
    }

    pub fn set_shell_type(&mut self, shell_type: &ShellType) {
        let pending_state = self.pending_state.get_or_insert_with(Default::default);
        pending_state.shell_type = Some(shell_type.to_owned());
        pending_state.is_shell_detection_in_progress = false;
    }

    pub fn get_shell_type(&self) -> Option<ShellType> {
        self.pending_state
            .as_ref()
            .and_then(|state| state.shell_type)
    }

    pub fn add_subshell_separator(
        &mut self,
        subshell_info: &SubshellInitializationInfo,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        ctx: &mut ViewContext<TerminalView>,
    ) {
        let Some(command) = subshell_info.spawning_command.split_whitespace().next() else {
            return;
        };
        let separator_id = self.subshell_separator_state.next_separator_id();
        let appearance = Appearance::as_ref(ctx);
        let terminal_spacing =
            TerminalSettings::as_ref(ctx).terminal_spacing(appearance.line_height_ratio(), ctx);
        let height = terminal_spacing.subshell_separator_height;
        self.subshell_separator_state
            .separators
            .insert(separator_id, command.to_owned());
        terminal_model
            .lock()
            .block_list_mut()
            .append_subshell_separator(separator_id, height);
        ctx.notify();
    }

    pub fn get_subshell_separators(&self) -> &HashMap<SeparatorId, String> {
        &self.subshell_separator_state.separators
    }

    pub fn add_subshell_banner_abort_handle(&mut self, spawned_future_handle: SpawnedFutureHandle) {
        let pending_state = self.pending_state.get_or_insert_with(Default::default);
        pending_state.subshell_banner_abort_handle = Some(spawned_future_handle);
    }

    pub fn take_subshell_banner_abort_handle(&mut self) -> Option<SpawnedFutureHandle> {
        self.pending_state
            .as_mut()
            .and_then(|state| state.subshell_banner_abort_handle.take())
    }

    pub fn add_auto_warpify_abort_handle(&mut self, spawned_future_handle: SpawnedFutureHandle) {
        let pending_state = self.pending_state.get_or_insert_with(Default::default);
        pending_state.auto_warpify_abort_handle = Some(spawned_future_handle);
    }

    pub fn abort_auto_warpify(&mut self) {
        if let Some(abort_handle) = self
            .pending_state
            .as_mut()
            .and_then(|state| state.auto_warpify_abort_handle.take())
        {
            abort_handle.abort();
        };
    }

    pub fn add_ssh_warpify_timeout_handle(&mut self, spawned_future_handle: SpawnedFutureHandle) {
        let pending_state = self.pending_state.get_or_insert_with(Default::default);
        pending_state.ssh_warpify_timeout_handle = Some(spawned_future_handle);
    }

    pub fn abort_ssh_warpify_timeout(&mut self) {
        self.replace_timeout_id();
        if let Some(handle) = self
            .pending_state
            .as_mut()
            .and_then(|state| state.ssh_warpify_timeout_handle.take())
        {
            handle.abort();
        };
    }

    pub fn collapse_ssh_block(&mut self, ctx: &mut ViewContext<TerminalView>) -> bool {
        if let Some(ref mut pending_state) = self.pending_state.as_mut() {
            if let Some(ref mut ssh_block_state) = &mut pending_state.ssh_block_state {
                return ssh_block_state.collapse_script(ctx);
            }
        }
        false
    }

    pub fn focus(&mut self, ctx: &mut ViewContext<TerminalView>) {
        if let Some(ref mut pending_state) = self.pending_state.as_mut() {
            if let Some(ref mut ssh_block_state) = &mut pending_state.ssh_block_state {
                ssh_block_state.focus(ctx);
            }
        }
    }

    pub fn clear_ssh_block_state(&mut self) {
        if let Some(ref mut pending_state) = self.pending_state.as_mut() {
            pending_state.ssh_block_state = None;
        }
    }

    pub fn set_ssh_block_state(&mut self, ssh_block_state: SshBlockState) {
        let pending_state = self.pending_state.get_or_insert_with(Default::default);
        pending_state.ssh_block_state = Some(ssh_block_state);
    }

    pub fn ssh_block_state(&self) -> Option<&SshBlockState> {
        self.pending_state
            .as_ref()
            .and_then(|state| state.ssh_block_state.as_ref())
    }

    pub fn get_pending_ssh_host(&self) -> Option<String> {
        self.pending_state
            .as_ref()
            .and_then(|state: &WarpifyTriggerState| state.pending_warpify_ssh_host.clone())
    }

    pub fn get_pending_ssh_command(&self) -> Option<String> {
        self.pending_state
            .as_ref()
            .and_then(|state: &WarpifyTriggerState| state.pending_command.clone())
    }

    pub fn take_pending_ssh_host(&mut self) -> Option<String> {
        self.pending_state
            .as_mut()
            .and_then(|state: &mut WarpifyTriggerState| state.pending_warpify_ssh_host.take())
    }

    pub fn clear_pending_ssh_host(&mut self) {
        if let Some(ref mut pending_state) = self.pending_state.as_mut() {
            pending_state.pending_warpify_ssh_host = None;
        }
    }

    pub fn set_pending_ssh_host(&mut self, command: String, ssh_host: Option<String>) {
        let pending_state = self.pending_state.get_or_insert_with(Default::default);
        pending_state.pending_command = Some(command);
        pending_state.pending_warpify_ssh_host = ssh_host;
    }

    pub fn set_tmux_installation_state(&mut self, tmux_installation: TmuxInstallationState) {
        if let Some(ref mut pending_state) = self.pending_state.as_mut() {
            pending_state.tmux_installation = tmux_installation;
        }
    }

    pub fn tmux_installation(&self) -> Option<TmuxInstallationState> {
        self.pending_state
            .as_ref()
            .map(|state| state.tmux_installation)
    }

    pub fn set_block_id(&mut self, block_id: BlockId) {
        let pending_state = self.pending_state.get_or_insert_with(Default::default);
        pending_state.block_id = Some(block_id);
    }

    pub fn block_id(&self) -> Option<BlockId> {
        self.pending_state
            .as_ref()
            .and_then(|state| state.block_id.clone())
    }

    pub fn timeout_id(&self) -> u8 {
        self.timeout_id
    }

    /// Generates a new timeout ID. This is used to validate that a timeout is still valid.
    /// Call this to get a new timeout ID before starting a new timeout, or to invalidate
    /// an existing timeout.
    pub fn replace_timeout_id(&mut self) -> u8 {
        self.timeout_id = self.timeout_id.wrapping_add(1);
        self.timeout_id
    }

    /// The terminal view should prevent typing
    pub fn should_prevent_input(&self) -> bool {
        let Some(state) = self.ssh_block_state() else {
            return false;
        };
        state.should_prevent_input()
    }

    /// Called once whenever we get a local block completed, as opposed to a remote ssh block
    /// and we have a Warpify Success block.
    fn on_warpified_session_complete(
        &mut self,
        state: WarpifyTriggerState,
        ctx: &mut ViewContext<TerminalView>,
    ) -> Option<EntityId> {
        self.clear_ssh_block_state();
        ctx.notify();
        let Some(block) = &state.ssh_block_state else {
            return None;
        };
        block.on_warpified_session_complete(ctx)
    }

    pub fn on_warpify_start(&mut self, active_session_id: Option<SessionId>) {
        self.session_id = active_session_id;
    }

    /// Called whenever a block is completed, to determine whether a Warpified session
    /// has been completed.
    pub fn get_completed_warpify_session_id(
        &mut self,
        active_session_id: Option<SessionId>,
        ctx: &mut ViewContext<TerminalView>,
    ) -> Option<EntityId> {
        if self.session_id.is_none() || active_session_id == self.session_id {
            return None;
        }
        if let Some(state) = self.pending_state.take() {
            return self.on_warpified_session_complete(state, ctx);
        };
        None
    }
}
