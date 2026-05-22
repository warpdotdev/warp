use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

use crate::app_state::{
    AppState, LeafContents, PaneNodeSnapshot, SplitDirection as StateSplitDirection, TabSnapshot,
    WindowSnapshot,
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
}

impl From<WindowSnapshot> for WindowTemplate {
    fn from(snapshot: WindowSnapshot) -> Self {
        let mut active_tab_index = None;
        let mut num_valid_tabs = 0;

        let tabs = snapshot
            .tabs
            .into_iter()
            .enumerate()
            .filter_map(|(i, tab)| {
                let tab = tab.try_into().ok()?;

                if i == snapshot.active_tab_index {
                    active_tab_index = Some(num_valid_tabs);
                }

                num_valid_tabs += 1;

                Some(tab)
            })
            .collect::<Vec<TabTemplate>>();

        Self {
            active_tab_index,
            tabs,
        }
    }
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
                | LeafContents::ExecutionProfileEditor
                | LeafContents::GetStarted
                | LeafContents::NetworkLog
                | LeafContents::Welcome { .. }
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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub color: Option<AnsiColorIdentifier>,
}

impl TryFrom<TabSnapshot> for TabTemplate {
    type Error = ();

    fn try_from(snapshot: TabSnapshot) -> Result<Self, ()> {
        let color = snapshot.color();
        Ok(Self {
            title: snapshot.custom_title,
            layout: snapshot.root.try_into()?,
            color,
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
            active_tab_index: Some(0),
            tabs: vec![
                TabTemplate {
                    title: Some("First Tab".to_string()),
                    layout: PaneTemplateType::PaneTemplate {
                        is_focused: Some(true),
                        cwd: PathBuf::from("/some/path"),
                        commands: vec!["echo test_command".into()],
                        pane_mode: PaneMode::Terminal,
                        shell: None,
                    },
                    color: None,
                },
                TabTemplate {
                    title: Some("Second Tab".to_string()),
                    layout: PaneTemplateType::PaneTemplate {
                        is_focused: Some(true),
                        cwd: PathBuf::from("/some/path"),
                        commands: vec!["echo test_command_on_another_tab".into()],
                        pane_mode: PaneMode::Terminal,
                        shell: None,
                    },
                    color: None,
                },
            ],
        }],
    }
}
