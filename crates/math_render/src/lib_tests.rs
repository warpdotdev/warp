use super::*;

#[test]
fn renders_inline_math() {
    let svg = render_math_to_svg(r"O(n \log n)", false, "#e0e0e0", 16.0).expect("should render");
    assert!(svg.starts_with("<svg"), "output should be an SVG document");
    assert!(svg.contains("<path"), "glyphs should be embedded as paths");
}

#[test]
fn renders_display_math() {
    let svg = render_math_to_svg(
        r"\sum_{n=1}^{\infty} \frac{1}{n^2} = \frac{\pi^2}{6}",
        true,
        "#000000",
        16.0,
    )
    .expect("should render");
    assert!(svg.starts_with("<svg"));
}

#[test]
fn parse_error_on_invalid_latex() {
    let result = render_math_to_svg(r"\frac{unclosed", true, "#000000", 16.0);
    assert!(matches!(result, Err(MathRenderError::Parse(_))));
}

#[test]
fn error_on_invalid_color() {
    let result = render_math_to_svg("x", false, "not-a-color", 16.0);
    assert!(matches!(result, Err(MathRenderError::InvalidColor(_))));
}
