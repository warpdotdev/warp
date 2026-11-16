use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Align, Border, ChildAnchor, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, Flex, Highlight, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, ScrollbarWidth,
    Shrinkable, Stack, Text,
};
use warpui::platform::{FilePickerConfiguration, FileType};
use warpui::presenter::ChildView;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use super::modal_body::{ImportModalBody, ImportModalBodyAction, ImportModalBodyEvent};
use crate::appearance::Appearance;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObject, Owner};
use crate::server::ids::SyncId;
use crate::server::sync_queue::SyncQueue;
use crate::themes::theme::WarpTheme;
use crate::workspaces::user_workspaces::UserWorkspaces;

const CLOSE_BUTTON_SIZE: f32 = 24.;
const HEADER_FONT_SIZE: f32 = 16.;
const MODAL_CORNER_RADIUS: f32 = 8.;
pub const BODY_HEIGHT: f32 = 244.;

#[derive(Debug)]
pub enum ImportModalAction {
    Close,
}

pub enum ImportModalEvent {
    OpenTargetWithHashedId(String),
    Close,
}

pub struct ImportModal {
    import_modal: ViewHandle<ImportModalBody>,
    owner: Option<Owner>,
    folder_id: Option<SyncId>,

    clipped_scroll_state: ClippedScrollStateHandle,

    close_button_mouse_state: MouseStateHandle,
    footer_button_mouse_state: MouseStateHandle,
}

impl ImportModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let import_modal_body = ctx.add_typed_action_view(ImportModalBody::new);

        ctx.subscribe_to_view(&import_modal_body, move |me, _, event, ctx| {
            me.handle_import_body_event(event, ctx);
        });

        Self {
            import_modal: import_modal_body,
            owner: None,
            folder_id: None,
            clipped_scroll_state: Default::default(),
            close_button_mouse_state: Default::default(),
            footer_button_mouse_state: Default::default(),
        }
    }

    pub fn open_with_target(
        &mut self,
        owner: Owner,
        initial_folder_id: Option<SyncId>,
        ctx: &mut ViewContext<Self>,
    ) {
        // TODO: This should take an owner OR folder.
        self.owner = Some(owner);
        self.folder_id = initial_folder_id;

        self.import_modal.update(ctx, |import_modal, _ctx| {
            import_modal.set_new_target(owner, initial_folder_id);
        });

        ctx.notify();
    }

    pub fn open_file_picker(&mut self, ctx: &mut ViewContext<Self>) {
        let window_id = ctx.window_id();
        let import_body_id = self.import_modal.id();

        let sync_queue_is_dequeueing = SyncQueue::as_ref(ctx).is_dequeueing();

        let allowed_file_types = vec![FileType::Yaml, FileType::Markdown];

        let mut file_picker_config = FilePickerConfiguration::new()
            .allow_multi_select()
            .set_allowed_file_types(allowed_file_types);

        // Files under a folder could only be uploaded when the folder is created on the server.
        // When sync queue is not dequeueing, disable folder upload in the import modal.
        if sync_queue_is_dequeueing {
            file_picker_config = file_picker_config.allow_folder();
        }

        ctx.open_file_picker(
            move |result, ctx| match result {
                Ok(paths) if !paths.is_empty() => {
                    ctx.dispatch_typed_action_for_view(
                        window_id,
                        import_body_id,
                        &ImportModalBodyAction::PathsSelected(paths),
                    );
                }
                Ok(_) => {
                    ctx.dispatch_typed_action_for_view(
                        window_id,
                        import_body_id,
                        &ImportModalBodyAction::FilePickerCancelled,
                    );
                }
                Err(err) => {
                    ctx.dispatch_typed_action_for_view(
                        window_id,
                        import_body_id,
                        &ImportModalBodyAction::FilePickerError(err),
                    );
                }
            },
            file_picker_config,
        );

        ctx.notify();
    }

    fn handle_import_body_event(
        &mut self,
        event: &ImportModalBodyEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ImportModalBodyEvent::OpenFilePicker => self.open_file_picker(ctx),
            ImportModalBodyEvent::OpenTargetWithHashedId(hashed_id) => {
                ctx.emit(ImportModalEvent::OpenTargetWithHashedId(hashed_id.clone()))
            }
            ImportModalBodyEvent::UploadCompleted
            | ImportModalBodyEvent::AllFileSavedLocally
            | ImportModalBodyEvent::UploadSelected => ctx.notify(),
        }
    }

    fn breadcrumb(&self, app: &AppContext) -> (String, Vec<usize>) {
        let (text, highlight_start) = match self
            .folder_id
            .as_ref()
            .and_then(|folder_id| CloudModel::as_ref(app).get_folder(folder_id))
        {
            Some(folder) => {
                let breadcrumbs = folder.breadcrumbs(app);
                (
                    format!("{} / {}", breadcrumbs, folder.display_name()),
                    breadcrumbs.chars().count() + 3,
                )
            }
            None => (
                // Convert to a Space for display, in case we're importing into a shared folder.
                self.owner
                    .map(|owner| {
                        UserWorkspaces::as_ref(app)
                            .owner_to_space(owner, app)
                            .name(app)
                    })
                    .unwrap_or_default(),
                0,
            ),
        };

        // The unit of highlight index is character index not byte index.
        let highlight_range = (highlight_start..text.chars().count()).collect();
        (text, highlight_range)
    }

    fn render_breadcrumbs(
        &self,
        appearance: &Appearance,
        theme: &WarpTheme,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let (breadcrumb_text, highlight_indices) = self.breadcrumb(app);

        Container::new(
            appearance
                .ui_builder()
                .span(breadcrumb_text)
                .with_highlights(
                    highlight_indices,
                    Highlight::new().with_foreground_color(
                        theme.main_text_color(theme.surface_2()).into_solid(),
                    ),
                )
                .with_style(UiComponentStyles {
                    font_color: Some(theme.sub_text_color(theme.surface_2()).into_solid()),
                    ..Default::default()
                })
                .build()
                .finish(),
        )
        .with_margin_top(6.)
        .finish()
    }

    fn render_close_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .close_button(CLOSE_BUTTON_SIZE, self.close_button_mouse_state.clone())
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(ImportModalAction::Close))
            .finish()
    }

    fn render_header(
        &self,
        appearance: &Appearance,
        theme: &WarpTheme,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let top_row = Flex::row()
            .with_child(
                Shrinkable::new(
                    1.0,
                    Align::new(
                        Text::new_inline("Import", appearance.ui_font_family(), HEADER_FONT_SIZE)
                            .with_color(appearance.theme().active_ui_text_color().into())
                            .finish(),
                    )
                    .left()
                    .finish(),
                )
                .finish(),
            )
            .with_child(self.render_close_button(appearance))
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish();

        let header = Flex::column()
            .with_child(top_row)
            .with_child(self.render_breadcrumbs(appearance, theme, app))
            .finish();

        Container::new(header)
            .with_corner_radius(CornerRadius::with_top(Radius::Pixels(MODAL_CORNER_RADIUS)))
            .with_padding_left(24.)
            .with_padding_top(16.)
            .with_padding_right(16.)
            .with_padding_bottom(16.)
            .with_border(Border::bottom(1.).with_border_fill(theme.outline()))
            .finish()
    }

    fn render_body(&self, theme: &WarpTheme) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                ClippedScrollable::vertical(
                    self.clipped_scroll_state.clone(),
                    ChildView::new(&self.import_modal).finish(),
                    ScrollbarWidth::Auto,
                    theme.disabled_text_color(theme.surface_2()).into(),
                    theme.main_text_color(theme.surface_2()).into(),
                    theme.surface_2().into(),
                )
                .finish(),
            )
            .with_height(BODY_HEIGHT)
            .finish(),
        )
        .with_uniform_padding(5.)
        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(
            MODAL_CORNER_RADIUS,
        )))
        .with_background(theme.surface_2())
        .finish()
    }

    fn render_footer(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let button_text = if !self.import_modal.as_ref(app).upload_in_progress(app) {
            "Close".to_string()
        } else {
            "Cancel".to_string()
        };

        Container::new(
            Align::new(
                appearance
                    .ui_builder()
                    .button(
                        ButtonVariant::Outlined,
                        self.footer_button_mouse_state.clone(),
                    )
                    .with_centered_text_label(button_text.to_string())
                    .with_style(UiComponentStyles {
                        width: Some(150.),
                        height: Some(40.),
                        font_size: Some(14.),
                        ..Default::default()
                    })
                    .build()
                    .on_click(|ctx, _, _| ctx.dispatch_typed_action(ImportModalAction::Close))
                    .finish(),
            )
            .right()
            .finish(),
        )
        .with_uniform_padding(16.)
        .finish()
    }
}

