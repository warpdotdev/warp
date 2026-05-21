//! Grid tooltips for the terminal view

use pathfinder_geometry::vector::vec2f;

use warpui::{
    elements::{
        ChildAnchor, Dismiss, MouseStateHandle, OffsetPositioning, PositionedElementAnchor,
        PositionedElementOffsetBounds, Stack,
    },
    AppContext, Element, EventContext,
};

use super::{TerminalAction, TerminalView};
use crate::util::tooltips::{TooltipLink, TooltipRedaction};
use crate::{
    appearance::Appearance,
    terminal::{
        links::directly_open_link_keybinding_string,
        model::{ObfuscateSecrets, Secret},
        safe_mode_settings::get_secret_obfuscation_mode,
        view::SecretTooltip,
        TerminalModel,
    },
};

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use crate::terminal::view::RichContentLink;
        use super::GridHighlightedLink;
    }
}

struct GridTooltipLink {
    text: String,
    action: TerminalAction,
    /// Optional detail text to show after the link.
    detail: Option<String>,
    mouse_state: MouseStateHandle,
}

/// If appropriate, returns a GridTooltipLink for opening the file in warp.
/// Mutates `detail_for_default` leaving None in place if the GridTooltipLink returned is the default
/// action on "Cmd+Click" and thus should use the detail_for_default.
#[cfg(feature = "local_fs")]
fn open_in_warp_tooltip(
    path: std::path::PathBuf,
    line_and_column_num: Option<warp_util::path::LineAndColumnArg>,
    detail_for_default: &mut Option<String>,
    mouse_state: MouseStateHandle,
    app: &AppContext,
) -> Option<GridTooltipLink> {
    use crate::{
        settings::CodeSettings, util::file::external_editor::EditorSettings,
        util::tooltips::should_show_open_in_warp_link,
    };
    use settings::Setting as _;
    use warpui::SingletonEntity;

    if !should_show_open_in_warp_link(&path, app) {
        return None;
    }

    let detail = if *CodeSettings::as_ref(app).code_as_default_editor.value() {
        detail_for_default.take()
    } else {
        None
    };
    Some(GridTooltipLink {
        text: "Open in Warp".to_string(),
        action: TerminalAction::OpenCodeInWarp {
            path,
            layout: *EditorSettings::as_ref(app).open_file_layout.value(),
            line_col: line_and_column_num,
        },
        mouse_state,
        detail,
    })
}

/// Returns a GridTooltipLink for revealing the file in the platform's file explorer
/// (Finder on macOS, file manager on Linux/Windows).
#[cfg(feature = "local_fs")]
fn show_in_file_explorer_tooltip(
    path: std::path::PathBuf,
    mouse_state: MouseStateHandle,
) -> GridTooltipLink {
    let text = if cfg!(target_os = "macos") {
        "Show in Finder"
    } else {
        "Show containing folder"
    }
    .to_string();
    GridTooltipLink {
        text,
        action: TerminalAction::ShowInFileExplorer(path),
        mouse_state,
        detail: None,
    }
}

