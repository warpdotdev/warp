use futures_util::stream::AbortHandle;
use pathfinder_geometry::vector::vec2f;
use std::path::PathBuf;
use warpui::{
    elements::{
        Align, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex,
        MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius,
    },
    platform::{file_picker::FilePickerError, Cursor},
    ui_components::{
        button::{ButtonVariant, TextAndIcon, TextAndIconAlignment},
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::{
    appearance::Appearance,
    cloud_object::Owner,
    server::{
        ids::{ClientId, SyncId},
        sync_queue::SyncQueue,
    },
    ui_components::icons::Icon,
    view_components::DismissibleToast,
    workspace::ToastStack,
};

use super::{
    modal::BODY_HEIGHT,
    nodes::{
        expand_dirs, parse_file, FileContent, FileId, FileUploadState, FolderId, UploadResult,
    },
    queue::{ImportQueue, ImportQueueArgs, ImportQueueEvent, ParentId, RequestContent},
};

const FILE_PICKER_BUTTON_WIDTH: f32 = 250.;
const BUTTON_FONT_SIZE: f32 = 14.;
const BUTTON_BORDER_RADIUS: f32 = 4.;
pub(super) const IMPORT_FONT_SIZE: f32 = 14.;
pub(super) const INDENT_MARGIN: f32 = 22.;
pub(super) const BASE_INDENT: f32 = 30.;

const FILE_TYPE_DOCS_URL: &str =
    "https://docs.warp.dev/knowledge-and-collaboration/warp-drive#import-and-export";
const SUPPORTED_FILE_TYPE_TEXT: &str = "md, yaml, yml";

#[cfg(test)]
#[path = "import_tests.rs"]
mod import_tests;

/// Current state of the import modal.
///
/// The entire import flow goes as follows:
/// 1. Modal prompts user with native file picker
/// 2. User selects paths from the file picker
/// 3. The modal expands paths into a tree of folders and matching files
/// 4. Insert all folders we need to upload into the import queue
/// 5. Iteratively parse out all file contents from the file paths
///    - If we fail to parse the file, mark the file node as errored
///    - If we successfully parsed the file, push the file content to the import queue
enum ImportState {
    // Before the user opens the file picker.
    Upload,
    // We are waiting for users to select paths from the file picker.
    Loading,
    // Users have selected paths from the file picker.
    PathLoaded,
    PathExpanded(FileUploadState),
}

#[derive(Debug)]
pub enum ImportModalBodyAction {
    RetryFile(FileId),
    OpenFilePicker,
    FilePickerCancelled,
    PathsSelected(Vec<String>),
    FilePickerError(FilePickerError),
    ClickedToOpenTarget(String),
}

pub enum ImportModalBodyEvent {
    OpenFilePicker,
    UploadCompleted,
    AllFileSavedLocally,
    UploadSelected,
    OpenTargetWithHashedId(String),
}

pub struct ImportModalBody {
    state: ImportState,
    in_progress_handle: Option<AbortHandle>,
    // Queue to handle requests to upload objects to warp drive.
    // All updates should go through the queue rather than calling
    // UpdateManager directly.
    import_queue: ModelHandle<ImportQueue>,
    owner: Option<Owner>,
    initial_folder_id: Option<SyncId>,

    file_picker_mouse_state: MouseStateHandle,
    link_mouse_state: MouseStateHandle,
}

impl ImportModalBody {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let import_queue = ctx.add_model(ImportQueue::new);
        ctx.subscribe_to_model(&import_queue, |me, _, event, ctx| {
            me.handle_import_queue_event(event, ctx)
        });

        Self {
            state: ImportState::Upload,
            owner: None,
            initial_folder_id: None,
            import_queue,
            file_picker_mouse_state: Default::default(),
            link_mouse_state: Default::default(),
            in_progress_handle: None,
        }
    }

    fn handle_import_queue_event(&mut self, event: &ImportQueueEvent, ctx: &mut ViewContext<Self>) {
        // Only handle event when path is expanded.
        if let ImportState::PathExpanded(state) = &mut self.state {
            match event {
                ImportQueueEvent::FileCompleted { file_id, server_id } => {
                    let result = match server_id {
                        Some(id) => UploadResult::Success(id.clone()),
                        None => UploadResult::Error("Failed to upload file to server".to_string()),
                    };

                    // Update the upstream folder status with the upload success state.
                    if state.update_tree_with_file_upload_result(result, *file_id) {
                        ctx.notify();
                    }
                }
                ImportQueueEvent::FolderCompleted {
                    folder_id,
                    server_id,
                } => {
                    let result = match server_id {
                        Some(id) => UploadResult::Success(id.clone()),
                        None => {
                            UploadResult::Error("Failed to upload folder to server".to_string())
                        }
                    };

                    state.mark_folder_synced(result, *folder_id);
                    ctx.notify();
                }
                ImportQueueEvent::FileSavedLocally(file_id) => {
                    let file_node = state
                        .file_id_to_node
                        .get_mut(file_id)
                        .expect("File node should exist");

                    file_node.saved_locally();
                    ctx.notify();
                }
            }

            let sync_queue_dequeueing = SyncQueue::as_ref(ctx).is_dequeueing();

            if !sync_queue_dequeueing && state.all_files_saved_locally() {
                ctx.emit(ImportModalBodyEvent::AllFileSavedLocally);
            } else if state.is_complete() {
                ctx.emit(ImportModalBodyEvent::UploadCompleted);
            }
        }
    }

    pub fn set_new_target(&mut self, owner: Owner, initial_folder_id: Option<SyncId>) {
        // TODO: this should take an owner OR folder.
        self.owner = Some(owner);
        self.initial_folder_id = initial_folder_id;
    }

    // Push a new update to the import queue to sync with the server.
    fn push_new_update(&mut self, arg: ImportQueueArgs, ctx: &mut ViewContext<Self>) {
        self.import_queue
            .update(ctx, |queue, ctx| queue.enqueue(arg, ctx));
    }

    // Whether there is an active upload in progress (If all uploads are completed,
    // we don't consider the import modal upload to be in progress).
    pub fn upload_in_progress(&self, app: &AppContext) -> bool {
        let sync_queue_dequeueing = SyncQueue::as_ref(app).is_dequeueing();

        match &self.state {
            ImportState::Upload => false,
            ImportState::PathExpanded(state)
                if !sync_queue_dequeueing && state.all_files_saved_locally() =>
            {
                false
            }
            ImportState::PathExpanded(state) if state.is_complete() => false,
            _ => true,
        }
    }

    pub fn before_upload(&self) -> bool {
        matches!(&self.state, ImportState::Upload | ImportState::Loading)
    }

    fn parent_id_for_upload(&self, parent_folder_cloud_id: Option<ClientId>) -> ParentId {
        match parent_folder_cloud_id {
            Some(id) => ParentId::FolderToUpload(id),
            None => ParentId::InitialFolder(self.initial_folder_id),
        }
    }

    /// Populate folder nodes with actual cloud objects.
    pub(super) fn populate_folder_cloud_object(
        &mut self,
        state: &mut FileUploadState,
        ctx: &mut ViewContext<Self>,
    ) {
        // Start with the first non-root node folder.
        let mut id = FolderId::root_id();
        id += 1;

        let Some(owner) = self.owner else {
            log::warn!("Import modal opened without owner");
            return;
        };

        // Push all folders to the queue in order. This is more time efficient when dequeueing
        // from the queue.
        while let Some(node) = state.folder_id_to_node.get(&id) {
            let parent_id = node.parent_id();

            // If a node's parent is the root / dummy node, consider it to have no initial folder.
            let parent_folder_cloud_id = state.folder_cloud_id(parent_id);
            self.push_new_update(
                ImportQueueArgs {
                    owner,
                    parent_id: self.parent_id_for_upload(parent_folder_cloud_id),
                    content: RequestContent::Folder {
                        name: node.name(),
                        client_id: node.cloud_id(),
                        folder_id: id,
                    },
                },
                ctx,
            );

            id += 1;
        }
    }

    /// Parse the next file that has not been uploaded. We determine the next file by iteratively
    /// by adding 1 to the previous uploaded file id.
    fn parse_next_file(
        &mut self,
        file_id: FileId,
        continue_parsing_after_completion: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if let ImportState::PathExpanded(state) = &self.state {
            let Some(node) = state.file_id_to_node.get(&file_id) else {
                return;
            };

            let path = node.full_path();
            let file_type = node.file_type();

            let handle = ctx.spawn(parse_file(path, file_type), move |view, response, ctx| {
                let metadata = match &mut view.state {
                    ImportState::PathExpanded(state) => {
                        // If there is an error with the file, update the file state with the error and notify
                        // upstream folders.
                        if let Err(e) = &response {
                            state.update_tree_with_file_upload_result(
                                UploadResult::Error(e.to_string()),
                                file_id,
                            );
                        }

                        state.file_name_and_parent_cloud_id(file_id)
                    }
                    _ => None,
                };

                let Some((file_name, parent_cloud_id)) = metadata else {
                    return;
                };

                let Some(owner) = view.owner else {
                    log::warn!("Import modal opened without owner");
                    return;
                };

                match response {
                    Ok(FileContent::Notebook(data)) => {
                        let client_id = ClientId::default();

                        view.push_new_update(
                            ImportQueueArgs {
                                owner,
                                parent_id: view.parent_id_for_upload(parent_cloud_id),
                                content: RequestContent::Notebook {
                                    title: file_name,
                                    data,
                                    client_id,
                                    file_id,
                                },
                            },
                            ctx,
                        )
                    }
                    Ok(FileContent::Workflow {
                        workflows,
                        workflow_enums,
                    }) => view.push_new_update(
                        ImportQueueArgs {
                            owner,
                            parent_id: view.parent_id_for_upload(parent_cloud_id),
                            content: RequestContent::Workflow {
                                workflows: workflows
                                    .into_iter()
                                    .map(|workflow| (workflow, ClientId::new()))
                                    .collect(),
                                workflow_enums,
                                file_id,
                            },
                        },
                        ctx,
                    ),
                    _ => (),
                }

                let next_file_id = file_id + 1;
                match &mut view.state {
                    ImportState::PathExpanded(state) => {
                        if continue_parsing_after_completion
                            && state.file_id_to_node.contains_key(&next_file_id)
                        {
                            view.parse_next_file(
                                next_file_id,
                                continue_parsing_after_completion,
                                ctx,
                            );
                        } else {
                            // If we reach the end of the parsable files or should not continue parsing, reset the abort handle.
                            view.in_progress_handle = None;
                        }
                    }
                    _ => panic!("Validated state is path expanded already"),
                };

                ctx.notify();
            });

            self.in_progress_handle = Some(handle.abort_handle());
        } else {
            log::error!("State should be path expanded when parsing files");
        };
    }

    pub fn reset(&mut self, ctx: &mut ViewContext<Self>) {
        self.state = ImportState::Upload;
        if let Some(handle) = self.in_progress_handle.take() {
            handle.abort();
        }
        ctx.notify();
    }

    fn render_upload_state(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let is_loading = matches!(self.state, ImportState::PathLoaded | ImportState::Loading);
        let base_button = appearance
            .ui_builder()
            .button(ButtonVariant::Accent, self.file_picker_mouse_state.clone())
            .with_style(UiComponentStyles {
                font_size: Some(BUTTON_FONT_SIZE),
                font_family_id: Some(appearance.ui_font_family()),
                padding: Some(Coords {
                    top: 10.,
                    bottom: 10.,
                    left: 70.,
                    right: 70.,
                }),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(BUTTON_BORDER_RADIUS))),
                border_color: Some(appearance.theme().outline().into()),
                width: Some(FILE_PICKER_BUTTON_WIDTH),
                ..Default::default()
            });

        let file_picker_button = if is_loading {
            base_button
                .with_centered_text_label(crate::i18n::tr_static(app, "Preparing...").to_string())
                .disabled()
        } else {
            base_button.with_text_and_icon_label(
                TextAndIcon::new(
                    TextAndIconAlignment::TextFirst,
                    crate::i18n::tr_static(app, "Choose files...").to_string(),
                    Icon::Import.to_warpui_icon(
                        appearance
                            .theme()
                            .main_text_color(appearance.theme().accent_button_color()),
                    ),
                    MainAxisSize::Max,
                    MainAxisAlignment::Center,
                    vec2f(16., 16.),
                )
                .with_inner_padding(4.),
            )
        };

        let file_picker_element = file_picker_button
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(ImportModalBodyAction::OpenFilePicker)
            })
            .with_cursor(Cursor::PointingHand)
            .finish();

        let supported_file_type = appearance
            .ui_builder()
            .span(SUPPORTED_FILE_TYPE_TEXT)
            .with_style(UiComponentStyles {
                font_color: Some(
                    appearance
                        .theme()
                        .hint_text_color(appearance.theme().surface_2())
                        .into_solid(),
                ),
                ..Default::default()
            })
            .build()
            .finish();

        let link_to_document = appearance
            .ui_builder()
            .link(
                "Learn about file support and formatting".to_string(),
                Some(FILE_TYPE_DOCS_URL.to_string()),
                None,
                self.link_mouse_state.clone(),
            )
            .soft_wrap(false)
            .build()
            .finish();

        ConstrainedBox::new(
            Align::new(
                Flex::column()
                    .with_child(file_picker_element)
                    .with_child(
                        Container::new(supported_file_type)
                            .with_margin_top(16.)
                            .with_margin_bottom(16.)
                            .finish(),
                    )
                    .with_child(link_to_document)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .finish(),
            )
            .finish(),
        )
        .with_height(BODY_HEIGHT)
        .finish()
    }

    fn render_loaded_state(
        &self,
        file_upload_state: &FileUploadState,
        sync_queue_dequeueing: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut column = Flex::column();
        let folder_id_to_node = &file_upload_state.folder_id_to_node;
        let file_id_to_node = &file_upload_state.file_id_to_node;

        let folder_node = folder_id_to_node
            .get(&FolderId::root_id())
            .expect("Root node should exist");

        for item in folder_node.children() {
            column.add_child(item.render(
                appearance,
                0,
                file_upload_state.is_complete(),
                sync_queue_dequeueing,
                folder_id_to_node,
                file_id_to_node,
            ));
        }

        Container::new(column.finish())
            .with_margin_left(10.)
            .with_margin_top(20.)
            .with_margin_right(10.)
            .finish()
    }
}

