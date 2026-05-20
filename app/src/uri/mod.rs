mod docker;
pub mod parse_url_paths;
pub mod web_intent_parser;

#[cfg(target_family = "wasm")]
pub mod browser_url_handler;

use crate::ai::active_agent_views_model::{ActiveAgentViewsModel, ConversationOrTaskId};
use crate::ai::agent::api::ServerConversationToken;
use crate::drive::OpenWarpDriveObjectSettings;
use crate::launch_configs::launch_config::LaunchConfig;
use crate::linear::{LinearAction, LinearIssueWork};
use crate::root_view::{open_new_window_get_handles, OpenLaunchConfigArg};
use crate::server::ids::ServerId;
use crate::server::telemetry::{LaunchConfigUiLocation, TelemetryEvent};
use crate::tab_configs::TabConfig;
use crate::util::openable_file_type::{
    is_file_openable_in_warp, is_markdown_file, is_runnable_shell_script, starts_with_shebang,
};
use crate::workspace::util::PaneViewLocator;
use crate::workspace::{Workspace, WorkspaceAction, WorkspaceRegistry};
use crate::{cloud_object::ObjectType, workspace::ToastStack};
use crate::{drive::OpenWarpDriveObjectArgs, view_components::DismissibleToast};
use crate::{features::FeatureFlag, workspace::active_terminal_in_window};

use crate::ai::ambient_agents::github_auth_notifier::GitHubAuthNotifier;
use crate::settings_view::{OpenTeamsSettingsModalArgs, SettingsSection};
use crate::user_config::{load_launch_configs, load_tab_configs, tab_configs_dir};
use crate::{
    quake_mode_window_id, quake_mode_window_is_open, safe_info, send_telemetry_from_app_ctx,
    ChannelState, OpenPath,
};
use anyhow::{anyhow, ensure, Result};
use itertools::Itertools;
use session_sharing_protocol::common::SessionId;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use url::Url;
use warpui::notification::UserNotification;
use warpui::{platform::TerminationMode, SingletonEntity as _, TypedActionView};

use warpui::{AppContext, EntityId, ViewHandle, WindowId};

use self::docker::open_docker_container;

const DESKTOP_REDIRECT_URI_PATH: &str = "/desktop_redirect";

/// Args for opening the MCP settings page via deeplink, with optional auto-install.
/// The `autoinstall` value is the raw query param string; it is matched case-insensitively
/// against gallery titles in `autoinstall_from_gallery`.
pub struct OpenMCPSettingsArgs {
    pub autoinstall: Option<String>,
}

/// Source query parameter value indicating auth was initiated from cloud agent setup.
/// Used to skip opening settings page after GitHub auth completes.
pub const CLOUD_SETUP_SOURCE: &str = "cloud_setup";

#[derive(Debug, PartialEq, Eq)]
pub enum UriHost {
    Auth,
    Team,
    /// A host prefix for all actions (e.g.: new tab, new window).
    Action,
    /// A host prefix for all actions that involve launch configurations
    Launch,
    /// Supports joining shared sessions via a warp:// URI.
    SharedSession,
    /// Supports viewing AI conversations via a warp:// URI.
    Conversation,
    /// Supports WD object actions
    Drive,
    /// Supports opening warp's settings panel via URI
    Settings,
    /// A host prefix for a general-purpose home/landing page. Unlike other intent URIs, the home
    /// page behavior may change over time and vary from platform to platform.
    Home,
    /// Actions related to MCP servers (e.g.: oauth callbacks).
    Mcp,
    /// Opens a new tab with the Codex model and starts a conversation.
    Codex,
    /// Actions triggered from Linear integrations (e.g. work on issue).
    Linear,
    /// Opens a saved tab config in an existing window or a new one.
    TabConfig,
    /// Focuses a specific terminal pane by its persistent session UUID.
    Session,
}

impl FromStr for UriHost {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "auth" => Ok(Self::Auth),
            "team" => Ok(Self::Team),
            "action" => Ok(Self::Action),
            "launch" => Ok(Self::Launch),
            "shared_session" if FeatureFlag::ViewingSharedSessions.is_enabled() => {
                Ok(Self::SharedSession)
            }
            "conversation" => Ok(Self::Conversation),
            "drive" => Ok(Self::Drive),
            "settings" => Ok(Self::Settings),
            "home" => Ok(Self::Home),
            "mcp" => Ok(Self::Mcp),
            "codex" => Ok(Self::Codex),
            "linear" => Ok(Self::Linear),
            "tab_config" if FeatureFlag::TabConfigs.is_enabled() => Ok(Self::TabConfig),
            "session" => Ok(Self::Session),
            _ => Err(anyhow!("Received url with unexpected host: {}", s)),
        }
    }
}

