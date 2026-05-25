use std::cmp::Ordering;

use typed_path::TypedPathBuf;
use warp_completer::completer::{EngineDirEntry, EngineFileType, PathCompletionContext};
use warp_util::file_type::is_binary_file;
use warpui::r#async::SpawnedFutureHandle;
use warpui::{AppContext, Entity, ModelContext};

use super::display_menu::GenericMenuItem;
use crate::completer::SessionContext;
use crate::ui_components::icons::Icon;

/// DirectoryFetcher model that caches directory state and provides an explicit refetch API
pub struct DirectoryFetcher {
    current_directory: String,
    /// Cached directory contents as menu items
    cached_files: Vec<DirectoryItem>,
    /// Session context for async operations (required for directory fetching)
    session_context: Option<SessionContext>,
    /// Handle to the fetch operation
    fetch_handle: Option<SpawnedFutureHandle>,
}

#[derive(Debug, Clone)]
pub enum DirectoryFetcherEvent {
    /// Emitted when directory contents have been updated
    DirectoryContentsUpdated,
    /// Emitted when a fetch operation starts
    FetchStarted,
    /// Emitted when a fetch operation completes (successfully or with error)
    FetchCompleted { success: bool },
}

impl DirectoryFetcher {
    /// Create a new DirectoryFetcher for the given directory
    pub fn new(
        directory_path: String,
        session_context: Option<SessionContext>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut fetcher = Self {
            current_directory: directory_path.clone(),
            cached_files: vec![],
            session_context,
            fetch_handle: None,
        };

        fetcher.refetch_directory(ctx);
        fetcher
    }

    /// Explicitly refetch the directory contents
    pub fn refetch_directory(&mut self, ctx: &mut ModelContext<Self>) {
        if self.is_fetching() {
            return;
        }

        // Always use async method - SessionContext works for both local and remote sessions
        if let Some(session_ctx) = self.session_context.clone() {
            let dir_path = self.current_directory.clone();

            self.fetch_handle = Some(ctx.spawn(
                async move { Self::fetch_files_async(&session_ctx, &dir_path).await },
                |fetcher, files, ctx| {
                    fetcher.cached_files = files;
                    fetcher.fetch_handle = None;
                    ctx.emit(DirectoryFetcherEvent::DirectoryContentsUpdated);
                    ctx.emit(DirectoryFetcherEvent::FetchCompleted { success: true });
                    ctx.notify();
                },
            ));
            ctx.emit(DirectoryFetcherEvent::FetchStarted);
        } else {
            // If no session context, we can't fetch directory contents
            log::warn!("No SessionContext available for directory fetching");
            ctx.emit(DirectoryFetcherEvent::FetchCompleted { success: false });
            ctx.notify();
        }
    }

    /// Asynchronously list directory files using SessionContext
    async fn fetch_files_async(
        session_context: &SessionContext,
        dir_path: &str,
    ) -> Vec<DirectoryItem> {
        // Convert the directory path to TypedPathBuf, expanding ~ if needed
        let expanded_path = shellexpand::tilde(dir_path).into_owned();
        let typed_path = if expanded_path != dir_path {
            TypedPathBuf::from(expanded_path)
        } else {
            TypedPathBuf::from(dir_path)
        };

        // Use SessionContext to get directory entries (works for both local and remote sessions)
        let entries = session_context.list_directory_entries(typed_path).await;

        // Convert EngineDirEntry to GenericMenuItem, filtering out hidden files
        let mut items: Vec<DirectoryItem> = entries
            .iter()
            .filter(|entry| !entry.is_hidden()) // Skip hidden files (starting with '.')
            .map(engine_entry_to_menu_item)
            .collect();

        // Sort: directories first, then text files, then other files, all alphabetically within their groups
        sort_menu_items(&mut items);
        items
    }

    /// Update the session context (useful when it becomes available later)
    pub fn update_session_context(
        &mut self,
        session_context: Option<SessionContext>,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(handle) = self.fetch_handle.take() {
            // Cancel the fetch operation if it's in progress
            handle.abort();
        }

        self.session_context = session_context;
        self.refetch_directory(ctx);
    }

    /// Get the current directory path
    pub fn current_directory(&self) -> &str {
        &self.current_directory
    }

    /// Get the cached directory files
    pub fn cached_files(&self) -> &[DirectoryItem] {
        &self.cached_files
    }

    /// Check if a fetch operation is in progress
    pub fn is_fetching(&self) -> bool {
        self.fetch_handle.is_some()
    }

    /// Change the current directory and refetch contents
    pub fn change_directory(&mut self, new_directory: String, ctx: &mut ModelContext<Self>) {
        if self.current_directory != new_directory {
            self.current_directory = new_directory;
            self.cached_files.clear();
            self.refetch_directory(ctx);
        }
    }
}

impl Entity for DirectoryFetcher {
    type Event = DirectoryFetcherEvent;
}

#[derive(Debug, Clone, PartialOrd, PartialEq)]
pub enum DirectoryType {
    Directory,
    TextFile,
    OtherFile,
    NavigateToParent,
}

#[derive(Debug, Clone, PartialOrd, PartialEq)]
pub struct DirectoryItem {
    pub name: String,
    pub directory_type: DirectoryType,
}

impl GenericMenuItem for DirectoryItem {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> String {
        self.name.clone()
    }

    fn icon(&self, _app: &AppContext) -> Option<Icon> {
        Some(match self.directory_type {
            DirectoryType::Directory => Icon::Folder,
            DirectoryType::TextFile => Icon::File,
            DirectoryType::OtherFile => Icon::File,
            DirectoryType::NavigateToParent => Icon::ArrowUp,
        })
    }

    fn action_data(&self) -> String {
        self.name.clone()
    }
}

/// Sort menu items: directories first, then text files, then other files, all alphabetically within their groups
fn sort_menu_items(items: &mut [DirectoryItem]) {
    items.sort_by(|a, b| {
        match (&a.directory_type, &b.directory_type) {
            (DirectoryType::Directory, DirectoryType::TextFile)
            | (DirectoryType::Directory, DirectoryType::OtherFile) => Ordering::Less,
            (DirectoryType::TextFile, DirectoryType::Directory)
            | (DirectoryType::OtherFile, DirectoryType::Directory) => Ordering::Greater,
            (DirectoryType::TextFile, DirectoryType::OtherFile) => Ordering::Less,
            (DirectoryType::OtherFile, DirectoryType::TextFile) => Ordering::Greater,
            _ => a.name.cmp(&b.name), // Same type, sort alphabetically
        }
    });
}

/// Convert an EngineDirEntry to a DirectoryItem
fn engine_entry_to_menu_item(entry: &EngineDirEntry) -> DirectoryItem {
    let name: String = entry.file_name().to_string();
    DirectoryItem {
        name: name.clone(),
        directory_type: match entry.file_type {
            EngineFileType::Directory => DirectoryType::Directory,
            EngineFileType::File => {
                if is_binary_file(&name) {
                    DirectoryType::OtherFile
                } else {
                    DirectoryType::TextFile
                }
            }
        },
    }
}

#[cfg(test)]
#[path = "directory_fetcher_tests.rs"]
mod tests;
