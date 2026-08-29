//! State for banners that render inside a block. The render functions live in the app's view
//! layer; this module holds only the state the model stores and measures.

use warpui_core::elements::MouseStateHandle;
use warpui_core::keymap::Keystroke;

pub const CONSTRAINED_BANNER_HEIGHT: f32 = 48.;
pub const BANNER_TOP_MARGIN: f32 = 16.;
pub const BLOCK_BANNER_HEIGHT: f32 = CONSTRAINED_BANNER_HEIGHT + BANNER_TOP_MARGIN;

pub enum WithinBlockBanner {
    WarpifyBanner(WarpifyBannerState),
}

impl WithinBlockBanner {
    pub fn banner_height(&self) -> f32 {
        match self {
            WithinBlockBanner::WarpifyBanner(_) => BLOCK_BANNER_HEIGHT,
        }
    }
}

pub struct WarpifyBannerState {
    /// The subshell command that triggered the banner.
    pub command: String,
    pub height: f32,
    pub accept_button_mouse_state: MouseStateHandle,
    pub dont_ask_button_mouse_state: MouseStateHandle,
    pub dismiss_button_mouse_state: MouseStateHandle,

    /// This keybinding gets rendered in the Warpification banner, but we can't look it up
    /// during render as a &mut AppContext is not available then. This needs to get
    /// looked up during action handling and cached here.
    pub initialize_warpify_keybinding: Option<Keystroke>,
    pub hover_state: MouseStateHandle,
}

impl WarpifyBannerState {
    pub fn new(command: String, initialize_warpify_keybinding: Option<Keystroke>) -> Self {
        Self {
            command,
            height: 0.0,
            initialize_warpify_keybinding,
            accept_button_mouse_state: Default::default(),
            dont_ask_button_mouse_state: Default::default(),
            dismiss_button_mouse_state: Default::default(),
            hover_state: Default::default(),
        }
    }

    pub fn title(&self) -> &str {
        "Warpify subshell"
    }
}