impl UriHost {
    fn handle(&self, primary_window_id: Option<WindowId>, url: &Url, ctx: &mut AppContext) {
        // Handle host
        match self {
            UriHost::Auth => {
                ctx.window_ids()
                    .collect_vec()
                    .into_iter()
                    .for_each(|window_id| {
                        let Some(root_view_id) = ctx.root_view_id(window_id) else {
                            return;
                        };
                        safe_info!(
                            safe: ("Dispatched auth url to window {window_id}"),
                            full: ("Dispatched auth url {url} to window {window_id}")
                        );
                        ctx.dispatch_action(
                            window_id,
                            &[root_view_id],
                            "root_view:handle_incoming_auth_url",
                            &url.clone(),
                            log::Level::Info,
                        );
                    });
            }
            UriHost::Team => {
                match url.path_segments().into_iter().flatten().last() {
                    // If the last segment of the URL is "settings", open the team settings page.
                    Some("settings") => {
                        open_window_with_action(
                            primary_window_id,
                            "root_view:open_team_settings_page",
                            ctx,
                        );
                    }
                    // Otherwise default to previous behavior.
                    _ => {
                        // TODO: Parse URL to ensure the user is logged into the right account
                        // Shows the user the settings view of their newly joined team within the app.
                        open_window_with_action(
                            primary_window_id,
                            "root_view:handle_team_intent_link_action",
                            ctx,
                        );
                    }
                };
                send_telemetry_from_app_ctx!(TelemetryEvent::OpenTeamFromURI, ctx);
            }
            UriHost::Action => {
                match Action::parse(url) {
                    Ok(action) => action.handle(primary_window_id, url, ctx),
                    Err(err) => {
                        log::warn!("{err}");
                    }
                };
            }
            UriHost::Launch => {
                if let Some(desired_config_path) = get_launch_config_path(url.path()) {
                    let configs = load_launch_configs(&crate::user_config::launch_configs_dir());
                    if let Some(config) =
                        find_matching_config(desired_config_path.as_str(), &configs)
                    {
                        ctx.dispatch_global_action(
                            "root_view:open_launch_config",
                            &OpenLaunchConfigArg {
                                launch_config: config.clone(),
                                ui_location: LaunchConfigUiLocation::Uri,
                                open_in_active_window: false,
                            },
                        )
                    } else {
                        log::warn!(
                            "couldn't find a matching file path for '{}'",
                            desired_config_path.as_str()
                        );
                    }
                } else {
                    log::warn!("couldn't turn launch link '{}' into path", url.path());
                }
            }
            UriHost::TabConfig => {
                handle_tab_config_uri(primary_window_id, url, ctx);
            }
            UriHost::SharedSession => {
                // We expect the uri to have the ID of the session to join as the last segment.
                // e.g. warp://shared_session/{id}
                let session_id = url
                    .path_segments()
                    .into_iter()
                    .flatten()
                    .last()
                    .and_then(|id| SessionId::from_str(id).ok());
                if let Some(session_id) = session_id {
                    // If there's an existing window, join the session inc a new tab. Otherwise, open a new window.
                    match primary_window_id.and_then(|window_id| {
                        ctx.root_view_id(window_id)
                            .map(|view_id| (window_id, view_id))
                    }) {
                        Some((primary_window_id, root_view_id)) => {
                            ctx.dispatch_action(
                                primary_window_id,
                                &[root_view_id],
                                "root_view:join_shared_session_in_existing_window",
                                &session_id,
                                log::Level::Info,
                            );
                        }
                        None => {
                            ctx.dispatch_global_action("root_view:join_shared_session", &session_id)
                        }
                    }
                } else {
                    log::warn!("Failed to join shared session with uri={url}");
                }
            }
            UriHost::Conversation => {
                // We expect the uri to have the conversation ID as the last segment.
                // e.g. warp://conversation/{conversation_id}
                let conversation_id: Option<ServerConversationToken> = url
                    .path_segments()
                    .into_iter()
                    .flatten()
                    .last()
                    .map(|s| ServerConversationToken::new(s.to_owned()));

                if let Some(conversation_id) = conversation_id {
                    // If there's an existing window, open the conversation in a new tab. Otherwise, open a new window.
                    match primary_window_id.and_then(|window_id| {
                        ctx.root_view_id(window_id)
                            .map(|view_id| (window_id, view_id))
                    }) {
                        Some((primary_window_id, root_view_id)) => {
                            ctx.dispatch_action(
                                primary_window_id,
                                &[root_view_id],
                                "root_view:open_cloud_conversation_in_existing_window",
                                &conversation_id,
                                log::Level::Info,
                            );
                        }
                        None => ctx.dispatch_global_action(
                            "root_view:open_conversation_viewer",
                            &conversation_id,
                        ),
                    }
                } else {
                    log::warn!("Failed to open conversation with uri={url}");
                }
            }
            UriHost::Drive => {
                // We expect the uri to have the ID of the object we are trying to open and the object_type.
                // e.g. warp://drive/{object_type}?id={UID}
                // For folder links, we expect an additional query parameter primary_object_id which refers to the id object
                // that should be opened
                // When the user is directed here via the request access flow, we expect an additional query parameter invitee_email
                // If this parameter is present, we will open the sharing dialog with the email filled in.
                let object_type = url
                    .path_segments()
                    .into_iter()
                    .flatten()
                    .last()
                    .and_then(|object_type| ObjectType::from_str(object_type).ok());

                let query_string: HashMap<_, _> = url.query_pairs().collect();
                let object_server_id: Option<ServerId> =
                    query_string.get("id").map(ServerId::from_string_lossy);

                let focused_folder_id: Option<ServerId> = query_string
                    .get("focused_folder_id")
                    .map(ServerId::from_string_lossy);

                let invitee_email: Option<String> =
                    query_string.get("invitee_email").map(|s| s.to_string());

                if let Some((object_type, server_id)) = object_type.zip(object_server_id) {
                    let primary_window_and_view = primary_window_id.and_then(|window_id| {
                        ctx.root_view_id(window_id)
                            .map(|view_id| (window_id, view_id))
                    });
                    let args = OpenWarpDriveObjectArgs {
                        object_type,
                        server_id,
                        settings: OpenWarpDriveObjectSettings {
                            focused_folder_id,
                            invitee_email,
                        },
                    };
                    // If there's an existing window, open the object in that window, otherwise open a new window
                    if let Some((primary_window_id, root_view_id)) = primary_window_and_view {
                        // `args` may contain user-identifiable fields
                        // (e.g. `invitee_email`), so avoid writing the full
                        // debug representation to `warp.log` on non-dogfood
                        // release channels.
                        safe_info!(
                            safe: (
                                "Opening drive object in existing window: object_type={:?} server_id={}",
                                args.object_type, args.server_id,
                            ),
                            full: ("Opening drive object in existing window: {args:?}")
                        );
                        ctx.dispatch_action(
                            primary_window_id,
                            &[root_view_id],
                            "root_view:open_drive_object_existing_window",
                            &args,
                            log::Level::Info,
                        );
                    } else {
                        ctx.dispatch_global_action("root_view:open_drive_object_new_window", &args)
                    }
                } else {
                    log::warn!("Failed to open drive object with uri={url}");
                }
            }
            UriHost::Settings => {
                // We support opening different settings pages through URI:
                // - warp://settings/teams?invite={email} - opens team settings with invite modal
                // - warp://settings/billing_and_usage - opens billing and usage settings page
                // - warp://settings/environments - opens environments settings page
                // - warp://settings/mcp - opens MCP servers settings page
                // - warp://settings/platform - opens platform settings page
                // - warp://settings/appearance - opens appearance settings page (themes, fonts, etc.)
                let settings_sub_page: Option<String> = url
                    .path_segments()
                    .into_iter()
                    .flatten()
                    .last()
                    .map(|s| s.to_string());
                let query_string: HashMap<_, _> = url.query_pairs().collect();

                if let Some(settings_sub_page) = settings_sub_page {
                    match settings_sub_page.as_str() {
                        "teams" => {
                            let invite_email = query_string.get("invite").map(|s| s.to_string());
                            let args = OpenTeamsSettingsModalArgs { invite_email };
                            dispatch_action_in_new_or_existing_window(
                                primary_window_id,
                                "root_view:open_team_settings_with_email_invite_in_existing_window",
                                "root_view:open_team_settings_with_email_invite_in_new_window",
                                &args,
                                ctx,
                            );
                        }
                        "billing_and_usage" => {
                            dispatch_action_in_new_or_existing_window(
                                primary_window_id,
                                "root_view:open_settings_page_in_existing_window",
                                "root_view:open_settings_page_in_new_window",
                                &SettingsSection::BillingAndUsage,
                                ctx,
                            );
                        }
                        "environments" => {
                            // Notify that GitHub auth completed so views can refresh
                            GitHubAuthNotifier::handle(ctx).update(ctx, |notifier, ctx| {
                                notifier.notify_auth_completed(ctx);
                            });

                            // Open settings page unless auth was initiated from cloud setup
                            // (cloud setup users should stay on their current page)
                            let source = query_string.get("source").map(|s| s.as_ref());
                            let skip_settings = source == Some(CLOUD_SETUP_SOURCE);
                            if !skip_settings {
                                dispatch_action_in_new_or_existing_window(
                                    primary_window_id,
                                    "root_view:open_settings_page_in_existing_window",
                                    "root_view:open_settings_page_in_new_window",
                                    &SettingsSection::CloudEnvironments,
                                    ctx,
                                );
                            }
                        }
                        "mcp" => {
                            // warp://settings/mcp?autoinstall=<name> auto-installs a gallery MCP server.
                            // The value is matched case-insensitively against gallery titles.
                            let autoinstall =
                                query_string.get("autoinstall").map(|v| v.to_string());
                            let args = OpenMCPSettingsArgs { autoinstall };
                            dispatch_action_in_new_or_existing_window(
                                primary_window_id,
                                "root_view:open_mcp_settings_in_existing_window",
                                "root_view:open_mcp_settings_in_new_window",
                                &args,
                                ctx,
                            );
                        }
                        "platform" => {
                            dispatch_action_in_new_or_existing_window(
                                primary_window_id,
                                "root_view:open_settings_page_in_existing_window",
                                "root_view:open_settings_page_in_new_window",
                                &SettingsSection::OzCloudAPIKeys,
                                ctx,
                            );
                        }
                        "appearance" => {
                            dispatch_action_in_new_or_existing_window(
                                primary_window_id,
                                "root_view:open_settings_page_in_existing_window",
                                "root_view:open_settings_page_in_new_window",
                                &SettingsSection::Appearance,
                                ctx,
                            );
                        }
                        _ => {
                            log::warn!("Failed to open settings pane with uri={url}");
                        }
                    }
                } else {
                    log::warn!("Failed to open settings pane with uri={url}");
                }
            }
            UriHost::Home => {
                ctx.dispatch_global_action("root_view::open_new", &());
            }
            UriHost::Mcp => {
                #[cfg(not(target_family = "wasm"))]
                {
                    let result = crate::ai::mcp::TemplatableMCPServerManager::handle(ctx)
                        .update(ctx, |manager, _ctx| manager.handle_oauth_callback(url));
                    if let Err(e) = result {
                        log::error!("Failed to handle MCP OAuth callback: {e:?}");
                    }
                }
            }
            UriHost::Codex => {
                dispatch_action_in_new_or_existing_window(
                    primary_window_id,
                    "root_view:open_codex_in_existing_window",
                    "root_view:open_codex_in_new_window",
                    &(),
                    ctx,
                );
            }
            UriHost::Linear => match LinearAction::parse(url) {
                Ok(LinearAction::WorkOnIssue) => {
                    let args = LinearIssueWork::from_url(url);
                    dispatch_action_in_new_or_existing_window(
                        primary_window_id,
                        "root_view:open_linear_issue_work_in_existing_window",
                        "root_view:open_linear_issue_work_in_new_window",
                        &args,
                        ctx,
                    );
                }
                Err(err) => {
                    log::warn!("{err}");
                }
            },
            UriHost::Session => {
                let uuid_hex = url
                    .path_segments()
                    .into_iter()
                    .flatten()
                    .last()
                    .unwrap_or("");

                let Some(uuid_bytes) = decode_uuid_hex(uuid_hex) else {
                    log::warn!(
                        "session deep link received invalid UUID hex (safe: len={})",
                        uuid_hex.len()
                    );
                    return;
                };

                let result = WorkspaceRegistry::as_ref(ctx)
                    .all_workspaces(ctx)
                    .iter()
                    .find_map(|(win_id, workspace)| {
                        workspace.as_ref(ctx).tab_views().find_map(|pane_group| {
                            let pane_id = pane_group
                                .as_ref(ctx)
                                .find_terminal_pane_by_session_uuid(&uuid_bytes)?;
                            Some((
                                *win_id,
                                PaneViewLocator {
                                    pane_group_id: pane_group.id(),
                                    pane_id,
                                },
                            ))
                        })
                    });

                if let Some((window_id, locator)) = result {
                    ctx.windows().show_window_and_focus_app(window_id);
                    if let Some(root_view_id) = ctx.root_view_id(window_id) {
                        ctx.dispatch_action_for_view(
                            window_id,
                            root_view_id,
                            "root_view:handle_pane_navigation_event",
                            &locator,
                        );
                    }
                } else {
                    log::warn!("session deep link could not find pane with given UUID");
                }
            }
        }
    }

