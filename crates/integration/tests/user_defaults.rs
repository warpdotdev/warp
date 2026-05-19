use integration::user_defaults;
use settings::Setting as _;
use warp::settings::{AppLanguage, AppLanguageSetting};

#[test]
fn integration_default_user_defaults_pin_app_language_to_english() {
    let defaults = user_defaults::defaults_for_integration_tests();
    let language = defaults
        .get(AppLanguageSetting::storage_key())
        .expect("integration defaults should set the app language");

    assert_eq!(
        serde_json::from_str::<AppLanguage>(language).expect("app language should deserialize"),
        AppLanguage::English
    );
}
