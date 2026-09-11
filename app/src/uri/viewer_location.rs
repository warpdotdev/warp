#[cfg(any(target_family = "wasm", test))]
use std::collections::HashSet;
#[cfg(any(target_family = "wasm", test))]
use std::future::Future;

#[cfg(any(target_family = "wasm", test))]
use anyhow::{Result, anyhow};
#[cfg(any(target_family = "wasm", test))]
use url::{Url, form_urlencoded};

#[cfg(any(target_family = "wasm", test))]
use crate::ai::ambient_agents::{
    AmbientAgentLiveSessionState, AmbientAgentTask, AmbientAgentTaskId,
};

#[cfg(any(target_family = "wasm", test))]
pub(crate) const MAX_PARENT_EDGES: usize = 64;

#[cfg(any(target_family = "wasm", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChildAnchor {
    Root,
    Selected(AmbientAgentTaskId),
    Invalid,
}

#[cfg(any(target_family = "wasm", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HydratedAnchorAction {
    None,
    Wait,
    Select(AmbientAgentTaskId),
    Clear,
}

#[cfg(any(target_family = "wasm", test))]
pub(crate) fn hydrated_anchor_action(
    anchor: ChildAnchor,
    seeded_child_ids: &HashSet<AmbientAgentTaskId>,
    registered_child_ids: &HashSet<AmbientAgentTaskId>,
) -> HydratedAnchorAction {
    match anchor {
        ChildAnchor::Root => HydratedAnchorAction::None,
        ChildAnchor::Invalid => HydratedAnchorAction::Clear,
        ChildAnchor::Selected(task_id) if !seeded_child_ids.contains(&task_id) => {
            HydratedAnchorAction::Clear
        }
        ChildAnchor::Selected(task_id) if !registered_child_ids.contains(&task_id) => {
            HydratedAnchorAction::Wait
        }
        ChildAnchor::Selected(task_id) => HydratedAnchorAction::Select(task_id),
    }
}

#[cfg(any(target_family = "wasm", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ViewerLocation {
    pub root_url: Url,
    pub standalone: bool,
    pub child_anchor: ChildAnchor,
}

#[cfg(any(target_family = "wasm", test))]
impl ViewerLocation {
    pub(crate) fn parse(url: &Url) -> Option<Self> {
        if !matches!(
            url.path_segments()?.next(),
            Some("conversation" | "session")
        ) {
            return None;
        }

        let standalone = url
            .query_pairs()
            .any(|(key, value)| key == "view" && value == "standalone");
        let child_anchor = parse_child_anchor(url.fragment());
        let mut root_url = url.clone();
        root_url.set_fragment(None);
        Some(Self {
            root_url,
            standalone,
            child_anchor,
        })
    }

    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
    pub(crate) fn with_child(&self, child_run_id: AmbientAgentTaskId) -> Url {
        let mut url = self.root_url.clone();
        url.set_fragment(Some(&format!("child={child_run_id}")));
        url
    }
}

#[cfg(any(target_family = "wasm", test))]
fn parse_child_anchor(fragment: Option<&str>) -> ChildAnchor {
    let Some(fragment) = fragment else {
        return ChildAnchor::Root;
    };
    let child_values = form_urlencoded::parse(fragment.as_bytes())
        .filter_map(|(key, value)| (key == "child").then_some(value))
        .collect::<Vec<_>>();
    if child_values.is_empty() {
        return ChildAnchor::Root;
    }
    if child_values.len() != 1 || child_values[0].is_empty() {
        return ChildAnchor::Invalid;
    }
    child_values[0]
        .parse()
        .map(ChildAnchor::Selected)
        .unwrap_or(ChildAnchor::Invalid)
}

#[cfg(any(target_family = "wasm", test))]
pub(crate) struct RootResolution {
    pub entry_task_id: AmbientAgentTaskId,
    pub root_task: AmbientAgentTask,
}

#[cfg(any(target_family = "wasm", test))]
pub(crate) async fn resolve_root_task<F, Fut>(
    entry_task_id: AmbientAgentTaskId,
    mut fetch: F,
) -> Result<Option<RootResolution>>
where
    F: FnMut(AmbientAgentTaskId) -> Fut,
    Fut: Future<Output = Result<AmbientAgentTask>>,
{
    let mut visited = HashSet::from([entry_task_id]);
    let mut current_task_id = entry_task_id;
    let mut parent_edges = 0;

    loop {
        let task = fetch(current_task_id).await?;
        if task.task_id != current_task_id {
            return Err(anyhow!("run response did not match requested run id"));
        }
        let Some(parent_run_id) = task.parent_run_id.as_deref() else {
            return Ok(
                (current_task_id != entry_task_id).then_some(RootResolution {
                    entry_task_id,
                    root_task: task,
                }),
            );
        };
        if parent_edges == MAX_PARENT_EDGES {
            return Err(anyhow!(
                "run ancestry exceeds {MAX_PARENT_EDGES} parent edges"
            ));
        }
        let parent_task_id = parent_run_id
            .parse::<AmbientAgentTaskId>()
            .map_err(|_| anyhow!("invalid parent run id"))?;
        if !visited.insert(parent_task_id) {
            return Err(anyhow!("cycle in run ancestry"));
        }
        current_task_id = parent_task_id;
        parent_edges += 1;
    }
}

#[cfg(any(target_family = "wasm", test))]
pub(crate) fn canonical_root_url(current_url: &Url, resolution: &RootResolution) -> Option<Url> {
    let mut url = current_url.clone();
    url.set_query(None);
    url.set_fragment(None);

    match resolution.root_task.active_live_session_state() {
        AmbientAgentLiveSessionState::Attachable { session_id } => {
            url.set_path(&format!("/session/{session_id}"));
        }
        AmbientAgentLiveSessionState::Inactive
        | AmbientAgentLiveSessionState::ActiveUnattachable => {
            let conversation_id = resolution.root_task.conversation_id()?;
            url.set_path(&format!("/conversation/{conversation_id}"));
        }
    }
    url.set_fragment(Some(&format!("child={}", resolution.entry_task_id)));
    Some(url)
}

#[cfg(test)]
#[path = "viewer_location_tests.rs"]
mod tests;
