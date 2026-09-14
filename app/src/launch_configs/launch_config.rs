use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

use crate::app_state::{
    AppState, LeafContents, PaneNodeSnapshot, SplitDirection as StateSplitDirection,
    TabGroupSnapshot, TabSnapshot, WindowSnapshot,
};
use crate::themes::theme::AnsiColorIdentifier;

#[cfg(test)]
#[path = "launch_config_tests.rs"]
mod tests;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct LaunchConfig {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub active_window_index: Option<usize>,
    pub windows: Vec<WindowTemplate>,
}

impl LaunchConfig {
    pub fn from_snapshot(name: String, app_state: &AppState) -> Self {
        Self {
            name,
            active_window_index: app_state.active_window_index,
            windows: app_state
                .windows
                .iter()
                .filter_map(|window| (!window.quake_mode).then_some(window.clone().into()))
                .collect::<Vec<WindowTemplate>>(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct WindowTemplate {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub active_tab_index: Option<usize>,
    pub tabs: Vec<TabTemplate>,
    /// Tab groups in this window, in tab-bar order. A tab joins one by
    /// index through [`TabTemplate::group`]; runtime `TabGroupId`s are not
    /// serialized because they are regenerated on every restore.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tab_groups: Vec<TabGroupTemplate>,
}

/// A tab group as stored in a launch config.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct TabGroupTemplate {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub color: Option<AnsiColorIdentifier>,
    #[serde(skip_serializing_if = "is_false", default)]
    pub collapsed: bool,
    #[serde(skip_serializing_if = "is_false", default)]
    pub pinned: bool,
}

impl From<&TabGroupSnapshot> for TabGroupTemplate {
    fn from(snapshot: &TabGroupSnapshot) -> Self {
        Self {
            name: snapshot.name.clone(),
            // Groups have no default-directory color to fall back to, so the
            // manual selection is the whole story here.
            color: snapshot.color.resolve(None),
            collapsed: snapshot.collapsed,
            pinned: snapshot.pinned,
        }
    }
}

impl From<WindowSnapshot> for WindowTemplate {
    fn from(snapshot: WindowSnapshot) -> Self {
        let mut active_tab_index = None;
        let mut num_valid_tabs = 0;

        // A tab can fail to convert, so group membership has to be carried on
        // the tab's own `group_id` rather than inferred from its position --
        // the surviving tabs are renumbered below and the two indices diverge.
        let tabs_with_groups = snapshot
            .tabs
            .into_iter()
            .enumerate()
            .filter_map(|(i, tab)| {
                let group_id = tab.group_id;
                let tab: TabTemplate = tab.try_into().ok()?;

                if i == snapshot.active_tab_index {
                    active_tab_index = Some(num_valid_tabs);
                }

                num_valid_tabs += 1;

                Some((tab, group_id))
            })
            .collect::<Vec<_>>();

        // Keep only groups that still have a member, so a config never
        // restores an empty group the user cannot see or remove.
        let tab_groups = snapshot
            .tab_groups
            .iter()
            .filter(|group| {
                tabs_with_groups
                    .iter()
                    .any(|(_, group_id)| *group_id == Some(group.id))
            })
            .collect::<Vec<_>>();

        let tabs = tabs_with_groups
            .into_iter()
            .map(|(mut tab, group_id)| {
                tab.group = group_id
                    .and_then(|group_id| tab_groups.iter().position(|group| group.id == group_id));
                tab
            })
            .collect::<Vec<TabTemplate>>();

        Self {
            active_tab_index,
            tabs,
            tab_groups: tab_groups.into_iter().map(TabGroupTemplate::from).collect(),
        }
    }
}

fn is_false(val: &bool) -> bool {
    !*val
}

/// Resolves each tab's group index for restore, keeping every group to a
/// single contiguous run.
///
/// The tab bar collapses each *contiguous* run of same-group tabs into one
/// group container (`Workspace::tab_bar_slots`), so interleaved membership --
/// group 0, an ungrouped tab, group 0 again -- would render as two containers
/// sharing one id, which no other code path can produce. Configs written by
/// `From<WindowSnapshot>` are always contiguous because a live window is, so
/// this only bites on hand-edited YAML.
///
/// The first run of each group wins and later stragglers come back ungrouped.
/// Reordering the tabs would also restore the invariant, but silently moving
/// tabs the config explicitly ordered is the more surprising of the two.
/// Out-of-range indices are dropped the same way.
pub fn resolve_group_memberships(tabs: &[TabTemplate], group_count: usize) -> Vec<Option<usize>> {
    let mut closed: Vec<bool> = vec![false; group_count];
    let mut previous: Option<usize> = None;

    tabs.iter()
        .map(|tab| {
            let group = tab
                .group
                .filter(|index| *index < group_count)
                .filter(|index| !closed[*index]);

            if previous != group
                && let Some(previous) = previous
            {
                closed[previous] = true;
            }
            previous = group;
            group
        })
        .collect()
}

fn is_falsey(val: &Option<bool>) -> bool {
    val.is_none_or(|v| !v)
}

/// The mode a leaf pane opens in.
///
/// Used by tab configs to distinguish terminal, agent, and cloud panes.
/// Launch configs always produce `Terminal` (the default).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PaneMode {
    /// A standard terminal shell session.
    #[default]
    Terminal,
    /// A terminal that immediately enters Agent Mode.
    Agent,
    /// A cloud-mode (ambient agent) pane with no local shell.
    Cloud,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(untagged, rename_all = "lowercase")]
pub enum PaneTemplateType {
    PaneTemplate {
        #[serde(deserialize_with = "deserialize_path")]
        cwd: PathBuf,
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        commands: Vec<CommandTemplate>,
        #[serde(skip_serializing_if = "is_falsey", default)]
        is_focused: Option<bool>,
        #[serde(default)]
        pane_mode: PaneMode,
        /// Optional shell override for this pane (e.g. `"pwsh"`, `"zsh"`).
        /// Sourced from the `shell` field of a tab config pane node.
        #[serde(skip_serializing_if = "Option::is_none", default)]
        shell: Option<String>,
    },
    PaneBranchTemplate {
        split_direction: SplitDirection,
        panes: Vec<PaneTemplateType>,
    },
}

impl TryFrom<PaneNodeSnapshot> for PaneTemplateType {
    type Error = ();

    #[allow(clippy::unwrap_in_result)]
    fn try_from(snapshot: PaneNodeSnapshot) -> Result<Self, ()> {
        match snapshot {
            PaneNodeSnapshot::Branch(branch) => {
                let panes = branch
                    .children
                    .iter()
                    .filter_map(|(_, snapshot)| snapshot.clone().try_into().ok())
                    .collect::<Vec<PaneTemplateType>>();
                match panes.len() {
                    0 => Err(()),
                    1 => Ok(panes
                        .into_iter()
                        .next()
                        .expect("Checked that panes has 1 element")),
                    _ => Ok(Self::PaneBranchTemplate {
                        split_direction: branch.direction.into(),
                        panes,
                    }),
                }
            }
            PaneNodeSnapshot::Leaf(leaf) => match leaf.contents {
                LeafContents::Terminal(terminal) => Ok(Self::PaneTemplate {
                    cwd: PathBuf::from(terminal.cwd.unwrap_or_default()),
                    commands: Vec::new(),
                    is_focused: Some(leaf.is_focused),
                    pane_mode: PaneMode::Terminal,
                    shell: None,
                }),
                // Currently, notebook panes cannot be saved in launch configurations.
                LeafContents::Notebook(_)
                | LeafContents::EnvVarCollection(_)
                | LeafContents::Code(_)
                | LeafContents::Workflow(_)
                | LeafContents::Settings(_)
                | LeafContents::AIFact(_)
                | LeafContents::CodeReview(_)
                | LeafContents::CustomRouterEditor
                | LeafContents::ExecutionProfileEditor
                | LeafContents::GetStarted
                | LeafContents::NetworkLog
                | LeafContents::AIDocument(_)
                | LeafContents::EnvironmentManagement(_)
                | LeafContents::AmbientAgent(_) => {
                    // TODO: Handle AIDocument in launch config
                    Err(())
                }
            },
        }
    }
}

/// Deserializes a string that semantically represents a path, expanding ~ as
/// needed.
fn deserialize_path<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
where
    D: Deserializer<'de>,
{
    let raw_path = String::deserialize(deserializer)?;
    Ok(PathBuf::from(shellexpand::tilde(&raw_path).into_owned()))
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct TabTemplate {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    pub layout: PaneTemplateType,
    #[serde(skip_serializing, default)]
    pub commands: Vec<CommandTemplate>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub color: Option<AnsiColorIdentifier>,
    /// Index into [`WindowTemplate::tab_groups`], when this tab is grouped.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub group: Option<usize>,
}

impl TabTemplate {
    pub fn layout_with_tab_commands(&self) -> PaneTemplateType {
        let mut layout = self.layout.clone();
        if !self.commands.is_empty() {
            layout.add_commands_to_startup_pane(self.commands.clone());
        }
        layout
    }
}

impl PaneTemplateType {
    fn add_commands_to_startup_pane(&mut self, tab_commands: Vec<CommandTemplate>) -> bool {
        match self {
            PaneTemplateType::PaneTemplate { commands, .. } => {
                commands.extend(tab_commands);
                true
            }
            PaneTemplateType::PaneBranchTemplate { panes, .. } => {
                if let Some(focused_pane) = panes.iter_mut().find(|pane| pane.is_focused_pane()) {
                    focused_pane.add_commands_to_startup_pane(tab_commands)
                } else if let Some(first_pane) = panes.first_mut() {
                    first_pane.add_commands_to_startup_pane(tab_commands)
                } else {
                    false
                }
            }
        }
    }

    fn is_focused_pane(&self) -> bool {
        match self {
            PaneTemplateType::PaneTemplate { is_focused, .. } => is_focused.unwrap_or_default(),
            PaneTemplateType::PaneBranchTemplate { panes, .. } => {
                panes.iter().any(Self::is_focused_pane)
            }
        }
    }
}

impl TryFrom<TabSnapshot> for TabTemplate {
    type Error = ();

    fn try_from(snapshot: TabSnapshot) -> Result<Self, ()> {
        let color = snapshot.color();
        Ok(Self {
            title: snapshot.custom_title,
            layout: snapshot.root.try_into()?,
            commands: Vec::new(),
            color,
            // Resolved by `From<WindowSnapshot>`, which is the only place
            // that knows the window's surviving group list.
            group: None,
        })
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirection {
    Vertical,
    Horizontal,
}

impl From<StateSplitDirection> for SplitDirection {
    fn from(snapshot: StateSplitDirection) -> Self {
        match snapshot {
            StateSplitDirection::Horizontal => Self::Horizontal,
            StateSplitDirection::Vertical => Self::Vertical,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct CommandTemplate {
    pub exec: String,
}

impl From<&str> for CommandTemplate {
    fn from(s: &str) -> CommandTemplate {
        CommandTemplate {
            exec: s.to_string(),
        }
    }
}

// TODO add extra elements to the mock (split panes, multiple tabs, multiple windows)
pub fn make_mock_single_window_launch_config() -> LaunchConfig {
    LaunchConfig {
        name: "Mocked Config".to_string(),
        active_window_index: Some(0),
        windows: vec![WindowTemplate {
            tab_groups: vec![],
            active_tab_index: Some(0),
            tabs: vec![
                TabTemplate {
                    group: None,
                    title: Some("First Tab".to_string()),
                    layout: PaneTemplateType::PaneTemplate {
                        is_focused: Some(true),
                        cwd: PathBuf::from("/some/path"),
                        commands: vec!["echo test_command".into()],
                        pane_mode: PaneMode::Terminal,
                        shell: None,
                    },
                    commands: Vec::new(),
                    color: None,
                },
                TabTemplate {
                    group: None,
                    title: Some("Second Tab".to_string()),
                    layout: PaneTemplateType::PaneTemplate {
                        is_focused: Some(true),
                        cwd: PathBuf::from("/some/path"),
                        commands: vec!["echo test_command_on_another_tab".into()],
                        pane_mode: PaneMode::Terminal,
                        shell: None,
                    },
                    commands: Vec::new(),
                    color: None,
                },
            ],
        }],
    }
}
