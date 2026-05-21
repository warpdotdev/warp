use std::sync::Arc;

use pathfinder_color::ColorU;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::{Fill, WarpTheme};
use warpui::clipboard::ClipboardContent;
use warpui::elements::{
    ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DispatchEventResult,
    Element, EventHandler, Flex, MainAxisAlignment, MainAxisSize, ParentElement, Radius, Rect,
    Shrinkable,
};
use warpui::fonts::Weight;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{View, ViewContext, ViewHandle};

use warp_core::ui::appearance::Appearance as CoreAppearance;
use warp_core::ui::theme::color::internal_colors;

use crate::ai::agent::conversation::ConversationStatus;
use crate::ai::agent_management::notifications::item::NotificationSourceAgent;
use crate::ai::agent_management::notifications::{NotificationCategory, NotificationItem};
use crate::ai::agent_management::telemetry::{AgentManagementTelemetryEvent, ArtifactType};
use crate::ai::artifacts::{
    open_screenshot_lightbox, Artifact, ArtifactButtonsRow, ArtifactButtonsRowEvent,
};
use crate::appearance::Appearance;
use crate::send_telemetry_from_ctx;
use crate::ui_components::icon_with_status::{render_icon_with_status, IconWithStatusVariant};
use crate::util::time_format::format_elapsed_since;
use crate::view_components::action_button::ActionButtonTheme;
use crate::workspace::WorkspaceAction;

const COLLAPSED_MAX_CHARS: usize = 100;
const EXPANDED_MAX_CHARS: usize = 500;

fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() > max_chars - 3 {
        let truncated: String = text.chars().take(max_chars).collect();
        format!("{truncated}…")
    } else {
        text.to_owned()
    }
}

/// Returns true when either the title or message would be truncated by `truncate_text`.
fn content_is_truncated(title: &str, message: &str) -> bool {
    title.chars().count() > COLLAPSED_MAX_CHARS - 3
        || message.chars().count() > COLLAPSED_MAX_CHARS - 3
}

/// Determines toast-vs-mailbox rendering differences.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotificationRenderContext {
    Toast,
    Mailbox,
}

/// Button theme for artifact chips in notifications.
/// Uses `outline` for the border so it's visible against `surface_2`.
pub(crate) struct NotificationArtifactButtonTheme;

impl ActionButtonTheme for NotificationArtifactButtonTheme {
    fn background(&self, hovered: bool, appearance: &CoreAppearance) -> Option<Fill> {
        if hovered {
            Some(internal_colors::fg_overlay_2(appearance.theme()))
        } else {
            None
        }
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &CoreAppearance,
    ) -> ColorU {
        appearance.theme().foreground().into_solid()
    }

    fn border(&self, appearance: &CoreAppearance) -> Option<ColorU> {
        Some(appearance.theme().outline().into_solid())
    }
}

/// Callback invoked when the user clicks the expand/collapse affordance on a clamped message.
pub(crate) type OnExpandClick = Box<dyn Fn(&mut warpui::EventContext)>;

/// Renders the inner content of a notification item.
/// Dispatches to the rich layout (with branch row) or simple layout based on `item.branch`.
pub(crate) fn render_notification_item_content(
    item: &NotificationItem,
    artifact_buttons: Option<&ViewHandle<ArtifactButtonsRow>>,
    context: NotificationRenderContext,
    message_expanded: bool,
    on_expand_click: OnExpandClick,
    extra_content: Option<Box<dyn Element>>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();

    let text_column = if item.branch.is_some() {
        render_rich_text_column(
            item,
            artifact_buttons,
            context,
            message_expanded,
            on_expand_click,
            extra_content,
            appearance,
        )
    } else {
        render_simple_text_column(
            item,
            artifact_buttons,
            context,
            message_expanded,
            on_expand_click,
            extra_content,
            appearance,
        )
    };

    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_main_axis_size(MainAxisSize::Max)
        .with_child(
            Container::new(render_agent_avatar(item.agent, item.category, theme))
                .with_margin_right(8.)
                .with_margin_top(2.)
                .finish(),
        )
        .with_child(Shrinkable::new(1.0, text_column).finish())
        .finish()
}

/// Rich layout: branch row + clamped title + clamped message + artifact buttons.
fn render_rich_text_column(
    item: &NotificationItem,
    artifact_buttons: Option<&ViewHandle<ArtifactButtonsRow>>,
    context: NotificationRenderContext,
    message_expanded: bool,
    on_expand_click: OnExpandClick,
    extra_content: Option<Box<dyn Element>>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let branch = item.branch.as_deref().unwrap_or_default();

    let branch_left = render_branch_label(branch, appearance);
    let is_truncated = content_is_truncated(&item.title, &item.message);

    let branch_right: Box<dyn Element> = match context {
        NotificationRenderContext::Toast if is_truncated || message_expanded => {
            render_expand_chevron(message_expanded, on_expand_click, theme)
        }
        NotificationRenderContext::Toast => {
            // No chevron when content fits.
            Flex::row().finish()
        }
        NotificationRenderContext::Mailbox => render_timestamp_with_dot(item, appearance),
    };

    let branch_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_main_axis_size(MainAxisSize::Max)
        .with_child(branch_left)
        .with_child(branch_right)
        .finish();

    let title = render_clamped_title(&item.title, message_expanded, appearance);
    let message = render_message_text(&item.message, message_expanded, appearance);

    let mut content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(branch_row)
        .with_child(title)
        .with_child(Container::new(message).with_margin_top(2.).finish());

    append_trailing_content(&mut content, artifact_buttons, extra_content);
    content.finish()
}

