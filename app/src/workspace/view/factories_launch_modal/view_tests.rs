use super::*;

#[test]
fn with_email_id_prefill_appends_id_parameter_to_a_bare_url() {
    let result = with_email_id_prefill(
        "https://warp-dev.chilipiper.com/round-robin/factories-warp-intro",
        Some("ada@example.com"),
    );
    assert_eq!(
        result,
        "https://warp-dev.chilipiper.com/round-robin/factories-warp-intro?id=ada%40example.com"
    );
}

#[test]
fn with_email_id_prefill_adds_to_an_existing_query_string_rather_than_corrupting_it() {
    let result = with_email_id_prefill(
        "https://warp-dev.chilipiper.com/round-robin/factories-warp-intro?utm_source=warp",
        Some("ada@example.com"),
    );
    assert_eq!(
        result,
        "https://warp-dev.chilipiper.com/round-robin/factories-warp-intro?utm_source=warp&id=ada%40example.com"
    );
}

#[test]
fn with_email_id_prefill_replaces_a_preexisting_id_pair_instead_of_duplicating_it() {
    let result = with_email_id_prefill(
        "https://warp-dev.chilipiper.com/round-robin/factories-warp-intro?id=other&campaign=x",
        Some("ada@example.com"),
    );
    assert_eq!(
        result,
        "https://warp-dev.chilipiper.com/round-robin/factories-warp-intro?campaign=x&id=ada%40example.com"
    );
    assert_eq!(
        result.matches("id=").count(),
        1,
        "the result must carry exactly one id pair, not the configured one plus the signed-in user's"
    );
}

#[test]
fn with_email_id_prefill_leaves_the_url_untouched_when_email_is_unavailable() {
    let url = "https://warp-dev.chilipiper.com/round-robin/factories-warp-intro";
    assert_eq!(with_email_id_prefill(url, None), url);
}

#[test]
fn with_email_id_prefill_leaves_the_url_untouched_for_an_anonymous_users_empty_email() {
    let url = "https://warp-dev.chilipiper.com/round-robin/factories-warp-intro";
    assert_eq!(with_email_id_prefill(url, Some("")), url);
}

#[test]
fn with_email_id_prefill_leaves_an_unparseable_url_untouched() {
    let url = "not a url";
    assert_eq!(with_email_id_prefill(url, Some("ada@example.com")), url);
}