    /// When handling this URI action, determine which window(s) should be focused.
    #[cfg_attr(not(any(target_os = "linux", target_os = "freebsd")), allow(dead_code))]
    fn window_behavior_hint(&self) -> WindowBehaviorHint {
        use WindowBehaviorHint as W;
        match self {
            Self::Auth => W::ShowPrimaryWindow(WindowActivationFallbackBehavior::NewWindow {
                replace_existing: true,
            }),
            Self::Team | Self::Drive | Self::Settings => W::default(),
            // These URLs always open new windows.
            Self::Launch | Self::SharedSession | Self::Conversation | Self::Home => W::Nothing,
            // This will actually be handled by [`Action::window_behavior_hint`].
            Self::Action => W::Nothing,
            // TODO(vorporeal): probably want to focus the window with the MCP pane open
            Self::Mcp => W::Nothing,
            // Codex opens a new tab with AI mode, use default behavior
            Self::Codex => W::default(),
            // Linear deeplink opens a new tab with agent view
            Self::Linear => W::default(),
            // Handler picks the window itself based on `?new_window=true`.
            Self::TabConfig => W::Nothing,
            Self::Session => W::Nothing,
        }
    }
}

/// This determines which windows, if any, will become visible on handling a URI. This is a "hint"
/// because it is platform-dependent, and not all platforms can conform. For example, MacOS
/// automatically shows the frontmost window, and so the Nothing variant of this is impossible on
/// MacOS.
#[derive(Clone, Debug)]
enum WindowBehaviorHint {
    /// Determined by the [`get_primary_window`] function.
    ShowPrimaryWindow(WindowActivationFallbackBehavior),
    Nothing,
}

