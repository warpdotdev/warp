use super::*;

#[test]
fn qr_matrix_for_url_returns_square_matrix_with_dark_modules() {
    let matrix = match qr_matrix_for_url("https://app.warp.dev/session/test-session") {
        Ok(matrix) => matrix,
        Err(error) => panic!("failed to generate QR matrix: {error}"),
    };

    assert!(matrix.width() > 0);
    assert_eq!(matrix.modules.len(), matrix.width() * matrix.width());
    assert!(matrix.modules.iter().any(|is_dark| *is_dark));
}
