use std::sync::Arc;

use warp_core::ui::icons::Icon;
use warp_core::ui::theme::AnsiColorIdentifier;
use warpui::elements::{
    ChildView, Container, CrossAxisAlignment, Element, Empty, Flex, MainAxisSize, ParentElement,
    Text, Wrap,
};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use super::{file_button_label, Artifact};
use crate::appearance::Appearance;
use crate::notebooks::NotebookId;
use crate::terminal::input::MenuPositioning;
use crate::ui_components::blended_colors;
use crate::view_components::action_button::{
    ActionButton, ActionButtonTheme, ButtonSize, SecondaryTheme, TooltipAlignment,
};

const BUTTON_SPACING: f32 = 8.;
const BUTTON_MAX_TEXT_WIDTH: f32 = 200.;

/// Maximum number of artifact pills rendered before collapsing the remainder
/// into a single "+N more" indicator, so an orchestration tree with many
/// artifacts (e.g. 100 PRs) doesn't flood the row.
const MAX_VISIBLE_ARTIFACT_BUTTONS: usize = 25;

/// A view that renders a set of artifact buttons (plans, branches, PRs)
pub struct ArtifactButtonsRow {
    artifacts: Vec<Artifact>,
    buttons: Vec<ViewHandle<ActionButton>>,
    theme: Arc<dyn ActionButtonTheme>,
}

impl ArtifactButtonsRow {
    pub fn new(artifacts: &[Artifact], ctx: &mut ViewContext<Self>) -> Self {
        let theme: Arc<dyn ActionButtonTheme> = Arc::new(SecondaryTheme);
        Self {
            artifacts: artifacts.to_vec(),
            buttons: collect_buttons(artifacts, &theme, ctx),
            theme,
        }
    }

    pub fn with_theme(
        artifacts: &[Artifact],
        theme: Arc<dyn ActionButtonTheme>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self {
            artifacts: artifacts.to_vec(),
            buttons: collect_buttons(artifacts, &theme, ctx),
            theme,
        }
    }

    /// Rebuilds the buttons for `artifacts`. No-ops when the artifacts are
    /// unchanged, so redundant refreshes (e.g. several children reporting
    /// into the same ancestor card) skip the rebuild and re-render.
    pub fn update_artifacts(&mut self, artifacts: &[Artifact], ctx: &mut ViewContext<Self>) {
        if self.artifacts == artifacts {
            return;
        }
        self.artifacts = artifacts.to_vec();
        self.buttons = collect_buttons(artifacts, &self.theme, ctx);
        ctx.notify();
    }

    pub fn is_empty(&self) -> bool {
        self.buttons.is_empty()
    }
}

pub enum ArtifactButtonsRowEvent {
    OpenPlan { notebook_uid: NotebookId },
    CopyBranch { branch: String },
    OpenPullRequest { url: String },
    ViewScreenshots { artifact_uids: Vec<String> },
    DownloadFile { artifact_uid: String },
}

#[derive(Debug, Clone)]
pub enum ArtifactButtonAction {
    OpenPlan { notebook_uid: NotebookId },
    CopyBranch { branch: String },
    OpenPullRequest { url: String },
    ViewScreenshots { artifact_uids: Vec<String> },
    DownloadFile { artifact_uid: String },
}

impl Entity for ArtifactButtonsRow {
    type Event = ArtifactButtonsRowEvent;
}

impl View for ArtifactButtonsRow {
    fn ui_name() -> &'static str {
        "ArtifactButtonsRow"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if self.buttons.is_empty() {
            return Empty::new().finish();
        }

        let pills = Wrap::row()
            .with_spacing(BUTTON_SPACING)
            .with_run_spacing(BUTTON_SPACING)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_children(
                self.buttons
                    .iter()
                    .take(MAX_VISIBLE_ARTIFACT_BUTTONS)
                    .map(|button| ChildView::new(button).finish()),
            )
            .finish();

        let hidden_count = self
            .buttons
            .len()
            .saturating_sub(MAX_VISIBLE_ARTIFACT_BUTTONS);
        if hidden_count == 0 {
            return pills;
        }

        // Plain, muted text on its own line below the pills — deliberately not a
        // button/chip, so it has no hover or click affordance.
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let more_label = Text::new(
            format!("+{hidden_count} more"),
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(blended_colors::text_sub(theme, theme.surface_1()))
        .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(pills)
            .with_child(
                Container::new(more_label)
                    .with_margin_top(BUTTON_SPACING)
                    .finish(),
            )
            .finish()
    }
}

impl TypedActionView for ArtifactButtonsRow {
    type Action = ArtifactButtonAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        let event = match action {
            ArtifactButtonAction::OpenPlan { notebook_uid } => ArtifactButtonsRowEvent::OpenPlan {
                notebook_uid: *notebook_uid,
            },
            ArtifactButtonAction::CopyBranch { branch } => ArtifactButtonsRowEvent::CopyBranch {
                branch: branch.clone(),
            },
            ArtifactButtonAction::OpenPullRequest { url } => {
                ArtifactButtonsRowEvent::OpenPullRequest { url: url.clone() }
            }
            ArtifactButtonAction::ViewScreenshots { artifact_uids } => {
                ArtifactButtonsRowEvent::ViewScreenshots {
                    artifact_uids: artifact_uids.clone(),
                }
            }
            ArtifactButtonAction::DownloadFile { artifact_uid } => {
                ArtifactButtonsRowEvent::DownloadFile {
                    artifact_uid: artifact_uid.clone(),
                }
            }
        };