impl Default for WindowBehaviorHint {
    fn default() -> Self {
        Self::ShowPrimaryWindow(WindowActivationFallbackBehavior::NewWindow {
            replace_existing: false,
        })
    }
}

impl WindowBehaviorHint {
    /// Perform the desired window focus behavior for the URI being handled. This may change the
    /// "primary window" if a new one has to be created. Return the new primary WindowId.
    #[cfg_attr(not(any(target_os = "linux", target_os = "freebsd")), allow(dead_code))]
    fn resolve(
        self,
        primary_window_id: Option<WindowId>,
        ctx: &mut AppContext,
    ) -> Option<WindowId> {
        match self {
            Self::ShowPrimaryWindow(fallback_behavior) => {
                if let Some(window_id) = primary_window_id {
                    match ctx.windows().windowing_system() {
                        Some(windowing_system)
                            if windowing_system.allows_programmatic_window_activation() =>
                        {
                            ctx.windows().show_window_and_focus_app(window_id);
                        }
                        _ => {
                            return fallback_behavior.resolve(window_id, ctx);
                        }
                    }
                }
            }
            Self::Nothing => {}
        };
        primary_window_id
    }
}

/// If we're in an environment where we can't fulfill [`WindowBehaviorHint`], and the OS default
/// behavior isn't acceptable/reliable, e.g. Wayland doesn't allow windows to programmatically show
/// themselves, try this fallback behavior instead.
#[derive(Clone, Debug)]
enum WindowActivationFallbackBehavior {
    /// If the primary window picked to handle the URL is not the active one, send a native push
    /// notification.
    Notify { title: String, description: String },
    /// Create a new window to handle the URI.
    NewWindow {
        /// Close the former "primary window" as determined by [`get_primary_window`]. This should
        /// generally default to `false` to avoid closing a window with information that the user
        /// may still want. One exception is the Auth route where the old window just showed the
        /// auth page.
        replace_existing: bool,
    },
}

impl WindowActivationFallbackBehavior {
    /// Perform the desired window fallback behavior for the URI being handled. This may change the
    /// "primary window" if a new one has to be created. Return the new primary WindowId.
    #[cfg_attr(not(any(target_os = "linux", target_os = "freebsd")), allow(dead_code))]
    fn resolve(self, primary_window_id: WindowId, ctx: &mut AppContext) -> Option<WindowId> {
        match self {
            WindowActivationFallbackBehavior::Notify { title, description } => {
                if ctx
                    .windows()
                    .active_window()
                    .is_some_and(|active_window| active_window == primary_window_id)
                {
                    return Some(primary_window_id);
                }
                if let Some(view_handle) = ctx
                    .views_of_type::<Workspace>(primary_window_id)
                    .filter(|views| !views.is_empty())
                    .map(|mut views| views.swap_remove(0))
                {
                    view_handle.update(ctx, |_, ctx| {
                        ctx.send_desktop_notification(
                            UserNotification::new(title, description, None),
                            |_, err, ctx| {
                                log::warn!(
                                    "Error showing URL intent notification on {:?}: {err:?}",
                                    ctx.window_id()
                                )
                            },
                        );
                    });
                }
                Some(primary_window_id)
            }
            WindowActivationFallbackBehavior::NewWindow { replace_existing } => {
                let new_window_id = open_new_window_get_handles(None, ctx).0;
                if replace_existing {
                    ctx.windows()
                        .close_window(primary_window_id, TerminationMode::Cancellable);
                }
                Some(new_window_id)
            }
        }
    }
}

/// Turn the launch config URL into a filename.
/// "/hello%20world" --> "hello world"
fn get_launch_config_path(path: &str) -> Option<String> {
    // Remove the leading slash before the filename.
    let (_, config_path) = path.split_once('/')?;

    // URL-decode the filename to recover spaces and
    // other non-URL-friendly characters
    let decoded = serde_urlencoded::from_str::<Vec<(String, String)>>(config_path).ok()?;

    // serde_urlencoded::from_str tries to find a vector key-value pairs,
    // so we'll take the first tuple in the vector...
    let decoded_config_name = decoded.first()?;

    // ... and read the first member of the tuple.
    let validated_path = validate_launch_config_path(decoded_config_name.0.as_str())?;

    // Finally, return the validated path.
    Some(validated_path.to_string())
}

/// Remove file extension, which consists of the last '.' in the filename
/// and whatever characters follow it.
fn remove_extension(full_path: &str) -> Option<&str> {
    let (no_extension, _) = full_path.rsplit_once('.')?;
    Some(no_extension)
}

/// Ensure that a path is relative and doesn't contain '/../',
/// to prevent launch config links from escaping the launch config directory.
fn validate_launch_config_path(path: &str) -> Option<&str> {
    if path.starts_with('/')
        || path.starts_with("../")
        || path.contains("/../")
        || path.ends_with("/..")
    {
        None
    } else {
        Some(path)
    }
}

/// Given a config path, find a matching launch config file
fn find_matching_config<'a>(
    target_path: &str,
    configs: &'a [LaunchConfig],
) -> Option<&'a LaunchConfig> {
    // first, try to match the exact filename.
    if let Some(matched_config) = find_matching_config_name(target_path, configs) {
        return Some(matched_config);
    }

    // next, try to match the filename without the extension
    let no_extension = remove_extension(target_path)?;
    find_matching_config_name(no_extension, configs)
}

/// Case-insensitive matching on the config's name
/// (field in the YAML file).
fn find_matching_config_name<'a>(
    target_name: &str,
    configs: &'a [LaunchConfig],
) -> Option<&'a LaunchConfig> {
    let target_name_lower = target_name.to_lowercase();
    configs
        .iter()
        .find(|&config| config.name.to_lowercase() == target_name_lower)
}

