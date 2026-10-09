//! Driving third-party harnesses over the Agent Client Protocol.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the ACP harness runner that drives the bridge lands in a follow-up"
    )
)]
mod bridge;
mod launch;

pub(crate) use bridge::run_bridge;
pub(crate) use launch::AcpLaunchSpec;
