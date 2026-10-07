//! UI state for the ambient agent progress/loading screen.

use warpui::ModelHandle;
use warpui::elements::shimmering_text::ShimmeringTextStateHandle;
use warpui::elements::{MouseStateHandle, SelectionHandle};

use crate::ai::agent_tips::AITipModel;
use crate::terminal::view::ambient_agent::CloudModeTip;
use crate::terminal::view::ambient_agent::model::AmbientAgentViewModel;

/// UI state for rendering the ambient agent progress screen (loading or error).
/// This keeps all cloud mode UI handles together and separates them from the main TerminalView.
pub struct AmbientAgentProgressUIState {
    /// State handle for the shimmering text animation in the cloud mode loading screen.
    pub loading_shimmer_handle: ShimmeringTextStateHandle,

    /// Model for displaying tips in the cloud mode loading screen (with 60s cooldown).
    pub tip_model: ModelHandle<AITipModel<CloudModeTip>>,

    /// Selection handle for making error text selectable in the cloud mode error screen.
    pub error_selection_handle: SelectionHandle,

    /// Stores selected text from the cloud mode error screen for copying.
    pub error_selected_text: std::rc::Rc<parking_lot::RwLock<Option<String>>>,

    /// Mouse state handle for the authenticate button in the GitHub auth screen.
    pub auth_button_mouse_state: MouseStateHandle,
}

impl AmbientAgentProgressUIState {
    /// Creates a new ambient agent progress UI state with initialized handles.
    pub fn new(ctx: &mut warpui::ModelContext<AmbientAgentViewModel>) -> Self {
        let tip_model = ctx.add_model(|_ctx| {
            use crate::terminal::view::ambient_agent;
            AITipModel::new(ambient_agent::get_cloud_mode_tips())
        });

        Self {
            loading_shimmer_handle: ShimmeringTextStateHandle::new(),
            tip_model,
            error_selection_handle: SelectionHandle::default(),
            error_selected_text: std::rc::Rc::new(parking_lot::RwLock::new(None)),
            auth_button_mouse_state: MouseStateHandle::default(),
        }
    }
}