/// Handles `warp://tab_config/<name>` deeplinks.
///
/// Resolution rules:
/// - `<name>` is matched case-insensitively against each tab config's file
///   stem, so both `warp://tab_config/my_tab` and
///   `warp://tab_config/my_tab.toml` work.
/// - When `?new_window=true` (or no Warp window is open) the tab config opens
///   in a brand-new window. Otherwise it opens as a new tab in the active
///   window.
fn handle_tab_config_uri(primary_window_id: Option<WindowId>, url: &Url, ctx: &mut AppContext) {
    let Some(desired) = get_launch_config_path(url.path()) else {
        log::warn!("couldn't turn tab config link '{}' into name", url.path());
        return;
    };

    let (configs, _errors) = load_tab_configs(&tab_configs_dir());
    let Some(config) = find_matching_tab_config(desired.as_str(), configs) else {
        log::warn!("couldn't find a tab config matching '{}'", desired);
        return;
    };

    let force_new_window = url
        .query_pairs()
        .any(|(k, v)| k == "new_window" && matches!(v.as_ref(), "1" | "true"));

    let target_window_id = if force_new_window {
        None
    } else {
        primary_window_id.filter(|id| WorkspaceRegistry::as_ref(ctx).get(*id, ctx).is_some())
    };

    let workspace = match target_window_id {
        Some(window_id) => WorkspaceRegistry::as_ref(ctx).get(window_id, ctx),
        None => {
            let new_window_id = open_new_window_get_handles(None, ctx).0;
            WorkspaceRegistry::as_ref(ctx).get(new_window_id, ctx)
        }
    };

    let Some(workspace) = workspace else {
        log::warn!(
            "no workspace available to open tab config '{}'",
            config.name
        );
        return;
    };

    workspace.update(ctx, |workspace, ctx| {
        workspace.open_tab_config(config, ctx);
    });
}

/// Case-insensitive match against each tab config's file stem. Tab config
/// `name` fields are not unique across files, so we key off the filename.
///
/// Tries the target as-is first, then with the extension stripped, so both
/// `my_tab` and `my_tab.toml` resolve to `my_tab.toml` and dotted stems like
/// `foo.bar` (from `foo.bar.toml`) still work when written without `.toml`.
fn find_matching_tab_config(target: &str, configs: Vec<TabConfig>) -> Option<TabConfig> {
    let raw = target.to_lowercase();
    let stripped = remove_extension(target).map(str::to_lowercase);
    configs.into_iter().find(|c| {
        c.source_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .and_then(|s| s.to_str())
            .map(|s| {
                let stem = s.to_lowercase();
                stem == raw || Some(stem.as_str()) == stripped.as_deref()
            })
            .unwrap_or(false)
    })
}

/// Extract the `path` query parameter, expanding a leading `~` to the
/// user's home directory.
fn parse_tab_path(url: &Url) -> Option<PathBuf> {
    let raw = url.query_pairs().find(|(k, _)| k == "path")?.1;
    Some(PathBuf::from(shellexpand::tilde(&raw).into_owned()))
}

#[derive(Debug)]
enum Action {
    NewTab,
    NewWindow,
    Docker,
    OpenRepo,
    CloudAgentSetup,
    NewCloudAgentConversation,
    NewAgentConversation,
    CreateEnvironment { repos: Vec<String> },
    FocusCloudMode,
}

impl Action {
    fn parse(url: &Url) -> Result<Self> {
        match url.path() {
            "/new_tab" => Ok(Self::NewTab),
            "/new_window" => Ok(Self::NewWindow),
            "/docker/open_subshell" => Ok(Self::Docker),
            "/open-repo" => Ok(Self::OpenRepo),
            "/cloud_agent_setup" => Ok(Self::CloudAgentSetup),
            "/new_cloud_agent_conversation" => Ok(Self::NewCloudAgentConversation),
            "/new_agent_conversation" => Ok(Self::NewAgentConversation),
            "/create_environment" => {
                let repos = url
                    .query_pairs()
                    .filter_map(|(k, v)| (k == "repo").then(|| v.into_owned()))
                    .collect::<Vec<_>>();

                Ok(Self::CreateEnvironment { repos })
            }
            "/focus_cloud_mode" => Ok(Self::FocusCloudMode),
            _ => Err(anyhow!(
                "Received \"action\" intent with unexpected action: {}",
                url.path()
            )),
        }
    }

