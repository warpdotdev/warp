mod docker;

use crate::launch_configs::launch_config::LaunchConfig;
use crate::linear::{LinearAction, LinearIssueWork};
use crate::root_view::{open_new_window_get_handles, OpenLaunchConfigArg};
use crate::util::openable_file_type::{is_file_openable_in_warp, is_markdown_file};
use crate::workspace::active_terminal_in_window;
use crate::workspace::metadata::LaunchConfigUiLocation;
use crate::workspace::{Workspace, WorkspaceAction, WorkspaceRegistry};
use crate::{view_components::DismissibleToast, workspace::ToastStack};

use crate::settings_view::SettingsSection;
use crate::user_config::load_launch_configs;
use crate::{quake_mode_window_id, quake_mode_window_is_open, safe_info, ChannelState, OpenPath};
use anyhow::{anyhow, ensure, Result};
use std::path::PathBuf;
use std::str::FromStr;
use url::Url;
use warpui::notification::UserNotification;
use warpui::{platform::TerminationMode, SingletonEntity as _, TypedActionView};

use warpui::{AppContext, WindowId};

use self::docker::open_docker_container;

const DESKTOP_REDIRECT_URI_PATH: &str = "/desktop_redirect";

/// Args for opening the MCP settings page via deeplink.
pub struct OpenMCPSettingsArgs;

#[derive(Debug, PartialEq, Eq)]
pub enum UriHost {
    /// A host prefix for all actions (e.g.: new tab, new window).
    Action,
    /// A host prefix for all actions that involve launch configurations
    Launch,
    /// Supports opening warp's settings panel via URI
    Settings,
    /// A host prefix for a general-purpose home/landing page. Unlike other intent URIs, the home
    /// page behavior may change over time and vary from platform to platform.
    Home,
    /// Actions related to MCP servers (e.g.: oauth callbacks).
    Mcp,
    /// Actions triggered from Linear integrations (e.g. work on issue).
    Linear,
}

impl FromStr for UriHost {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "action" => Ok(Self::Action),
            "launch" => Ok(Self::Launch),
            "settings" => Ok(Self::Settings),
            "home" => Ok(Self::Home),
            "mcp" => Ok(Self::Mcp),
            "linear" => Ok(Self::Linear),
            _ => Err(anyhow!("Received url with unexpected host: {}", s)),
        }
    }
}

