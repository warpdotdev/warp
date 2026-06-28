use warpui::browser::escape_html_attribute;

#[test]
fn escape_html_attribute_escapes_attribute_breakout_characters() {
    assert_eq!(
        escape_html_attribute("\" onload=\"alert(1)\" data-x='><script>alert(1)</script>&"),
        "&quot; onload=&quot;alert(1)&quot; data-x=&#39;&gt;&lt;script&gt;alert(1)&lt;/script&gt;&amp;"
    );
}

#[test]
fn escape_html_attribute_leaves_safe_text_unchanged() {
    assert_eq!(
        escape_html_attribute("embedded warp block"),
        "embedded warp block"
    );
}