    fn handle(&self, primary_window_id: Option<WindowId>, url: &Url, ctx: &mut AppContext) {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let primary_window_id = self.window_behavior_hint().resolve(primary_window_id, ctx);
        match self {
            Self::NewTab | Self::NewWindow => {
                let window_id = if let Self::NewTab = self {
                    primary_window_id
                } else {
                    None
                };
                let Some(path) = parse_tab_path(url) else {
                    log::warn!("Could not parse path to open a new tab/window");
                    return;
                };
                open_file(window_id, path, ctx);
            }
            Action::Docker => {
                if let Err(err) = open_docker_container(url, ctx) {
                    if let Some(window_id) = primary_window_id {
                        ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                            let toast =
                                DismissibleToast::error("Custom URI is invalid.".to_owned());
                            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                        });
                    }

                    log::warn!("error opening docker container: {err}");
                }
            }
            Action::OpenRepo => {
                let window_id =
                    primary_window_id.or_else(|| Some(open_new_window_get_handles(None, ctx).0));

                let Some(window_id) = window_id else {
                    log::warn!("unable to determine window for open repo action");
                    return;
                };

                let Some(mut workspaces) = ctx.views_of_type::<Workspace>(window_id) else {
                    log::warn!("no workspace found in window {window_id} for open repo action");
                    return;
                };

                if let Some(workspace) = workspaces.pop() {
                    workspace.update(ctx, |workspace, ctx| {
                        workspace
                            .handle_action(&WorkspaceAction::OpenRepository { path: None }, ctx);
                    });
                } else {
                    log::warn!("no workspace views in window {window_id} for open repo action");
                }
            }
            Action::CloudAgentSetup => {
                let window_id =
                    primary_window_id.or_else(|| Some(open_new_window_get_handles(None, ctx).0));

                let Some(window_id) = window_id else {
                    log::warn!("unable to determine window for cloud agent setup action");
                    return;
                };

                let Some(mut workspaces) = ctx.views_of_type::<Workspace>(window_id) else {
                    log::warn!(
                        "no workspace found in window {window_id} for cloud agent setup action"
                    );
                    return;
                };

                if let Some(workspace) = workspaces.pop() {
                    workspace.update(ctx, |workspace, ctx| {
                        workspace.handle_action(&WorkspaceAction::OpenCloudAgentSetupGuide, ctx);
                    });
                } else {
                    log::warn!(
                        "no workspace views in window {window_id} for cloud agent setup action"
                    );
                }
            }
            Action::NewCloudAgentConversation => {
                let window_id =
                    primary_window_id.or_else(|| Some(open_new_window_get_handles(None, ctx).0));

                let Some(window_id) = window_id else {
                    log::warn!(
                        "unable to determine window for new cloud agent conversation action"
                    );
                    return;
                };

                let Some(mut workspaces) = ctx.views_of_type::<Workspace>(window_id) else {
                    log::warn!(
                        "no workspace found in window {window_id} for new cloud agent conversation action"
                    );
                    return;
                };

                if let Some(workspace) = workspaces.pop() {
                    workspace.update(ctx, |workspace, ctx| {
                        workspace.handle_action(&WorkspaceAction::AddAmbientAgentTab, ctx);
                    });
                } else {
                    log::warn!(
                        "no workspace views in window {window_id} for new cloud agent conversation action"
                    );
                }
            }
            Action::NewAgentConversation => {
                let window_id =
                    primary_window_id.or_else(|| Some(open_new_window_get_handles(None, ctx).0));

                let Some(window_id) = window_id else {
                    log::warn!("unable to determine window for new agent conversation action");
                    return;
                };

                let Some(workspace) = WorkspaceRegistry::as_ref(ctx).get(window_id, ctx) else {
                    log::warn!(
                        "no workspace found in window {window_id} for new agent conversation action"
                    );
                    return;
                };

                workspace.update(ctx, |workspace, ctx| {
                    workspace.handle_action(&WorkspaceAction::AddAgentTab, ctx);
                });
            }
            Action::CreateEnvironment { repos } => {
                use crate::root_view::CreateEnvironmentArg;

                let arg = CreateEnvironmentArg {
                    repos: repos.clone(),
                };

                let primary_window_and_view = primary_window_id.and_then(|window_id| {
                    ctx.root_view_id(window_id)
                        .map(|view_id| (window_id, view_id))
                });

                if let Some((primary_window_id, root_view_id)) = primary_window_and_view {
                    ctx.dispatch_action(
                        primary_window_id,
                        &[root_view_id],
                        "root_view:create_environment_in_existing_window",
                        &arg,
                        log::Level::Info,
                    );
                } else {
                    ctx.dispatch_global_action("root_view:create_environment", &arg);
                }
            }
            Action::FocusCloudMode => {
                let active_agent_views = ActiveAgentViewsModel::as_ref(ctx);
                let focused_conversation = primary_window_id
                    .and_then(|wid| active_agent_views.get_focused_conversation(wid));
                let mut terminal_view_id = match focused_conversation {
                    Some(ConversationOrTaskId::TaskId(task_id)) => {
                        active_agent_views.get_terminal_view_id_for_ambient_task(task_id)
                    }
                    Some(ConversationOrTaskId::ConversationId(conversation_id)) => {
                        active_agent_views
                            .get_terminal_view_id_for_conversation(conversation_id, ctx)
                    }
                    None => None,
                };
                if terminal_view_id.is_none() {
                    terminal_view_id = find_cloud_mode_terminal_view_id(primary_window_id, ctx);
                }
                if terminal_view_id.is_none() {
                    terminal_view_id = active_agent_views.get_last_focused_terminal_id();
                }
                if terminal_view_id.is_none() {
                    terminal_view_id = primary_window_id
                        .and_then(|window_id| active_terminal_view_id_in_window(window_id, ctx));
                }

                if let Some(terminal_view_id) = terminal_view_id {
                    if let Some((window_id, workspace)) =
                        find_workspace_for_terminal_view(terminal_view_id, ctx)
                    {
                        ctx.windows().show_window_and_focus_app(window_id);
                        workspace.update(ctx, |workspace, ctx| {
                            workspace.handle_action(
                                &WorkspaceAction::FocusTerminalViewInWorkspace { terminal_view_id },
                                ctx,
                            );
                        });
                        // Notify after focusing so Cloud Mode panes can retry in the selected pane.
                        GitHubAuthNotifier::handle(ctx).update(ctx, |notifier, ctx| {
                            notifier.notify_auth_completed(ctx);
                        });
                        return;
                    }
                }

                GitHubAuthNotifier::handle(ctx).update(ctx, |notifier, ctx| {
                    notifier.notify_auth_completed(ctx);
                });
                dispatch_action_in_new_or_existing_window(
                    primary_window_id,
                    "root_view:open_settings_page_in_existing_window",
                    "root_view:open_settings_page_in_new_window",
                    &SettingsSection::CloudEnvironments,
                    ctx,
                );
            }
        }
    }

    /// When handling this URI action, determine which window(s) should be focused.
    #[cfg_attr(not(any(target_os = "linux", target_os = "freebsd")), allow(dead_code))]
    fn window_behavior_hint(&self) -> WindowBehaviorHint {
        use WindowBehaviorHint as W;
        match self {
            Self::Docker
            | Self::CreateEnvironment { .. }
            | Self::OpenRepo
            | Self::CloudAgentSetup
            | Self::NewCloudAgentConversation
            | Self::NewAgentConversation
            | Self::FocusCloudMode => W::default(),
            Self::NewTab => W::ShowPrimaryWindow(WindowActivationFallbackBehavior::Notify {
                title: "New tab created".to_owned(),
                description: "Go to Warp to see your new tab.".to_owned(),
            }),
            Self::NewWindow => W::Nothing,
        }
    }
}

/// Handles all incoming urls. These urls are file urls, auth urls for login,
/// and team urls for opening team settings.
pub fn handle_incoming_uri(url: &Url, ctx: &mut AppContext) {
    // Non-dogfood builds must never log the full URL here: URLs routed to this
    // handler can carry secrets in their query string (for example, the
    // Firebase `refresh_token` on `warp://auth/desktop_redirect?...`). Log
    // only the non-sensitive components (scheme, host, path) on release
    // channels; dogfood builds retain the full URL for local debugging.
    safe_info!(
        safe: ("received url {}", safe_url_log_fields(url)),
        full: ("received url {:?}", &url)
    );

    // Pick the window that should be handling the URI.  This has some
    // additional logic to handle the hotkey window and there being no
    // currently-active window.
    let primary_window_id = get_primary_window(ctx.windows().frontmost_window_id(), ctx);

    // If we're running on a platform where we can spawn local TTYs,
    // check if this is a file:// URL and if so, spawn a new session
    // with an initial working directory based on the provided path.
    #[cfg(feature = "local_tty")]
    if url.scheme() == "file" {
        if let Ok(path) = url.to_file_path() {
            open_file(primary_window_id, path, ctx);
        }
        return;
    }

    match validate_custom_uri(url) {
        Ok(host) => {
            #[cfg(any(target_os = "linux", target_os = "freebsd", windows))]
            let primary_window_id = host.window_behavior_hint().resolve(primary_window_id, ctx);
            host.handle(primary_window_id, url, ctx);
        }
        Err(e) => {
            if let Some(window_id) = primary_window_id {
                ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = DismissibleToast::error(format!("Custom URI is invalid: {e:?}"));
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
            }
            log::warn!("Custom URI is invalid: {e:?}");
        }
    }
}