/// Simple layout: title (+ optional chevron) | timestamp row + message + artifact buttons.
fn render_simple_text_column(
    item: &NotificationItem,
    artifact_buttons: Option<&ViewHandle<ArtifactButtonsRow>>,
    context: NotificationRenderContext,
    message_expanded: bool,
    on_expand_click: OnExpandClick,
    extra_content: Option<Box<dyn Element>>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let is_truncated = content_is_truncated(&item.title, &item.message);
    let title_text = render_clamped_title(&item.title, message_expanded, appearance);

    let title_row: Box<dyn Element> = if context == NotificationRenderContext::Mailbox {
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Shrinkable::new(1.0, title_text).finish())
            .with_child(render_timestamp_with_dot(item, appearance))
            .finish()
    } else if is_truncated || message_expanded {
        let chevron = render_expand_chevron(message_expanded, on_expand_click, theme);
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Shrinkable::new(1.0, title_text).finish())
            .with_child(Container::new(chevron).with_margin_top(2.).finish())
            .finish()
    } else {
        title_text
    };

    let message = render_message_text(&item.message, message_expanded, appearance);

    let mut content = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(title_row)
        .with_child(Container::new(message).with_margin_top(2.).finish());

    append_trailing_content(&mut content, artifact_buttons, extra_content);
    content.finish()
}

/// Appends artifact buttons and extra content to a text column.
fn append_trailing_content(
    content: &mut Flex,
    artifact_buttons: Option<&ViewHandle<ArtifactButtonsRow>>,
    extra_content: Option<Box<dyn Element>>,
) {
    if let Some(artifact_buttons) = artifact_buttons {
        content.add_child(
            Container::new(ChildView::new(artifact_buttons).finish())
                .with_margin_top(8.)
                .finish(),
        );
    }
    if let Some(extra) = extra_content {
        content.add_child(extra);
    }
}

