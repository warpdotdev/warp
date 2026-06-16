use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ai::index::full_source_code_embedding::manager::{
    CodebaseIndexManager, CodebaseIndexManagerEvent,
};
use settings::Setting;
use warp_util::path::user_friendly_path;
use warpui::elements::{
    Border, ChildView, ConstrainedBox, Container, CrossAxisAlignment, Flex, Hoverable,
    MainAxisSize, MouseStateHandle, ParentElement, Text,
};
use warpui::platform::Cursor;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::persisted_workspace::{PersistedWorkspace, PersistedWorkspaceEvent};
use crate::appearance::Appearance;
use crate::ui_components::icons;
use crate::view_components::action_button::{ActionButton, SecondaryTheme};
use crate::view_components::{DropdownItem, FilterableDropdown};
use crate::workspace::tab_settings::{
    canonical_directory_key, DirectoryTabColor, DirectoryTabColors, TabSettings,
    TabSettingsChangedEvent,
};

const ADD_DIRECTORY_LABEL: &str = "+ Add directory…";
const BUTTON_LABEL: &str = "Add directory color";
const MENU_WIDTH: f32 = 340.;

/// A dropdown used by the Directory tab colors settings widget, with a button fallback
/// when there are no known repos left to show in the dropdown.
///
/// Lists known repos (from `CodebaseIndexManager` and `PersistedWorkspace`) that
/// are not yet present in the user's `directory_tab_colors` with a non-`Suppressed`
/// color, and exposes a pinned `+ Add directory…` footer that falls back to the
/// native folder picker.
///
/// Emits:
/// - [`DirectoryColorAddPickerEvent::Selected`] when the user picks a row.
/// - [`DirectoryColorAddPickerEvent::RequestAddFromFilePicker`] when the user clicks
///   the pinned footer or the fallback button.
pub(super) struct DirectoryColorAddPicker {
    button: ViewHandle<ActionButton>,
    dropdown: ViewHandle<FilterableDropdown<DirectoryColorAddPickerAction>>,
    footer_mouse_state: MouseStateHandle,
    has_dropdown_items: bool,
    /// Inputs used for the last `refresh_items` computation.
    /// Used to short-circuit `refresh_items` when nothing relevant has changed,
    /// so noisy events like `SyncStateUpdated` / `IndexMetadataUpdated` don't pay
    /// for per-path `exists()` + `canonicalize()` on every fire.
    cached_inputs: Option<RefreshCacheKey>,
}

/// Cheap-to-compute inputs to `refresh_items` used to skip the expensive
/// filesystem checks inside `compute_candidate_paths` when none of these
/// inputs have changed since the last refresh.
#[derive(PartialEq, Eq)]
struct RefreshCacheKey {
    indexed_paths: HashSet<PathBuf>,
    persisted_paths: HashSet<PathBuf>,
    existing: DirectoryTabColors,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum DirectoryColorAddPickerAction {
    Select(PathBuf),
    AddNewDirectory,
}

pub(super) enum DirectoryColorAddPickerEvent {
    Selected(PathBuf),
    RequestAddFromFilePicker,
}

impl DirectoryColorAddPicker {
    pub(super) fn new(ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&CodebaseIndexManager::handle(ctx), |me, _, event, ctx| {
            // Refresh for any event that may change the set of indexed codebase paths or
            // persisted workspaces: new index created, sync state updated (which covers
            // `index_directory`), indices removed, or index metadata updated (which covers
            // workspaces persisted via `PersistedWorkspace::handle_index_metadata_event`
            // without a `WorkspaceAdded` event). Refresh is idempotent thanks to the
            // cache in `refresh_items`, so the noisier events (`Modified`/`Queried`) are
            // cheap when nothing relevant has changed.
            match event {
                CodebaseIndexManagerEvent::NewIndexCreated { .. }
                | CodebaseIndexManagerEvent::SyncStateUpdated { .. }
                | CodebaseIndexManagerEvent::RemoveExpiredIndexMetadata { .. }
                | CodebaseIndexManagerEvent::IndexMetadataUpdated { .. } => {
                    me.refresh_items(ctx);
                }
                CodebaseIndexManagerEvent::RetrievalRequestCompleted { .. }
                | CodebaseIndexManagerEvent::RetrievalRequestFailed { .. } => {}
            }
        });