        ctx.emit(event);
    }
}

fn collect_buttons(
    artifacts: &[Artifact],
    theme: &Arc<dyn ActionButtonTheme>,
    ctx: &mut ViewContext<ArtifactButtonsRow>,
) -> Vec<ViewHandle<ActionButton>> {
    let mut buttons = Vec::new();
    let mut screenshot_uids = Vec::new();

    for artifact in artifacts {
        match artifact {
            Artifact::Plan {
                title,
                notebook_uid,
                document_uid: _,
            } => {
                // Only show plan button if synced to Warp Drive (has notebook_uid)
                if let Some(notebook_uid) = notebook_uid {
                    let button_text = title.clone().unwrap_or("Untitled Plan".to_string());
                    let theme = theme.clone();
                    buttons.push(ctx.add_typed_action_view(move |_| {
                        make_plan_button(button_text, *notebook_uid, theme)
                    }));
                }
            }
            Artifact::PullRequest {
                url,
                branch,
                repo,
                number,
            } => {
                if !branch.is_empty() {
                    let theme = theme.clone();
                    buttons.push(
                        ctx.add_typed_action_view(move |_| {
                            make_branch_button(branch.clone(), theme)
                        }),
                    );
                }

                if !url.is_empty() {
                    let theme = theme.clone();
                    buttons.push(ctx.add_typed_action_view(move |_| {
                        make_pr_button(url.clone(), repo.clone(), *number, theme)
                    }));
                }
            }
            Artifact::Screenshot {
                artifact_uid,
                mime_type: _,
                description: _,
            } => {
                screenshot_uids.push(artifact_uid.clone());
            }
            Artifact::File {
                artifact_uid,
                filepath,
                filename,
                ..
            } => {
                let button_text = file_button_label(filename, filepath);
                let theme = theme.clone();
                buttons.push(ctx.add_typed_action_view(move |_| {
                    make_file_button(button_text, artifact_uid.clone(), theme)
                }));
            }
        }
    }

    if !screenshot_uids.is_empty() {
        let theme = theme.clone();
        buttons.push(ctx.add_typed_action_view(move |_| {
            make_screenshot_button("Screenshots".to_string(), screenshot_uids, theme)
        }));
    }

    buttons
}

fn make_plan_button(
    title: String,
    notebook_uid: NotebookId,
    theme: Arc<dyn ActionButtonTheme>,
) -> ActionButton {
    make_artifact_button(
        title,
        Icon::Compass,
        "Open plan",
        None,
        ArtifactButtonAction::OpenPlan { notebook_uid },
        theme,
    )
}

fn make_branch_button(branch: String, theme: Arc<dyn ActionButtonTheme>) -> ActionButton {
    make_artifact_button(
        branch.clone(),
        Icon::GitBranch,
        "Copy branch name",
        Some(AnsiColorIdentifier::Green),
        ArtifactButtonAction::CopyBranch { branch },
        theme,
    )
}

fn make_pr_button(
    url: String,
    repo: Option<String>,
    number: Option<u32>,
    theme: Arc<dyn ActionButtonTheme>,
) -> ActionButton {
    let display_text = match (repo, number) {
        (Some(repo), Some(num)) => format!("{repo} #{num}"),
        // When we deserialize, we either get both values or neither, hence the
        // wildcard match here.
        _ => String::from("PR"),
    };
    make_artifact_button(
        display_text,
        Icon::Github,
        "Open pull request",
        None,
        ArtifactButtonAction::OpenPullRequest { url },
        theme,
    )
}

fn make_screenshot_button(
    label: String,
    artifact_uids: Vec<String>,
    theme: Arc<dyn ActionButtonTheme>,
) -> ActionButton {
    make_artifact_button(
        label,
        Icon::Image,
        "View screenshots",
        None,
        ArtifactButtonAction::ViewScreenshots { artifact_uids },
        theme,
    )
}

fn make_file_button(
    label: String,
    artifact_uid: String,
    theme: Arc<dyn ActionButtonTheme>,
) -> ActionButton {
    make_artifact_button(
        label,
        Icon::File,
        "Download file",
        None,
        ArtifactButtonAction::DownloadFile { artifact_uid },
        theme,
    )
}

fn make_artifact_button(
    display_text: String,
    icon: Icon,
    tooltip: &str,
    icon_color: Option<AnsiColorIdentifier>,
    action: ArtifactButtonAction,
    theme: Arc<dyn ActionButtonTheme>,
) -> ActionButton {
    let mut button = ActionButton::new_with_boxed_theme(display_text, theme)
        .with_size(ButtonSize::Small)
        .with_icon(icon)
        .with_tooltip(tooltip)
        .with_tooltip_alignment(TooltipAlignment::Center)
        .with_tooltip_positioning_provider(Arc::new(MenuPositioning::BelowInputBox))
        .with_max_label_width(BUTTON_MAX_TEXT_WIDTH)
        .on_click(move |ctx| {
            ctx.dispatch_typed_action(action.clone());
        });

    if let Some(color) = icon_color {
        button = button.with_icon_ansi_color(color);
    }

    button
}