impl TerminalView {
    /// Renders the link and/or secrets tooltips on top of the grid
    /// Expects at least one of the two tooltips to be visible.
    // Unused variables allowed when no local filesystem as the `app` argument
    // is unused.
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    pub(super) fn render_grid_tooltip(
        &self,
        stack: &mut Stack,
        model: &TerminalModel,
        appearance: &Appearance,
        app: &AppContext,
    ) {
        let mut element_id = "terminal_view:first_cell_in_link".to_string();
        let mut links = vec![];
        let mut is_agent_conversation = false;

        if let Some(open_secret_tooltip) = &self.open_secret_tool_tip {
            match open_secret_tooltip {
                SecretTooltip::Grid {
                    tooltip,
                    is_agent_mode,
                } => {
                    let handle = *tooltip;
                    is_agent_conversation = *is_agent_mode;

                    // Position the tooltip above the first cell in the secret.
                    element_id = format!(
                        "terminal_view:first_cell_in_secret_{}",
                        handle.get_inner().id()
                    );

                    if matches!(get_secret_obfuscation_mode(app), ObfuscateSecrets::Yes) {
                        let is_redacted = model
                            .secret_from_handle(tooltip)
                            .is_some_and(Secret::is_obfuscated);

                        if is_redacted {
                            links.push(GridTooltipLink {
                                text: "Reveal secret".to_string(),
                                action: TerminalAction::ToggleGridSecret {
                                    handle,
                                    show_secret: true,
                                },
                                mouse_state: self.mouse_states.toggle_secrets_tooltip.clone(),
                                detail: None,
                            });
                        } else {
                            links.push(GridTooltipLink {
                                text: "Hide secret".to_string(),
                                action: TerminalAction::ToggleGridSecret {
                                    handle,
                                    show_secret: false,
                                },
                                mouse_state: self.mouse_states.toggle_secrets_tooltip.clone(),
                                detail: None,
                            })
                        }
                    }

                    links.push(GridTooltipLink {
                        text: "Copy secret".to_string(),
                        action: TerminalAction::CopyGridSecret(handle),
                        mouse_state: self.mouse_states.copy_secrets_tooltip.clone(),
                        detail: None,
                    });
                }
                SecretTooltip::RichContent {
                    tooltip,
                    is_agent_mode,
                } => {
                    let tooltip_info = tooltip;
                    // Position the tooltip above the first cell in the secret.
                    element_id = tooltip_info.position_id.to_owned();
                    is_agent_conversation = *is_agent_mode;

                    if matches!(get_secret_obfuscation_mode(app), ObfuscateSecrets::Yes) {
                        let is_obfuscated = tooltip_info.is_obfuscated;

                        if is_obfuscated {
                            links.push(GridTooltipLink {
                                text: "Reveal secret".to_string(),
                                action: TerminalAction::ToggleRichContentSecret {
                                    rich_content_tooltip_info: tooltip_info.clone(),
                                    show_secret: true,
                                },
                                mouse_state: self.mouse_states.toggle_secrets_tooltip.clone(),
                                detail: None,
                            });
                        } else {
                            links.push(GridTooltipLink {
                                text: "Hide secret".to_string(),
                                action: TerminalAction::ToggleRichContentSecret {
                                    rich_content_tooltip_info: tooltip_info.clone(),
                                    show_secret: false,
                                },
                                mouse_state: self.mouse_states.toggle_secrets_tooltip.clone(),
                                detail: None,
                            })
                        }
                    }

                    links.push(GridTooltipLink {
                        text: "Copy secret".to_string(),
                        action: TerminalAction::CopyRichContentSecret(tooltip_info.clone()),
                        mouse_state: self.mouse_states.copy_secrets_tooltip.clone(),
                        detail: None,
                    });
                }
            }
        }

        #[cfg_attr(not(feature = "local_fs"), allow(unused_mut))]
        if let Some(link) = &self.open_grid_link_tool_tip {
            let mut open_in_warp = None;
            let mut show_in_file_explorer = None;
            let modifier = directly_open_link_keybinding_string();
            let mut detail = Some(format!("[{modifier} Click]"));
            #[cfg(feature = "local_fs")]
            {
                if let GridHighlightedLink::File(file_link) = link {
                    if let Some(path) = file_link.get_inner().absolute_path() {
                        open_in_warp = open_in_warp_tooltip(
                            path.clone(),
                            file_link.get_inner().line_and_column_num,
                            &mut detail,
                            self.mouse_states.open_in_warp_tooltip.clone(),
                            app,
                        );
                        show_in_file_explorer = Some(show_in_file_explorer_tooltip(
                            path,
                            self.mouse_states.show_in_file_explorer_tooltip.clone(),
                        ));
                    }
                }
            }

            links.push(GridTooltipLink {
                text: link.tooltip_text().to_owned(),
                action: TerminalAction::OpenGridLink(link.clone()),
                mouse_state: self.mouse_states.grid_link_tooltip.clone(),
                detail,
            });

            links.extend(open_in_warp);
            links.extend(show_in_file_explorer);
        }

        #[cfg_attr(not(feature = "local_fs"), allow(unused_mut))]
        if let Some(tooltip_info) = &self.open_rich_content_link_tool_tip {
            element_id = tooltip_info.position_id.to_owned();
            let mut open_in_warp = None;
            let mut show_in_file_explorer = None;
            let modifier_string = directly_open_link_keybinding_string();
            let mut detail = Some(format!("[{modifier_string} Click]"));

            #[cfg(feature = "local_fs")]
            {
                if let RichContentLink::FilePath {
                    absolute_path,
                    line_and_column_num,
                    ..
                } = &tooltip_info.link
                {
                    open_in_warp = open_in_warp_tooltip(
                        absolute_path.clone(),
                        *line_and_column_num,
                        &mut detail,
                        self.mouse_states.open_in_warp_tooltip.clone(),
                        app,
                    );
                    show_in_file_explorer = Some(show_in_file_explorer_tooltip(
                        absolute_path.clone(),
                        self.mouse_states.show_in_file_explorer_tooltip.clone(),
                    ));
                }
            }

            links.push(GridTooltipLink {
                text: tooltip_info.link.tooltip_text().to_owned(),
                action: TerminalAction::OpenRichContentLink(tooltip_info.link.clone()),
                mouse_state: self.mouse_states.rich_content_link_tooltip.clone(),
                detail,
            });

            links.extend(open_in_warp);
            links.extend(show_in_file_explorer);
        }

        let secret_redaction = get_secret_obfuscation_mode(app);

        // Get the secret level from the current tooltip
        let secret_level = self.open_secret_tool_tip.as_ref().and_then(|tooltip| {
            match tooltip {
                SecretTooltip::Grid { tooltip, .. } => {
                    // For grid secrets, get the secret level from the secret itself
                    model
                        .secret_from_handle(tooltip)
                        .map(|secret| secret.secret_level())
                }
                SecretTooltip::RichContent { tooltip, .. } => Some(tooltip.secret_level),
            }
        });

        let redaction = match (
            self.open_secret_tool_tip.is_some(),
            secret_redaction.should_redact_secret(),
            is_agent_conversation,
        ) {
            (true, true, true) => TooltipRedaction::SecretNotSentToLLMMessaging { secret_level },
            (true, true, false) => {
                TooltipRedaction::SecretWillNotBeSentToLLMMessaging { secret_level }
            }
            (_, _, _) => TooltipRedaction::NoRedaction,
        };
        stack.add_positioned_overlay_child(
            render_tooltip(links, redaction, appearance, app),
            OffsetPositioning::offset_from_save_position_element(
                element_id,
                // Add a small buffer between the tooltip and the top of the cell.
                vec2f(0., -2.),
                PositionedElementOffsetBounds::ParentByPosition,
                PositionedElementAnchor::TopLeft,
                ChildAnchor::BottomLeft,
            ),
        );
    }
}

fn render_tooltip(
    tooltip_links: impl IntoIterator<Item = GridTooltipLink>,
    redaction: TooltipRedaction,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    // Convert GridTooltipLink to shared TooltipLink
    let shared_links = tooltip_links.into_iter().map(|link| {
        let action = link.action;
        TooltipLink {
            text: link.text,
            on_click: move |ctx: &mut EventContext| {
                ctx.dispatch_typed_action(action.clone());
            },
            detail: link.detail,
            mouse_state: link.mouse_state,
        }
    });

    let tooltip_content =
        crate::util::tooltips::render_tooltip(shared_links, redaction, appearance, app);

    Dismiss::new(tooltip_content)
        .on_dismiss(|ctx, _app| {
            ctx.dispatch_typed_action(TerminalAction::MaybeDismissToolTip {
                from_keybinding: false,
            })
        })
        .finish()
}
