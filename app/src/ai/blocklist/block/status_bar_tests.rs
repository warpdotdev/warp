//! Unit tests for the router warping indicator resolver (APP-4978).
//!
//! These cover the pure resolver seam ([`super::resolve_router_warping`],
//! [`super::classify_router`], [`super::cloud_router_search_query`],
//! [`super::ModelInfoSnapshot`]) with synthetic model ids and output
//! metadata, avoiding a live server or `AppContext`. The live
//! `resolve_router_warping_for_exchange` wrapper is exercised end-to-end by
//! the repo's integration/visual checks; here we lock down the
//! classification, display-label, stale-data, link-target, cloud-query
//! derivation, and feature-gate behavior deterministically.

use std::collections::HashMap;
use std::path::PathBuf;

use super::{
    ModelInfoSnapshot, RouterConfigLink, RouterKind, RouterWarpingResolution, classify_router,
    cloud_router_search_query, resolve_router_warping,
};
use crate::ai::custom_model_routers::{CLOUD_CUSTOM_ROUTER_PREFIX, LOCAL_CUSTOM_ROUTER_PREFIX};
use crate::ai::llms::{LLMContextWindow, LLMInfo, LLMProvider, LLMUsageMetadata};

fn info(display_name: &str, model_id: &str) -> ModelInfoSnapshot {
    ModelInfoSnapshot {
        display_name: display_name.to_string(),
        model_id: model_id.to_string(),
    }
}

/// Minimal server-style [`LLMInfo`] with the given display name, mirroring the
/// fixtures in `llms_tests.rs`. Used to exercise [`cloud_router_search_query`]
/// — the helper the live wrapper calls with `LLMPreferences::get_llm_info`'s
/// result, so the test type-signature enforces that the query is derived from
/// an `LLMInfo` (what `get_llm_info` returns), not from a local registry entry.
fn llm_info(display_name: &str) -> LLMInfo {
    LLMInfo {
        display_name: display_name.to_string(),
        base_model_name: display_name.to_string(),
        id: format!("{CLOUD_CUSTOM_ROUTER_PREFIX}team-router").into(),
        reasoning_level: None,
        usage_metadata: LLMUsageMetadata {
            request_multiplier: 1,
            credit_multiplier: None,
        },
        description: None,
        disable_reason: None,
        vision_supported: false,
        spec: None,
        provider: LLMProvider::Unknown,
        host_configs: HashMap::new(),
        discount_percentage: None,
        context_window: LLMContextWindow::default(),
    }
}

#[test]
fn classify_router_distinguishes_local_cloud_builtin_and_direct() {
    let local = format!("{LOCAL_CUSTOM_ROUTER_PREFIX}my-router");
    let cloud = format!("{CLOUD_CUSTOM_ROUTER_PREFIX}team-router");
    assert_eq!(classify_router(Some(&local)), Some(RouterKind::CustomLocal));
    assert_eq!(classify_router(Some(&cloud)), Some(RouterKind::CustomCloud));
    // Built-in auto routers.
    assert_eq!(classify_router(Some("auto")), Some(RouterKind::BuiltInAuto));
    assert_eq!(
        classify_router(Some("auto-fast")),
        Some(RouterKind::BuiltInAuto)
    );
    assert_eq!(
        classify_router(Some("cli-agent-auto")),
        Some(RouterKind::BuiltInAuto)
    );
    assert_eq!(
        classify_router(Some("computer-use-agent-auto")),
        Some(RouterKind::BuiltInAuto)
    );
    // Direct (non-router) model ids are ineligible.
    assert_eq!(classify_router(Some("claude-sonnet-4-5")), None);
    assert_eq!(classify_router(None), None);
    // Whitespace is tolerated.
    assert_eq!(
        classify_router(Some("  auto  ")),
        Some(RouterKind::BuiltInAuto)
    );
}

#[test]
fn model_info_snapshot_display_label_prefers_display_name_then_model_id() {
    assert_eq!(
        info("Claude Sonnet", "claude-sonnet-4-5").display_label(),
        Some("Claude Sonnet")
    );
    // Empty display name falls back to the model id.
    assert_eq!(
        info("", "claude-sonnet-4-5").display_label(),
        Some("claude-sonnet-4-5")
    );
    // Both empty -> no label (caller keeps `Warping...`).
    assert_eq!(info("", "").display_label(), None);
}

#[test]
fn resolve_router_warping_flag_disabled_returns_none_even_for_routers() {
    // The new flag off => no router display, regardless of inputs. This is the
    // feature-gate regression: with the flag disabled the existing fallback
    // messaging (governed independently by FallbackModelLoadOutputMessaging)
    // is the only source of warping text.
    let local = format!("{LOCAL_CUSTOM_ROUTER_PREFIX}my-router");
    assert!(
        resolve_router_warping(
            false,
            Some(&local),
            Some(info("Claude Sonnet", "claude-sonnet-4-5")),
            None,
            false,
            None,
            None,
        )
        .is_none()
    );
}