/// Renders a git-branch icon + branch name label.
fn render_branch_label(branch: &str, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let color = theme.sub_text_color(theme.surface_1());

    Shrinkable::new(
        1.0,
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(2.)
            .with_child(
                ConstrainedBox::new(Icon::GitBranch.to_warpui_icon(color).finish())
                    .with_width(10.)
                    .with_height(10.)
                    .finish(),
            )
            .with_child(
                Shrinkable::new(
                    1.0,
                    appearance
                        .ui_builder()
                        .wrappable_text(branch.to_owned(), false)
                        .with_style(UiComponentStyles {
                            font_size: Some(12.),
                            font_color: Some(color.into()),
                            font_family_id: Some(appearance.ui_font_family()),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .finish(),
            )
            .finish(),
    )
    .finish()
}

/// Renders the timestamp text + optional unread dot.
fn render_timestamp_with_dot(item: &NotificationItem, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();

    let mut row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min)
        .with_child(
            appearance
                .ui_builder()
                .wrappable_text(format_elapsed_since(item.created_at), false)
                .with_style(UiComponentStyles {
                    font_size: Some(12.),
                    font_color: Some(theme.disabled_text_color(theme.surface_1()).into()),
                    font_family_id: Some(appearance.ui_font_family()),
                    ..Default::default()
                })
                .build()
                .finish(),
        );

    if !item.is_read {
        row.add_child(
            Container::new(
                ConstrainedBox::new(
                    Rect::new()
                        .with_background(theme.accent())
                        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                        .finish(),
                )
                .with_width(10.)
                .with_height(10.)
                .finish(),
            )
            .with_margin_left(8.)
            .finish(),
        );
    }

    row.finish()
}

/// Renders a clickable expand/collapse chevron icon.
fn render_expand_chevron(
    expanded: bool,
    on_click: OnExpandClick,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    let icon = if expanded {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    let chevron = ConstrainedBox::new(
        icon.to_warpui_icon(theme.disabled_text_color(theme.surface_1()))
            .finish(),
    )
    .with_width(12.)
    .with_height(12.)
    .finish();

    EventHandler::new(chevron)
        .on_left_mouse_down(move |ctx, _, _| {
            on_click(ctx);
            DispatchEventResult::StopPropagation
        })
        .finish()
}

/// Renders the title text, truncated based on expanded state.
fn render_clamped_title(title: &str, expanded: bool, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let max = if expanded {
        EXPANDED_MAX_CHARS
    } else {
        COLLAPSED_MAX_CHARS
    };

    appearance
        .ui_builder()
        .wrappable_text(truncate_text(title, max), true)
        .with_style(UiComponentStyles {
            font_size: Some(14.),
            font_weight: Some(Weight::Semibold),
            font_color: Some(theme.main_text_color(theme.surface_1()).into()),
            font_family_id: Some(appearance.ui_font_family()),
            ..Default::default()
        })
        .build()
        .finish()
}

/// Renders the message text, truncated based on expanded state.
fn render_message_text(message: &str, expanded: bool, appearance: &Appearance) -> Box<dyn Element> {
    let theme = appearance.theme();
    let max = if expanded {
        EXPANDED_MAX_CHARS
    } else {
        COLLAPSED_MAX_CHARS
    };

    appearance
        .ui_builder()
        .wrappable_text(truncate_text(message, max), true)
        .with_style(UiComponentStyles {
            font_size: Some(14.),
            font_color: Some(theme.sub_text_color(theme.surface_1()).into()),
            font_family_id: Some(appearance.ui_font_family()),
            ..Default::default()
        })
        .build()
        .finish()
}

/// Total size of the agent avatar component rendered alongside each notification.
const NOTIFICATION_AVATAR_SIZE: f32 = 32.;

fn render_agent_avatar(
    agent: NotificationSourceAgent,
    category: NotificationCategory,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    let status = notification_category_to_conversation_status(category);
    let variant = match agent {
        NotificationSourceAgent::Oz { is_ambient } => IconWithStatusVariant::OzAgent {
            status: Some(status),
            is_ambient,
        },
        NotificationSourceAgent::CLI { agent, is_ambient } => IconWithStatusVariant::CLIAgent {
            agent,
            status: Some(status),
            is_ambient,
        },
    };
    render_icon_with_status(
        variant,
        NOTIFICATION_AVATAR_SIZE,
        0.,
        theme,
        theme.surface_2(),
    )
}

fn notification_category_to_conversation_status(
    category: NotificationCategory,
) -> ConversationStatus {
    match category {
        NotificationCategory::Complete => ConversationStatus::Success,
        NotificationCategory::Request => ConversationStatus::Blocked {
            blocked_action: String::new(),
        },
        NotificationCategory::Error => ConversationStatus::Error,
    }
}

/// Creates an `ArtifactButtonsRow` view with the notification-specific theme.
/// The caller is responsible for subscribing to events on the returned view.
pub(crate) fn create_notification_artifact_buttons_view(
    artifacts: &[Artifact],
    ctx: &mut ViewContext<impl View>,
) -> Option<ViewHandle<ArtifactButtonsRow>> {
    if artifacts.is_empty() {
        return None;
    }
    let theme = Arc::new(NotificationArtifactButtonTheme);
    Some(ctx.add_typed_action_view(|ctx| ArtifactButtonsRow::with_theme(artifacts, theme, ctx)))
}

/// Handles artifact button events from notification views (toasts and mailbox).
pub(crate) fn handle_notification_artifact_buttons_event(
    event: &ArtifactButtonsRowEvent,
    ctx: &mut ViewContext<impl View>,
) {
    match event {
        ArtifactButtonsRowEvent::OpenPlan { notebook_uid } => {
            send_telemetry_from_ctx!(
                AgentManagementTelemetryEvent::ArtifactClicked {
                    artifact_type: ArtifactType::Plan
                },
                ctx
            );
            ctx.dispatch_typed_action(&WorkspaceAction::OpenNotebook {
                id: (*notebook_uid).into(),
            });
        }
        ArtifactButtonsRowEvent::CopyBranch { branch } => {
            send_telemetry_from_ctx!(
                AgentManagementTelemetryEvent::ArtifactClicked {
                    artifact_type: ArtifactType::Branch
                },
                ctx
            );
            ctx.clipboard()
                .write(ClipboardContent::plain_text(branch.clone()));
        }
        ArtifactButtonsRowEvent::OpenPullRequest { url } => {
            send_telemetry_from_ctx!(
                AgentManagementTelemetryEvent::ArtifactClicked {
                    artifact_type: ArtifactType::PullRequest
                },
                ctx
            );
            ctx.open_url(url);
        }
        ArtifactButtonsRowEvent::ViewScreenshots { artifact_uids } => {
            open_screenshot_lightbox(artifact_uids, ctx);
        }
        ArtifactButtonsRowEvent::DownloadFile { artifact_uid } => {
            send_telemetry_from_ctx!(
                AgentManagementTelemetryEvent::ArtifactClicked {
                    artifact_type: ArtifactType::File
                },
                ctx
            );
            crate::ai::artifacts::download_file_artifact(artifact_uid, ctx);
        }
    }
}
