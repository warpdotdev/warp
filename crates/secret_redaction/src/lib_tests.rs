use regex::Regex;

use super::*;

#[test]
fn test_firebase_domain() {
    let firebase_domain = "warp-server-staging.firebaseapp.com";
    assert_regex_match_found(regexes::FIREBASE_AUTH_DOMAIN, firebase_domain);

    let bad_firebase_domain = "warp-server-staging_.firebaseapp.com";
    assert_regex_match_not_found(regexes::FIREBASE_AUTH_DOMAIN, bad_firebase_domain);
}

#[test]
fn test_stripe_api_key() {
    let stripe_api_key = "sk_live_4eC39HqLyjWDarjtT1zdp7dc";
    let stripe_restricted_api_key = "rk_live_4eC39HqLyjWDarjtT1zdp7dc";
    let stripe_api_test_key = "sk_test_4eC39HqLyjWDarjtT1zdp7dc";
    assert_regex_match_found(regexes::STRIPE_KEY, stripe_api_key);
    assert_regex_match_found(regexes::STRIPE_KEY, stripe_restricted_api_key);
    assert_regex_match_found(regexes::STRIPE_KEY, stripe_api_test_key);
}

#[test]
fn test_github_tokens() {
    let classic_personal_token = "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";
    let fine_grained_personal_access_token = "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";
    let gh_oauth_token = "gho_16C7e42F292c6912E7710c838347Ae178B4a";
    assert_regex_match_found(
        regexes::GITHUB_CLASSIC_PERSONAL_ACCESS_TOKEN,
        classic_personal_token,
    );
    assert_regex_match_found(
        regexes::GITHUB_FINE_GRAINED_PERSONAL_ACCESS_TOKEN,
        fine_grained_personal_access_token,
    );
    assert_regex_match_found(regexes::GITHUB_OAUTH_ACCESS_TOKEN, gh_oauth_token);
}

#[test]
fn test_phone_number() {
    let phone_number_dashes = "123-456-7890";
    let phone_number_default_format = "(123) 456-7890";
    let phone_number_spaces = "123 456 7890";
    let phone_number_dots = "123.456.7890";
    let international_number = "+91 (123) 456-7890";
    let phone_number_in_sentence = "Phone number: 123-456-7890.";
    assert_regex_match_found(regexes::PHONE_NUMBER, phone_number_dashes);
    assert_regex_match_found(regexes::PHONE_NUMBER, phone_number_default_format);
    assert_regex_match_found(regexes::PHONE_NUMBER, phone_number_spaces);
    assert_regex_match_found(regexes::PHONE_NUMBER, phone_number_dots);
    assert_regex_match_found(regexes::PHONE_NUMBER, international_number);
    assert_regex_match_found(regexes::PHONE_NUMBER, phone_number_in_sentence);

    let unformatted_phone_number = "1234567890";
    // We don't want to match on unformatted phone numbers, since obfuscating
    // every 10 digit number will lead to false positives.
    assert_regex_match_not_found(regexes::PHONE_NUMBER, unformatted_phone_number);
}

#[test]
fn test_google_api_key() {
    let google_api_key = "AIzaSyDaGmWKa4JsXZ-HjGw7ISLn_3namBGewQe";
    assert_regex_match_found(regexes::GOOGLE_API_KEY, google_api_key);
}

// Note all of these keys are fake and made up for testing purposes.
#[test]
fn test_openai_api_key() {
    let openai_key = "sk-5q3faKUz5GMaR8gX2Mf0u1dKgD9u9svG1cECxAaFbYGHZcJG";
    assert_regex_match_found(regexes::OPENAI_API_KEY, openai_key);
}

#[test]
fn test_anthropic_api_key() {
    let anthropic_key = "sk-ant-api03-E0eqRhui1BHGVEkx4aGj8RF6St9vjgGfUhAmwOkOGpDXxTm1qLJfBWT87iWlwapOWeD882FNZfqQLWn-dUnuwQ-hIhDiBBB";
    assert_regex_match_found(regexes::ANTHROPIC_API_KEY, anthropic_key);
}

#[test]
fn test_generic_sk_api_key() {
    let generic_sk_key = "sk-prod-super-secret-key-XYZ123abc-987654";
    assert_regex_match_found(regexes::GENERIC_SK_API_KEY, generic_sk_key);
}

#[test]
fn test_fireworks_api_key() {
    let fireworks_key = "fw_6AM3Dbkj3aiKcTe6fMabW4Zx";
    assert_regex_match_found(regexes::FIREWORKS_API_KEY, fireworks_key);
}

#[test]
fn test_mac_address() {
    let mac_addr_dashes = "00-B0-D0-63-C2-26";
    let mac_addr_colons = "c6:2c:99:54:5f:ef";
    assert_regex_match_found(regexes::MAC_ADDRESS, mac_addr_dashes);
    assert_regex_match_found(regexes::MAC_ADDRESS, mac_addr_colons);

    let unformatted_mac_addr = "00B0D063C226";
    assert_regex_match_not_found(regexes::MAC_ADDRESS, unformatted_mac_addr);
}

