use std::io::Write;
use std::path::PathBuf;

use anyhow::{bail, Context as _, Result};
use channel_versions::VersionInfo;
use instant::Duration;
use warp_core::channel::{Channel, ChannelState};
use warp_terminal::shell::ShellType;
use warpui::ViewContext;

use crate::workspace::Workspace;

use super::release_assets_directory_url;
use super::{DownloadReady, ReadyForRelaunch};

lazy_static::lazy_static! {
    /// Stores the path to the current executable.
    ///
    /// We cache this before running auto-update because the returned path for
    /// a deleted file includes " (deleted)" _in the file name_, which breaks
    /// the relaunch logic.
    static ref CURRENT_EXE: std::io::Result<PathBuf> = std::env::current_exe();
}

pub(super) async fn download_update_and_cleanup(
    version_info: &VersionInfo,
    _update_id: &str,
    client: &http_client::Client,
) -> Result<DownloadReady> {
    match UpdateMethod::detect() {
        UpdateMethod::Unknown => Ok(DownloadReady::NeedsAuthorization),
        UpdateMethod::AppImage(appimage_path) => {
            appimage::download_update_and_cleanup(version_info, &appimage_path, client).await
        }
        UpdateMethod::PackageManager(package_manager) => {
            log::info!("Detected that Warp was installed using {package_manager:?}");
            Ok(DownloadReady::Yes)
        }
    }
}

pub(super) fn apply_update(
    initiating_workspace: &mut Workspace,
    update_id: &str,
    ctx: &mut ViewContext<Workspace>,
) -> Result<ReadyForRelaunch> {
    // Make sure CURRENT_EXE is initialized before we actually apply the update.
    let _ = CURRENT_EXE.as_ref();

    match UpdateMethod::detect() {
        UpdateMethod::Unknown => bail!("Cannot apply update for unknown update method!"),
        UpdateMethod::AppImage(_) => Ok(ReadyForRelaunch::Yes),
        UpdateMethod::PackageManager(package_manager) => {
            let context_block =
                ctx.add_view(|_| package_manager::AutoupdateContextBlock::new(package_manager));
            let owned_update_id = update_id.to_owned();
            initiating_workspace.add_tab_for_assisted_autoupdate(
                move |shell_type| package_manager.update_command(shell_type, &owned_update_id),
                context_block,
                ctx,
            );
            Ok(ReadyForRelaunch::No)
        }
    }
}

pub(super) fn relaunch() -> Result<()> {
    match UpdateMethod::detect() {
        UpdateMethod::Unknown => bail!("Don't know how to relaunch for an unknown update method!"),
        UpdateMethod::AppImage(appimage_path) => appimage::relaunch(&appimage_path),
        UpdateMethod::PackageManager(_) => package_manager::relaunch(),
    }
}

mod appimage {
    use std::path::Path;

    use super::*;

    pub(super) async fn download_update_and_cleanup(
        version_info: &VersionInfo,
        appimage_path: &Path,
        client: &http_client::Client,
    ) -> Result<DownloadReady> {
        const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

        // Compute the URL where we can download the new release.
        let Some(appimage_name) = option_env!("APPIMAGE_NAME") else {
            bail!("APPIMAGE_NAME environment variable was not set at compile time!");
        };

        let url = format!(
            "{}/{}",
            release_assets_directory_url(ChannelState::channel(), &version_info.version),
            appimage_name
        );

        // Create a temporary file that we'll write the download into.
        let mut new_appimage = tempfile::NamedTempFile::new()?;

        log::info!("Downloading {url} to {}...", new_appimage.path().display());

        let response = client
            .get(&url)
            .timeout(DOWNLOAD_TIMEOUT)
            .send()
            .await?
            .error_for_status()?;
        new_appimage
            .as_file_mut()
            .write_all(&response.bytes().await?)?;

        log::info!(
            "Copying downloaded AppImage from {} to {}",
            new_appimage.path().display(),
            appimage_path.display()
        );

        // Copy permissions to new app before moving it to ensure we don't leave it
        // in a bad state if the move succeeds but we are unable to update the
        // permissions afterwards.
        new_appimage
            .as_file_mut()
            .set_permissions(appimage_path.metadata()?.permissions())?;

        // Move new AppImage over the one that launched the current Warp instance.
        let new_appimage_path = new_appimage.into_temp_path();
        let mv_status = command::r#async::Command::new("mv")
            .arg(new_appimage_path.as_os_str())
            .arg(appimage_path)
            .output()
            .await?
            .status;
        if !mv_status.success() {
            bail!("Failed to move new AppImage over the old one: {mv_status}");
        }

        // Ensure we don't accidentally drop `new_appimage_path` before we finish
        // moving it to its final location.
        let _ = new_appimage_path;

        Ok(DownloadReady::Yes)
    }

