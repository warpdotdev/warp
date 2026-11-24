use super::*;
use crate::server::server_api::ai::RunPreview;

#[test]
fn parses_listener_port_configuration() {
    assert_eq!(listener_port("1").unwrap(), 1);
    assert_eq!(listener_port("65535").unwrap(), 65535);
    for value in ["", "0", "65536", "+30000", " 30000"] {
        assert!(listener_port(value).is_err(), "{value:?}");
    }
}

fn preview(id: &str, port: u16, url: &str) -> RunPreview {
    RunPreview {
        port,
        id: Some(id.to_owned()),
        url: Some(url.to_owned()),
    }
}

#[test]
fn preview_records_preserve_uuid_port_and_origin() {
    let id = "550e8400-e29b-41d4-a716-446655440003";
    let records = preview_records(
        ListRunPreviewsResponse {
            execution_id: Some(42),
            previews: vec![preview(id, 3000, "https://PREVIEW.example.test:443/")],
        },
        42,
    )
    .unwrap();

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, Uuid::parse_str(id).unwrap());
    assert_eq!(records[0].port, 3000);
    assert_eq!(records[0].origin, "https://preview.example.test");
}

#[test]
fn preview_origin_uses_only_scheme_host_and_port() {
    for (url, expected) in [
        (
            "HTTPS://PREVIEW.example.test:443/path?query=1#fragment",
            "https://preview.example.test",
        ),
        (
            "http://preview.example.test:80/path?query=1#fragment",
            "http://preview.example.test",
        ),
        (
            "https://preview.example.test:8443/path?query=1#fragment",
            "https://preview.example.test:8443",
        ),
        (
            "https://[2001:db8::1]:8443/path?query=1#fragment",
            "https://[2001:db8::1]:8443",
        ),
        (
            "http://[::1]:8080/path?query=1#fragment",
            "http://[::1]:8080",
        ),
    ] {
        assert_eq!(preview_origin(url).unwrap(), expected, "{url}");
    }
}

#[test]
fn preview_records_reject_execution_mismatch_and_missing_fields() {
    let valid = preview(
        "550e8400-e29b-41d4-a716-446655440003",
        3000,
        "https://preview.example.test",
    );
    assert!(
        preview_records(
            ListRunPreviewsResponse {
                execution_id: Some(41),
                previews: vec![valid],
            },
            42,
        )
        .is_err()
    );

    let mut invalid_id = preview("not-a-uuid", 3000, "https://preview.example.test");
    invalid_id.id = Some("not-a-uuid".to_owned());
    let mut partial_origin = preview(
        "550e8400-e29b-41d4-a716-446655440003",
        3000,
        "https://preview.example.test",
    );
    partial_origin.url = None;
    let mut partial_id = preview(
        "550e8400-e29b-41d4-a716-446655440003",
        3000,
        "https://preview.example.test",
    );
    partial_id.id = None;
    for invalid in [invalid_id, partial_origin, partial_id] {
        assert!(
            preview_records(
                ListRunPreviewsResponse {
                    execution_id: Some(42),
                    previews: vec![invalid],
                },
                42,
            )
            .is_err()
        );
    }
}

#[test]
fn preview_records_reject_invalid_urls_and_duplicate_identity() {
    let mut nil_id = preview(
        "550e8400-e29b-41d4-a716-446655440003",
        3000,
        "https://preview.example.test",
    );
    nil_id.id = Some(Uuid::nil().to_string());
    assert!(
        preview_records(
            ListRunPreviewsResponse {
                execution_id: Some(42),
                previews: vec![nil_id],
            },
            42,
        )
        .is_err()
    );
    for url in [
        "not a URL",
        "javascript:alert(1)",
        "ftp://preview.example.test/path",
        "https://user:password@preview.example.test",
    ] {
        assert!(
            preview_records(
                ListRunPreviewsResponse {
                    execution_id: Some(42),
                    previews: vec![preview("550e8400-e29b-41d4-a716-446655440003", 3000, url,)],
                },
                42,
            )
            .is_err(),
            "{url}"
        );
    }

    let id = "550e8400-e29b-41d4-a716-446655440003";
    let cases = [
        vec![
            preview(id, 3000, "https://one.example.test"),
            preview(id, 3001, "https://two.example.test"),
        ],
        vec![
            preview(id, 3000, "https://one.example.test"),
            preview(
                "550e8400-e29b-41d4-a716-446655440004",
                3000,
                "https://two.example.test",
            ),
        ],
        vec![
            preview(id, 3000, "https://example.test"),
            preview(
                "550e8400-e29b-41d4-a716-446655440004",
                3001,
                "https://EXAMPLE.test/",
            ),
        ],
    ];
    for previews in cases {
        assert!(
            preview_records(
                ListRunPreviewsResponse {
                    execution_id: Some(42),
                    previews,
                },
                42,
            )
            .is_err()
        );
    }
}
