use std::collections::HashSet;

use lazy_static::lazy_static;

lazy_static! {
    /// The set of command corrections that are NOT preferred over Next Command, in the case that both
    /// features are enabled. Based on acceptance rates and rules themselves.
    pub static ref COMMAND_CORRECTIONS_PREFERRED_DENYLIST: HashSet<&'static str> = HashSet::from([
        "NoCommand",
        "CdMkdir",
    ]);
}