        ctx.subscribe_to_model(&PersistedWorkspace::handle(ctx), |me, _, event, ctx| {
            if let PersistedWorkspaceEvent::WorkspaceAdded { .. } = event {
                me.refresh_items(ctx);
            }
        });

        ctx.subscribe_to_model(&TabSettings::handle(ctx), |me, _, event, ctx| {
            if let TabSettingsChangedEvent::DirectoryTabColors { .. } = event {
                me.refresh_items(ctx);
            }
        });

        let button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new(BUTTON_LABEL, SecondaryTheme)
                .with_icon(icons::Icon::Plus)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(DirectoryColorAddPickerAction::AddNewDirectory);
                })
        });

        let dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = FilterableDropdown::new(ctx);
            dropdown.set_top_bar_max_width(MENU_WIDTH);
            dropdown.set_menu_width(MENU_WIDTH, ctx);
            dropdown.set_menu_header_to_static(BUTTON_LABEL);
            dropdown
        });

        let mut picker = Self {
            button,
            dropdown,
            footer_mouse_state: MouseStateHandle::default(),
            has_dropdown_items: false,
            cached_inputs: None,
        };

        let mouse_state = picker.footer_mouse_state.clone();
        picker.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_footer(
                move |app| {
                    let appearance = Appearance::as_ref(app);
                    let theme = appearance.theme();
                    let is_hovered = mouse_state.lock().unwrap().is_hovered();
                    let bg = if is_hovered {
                        theme.accent_button_color()
                    } else {
                        theme.surface_2()
                    };
                    let font_family = appearance.ui_font_family();
                    let font_size = appearance.ui_font_size();
                    let text_color = theme.main_text_color(bg);
                    let border_fill = theme.outline();
                    let mouse_state_clone = mouse_state.clone();
                    Hoverable::new(mouse_state_clone, move |_| {
                        ConstrainedBox::new(
                            Container::new(
                                Flex::row()
                                    .with_main_axis_size(MainAxisSize::Max)
                                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                    .with_child(
                                        Text::new_inline(
                                            ADD_DIRECTORY_LABEL,
                                            font_family,
                                            font_size,
                                        )
                                        .with_color(text_color.into())
                                        .finish(),
                                    )
                                    .finish(),
                            )
                            .with_horizontal_padding(8.)
                            .with_vertical_padding(6.)
                            .with_background(bg)
                            .with_border(Border::top(1.).with_border_fill(border_fill))
                            .finish(),
                        )
                        .with_width(MENU_WIDTH)
                        .finish()
                    })
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(DirectoryColorAddPickerAction::AddNewDirectory);
                    })
                    .with_cursor(Cursor::PointingHand)
                    .finish()
                },
                ctx,
            );
        });

        picker.refresh_items(ctx);
        picker
    }

    fn refresh_items(&mut self, ctx: &mut ViewContext<Self>) {
        let indexed_paths: HashSet<PathBuf> = CodebaseIndexManager::as_ref(ctx)
            .get_codebase_paths()
            .cloned()
            .collect();
        let persisted_paths: HashSet<PathBuf> = PersistedWorkspace::as_ref(ctx)
            .workspaces()
            .map(|ws| ws.path)
            .collect();
        let existing = TabSettings::as_ref(ctx)
            .directory_tab_colors
            .value()
            .clone();

        let cache_key = RefreshCacheKey {
            indexed_paths,
            persisted_paths,
            existing,
        };
        if self.cached_inputs.as_ref() == Some(&cache_key) {
            return;
        }

        let candidates = compute_candidate_paths(
            cache_key.indexed_paths.iter().cloned(),
            cache_key.persisted_paths.iter().cloned(),
            &cache_key.existing,
            |p| p.exists(),
        );

        let home_dir =
            dirs::home_dir().and_then(|home_dir| home_dir.to_str().map(|s| s.to_owned()));
        let items: Vec<DropdownItem<DirectoryColorAddPickerAction>> = candidates
            .into_iter()
            .map(|path| {
                let label =
                    user_friendly_path(&path.to_string_lossy(), home_dir.as_deref()).to_string();
                DropdownItem::new(label, DirectoryColorAddPickerAction::Select(path))
            })
            .collect();

        self.cached_inputs = Some(cache_key);
        self.has_dropdown_items = !items.is_empty();
        let has_dropdown_items = self.has_dropdown_items;
        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(items, ctx);
            if !has_dropdown_items {
                dropdown.close(ctx);
            }
        });
        ctx.notify();
    }
}

