use crate::{
    drive::{cloud_object_styling::warp_drive_icon_color, DriveObjectType},
    notebooks::post_process_notebook,
    workflows::{
        export_workflow::export_deserialize, workflow::Workflow, workflow_enum::WorkflowEnum,
    },
};
use anyhow::Result;
use async_recursion::async_recursion;
use futures_lite::StreamExt;
use pathfinder_color::ColorU;
use std::{
    collections::HashMap,
    ffi::OsStr,
    ops::{Add, AddAssign, SubAssign},
    path::{Path, PathBuf},
};
use warpui::{
    elements::{
        Align, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Flex, Hoverable,
        MouseStateHandle, ParentElement, Radius, Shrinkable,
    },
    platform::Cursor,
    ui_components::components::{UiComponent, UiComponentStyles},
    Element,
};

use crate::{
    appearance::Appearance, notebooks::file::is_markdown_file, server::ids::ClientId,
    themes::theme::Fill, ui_components::icons::Icon,
};

use super::modal_body::{ImportModalBodyAction, BASE_INDENT, IMPORT_FONT_SIZE, INDENT_MARGIN};

#[cfg(test)]
#[path = "node_tests.rs"]
mod node_tests;

/// Unique ID for a file node.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileId(pub usize);

impl FileId {
    pub(super) fn first_id() -> Self {
        FileId(0)
    }
}

/// Unique ID for a folder node.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct FolderId(pub usize);

impl FolderId {
    pub(super) fn root_id() -> Self {
        FolderId(0)
    }
}

impl From<usize> for FileId {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl From<usize> for FolderId {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl AddAssign<usize> for FileId {
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs
    }
}

impl Add<usize> for FileId {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl AddAssign<usize> for FolderId {
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs
    }
}

impl SubAssign<usize> for FolderId {
    fn sub_assign(&mut self, rhs: usize) {
        self.0 -= rhs
    }
}

/// A node representing either a imported folder or a file.
pub(super) enum ImportedNode {
    File(FileId),
    Folder(FolderId),
}

impl ImportedNode {
    /// Initiate the import file tree from a path.
    #[async_recursion]
    pub(super) async fn initiate_from_path(
        full_path: PathBuf,
        parent_folder_id: FolderId,
        next_folder_id: &mut FolderId,
        next_file_id: &mut FileId,
        folder_id_to_node: &mut HashMap<FolderId, FolderNode>,
        file_id_to_node: &mut HashMap<FileId, FileNode>,
    ) -> Option<Self> {
        let name = full_path.file_stem()?.to_str()?.to_string();

        if full_path.is_dir() {
            let current_folder_id = *next_folder_id;
            let mut folder_node = FolderNode::new(name, parent_folder_id);
            *next_folder_id += 1;

            let mut entries = async_fs::read_dir(full_path).await.ok()?;

            // Recursively create children nodes from the files and folders under the current directory.
            while let Some(entry) = entries.try_next().await.ok()? {
                let child_path = entry.path();

                if let Some(child_node) = ImportedNode::initiate_from_path(
                    child_path,
                    current_folder_id,
                    next_folder_id,
                    next_file_id,
                    folder_id_to_node,
                    file_id_to_node,
                )
                .await
                {
                    folder_node.children.push(child_node);
                }
            }

            // If the folder's parent is the root node, we should keep the folder node even if
            // it has no children.
            if !folder_node.children.is_empty() || parent_folder_id == FolderId::root_id() {
                folder_id_to_node.insert(current_folder_id, folder_node);
                return Some(ImportedNode::Folder(current_folder_id));
            }

            // If the folder children is empty, we don't consider the folder as an import node.
            *next_folder_id -= 1;
        } else if full_path.is_file() {
            let file_type = full_path.as_path().try_into();
            if let Ok(file_type) = file_type {
                let file_node = FileNode::new(name, file_type, full_path, parent_folder_id);
                let current_file_id = *next_file_id;
                file_id_to_node.insert(current_file_id, file_node);
                *next_file_id += 1;

                return Some(ImportedNode::File(current_file_id));
            }
        }

        None
    }