/// Gets the primary window ID, and returns `None` if it does not exist.
/// A primary window is the foregrounded window, or one of the inactive non-quake windows.
/// A closed quake window is not counted.
fn get_primary_window(
    active_window_id: Option<WindowId>,
    ctx: &mut AppContext,
) -> Option<WindowId> {
    // Return quake mode window if it's open
    if let Some(window_id) = quake_mode_window_id()
        .filter(|window_id| quake_mode_window_is_open() && ctx.is_window_open(*window_id))
    {
        return Some(window_id);
    }

    // Otherwise, return active window
    if let Some(window_id) = active_window_id {
        return Some(window_id);
    }

    let mut non_quake_mode_windows = ctx
        .window_ids()
        .filter(|window_id| Some(*window_id) != quake_mode_window_id());

    // There's no active window, return first non-quake mode window or None if none exist.
    non_quake_mode_windows.next()
}

/// What `open_file` should do with an incoming `file://` URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenFileAction {
    /// Open in the markdown notebook pane.
    Notebook,
    /// Open in Warp's code/text editor pane.
    Editor,
    /// Open a session at the parent directory and queue the file as the pending command,
    /// or just open a session at the directory path if `path` is a directory.
    ExecuteInSession,
}

/// Pure routing decision for `open_file`. Extracted so it can be unit-tested without
/// standing up a full `AppContext`.
fn classify_open_file_action(path: &Path) -> OpenFileAction {
    if is_markdown_file(path) {
        return OpenFileAction::Notebook;
    }
    if path.is_file() {
        if is_runnable_shell_script(path) {
            return OpenFileAction::ExecuteInSession;
        }
        // Anything we can show in the editor opens there. The second branch catches
        // shebang scripts that `is_file_openable_in_warp` rejects on extension alone
        // (e.g. an extensionless `#!/bin/sh` file without the user-execute bit) so
        // they don't fall through to the executor and produce a `permission denied`.
        if is_file_openable_in_warp(path).is_some() || starts_with_shebang(path) {
            return OpenFileAction::Editor;
        }
    }
    OpenFileAction::ExecuteInSession
}

/// Handle an incoming `file://` URL.
/// * Markdown files are opened as notebook panes.
/// * For directories, open a new session at the directory path.
/// * For other files, open a new session at the parent directory path, then possibly execute the
///   file.
fn open_file(window_id: Option<WindowId>, path: PathBuf, ctx: &mut AppContext) {
    let primary_window_and_view = window_id.and_then(|window_id| {
        ctx.root_view_id(window_id)
            .map(|view_id| (window_id, view_id))
    });

    let action = classify_open_file_action(&path);
    if action == OpenFileAction::Notebook {
        if let Some((primary_window_id, root_view_id)) = primary_window_and_view {
            ctx.dispatch_action(
                primary_window_id,
                &[root_view_id],
                "root_view:add_file_pane",
                &path,
                log::Level::Info,
            );
        } else {
            ctx.dispatch_global_action("root_view:open_new_with_file_notebook", &path);
        }
    } else if action == OpenFileAction::Editor {
        #[cfg(feature = "local_fs")]
        {
            use crate::code::editor_management::CodeSource;
            use crate::root_view::{open_new_with_workspace_source, NewWorkspaceSource};
            use crate::util::{
                file::external_editor::EditorSettings,
                openable_file_type::resolve_file_target_to_open_in_warp,
            };

            // Open text/code files in Warp's code editor, respecting the user's layout preference.
            let editor_settings = EditorSettings::as_ref(ctx);
            let target = resolve_file_target_to_open_in_warp(&path, editor_settings, None);

            let window_id = if let Some((wid, _)) = primary_window_and_view {
                wid
            } else {
                open_new_with_workspace_source(
                    NewWorkspaceSource::Session {
                        options: Box::default(),
                    },
                    ctx,
                )
                .0
            };

            ctx.windows().show_window_and_focus_app(window_id);

            if let Some(workspaces) = ctx.views_of_type::<Workspace>(window_id) {
                if let Some(workspace) = workspaces.into_iter().next() {
                    workspace.update(ctx, |workspace, ctx| {
                        let source = CodeSource::Finder { path: path.clone() };
                        workspace.open_file_with_target(path, target, None, source, ctx);
                    });
                }
            }
        }
    } else {
        let directory_path = if path.is_file() {
            match path.parent() {
                Some(parent) => parent.to_path_buf(),
                None => PathBuf::new(),
            }
        } else {
            path.clone()
        };

        if let Some((primary_window_id, root_view_id)) = primary_window_and_view {
            ctx.dispatch_action(
                primary_window_id,
                &[root_view_id],
                "root_view:add_session_at_path",
                &directory_path,
                log::Level::Info,
            );

            // Run command after session has been added
            if path.is_file() {
                if let Some(path_str) = path.to_str() {
                    execute_file(primary_window_id, path_str, ctx);
                }
            }
        } else {
            let open_path = OpenPath {
                path: directory_path,
            };
            ctx.dispatch_global_action("root_view:open_new_from_path", &open_path);

            // Run command after window has been added
            if path.is_file() {
                let active_window_id = ctx.windows().active_window();
                if let Some(primary_window_id) = get_primary_window(active_window_id, ctx) {
                    if let Some(path_str) = path.to_str() {
                        execute_file(primary_window_id, path_str, ctx);
                    }
                }
            }
        }

        send_telemetry_from_app_ctx!(TelemetryEvent::OpenNewSessionFromFilePath, ctx);
    }
}

fn execute_file(window_id: WindowId, path_str: &str, ctx: &mut AppContext) {
    active_terminal_in_window(window_id, ctx, |term, t_ctx| {
        let path_str = term.shell_family(t_ctx).shell_escape(path_str);
        term.input().update(t_ctx, |input, i_ctx| {
            input.set_pending_command(&path_str, i_ctx);
        })
    });

    send_telemetry_from_app_ctx!(TelemetryEvent::CommandFileRun, ctx);
}

fn open_window_with_action(active_window_id: Option<WindowId>, action: &str, ctx: &mut AppContext) {
    if let Some(primary_window_id) = active_window_id {
        // Dispatch action to primary window
        if let Some(root_view_id) = ctx.root_view_id(primary_window_id) {
            ctx.dispatch_action(
                primary_window_id,
                &[root_view_id],
                action,
                &(),
                log::Level::Info,
            );
        }
    } else {
        log::warn!("no primary window id to dispatch action to");

        // Open a new window and dispatch action there
        ctx.dispatch_global_action("root_view:open_new", &());
        // TODO: Note we cannot just dispatch here as it will be a no-op.
        // Need to send a callback once window is fully open.
    }
}

