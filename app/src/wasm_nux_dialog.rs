use crate::appearance::Appearance;
use crate::report_if_error;
use crate::settings::app_installation_detection::{
    UserAppInstallDetectionSettings, UserAppInstallStatus,
};
use crate::settings::{NativePreferenceSettings, UserNativePreference};
use crate::ui_components::dialog::{dialog_styles, Dialog};
use crate::uri::web_intent_parser::{self, WebIntent};
use settings::Setting as _;
use warpui::elements::{Align, CrossAxisAlignment, Flex};
use warpui::ui_components::{
    button::ButtonVariant,
    components::{Coords, UiComponent, UiComponentStyles},
};
use warpui::{
    elements::{MainAxisSize, MouseStateHandle, ParentElement as _},
    fonts::Weight,
    platform::Cursor,
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
};

const CLOSE_BUTTON_DIAMETER: f32 = 20.;
const DIALOG_WIDTH: f32 = 350.;
const DIALOG_PADDING: f32 = 24.;
const OPEN_NATIVE_BUTTON_WIDTH: f32 = 260.;
const OPEN_NATIVE_BUTTON_HEIGHT: f32 = 40.;

#[derive(Debug, Copy, Clone)]
pub enum WasmNUXDialogAction {
    /// Close and dismiss the dialog
    Close,
    /// Closes the dialog and sets the native preference to web
    SetWebAndClose,
    /// Closes the dialog and open on the desktop
    OpenNativeAndClose,
    /// Open the Warp download page
    OpenDownloadDesktopAppLink,
    /// Open a link to learn more about Warp
    LearnMore,
}

pub enum WasmNUXDialogEvent {
    Close,
}

/// A dialog that prompts the user to:
/// * Download Warp if they haven't already
/// * Explicitly choose between native and web.
pub struct WasmNUXDialog {
    close_mouse_state: MouseStateHandle,
    confirm_mouse_state: MouseStateHandle,
    download_warp_mouse_state: MouseStateHandle,
    learn_more_mouse_state: MouseStateHandle,
    requested_download: bool,
}

impl Entity for WasmNUXDialog {
    type Event = WasmNUXDialogEvent;
}

impl WasmNUXDialog {
    pub fn new() -> Self {
        Self {
            close_mouse_state: Default::default(),
            confirm_mouse_state: Default::default(),
            download_warp_mouse_state: Default::default(),
            learn_more_mouse_state: Default::default(),
            requested_download: false,
        }
    }

    /// Whether or not the new-user dialog should be shown.
    ///
    /// It's shown if all of the following are true:
    /// * Not on a mobile device (mobile users can't use the desktop app)
    /// * The user does not have an explicit native/web preference
    /// * The user hasn't dismissed the dialog
    ///
    /// If the user dismisses the dialog without choosing a preference, we'll continue to use the default autodetection
    /// behavior: if Warp is installed, redirect to it; otherwise stay on the web.
    pub fn should_display(app: &AppContext) -> bool {
        // Don't show on mobile devices - they can't use the desktop app
        if warpui::platform::wasm::is_mobile_device() {
            return false;
        }

        let preference_settings = NativePreferenceSettings::handle(app).as_ref(app);
        *preference_settings.user_native_redirect_preference.value()
            == UserNativePreference::NotSelected
            && !*preference_settings.preference_dialog_dismissed.value()
    }

    fn render_dialog_button(
        text: impl Into<String>,
        action: WasmNUXDialogAction,
        mouse_state: &MouseStateHandle,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let button_style = UiComponentStyles {
            font_size: Some(14.),
            font_weight: Some(Weight::Bold),
            width: Some(OPEN_NATIVE_BUTTON_WIDTH),
            height: Some(OPEN_NATIVE_BUTTON_HEIGHT),
            ..Default::default()
        };

        appearance
            .ui_builder()
            .button(ButtonVariant::Accent, mouse_state.clone())
            .with_centered_text_label(text.into())
            .with_style(button_style)
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(action);
            })
            .finish()
    }
}

impl View for WasmNUXDialog {
    fn ui_name() -> &'static str {
        "WasmNUXDialog"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::handle(app).as_ref(app);

        // There are two general cases with the dialog:
        // 1. The user doesn't have Warp installed - treat them as a potential new user and encourage downloading Warp.
        // 2. The user has Warp installed, but clicked through to the web - ask if they want to always default to web.
        // As a sub-state of case 1, if the user clicks the download button, we provide an intent into the app.

