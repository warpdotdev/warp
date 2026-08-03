//! CLI argument conversion into shared local-control selectors.
use local_control::protocol::{
    ControlError, ErrorCode, PaneSelector, PaneTarget, SessionSelector, SessionTarget, TabSelector,
    TabTarget, TargetSelector, WindowSelector, WindowTarget,
};
use local_control::selection::InstanceSelector;

use crate::local_control::TargetArgs;

pub(super) fn instance_selector(args: &TargetArgs) -> InstanceSelector {
    if let Some(instance_id) = &args.instance {
        return InstanceSelector::Id(local_control::discovery::InstanceId(instance_id.clone()));
    }
    if let Some(pid) = args.pid {
        return InstanceSelector::Pid(pid);
    }
    InstanceSelector::Active
}

pub(super) fn target_selector(args: &TargetArgs) -> Result<TargetSelector, ControlError> {
    Ok(TargetSelector {
        window: window_target(args)?,
        tab: tab_target(args)?,
        pane: pane_target(args)?,
        session: session_target(args)?,
    })
}

fn window_target(args: &TargetArgs) -> Result<Option<WindowTarget>, ControlError> {
    if let Some(window) = &args.window {
        if window == "active" {
            return Ok(Some(WindowTarget::Active));
        }
        return Ok(Some(WindowTarget::Id {
            id: WindowSelector(window.clone()),
        }));
    }
    if let Some(index) = args.window_index {
        return Ok(Some(WindowTarget::Index { index }));
    }
    if let Some(title) = &args.window_title {
        return Ok(Some(WindowTarget::Title {
            title: title.clone(),
        }));
    }
    Ok(None)
}

fn tab_target(args: &TargetArgs) -> Result<Option<TabTarget>, ControlError> {
    if let Some(tab) = &args.tab {
        if tab == "active" {
            return Ok(Some(TabTarget::Active));
        }
        return Ok(Some(TabTarget::Id {
            id: TabSelector(tab.clone()),
        }));
    }
    if let Some(index) = args.tab_index {
        return Ok(Some(TabTarget::Index { index }));
    }
    if let Some(title) = &args.tab_title {
        return Ok(Some(TabTarget::Title {
            title: title.clone(),
        }));
    }
    Ok(None)
}

fn pane_target(args: &TargetArgs) -> Result<Option<PaneTarget>, ControlError> {
    if let Some(pane) = &args.pane {
        if pane == "active" {
            return Ok(Some(PaneTarget::Active));
        }
        return Ok(Some(PaneTarget::Id {
            id: PaneSelector(pane.clone()),
        }));
    }
    if let Some(index) = args.pane_index {
        return Ok(Some(PaneTarget::Index { index }));
    }
    Ok(None)
}

fn session_target(args: &TargetArgs) -> Result<Option<SessionTarget>, ControlError> {
    if let Some(session) = &args.session {
        if session == "active" {
            return Ok(Some(SessionTarget::Active));
        }
        if session.is_empty() {
            return Err(ControlError::new(
                ErrorCode::InvalidSelector,
                "session selector cannot be empty",
            ));
        }
        return Ok(Some(SessionTarget::Id {
            id: SessionSelector(session.clone()),
        }));
    }
    Ok(None)
}
