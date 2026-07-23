use std::collections::HashMap;

use chrono::{Local, TimeZone};
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::Icon;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, Element, Expanded, Flex, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, ScrollbarWidth, Shrinkable, Stack, Text,
};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use super::data::{
    load_history, load_repository, CommitNode, GitRefKind, GitRefLabel, RepositorySnapshot,
    HISTORY_PAGE_SIZE,
};
use super::layout::{layout_commits, GraphLayout};
use super::row_canvas::GraphRowCanvas;
use crate::appearance::Appearance;
use crate::code::buffer_location::LocalOrRemotePath;
use crate::code::editor::{add_color, remove_color};
use crate::code_review::git_repo_model::{GitRepoModels, GitRepoStatusEvent, GitRepoStatusModel};
use crate::ui_components::buttons::icon_button;
use crate::view_components::dropdown::{Dropdown, DropdownItem};

#[derive(Clone, Debug, PartialEq)]
pub enum SourceControlAction {
    SelectRepository(LocalOrRemotePath),
    Refresh,
    LoadMore,
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
    load_more: MouseStateHandle,
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
    history_page_in_progress: bool,
    generation: u64,
    scroll_state: ClippedScrollStateHandle,
    static_mouse_states: StaticMouseStates,
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
            history_page_in_progress: false,
            generation: 0,
            scroll_state: ClippedScrollStateHandle::default(),
            static_mouse_states: StaticMouseStates::default(),
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
        self.history_page_in_progress = false;
        self.needs_refresh = repository.is_some();
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
        for commit in &snapshot.commits {
            self.commit_mouse_states
                .entry(commit.hash.clone())
                .or_default();
        }
        self.snapshot = Some(snapshot);
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
            self.is_loading,
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
    fn render_history_region(
        &self,
        snapshot: &RepositorySnapshot,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let history = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            self.render_history_content(snapshot, appearance),
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .finish();
        Container::new(history)
            .with_border(Border::top(1.).with_border_fill(theme.surface_3()))
            .finish()
    }

    fn render_single_scrollable(
        &self,
        body: Box<dyn Element>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        ClippedScrollable::vertical(
            self.scroll_state.clone(),
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
    type Event = ();
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
            if let Some(error) = &self.error {
                root.add_child(self.render_error(error, appearance));
            }
            root.add_child(
                Shrinkable::new(1., self.render_history_region(snapshot, appearance)).finish(),
            );
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
