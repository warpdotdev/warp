use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{Local, TimeZone};
use pathfinder_geometry::vector::{vec2f, Vector2F};
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::Icon;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, DragBarSide, Element, Expanded, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Point, Radius, Resizable, ResizableStateHandle,
    ScrollbarWidth, Shrinkable, Stack, Text,
};
use warpui::event::DispatchedEvent;
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    AfterLayoutContext, AppContext, Entity, EventContext, LayoutContext, ModelHandle, PaintContext,
    SingletonEntity, SizeConstraint, TypedActionView, View, ViewContext, ViewHandle,
};

use super::data::{
    apply_mutation, load_history, load_repository, CommitNode, FileChange, GitChangeKind,
    GitMutation, GitRefKind, GitRefLabel, RepositorySnapshot, HISTORY_PAGE_SIZE,
};
use super::layout::{layout_commits, GraphLayout};
use super::row_canvas::GraphRowCanvas;
use crate::appearance::Appearance;
use crate::code::buffer_location::LocalOrRemotePath;
use crate::code::editor::{add_color, remove_color};
use crate::code_review::git_repo_model::{GitRepoModels, GitRepoStatusEvent, GitRepoStatusModel};
use crate::ui_components::buttons::icon_button;
use crate::view_components::dropdown::{Dropdown, DropdownItem};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ChangeSection {
    Merge,
    Staged,
    Changes,
    Untracked,
}

impl ChangeSection {
    fn title(self) -> &'static str {
        match self {
            Self::Merge => "Merge Changes",
            Self::Staged => "Staged Changes",
            Self::Changes => "Changes",
            Self::Untracked => "Untracked Changes",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SourceControlAction {
    SelectRepository(LocalOrRemotePath),
    Refresh,
    ToggleSection(ChangeSection),
    ToggleHistory,
    OpenCodeReview,
    StagePaths(Vec<String>),
    UnstagePaths(Vec<String>),
    StageAll,
    UnstageAll,
    LoadMore,
}

#[derive(Clone, Debug)]
pub enum SourceControlEvent {
    OpenCodeReview { repo_path: LocalOrRemotePath },
}

fn relative_time_string(timestamp: i64) -> Option<String> {
    let elapsed_seconds = Local::now().timestamp().saturating_sub(timestamp).max(0);
    if elapsed_seconds < 60 {
        Some("just now".to_string())
    } else if elapsed_seconds < 60 * 60 {
        let minutes = elapsed_seconds / 60;
        Some(format!(
            "{minutes} minute{} ago",
            if minutes == 1 { "" } else { "s" }
        ))
    } else if elapsed_seconds < 24 * 60 * 60 {
        let hours = elapsed_seconds / (60 * 60);
        Some(format!(
            "{hours} hour{} ago",
            if hours == 1 { "" } else { "s" }
        ))
    } else if elapsed_seconds <= 30 * 24 * 60 * 60 {
        let days = elapsed_seconds / (24 * 60 * 60);
        Some(format!(
            "{days} day{} ago",
            if days == 1 { "" } else { "s" }
        ))
    } else {
        None
    }
}

fn absolute_time_string(timestamp: i64) -> String {
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|timestamp| timestamp.format("%B %-d, %Y at %-I:%M %p").to_string())
        .unwrap_or_else(|| "Unknown date".to_string())
}

#[derive(Default)]
struct StaticMouseStates {
    refresh: MouseStateHandle,
    history: MouseStateHandle,
    load_more: MouseStateHandle,
}

struct MeasuredHeight {
    child: Box<dyn Element>,
    handle: Arc<Mutex<f32>>,
}

impl MeasuredHeight {
    fn new(handle: Arc<Mutex<f32>>, child: Box<dyn Element>) -> Self {
        Self { child, handle }
    }
}

impl Element for MeasuredHeight {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let size = self.child.layout(constraint, ctx, app);
        *self
            .handle
            .lock()
            .expect("split area height should be accessible") = size.y();
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.child.paint(origin, ctx, app);
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }
}

pub struct SourceControlView {
    repositories: Vec<LocalOrRemotePath>,
    selected_repository: Option<LocalOrRemotePath>,
    repository_dropdown: ViewHandle<Dropdown<SourceControlAction>>,
    git_status_model: Option<ModelHandle<GitRepoStatusModel>>,
    snapshot: Option<RepositorySnapshot>,
    graph_layout: GraphLayout,
    error: Option<String>,
    is_active: bool,
    needs_refresh: bool,
    is_loading: bool,
    reload_after_current: bool,
    mutation_in_progress: bool,
    history_page_in_progress: bool,
    generation: u64,
    collapsed_sections: HashMap<ChangeSection, bool>,
    history_collapsed: bool,
    history_scroll_state: ClippedScrollStateHandle,
    changes_scroll_state: ClippedScrollStateHandle,
    split_state: ResizableStateHandle,
    split_area_height: Arc<Mutex<f32>>,
    static_mouse_states: StaticMouseStates,
    section_mouse_states: HashMap<ChangeSection, MouseStateHandle>,
    section_action_mouse_states: HashMap<ChangeSection, MouseStateHandle>,
    row_mouse_states: HashMap<(ChangeSection, String), MouseStateHandle>,
    row_action_mouse_states: HashMap<(ChangeSection, String), MouseStateHandle>,
    commit_mouse_states: HashMap<String, MouseStateHandle>,
}