#[test]
fn resolve_router_warping_direct_model_returns_none() {
    // A direct (non-router) selected model never produces router display, even
    // with the flag on and a resolved model name available. Non-router turns
    // keep the existing implicit `Warping...` text.
    assert!(
        resolve_router_warping(
            true,
            Some("claude-sonnet-4-5"),
            Some(info("Claude Sonnet", "claude-sonnet-4-5")),
            None,
            false,
            None,
            None,
        )
        .is_none()
    );
    assert!(
        resolve_router_warping(true, None, Some(info("X", "x")), None, false, None, None).is_none()
    );
}

#[test]
fn resolve_router_warping_builtin_auto_shows_model_without_link() {
    let res = resolve_router_warping(
        true,
        Some("auto"),
        Some(info("Claude Sonnet", "claude-sonnet-4-5")),
        None,
        false,
        None,
        None,
    )
    .expect("builtin auto with a resolved model should resolve");
    assert_eq!(res.label, "Warping with Claude Sonnet.");
    assert_eq!(res.link, RouterConfigLink::None);
}

#[test]
fn resolve_router_warping_builtin_auto_empty_display_falls_back_to_model_id() {
    let res = resolve_router_warping(
        true,
        Some("auto"),
        Some(info("", "claude-haiku")),
        None,
        false,
        None,
        None,
    )
    .expect("builtin auto with a model id fallback should resolve");
    assert_eq!(res.label, "Warping with claude-haiku.");
    assert_eq!(res.link, RouterConfigLink::None);
}

#[test]
fn resolve_router_warping_missing_model_info_returns_none() {
    // Before ModelUsed arrives, there's nothing to label; the indicator stays
    // on the safe default `Warping...` text.
    assert!(resolve_router_warping(true, Some("auto"), None, None, false, None, None).is_none());
}

#[test]
fn resolve_router_warping_local_custom_with_source_path_links_to_file() {
    let local = format!("{LOCAL_CUSTOM_ROUTER_PREFIX}my-router");
    let path = PathBuf::from("/home/user/.warp/custom_model_routers/my-router.yaml");
    let res = resolve_router_warping(
        true,
        Some(&local),
        Some(info("Claude Sonnet", "claude-sonnet-4-5")),
        None,
        false,
        Some(&path),
        None,
    )
    .expect("local custom router with a source path should resolve");
    assert_eq!(res.label, "Warping with Claude Sonnet.");
    assert_eq!(res.link, RouterConfigLink::OpenLocalFile(path));
}

#[test]
fn resolve_router_warping_local_custom_without_source_path_has_no_link() {
    let local = format!("{LOCAL_CUSTOM_ROUTER_PREFIX}my-router");
    // A missing source_path produces no link, but the resolved model still
    // shows (criterion: a pathless local router renders no broken link).
    let res = resolve_router_warping(
        true,
        Some(&local),
        Some(info("Claude Sonnet", "claude-sonnet-4-5")),
        None,
        false,
        None,
        None,
    )
    .expect("local custom router should still resolve without a source path");
    assert_eq!(res.label, "Warping with Claude Sonnet.");
    assert_eq!(res.link, RouterConfigLink::None);
}

#[test]
fn resolve_router_warping_cloud_custom_links_to_settings_with_router_name() {
    let cloud = format!("{CLOUD_CUSTOM_ROUTER_PREFIX}team-router");
    let res = resolve_router_warping(
        true,
        Some(&cloud),
        Some(info("Claude Sonnet", "claude-sonnet-4-5")),
        None,
        false,
        None,
        Some("Team Router"),
    )
    .expect("cloud custom router should resolve");
    assert_eq!(res.label, "Warping with Claude Sonnet.");
    assert_eq!(
        res.link,
        RouterConfigLink::OpenCloudSettings {
            search_query: "Team Router".to_string(),
        }
    );
}

#[test]
fn resolve_router_warping_cloud_custom_without_query_falls_back_to_id() {
    let cloud = format!("{CLOUD_CUSTOM_ROUTER_PREFIX}team-router");
    // No display-name query supplied -> fall back to the config_key id so the
    // settings search is still deterministic.
    let res = resolve_router_warping(
        true,
        Some(&cloud),
        Some(info("Claude Sonnet", "claude-sonnet-4-5")),
        None,
        false,
        None,
        None,
    )
    .expect("cloud custom router should resolve even without a query");
    assert_eq!(
        res.link,
        RouterConfigLink::OpenCloudSettings {
            search_query: cloud.clone(),
        }
    );
}

#[test]
fn resolve_router_warping_follow_up_may_use_previous_exchange() {
    // An agent-initiated follow-up (not a new user query) may reuse the
    // immediately previous exchange's model info before ModelUsed arrives,
    // mirroring the existing fallback anti-flicker lookback.
    let local = format!("{LOCAL_CUSTOM_ROUTER_PREFIX}my-router");
    let path = PathBuf::from("/r.yaml");
    let res = resolve_router_warping(
        true,
        Some(&local),
        None, // current exchange has no model info yet
        Some(info("Claude Haiku", "claude-haiku")),
        false, // not a new user query
        Some(&path),
        None,
    )
    .expect("follow-up should fall back to the previous exchange");
    assert_eq!(res.label, "Warping with Claude Haiku.");
    assert_eq!(res.link, RouterConfigLink::OpenLocalFile(path));
}