impl Entity for ImportModal {
    type Event = ImportModalEvent;
}

impl View for ImportModal {
    fn ui_name() -> &'static str {
        "ImportModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut contents = Flex::column()
            .with_child(self.render_header(appearance, theme, app))
            .with_child(self.render_body(theme));

        if !self.import_modal.as_ref(app).before_upload() {
            contents.add_child(self.render_footer(appearance, app));
        }

        let modal = ConstrainedBox::new(
            Container::new(contents.finish())
                .with_background(theme.surface_2())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(MODAL_CORNER_RADIUS)))
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .with_margin_top(35.)
                .finish(),
        )
        .with_width(500.)
        .with_height(300.)
        .finish();

        // Stack needed so that modal can get bounds information,
        // specifically to ensure no overlap with the window's traffic lights
        let mut stack = Stack::new();
        stack.add_positioned_child(
            modal,
            OffsetPositioning::offset_from_parent(
                vec2f(0., 0.),
                ParentOffsetBounds::WindowByPosition,
                ParentAnchor::Center,
                ChildAnchor::Center,
            ),
        );

        Container::new(Align::new(stack.finish()).finish())
            .with_background_color(Fill::blur().into())
            .finish()
    }
}

impl TypedActionView for ImportModal {
    type Action = ImportModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ImportModalAction::Close => {
                self.import_modal.update(ctx, |import_modal_body, ctx| {
                    import_modal_body.reset(ctx);
                });

                ctx.emit(ImportModalEvent::Close);
            }
        }
    }
}