fn find_workspace_for_terminal_view(
    terminal_view_id: EntityId,
    ctx: &mut AppContext,
) -> Option<(WindowId, ViewHandle<Workspace>)> {
    for window_id in ctx.window_ids() {
        let Some(workspaces) = ctx.views_of_type::<Workspace>(window_id) else {
            continue;
        };
        for workspace in workspaces {
            let contains_terminal = workspace
                .as_ref(ctx)
                .list_tab_pane_groups(ctx)
                .iter()
                .any(|group| group.terminal_ids.contains(&terminal_view_id));
            if contains_terminal {
                return Some((window_id, workspace));
            }
        }
    }

    None
}

fn active_terminal_view_id_in_window(window_id: WindowId, ctx: &AppContext) -> Option<EntityId> {
    let workspaces = ctx.views_of_type::<Workspace>(window_id)?;
    let workspace = workspaces.first()?;
    workspace.read(ctx, |workspace, w_ctx| {
        let pane_group = workspace.active_tab_pane_group().as_ref(w_ctx);
        pane_group
            .active_session_view(w_ctx)
            .map(|terminal_view| terminal_view.id())
            .or_else(|| {
                pane_group
                    .terminal_views(w_ctx)
                    .first()
                    .map(|view| view.id())
            })
    })
}

fn find_cloud_mode_terminal_view_id(
    primary_window_id: Option<WindowId>,
    ctx: &AppContext,
) -> Option<EntityId> {
    let mut window_ids = Vec::new();
    if let Some(primary_window_id) = primary_window_id {
        window_ids.push(primary_window_id);
    }
    window_ids.extend(
        ctx.window_ids()
            .filter(|window_id| Some(*window_id) != primary_window_id),
    );

    for window_id in window_ids {
        let Some(workspaces) = ctx.views_of_type::<Workspace>(window_id) else {
            continue;
        };
        for workspace in workspaces {
            if let Some(terminal_view_id) = workspace.read(ctx, |workspace, w_ctx| {
                find_cloud_mode_terminal_in_workspace(workspace, w_ctx)
            }) {
                return Some(terminal_view_id);
            }
        }
    }

    None
}

fn find_cloud_mode_terminal_in_workspace(
    workspace: &Workspace,
    ctx: &AppContext,
) -> Option<EntityId> {
    let mut fallback_ambient_terminal_id = None;

    for pane_group_handle in workspace.tab_views() {
        let pane_group = pane_group_handle.as_ref(ctx);
        let ambient_terminal_id =
            pane_group
                .terminal_views(ctx)
                .into_iter()
                .find_map(|terminal_view| {
                    terminal_view
                        .as_ref(ctx)
                        .ambient_agent_view_model()
                        .is_some()
                        .then_some(terminal_view.id())
                });

        let Some(ambient_terminal_id) = ambient_terminal_id else {
            continue;
        };

        let has_environment_management_pane = pane_group
            .pane_ids()
            .any(|pane_id| pane_id.is_environment_management_pane());
        if has_environment_management_pane {
            return Some(ambient_terminal_id);
        }

        if fallback_ambient_terminal_id.is_none() {
            fallback_ambient_terminal_id = Some(ambient_terminal_id);
        }
    }

    fallback_ambient_terminal_id
}
/// Helper function to dispatch an action to an existing window
/// or create new window if none exist.
fn dispatch_action_in_new_or_existing_window<T: 'static>(
    primary_window_id: Option<WindowId>,
    existing_window_action: &str,
    new_window_action: &str,
    args: &T,
    ctx: &mut AppContext,
) {
    let primary_window_and_view = primary_window_id.and_then(|window_id| {
        ctx.root_view_id(window_id)
            .map(|view_id| (window_id, view_id))
    });

    if let Some((primary_window_id, root_view_id)) = primary_window_and_view {
        ctx.dispatch_action(
            primary_window_id,
            &[root_view_id],
            existing_window_action,
            args,
            log::Level::Info,
        );
    } else {
        ctx.dispatch_global_action(new_window_action, args);
    }
}

/// Validates an incoming custom URI for security and returns the host.
fn validate_custom_uri(url: &Url) -> Result<UriHost> {
    // For now the only scheme we support is `[scheme_name]://[host_str]/...
    // Ignore all other urls that don't match this scheme for security purposes.
    if url.scheme() != ChannelState::url_scheme() {
        return Err(anyhow!(
            "Received url with unexpected scheme: {} ",
            url.scheme()
        ));
    }

    let host_str = url
        .host_str()
        .ok_or_else(|| anyhow!("Received url with no host str"))?;

    let host = UriHost::from_str(host_str)?;

    // Check if this host is allowed to have arbitrary paths.
    let host_allows_arbitrary_path = match host {
        UriHost::Action
        | UriHost::Launch
        | UriHost::SharedSession
        | UriHost::Conversation
        | UriHost::Drive
        | UriHost::Team
        | UriHost::Settings
        | UriHost::Mcp
        | UriHost::Codex
        | UriHost::Linear
        | UriHost::TabConfig
        | UriHost::Session => true,
        // Auth and Home only allow the desktop redirect path
        UriHost::Auth | UriHost::Home => false,
    };

    ensure!(
        host_allows_arbitrary_path || url.path() == DESKTOP_REDIRECT_URI_PATH,
        "Received url with unexpected path: {} ",
        url.path()
    );

    Ok(host)
}

/// Formats the non-sensitive components of an incoming URL for logging on
/// release channels.
///
/// The returned string contains only the URL's scheme, host, and path — never
/// its query string, fragment, or userinfo component. URLs that reach
/// [`handle_incoming_uri`] can carry secrets in their query (for example, the
/// Firebase refresh token in `warp://auth/desktop_redirect?refresh_token=...`),
/// so this helper exists to give [`safe_info!`] a redacted representation that
/// still preserves enough signal for triage.
///
/// `url.host_str()` can return `None` for schemes that don't require a host
/// (e.g. some `file://` URLs on certain platforms); the literal `-` is used
/// as a placeholder in that case so the formatter never panics.
fn safe_url_log_fields(url: &Url) -> String {
    format!(
        "scheme={} host={} path={}",
        url.scheme(),
        url.host_str().unwrap_or("-"),
        url.path(),
    )
}

fn decode_uuid_hex(hex: &str) -> Option<Vec<u8>> {
    let hex = hex.as_bytes();
    if hex.len() != 32 {
        return None;
    }

    hex.chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
}

#[cfg(test)]
#[path = "uri_tests.rs"]
mod tests;
