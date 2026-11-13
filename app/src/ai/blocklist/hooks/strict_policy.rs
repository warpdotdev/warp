use ai::hooks::DispatchPolicy;

/// The "strict hook policy" for a run. When on, every evaluation failure of a hook that runs
/// before an operation (a timeout, a crash, malformed or empty output, an unsupported control
/// output) denies the operation, and schema-accepted `ask` values the vendor would ignore are
/// honored. It is a Warp policy for security deployments, not vendor parity.
///
/// There is no setting for it yet. It is a run configuration value that the run's launcher
/// sets on the hook runtime; until a launcher does, it is off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StrictHookPolicy {
    pub enabled: bool,
}

impl StrictHookPolicy {
    pub fn dispatch_policy(self) -> DispatchPolicy {
        DispatchPolicy {
            strict: self.enabled,
        }
    }
}