impl Entity for DirectoryColorAddPicker {
    type Event = DirectoryColorAddPickerEvent;
}

impl View for DirectoryColorAddPicker {
    fn ui_name() -> &'static str {
        "DirectoryColorAddPicker"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        if self.has_dropdown_items {
            ChildView::new(&self.dropdown).finish()
        } else {
            ChildView::new(&self.button).finish()
        }
    }
}

impl TypedActionView for DirectoryColorAddPicker {
    type Action = DirectoryColorAddPickerAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            DirectoryColorAddPickerAction::Select(path) => {
                // Don't close the dropdown here: the FilterableDropdown is
                // mid-update while this handler runs, and it closes itself
                // after dispatch. See `FilterableDropdown::select_action_and_close`.
                ctx.emit(DirectoryColorAddPickerEvent::Selected(path.clone()));
            }
            DirectoryColorAddPickerAction::AddNewDirectory => {
                // Footer clicks dispatch via `EventContext` (a deferred effect),
                // so the dropdown is not mid-update when this handler runs.
                // The fallback button also dispatches here, so closing first is harmless.
                self.dropdown
                    .update(ctx, |dropdown, ctx| dropdown.close(ctx));
                ctx.emit(DirectoryColorAddPickerEvent::RequestAddFromFilePicker);
            }
        }
    }
}

/// Computes the set of directory paths that should be offered in the add-directory dropdown.
///
/// Candidates are the union of indexed codebase paths and persisted workspace
/// paths. An entry is filtered out if:
/// - its canonical key is already a key in `existing` with a value other than
///   [`DirectoryTabColor::Suppressed`] (those are already in the visible list), or
/// - `path_exists` returns `false` for the path.
///
/// Entries keyed as `Suppressed` are intentionally kept so the user can re-add
/// a previously removed directory.
///
/// The result is deduped by canonical key and sorted alphabetically by that key
/// so it matches the order of the visible colors list rendered below the picker.
fn compute_candidate_paths(
    indexed_paths: impl IntoIterator<Item = PathBuf>,
    persisted_paths: impl IntoIterator<Item = PathBuf>,
    existing: &DirectoryTabColors,
    path_exists: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    let mut seen_keys = HashSet::new();
    let mut candidates: Vec<(String, PathBuf)> = Vec::new();

    for path in indexed_paths.into_iter().chain(persisted_paths.into_iter()) {
        if !path_exists(&path) {
            continue;
        }

        let key = canonical_directory_key(&path);

        if let Some(existing_color) = existing.0.get(&key) {
            if !matches!(existing_color, DirectoryTabColor::Suppressed) {
                continue;
            }
        }

        if seen_keys.insert(key.clone()) {
            candidates.push((key, path));
        }
    }

    candidates.sort_by(|(a, _), (b, _)| a.cmp(b));
    candidates.into_iter().map(|(_, path)| path).collect()
}

#[cfg(test)]
#[path = "directory_color_add_picker_tests.rs"]
mod tests;
