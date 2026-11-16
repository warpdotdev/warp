use std::default::Default;
use std::fmt;
use std::path::PathBuf;
#[cfg(feature = "local_fs")]
use std::{fs::copy, io::Write};

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
#[cfg(feature = "local_fs")]
use warp_core::ui::theme::WarpTheme;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DispatchEventResult,
    EventHandler, Fill, Flex, Icon, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement, Radius, Rect, SavePosition, Shrinkable, Text,
};
use warpui::fonts::Weight;
use warpui::platform::Cursor;
use warpui::ui_components::button::{ButtonVariant, TextAndIcon, TextAndIconAlignment};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::text_input::TextInput;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::{Appearance, AppearanceManager};
use crate::editor::{EditorView, Event as EditorEvent};
use crate::themes::theme::{InMemoryThemeOptions, ThemeKind};
use crate::user_config;
#[cfg(feature = "local_fs")]
use crate::{
    send_telemetry_from_ctx, server::telemetry::TelemetryEvent, themes::theme::CustomTheme,
};

const BUTTON_PADDING: f32 = 12.;
const BUTTON_FONT_SIZE: f32 = 14.;
const BUTTON_BORDER_RADIUS: f32 = 4.;
const BORDER_WIDTH: f32 = 1.;

const MODAL_SUBHEADER: &str =
    "Automatically generate a theme based on extracted colors from an image (.png, .jpg).";
const IMAGE_PICKER_BUTTON_PRE_SELECT_TEXT: &str = "Select an image";
const IMAGE_PICKER_BUTTON_SELECTING_TEXT: &str = "Selecting image...";
const IMAGE_PICKER_BUTTON_POST_SELECT_TEXT: &str = "Select a new image";
const CANCEL_BUTTON_TEXT: &str = "Cancel";
const CREATE_BUTTON_TEXT: &str = "Create theme";

#[derive(Default)]
struct MouseStateHandles {
    image_picker_mouse_state: MouseStateHandle,
    cancel_mouse_state: MouseStateHandle,
    create_mouse_state: MouseStateHandle,
}

pub struct ThemeCreatorBody {
    button_mouse_states: MouseStateHandles,
    editor: ViewHandle<EditorView>,
    theme_options: Option<InMemoryThemeOptions>,
    image_state: ThemeCreatorImageState,
}

#[derive(Debug)]
pub enum ThemeCreatorBodyAction {
    Create,
    OpenFilePicker,
    HandleImageSelected(PathBuf),
    SetBackgroundColor(usize),
    Cancel,
    FilePickerCancelled,
}

pub enum ThemeCreatorBodyEvent {
    Close,
    OpenFilePicker,
    SetCustomTheme { theme: ThemeKind },
    ShowErrorToast { message: String },
}

#[derive(Debug)]
pub enum ThemeCreatorImageState {
    Empty,
    Uploading,
    Uploaded,
}

impl fmt::Display for ThemeCreatorImageState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ThemeCreatorImageState::Empty => write!(f, "{IMAGE_PICKER_BUTTON_PRE_SELECT_TEXT}"),
            ThemeCreatorImageState::Uploading => {
                write!(f, "{IMAGE_PICKER_BUTTON_SELECTING_TEXT}")
            }
            ThemeCreatorImageState::Uploaded => {
                write!(f, "{IMAGE_PICKER_BUTTON_POST_SELECT_TEXT}")
            }
        }
    }
}

impl ThemeCreatorBody {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let editor = Self::editor(ctx);

