//! Rubric for the first vertical slice of the warpctrl spec
//! (warp-control-cli-specs). Item IDs are stable: they are recorded as
//! `rubric.{id}` runtime tags and aggregated across runs, so renaming one
//! breaks longitudinal comparisons.

use super::agent_judge::{RubricSpec, RubricSpecItem};

pub static WARPCTRL_FIRST_SLICE: RubricSpec = RubricSpec {
    name: "warpctrl_first_slice",
    items: &[
        RubricSpecItem {
            id: "B-discovery",
            description: "`warpctrl instance list` enumerates running compatible instances and prints opaque IDs.",
            spec_section: "PRODUCT.md §6",
        },
        RubricSpecItem {
            id: "B-no-instance",
            description: "Missing instance returns structured non-zero exit with `no_instance` error code.",
            spec_section: "PRODUCT.md §1, TECH.md §1 error codes",
        },
        RubricSpecItem {
            id: "B-ambiguity",
            description: "Multiple compatible instances without selector returns `ambiguous_instance`.",
            spec_section: "PRODUCT.md §6",
        },
        RubricSpecItem {
            id: "B-tab-create",
            description: "`warpctrl tab create` end-to-end mutates the running app and returns a success envelope with instance identity and created-tab metadata sufficient to identify the visible result (for example `tab.count` and `tab.active_index`). A stable `tab_id` is acceptable but not required.",
            spec_section: "PRODUCT.md §27, TECH.md §1 response shape",
        },
        RubricSpecItem {
            id: "B-allowlist",
            description: "Unknown or unimplemented actions are not executed or forwarded to arbitrary internal dispatch, and direct protocol requests receive a structured control error envelope with `unsupported_action`, `not_allowlisted`, or an equivalent stable action-unsupported code rather than a generic HTTP deserialization error.",
            spec_section: "PRODUCT.md §2",
        },
        RubricSpecItem {
            id: "S-private-settings",
            description: "New Scripting settings are marked `private: true` AND `SyncToCloud::Never`, and do not appear in `settings.toml` or the generated settings schema.",
            spec_section: "TECH.md §0",
        },
        RubricSpecItem {
            id: "S-loopback",
            description: "Local-control listener binds loopback only (`127.0.0.1` or `::1` both acceptable); endpoints do not set permissive CORS headers.",
            spec_section: "TECH.md §2 binding, §3 CORS posture, SECURITY.md loopback gate",
        },
        RubricSpecItem {
            id: "S-credentials",
            description: "Per-instance bearer credential is not stored in plaintext in the discovery record; credentials are unique per instance, not shared across processes.",
            spec_section: "TECH.md §3, README.md security model",
        },
        RubricSpecItem {
            id: "S-inside-warp-reject",
            description: "The credential broker rejects both `InvocationContext::InsideWarp` (with any proof) and `InvocationContext::OutsideWarp` paired with `ExecutionContextProof::VerifiedWarpTerminal`, returning a structured error because the proof broker is reserved.",
            spec_section: "TECH.md §0, §4",
        },
        RubricSpecItem {
            id: "I-protocol-crate",
            description: "A shared protocol module exists (e.g. `crates/local_control`) that exports the selector types, the allowlisted `ControlAction` variants, and the stable error code enum.",
            spec_section: "TECH.md §1",
        },
    ],
};
