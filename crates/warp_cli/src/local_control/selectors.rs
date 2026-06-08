//! CLI argument conversion into shared local-control selectors.
use local_control::selection::InstanceSelector;

use crate::local_control::TargetArgs;

pub(super) fn instance_selector(args: TargetArgs) -> InstanceSelector {
    if let Some(instance_id) = args.instance {
        return InstanceSelector::Id(local_control::discovery::InstanceId(instance_id));
    }
    if let Some(pid) = args.pid {
        return InstanceSelector::Pid(pid);
    }
    InstanceSelector::Active
}
