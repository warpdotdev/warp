use super::LLMProvider;

#[test]
fn api_key_provider_help_matches_the_canonical_provider_list() {
    assert_eq!(
        LLMProvider::API_KEY_PROVIDERS
            .into_iter()
            .map(|provider| provider.api_key_slug().unwrap())
            .collect::<Vec<_>>()
            .join("|"),
        LLMProvider::API_KEY_PROVIDER_VALUE_NAME
    );
}