impl Entity for ImportModalBody {
    type Event = ImportModalBodyEvent;
}

impl View for ImportModalBody {
    fn ui_name() -> &'static str {
        "ImportModalBody"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let sync_queue_dequeueing = SyncQueue::as_ref(app).is_dequeueing();
        let appearance = Appearance::as_ref(app);

        match &self.state {
            ImportState::Upload | ImportState::Loading | ImportState::PathLoaded => {
                self.render_upload_state(appearance, app)
            }
            ImportState::PathExpanded(paths) => {
                self.render_loaded_state(paths, sync_queue_dequeueing, appearance)
            }
        }
    }
}

impl TypedActionView for ImportModalBody {
    type Action = ImportModalBodyAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ImportModalBodyAction::OpenFilePicker => {
                self.state = ImportState::Loading;
                ctx.emit(ImportModalBodyEvent::OpenFilePicker);
                ctx.notify();
            }
            ImportModalBodyAction::FilePickerCancelled => {
                self.state = ImportState::Upload;
                ctx.emit(ImportModalBodyEvent::UploadSelected);
                ctx.notify();
            }
            ImportModalBodyAction::FilePickerError(err) => {
                let window_id = ctx.window_id();
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    toast_stack.add_ephemeral_toast(
                        DismissibleToast::error(format!("{err}")),
                        window_id,
                        ctx,
                    );
                });
                self.state = ImportState::Upload;
                ctx.emit(ImportModalBodyEvent::UploadSelected);
                ctx.notify();
            }
            ImportModalBodyAction::RetryFile(file_id) => {
                if let ImportState::PathExpanded(state) = &mut self.state {
                    state.set_file_and_parent_to_loading(*file_id);
                }
                self.parse_next_file(*file_id, false, ctx);
                ctx.notify();
            }
            ImportModalBodyAction::ClickedToOpenTarget(hashed_id) => {
                self.reset(ctx);
                ctx.emit(ImportModalBodyEvent::OpenTargetWithHashedId(
                    hashed_id.clone(),
                ));
            }
            ImportModalBodyAction::PathsSelected(paths) => {
                self.state = ImportState::PathLoaded;

                let paths_cloned = paths.clone();
                let handle = ctx.spawn(
                    expand_dirs(paths_cloned.into_iter().map(PathBuf::from).collect()),
                    move |view, mut upload_state, ctx| {
                        view.populate_folder_cloud_object(&mut upload_state, ctx);
                        view.state = ImportState::PathExpanded(upload_state);
                        view.parse_next_file(FileId::first_id(), true, ctx);
                        ctx.notify();
                    },
                );

                self.in_progress_handle = Some(handle.abort_handle());
                ctx.notify();
                ctx.emit(ImportModalBodyEvent::UploadSelected);
            }
        }
    }
}