        let close_button = appearance
            .ui_builder()
            .close_button(CLOSE_BUTTON_DIAMETER, self.close_mouse_state.clone())
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(WasmNUXDialogAction::Close);
            })
            .finish();

        let app_install_detected = UserAppInstallDetectionSettings::handle(app)
            .as_ref(app)
            .user_app_installation_detected
            .value();

        let dialog_styles = UiComponentStyles {
            width: Some(DIALOG_WIDTH),
            padding: Some(Coords::uniform(DIALOG_PADDING)),
            ..dialog_styles(appearance)
        };

        let dialog = if self.requested_download {
            Dialog::new(
                crate::i18n::tr_static(app, "Open in Warp Desktop?").to_string(),
                Some(
                    crate::i18n::tr_static(app, "Future links will automatically open on desktop.")
                        .to_string(),
                ),
                dialog_styles,
            )
            .with_bottom_row_child(Self::render_dialog_button(
                crate::i18n::tr_static(app, "Open in Warp"),
                WasmNUXDialogAction::OpenNativeAndClose,
                &self.confirm_mouse_state,
                appearance,
            ))
        } else if app_install_detected == &UserAppInstallStatus::NotDetected {
            Dialog::new(
                crate::i18n::tr_static(app, "Download Warp Desktop?").to_string(),
                None,
                dialog_styles,
            )
                .with_child(
                    Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_main_axis_size(MainAxisSize::Min)
                        .with_child(
                            appearance
                                .ui_builder()
                                .span(crate::i18n::tr_static(app, "Warp is the intelligent terminal with AI and your dev team's knowledge built-in."))
                                .with_style(UiComponentStyles {
                                    font_weight: Some(Weight::Thin),
                                    font_color: Some(
                                        appearance
                                            .theme()
                                            .main_text_color(appearance.theme().surface_1())
                                            .into_solid(),
                                    ),
                                    ..Default::default()
                                })
                                .with_soft_wrap()
                                .build()
                                .finish(),
                        )
                        .with_child(
                            Align::new(
                                appearance
                                    .ui_builder()
                                    .link(
                                        crate::i18n::tr_static(app, "Learn more").to_string(),
                                        None,
                                        Some(Box::new(|ctx| {
                                            ctx.dispatch_typed_action(
                                                WasmNUXDialogAction::LearnMore,
                                            )
                                        })),
                                        self.learn_more_mouse_state.clone(),
                                    )
                                    .build()
                                    .finish(),
                            )
                            .left()
                            .finish(),
                        )
                        .finish(),
                )
                .with_bottom_row_child(Self::render_dialog_button(
                    crate::i18n::tr_static(app, "Download"),
                    WasmNUXDialogAction::OpenDownloadDesktopAppLink,
                    &self.download_warp_mouse_state,
                    appearance,
                ))
        } else {
            let object_kind = match web_intent_parser::current_web_intent() {
                Some(WebIntent::DriveObject(_)) => "Warp Drive objects",
                Some(WebIntent::SessionView(_)) => "shared sessions",
                _ => "Warp links",
            };

            Dialog::new(
                format!("Always open {object_kind} on the web?"),
                Some(
                    crate::i18n::tr_static(app, "You can change this at any time in settings.")
                        .to_string(),
                ),
                dialog_styles,
            )
            .with_bottom_row_child(Self::render_dialog_button(
                crate::i18n::tr_static(app, "Yes"),
                WasmNUXDialogAction::SetWebAndClose,
                &self.confirm_mouse_state,
                appearance,
            ))
        };

        dialog.with_close_button(close_button).build().finish()
    }
}

impl TypedActionView for WasmNUXDialog {
    type Action = WasmNUXDialogAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            WasmNUXDialogAction::Close => {
                NativePreferenceSettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings.preference_dialog_dismissed.set_value(true, ctx))
                });
                ctx.emit(WasmNUXDialogEvent::Close);
            }
            WasmNUXDialogAction::SetWebAndClose => {
                if let Err(e) = NativePreferenceSettings::handle(ctx).update(ctx, |setting, ctx| {
                    setting
                        .user_native_redirect_preference
                        .set_value(UserNativePreference::Web, ctx)
                }) {
                    log::error!("Failed to set the open preference to web. {e}");
                };
                ctx.emit(WasmNUXDialogEvent::Close);
            }
            WasmNUXDialogAction::OpenNativeAndClose => {
                // We intentionally do not set the native preference here, in case the user hasn't actually installed Warp.
                // If they have, on subsequent loads, we'll detect that Warp is installed and redirect to the desktop.
                ctx.emit(WasmNUXDialogEvent::Close);

                if let Some(url) = web_intent_parser::parse_web_intent_from_current_url() {
                    // Signals to the react app to open the native app.
                    crate::platform::wasm::emit_event(
                        crate::platform::wasm::WarpEvent::OpenOnNative {
                            url: String::from(url.as_str()),
                        },
                    );
                } else {
                    log::error!("Failed to open in app. Could not determine current url");
                }
            }
            WasmNUXDialogAction::OpenDownloadDesktopAppLink => {
                ctx.open_url("https://app.warp.dev/get_warp");
                self.requested_download = true;
                ctx.notify();
            }
            WasmNUXDialogAction::LearnMore => {
                ctx.open_url("https://www.warp.dev");
            }
        }
    }
}