impl SourceControlView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let repository_dropdown = ctx.add_typed_action_view(|ctx| {
            let ui_font_size = Appearance::as_ref(ctx).ui_font_size();
            let mut dropdown = Dropdown::new(ctx);
            dropdown.set_main_axis_size(MainAxisSize::Max, ctx);
            dropdown.set_vertical_margin(0., ctx);
            dropdown.set_top_bar_height(28., ctx);
            dropdown.set_font_size(ui_font_size, ctx);
            dropdown.set_match_menu_width_to_top_bar(true, ctx);
            dropdown
        });

        let sections = [
            ChangeSection::Merge,
            ChangeSection::Staged,
            ChangeSection::Changes,
            ChangeSection::Untracked,
        ];
        Self {
            repositories: Vec::new(),
            selected_repository: None,
            repository_dropdown,
            git_status_model: None,
            snapshot: None,
            graph_layout: GraphLayout::default(),
            error: None,
            is_active: false,
            needs_refresh: false,
            is_loading: false,
            reload_after_current: false,
            mutation_in_progress: false,
            history_page_in_progress: false,
            generation: 0,
            collapsed_sections: sections
                .into_iter()
                .map(|section| (section, false))
                .collect(),
            history_collapsed: false,
            history_scroll_state: ClippedScrollStateHandle::default(),
            changes_scroll_state: ClippedScrollStateHandle::default(),
            split_state: warpui::elements::resizable_state_handle(320.),
            split_area_height: Arc::new(Mutex::new(0.)),
            static_mouse_states: StaticMouseStates::default(),
            section_mouse_states: sections
                .into_iter()
                .map(|section| (section, MouseStateHandle::default()))
                .collect(),
            section_action_mouse_states: sections
                .into_iter()
                .map(|section| (section, MouseStateHandle::default()))
                .collect(),
            row_mouse_states: HashMap::new(),
            row_action_mouse_states: HashMap::new(),
            commit_mouse_states: HashMap::new(),
        }
    }

    pub fn set_available_repositories(
        &mut self,
        repositories: Vec<LocalOrRemotePath>,
        focused_repository: Option<LocalOrRemotePath>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.repositories = repositories;
        self.update_repository_dropdown(ctx);

        let next_repository = focused_repository
            .filter(|focused| self.repositories.contains(focused))
            .or_else(|| {
                self.selected_repository
                    .clone()
                    .filter(|selected| self.repositories.contains(selected))
            })
            .or_else(|| self.repositories.first().cloned());

        if next_repository != self.selected_repository {
            self.select_repository(next_repository, ctx);
        } else if self.repositories.is_empty() {
            self.clear_repository_state(ctx);
        }
        ctx.notify();
    }

    pub fn set_focused_repository(
        &mut self,
        focused_repository: Option<LocalOrRemotePath>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(repository) = focused_repository {
            if self.repositories.contains(&repository)
                && self.selected_repository.as_ref() != Some(&repository)
            {
                self.select_repository(Some(repository), ctx);
            }
        }
    }

    pub fn set_is_active(&mut self, is_active: bool, ctx: &mut ViewContext<Self>) {
        self.is_active = is_active;
        if is_active && self.needs_refresh && !self.is_loading {
            self.refresh(ctx);
        }
        ctx.notify();
    }

    fn select_repository(
        &mut self,
        repository: Option<LocalOrRemotePath>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.generation = self.generation.wrapping_add(1);
        self.selected_repository = repository.clone();
        if let Some(previous_model) = self.git_status_model.take() {
            ctx.unsubscribe_to_model(&previous_model);
        }
        self.snapshot = None;
        self.graph_layout = GraphLayout::default();
        self.error = None;
        self.is_loading = false;
        self.reload_after_current = false;
        self.mutation_in_progress = false;
        self.history_page_in_progress = false;
        self.needs_refresh = repository.is_some();
        self.row_mouse_states.clear();
        self.row_action_mouse_states.clear();
        self.commit_mouse_states.clear();
        self.update_repository_dropdown(ctx);

        let Some(repository) = repository else {
            ctx.notify();
            return;
        };
        if repository.is_remote() {
            ctx.notify();
            return;
        }

        match GitRepoModels::handle(ctx)
            .update(ctx, |models, ctx| models.subscribe(&repository, ctx))
        {
            Ok(model) => {
                let subscribed_repository = repository.clone();
                ctx.subscribe_to_model(&model, move |me, _, event, ctx| match event {
                    GitRepoStatusEvent::MetadataChanged
                        if me.selected_repository.as_ref() == Some(&subscribed_repository) =>
                    {
                        me.request_refresh(ctx);
                    }
                    GitRepoStatusEvent::MetadataChanged => {}
                });
                model.update(ctx, |model, ctx| model.refresh_metadata(ctx));
                self.git_status_model = Some(model);
            }
            Err(err) => {
                log::warn!(
                    "Source control could not subscribe to repository status for {}: {err}",
                    repository.display_path()
                );
            }
        }

        if self.is_active {
            self.refresh(ctx);
        }
        ctx.notify();
    }

    fn clear_repository_state(&mut self, ctx: &mut ViewContext<Self>) {
        self.select_repository(None, ctx);
    }

    fn update_repository_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        let items = self
            .repositories
            .iter()
            .map(|repository| {
                let label = repository
                    .file_name()
                    .filter(|name| !name.is_empty())
                    .unwrap_or("Repository");
                DropdownItem::new(
                    label,
                    SourceControlAction::SelectRepository(repository.clone()),
                )
                .with_tooltip(repository.display_path())
            })
            .collect();
        let selected = self.selected_repository.clone();
        self.repository_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(items, ctx);
            if let Some(selected) = selected {
                dropdown
                    .set_selected_by_action(SourceControlAction::SelectRepository(selected), ctx);
            } else {
                dropdown.set_selected_to_none(ctx);
            }
        });
    }

    fn request_refresh(&mut self, ctx: &mut ViewContext<Self>) {
        self.needs_refresh = true;
        if self.is_loading {
            self.reload_after_current = true;
        } else if self.is_active {
            self.refresh(ctx);
        }
    }

    fn refresh(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_loading {
            self.reload_after_current = true;
            return;
        }
        let Some(LocalOrRemotePath::Local(repo_path)) = self.selected_repository.clone() else {
            return;
        };

        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.is_loading = true;
        self.needs_refresh = false;
        self.reload_after_current = false;
        self.error = None;
        let expected_repository = LocalOrRemotePath::Local(repo_path.clone());
        ctx.spawn(
            async move { load_repository(&repo_path).await },
            move |me, result, ctx| {
                if me.generation != generation
                    || me.selected_repository.as_ref() != Some(&expected_repository)
                {
                    return;
                }
                me.is_loading = false;
                match result {
                    Ok(snapshot) => me.set_snapshot(snapshot),
                    Err(err) => {
                        log::warn!(
                            "Source control refresh failed for {}: {err}",
                            expected_repository.display_path()
                        );
                        me.error = Some(err.to_string());
                    }
                }

                if me.reload_after_current && me.is_active {
                    me.reload_after_current = false;
                    me.refresh(ctx);
                } else {
                    ctx.notify();
                }
            },
        );
        ctx.notify();
    }

    fn set_snapshot(&mut self, snapshot: RepositorySnapshot) {
        self.graph_layout = layout_commits(&snapshot.commits);
        for (section, changes) in [
            (ChangeSection::Merge, &snapshot.merge_changes),
            (ChangeSection::Staged, &snapshot.staged_changes),
            (ChangeSection::Changes, &snapshot.changes),
            (ChangeSection::Untracked, &snapshot.untracked_changes),
        ] {
            for change in changes {
                let key = (section, change.path.clone());
                self.row_mouse_states.entry(key.clone()).or_default();
                self.row_action_mouse_states.entry(key).or_default();
            }
        }
        for commit in &snapshot.commits {
            self.commit_mouse_states
                .entry(commit.hash.clone())
                .or_default();
        }
        self.snapshot = Some(snapshot);
    }

    fn run_mutation(&mut self, mutation: GitMutation, ctx: &mut ViewContext<Self>) {
        if self.mutation_in_progress {
            return;
        }
        let Some(LocalOrRemotePath::Local(repo_path)) = self.selected_repository.clone() else {
            return;
        };
        let has_head = self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.has_head);
        let generation = self.generation;
        let expected_repository = LocalOrRemotePath::Local(repo_path.clone());
        self.mutation_in_progress = true;
        self.error = None;
        ctx.spawn(
            async move {
                apply_mutation(&repo_path, &mutation, has_head)
                    .await
                    .map_err(|err| (mutation, err))
            },
            move |me, result, ctx| {
                if me.generation != generation
                    || me.selected_repository.as_ref() != Some(&expected_repository)
                {
                    return;
                }
                me.mutation_in_progress = false;
                match result {
                    Ok(()) => {
                        if let Some(model) = &me.git_status_model {
                            model.update(ctx, |model, ctx| model.refresh_metadata(ctx));
                        }
                        me.request_refresh(ctx);
                    }
                    Err((mutation, err)) => {
                        log::warn!(
                            "Source control failed to {} in {}: {err}",
                            mutation.label(),
                            expected_repository.display_path()
                        );
                        me.error = Some(format!("Unable to {}: {err}", mutation.label()));
                        ctx.notify();
                    }
                }
            },
        );
        ctx.notify();
    }

    fn load_more(&mut self, ctx: &mut ViewContext<Self>) {
        if self.history_page_in_progress {
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        if !snapshot.has_more_history {
            return;
        }
        let Some(LocalOrRemotePath::Local(repo_path)) = self.selected_repository.clone() else {
            return;
        };
        let generation = self.generation;
        let base_len = snapshot.commits.len();
        let has_head = snapshot.has_head;
        let expected_repository = LocalOrRemotePath::Local(repo_path.clone());
        self.history_page_in_progress = true;
        ctx.spawn(
            async move { load_history(&repo_path, base_len, HISTORY_PAGE_SIZE, has_head).await },
            move |me, result, ctx| {
                if me.generation != generation
                    || me.selected_repository.as_ref() != Some(&expected_repository)
                {
                    return;
                }
                me.history_page_in_progress = false;
                match result {
                    Ok((commits, has_more)) => {
                        if me
                            .snapshot
                            .as_ref()
                            .is_some_and(|snapshot| snapshot.commits.len() == base_len)
                        {
                            for commit in &commits {
                                me.commit_mouse_states
                                    .entry(commit.hash.clone())
                                    .or_default();
                            }
                            if let Some(snapshot) = &mut me.snapshot {
                                snapshot.commits.extend(commits);
                                snapshot.has_more_history = has_more;
                                me.graph_layout = layout_commits(&snapshot.commits);
                            }
                        }
                    }
                    Err(err) => {
                        log::warn!(
                            "Source control history pagination failed for {}: {err}",
                            expected_repository.display_path()
                        );
                        me.error = Some(err.to_string());
                    }
                }
                ctx.notify();
            },
        );
        ctx.notify();
    }

    fn branch_summary(&self, app: &AppContext) -> Option<String> {
        let metadata = self
            .git_status_model
            .as_ref()
            .and_then(|model| model.as_ref(app).metadata(app))?;
        let tracking = &metadata.branch_tracking_status;
        let mut summary = metadata.current_branch_name.clone();
        if tracking.counts_available {
            if tracking.behind > 0 {
                summary.push_str(&format!("  ↓{}", tracking.behind));
            }
            if tracking.ahead > 0 {
                summary.push_str(&format!("  ↑{}", tracking.ahead));
            }
        }
        Some(summary)
    }

    fn render_tooltip(appearance: &Appearance, label: impl Into<String>) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .clone()
            .tool_tip(label.into())
            .build()
            .finish()
    }

    fn render_icon_action(
        &self,
        icon: Icon,
        tooltip: &'static str,
        mouse_state: MouseStateHandle,
        action: SourceControlAction,
        disabled: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let tooltip_element = Self::render_tooltip(appearance, tooltip);
        let mut button =
            icon_button(appearance, icon, false, mouse_state).with_tooltip(move || tooltip_element);
        if disabled {
            button = button.disabled();
        }
        button
            .with_style(UiComponentStyles {
                width: Some(22.),
                height: Some(22.),
                ..Default::default()
            })
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
            .with_cursor(Cursor::PointingHand)
            .finish()
    }

    fn render_header(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();
        let main_color = theme.main_text_color(theme.background()).into_solid();
        let sub_color = theme.sub_text_color(theme.background()).into_solid();
        let title = Text::new_inline(
            "SOURCE CONTROL",
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(main_color)
        .finish();
        let refresh = self.render_icon_action(
            Icon::Refresh,
            "Refresh source control",
            self.static_mouse_states.refresh.clone(),
            SourceControlAction::Refresh,
            self.is_loading || self.mutation_in_progress,
            appearance,
        );
        let mut header = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(6.)
            .with_child(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(title)
                    .with_child(refresh)
                    .finish(),
            );
        if self.repositories.len() > 1 {
            header.add_child(ChildView::new(&self.repository_dropdown).finish());
        }
        if let Some(branch) = self.branch_summary(app) {
            header.add_child(
                Text::new_inline(
                    branch,
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(sub_color)
                .finish(),
            );
        }
        Container::new(header.finish())
            .with_padding_left(10.)
            .with_padding_right(8.)
            .with_padding_top(8.)
            .with_padding_bottom(8.)
            .finish()
    }

    fn render_message(
        &self,
        title: impl Into<String>,
        detail: impl Into<String>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let title = Text::new(
            title.into(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(theme.main_text_color(theme.background()).into_solid())
        .finish();
        let detail = Text::new(
            detail.into(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(theme.sub_text_color(theme.background()).into_solid())
        .soft_wrap(true)
        .finish();
        Container::new(
            Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(5.)
                .with_child(title)
                .with_child(detail)
                .finish(),
        )
        .with_padding_left(12.)
        .with_padding_right(12.)
        .with_padding_top(18.)
        .finish()
    }

    fn render_error(&self, error: &str, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Container::new(
            Text::new(
                error.to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(theme.ui_error_color())
            .soft_wrap(true)
            .finish(),
        )
        .with_background(theme.surface_1())
        .with_border(Border::all(1.).with_border_fill(theme.surface_3()))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(5.)))
        .with_padding_left(8.)
        .with_padding_right(8.)
        .with_padding_top(6.)
        .with_padding_bottom(6.)
        .with_margin_left(8.)
        .with_margin_right(8.)
        .with_margin_bottom(6.)
        .finish()
    }

    fn displayed_refs(commit: &CommitNode) -> Vec<&GitRefLabel> {
        let mut refs = Vec::new();
        for (index, label) in commit.refs.iter().enumerate() {
            let redundant_head = matches!(&label.kind, GitRefKind::Head)
                && commit
                    .refs
                    .get(index + 1)
                    .is_some_and(|next| matches!(&next.kind, GitRefKind::LocalBranch));
            if !redundant_head {
                refs.push(label);
            }
        }
        refs.sort_by_key(|label| match &label.kind {
            GitRefKind::Head | GitRefKind::LocalBranch => 0,
            GitRefKind::RemoteBranch => 1,
            GitRefKind::Tag => 2,
            GitRefKind::Other => 3,
        });
        refs
    }

    fn render_ref_badge(
        label: impl Into<String>,
        is_primary: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let background = if is_primary {
            theme.accent().into_solid()
        } else {
            internal_colors::fg_overlay_3(theme).into_solid()
        };
        let text_color = if is_primary {
            theme.background().into_solid()
        } else {
            theme.sub_text_color(theme.background()).into_solid()
        };
        let text = Text::new_inline(
            label.into(),
            appearance.ui_font_family(),
            (appearance.ui_font_size() - 2.).max(1.),
        )
        .with_clip(ClipConfig::ellipsis())
        .with_color(text_color)
        .finish();
        ConstrainedBox::new(
            Container::new(text)
                .with_background(background)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(9.)))
                .with_horizontal_padding(7.)
                .with_vertical_padding(1.5)
                .finish(),
        )
        .with_max_width(120.)
        .finish()
    }

    fn render_commit_details(
        &self,
        commit: &CommitNode,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let main_color = theme.main_text_color(theme.surface_1()).into_solid();
        let sub_color = theme.sub_text_color(theme.surface_1()).into_solid();
        let absolute_time = absolute_time_string(commit.timestamp);
        let time_label = relative_time_string(commit.timestamp)
            .map(|relative| format!("  {relative} ({absolute_time})"))
            .unwrap_or_else(|| format!("  {absolute_time}"));
        let header = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.,
                    Text::new_inline(
                        commit.author.clone(),
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_clip(ClipConfig::ellipsis())
                    .with_color(main_color)
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                Text::new_inline(
                    time_label,
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(sub_color)
                .finish(),
            )
            .finish();

        let mut message = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(4.)
            .with_child(
                Text::new(
                    commit.subject.clone(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(main_color)
                .soft_wrap(true)
                .finish(),
            );
        if !commit.body.is_empty() {
            message.add_child(
                Text::new(
                    commit.body.clone(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(sub_color)
                .soft_wrap(true)
                .finish(),
            );
        }

        let mut content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(6.)
            .with_child(header)
            .with_child(message.finish());

        if let Some(stats) = &commit.stats {
            let file_label = format!(
                "{} file{} changed",
                stats.files_changed,
                if stats.files_changed == 1 { "" } else { "s" }
            );
            let mut stats_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Text::new_inline(
                        file_label,
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(sub_color)
                    .finish(),
                );
            if stats.insertions > 0 {
                stats_row.add_child(
                    Text::new_inline(
                        format!("  {} insertions(+)", stats.insertions),
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(add_color(appearance))
                    .finish(),
                );
            }
            if stats.deletions > 0 {
                stats_row.add_child(
                    Text::new_inline(
                        format!("  {} deletions(-)", stats.deletions),
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(remove_color(appearance))
                    .finish(),
                );
            }
            content.add_child(stats_row.finish());
        }

        let refs = Self::displayed_refs(commit);
        if !refs.is_empty() {
            let mut ref_row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(4.);
            for label in refs.into_iter().take(6) {
                let is_primary = matches!(&label.kind, GitRefKind::Head | GitRefKind::LocalBranch);
                ref_row.add_child(Self::render_ref_badge(
                    label.name.clone(),
                    is_primary,
                    appearance,
                ));
            }
            content.add_child(ref_row.finish());
        }

        content.add_child(
            Text::new_inline(
                commit.short_hash().to_string(),
                appearance.ui_font_family(),
                appearance.ui_font_size(),
            )
            .with_color(sub_color)
            .finish(),
        );

        ConstrainedBox::new(
            Container::new(content.finish())
                .with_background(theme.surface_1())
                .with_border(Border::all(1.).with_border_fill(theme.surface_3()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                .with_horizontal_padding(11.)
                .with_vertical_padding(10.)
                .finish(),
        )
        .with_max_width(420.)
        .finish()
    }

    fn render_section(
        &self,
        section: ChangeSection,
        changes: &[FileChange],
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        if changes.is_empty() {
            return None;
        }
        let theme = appearance.theme();
        let main_color = theme.main_text_color(theme.background()).into_solid();
        let sub_color = theme.sub_text_color(theme.background()).into_solid();
        let collapsed = self
            .collapsed_sections
            .get(&section)
            .copied()
            .unwrap_or(false);
        let section_mouse_state = self.section_mouse_states.get(&section)?.clone();
        let section_action_mouse_state = self.section_action_mouse_states.get(&section)?.clone();
        let chevron = if collapsed {
            Icon::ChevronRight
        } else {
            Icon::ChevronDown
        };
        let header_left = Hoverable::new(section_mouse_state, |_| {
            Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(4.)
                    .with_child(
                        ConstrainedBox::new(chevron.to_warpui_icon(sub_color.into()).finish())
                            .with_width(appearance.ui_font_size())
                            .with_height(appearance.ui_font_size())
                            .finish(),
                    )
                    .with_child(
                        Text::new_inline(
                            format!("{} ({})", section.title(), changes.len()),
                            appearance.ui_font_family(),
                            appearance.ui_font_size(),
                        )
                        .with_color(main_color)
                        .finish(),
                    )
                    .finish(),
            )
            .with_padding_left(6.)
            .with_padding_top(5.)
            .with_padding_bottom(5.)
            .finish()
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(SourceControlAction::ToggleSection(section));
        })
        .with_cursor(Cursor::PointingHand)
        .finish();
        let (bulk_icon, bulk_tooltip, bulk_action) = match section {
            ChangeSection::Staged => (
                Icon::Minus,
                "Unstage all changes",
                SourceControlAction::UnstageAll,
            ),
            ChangeSection::Changes | ChangeSection::Untracked => (
                Icon::Plus,
                "Stage all changes",
                SourceControlAction::StageAll,
            ),
            ChangeSection::Merge => (
                Icon::Diff,
                "Review merge changes",
                SourceControlAction::OpenCodeReview,
            ),
        };
        let header = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., header_left).finish())
            .with_child(self.render_icon_action(
                bulk_icon,
                bulk_tooltip,
                section_action_mouse_state,
                bulk_action,
                self.mutation_in_progress,
                appearance,
            ))
            .finish();
        let mut section_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header);
        if !collapsed {
            for change in changes {
                section_column.add_child(self.render_file_row(section, change, appearance));
            }
        }
        Some(section_column.finish())
    }

    fn render_file_row(
        &self,
        section: ChangeSection,
        change: &FileChange,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let main_color = theme.main_text_color(theme.background()).into_solid();
        let sub_color = theme.sub_text_color(theme.background()).into_solid();
        let key = (section, change.path.clone());
        let row_state = self
            .row_mouse_states
            .get(&key)
            .cloned()
            .expect("file rows are assigned persistent mouse state when a snapshot is applied");
        let action_state =
            self.row_action_mouse_states.get(&key).cloned().expect(
                "file actions are assigned persistent mouse state when a snapshot is applied",
            );
        let path = Path::new(&change.path);
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&change.path)
            .to_string();
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.to_string_lossy().to_string());
        let mut label = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(5.)
            .with_child(
                Text::new_inline(
                    file_name,
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_clip(ClipConfig::ellipsis())
                .with_color(main_color)
                .finish(),
            );
        if let Some(parent) = parent {
            label.add_child(
                Text::new_inline(
                    parent,
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_clip(ClipConfig::ellipsis())
                .with_color(sub_color)
                .finish(),
            );
        }
        if let Some(old_path) = change.kind.previous_path() {
            label.add_child(
                Text::new_inline(
                    format!("← {old_path}"),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_clip(ClipConfig::ellipsis())
                .with_color(sub_color)
                .finish(),
            );
        }
        let clickable = Container::new(label.finish())
            .with_padding_left(22.)
            .with_padding_top(4.)
            .with_padding_bottom(4.)
            .finish();
        let status_color = match change.kind {
            GitChangeKind::Added | GitChangeKind::Untracked => add_color(appearance),
            GitChangeKind::Deleted => remove_color(appearance),
            GitChangeKind::Conflicted => theme.ui_error_color(),
            GitChangeKind::Modified
            | GitChangeKind::Renamed { .. }
            | GitChangeKind::Copied { .. } => theme.accent().into_solid(),
        };
        let status = Text::new_inline(
            change.kind.status_letter(),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(status_color)
        .finish();
        let action = match section {
            ChangeSection::Staged => Some((
                Icon::Minus,
                "Unstage file",
                SourceControlAction::UnstagePaths(change.kind.paths_for_action(&change.path)),
            )),
            ChangeSection::Changes | ChangeSection::Untracked => Some((
                Icon::Plus,
                "Stage file",
                SourceControlAction::StagePaths(change.kind.paths_for_action(&change.path)),
            )),
            ChangeSection::Merge => None,
        };
        let mut trailing = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(2.);
        if let Some((icon, tooltip, action)) = action {
            trailing.add_child(self.render_icon_action(
                icon,
                tooltip,
                action_state,
                action,
                self.mutation_in_progress,
                appearance,
            ));
        }
        trailing.add_child(status);

        let row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., clickable).finish())
            .with_child(
                Container::new(trailing.finish())
                    .with_padding_right(8.)
                    .finish(),
            )
            .finish();
        Hoverable::new(row_state, |state| {
            let mut row = Container::new(row);
            if state.is_hovered() {
                row = row.with_background(internal_colors::fg_overlay_3(theme));
            }
            row.finish()
        })
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(SourceControlAction::OpenCodeReview);
        })
        .with_defer_events_to_children()
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    fn render_history_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let main_color = theme.main_text_color(theme.background()).into_solid();
        let sub_color = theme.sub_text_color(theme.background()).into_solid();
        let chevron = if self.history_collapsed {
            Icon::ChevronRight
        } else {
            Icon::ChevronDown
        };
        let history_header = Hoverable::new(self.static_mouse_states.history.clone(), |_| {
            Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(4.)
                    .with_child(
                        ConstrainedBox::new(chevron.to_warpui_icon(sub_color.into()).finish())
                            .with_width(appearance.ui_font_size())
                            .with_height(appearance.ui_font_size())
                            .finish(),
                    )
                    .with_child(
                        Text::new_inline(
                            "HISTORY",
                            appearance.ui_font_family(),
                            appearance.ui_font_size(),
                        )
                        .with_color(main_color)
                        .finish(),
                    )
                    .finish(),
            )
            .with_padding_left(6.)
            .with_padding_top(7.)
            .with_padding_bottom(7.)
            .finish()
        })
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(SourceControlAction::ToggleHistory))
        .with_cursor(Cursor::PointingHand)
        .finish();

        Container::new(history_header)
            .with_border(Border::top(1.).with_border_fill(theme.surface_3()))
            .finish()
    }

    fn render_history_content(
        &self,
        snapshot: &RepositorySnapshot,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let main_color = theme.main_text_color(theme.background()).into_solid();
        let sub_color = theme.sub_text_color(theme.background()).into_solid();
        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        if snapshot.commits.is_empty() {
            column.add_child(
                Container::new(
                    Text::new_inline(
                        "No commits yet",
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(sub_color)
                    .finish(),
                )
                .with_padding_left(22.)
                .with_padding_bottom(8.)
                .finish(),
            );
            return column.finish();
        }

        for (index, commit) in snapshot.commits.iter().enumerate() {
            let Some(graph_row) = self.graph_layout.rows.get(index) else {
                continue;
            };
            let refs = Self::displayed_refs(commit);
            let mut badges = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(4.);
            for label in refs.iter().take(3) {
                let is_primary = matches!(&label.kind, GitRefKind::Head | GitRefKind::LocalBranch);
                badges.add_child(Self::render_ref_badge(
                    label.name.clone(),
                    is_primary,
                    appearance,
                ));
            }
            if refs.len() > 3 {
                badges.add_child(Self::render_ref_badge(
                    format!("+{}", refs.len() - 3),
                    false,
                    appearance,
                ));
            }

            let row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Box::new(GraphRowCanvas::new(
                    graph_row.clone(),
                    self.graph_layout.max_lanes,
                    theme.accent().into_solid(),
                )))
                .with_child(
                    Expanded::new(
                        1.,
                        Flex::row()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_spacing(6.)
                            .with_child(
                                Shrinkable::new(
                                    1.,
                                    Text::new_inline(
                                        commit.subject.clone(),
                                        appearance.ui_font_family(),
                                        appearance.ui_font_size(),
                                    )
                                    .with_clip(ClipConfig::ellipsis())
                                    .with_color(main_color)
                                    .finish(),
                                )
                                .finish(),
                            )
                            .with_child(badges.finish())
                            .finish(),
                    )
                    .finish(),
                )
                .finish();
            let details_card = self.render_commit_details(commit, appearance);
            let mouse_state = self.commit_mouse_states.get(&commit.hash).cloned().expect(
                "commit rows are assigned persistent mouse state when a snapshot is applied",
            );
            let hoverable = Hoverable::new(mouse_state, move |state| {
                let mut row = Container::new(row)
                    .with_padding_left(8.)
                    .with_padding_right(8.);
                if state.is_hovered() {
                    row = row.with_background(internal_colors::fg_overlay_3(theme));
                }
                let row = ConstrainedBox::new(row.finish()).with_height(24.).finish();
                if state.is_hovered() {
                    let mut stack = Stack::new();
                    stack.add_child(row);
                    stack.add_positioned_overlay_child(
                        details_card,
                        OffsetPositioning::offset_from_parent(
                            vec2f(8., 0.),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::TopRight,
                            ChildAnchor::TopLeft,
                        ),
                    );
                    stack.finish()
                } else {
                    row
                }
            })
            .finish();
            column.add_child(hoverable);
        }
        if snapshot.has_more_history {
            column.add_child(
                Container::new(self.render_icon_action(
                    Icon::ChevronDown,
                    "Load more history",
                    self.static_mouse_states.load_more.clone(),
                    SourceControlAction::LoadMore,
                    self.history_page_in_progress,
                    appearance,
                ))
                .with_margin_left(8.)
                .with_margin_top(4.)
                .with_margin_bottom(8.)
                .finish(),
            );
        }
        column.finish()
    }

    fn render_changes_content(
        &self,
        snapshot: &RepositorySnapshot,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        if !snapshot.has_changes() {
            content.add_child(self.render_message(
                "No changes",
                "The working tree is clean.",
                appearance,
            ));
        } else {
            for (section, changes) in [
                (ChangeSection::Merge, snapshot.merge_changes.as_slice()),
                (ChangeSection::Staged, snapshot.staged_changes.as_slice()),
                (ChangeSection::Changes, snapshot.changes.as_slice()),
                (
                    ChangeSection::Untracked,
                    snapshot.untracked_changes.as_slice(),
                ),
            ] {
                if let Some(section) = self.render_section(section, changes, appearance) {
                    content.add_child(section);
                }
            }
        }
        content.finish()
    }

    fn render_changes_region(
        &self,
        snapshot: &RepositorySnapshot,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let changes = ClippedScrollable::vertical(
            self.changes_scroll_state.clone(),
            self.render_changes_content(snapshot, appearance),
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .finish();
        let mut region = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        if let Some(error) = &self.error {
            region.add_child(self.render_error(error, appearance));
        }
        region.add_child(Shrinkable::new(1., changes).finish());
        region.finish()
    }

    fn render_history_region(
        &self,
        snapshot: &RepositorySnapshot,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let history = ClippedScrollable::vertical(
            self.history_scroll_state.clone(),
            self.render_history_content(snapshot, appearance),
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .finish();
        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(self.render_history_header(appearance))
            .with_child(Shrinkable::new(1., history).finish())
            .finish()
    }

    fn render_single_scrollable(
        &self,
        body: Box<dyn Element>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        ClippedScrollable::vertical(
            self.changes_scroll_state.clone(),
            body,
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .finish()
    }
}

impl Entity for SourceControlView {
    type Event = SourceControlEvent;
}

impl TypedActionView for SourceControlView {
    type Action = SourceControlAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SourceControlAction::SelectRepository(repository) => {
                if self.repositories.contains(repository) {
                    self.select_repository(Some(repository.clone()), ctx);
                }
            }
            SourceControlAction::Refresh => self.request_refresh(ctx),
            SourceControlAction::ToggleSection(section) => {
                let collapsed = self.collapsed_sections.entry(*section).or_default();
                *collapsed = !*collapsed;
                ctx.notify();
            }
            SourceControlAction::ToggleHistory => {
                self.history_collapsed = !self.history_collapsed;
                ctx.notify();
            }
            SourceControlAction::OpenCodeReview => {
                if let Some(repo_path) = self.selected_repository.clone() {
                    ctx.emit(SourceControlEvent::OpenCodeReview { repo_path });
                }
            }
            SourceControlAction::StagePaths(paths) => {
                self.run_mutation(GitMutation::StagePaths(paths.clone()), ctx)
            }
            SourceControlAction::UnstagePaths(paths) => {
                self.run_mutation(GitMutation::UnstagePaths(paths.clone()), ctx)
            }
            SourceControlAction::StageAll => self.run_mutation(GitMutation::StageAll, ctx),
            SourceControlAction::UnstageAll => self.run_mutation(GitMutation::UnstageAll, ctx),
            SourceControlAction::LoadMore => self.load_more(ctx),
        }
    }
}

impl View for SourceControlView {
    fn ui_name() -> &'static str {
        "SourceControlView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut root = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(self.render_header(appearance, app));

        let local_snapshot = match (&self.selected_repository, &self.snapshot) {
            (Some(LocalOrRemotePath::Local(_)), Some(snapshot)) => Some(snapshot),
            _ => None,
        };
        if let Some(snapshot) = local_snapshot {
            let changes_region = self.render_changes_region(snapshot, appearance);
            if self.history_collapsed {
                root.add_child(
                    Shrinkable::new(
                        1.,
                        Flex::column()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                            .with_child(Shrinkable::new(1., changes_region).finish())
                            .with_child(self.render_history_header(appearance))
                            .finish(),
                    )
                    .finish(),
                );
            } else {
                let bounds_height = self.split_area_height.clone();
                let reset_height = self.split_area_height.clone();
                let reset_state = self.split_state.clone();
                let changes_region = Resizable::new(self.split_state.clone(), changes_region)
                    .with_dragbar_side(DragBarSide::Bottom)
                    .on_resize(|ctx, _| ctx.notify())
                    .with_bounds_callback(Box::new(move |_| {
                        let measured = *bounds_height
                            .lock()
                            .expect("split area height should be accessible");
                        if measured <= 0. {
                            // Preserve the initial size until the split area is measured after layout.
                            (120., f32::MAX)
                        } else if measured <= 240. {
                            let half = measured / 2.;
                            (half, half)
                        } else {
                            (120., measured - 120.)
                        }
                    }))
                    .on_dragbar_double_click(move |ctx, _| {
                        let measured = *reset_height
                            .lock()
                            .expect("split area height should be accessible");
                        reset_state
                            .lock()
                            .expect("source control split state should be accessible")
                            .set_size(measured / 2.);
                        ctx.notify();
                    })
                    .finish();
                let split = Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(changes_region)
                    .with_child(
                        Shrinkable::new(1., self.render_history_region(snapshot, appearance))
                            .finish(),
                    )
                    .finish();
                root.add_child(
                    Shrinkable::new(
                        1.,
                        MeasuredHeight::new(self.split_area_height.clone(), split).finish(),
                    )
                    .finish(),
                );
            }
        } else {
            let body = match &self.selected_repository {
                None => self.render_message(
                    "No repository",
                    "Open or enter a local Git repository to view source control.",
                    appearance,
                ),
                Some(LocalOrRemotePath::Remote(_)) => self.render_message(
                    "Remote repository",
                    "Source-control operations for remote sessions are not supported yet.",
                    appearance,
                ),
                Some(LocalOrRemotePath::Local(_)) => self.render_message(
                    if self.is_loading {
                        "Loading…"
                    } else {
                        "Git unavailable"
                    },
                    self.error
                        .as_deref()
                        .unwrap_or("Refresh to load repository state."),
                    appearance,
                ),
            };
            root.add_child(
                Shrinkable::new(1., self.render_single_scrollable(body, appearance)).finish(),
            );
        }
        root.finish()
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