    pub(super) fn relaunch(appimage_path: &Path) -> Result<()> {
        let mut command = command::blocking::Command::new(appimage_path);
        // Pass a flag to the app to let it know it was restarted as part of the
        // autoupdate process.
        command.arg(warp_cli::finish_update_flag());
        // If we're testing with a local copy of channel_versions.json, have the
        // newly-started binary also reference that same file (so we can test
        // displaying an updated changelog after an autoupdate).
        if let Ok(path) = std::env::var("WARP_CHANNEL_VERSIONS_PATH") {
            command.env("WARP_CHANNEL_VERSIONS_PATH", path);
        }

        log::info!("Relaunching warp for update...");
        command.spawn()?;
        Ok(())
    }
}

mod package_manager {
    use markdown_parser::{
        FormattedText, FormattedTextFragment, FormattedTextHeader, FormattedTextLine,
    };
    use warpui::{
        elements::{Container, FormattedTextElement, HighlightedHyperlink},
        Element, SingletonEntity as _,
    };

    use crate::appearance::Appearance;

    use super::*;

    pub struct AutoupdateContextBlock {
        package_manager: PackageManager,
        hyperlink: HighlightedHyperlink,
    }

    impl AutoupdateContextBlock {
        pub fn new(package_manager: PackageManager) -> Self {
            AutoupdateContextBlock {
                package_manager,
                hyperlink: Default::default(),
            }
        }
    }

    impl warpui::Entity for AutoupdateContextBlock {
        type Event = ();
    }

    impl warpui::View for AutoupdateContextBlock {
        fn ui_name() -> &'static str {
            "AutoupdateContextBlock"
        }

        fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
            let appearance = Appearance::as_ref(app);
            let theme = appearance.theme();
            let package_manager_name = self.package_manager.to_string();

            let mut lines = vec![
                FormattedTextLine::Heading(FormattedTextHeader {
                    // Make this an <h3>
                    heading_size: 3,
                    text: vec![FormattedTextFragment::bold(format!(
                        "Run {package_manager_name} to update"
                    ))],
                }),
                FormattedTextLine::Line(vec![
                    FormattedTextFragment::plain_text("If you installed Warp using "),
                    FormattedTextFragment::bold(package_manager_name),
                    FormattedTextFragment::plain_text(
                        " or a compatible tool, the pre-filled command will update Warp for you.",
                    ),
                ]),
            ];

            if self.package_manager.needs_repository_configuration() {
                lines.push(FormattedTextLine::Line(vec![
                    FormattedTextFragment::plain_text(
                        "\nThe command below includes a one-time configuration of the Warp package repository and PGP signing key.",
                    ),
                ]));
            }

            if self
                .package_manager
                .distribution_update_disabled_repository()
            {
                lines.push(FormattedTextLine::Line(vec![
                    FormattedTextFragment::plain_text(
                        "\nThe ",
                    ),
                    FormattedTextFragment::inline_code("warp_handle_dist_upgrade"),
                    FormattedTextFragment::plain_text(
                        " function ensures the Warp package repository is enabled, as we've detected you recently upgraded your distribution.",
                    ),
                ]));
            }

