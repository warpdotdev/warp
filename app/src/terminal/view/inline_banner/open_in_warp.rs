use std::sync::Arc;

use warpui::{elements::MouseStateHandle, fonts::Weight, Element, EntityId};

use crate::{
    appearance::Appearance,
    terminal::{
        model::session::Session,
        view::{open_in_warp::OpenablePath, InlineBannerId, TerminalAction},
    },
    util::openable_file_type::OpenableFileType,
};

use super::{
    render_inline_block_list_banner, InlineBannerButtonState, InlineBannerCloseButton,
    InlineBannerContent, InlineBannerStyle, InlineBannerTextButton, InlineBannerTextButtonFont,
    InlineBannerTextButtonVariant,
};

#[derive(Clone, Copy, Debug)]
pub enum OpenInWarpBannerAction {
    OpenFile,
    LearnMore,
    Close,
}

pub struct OpenInWarpBannerState {
    pub id: InlineBannerId,
    pub target: OpenablePath,
    pub session: Arc<Session>,
    open_button_mouse_state: MouseStateHandle,
    learn_more_button_mouse_state: MouseStateHandle,
    close_button_mouse_state: MouseStateHandle,
}

impl OpenInWarpBannerState {
    pub fn new(id: InlineBannerId, openable_path: OpenablePath, session: Arc<Session>) -> Self {
        Self {
            id,
            target: openable_path,
            session,
            open_button_mouse_state: Default::default(),
            learn_more_button_mouse_state: Default::default(),
            close_button_mouse_state: Default::default(),
        }
    }
}

/// Given an openable file, format a file-specific title for the Open in Warp banner.
fn file_title_text(openable_path: &OpenablePath) -> String {
    match openable_path.file_type {
        OpenableFileType::Markdown => {
            "Did you know that Warp can directly display Markdown files?".to_string()
        }
        OpenableFileType::Code | OpenableFileType::Text => {
            cfg_if::cfg_if! {
                if #[cfg(not(target_family = "wasm"))] {
                    // Language is a temporary variable to ensure our copy of the Arc<Language>
                    // lives long enough to borrow the display name for the duration of the function.
                    let language = languages::language_by_local_filename(&openable_path.path);

                    match language.as_ref().map(|language| language.display_name()) {
                        Some(display_name) => {
                            format!("Did you know that Warp can directly edit {display_name} files?")
                        }
                        None => "Did you know that Warp can directly edit code?".to_string(),
                    }
                } else {
                    // The `languages` crate is not available on WASM, so use a fallback message.
                    "Did you know that Warp can directly edit code?".to_string()
                }
            }
        }
    }
}

pub fn render_open_in_warp_banner(
    state: &OpenInWarpBannerState,
    view_id: EntityId,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let button_text = match state.target.file_type {
        OpenableFileType::Markdown => "View in Warp",
        OpenableFileType::Code | OpenableFileType::Text => "Edit in Warp",
    };

    let open_button = InlineBannerTextButton {
        text: button_text.to_string(),
        text_color: appearance.theme().active_ui_text_color().into_solid(),
        button_state: InlineBannerButtonState {
            on_click_event: TerminalAction::OpenInWarpBanner(OpenInWarpBannerAction::OpenFile),
            mouse_state_handle: state.open_button_mouse_state.clone(),
        },
        font: InlineBannerTextButtonFont {
            weight: Some(Weight::Bold),
            ..Default::default()
        },
        position_id: Some(format!("open_in_warp_banner_button_{view_id}")),
        variant: InlineBannerTextButtonVariant::Primary,
    };

    let learn_more_button = InlineBannerTextButton {
        text: "Learn more".to_string(),
        text_color: appearance.theme().active_ui_text_color().into_solid(),
        button_state: InlineBannerButtonState {
            on_click_event: TerminalAction::OpenInWarpBanner(OpenInWarpBannerAction::LearnMore),
            mouse_state_handle: state.learn_more_button_mouse_state.clone(),
        },
        font: Default::default(),
        position_id: None,
        variant: InlineBannerTextButtonVariant::Secondary,
    };

    let close_button = InlineBannerCloseButton(InlineBannerButtonState {
        on_click_event: TerminalAction::OpenInWarpBanner(OpenInWarpBannerAction::Close),
        mouse_state_handle: state.close_button_mouse_state.clone(),
    });

    let title = file_title_text(&state.target);

    render_inline_block_list_banner(
        InlineBannerStyle::Recommendation,
        appearance,
        InlineBannerContent {
            title,
            buttons: vec![open_button, learn_more_button],
            close_button: Some(close_button),
            ..Default::default()
        },
    )
}
