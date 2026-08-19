//! Confirmation banner shown in the composer when the user picks a video file to attach as
//! context (behind `FeatureFlag::VideoAsContext`).
//!
//! LLM APIs don't accept native video attachments, so we convert the video into a handful of
//! still frames (plus an optional audio transcript) behind the scenes and send those through the
//! existing image-as-context path. This banner is the disclaimer that tells the user that's about
//! to happen, and lets them opt in to including a transcript of the video's audio before we do
//! any processing.

use std::path::PathBuf;

use warpui::Element;
use warpui::elements::{
    Align, ConstrainedBox, Container, CrossAxisAlignment, Flex, MainAxisSize, MouseStateHandle,
    ParentElement, Shrinkable,
};
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};

use super::Input;
use crate::appearance::Appearance;
use crate::terminal::view::TerminalAction;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;

const PADDING: f32 = 10.;
const ATTACH_BUTTON_TEXT: &str = "Attach as frames";
const CANCEL_BUTTON_TEXT: &str = "Cancel";
const INCLUDE_AUDIO_LABEL: &str = "Include audio transcript";

/// State for the video-attach confirmation banner. Rendered by [`Input`] while `Some`; cleared
/// once the user confirms or cancels.
#[derive(Clone)]
pub struct VideoAttachBannerState {
    /// Path to the video file on disk, as picked from the file picker.
    pub file_path: PathBuf,
    /// Display name of the video (e.g. `"screen-recording.mp4"`).
    pub file_name: String,
    /// Whether the "Include audio transcript" checkbox is checked. Defaults to unchecked
    /// (audio is opt-in) per the video-as-context prototype's product decision.
    pub include_audio_checked: bool,

    pub checkbox_mouse_state: MouseStateHandle,
    pub attach_button_mouse_state: MouseStateHandle,
    pub cancel_button_mouse_state: MouseStateHandle,
}

impl VideoAttachBannerState {
    pub fn new(file_path: PathBuf, file_name: String) -> Self {
        Self {
            file_path,
            file_name,
            include_audio_checked: false,
            checkbox_mouse_state: Default::default(),
            attach_button_mouse_state: Default::default(),
            cancel_button_mouse_state: Default::default(),
        }
    }
}

/// Actions dispatched from the video-attach banner. Routed through [`TerminalAction`] since the
/// banner's buttons dispatch actions that must reach the owning [`Input`] view.
#[derive(Clone, Debug)]
pub enum VideoAttachBannerAction {
    /// Shows the banner for a newly picked video file.
    Show {
        file_path: PathBuf,
        file_name: String,
    },
    /// Toggles the "Include audio transcript" checkbox.
    ToggleIncludeAudio,
    /// Confirms attaching the video: dismisses the banner and starts frame extraction.
    Confirm,
    /// Dismisses the banner without attaching anything.
    Cancel,
}

impl Input {
    /// Shows the video-attach confirmation banner for a video file path obtained outside the
    /// file picker (drag-and-drop or a system-clipboard paste of a file path). Mirrors what the
    /// file picker does in `EditorView::attach_files`, so every entrypoint that can produce a
    /// video file path routes through the same disclaimer/checkbox confirmation rather than only
    /// the picker.
    pub(crate) fn show_video_attach_banner_for_path(
        &mut self,
        video_path: String,
        ctx: &mut warpui::ViewContext<Self>,
    ) {
        let file_name = std::path::Path::new(&video_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&video_path)
            .to_string();
        self.handle_video_attach_banner_action(
            VideoAttachBannerAction::Show {
                file_path: PathBuf::from(video_path),
                file_name,
            },
            ctx,
        );
    }

    /// Handles a [`VideoAttachBannerAction`] dispatched from the banner (or from the file picker,
    /// for `Show`).
    pub(crate) fn handle_video_attach_banner_action(
        &mut self,
        action: VideoAttachBannerAction,
        ctx: &mut warpui::ViewContext<Self>,
    ) {
        match action {
            VideoAttachBannerAction::Show {
                file_path,
                file_name,
            } => {
                self.video_attach_banner_state =
                    Some(VideoAttachBannerState::new(file_path, file_name));
            }
            VideoAttachBannerAction::ToggleIncludeAudio => {
                if let Some(state) = &mut self.video_attach_banner_state {
                    state.include_audio_checked = !state.include_audio_checked;
                }
            }
            VideoAttachBannerAction::Cancel => {
                self.video_attach_banner_state = None;
            }
            VideoAttachBannerAction::Confirm => {
                let Some(state) = self.video_attach_banner_state.take() else {
                    return;
                };
                self.editor.update(ctx, |editor, ctx| {
                    editor.read_and_process_video_async(
                        state.file_path,
                        state.file_name,
                        state.include_audio_checked,
                        ctx,
                    );
                });
            }
        }
        ctx.notify();
    }