            lines.push(FormattedTextLine::Line(vec![
                FormattedTextFragment::plain_text("\nReview the command below, then "),
                FormattedTextFragment::bold("press enter"),
                FormattedTextFragment::plain_text(" to install the update and re-launch Warp.  "),
                FormattedTextFragment::hyperlink(
                    "Please report any issues",
                    "https://github.com/warpdotdev/Warp/issues/new/choose",
                ),
            ]));

            let formatted_text = FormattedText::new(lines);
            let inline_code_bg_color = appearance.theme().surface_3().into_solid();

            let text = FormattedTextElement::new(
                formatted_text,
                appearance.monospace_font_size(),
                appearance.monospace_font_family(),
                appearance.monospace_font_family(),
                theme.active_ui_text_color().into_solid(),
                self.hyperlink.clone(),
            )
            .with_inline_code_properties(
                Some(theme.nonactive_ui_text_color().into()),
                Some(inline_code_bg_color),
            )
            .register_default_click_handlers(|url, _, ctx| {
                ctx.open_url(&url.url);
            })
            .finish();

            Container::new(text)
                .with_background(theme.surface_2())
                .with_uniform_padding(16.)
                .finish()
        }
    }

    pub(super) fn relaunch() -> Result<()> {
        let Ok(program) = CURRENT_EXE.as_ref() else {
            bail!(
                "Failed to get path to current executable to relaunch after completing auto-update"
            );
        };
        log::info!("Relaunching using path: {program:?}");
        let mut command = command::blocking::Command::new(program);
        // Add any arguments that were passed to warp, skipping the first
        // argument (the name of the executable) and dropping the flag for
        // finishing an update.
        let finish_update_flag = warp_cli::finish_update_flag();
        command.args(
            std::env::args()
                .skip(1)
                .filter(|arg| arg != &finish_update_flag),
        );
        // Pass a flag to the app to let it know it was restarted as part of the
        // autoupdate process.
        command.arg(finish_update_flag);
        // If we're testing with a local copy of channel_versions.json, have the
        // newly-started binary also reference that same file (so we can test
        // displaying an updated changelog after an autoupdate).
        if let Ok(path) = std::env::var("WARP_CHANNEL_VERSIONS_PATH") {
            command.env("WARP_CHANNEL_VERSIONS_PATH", path);
        }

        log::info!("Relaunching warp for update...");
        command.spawn()?;
        Ok(())
    }
}

/// Returns which method should be used to update Warp.
#[derive(Debug)]
pub(crate) enum UpdateMethod {
    /// We don't know how to update Warp.
    Unknown,
    /// Warp is running as an AppImage and should be updated in-place.
    AppImage(PathBuf),
    /// Warp can be updated using the given package manager.
    PackageManager(PackageManager),
}

impl UpdateMethod {
    pub(crate) fn detect() -> Self {
        if let Some(appimage_path) = std::env::var_os("APPIMAGE").map(PathBuf::from) {
            return Self::AppImage(appimage_path);
        }
        if let Ok(package_manager) = PackageManager::detect() {
            return Self::PackageManager(package_manager);
        }
        Self::Unknown
    }
}

/// Package managers that we understand and can assist with auto-update
/// for.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PackageManager {
    Apt {
        distribution_update_disabled_repository: bool,
    },
    Yum,
    Dnf,
    Zypper,
    Pacman {
        is_repo_configured: bool,
        is_signing_key_configured: bool,
    },
}

