use std::sync::Arc;

use warp_core::ui::icons::Icon;
use warp_core::ui::theme::AnsiColorIdentifier;
use warpui::elements::{ChildView, Element, Empty, ParentElement, Wrap};
use warpui::{AppContext, Entity, TypedActionView, View, ViewContext, ViewHandle};

use super::{file_button_label, Artifact};
use crate::notebooks::NotebookId;
use crate::terminal::input::MenuPositioning;
use crate::view_components::action_button::{
    ActionButton, ActionButtonTheme, ButtonSize, SecondaryTheme, TooltipAlignment,
};

const BUTTON_SPACING: f32 = 8.;
const BUTTON_MAX_TEXT_WIDTH: f32 = 200.;

/// A view that renders a set of artifact buttons (plans, branches, PRs)
pub struct ArtifactButtonsRow {
    buttons: Vec<ViewHandle<ActionButton>>,
    theme: Arc<dyn ActionButtonTheme>,
}

impl ArtifactButtonsRow {
    pub fn new(artifacts: &[Artifact], ctx: &mut ViewContext<Self>) -> Self {
        let theme: Arc<dyn ActionButtonTheme> = Arc::new(SecondaryTheme);
        Self {
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
            buttons: collect_buttons(artifacts, &theme, ctx),
            theme,
        }
    }

    pub fn update_artifacts(&mut self, artifacts: &[Artifact], ctx: &mut ViewContext<Self>) {
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

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        if self.buttons.is_empty() {
            return Empty::new().finish();
        }

        Wrap::row()
            .with_spacing(BUTTON_SPACING)
            .with_run_spacing(BUTTON_SPACING)
            .with_children(
                self.buttons
                    .iter()
                    .map(|button| ChildView::new(button).finish()),
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
