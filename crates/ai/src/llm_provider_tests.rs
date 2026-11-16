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

#[test]
fn only_pasted_key_providers_report_pasted_key_support() {
    for provider in LLMProvider::API_KEY_PROVIDERS {
        assert_eq!(
            provider.supports_pasted_api_key(),
            matches!(
                provider,
                LLMProvider::OpenAI | LLMProvider::Anthropic | LLMProvider::Google
            )
        );
    }
}
