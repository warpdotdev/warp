use std::collections::HashMap;
use std::time::Duration;

use warp::integration_testing::orchestration_navigation::child_pill_after_reopening_closed_parent_tab;

use crate::Builder;

pub fn test_child_pill_after_reopening_closed_parent_tab() -> Builder {
    Builder::new()
        .with_user_defaults(HashMap::from([
            ("UndoCloseEnabled".to_string(), "true".to_string()),
            (
                "UndoCloseGracePeriod".to_string(),
                serde_json::to_string(&Duration::from_secs(120)).unwrap(),
            ),
        ]))
        .with_steps(child_pill_after_reopening_closed_parent_tab())
}
