use std::default::Default;
use std::fs;
use std::fs::remove_file;

use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{
    Container, CornerRadius, CrossAxisAlignment, Flex, MainAxisSize, MouseStateHandle,
    ParentElement, Radius, SavePosition, Shrinkable, Text,
};
use warpui::fonts::Weight;
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::appearance::Appearance;
use crate::server::telemetry::TelemetryEvent;
use crate::settings::{ThemeSettings, active_theme_kind};
use crate::themes::theme::{ThemeKind, WarpTheme};
use crate::user_config::util::from_yaml;
use crate::{send_telemetry_from_ctx, user_config};

const BUTTON_PADDING: f32 = 12.;
const BUTTON_FONT_SIZE: f32 = 14.;
const BUTTON_BORDER_RADIUS: f32 = 4.;
const BORDER_WIDTH: f32 = 1.;

const MODAL_SUBHEADER: &str = "This will permanently delete the theme.";
const CANCEL_BUTTON_TEXT: &str = "Cancel";
const DELETE_BUTTON_TEXT: &str = "Delete theme";

#[derive(Default)]
struct MouseStateHandles {
    cancel_mouse_state: MouseStateHandle,
    create_mouse_state: MouseStateHandle,
}

pub struct ThemeDeletionBody {
    button_mouse_states: MouseStateHandles,
    theme_kind: Option<ThemeKind>,
}

#[derive(Debug)]
pub enum ThemeDeletionBodyAction {
    Delete,
    Cancel,
}

pub enum ThemeDeletionBodyEvent {
    Close,
    ShowErrorToast { message: String },
    DeleteCurrentTheme,
}

impl Default for ThemeDeletionBody {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemeDeletionBody {
    pub fn new() -> Self {
        Self {
            button_mouse_states: Default::default(),
            theme_kind: None,
        }
    }

    pub fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(ThemeDeletionBodyEvent::Close);
    }

    pub fn delete_theme(&mut self, ctx: &mut ViewContext<Self>) {
        let mut errored = true;
        let dir = user_config::themes_dir();
        // Check if the theme directory exists
        if fs::metadata(&dir).is_ok()
            && let Some(ThemeKind::Custom(custom_theme)) = &self.theme_kind
            && let Ok(theme_from_yaml) = from_yaml::<WarpTheme>(custom_theme.path())
        {
            // If theme has an image
            if let Some(image) = theme_from_yaml.background_image() {
                // Only delete the image if it is in the ./warp/themes directory.
                // We don't want to delete images from other parts of the user's filesystem.
                match image.source() {
                    AssetSource::LocalFile { path, .. } => {
                        let image_path_in_themes_dir = dir.join(path.as_str());
                        let _ = remove_file(image_path_in_themes_dir);
                    }
                    _ => {
                        log::warn!(
                            "Attempted to delete a custom theme image with an unexpected image source"
                        );
                    }
                }
            }

            // Even if image can't be deleted, delete the theme .yaml file
            if remove_file(custom_theme.path()).is_ok() {
                let current_theme = active_theme_kind(ThemeSettings::as_ref(ctx), ctx);
                if matches!(current_theme, ThemeKind::Custom(theme) if &theme == custom_theme) {
                    // Reset theme to Dark if we are deleting the current theme
                    ctx.emit(ThemeDeletionBodyEvent::DeleteCurrentTheme)
                }
                errored = false;
                send_telemetry_from_ctx!(TelemetryEvent::DeleteCustomTheme, ctx);
                self.close(ctx);
                ctx.notify();
            }
        }
        if errored {
            self.send_error_toast("Something went wrong", ctx);
        }
    }

    pub fn set_theme_kind(&mut self, theme_kind: ThemeKind) {
        self.theme_kind = Some(theme_kind);
    }

    fn send_error_toast(&self, message: &str, ctx: &mut ViewContext<Self>) {
        ctx.emit(ThemeDeletionBodyEvent::ShowErrorToast {
            message: message.to_string(),
        });
    }
}

impl Entity for ThemeDeletionBody {
    type Event = ThemeDeletionBodyEvent;
}

