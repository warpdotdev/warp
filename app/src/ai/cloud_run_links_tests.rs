use super::*;

const STABLE_OZ: &str = "https://oz.warp.dev";
const STABLE_PLATFORM: &str = "https://platform.warp.dev";
const DEV_OZ: &str = "https://oz.staging.warp.dev";
const DEV_PLATFORM: &str = "https://platform.staging.warp.dev";

#[test]
fn allowed_with_platform_configured_routes_to_platform() {
    let url = cloud_run_web_url(
        FactoryAccess::Allowed,
        &CloudRunLink::Run { run_id: "run-1" },
        STABLE_OZ,
        Some(STABLE_PLATFORM),
    );
    assert_eq!(url, format!("{STABLE_PLATFORM}/runs/run-1"));
}

#[test]
fn allowed_on_dev_channel_routes_to_staging_platform() {
    let url = cloud_run_web_url(
        FactoryAccess::Allowed,
        &CloudRunLink::Run { run_id: "run-1" },
        DEV_OZ,
        Some(DEV_PLATFORM),
    );
    assert_eq!(url, format!("{DEV_PLATFORM}/runs/run-1"));
}

#[test]
fn denied_routes_to_oz() {
    let url = cloud_run_web_url(
        FactoryAccess::Denied,
        &CloudRunLink::Run { run_id: "run-1" },
        STABLE_OZ,
        Some(STABLE_PLATFORM),
    );
    assert_eq!(url, format!("{STABLE_OZ}/runs/run-1"));
}

#[test]
fn unknown_routes_to_oz() {
    let url = cloud_run_web_url(
        FactoryAccess::Unknown,
        &CloudRunLink::Run { run_id: "run-1" },
        STABLE_OZ,
        Some(STABLE_PLATFORM),
    );
    assert_eq!(url, format!("{STABLE_OZ}/runs/run-1"));
}

#[test]
fn allowed_without_configured_platform_origin_falls_back_to_oz() {
    // Mirrors a channel-config generator that predates the Platform origin field.
    let url = cloud_run_web_url(
        FactoryAccess::Allowed,
        &CloudRunLink::Run { run_id: "run-1" },
        STABLE_OZ,
        None,
    );
    assert_eq!(url, format!("{STABLE_OZ}/runs/run-1"));
}

#[test]
fn run_index_selects_runs_path_in_both_access_states() {
    assert_eq!(
        cloud_run_web_url(
            FactoryAccess::Allowed,
            &CloudRunLink::RunIndex,
            STABLE_OZ,
            Some(STABLE_PLATFORM),
        ),
        format!("{STABLE_PLATFORM}/runs")
    );
    assert_eq!(
        cloud_run_web_url(
            FactoryAccess::Denied,
            &CloudRunLink::RunIndex,
            STABLE_OZ,
            Some(STABLE_PLATFORM),
        ),
        format!("{STABLE_OZ}/runs")
    );
}

#[test]
fn artifact_link_preserves_encoded_artifact_query_in_both_access_states() {
    let link = CloudRunLink::Artifact {
        run_id: "run-1",
        artifact_uid: "recording with spaces",
    };
    assert_eq!(
        cloud_run_web_url(
            FactoryAccess::Allowed,
            &link,
            STABLE_OZ,
            Some(STABLE_PLATFORM)
        ),
        format!("{STABLE_PLATFORM}/runs/run-1?artifact=recording%20with%20spaces")
    );
    assert_eq!(
        cloud_run_web_url(
            FactoryAccess::Denied,
            &link,
            STABLE_OZ,
            Some(STABLE_PLATFORM)
        ),
        format!("{STABLE_OZ}/runs/run-1?artifact=recording%20with%20spaces")
    );
}

#[test]
fn cloud_run_web_url_now_falls_back_to_oz_when_factory_access_model_is_unregistered() {
    // Exercises the app-context wrapper's `has_singleton_model` guard: a test harness that
    // never registers `FactoryAccessModel` (like most existing view/panel tests) must still
    // resolve links, defaulting to `Unknown` access rather than panicking.
    warpui::App::test((), |mut app| async move {
        app.update(|ctx| {
            let url = cloud_run_web_url_now(&CloudRunLink::RunIndex, ctx);
            assert!(url.ends_with("/runs"));
            assert!(!url.contains("platform"));
        });
    });
}
