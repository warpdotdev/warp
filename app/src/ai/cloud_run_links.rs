//! Resolves the web destination for a cloud-run link, routing through Platform for viewers
//! with Factory access and falling back to Oz otherwise. See `specs/APP-5583/TECH.md`.

use warp_core::channel::ChannelState;
use warpui::{AppContext, SingletonEntity};

use crate::ai::factory_access::{FactoryAccess, FactoryAccessModel};

/// A cloud-run web destination the desktop client can open.
pub enum CloudRunLink<'a> {
    /// A specific run's detail page.
    Run { run_id: &'a str },
    /// The run index ("View all cloud runs").
    RunIndex,
    /// A run's recording artifact; preserves the artifact query on both origins.
    Artifact {
        run_id: &'a str,
        artifact_uid: &'a str,
    },
}

impl CloudRunLink<'_> {
    fn path(&self) -> String {
        match self {
            CloudRunLink::Run { run_id } => format!("/runs/{run_id}"),
            CloudRunLink::RunIndex => "/runs".to_string(),
            CloudRunLink::Artifact {
                run_id,
                artifact_uid,
            } => format!(
                "/runs/{run_id}?artifact={}",
                urlencoding::encode(artifact_uid)
            ),
        }
    }
}

/// Resolves the web URL for `link` given the viewer's current Factory `access` and this
/// channel's configured origins. Platform only for [`FactoryAccess::Allowed`] with a
/// configured `platform_root_url`; Oz for `Denied`, `Unknown`, or a channel with no Platform
/// origin yet.
pub fn cloud_run_web_url(
    access: FactoryAccess,
    link: &CloudRunLink,
    oz_root_url: &str,
    platform_root_url: Option<&str>,
) -> String {
    match (access, platform_root_url) {
        (FactoryAccess::Allowed, Some(platform_root_url)) => {
            format!("{platform_root_url}{}", link.path())
        }
        _ => format!("{oz_root_url}{}", link.path()),
    }
}

/// Resolves `link` against the current channel's origins and the viewer's current Factory
/// access, read fresh from [`FactoryAccessModel`] so a startup result that lands while a menu
/// or panel is already open still affects the next call. Falls back to
/// [`FactoryAccess::Unknown`] when the model isn't registered (e.g. a test harness that
/// doesn't need it), which also routes to Oz.
pub fn cloud_run_web_url_now(link: &CloudRunLink, app: &AppContext) -> String {
    let access = if app.has_singleton_model::<FactoryAccessModel>() {
        FactoryAccessModel::as_ref(app).access()
    } else {
        FactoryAccess::Unknown
    };
    cloud_run_web_url(
        access,
        link,
        &ChannelState::oz_root_url(),
        ChannelState::platform_root_url().as_deref(),
    )
}

#[cfg(test)]
#[path = "cloud_run_links_tests.rs"]
mod tests;