impl PackageManager {
    pub fn update_command(&self, shell_type: ShellType, update_id: &str) -> String {
        let package_name = Self::package_name();
        let repo_name = Self::repo_name();
        let and = shell_type.and_combiner();
        let or = shell_type.or_combiner();

        let base_command = match self {
            PackageManager::Apt {
                distribution_update_disabled_repository,
            } => {
                let dist_upgrade_fn = match shell_type {
                    ShellType::Zsh | ShellType::Bash | ShellType::Fish => {
                        "warp_handle_dist_upgrade"
                    }
                    ShellType::PowerShell => "Warp-Handle-DistUpgrade",
                };
                // If running with apt, attempt to handle a distribution update that may rename the
                // warp source file to `{repo_name}.distUpgrade`.
                // We explicitly use `or` here instead of `and` to limit the blast radius of this
                // change, if handling a dist upgrade was unsuccessful we still want to try to
                // install the new version.
                let command = format!("sudo apt update{and}sudo apt install {package_name}");
                if *distribution_update_disabled_repository {
                    format!("{dist_upgrade_fn} {repo_name}{or}{command}")
                } else {
                    command
                }
            }
            PackageManager::Yum => {
                format!("sudo yum --refresh --repo {repo_name} upgrade {package_name}")
            }
            PackageManager::Dnf => {
                format!("sudo dnf --refresh --repo {repo_name} upgrade {package_name}")
            }
            PackageManager::Zypper => {
                format!("sudo zypper update {package_name}")
            }
            PackageManager::Pacman {
                is_repo_configured,
                is_signing_key_configured,
            } => {
                let repo_prefix = if !is_repo_configured {
                    let cache_dir = warp_core::paths::cache_dir();
                    let cache_dir_str = cache_dir.display();
                    // Back up the existing pacman.conf file just in case
                    // anything goes wrong, then add the repository config.
                    format!("mkdir -p {cache_dir_str}{and}\\\ncp /etc/pacman.conf {cache_dir_str}{and}\\\nsudo sh -c \"echo '\n[{repo_name}]\nServer = https://releases.warp.dev/linux/pacman/\\$repo/\\$arch' >> /etc/pacman.conf\"{and}\\\n")
                } else {
                    String::new()
                };
                let key_prefix = if !is_signing_key_configured {
                    // Retrieve our key from keys.openpgp.org and locally sign
                    // it before retrieving the package repository and
                    // installing the updated package.
                    format!("sudo pacman-key -r \"linux-maintainers@warp.dev\" --keyserver hkp://keys.openpgp.org:80{and}\\\nsudo pacman-key --lsign-key \"linux-maintainers@warp.dev\"{and}\\\n")
                } else {
                    String::new()
                };
                format!("{key_prefix}{repo_prefix}sudo pacman -Sy {package_name}")
            }
        };

        let finish_update_fn = match shell_type {
            ShellType::Zsh | ShellType::Bash | ShellType::Fish => "warp_finish_update",
            ShellType::PowerShell => "Warp-Finish-Update",
        };
        format!("{base_command}{and}{finish_update_fn} {update_id}")
    }

