use std::path::PathBuf;

use warpui::{
    elements::ChildView, ui_components::components::UiComponentStyles, AppContext, Element, Entity,
    TypedActionView, View, ViewContext, ViewHandle,
};

use crate::{
    tab_configs::PickerStyle,
    util::git::{
        detect_current_branch, get_all_branches, get_all_branches_with_known_main,
        sort_branches_main_first, BranchEntry,
    },
    view_components::{DropdownItem, FilterableDropdown},
};

const DEFAULT_DROPDOWN_WIDTH: f32 = 380.;
/// Placeholder text shown in the dropdown top bar while branches are loading.
const LOADING_PLACEHOLDER: &str = "Fetching branches\u{2026}";

/// A filterable dropdown that lists local git branches for the given repo path.
///
/// Created with an optional `cwd` — if `None`, the picker starts with the
/// default value pre-populated while the async fetch runs.
/// When branches are available, main branches are sorted to the top.
///
/// Emits the selected branch name (a `String`) as its event.
pub struct BranchPicker {
    dropdown: ViewHandle<FilterableDropdown<String>>,
    /// Pre-selected default value from the TOML `default =` field, used to
    /// restore a selection after the async branch list arrives.
    default_value: Option<String>,
    /// Monotonically increasing counter incremented on every `fetch_branches` call.
    /// The async callback compares against the epoch captured at spawn time and
    /// discards stale results, preventing a slow earlier fetch from overwriting a
    /// faster later one when the repo changes mid-flight.
    fetch_epoch: usize,
    /// Main branch name cached after the first successful fetch for this repo.
    /// Passed to `get_all_branches_with_known_main` on subsequent fetches to skip
    /// the `detect_main_branch` step (which can make up to 6 sequential subprocess
    /// calls). Cleared in `refetch_branches` because a different repo may have a
    /// different main branch.
    cached_main_branch: Option<String>,
    /// True while an async branch fetch is in-flight. While loading, the
    /// dropdown is disabled so the user cannot interact with an empty list.
    is_loading: bool,
}

impl BranchPicker {
    /// Creates a new picker and immediately spawns an async fetch for the
    /// branches of the repo at `cwd`. `default_value` is pre-selected once
    /// the list arrives (if it appears in the list).
    pub fn new(
        cwd: Option<PathBuf>,
        default_value: Option<String>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self::new_with_style(cwd, default_value, None, ctx)
    }