    pub(super) fn render(
        &self,
        appearance: &Appearance,
        indent_level: usize,
        allow_click_to_open_target: bool,
        sync_queue_dequeueing: bool,
        folder_id_to_node: &HashMap<FolderId, FolderNode>,
        file_id_to_node: &HashMap<FileId, FileNode>,
    ) -> Box<dyn Element> {
        match &self {
            ImportedNode::File(file_id) => {
                let file_node = file_id_to_node.get(file_id).expect("Should exist");
                file_node.render(
                    indent_level,
                    sync_queue_dequeueing,
                    allow_click_to_open_target,
                    *file_id,
                    appearance,
                )
            }
            ImportedNode::Folder(folder_id) => {
                let folder_node = folder_id_to_node.get(folder_id).expect("Should exist");
                folder_node.render(
                    sync_queue_dequeueing,
                    appearance,
                    indent_level,
                    allow_click_to_open_target,
                    folder_id_to_node,
                    file_id_to_node,
                )
            }
        }
    }

    #[cfg(test)]
    fn debug_print(
        &self,
        folder_id_to_node: &HashMap<FolderId, FolderNode>,
        file_id_to_node: &HashMap<FileId, FileNode>,
    ) -> String {
        match &self {
            ImportedNode::File(file_id) => {
                let file_node = file_id_to_node.get(file_id).expect("Should exist");
                file_node.debug_print()
            }
            ImportedNode::Folder(folder_id) => {
                let folder_node = folder_id_to_node.get(folder_id).expect("Should exist");
                folder_node.debug_print(folder_id_to_node, file_id_to_node)
            }
        }
    }
}

pub(super) struct FolderNode {
    parent_id: FolderId,
    cloud_object_id: ClientId,
    name: String,
    children: Vec<ImportedNode>,
    server_id: Option<String>,
    all_children_synced: bool,
    status: UploadStatus,

    open_button_mouse_state: MouseStateHandle,
}

impl FolderNode {
    fn new(name: String, parent_id: FolderId) -> Self {
        Self {
            name,
            parent_id,
            cloud_object_id: ClientId::new(),
            children: Vec::new(),
            server_id: None,
            all_children_synced: false,
            status: UploadStatus::Loading,
            open_button_mouse_state: Default::default(),
        }
    }

    #[cfg(test)]
    fn debug_print(
        &self,
        folder_id_to_node: &HashMap<FolderId, FolderNode>,
        file_id_to_node: &HashMap<FileId, FileNode>,
    ) -> String {
        use itertools::Itertools;

        if self.children.is_empty() {
            return self.name.clone();
        }

        let children_string = self
            .children
            .iter()
            .map(|child_node| child_node.debug_print(folder_id_to_node, file_id_to_node))
            .sorted()
            .join(", ");
        format!("{}({})", self.name.clone(), children_string)
    }

    fn are_children_saved_locally(
        &self,
        folder_id_to_node: &HashMap<FolderId, FolderNode>,
        file_id_to_node: &HashMap<FileId, FileNode>,
    ) -> bool {
        self.children.iter().all(|node| match node {
            ImportedNode::File(file_id) => file_id_to_node
                .get(file_id)
                .map(|file_node| file_node.status.is_saved_locally())
                .unwrap_or(true),
            ImportedNode::Folder(folder_id) => folder_id_to_node
                .get(folder_id)
                .map(|folder_node| folder_node.status.is_loaded())
                .unwrap_or(true),
        })
    }

    // Check if the folder is loaded. This is true if all of its children are loaded.
    fn are_children_loaded(
        &self,
        folder_id_to_node: &HashMap<FolderId, FolderNode>,
        file_id_to_node: &HashMap<FileId, FileNode>,
    ) -> bool {
        self.children.iter().all(|node| match node {
            ImportedNode::File(file_id) => file_id_to_node
                .get(file_id)
                .map(|file_node| file_node.status.is_loaded())
                .unwrap_or(true),
            ImportedNode::Folder(folder_id) => folder_id_to_node
                .get(folder_id)
                .map(|folder_node| folder_node.status.is_loaded())
                .unwrap_or(true),
        })
    }

    pub(super) fn cloud_id(&self) -> ClientId {
        self.cloud_object_id
    }

    pub(super) fn name(&self) -> String {
        self.name.clone()
    }

    pub(super) fn parent_id(&self) -> FolderId {
        self.parent_id
    }

    pub(super) fn children(&self) -> &Vec<ImportedNode> {
        &self.children
    }