        Self {
            button_mouse_states: Default::default(),
            editor,
            theme_options: None,
            image_state: ThemeCreatorImageState::Empty,
        }
    }

    fn editor(ctx: &mut ViewContext<Self>) -> ViewHandle<EditorView> {
        let editor = { ctx.add_typed_action_view(|ctx| EditorView::new(Default::default(), ctx)) };
        ctx.subscribe_to_view(&editor, move |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        editor
    }

    pub fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        if let EditorEvent::Edited(_) = event
            && let Some(theme_options) = &mut self.theme_options
        {
            self.editor.update(ctx, |editor, ctx| {
                theme_options.set_name(editor.buffer_text(ctx));

                let theme_kind = ThemeKind::InMemory(theme_options.clone());
                AppearanceManager::handle(ctx).update(ctx, |appearance_manager, ctx| {
                    appearance_manager.set_transient_theme(theme_kind, ctx);
                });
            });
        }
        ctx.notify();
    }

    pub fn close(&mut self, ctx: &mut ViewContext<Self>) {
        self.theme_options = None;
        self.image_state = ThemeCreatorImageState::Empty;

        ctx.emit(ThemeCreatorBodyEvent::Close);
    }

    pub fn cancel(&mut self, ctx: &mut ViewContext<Self>) {
        AppearanceManager::handle(ctx).update(ctx, |appearance_manager, ctx| {
            appearance_manager.clear_transient_theme(ctx);
        });
        self.close(ctx);
    }

    pub fn open_file_picker(&mut self, ctx: &mut ViewContext<Self>) {
        self.image_state = ThemeCreatorImageState::Uploading;
        ctx.notify();
        ctx.emit(ThemeCreatorBodyEvent::OpenFilePicker);
    }

    pub fn handle_file_picker_cancelled(&mut self, ctx: &mut ViewContext<Self>) {
        self.image_state = if self.theme_options.is_some() {
            ThemeCreatorImageState::Uploaded
        } else {
            ThemeCreatorImageState::Empty
        };
        ctx.notify();
    }

    #[cfg_attr(not(feature = "local_fs"), allow(unused))]
    pub fn create_theme(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(theme_options) = self.theme_options.as_mut() {
            let theme_name = theme_options.name();
            let theme_yaml_file_name = format!("{theme_name}.yaml");
            let original_theme_image_path = theme_options.path();
            let original_theme_image_path_clone = original_theme_image_path.clone();

            let image_extension = original_theme_image_path
                .extension()
                .and_then(|extension| extension.to_str());

            let Some(image_extension) = image_extension else {
                self.send_error_toast(
                    "Failed to process selected image. Please try again with a different image."
                        .to_string(),
                    ctx,
                );
                return;
            };

            let dir = user_config::themes_dir();

            theme_options.set_path(dir.join(format!("{theme_name}.{image_extension}")));
            let mut errored = true;
            #[cfg(feature = "local_fs")]
            {
                ThemeCreatorBody::write_theme(
                    &theme_options.theme(),
                    dir,
                    theme_yaml_file_name,
                    Some((
                        original_theme_image_path_clone,
                        theme_name.clone(),
                        image_extension,
                    )),
                    |path| {
                        send_telemetry_from_ctx!(TelemetryEvent::CreateCustomTheme, ctx);
                        ctx.emit(ThemeCreatorBodyEvent::SetCustomTheme {
                            theme: ThemeKind::Custom(CustomTheme::new(theme_name, path)),
                        });
                        errored = false;
                        self.close(ctx);
                        ctx.notify();
                    },
                );
            }
            #[cfg(not(feature = "local_fs"))]
            log::warn!("Tried to save theme without a local filesystem.");
            if errored {
                self.send_error_toast("Something went wrong".to_string(), ctx);
            }
        }
    }

    /// Writes a theme to the filesystem. Calls the success callback if successful.
    /// Note: the image option should be (original_theme_image_path, theme_name, image_extension).
    #[cfg(feature = "local_fs")]
    pub fn write_theme<T>(
        theme: &WarpTheme,
        dir: PathBuf,
        theme_yaml_file_name: String,
        image_option: Option<(PathBuf, String, &str)>,
        success_callback: impl FnOnce(PathBuf) -> T,
    ) -> Option<T> {
        if let Ok(theme_yaml) = serde_yaml::to_string(theme) {
            let path = dir.join(theme_yaml_file_name);
            if let Ok(mut file) = crate::util::file::create_file(&path)
                && write!(file, "{theme_yaml}").is_ok()
            {
                match image_option {
                    Some((image_path, theme_name, image_extension)) => {
                        if copy(
                            image_path.clone(),
                            dir.join(format!("{theme_name}.{image_extension}")),
                        )
                        .is_ok()
                        {
                            return Some((success_callback)(path));
                        }
                    }
                    None => return Some((success_callback)(path)),
                }
            }
        }
        None
    }

    pub fn set_theme_from_image_path(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>) {
        let file_stem_string = path
            .clone()
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();

        ctx.spawn(
            InMemoryThemeOptions::new(file_stem_string.clone(), path.clone()),
            move |theme_creator_body, theme_options, ctx| {
                match theme_options {
                    Ok(theme_options) => {
                        AppearanceManager::handle(ctx).update(ctx, |appearance_manager, ctx| {
                            appearance_manager.clear_transient_theme(ctx);
                        });

                        theme_creator_body.theme_options = Some(theme_options);
                        theme_creator_body.editor.update(ctx, |editor, ctx| {
                            editor.set_buffer_text(&file_stem_string, ctx);
                        });
                        theme_creator_body.image_state = ThemeCreatorImageState::Uploaded;
                    },
                    Err(e) => {
                        theme_creator_body.send_error_toast(
                            format!("Failed to process selected image due to error: {e}. Please try again with a different image."),
                            ctx,
                        );
                    }
                }
            },
        );
    }

    pub fn set_background_color(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if let Some(theme_options) = &mut self.theme_options {
            theme_options.set_chosen_bg_color_index(index);

            let theme_kind = ThemeKind::InMemory(theme_options.clone());
            AppearanceManager::handle(ctx).update(ctx, |appearance_manager, ctx| {
                appearance_manager.set_transient_theme(theme_kind, ctx);
            });
        }

        ctx.notify();
    }

    fn send_error_toast(&self, message: String, ctx: &mut ViewContext<Self>) {
        ctx.emit(ThemeCreatorBodyEvent::ShowErrorToast { message });
    }
}