impl UriHost {
    fn handle(&self, primary_window_id: Option<WindowId>, url: &Url, ctx: &mut AppContext) {
        // Handle host
        match self {
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
            UriHost::Settings => {
                // We support opening different settings pages through URI:
                // - warp://settings/mcp - opens MCP servers settings page
                // - warp://settings/appearance - opens appearance settings page (themes, fonts, etc.)
                let settings_sub_page: Option<String> = url
                    .path_segments()
                    .into_iter()
                    .flatten()
                    .last()
                    .map(|s| s.to_string());
                if let Some(settings_sub_page) = settings_sub_page {
                    match settings_sub_page.as_str() {
                        "mcp" => {
                            let args = OpenMCPSettingsArgs;
                            dispatch_action_in_new_or_existing_window(
                                primary_window_id,
                                "root_view:open_mcp_settings_in_existing_window",
                                "root_view:open_mcp_settings_in_new_window",
                                &args,
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
                            log::warn!(
                                "Rejected unsupported local-only settings pane with uri={url}"
                            );
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
                let result = crate::ai::mcp::TemplatableMCPServerManager::handle(ctx)
                    .update(ctx, |manager, _ctx| manager.handle_oauth_callback(url));
                if let Err(e) = result {
                    log::error!("Failed to handle MCP OAuth callback: {e:?}");
                }
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
        }
    }

    /// When handling this URI action, determine which window(s) should be focused.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn window_behavior_hint(&self) -> WindowBehaviorHint {
        use WindowBehaviorHint as W;
        match self {
            Self::Settings => W::default(),
            // These URLs always open new windows.
            Self::Launch | Self::Home => W::Nothing,
            // This will actually be handled by [`Action::window_behavior_hint`].
            Self::Action => W::Nothing,
            // TODO(vorporeal): probably want to focus the window with the MCP pane open
            Self::Mcp => W::Nothing,
            // Linear deeplink opens a new tab with agent view
            Self::Linear => W::default(),
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
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
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
        /// may still want.
        replace_existing: bool,
    },
}

impl WindowActivationFallbackBehavior {
    /// Perform the desired window fallback behavior for the URI being handled. This may change the
    /// "primary window" if a new one has to be created. Return the new primary WindowId.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
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
    NewAgentConversation,
}

impl Action {
    fn parse(url: &Url) -> Result<Self> {
        match url.path() {
            "/new_tab" => Ok(Self::NewTab),
            "/new_window" => Ok(Self::NewWindow),
            "/docker/open_subshell" => Ok(Self::Docker),
            "/open-repo" => Ok(Self::OpenRepo),
            "/new_agent_conversation" => Ok(Self::NewAgentConversation),
            _ => Err(anyhow!(
                "Received \"action\" intent with unexpected action: {}",
                url.path()
            )),
        }
    }

    fn handle(&self, primary_window_id: Option<WindowId>, url: &Url, ctx: &mut AppContext) {
        #[cfg(target_os = "linux")]
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
        }
    }

    /// When handling this URI action, determine which window(s) should be focused.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn window_behavior_hint(&self) -> WindowBehaviorHint {
        use WindowBehaviorHint as W;
        match self {
            Self::Docker | Self::OpenRepo | Self::NewAgentConversation => W::default(),
            Self::NewTab => W::ShowPrimaryWindow(WindowActivationFallbackBehavior::Notify {
                title: "New tab created".to_owned(),
                description: "Go to Warper to see your new tab.".to_owned(),
            }),
            Self::NewWindow => W::Nothing,
        }
    }
}

/// Handles all incoming file and local-only custom URLs.
pub fn handle_incoming_uri(url: &Url, ctx: &mut AppContext) {
    // Non-dogfood builds must never log the full URL here: URLs routed to this
    // handler can carry secrets in their query string. Log only the
    // non-sensitive components (scheme, host, path) on release channels;
    // dogfood builds retain the full URL for local debugging.
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
            #[cfg(any(target_os = "linux", windows))]
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

    if is_markdown_file(&path) {
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
    } else if path.is_file() && is_file_openable_in_warp(&path).is_some() {
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
    }
}

fn execute_file(window_id: WindowId, path_str: &str, ctx: &mut AppContext) {
    active_terminal_in_window(window_id, ctx, |term, t_ctx| {
        let path_str = term.shell_family(t_ctx).shell_escape(path_str);
        term.input().update(t_ctx, |input, i_ctx| {
            input.set_pending_command(&path_str, i_ctx);
        })
    });
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
    if matches!(host, UriHost::Settings) && !settings_uri_is_supported_local_page(url) {
        return Err(anyhow!(
            "Received unsupported local-only settings url: {}",
            url.path()
        ));
    }

    // Check if this host is allowed to have arbitrary paths.
    let host_allows_arbitrary_path = match host {
        UriHost::Action | UriHost::Launch | UriHost::Settings | UriHost::Mcp | UriHost::Linear => {
            true
        }
        // Home only allows the desktop redirect path
        UriHost::Home => false,
    };

    ensure!(
        host_allows_arbitrary_path || url.path() == DESKTOP_REDIRECT_URI_PATH,
        "Received url with unexpected path: {} ",
        url.path()
    );

    Ok(host)
}

fn settings_uri_is_supported_local_page(url: &Url) -> bool {
    let Some(settings_sub_page) = url.path_segments().into_iter().flatten().last() else {
        return false;
    };

    matches!(settings_sub_page, "appearance" | "mcp")
}

/// Formats the non-sensitive components of an incoming URL for logging on
/// release channels.
///
/// The returned string contains only the URL's scheme, host, and path — never
/// its query string, fragment, or userinfo component. URLs that reach
/// [`handle_incoming_uri`] can carry secrets in their query, so this helper
/// exists to give [`safe_info!`] a redacted representation that
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

#[cfg(test)]
#[path = "uri_test.rs"]
mod tests;