#[test]
fn test_aws_access_id() {
    let amazon_credentials = r#"{
        "Credentials": {
            "AccessKeyId": "ASIAZRUF5DH7SYC4A3NF",
        }
      }"#;
    assert_regex_match_found(regexes::AWS_ACCESS_ID, amazon_credentials);
}

#[test]
fn test_ip_address() {
    let ipv4_address = "192.0.2.1";
    let ivp6_address = "2001:0db8:85a3:0000:0000:8a2e:0370:7334";
    let false_ip_address = "Grid::scan_dirty_cells_for_secrets";
    let double_colon = "::";
    let ip_in_sentence = "Could not connect to 143.63.215.9";
    let warp_version_number = "v0.2023.08.01.08.04.dev_00";
    assert_regex_match_found(regexes::IPV4_ADDRESS, ipv4_address);
    assert_regex_match_found(regexes::IPV4_ADDRESS, ip_in_sentence);
    assert_regex_match_not_found(regexes::IPV4_ADDRESS, ivp6_address);
    assert_regex_match_not_found(regexes::IPV4_ADDRESS, warp_version_number);

    assert_regex_match_found(regexes::IPV6_ADDRESS, ivp6_address);
    assert_regex_match_not_found(regexes::IPV6_ADDRESS, ipv4_address);
    assert_regex_match_not_found(regexes::IPV6_ADDRESS, false_ip_address);
    assert_regex_match_not_found(regexes::IPV6_ADDRESS, double_colon);
}

fn assert_regex_match_found(given_regex: &str, match_string: &str) {
    let test_regex = Regex::new(given_regex).expect("Test regex did not compile");
    assert!(test_regex.is_match(match_string));
}

fn assert_regex_match_not_found(given_regex: &str, match_string: &str) {
    let test_regex = Regex::new(given_regex).expect("Test regex did not compile");
    assert!(!test_regex.is_match(match_string));
}

#[test]
fn test_jwt() {
    let jwt = "eyJ0eXAiOiJKV1QiLCJhbGciOiJSUzI1NiJ9.eyJpc3MiOiJodHRwczovL3NlY3VyZXRva2VuLmdvb2dsZS5jb20vd2FycC1zZXJ2ZXItc3RhZ2luZyIsImVtYWlsIjoiYW5keUB3YXJwLmRldiJ9.py4ujtlMjUOs5FpM8c9cpA-bwc8-VQTlhnWDBcjHrmkZwkPtOGN0Sel3EKLjFaC3YlgGMOfgyC80q5f6XG_uuQ8b";
    let missing_header = "eyJpc3MiOiJodHRwczovL3NlY3VyZXRva2VuLmdvb2dsZS5jb20vd2FycC1zZXJ2ZXItc3RhZ2luZyIsImVtYWlsIjoiYW5keUB3YXJwLmRldiJ9.py4ujtlMjUOs5FpM8c9cpA-bwc8-VQTlhnWDBcjHrmkZwkPtOGN0Sel3EKLjFaC3YlgGMOfgyC80q5f6XG_uuQ8b";
    let missing_claims = "eyJ0eXAiOiJKV1QiLCJhbGciOiJSUzI1NiJ9.py4ujtlMjUOs5FpM8c9cpA-bwc8-VQTlhnWDBcjHrmkZwkPtOGN0Sel3EKLjFaC3YlgGMOfgyC80q5f6XG_uuQ8b";
    let missing_signature = "eyJ0eXAiOiJKV1QiLCJhbGciOiJSUzI1NiJ9.eyJpc3MiOiJodHRwczovL3NlY3VyZXRva2VuLmdvb2dsZS5jb20vd2FycC1zZXJ2ZXItc3RhZ2luZyIsImVtYWlsIjoiYW5keUB3YXJwLmRldiJ9";
    let missing_periods = "eyJ0eXAiOiJKV1QiLCJhbGciOiJSUzI1NiJ9eyJpc3MiOiJodHRwczovL3NlY3VyZXRva2VuLmdvb2dsZS5jb20vd2FycC1zZXJ2ZXItc3RhZ2luZyIsImVtYWlsIjoiYW5keUB3YXJwLmRldiJ9py4ujtlMjUOs5FpM8c9cpA-bwc8-VQTlhnWDBcjHrmkZwkPtOGN0Sel3EKLjFaC3YlgGMOfgyC80q5f6XG_uuQ8b";
    let not_a_jwt = "noblenumbat24.jammyjellyfish22.focalfossa20";
    assert_regex_match_found(regexes::JWT, jwt);
    assert_regex_match_not_found(regexes::JWT, missing_header);
    assert_regex_match_not_found(regexes::JWT, missing_claims);
    assert_regex_match_not_found(regexes::JWT, missing_signature);
    assert_regex_match_not_found(regexes::JWT, missing_periods);
    assert_regex_match_not_found(regexes::JWT, not_a_jwt);
}