    /// Renders the video-attach confirmation banner, or `None` when there's no pending video.
    pub(super) fn render_video_attach_banner(
        &self,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let state = self.video_attach_banner_state.as_ref()?;
        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder();

        let mut banner = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);

        let info_icon = Container::new(
            ConstrainedBox::new(
                Icon::Info
                    .to_warpui_icon(theme.active_ui_text_color())
                    .finish(),
            )
            .with_width(appearance.ui_font_size() * 1.2)
            .with_height(appearance.ui_font_size() * 1.2)
            .finish(),
        )
        .with_padding_right(6.)
        .finish();

        let disclaimer_text = ui_builder
            .span(format!(
                "\u{201c}{}\u{201d} will be sent as still frames (not native video) — audio isn't included unless you check the box.",
                state.file_name
            ))
            .with_style(UiComponentStyles {
                font_color: Some(blended_colors::text_sub(theme, theme.surface_1())),
                font_size: Some(appearance.ui_font_size()),
                ..Default::default()
            })
            .build()
            .finish();

        let mut left = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        left.add_child(info_icon);
        left.add_child(Shrinkable::new(1., Align::new(disclaimer_text).left().finish()).finish());
        banner.add_child(Shrinkable::new(1., left.finish()).finish());

        let checkbox = ui_builder
            .checkbox(
                state.checkbox_mouse_state.clone(),
                Some(appearance.ui_font_size()),
            )
            .check(state.include_audio_checked)
            .with_style(UiComponentStyles {
                padding: Some(Coords::default().right(2.)),
                ..Default::default()
            })
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(TerminalAction::VideoAttachBanner(
                    VideoAttachBannerAction::ToggleIncludeAudio,
                ));
            })
            .with_cursor(Cursor::PointingHand)
            .finish();

        let checkbox_label = ui_builder
            .span(INCLUDE_AUDIO_LABEL)
            .with_style(UiComponentStyles {
                font_color: Some(blended_colors::text_main(theme, theme.surface_1())),
                font_size: Some(appearance.ui_font_size()),
                ..Default::default()
            })
            .build()
            .finish();

        banner.add_child(
            Container::new(
                Align::new(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(checkbox)
                        .with_child(checkbox_label)
                        .finish(),
                )
                .finish(),
            )
            .with_horizontal_padding(8.)
            .finish(),
        );

        let button_styles = UiComponentStyles {
            font_color: Some(theme.foreground().into_solid()),
            font_size: Some(appearance.ui_font_size()),
            padding: Some(Coords {
                top: 5.0,
                bottom: 5.0,
                left: 8.0,
                right: 8.0,
            }),
            ..Default::default()
        };

        banner.add_child(
            Container::new(
                Align::new(
                    ui_builder
                        .button(
                            ButtonVariant::Outlined,
                            state.cancel_button_mouse_state.clone(),
                        )
                        .with_text_label(CANCEL_BUTTON_TEXT.to_string())
                        .with_style(button_styles)
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(TerminalAction::VideoAttachBanner(
                                VideoAttachBannerAction::Cancel,
                            ));
                        })
                        .finish(),
                )
                .finish(),
            )
            .with_horizontal_padding(4.)
            .finish(),
        );

        banner.add_child(
            Container::new(
                Align::new(
                    ui_builder
                        .button(
                            ButtonVariant::Outlined,
                            state.attach_button_mouse_state.clone(),
                        )
                        .with_text_label(ATTACH_BUTTON_TEXT.to_string())
                        .with_style(button_styles)
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(TerminalAction::VideoAttachBanner(
                                VideoAttachBannerAction::Confirm,
                            ));
                        })
                        .finish(),
                )
                .finish(),
            )
            .with_horizontal_padding(4.)
            .finish(),
        );

        Some(
            Container::new(banner.finish())
                .with_background(theme.surface_1())
                .with_uniform_padding(PADDING)
                .finish(),
        )
    }
}