    fn render(
        &self,
        sync_queue_dequeueing: bool,
        appearance: &Appearance,
        indent_level: usize,
        allow_click_to_open_target: bool,
        folder_id_to_node: &HashMap<FolderId, FolderNode>,
        file_id_to_node: &HashMap<FileId, FileNode>,
    ) -> Box<dyn Element> {
        let override_color = self.status.override_text_color(appearance);
        let status_icon = self
            .status
            .render_status_icon(sync_queue_dequeueing, appearance);

        let icon_color =
            override_color.unwrap_or(warp_drive_icon_color(appearance, DriveObjectType::Folder));
        let icon = ConstrainedBox::new(
            Icon::Folder
                .to_warpui_icon(Fill::Solid(icon_color))
                .finish(),
        )
        .with_height(IMPORT_FONT_SIZE)
        .with_width(IMPORT_FONT_SIZE)
        .finish();

        let mut column = Flex::column();
        let row = Flex::row()
            .with_child(status_icon)
            .with_child(Container::new(icon).with_margin_right(3.).finish())
            .with_child(
                appearance
                    .ui_builder()
                    .span(self.name.clone())
                    .with_style(UiComponentStyles {
                        font_size: Some(IMPORT_FONT_SIZE),
                        font_color: override_color,
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish();

        let total_indent = BASE_INDENT + INDENT_MARGIN * indent_level as f32;
        let folder_row = match &self.status {
            UploadStatus::Loaded(server_id) if allow_click_to_open_target => {
                render_highlighted_pill(
                    row,
                    total_indent,
                    self.open_button_mouse_state.clone(),
                    server_id.to_owned(),
                    appearance,
                )
            }
            _ => Container::new(row)
                .with_margin_left(total_indent)
                .with_padding_top(10.)
                .with_padding_bottom(10.)
                .finish(),
        };

        column.add_child(folder_row);

        for item in &self.children {
            column.add_child(item.render(
                appearance,
                indent_level + 1,
                allow_click_to_open_target,
                sync_queue_dequeueing,
                folder_id_to_node,
                file_id_to_node,
            ));
        }

        column.finish()
    }
}

pub(super) enum FileContent {
    Workflow {
        workflows: Vec<Workflow>,
        workflow_enums: HashMap<ClientId, WorkflowEnum>,
    },
    Notebook(String),
}

#[derive(Debug, Clone, Copy)]
pub(super) enum FileType {
    Workflow,
    Notebook,
}

impl TryFrom<&Path> for FileType {
    type Error = ();

    fn try_from(path: &Path) -> Result<Self, Self::Error> {
        if is_markdown_file(path) {
            Ok(FileType::Notebook)
        } else {
            let extension = path.extension();
            if extension == Some(OsStr::new("yaml")) || extension == Some(OsStr::new("yml")) {
                Ok(FileType::Workflow)
            } else {
                Err(())
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum UploadStatus {
    Loading,
    SavedLocally,
    Loaded(String),
    Error(String),
}

impl UploadStatus {
    pub(super) fn is_loaded(&self) -> bool {
        !matches!(&self, Self::Loading | Self::SavedLocally)
    }

    pub(super) fn is_saved_locally(&self) -> bool {
        !matches!(&self, Self::Loading)
    }

    fn override_text_color(&self, appearance: &Appearance) -> Option<ColorU> {
        match &self {
            Self::Error(_) => Some(
                appearance
                    .theme()
                    .sub_text_color(appearance.theme().surface_1())
                    .into_solid(),
            ),
            _ => None,
        }
    }

    fn render_status_icon(
        &self,
        sync_queue_dequeueing: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let status_icon_element = match &self {
            UploadStatus::SavedLocally if !sync_queue_dequeueing => Icon::Laptop
                .to_warpui_icon(
                    appearance
                        .theme()
                        .sub_text_color(appearance.theme().surface_1()),
                )
                .finish(),
            UploadStatus::Loading | UploadStatus::SavedLocally => Icon::Refresh
                .to_warpui_icon(
                    appearance
                        .theme()
                        .sub_text_color(appearance.theme().surface_1()),
                )
                .finish(),
            UploadStatus::Loaded(_) => Icon::Check
                .to_warpui_icon(Fill::Solid(ColorU::new(11, 142, 71, 255)))
                .finish(),
            UploadStatus::Error(_) => Icon::AlertTriangle
                .to_warpui_icon(Fill::Solid(appearance.theme().ui_error_color()))
                .finish(),
        };

        Container::new(
            ConstrainedBox::new(status_icon_element)
                .with_height(IMPORT_FONT_SIZE)
                .with_width(IMPORT_FONT_SIZE)
                .finish(),
        )
        .with_margin_right(8.)
        .finish()
    }
}

#[derive(Debug)]
pub(super) struct FileNode {
    /// The display name of the file. This is not necessarily the same as its on-disk filename.
    name: String,
    file_type: FileType,
    status: UploadStatus,
    full_path: PathBuf,
    parent_id: FolderId,

    refresh_button_mouse_state: MouseStateHandle,
    open_button_mouse_state: MouseStateHandle,
}

impl FileNode {
    fn new(name: String, file_type: FileType, full_path: PathBuf, parent_id: FolderId) -> Self {
        Self {
            name,
            file_type,
            full_path,
            parent_id,
            status: UploadStatus::Loading,

            refresh_button_mouse_state: Default::default(),
            open_button_mouse_state: Default::default(),
        }
    }

    #[cfg(test)]
    fn debug_print(&self) -> String {
        self.name.clone()
    }

    pub(super) fn full_path(&self) -> PathBuf {
        self.full_path.clone()
    }

    pub(super) fn file_type(&self) -> FileType {
        self.file_type
    }

    pub(super) fn saved_locally(&mut self) {
        if !self.status.is_loaded() {
            self.status = UploadStatus::SavedLocally;
        }
    }

    fn render(
        &self,
        indent_level: usize,
        sync_queue_dequeueing: bool,
        allow_click_to_open_target: bool,
        file_id: FileId,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let status_icon = self
            .status
            .render_status_icon(sync_queue_dequeueing, appearance);

        let override_color = self.status.override_text_color(appearance);

        let icon_element = match &self.file_type {
            FileType::Workflow => Icon::Workflow
                .to_warpui_icon(Fill::Solid(override_color.unwrap_or(
                    warp_drive_icon_color(appearance, DriveObjectType::Workflow),
                )))
                .finish(),
            FileType::Notebook => Icon::Notebook
                .to_warpui_icon(Fill::Solid(override_color.unwrap_or(
                    warp_drive_icon_color(
                        appearance,
                        DriveObjectType::Notebook {
                            is_ai_document: false,
                        },
                    ),
                )))
                .finish(),
        };
        let icon = ConstrainedBox::new(icon_element)
            .with_height(IMPORT_FONT_SIZE)
            .with_width(IMPORT_FONT_SIZE)
            .finish();

        let mut item_row = Flex::row()
            .with_child(status_icon)
            .with_child(Container::new(icon).with_margin_right(4.).finish())
            .with_child(
                appearance
                    .ui_builder()
                    .span(self.name.clone())
                    .with_style(UiComponentStyles {
                        font_size: Some(IMPORT_FONT_SIZE),
                        font_color: override_color,
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        let total_indent = BASE_INDENT + INDENT_MARGIN * indent_level as f32;
        match &self.status {
            UploadStatus::Error(e) => {
                item_row.add_child(
                    Shrinkable::new(
                        1.,
                        Align::new(
                            Container::new(
                                appearance
                                    .ui_builder()
                                    .retry_button(16., self.refresh_button_mouse_state.clone())
                                    .build()
                                    .on_click(move |ctx, _, _| {
                                        ctx.dispatch_typed_action(ImportModalBodyAction::RetryFile(
                                            file_id,
                                        ))
                                    })
                                    .finish(),
                            )
                            .with_margin_right(4.)
                            .with_margin_left(4.)
                            .finish(),
                        )
                        .right()
                        .finish(),
                    )
                    .finish(),
                );

                let error_row = appearance
                    .ui_builder()
                    .span(e.to_string())
                    .with_style(UiComponentStyles {
                        font_color: Some(appearance.theme().ui_error_color()),
                        ..Default::default()
                    })
                    .build()
                    .finish();

                Container::new(
                    Flex::column()
                        .with_child(item_row.finish())
                        .with_child(
                            Container::new(error_row)
                                .with_margin_top(4.)
                                .with_margin_left(INDENT_MARGIN)
                                .finish(),
                        )
                        .finish(),
                )
                .with_margin_left(total_indent)
                .with_padding_top(10.)
                .with_padding_bottom(10.)
                .finish()
            }
            UploadStatus::Loaded(server_id) if allow_click_to_open_target => {
                render_highlighted_pill(
                    item_row.finish(),
                    total_indent,
                    self.open_button_mouse_state.clone(),
                    server_id.to_owned(),
                    appearance,
                )
            }
            _ => Container::new(item_row.finish())
                .with_margin_left(total_indent)
                .with_padding_top(10.)
                .with_padding_bottom(10.)
                .finish(),
        }
    }
}

pub(super) async fn expand_dirs(dirs: Vec<PathBuf>) -> FileUploadState {
    let mut next_folder_id = FolderId::from(0);
    let mut next_file_id = FileId::from(0);

    let mut folder_id_to_node = HashMap::new();
    let mut file_id_to_node = HashMap::new();

    let current_folder_id = next_folder_id;
    let mut dummy_root_node = FolderNode::new(String::new(), current_folder_id);

    next_folder_id += 1;
    for child_path in dirs {
        if let Some(child_node) = ImportedNode::initiate_from_path(
            child_path,
            current_folder_id,
            &mut next_folder_id,
            &mut next_file_id,
            &mut folder_id_to_node,
            &mut file_id_to_node,
        )
        .await
        {
            dummy_root_node.children.push(child_node);
        }
    }
    folder_id_to_node.insert(current_folder_id, dummy_root_node);

    FileUploadState::new(folder_id_to_node, file_id_to_node)
}

pub(super) struct FileUploadState {
    pub(super) folder_id_to_node: HashMap<FolderId, FolderNode>,
    pub(super) file_id_to_node: HashMap<FileId, FileNode>,
}

impl FileUploadState {
    fn new(
        folder_id_to_node: HashMap<FolderId, FolderNode>,
        file_id_to_node: HashMap<FileId, FileNode>,
    ) -> Self {
        Self {
            folder_id_to_node,
            file_id_to_node,
        }
    }

    #[cfg(test)]
    pub(super) fn debug_print(&self) -> String {
        ImportedNode::Folder(FolderId::root_id())
            .debug_print(&self.folder_id_to_node, &self.file_id_to_node)
    }

    // Get the cloud id for the provided folder id. Returns None if the folder
    // is the root node folder.
    pub(super) fn folder_cloud_id(&self, folder_id: FolderId) -> Option<ClientId> {
        if folder_id == FolderId::root_id() {
            // Root folder should not have a client id.
            return None;
        }
        match self.folder_id_to_node.get(&folder_id) {
            Some(folder_node) => Some(folder_node.cloud_id()),
            None => {
                log::error!("Provided folder id should exist");
                None
            }
        }
    }

    pub(super) fn file_name_and_parent_cloud_id(
        &self,
        file_id: FileId,
    ) -> Option<(String, Option<ClientId>)> {
        let file_node = self.file_id_to_node.get(&file_id)?;
        let parent_cloud_id = self.folder_cloud_id(file_node.parent_id);
        Some((file_node.name.clone(), parent_cloud_id))
    }

    pub(super) fn mark_folder_synced(&mut self, result: UploadResult, folder_id: FolderId) {
        let parent_id = if let Some(folder) = self.folder_id_to_node.get_mut(&folder_id) {
            let should_update_upstream_folders = match result {
                // If uploading the folder is not successful, its children will not upload.
                // Mark the folder as errored and update upstream folders.
                UploadResult::Error(e) => {
                    folder.status = UploadStatus::Error(e);
                    true
                }
                // If the folder has no children, mark the folder as completed and update
                // upstream folders.
                UploadResult::Success(server_id) => {
                    folder.server_id = Some(server_id.clone());

                    // If a folder has no children or all of its children complete syncing,
                    // we need to bubble the state up in the folder hierarchy tree.
                    if folder.children().is_empty() || folder.all_children_synced {
                        folder.status = UploadStatus::Loaded(server_id);
                        true
                    } else {
                        false
                    }
                }
            };

            if should_update_upstream_folders {
                Some(folder.parent_id)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(parent_id) = parent_id {
            self.update_upstream_folders_loaded(parent_id);
        }
    }

    pub(super) fn set_file_and_parent_to_loading(&mut self, file_id: FileId) {
        let parent_id = match self.file_id_to_node.get_mut(&file_id) {
            Some(file_node) => {
                file_node.status = UploadStatus::Loading;
                Some(file_node.parent_id)
            }
            None => None,
        };

        if let Some(parent_id) = parent_id {
            self.update_upstream_folders_loading(parent_id);
        }
    }

    /// File upload is completed if the root node is marked as complete.
    pub(super) fn is_complete(&self) -> bool {
        self.folder_id_to_node
            .get(&FolderId::root_id())
            .expect("Root node should exist")
            .status
            .is_loaded()
    }

    pub(super) fn all_files_saved_locally(&self) -> bool {
        self.folder_id_to_node
            .get(&FolderId::root_id())
            .expect("Root node should exist")
            .are_children_saved_locally(&self.folder_id_to_node, &self.file_id_to_node)
    }

    /// This recursively updates the upstream folder to be loading.
    fn update_upstream_folders_loading(&mut self, parent_folder_id: FolderId) {
        let mut next_folder_to_update = parent_folder_id;

        while let Some(folder_node) = self.folder_id_to_node.get(&next_folder_to_update) {
            let parent_node = folder_node.parent_id;

            if folder_node.status.is_loaded() {
                let folder_node = self
                    .folder_id_to_node
                    .get_mut(&next_folder_to_update)
                    .expect("Should exist");
                folder_node.all_children_synced = false;
                folder_node.status = UploadStatus::Loading
            } else {
                break;
            }

            if next_folder_to_update == FolderId::root_id() {
                break;
            }

            next_folder_to_update = parent_node;
        }
    }

    /// This recursively updates the upstream folder if it is completed.
    fn update_upstream_folders_loaded(&mut self, parent_folder_id: FolderId) {
        let mut next_folder_to_update = parent_folder_id;

        while let Some(folder_node) = self.folder_id_to_node.get(&next_folder_to_update) {
            let parent_node = folder_node.parent_id;
            let folder_is_root_node = next_folder_to_update == FolderId::root_id();

            if !folder_node.status.is_loaded()
                && folder_node.are_children_loaded(&self.folder_id_to_node, &self.file_id_to_node)
            {
                let folder_node = self
                    .folder_id_to_node
                    .get_mut(&next_folder_to_update)
                    .expect("Should exist");
                folder_node.all_children_synced = true;

                if let Some(id) = &folder_node.server_id {
                    folder_node.status = UploadStatus::Loaded(id.clone());
                } else if folder_is_root_node {
                    folder_node.status = UploadStatus::Loaded(String::new());
                } else {
                    // Normally a folder should have been uploaded for a file to be uploaded.
                    // However, in the rare case when a file errors out when parsing and it happens
                    // to be the last file in the folder, we could get into a situation where the
                    // the folder is not uploaded but all of its children complete syncing.
                    break;
                }
            } else {
                break;
            }

            if folder_is_root_node {
                break;
            }

            next_folder_to_update = parent_node;
        }
    }

    pub(super) fn update_tree_with_file_upload_result(
        &mut self,
        result: UploadResult,
        file_id: FileId,
    ) -> bool {
        let Some(file_node_to_update) = self.file_id_to_node.get_mut(&file_id) else {
            return false;
        };

        file_node_to_update.status = match result {
            UploadResult::Success(id) => UploadStatus::Loaded(id),
            UploadResult::Error(e) => UploadStatus::Error(format!("Failed to parse file: {e}")),
        };

        let parent_id = file_node_to_update.parent_id;
        self.update_upstream_folders_loaded(parent_id);
        true
    }
}

pub(super) async fn parse_file(path: PathBuf, file_type: FileType) -> Result<FileContent> {
    match file_type {
        FileType::Notebook => Ok(FileContent::Notebook(post_process_notebook(
            &async_fs::read_to_string(path).await?,
        ))),
        FileType::Workflow => {
            let file = async_fs::read(path).await?;
            let mut workflow_enums: HashMap<ClientId, WorkflowEnum> = HashMap::new();
            let mut workflows = vec![];

            for document in serde_yaml::Deserializer::from_slice(&file) {
                let (workflow, new_enums) = export_deserialize(document)?;

                workflows.push(workflow);
                workflow_enums.extend(new_enums);
            }

            Ok(FileContent::Workflow {
                workflows,
                workflow_enums,
            })
        }
    }
}

pub(super) enum UploadResult {
    Success(String),
    Error(String),
}

fn render_highlighted_pill(
    row: Box<dyn Element>,
    total_indent: f32,
    mouse_state_handle: MouseStateHandle,
    server_id: String,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let inner = Container::new(row)
        .with_margin_left(total_indent)
        .with_padding_top(5.)
        .with_padding_bottom(5.)
        .finish();

    Container::new(
        Hoverable::new(mouse_state_handle, |state| {
            if state.is_hovered() || state.is_clicked() {
                Container::new(inner)
                    .with_background(appearance.theme().surface_3())
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .finish()
            } else {
                inner
            }
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(ImportModalBodyAction::ClickedToOpenTarget(
                server_id.clone(),
            ))
        })
        .finish(),
    )
    .with_padding_top(5.)
    .with_padding_bottom(5.)
    .finish()
}