    fn package_name() -> &'static str {
        package_name(ChannelState::channel())
    }

    fn repo_name() -> String {
        repo_name(ChannelState::channel())
    }

    fn detect() -> Result<Self> {
        let package_name = Self::package_name();

        let detect_script = r#"
            command -p pacman -Qi $PACKAGE_NAME >/dev/null 2>/dev/null
            if [ $? -eq 0 ]; then
              echo "pacman"
              exit
            fi

            command -p zypper search --match-exact --installed-only $PACKAGE_NAME >/dev/null 2>/dev/null
            if [ $? -eq 0 ]; then
              echo "zypper"
              exit
            fi

            command -p dnf list --installed $PACKAGE_NAME >/dev/null 2>/dev/null
            if [ $? -eq 0 ]; then
              echo "dnf"
              exit
            fi

            command -p yum list installed $PACKAGE_NAME >/dev/null 2>/dev/null
            if [ $? -eq 0 ]; then
              echo "yum"
              exit
            fi

            if [ "$(command -p dpkg-query --show --showformat='${db:Status-Status}' $PACKAGE_NAME 2>/dev/null)" = "installed" ]; then
              echo "apt"
              exit
            fi

            exit 1
        "#;

        let output = command::blocking::Command::new("sh")
            .args(["-c", detect_script])
            .env("PACKAGE_NAME", package_name)
            .output();
        match output {
            Ok(output) => {
                if !output.status.success() {
                    bail!("Failed to determine which package manager was used to install warp");
                }
                let Ok(stdout) = std::str::from_utf8(&output.stdout) else {
                    bail!("Could not parse package manager detection script output as UTF-8");
                };
                match stdout.trim() {
                    "pacman" => {
                        let is_repo_configured = is_pacman_repo_installed(package_name);
                        let is_signing_key_configured = is_pacman_signing_key_installed();
                        Ok(Self::Pacman {
                            is_repo_configured,
                            is_signing_key_configured,
                        })
                    }
                    "zypper" => Ok(Self::Zypper),
                    "dnf" => Ok(Self::Dnf),
                    "yum" => Ok(Self::Yum),
                    "apt" => {
                        let distribution_update_disabled_repository =
                            is_apt_repository_disabled_due_to_version_update(&Self::repo_name());
                        Ok(Self::Apt {
                            distribution_update_disabled_repository,
                        })
                    }
                    _ => bail!(
                        "Received unexpected output from the package manager detection script"
                    ),
                }
            }
            Err(err) => Err(err).context("Failed to run package manager detection script"),
        }
    }

    fn distribution_update_disabled_repository(&self) -> bool {
        match self {
            PackageManager::Apt {
                distribution_update_disabled_repository,
            } => *distribution_update_disabled_repository,
            _ => false,
        }
    }

    fn needs_repository_configuration(&self) -> bool {
        match self {
            PackageManager::Pacman {
                is_repo_configured,
                is_signing_key_configured,
            } => !is_repo_configured || !is_signing_key_configured,
            // We only need to perform in-app post-installation repo configuration
            // when using pacman, and not with other package managers.
            PackageManager::Apt { .. }
            | PackageManager::Yum
            | PackageManager::Dnf
            | PackageManager::Zypper => false,
        }
    }
}

impl std::fmt::Display for PackageManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageManager::Apt { .. } => write!(f, "apt"),
            PackageManager::Yum => write!(f, "yum"),
            PackageManager::Dnf => write!(f, "dnf"),
            PackageManager::Zypper => write!(f, "zypper"),
            PackageManager::Pacman { .. } => write!(f, "pacman"),
        }
    }
}

/// Returns whether the warp apt repository is disabled due to a version update.
/// This occurs if there's a `warpdotdev.list.distUpgrade` file but no `warpdotdev.sources` or
/// `warpdotdev.list` file.
/// In a traditional Ubuntu distro update, Ubuntu renames each source file from `foo.list` to
/// `foo.list.distUpgrade`. It then creates a new version of `foo.list` (or `foo.sources` if
/// updating to Ubuntu 24+) with the repo disabled.
///
/// However, Ubuntu incorrectly thinks the Warp source file is invalid (due to the addition of the
/// `signed-by` key) so it only leaves the `*.distUpgrade` source file. We use the existence of this
/// file to determine whether we need to run the special `warp_handle_dist_upgrade` function to copy
/// `warpdotdev.list.distUpgrade` back to `warpdotdev.list` to re-enable the repository.
fn is_apt_repository_disabled_due_to_version_update(repo_name: &str) -> bool {
    let apt_sources_directory = match get_apt_sources_directory() {
        Ok(apt_sources_directory) => apt_sources_directory,
        Err(err) => {
            log::warn!("Failed to compute default apt source list directory: {err:#}");
            log::warn!("Falling back to /etc/apt/sources.list.d/...");
            PathBuf::from("/etc/apt/sources.list.d/")
        }
    };

    !apt_sources_directory
        .join(format!("{repo_name}.list"))
        .exists()
        && !apt_sources_directory
            .join(format!("{repo_name}.sources"))
            .exists()
        && apt_sources_directory
            .join(format!("{repo_name}.list.distUpgrade"))
            .exists()
}