#[test]
fn resolve_router_warping_new_user_query_never_uses_previous_exchange() {
    // A fresh user query must never display stale model info from an earlier
    // exchange, even if the previous exchange carried a resolved model.
    let local = format!("{LOCAL_CUSTOM_ROUTER_PREFIX}my-router");
    assert!(
        resolve_router_warping(
            true,
            Some(&local),
            None,
            Some(info("Claude Haiku", "claude-haiku")),
            true, // new user query
            None,
            None,
        )
        .is_none()
    );
}

#[test]
fn router_warping_resolution_link_is_not_displayed_for_builtin_auto() {
    // Sanity: built-in auto never carries a config link, so the footer never
    // renders a "Configure router" affordance for it (criterion 4).
    let res = resolve_router_warping(
        true,
        Some("auto"),
        Some(info("Claude Sonnet", "claude-sonnet-4-5")),
        None,
        false,
        // Even if a stray local path were supplied, built-in auto ignores it.
        Some(&PathBuf::from("/x.yaml")),
        None,
    )
    .expect("builtin auto should resolve");
    assert_eq!(res.link, RouterConfigLink::None);
    // Confirm the resolution type carries the expected shape for renderers.
    let RouterWarpingResolution { label, link } = res;
    assert!(label.starts_with("Warping with "));
    assert_eq!(link, RouterConfigLink::None);
}

// ── Cloud-query derivation (live wrapper seam) ───────────────────────────────

#[test]
fn cloud_router_search_query_uses_llm_info_display_name() {
    // The live wrapper `resolve_router_warping_for_exchange` derives the cloud
    // settings search query from `LLMPreferences::get_llm_info(base_id)`'s
    // display name (cloud/team routers are server-synced `LLMInfo` entries, not
    // local custom-router registry entries). This is the case the previous
    // `custom_model_router_for_id` lookup missed: it returned `None` for every
    // cloud router, so the `Configure router` link searched for the raw
    // `custom-router:cloud:<id>` key instead of the visible router name.
    assert_eq!(
        cloud_router_search_query(Some(&llm_info("Team Router"))),
        Some("Team Router".to_string())
    );
}

#[test]
fn cloud_router_search_query_none_when_llm_info_missing() {
    // `get_llm_info` returns `None` when the router isn't a known model -> the
    // query is `None` and the pure resolver falls back to the config-key id
    // (spec invariant 3: router name/ID supplied as the search query).
    assert_eq!(cloud_router_search_query(None), None);
}

#[test]
fn cloud_router_search_query_none_when_display_name_empty_or_whitespace() {
    // An empty or whitespace-only display name would search for nothing useful,
    // so the helper returns `None` and the pure resolver falls back to the id.
    assert_eq!(cloud_router_search_query(Some(&llm_info(""))), None);
    assert_eq!(cloud_router_search_query(Some(&llm_info("   "))), None);
    assert_eq!(cloud_router_search_query(Some(&llm_info("\t\n"))), None);
}

#[test]
fn cloud_router_search_query_trims_only_for_the_emptiness_check() {
    // A display name with surrounding whitespace is still used as-is (only
    // all-whitespace names are rejected); the resolver does not strip visible
    // padding from a legitimate name.
    assert_eq!(
        cloud_router_search_query(Some(&llm_info("  Team Router  "))),
        Some("  Team Router  ".to_string())
    );
}

// ── Feature-gate regression (spec criterion 5) ──────────────────────────────

#[test]
fn resolve_router_warping_flag_disabled_returns_none_for_every_router_kind() {
    // Spec criterion 5: with `RouterWarpingIndicator` disabled, no router
    // display/link is returned for any router kind (local, cloud, or built-in
    // auto). The router resolver is gated solely by `RouterWarpingIndicator`
    // and never consults `FallbackModelLoadOutputMessaging`, so the existing
    // fallback messaging (`resolve_fallback_warping_message`) remains the only
    // source of warping text when this flag is off — unchanged across all four
    // combinations of the two flags. The wrapper
    // `resolve_router_warping_for_exchange` mirrors this with an early `None`
    // return when the flag is off.
    let local = format!("{LOCAL_CUSTOM_ROUTER_PREFIX}my-router");
    let cloud = format!("{CLOUD_CUSTOM_ROUTER_PREFIX}team-router");
    let path = PathBuf::from("/r.yaml");
    for id in [local.as_str(), cloud.as_str(), "auto"] {
        assert!(
            resolve_router_warping(
                false, // router flag off
                Some(id),
                Some(info("Claude Sonnet", "claude-sonnet-4-5")),
                None,
                false,
                Some(&path),
                Some("Team Router"),
            )
            .is_none(),
            "router flag off => no router display for {id}"
        );
    }
}
