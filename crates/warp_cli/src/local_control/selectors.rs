//! CLI argument conversion into shared local-control selectors.
use local_control::protocol::{
    PaneSelector, PaneTarget, TabSelector, TabTarget, TargetSelector, WindowSelector, WindowTarget,
};
use local_control::selection::InstanceSelector;

use crate::local_control::TargetArgs;

pub(super) fn instance_selector(args: &TargetArgs) -> InstanceSelector {
    if let Some(instance_id) = args.instance.clone() {
        return InstanceSelector::Id(local_control::discovery::InstanceId(instance_id));
    }
    if let Some(pid) = args.pid {
        return InstanceSelector::Pid(pid);
    }
    InstanceSelector::Active
}

pub(super) fn target_selector(args: &TargetArgs) -> TargetSelector {
    TargetSelector {
        window: args.window.as_ref().map(|window| {
            if window == "active" {
                WindowTarget::Active
            } else {
                WindowTarget::Id {
                    id: WindowSelector(window.clone()),
                }
            }
        }),
        tab: args.tab_index.map(|index| TabTarget::Index { index }).or_else(|| {
            args.tab.as_ref().map(|tab| {
                if tab == "active" {
                    TabTarget::Active
                } else {
                    TabTarget::Id {
                        id: TabSelector(tab.clone()),
                    }
                }
            })
        }),
        pane: args
            .pane_index
            .map(|index| PaneTarget::Index { index })
            .or_else(|| {
                args.pane.as_ref().map(|pane| {
                    if pane == "active" {
                        PaneTarget::Active
                    } else {
                        PaneTarget::Id {
                            id: PaneSelector(pane.clone()),
                        }
                    }
                })
            }),
    }
}