impl Entity for ThemeCreatorBody {
    type Event = ThemeCreatorBodyEvent;
}

impl View for ThemeCreatorBody {
    fn ui_name() -> &'static str {
        "ThemeCreatorBody"
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

        let image_picker_button_background = if self.theme_options.is_some() {
            appearance.theme().surface_1()
        } else {
            appearance.theme().accent()
        };

        let image_picker_button_hovered_styles = if self.theme_options.is_some() {
            cancel_hovered_styles
        } else {
            UiComponentStyles {
                border_color: Some(appearance.theme().foreground().into()),
                background: Some(image_picker_button_background.into()),
                font_color: Some(
                    appearance
                        .theme()
                        .main_text_color(image_picker_button_background)
                        .into(),
                ),
                ..default_button_styles
            }
        };

        let image_picker_button = appearance.ui_builder().button_with_custom_styles(
            ButtonVariant::Accent,
            self.button_mouse_states.image_picker_mouse_state.clone(),
            UiComponentStyles {
                background: Some(image_picker_button_background.into()),
                font_color: Some(
                    appearance
                        .theme()
                        .main_text_color(image_picker_button_background)
                        .into(),
                ),
                ..default_button_styles
            },
            Some(image_picker_button_hovered_styles),
            Some(image_picker_button_hovered_styles),
            Some(disabled_styles),
        );