    pub fn new_with_style(
        cwd: Option<PathBuf>,
        default_value: Option<String>,
        style: Option<PickerStyle>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let width = style.as_ref().map_or(DEFAULT_DROPDOWN_WIDTH, |s| s.width);
        let bg = style.and_then(|s| s.background);
        let dropdown = ctx.add_typed_action_view(|ctx| {
            let mut dropdown = FilterableDropdown::new(ctx);
            dropdown.set_top_bar_max_width(width);
            dropdown.set_menu_width(width, ctx);
            if let Some(bg) = bg {
                dropdown.set_style(UiComponentStyles {
                    background: Some(bg.into()),
                    ..Default::default()
                });
            }
            dropdown
        });

        let mut picker = Self {
            dropdown,
            default_value: default_value.clone(),
            fetch_epoch: 0,
            cached_main_branch: None,
            is_loading: false,
        };

        // Synchronously show the default value immediately so the dropdown is
        // never empty while the async branch fetch is in flight (or if the
        // repo has no commits / is not a git repo).
        if let Some(ref default) = default_value {
            let default = default.clone();
            picker.dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_items(
                    vec![DropdownItem::new(default.clone(), default.clone())],
                    ctx,
                );
                dropdown.set_selected_by_name(default.as_str(), ctx);
            });
        }

        // Kick off the async branch fetch to replace the placeholder with
        // the full list of branches from the repo.
        if let Some(cwd) = cwd {
            picker.fetch_branches(cwd, ctx);
        }

        picker
    }

    /// Fetches branches for `cwd` asynchronously and populates the dropdown.
    ///
    /// On the first call for a given repo this runs `get_all_branches`, which
    /// internally calls `detect_main_branch` (up to 6 sequential subprocess calls).
    /// The detected main branch is cached and reused on all subsequent calls via
    /// `get_all_branches_with_known_main`, reducing each refetch to a single
    /// `git for-each-ref` invocation.
    ///
    /// A `fetch_epoch` counter guards against stale results: if a second fetch
    /// completes before a slower first one, the first result is silently discarded.
    fn fetch_branches(&mut self, cwd: PathBuf, ctx: &mut ViewContext<Self>) {
        self.is_loading = true;
        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_disabled(ctx);
            // Show loading text in the dropdown top bar so the modal
            // doesn't shift layout while the fetch is in-flight.
            let placeholder = DropdownItem::new(LOADING_PLACEHOLDER.to_string(), String::new());
            dropdown.set_items(vec![placeholder], ctx);
            dropdown.set_selected_by_name(LOADING_PLACEHOLDER, ctx);
        });

        self.fetch_epoch += 1;
        let epoch = self.fetch_epoch;
        let known_main = self.cached_main_branch.clone();

        ctx.spawn(
            async move {
                let branches = match known_main {
                    Some(ref main) => {
                        get_all_branches_with_known_main(&cwd, main, None, false).await
                    }
                    None => get_all_branches(&cwd, None, false).await,
                };

                // git for-each-ref only lists refs backed by actual commits,
                // so a freshly initialised repo (`git init`, no commits yet)
                // returns an empty list even though HEAD points to a valid
                // branch name (e.g. "main"). Fall back to the current branch
                // so the picker still has a usable entry.
                match branches {
                    Ok(ref list) if list.is_empty() => {
                        if let Ok(current) = detect_current_branch(&cwd).await {
                            let trimmed = current.trim().to_string();
                            if !trimmed.is_empty() {
                                return Ok(vec![BranchEntry {
                                    name: trimmed,
                                    is_main: true,
                                }]);
                            }
                        }
                        branches
                    }
                    _ => branches,
                }
            },
            move |me, result, ctx| {
                // Discard results from a superseded fetch.
                if me.fetch_epoch != epoch {
                    return;
                }

                me.is_loading = false;
                me.dropdown.update(ctx, |dropdown, ctx| {
                    dropdown.set_enabled(ctx);
                });

                let branches = match result {
                    Ok(branches) => branches,
                    Err(err) => {
                        log::warn!("BranchPicker: failed to fetch branches: {err}");
                        Vec::new()
                    }
                };

                // Cache the detected main branch for subsequent refetches so
                // detect_main_branch is only run once per repo.
                if me.cached_main_branch.is_none() {
                    me.cached_main_branch = branches
                        .iter()
                        .find(|entry| entry.is_main)
                        .map(|entry| entry.name.clone());
                }

                // Main branches first, then the rest in recency order.
                let mut items: Vec<DropdownItem<String>> = sort_branches_main_first(&branches)
                    .map(|entry| DropdownItem::new(entry.name.clone(), entry.name.clone()))
                    .collect();

                // Add the default as the first item if it isn't already in the list
                // (e.g. the user typed a branch name that doesn't exist locally yet).
                if let Some(ref default) = me.default_value {
                    if !branches.iter().any(|entry| entry.name == *default) {
                        items.insert(0, DropdownItem::new(default.clone(), default.clone()));
                    }
                }

                // Determine which branch to auto-select: explicit default
                // first, then the detected main branch — but only if the
                // branch actually exists in the fetched list.
                let auto_select = me
                    .default_value
                    .clone()
                    .or_else(|| me.cached_main_branch.clone())
                    .filter(|name| items.iter().any(|item| item.display_text == *name));

                me.dropdown.update(ctx, |dropdown, ctx| {
                    dropdown.set_items(items, ctx);

                    if let Some(ref name) = auto_select {
                        dropdown.set_selected_by_name(name.as_str(), ctx);
                    }
                });

                // Emit the auto-selected value so parent views (e.g. the
                // worktree modal) update their state without requiring
                // an explicit user click.
                if let Some(ref name) = auto_select {
                    ctx.emit(name.clone());
                }

                ctx.notify();
            },
        );
    }

    /// Clears the current branch list and re-fetches for a new repo path.
    ///
    /// Called when the user changes the repo selection in the params modal.
    /// Clears stale items immediately so the dropdown shows empty while loading.
    /// Also clears `cached_main_branch` because the new repo may have a different
    /// main branch name.
    pub fn refetch_branches(&mut self, new_cwd: PathBuf, ctx: &mut ViewContext<Self>) {
        self.default_value = None;
        self.cached_main_branch = None;
        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(vec![], ctx);
            // Clear the cached selected_item so the closed dropdown no longer
            // shows a stale branch name from the previous repo.
            dropdown.set_selected_by_name("", ctx);
        });
        self.fetch_branches(new_cwd, ctx);
    }

    pub fn toggle_dropdown(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        self.dropdown.update(ctx, |dropdown, ctx| {
            dropdown.toggle_expanded(ctx);
        });
        self.dropdown.as_ref(ctx).is_expanded()
    }

    pub fn selected_value(&self, app: &AppContext) -> Option<String> {
        // While loading, the dropdown shows a placeholder label
        // ("Fetching branches…") that must not be treated as a real
        // branch selection.
        if self.is_loading {
            return None;
        }
        self.dropdown.as_ref(app).selected_item_label()
    }

    /// Returns `true` while an async branch fetch is in-flight.
    pub fn is_loading(&self) -> bool {
        self.is_loading
    }
}

impl Entity for BranchPicker {
    type Event = String;
}

impl View for BranchPicker {
    fn ui_name() -> &'static str {
        "BranchPicker"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.dropdown).finish()
    }
}

impl TypedActionView for BranchPicker {
    type Action = String;

    fn handle_action(&mut self, action: &String, ctx: &mut ViewContext<Self>) {
        ctx.emit(action.clone());
    }
}
