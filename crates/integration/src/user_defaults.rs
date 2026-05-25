use settings::Setting as _;
use std::collections::HashMap;

use warp::settings::INPUT_MODE;
use warp::settings::{AppLanguage, AppLanguageSetting};
use warp::terminal::block_list_viewport::InputMode;

/// Returns user defaults that every integration test should start with.
pub fn defaults_for_integration_tests() -> HashMap<String, String> {
    HashMap::from_iter([(
        AppLanguageSetting::storage_key().to_owned(),
        serde_json::to_string(&AppLanguage::English)
            .expect("app_language value should convert to json string"),
    )])
}

/// Returns a user defaults map with the `InputMode` set to `input_mode`.
#[allow(dead_code)]
pub fn input_mode(input_mode: InputMode) -> HashMap<String, String> {
    HashMap::from_iter([(
        INPUT_MODE.to_owned(),
        serde_json::to_string(&input_mode).expect("input_mode value should convert to json string"),
    )])
}