        let cancel_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Secondary,
                self.button_mouse_states.cancel_mouse_state.clone(),
            )
            .with_style(UiComponentStyles {
                font_size: Some(BUTTON_FONT_SIZE),
                font_weight: Some(Weight::Bold),
                padding: Some(Coords::uniform(BUTTON_PADDING)),
                ..Default::default()
            })
            .with_centered_text_label(CANCEL_BUTTON_TEXT.into());

        let mut create_button = appearance
            .ui_builder()
            .button_with_custom_styles(
                ButtonVariant::Basic,
                self.button_mouse_states.create_mouse_state.clone(),
                create_default_styles,
                Some(create_hovered_styles),
                Some(create_hovered_styles),
                Some(disabled_styles),
            )
            .with_centered_text_label(CREATE_BUTTON_TEXT.into());

        let mut flex: Flex = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(
                    Text::new_inline(MODAL_SUBHEADER, appearance.ui_font_family(), 14.)
                        .with_color(appearance.theme().active_ui_text_color().into())
                        .finish(),
                )
                .finish(),
            );

        if let Some(theme_options) = &self.theme_options {
            flex.add_child(
                Container::new(
                    Text::new_inline("Theme name", appearance.ui_font_family(), 14.)
                        .with_color(appearance.theme().active_ui_text_color().into())
                        .finish(),
                )
                .with_margin_top(12.)
                .finish(),
            );
            flex.add_child(
                Container::new(
                    TextInput::new(
                        self.editor.clone(),
                        UiComponentStyles::default()
                            .set_border_color(appearance.theme().outline().into())
                            .set_font_family_id(appearance.header_font_family())
                            .set_font_size(14.)
                            .set_background(Fill::None)
                            .set_border_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                            .set_padding(Coords::uniform(20.).top(10.).bottom(10.))
                            .set_border_width(2.),
                    )
                    .build()
                    .finish(),
                )
                .with_margin_top(8.)
                .finish(),
            );

            flex.add_child(
                Container::new(
                    Text::new_inline("Background color", appearance.ui_font_family(), 14.)
                        .with_color(appearance.theme().active_ui_text_color().into())
                        .finish(),
                )
                .with_margin_top(24.)
                .finish(),
            );

            let mut color_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

            for (bg_color_index, bg_color) in
                theme_options.possible_bg_colors().into_iter().enumerate()
            {
                // Add corner radius if the rect is the first or last one
                let corner_radius = if bg_color_index == 0 {
                    CornerRadius::with_left(Radius::Pixels(8.))
                } else if bg_color_index == 4 {
                    CornerRadius::with_right(Radius::Pixels(8.))
                } else {
                    CornerRadius::with_all(Radius::Pixels(0.))
                };

                // Add a border around the chosen background color
                let border_width = if bg_color_index == theme_options.chosen_bg_color_index() {
                    3.
                } else {
                    0.
                };

                color_row.add_child(
                    Flex::row()
                        .with_child(
                            EventHandler::new(
                                ConstrainedBox::new(
                                    Rect::new()
                                        .with_background_color(bg_color)
                                        .with_corner_radius(corner_radius)
                                        .with_border(
                                            Border::all(border_width).with_border_fill(
                                                appearance.theme().main_text_color(
                                                    appearance.theme().background(),
                                                ),
                                            ),
                                        )
                                        .finish(),
                                )
                                .with_width(110.)
                                .with_height(40.)
                                .finish(),
                            )
                            .on_left_mouse_down(move |ctx, _, _| {
                                ctx.dispatch_typed_action(
                                    ThemeCreatorBodyAction::SetBackgroundColor(bg_color_index),
                                );
                                DispatchEventResult::StopPropagation
                            })
                            .finish(),
                        )
                        .finish(),
                );
            }

            flex.add_child(
                Container::new(color_row.finish())
                    .with_margin_top(8.)
                    .finish(),
            );
        } else {
            create_button = create_button.disabled();
        }

        flex.add_child(
            Container::new(
                if let ThemeCreatorImageState::Uploading = self.image_state {
                    image_picker_button
                        .with_centered_text_label(self.image_state.to_string())
                        .disabled()
                        .build()
                        .finish()
                } else {
                    image_picker_button
                        .with_text_and_icon_label(
                            TextAndIcon::new(
                                TextAndIconAlignment::TextFirst,
                                self.image_state.to_string(),
                                Icon::new("bundled/svg/upload-01.svg", ColorU::white()),
                                MainAxisSize::Max,
                                MainAxisAlignment::Center,
                                vec2f(16., 16.),
                            )
                            .with_inner_padding(4.),
                        )
                        .build()
                        .with_cursor(Cursor::PointingHand)
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(ThemeCreatorBodyAction::OpenFilePicker)
                        })
                        .finish()
                },
            )
            .with_margin_top(24.)
            .finish(),
        );

        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(
                    flex.with_child(
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
                                                            ThemeCreatorBodyAction::Cancel,
                                                        )
                                                    })
                                                    .finish(),
                                                "theme_creator_cancel_button",
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
                                                ctx.dispatch_typed_action(
                                                    ThemeCreatorBodyAction::Create,
                                                )
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
                    .finish(),
                )
                .finish(),
            )
            .finish()
    }
}

impl TypedActionView for ThemeCreatorBody {
    type Action = ThemeCreatorBodyAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ThemeCreatorBodyAction::Cancel => self.cancel(ctx),
            ThemeCreatorBodyAction::OpenFilePicker => self.open_file_picker(ctx),
            ThemeCreatorBodyAction::SetBackgroundColor(index) => {
                self.set_background_color(*index, ctx)
            }
            ThemeCreatorBodyAction::Create => self.create_theme(ctx),
            ThemeCreatorBodyAction::HandleImageSelected(path) => {
                self.set_theme_from_image_path(path.clone(), ctx);
                ctx.notify();
            }
            ThemeCreatorBodyAction::FilePickerCancelled => self.handle_file_picker_cancelled(ctx),
        }
    }
}
