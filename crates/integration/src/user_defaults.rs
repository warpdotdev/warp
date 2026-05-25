use std::collections::HashMap;

use warp::settings::INPUT_MODE;
use warp::terminal::block_list_viewport::InputMode;

/// Returns a user defaults map with the `InputMode` set to `input_mode`.
#[allow(dead_code)]
pub fn input_mode(input_mode: InputMode) -> HashMap<String, String> {
    HashMap::from_iter([(
        INPUT_MODE.to_owned(),
        serde_json::to_string(&input_mode).expect("input_mode value should convert to json string"),
    )])
}