/// Returns the directory that contains apt sources.
fn get_apt_sources_directory() -> Result<PathBuf> {
    let output = command::blocking::Command::new("sh")
        .arg("-c")
        .arg("eval $(apt-config shell APT_SOURCESDIR \"Dir::Etc::sourceparts/d\"); echo $APT_SOURCESDIR")
        .output()?;
    let stdout = std::str::from_utf8(&output.stdout)
        .context("FAiled to parse apt sources directory script output")?;

    Ok(PathBuf::from(stdout.trim()))
}

fn is_pacman_repo_installed(package_name: &str) -> bool {
    match command::blocking::Command::new("pacman")
        .arg("-S")
        .arg("--print")
        .arg(package_name)
        .output()
    {
        Ok(output) => output.status.success(),
        Err(err) => {
            log::warn!("Failed to determine if pacman repository is configured: {err:#}");
            // Fail open, to ensure we don't insert duplicate entries in /etc/pacman.conf.
            true
        }
    }
}

fn is_pacman_signing_key_installed() -> bool {
    // Check if the key exists and get its expiry date from pacman's GPG keyring.
    let output = match command::blocking::Command::new("gpg")
        .args([
            "--homedir",
            "/etc/pacman.d/gnupg",
            "--list-keys",
            "--with-colons",
            "linux-maintainers@warp.dev",
        ])
        .output()
    {
        Ok(output) if output.status.success() => output,
        Ok(_) => return false, // Key not found.
        Err(err) => {
            log::warn!("Failed to check pacman signing key: {err:#}");
            // If we're not sure, try to refresh the key.
            return false;
        }
    };

    let Ok(stdout) = std::str::from_utf8(&output.stdout) else {
        return false;
    };

    // After parsing the pub: line, also check validity field (index 1 = validity)
    let fields: Vec<&str> = stdout
        .lines()
        .find(|line| line.starts_with("pub:"))
        .map(|line| line.split(':').collect())
        .unwrap_or_default();

    // Field index 1 = validity: 'f' (full), 'u' (ultimate) are valid;
    // 'e' (expired), 'r' (revoked), '-', 'q' = invalid
    let validity = fields
        .get(1)
        .and_then(|field| field.chars().next())
        .unwrap_or('\0');
    if !matches!(validity, 'f' | 'u') {
        return false; // Force key reconfiguration
    }

    // Parse the expiry timestamp from the pub: line (field 7, 1-indexed).
    let Some(expiry_field) = stdout
        .lines()
        .find(|line| line.starts_with("pub:"))
        .and_then(|line| line.split(':').nth(6))
    else {
        // Couldn't find pub line, try to refresh.
        return false;
    };

    // An empty field or "0" means the key has no expiration date.
    if expiry_field.is_empty() || expiry_field == "0" {
        return true;
    }

    let Ok(expiry_timestamp) = expiry_field.parse::<i64>() else {
        // Couldn't parse expiry, try to refresh.
        return false;
    };

    // If the key expires within 60 days, consider it as needing refresh.
    let sixty_days_from_now = chrono::Utc::now() + chrono::Duration::days(60);
    expiry_timestamp > sixty_days_from_now.timestamp()
}

fn package_name(channel: Channel) -> &'static str {
    match channel {
        Channel::Stable => "warp-terminal",
        Channel::Preview => "warp-terminal-preview",
        Channel::Dev => "warp-terminal-dev",
        Channel::Integration => "warp-terminal-integration",
        Channel::Local => "warp-terminal-local",
        Channel::Oss => "warp-oss",
    }
}

fn repo_name(channel: Channel) -> String {
    let package_name = package_name(channel);
    let channel_suffix = package_name
        .strip_prefix("warp-terminal")
        .unwrap_or_default();
    format!("warpdotdev{channel_suffix}")
}

#[cfg(test)]
#[path = "linux_tests.rs"]
mod tests;
