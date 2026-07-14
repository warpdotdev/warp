use super::SetupStep;

#[test]
fn cache_setup_step_maps_to_setup_caches() {
    let (name, _span) = SetupStep::CacheSetup.to_event_name_and_span();
    assert_eq!(name, "setup_caches");
}