impl View for ThemeDeletionBody {
    fn ui_name() -> &'static str {
        "ThemeDeletionBody"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let default_button_styles = UiComponentStyles {
            font_size: Some(BUTTON_FONT_SIZE),
            font_family_id: Some(appearance.ui_font_family()),
            font_color: Some(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().background())
                    .into(),
            ),
            font_weight: Some(Weight::Bold),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(BUTTON_BORDER_RADIUS))),
            border_color: Some(appearance.theme().outline().into()),
            border_width: Some(BORDER_WIDTH),
            padding: Some(Coords::uniform(BUTTON_PADDING)),
            background: Some(appearance.theme().surface_1().into()),
            ..Default::default()
        };

        let cancel_hovered_styles = UiComponentStyles {
            background: Some(appearance.theme().outline().into()),
            border_color: Some(appearance.theme().accent().into()),
            ..default_button_styles
        };

        let disabled_styles = UiComponentStyles {
            background: Some(appearance.theme().surface_3().into()),
            font_color: Some(appearance.theme().disabled_ui_text_color().into()),
            ..default_button_styles
        };

        let create_default_styles = UiComponentStyles {
            background: Some(appearance.theme().accent().into()),
            border_color: Some(appearance.theme().accent().into()),
            font_color: Some(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().accent())
                    .into(),
            ),
            ..default_button_styles
        };

        let create_hovered_styles = UiComponentStyles {
            border_color: Some(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().background())
                    .into(),
            ),
            ..create_default_styles
        };

        let cancel_button = appearance
            .ui_builder()
            .button_with_custom_styles(
                ButtonVariant::Basic,
                self.button_mouse_states.cancel_mouse_state.clone(),
                default_button_styles,
                Some(cancel_hovered_styles),
                Some(cancel_hovered_styles),
                Some(disabled_styles),
            )
            .with_centered_text_label(CANCEL_BUTTON_TEXT.into());

        let create_button = appearance
            .ui_builder()
            .button_with_custom_styles(
                ButtonVariant::Basic,
                self.button_mouse_states.create_mouse_state.clone(),
                create_default_styles,
                Some(create_hovered_styles),
                Some(create_hovered_styles),
                Some(disabled_styles),
            )
            .with_centered_text_label(DELETE_BUTTON_TEXT.into());

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(
                    Text::new_inline(MODAL_SUBHEADER, appearance.ui_font_family(), 14.)
                        .with_color(appearance.theme().active_ui_text_color().into())
                        .finish(),
                )
                .finish(),
            )
            .with_child(
                Container::new(
                    Flex::row()
                        .with_child(
                            Shrinkable::new(
                                0.5,
                                Container::new(
                                    SavePosition::new(
                                        cancel_button
                                            .build()
                                            .with_cursor(Cursor::PointingHand)
                                            .on_click(move |ctx, _, _| {
                                                ctx.dispatch_typed_action(
                                                    ThemeDeletionBodyAction::Cancel,
                                                )
                                            })
                                            .finish(),
                                        "theme_deletion_cancel_button",
                                    )
                                    .finish(),
                                )
                                .with_margin_right(8.)
                                .finish(),
                            )
                            .finish(),
                        )
                        .with_child(
                            Shrinkable::new(
                                0.5,
                                create_button
                                    .build()
                                    .with_cursor(Cursor::PointingHand)
                                    .on_click(move |ctx, _, _| {
                                        ctx.dispatch_typed_action(ThemeDeletionBodyAction::Delete)
                                    })
                                    .finish(),
                            )
                            .finish(),
                        )
                        .with_main_axis_size(MainAxisSize::Max)
                        .finish(),
                )
                .with_margin_top(24.)
                .finish(),
            )
            .finish()
    }
}

impl TypedActionView for ThemeDeletionBody {
    type Action = ThemeDeletionBodyAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ThemeDeletionBodyAction::Cancel => self.close(ctx),
            ThemeDeletionBodyAction::Delete => self.delete_theme(ctx),
        }
    }
}
