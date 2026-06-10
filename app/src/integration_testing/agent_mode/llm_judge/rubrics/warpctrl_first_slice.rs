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
            description: "`warpctrl tab create` end-to-end mutates the running app and returns success envelope with `instance_id` and `tab_id`.",
            spec_section: "PRODUCT.md §27, TECH.md §1 response shape",
        },
        RubricSpecItem {
            id: "B-allowlist",
            description: "Unknown actions return `unsupported_action` or `not_allowlisted`; not silently forwarded to arbitrary internal dispatch.",
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
            description: "`InvocationContext::InsideWarp` and `ExecutionContextProof::VerifiedWarpTerminal` are rejected by the credential broker with a structured error (proof broker is reserved).",
            spec_section: "TECH.md §0, §4",
        },
        RubricSpecItem {
            id: "I-protocol-crate",
            description: "A shared protocol module exists (e.g. `crates/local_control`) that exports the selector types, the allowlisted `ControlAction` variants, and the stable error code enum.",
            spec_section: "TECH.md §1",
        },
    ],
};
