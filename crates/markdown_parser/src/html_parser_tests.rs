use super::*;

// Simple transformer to make testing easier.
fn test_parse_html(source: &str) -> Vec<FormattedTextLine> {
    parse_html(source).unwrap().lines.into()
}

#[test]
fn test_parse_plain_text() {
    assert_eq!(
        test_parse_html("<meta charset='utf-8'>Some"),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("Some"),
        ])]
    );

    assert_eq!(
        test_parse_html("<meta charset='utf-8'><p>Some</p><p>tests</p>"),
        vec![
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("Some"),]),
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("tests"),])
        ]
    );

    // Example from GDocs.
    assert_eq!(
        test_parse_html(
            "<meta charset='utf-8'><meta charset=\"utf-8\"><b style=\"font-weight:normal;\" id=\"docs-internal-guid-27b0e865-7fff-b40d-5b19-8e9e7ccf7c8c\">\
        <p dir=\"ltr\" style=\"line-height:1.38;margin-top:0pt;margin-bottom:0pt;\"><span style=\"font-size:11pt;font-family:Arial;color:#000000;background-color:transparent;font-weight:400;font-style:normal;\
        font-variant:normal;text-decoration:none;vertical-align:baseline;white-space:pre;white-space:pre-wrap;\">Some plain</span></p><p dir=\"ltr\" style=\"line-height:1.38;margin-top:0pt;margin-bottom:0pt;\">\
        <span style=\"font-size:11pt;font-family:Arial;color:#000000;background-color:transparent;font-weight:400;font-style:normal;font-variant:normal;text-decoration:none;vertical-align:baseline;white-space:pre;\
        white-space:pre-wrap;\">text</span></p></b>"
        ),
        vec![
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("Some plain"),]),
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("text"),])
        ]
    );
}

#[test]
fn test_parse_text_styles() {
    assert_eq!(
        test_parse_html(
            "<meta charset='utf-8'>So<span style=\"font-weight:600\" data-token-index=\"1\" class=\"notion-enable-hover\">me</span>"
        ),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("So"),
            FormattedTextFragment::bold("me"),
        ])]
    );

    assert_eq!(
        test_parse_html(
            "<meta charset='utf-8'><p>So<strong>me</strong></p><p><em><strong>tes</strong></em>ts</p>"
        ),
        vec![
            FormattedTextLine::Line(vec![
                FormattedTextFragment::plain_text("So"),
                FormattedTextFragment::bold("me")
            ]),
            FormattedTextLine::Line(vec![
                FormattedTextFragment::bold_italic("tes"),
                FormattedTextFragment::plain_text("ts")
            ])
        ]
    );

    assert_eq!(
        test_parse_html(
            "<meta charset='utf-8'><meta charset=\"utf-8\"><b style=\"font-weight:normal;\" id=\"docs-internal-guid-a96b449f-7fff-d755-78a4-efcebc867940\">\
        <p dir=\"ltr\" style=\"line-height:1.38;margin-top:0pt;margin-bottom:0pt;\"><span style=\"font-size:11pt;font-family:Arial;color:#000000;background-color:transparent;\
        font-weight:400;font-style:normal;font-variant:normal;text-decoration:none;vertical-align:baseline;white-space:pre;white-space:pre-wrap;\">So</span>\
        <span style=\"font-size:11pt;font-family:Arial;color:#000000;background-color:transparent;font-weight:400;font-style:italic;font-variant:normal;text-decoration:none;vertical-align:baseline;white-space:pre;white-space:pre-wrap;\">\
        me</span><span style=\"font-size:11pt;font-family:Arial;color:#000000;background-color:transparent;font-weight:700;font-style:italic;font-variant:normal;text-decoration:none;vertical-align:baseline;white-space:pre;white-space:pre-wrap;\">\
         p</span><span style=\"font-size:11pt;font-family:Arial;color:#000000;background-color:transparent;font-weight:700;font-style:normal;font-variant:normal;text-decoration:none;vertical-align:baseline;white-space:pre;white-space:pre-wrap;\">\
         la</span><span style=\"font-size:11pt;font-family:Arial;color:#000000;background-color:transparent;font-weight:400;font-style:normal;font-variant:normal;text-decoration:none;vertical-align:baseline;white-space:pre;white-space:pre-wrap;\">in\
         </span></p><span style=\"font-size:11pt;font-family:Arial;color:#000000;background-color:transparent;font-weight:400;font-style:normal;font-variant:normal;text-decoration:none;vertical-align:baseline;white-space:pre;white-space:pre-wrap;\">t\
         </span><span style=\"font-size:11pt;font-family:Arial;color:#000000;background-color:transparent;font-weight:400;font-style:italic;font-variant:normal;text-decoration:none;vertical-align:baseline;white-space:pre;white-space:pre-wrap;\">ext</span></b>"
        ),
        vec![
            FormattedTextLine::Line(vec![
                FormattedTextFragment::plain_text("So"),
                FormattedTextFragment::italic("me"),
                FormattedTextFragment::bold_italic("p"),
                FormattedTextFragment::bold("la"),
                FormattedTextFragment::plain_text("in")
            ]),
            FormattedTextLine::Line(vec![
                FormattedTextFragment::plain_text("t"),
                FormattedTextFragment::italic("ext")
            ])
        ]
    );

    assert_eq!(
        test_parse_html(
            "<meta charset='utf-8'><meta charset=\"utf-8\"><b style=\"font-weight:normal;\" id=\"docs-internal-guid-27b0e865-7fff-b40d-5b19-8e9e7ccf7c8c\">\
        <p dir=\"ltr\" style=\"line-height:1.38;margin-top:0pt;margin-bottom:0pt;\"><span style=\"font-size:11pt;font-family:Arial;color:#000000;background-color:transparent;font-weight:400;font-style:normal;\
        font-variant:normal;text-decoration:underline;vertical-align:baseline;white-space:pre;white-space:pre-wrap;\">This is underlined</span></p><p dir=\"ltr\" style=\"line-height:1.38;margin-top:0pt;margin-bottom:0pt;\">\
        <span style=\"font-size:11pt;font-family:Arial;color:#000000;background-color:transparent;font-weight:400;font-style:normal;font-variant:normal;text-decoration:none;vertical-align:baseline;white-space:pre;\
        white-space:pre-wrap;\"> text</span></p></b>"
        ),
        vec![
            FormattedTextLine::Line(vec![FormattedTextFragment::underline("This is underlined")]),
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text(" text")]),
        ]
    );

    assert_eq!(
        test_parse_html(
            "<meta charset='utf-8'>abc <a href=\"https://google.com/\" style=\"cursor:pointer;color:inherit;word-wrap:break-word;text-decoration:inherit\" class=\"notion-link-token notion-focusable-token notion-enable-hover\" rel=\"noopener noreferrer\"\
            data-token-index=\"1\" tabindex=\"0\"><span style=\"border-bottom:0.05em solid;border-color:rgba(55,53,47,0.4);opacity:0.7\" class=\"link-annotation-unknown-block-id--760432549\">NewLinadafekene</span></a> ghi def"
        ),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("abc "),
            FormattedTextFragment::hyperlink("NewLinadafekene", "https://google.com/"),
            FormattedTextFragment::plain_text(" ghi def")
        ]),]
    );

    assert_eq!(
        test_parse_html(
            r#"<meta charset='utf-8'>the <span style="font-family:&quot;SFMono-Regular&quot;, Menlo, Consolas, &quot;PT Mono&quot;, &quot;Liberation Mono&quot;, Courier, monospace;line-height:normal;background:rgba(135,131,120,.15);color:#EB5757;border-radius:3px;font-size:85%;padding:0.2em 0.4em" data-token-index="1" spellcheck="false" class="notion-enable-hover">&lt;ul&gt;</span> and <span style="font-family:&quot;SFMono-Regular&quot;, Menlo, Consolas, &quot;PT Mono&quot;, &quot;Liberation Mono&quot;, Courier, monospace;line-height:normal;background:rgba(135,131,120,.15);color:#EB5757;border-radius:3px;font-size:85%;padding:0.2em 0.4em" data-token-index="3" spellcheck="false" class="notion-enable-hover">&lt;li&gt;</span>"#
        ),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::plain_text("the "),
            FormattedTextFragment::inline_code("<ul>"),
            FormattedTextFragment::plain_text(" and "),
            FormattedTextFragment::inline_code("<li>"),
        ]),]
    );

    assert_eq!(
        test_parse_html(
            r#"<meta charset='utf-8'><span style="text-decoration:line-through" data-token-index="0" class="notion-enable-hover">Strike</span><span style="font-weight:600" data-token-index="1" class="notion-enable-hover">Bold</span>"#
        ),
        vec![FormattedTextLine::Line(vec![
            FormattedTextFragment::strikethrough("Strike"),
            FormattedTextFragment::bold("Bold"),
        ]),]
    )
}

#[test]
fn test_block() {
    assert_eq!(
        test_parse_html(
            "<meta charset='utf-8'><pre><code class=\"language-jsx\">git checkout -b branch</code></pre>"
        ),
        vec![FormattedTextLine::CodeBlock(CodeBlockText {
            lang: "jsx".to_string(),
            code: "git checkout -b branch".to_string()
        })]
    );

    assert_eq!(
        test_parse_html(
            "<meta charset='utf-8'><p>tests</p><pre><code class=\"language-jsx\">git checkout -b branch</code></pre><p>More</p>"
        ),
        vec![
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("tests"),]),
            FormattedTextLine::CodeBlock(CodeBlockText {
                lang: "jsx".to_string(),
                code: "git checkout -b branch".to_string()
            }),
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("More"),])
        ]
    );
}

#[test]
fn test_transform_non_breaking_spaces() {
    let expected_text = vec![FormattedTextLine::Line(vec![
        FormattedTextFragment::plain_text(
            "Open the Docker desktop app. This is necessary to create the symbolic links that will make the",
        ),
        FormattedTextFragment::plain_text(" "),
        FormattedTextFragment::inline_code("docker"),
        FormattedTextFragment::plain_text(" "),
        FormattedTextFragment::plain_text("CLI available."),
    ])];

    let chromium_html = r#"Open the Docker desktop app. This is necessary to create the symbolic links that will make the<span> </span><code>docker</code><span> </span>CLI available."#;
    assert_eq!(test_parse_html(chromium_html), expected_text);

    let safari_html = r#"Open the Docker desktop app. This is necessary to create the symbolic links that will make the<span class="Apple-converted-space"> </span><code>docker</code><span class="Apple-converted-space"> </span>CLI available."#;
    assert_eq!(test_parse_html(safari_html), expected_text);
}

// TODO: remove/update this test when we eventually support these HTML element types!
#[test]
fn test_unsupported_html_types() {
    assert_eq!(
        test_parse_html(
            "<meta charset='utf-8'><p>Test test</p><blockquote><p>Block quotes with <strong>bold text</strong></p></blockquote>"
        ),
        vec![
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("Test test"),]),
            FormattedTextLine::Line(vec![
                FormattedTextFragment::plain_text("Block quotes with "),
                FormattedTextFragment::bold("bold text")
            ])
        ]
    );

    assert_eq!(
        test_parse_html(
            "<meta charset='utf-8'><table><thead><tr><th>Text 1</th><th>Text 2</th></tr></thead><tbody><tr><td>Test</td><td>Test</td></tr></tbody></table>"
        ),
        vec![
            FormattedTextLine::Line(vec![
                FormattedTextFragment::plain_text("Text 1"),
                FormattedTextFragment::plain_text("Text 2")
            ]),
            FormattedTextLine::Line(vec![
                FormattedTextFragment::plain_text("Test"),
                FormattedTextFragment::plain_text("Test")
            ])
        ]
    );
}

#[test]
fn test_sub_lists() {
    assert_eq!(
        test_parse_html(
            "<meta charset='utf-8'><ul><li>def<ul><li>abc<ul><li>sub-list</li></ul></li><li>abc</li></ul></li></ul>"
        ),
        vec![
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 0,
                text: vec![FormattedTextFragment::plain_text("def")]
            }),
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 1,
                text: vec![FormattedTextFragment::plain_text("abc")]
            }),
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 2,
                text: vec![FormattedTextFragment::plain_text("sub-list")]
            }),
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 1,
                text: vec![FormattedTextFragment::plain_text("abc")]
            })
        ]
    );

    assert_eq!(
        test_parse_html(
            "<meta charset='utf-8'><ul><li>d<strong>ef</strong><ul><li><em>abc</em><ul><li>sub-list</li></ul></li><li>abc</li></ul></li></ul><p>normal text</p>"
        ),
        vec![
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 0,
                text: vec![
                    FormattedTextFragment::plain_text("d"),
                    FormattedTextFragment::bold("ef")
                ]
            }),
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 1,
                text: vec![FormattedTextFragment::italic("abc")]
            }),
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 2,
                text: vec![FormattedTextFragment::plain_text("sub-list")]
            }),
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 1,
                text: vec![FormattedTextFragment::plain_text("abc")]
            }),
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("normal text")])
        ]
    );

    assert_eq!(
        test_parse_html(
            "<meta charset='utf-8'><ul><li>abc<ul><li>def</li></ul></li></ul><ol><li>abc<ol><li>def</li></ol></li></ol>"
        ),
        vec![
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 0,
                text: vec![FormattedTextFragment::plain_text("abc")]
            }),
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 1,
                text: vec![FormattedTextFragment::plain_text("def")]
            }),
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                number: None,
                indented_text: FormattedIndentTextInline {
                    indent_level: 0,
                    text: vec![FormattedTextFragment::plain_text("abc".to_string())]
                }
            }),
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                number: None,
                indented_text: FormattedIndentTextInline {
                    indent_level: 1,
                    text: vec![FormattedTextFragment::plain_text("def".to_string())]
                }
            }),
        ]
    );

    assert_eq!(
        test_parse_html("<meta charset='utf-8'><ul><li>abc<ol><li>def</li></ol></li></ul>"),
        vec![
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 0,
                text: vec![FormattedTextFragment::plain_text("abc".to_string())]
            }),
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                number: None,
                indented_text: FormattedIndentTextInline {
                    indent_level: 1,
                    text: vec![FormattedTextFragment::plain_text("def")]
                }
            }),
        ]
    );
}

#[test]
fn test_formatted_sub_lists() {
    assert_eq!(
        test_parse_html(
            "<meta charset='utf-8'><ul>\n<li>By default, the client goes to staging\n<ul>\n<li>To target localhost, build the client with</li>\n</ul>\n</li>\n</ul>\n"
        ),
        vec![
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 0,
                text: vec![FormattedTextFragment::plain_text(
                    "By default, the client goes to staging"
                )]
            }),
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 1,
                text: vec![FormattedTextFragment::plain_text(
                    "To target localhost, build the client with"
                )]
            }),
        ]
    );
}

#[test]
fn test_ordered_lists() {
    let html = r#"
<meta charset='utf-8'>
<ol start="3">
  <li>First</li>
  <li>Second<ol>
      <li>A</li>
      <li>B</li>
    </ol>
  </li>
  <li>Third</li>
  <li>Fourth<ol start="invalid">
        <li>G</li>
        <li>H</li>
    </ol>
  </li>
</ol>
    "#;

    assert_eq!(
        test_parse_html(html),
        vec![
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                number: Some(3),
                indented_text: FormattedIndentTextInline {
                    indent_level: 0,
                    text: vec![FormattedTextFragment::plain_text("First")]
                }
            }),
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                number: None,
                indented_text: FormattedIndentTextInline {
                    indent_level: 0,
                    text: vec![FormattedTextFragment::plain_text("Second")]
                }
            }),
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                number: None,
                indented_text: FormattedIndentTextInline {
                    indent_level: 1,
                    text: vec![FormattedTextFragment::plain_text("A")]
                }
            }),
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                number: None,
                indented_text: FormattedIndentTextInline {
                    indent_level: 1,
                    text: vec![FormattedTextFragment::plain_text("B")]
                }
            }),
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                number: None,
                indented_text: FormattedIndentTextInline {
                    indent_level: 0,
                    text: vec![FormattedTextFragment::plain_text("Third")]
                }
            }),
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                number: None,
                indented_text: FormattedIndentTextInline {
                    indent_level: 0,
                    text: vec![FormattedTextFragment::plain_text("Fourth")]
                }
            }),
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                number: None,
                indented_text: FormattedIndentTextInline {
                    indent_level: 1,
                    text: vec![FormattedTextFragment::plain_text("G")]
                }
            }),
            FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                number: None,
                indented_text: FormattedIndentTextInline {
                    indent_level: 1,
                    text: vec![FormattedTextFragment::plain_text("H")]
                }
            }),
        ]
    );
}

#[test]
fn test_horizontal_rules() {
    assert_eq!(
        test_parse_html("<meta charset='utf-8'><ul>\n<li>bcf</li>\n</ul>\n<hr>\n<p>abc</p>"),
        vec![
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 0,
                text: vec![FormattedTextFragment::plain_text("bcf")]
            }),
            FormattedTextLine::HorizontalRule,
            FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("abc")])
        ]
    );
}

#[test]
fn test_headings() {
    assert_eq!(
        test_parse_html(
            "<h1>Heading 1</h1><h2>Heading 2</h2><h3>Heading 3</h3><h4>Heading 4</h4><h5>Heading 5</h5><h6>Heading 6</h6>"
        ),
        vec![
            FormattedTextLine::Heading(FormattedTextHeader {
                heading_size: 1,
                text: vec![FormattedTextFragment::plain_text("Heading 1")]
            }),
            FormattedTextLine::Heading(FormattedTextHeader {
                heading_size: 2,
                text: vec![FormattedTextFragment::plain_text("Heading 2")]
            }),
            FormattedTextLine::Heading(FormattedTextHeader {
                heading_size: 3,
                text: vec![FormattedTextFragment::plain_text("Heading 3")]
            }),
            FormattedTextLine::Heading(FormattedTextHeader {
                heading_size: 4,
                text: vec![FormattedTextFragment::plain_text("Heading 4")]
            }),
            FormattedTextLine::Heading(FormattedTextHeader {
                heading_size: 5,
                text: vec![FormattedTextFragment::plain_text("Heading 5")]
            }),
            FormattedTextLine::Heading(FormattedTextHeader {
                heading_size: 6,
                text: vec![FormattedTextFragment::plain_text("Heading 6")]
            }),
        ]
    );
}

#[test]
fn test_task_lists() {
    // HTML copied from Github.
    assert_eq!(
        test_parse_html(
            r#"<meta charset='utf-8'><ul data-sourcepos="3:1-4:26" class="contains-task-list" style="box-sizing: border-box; padding-left: 2em; margin-top: 0px; margin-bottom: 0px !important; position: relative; color: rgb(31, 35, 40); font-family: -apple-system, &quot;system-ui&quot;, &quot;Segoe UI&quot;, &quot;Noto Sans&quot;, Helvetica, Arial, sans-serif, &quot;Apple Color Emoji&quot;, &quot;Segoe UI Emoji&quot;; font-size: 16px; font-style: normal; font-variant-ligatures: normal; font-variant-caps: normal; font-weight: 400; letter-spacing: normal; orphans: 2; text-align: start; text-indent: 0px; text-transform: none; widows: 2; word-spacing: 0px; -webkit-text-stroke-width: 0px; white-space: normal; background-color: rgb(255, 255, 255); text-decoration-thickness: initial; text-decoration-style: initial; text-decoration-color: initial;"><li data-sourcepos="3:1-4:26" class="task-list-item" style="box-sizing: border-box; list-style-type: none;"><input type="checkbox" id="" disabled="" class="task-list-item-checkbox" style="box-sizing: border-box; font: inherit; margin: 0px 0.2em 0.25em -1.4em; overflow: visible; padding: 0px; vertical-align: middle;"><span> </span>Checklist item 1<ul data-sourcepos="4:5-4:26" class="contains-task-list" style="box-sizing: border-box; padding-left: 2em; margin-top: 0px; margin-bottom: 0px; position: relative;"><li data-sourcepos="4:5-4:26" class="task-list-item" style="box-sizing: border-box; list-style-type: none;"><input type="checkbox" id="" disabled="" class="task-list-item-checkbox" checked="" style="box-sizing: border-box; font: inherit; margin: 0px 0.2em 0.25em -1.4em; overflow: visible; padding: 0px; vertical-align: middle;"><span> </span>Checklist item 2</li></ul></li></ul>"#
        ),
        vec![
            FormattedTextLine::TaskList(FormattedTaskList {
                complete: false,
                indent_level: 0,
                text: vec![
                    FormattedTextFragment::plain_text(" "),
                    FormattedTextFragment::plain_text("Checklist item 1")
                ]
            }),
            FormattedTextLine::TaskList(FormattedTaskList {
                complete: true,
                indent_level: 1,
                text: vec![
                    FormattedTextFragment::plain_text(" "),
                    FormattedTextFragment::plain_text("Checklist item 2")
                ]
            }),
        ]
    );
}

#[test]
fn test_google_docs_formatted_sub_list() {
    assert_eq!(
        test_parse_html(
            r#"<meta charset='utf-8'><meta charset="utf-8"><b style="font-weight:normal;" id="docs-internal-guid-0eb9725b-7fff-4ed4-502c-49a79adaf702"><ul style="margin-top:0;margin-bottom:0;padding-inline-start:48px;"><li dir="ltr" style="list-style-type:disc;font-size:11pt;font-family:Arial,sans-serif;color:#000000;background-color:transparent;font-weight:400;font-style:normal;font-variant:normal;text-decoration:none;vertical-align:baseline;white-space:pre;" aria-level="1"><p dir="ltr" style="line-height:1.2;margin-top:0pt;margin-bottom:0pt;" role="presentation"><span style="font-size:11pt;font-family:Arial,sans-serif;color:#000000;background-color:transparent;font-weight:400;font-style:normal;font-variant:normal;text-decoration:none;vertical-align:baseline;white-space:pre;white-space:pre-wrap;">Favoriting drive objects</span></p></li><ul style="margin-top:0;margin-bottom:0;padding-inline-start:48px;"><li dir="ltr" style="list-style-type:circle;font-size:11pt;font-family:Arial,sans-serif;color:#000000;background-color:transparent;font-weight:400;font-style:normal;font-variant:normal;text-decoration:none;vertical-align:baseline;white-space:pre;" aria-level="2"><p dir="ltr" style="line-height:1.2;margin-top:0pt;margin-bottom:0pt;" role="presentation"><span style="font-size:11pt;font-family:Arial,sans-serif;color:#000000;background-color:transparent;font-weight:400;font-style:normal;font-variant:normal;text-decoration:none;vertical-align:baseline;white-space:pre;white-space:pre-wrap;">Something else</span></p></li></ul><li dir="ltr" style="list-style-type:disc;font-size:11pt;font-family:Arial,sans-serif;color:#000000;background-color:transparent;font-weight:400;font-style:normal;font-variant:normal;text-decoration:none;vertical-align:baseline;white-space:pre;" aria-level="1"><p dir="ltr" style="line-height:1.2;margin-top:0pt;margin-bottom:0pt;" role="presentation"><span style="font-size:11pt;font-family:Arial,sans-serif;color:#000000;background-color:transparent;font-weight:400;font-style:normal;font-variant:normal;text-decoration:none;vertical-align:baseline;white-space:pre;white-space:pre-wrap;">Drive keyboard nav</span></p></li></ul></b>"#,
        ),
        vec![
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 0,
                text: vec![FormattedTextFragment::plain_text(
                    "Favoriting drive objects"
                )]
            }),
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 1,
                text: vec![FormattedTextFragment::plain_text("Something else")]
            }),
            FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                indent_level: 0,
                text: vec![FormattedTextFragment::plain_text("Drive keyboard nav")]
            }),
        ]
    )
}

#[test]
fn test_code_and_inline_code() {
    let github_code_block = r#"<meta charset='utf-8'><p data-sourcepos="59:1-59:27" dir="auto" style="box-sizing: border-box; margin-top: 0px; margin-bottom: 16px; color: rgb(31, 35, 40); font-family: -apple-system, &quot;system-ui&quot;, &quot;Segoe UI&quot;, &quot;Noto Sans&quot;, Helvetica, Arial, sans-serif, &quot;Apple Color Emoji&quot;, &quot;Segoe UI Emoji&quot;; font-size: 16px; font-style: normal; font-variant-ligatures: normal; font-variant-caps: normal; font-weight: 400; letter-spacing: normal; orphans: 2; text-align: start; text-indent: 0px; text-transform: none; widows: 2; word-spacing: 0px; -webkit-text-stroke-width: 0px; white-space: normal; background-color: rgb(255, 255, 255); text-decoration-thickness: initial; text-decoration-style: initial; text-decoration-color: initial;">Some<span> </span><code style="box-sizing: border-box; font-family: ui-monospace, SFMono-Regular, &quot;SF Mono&quot;, Menlo, Consolas, &quot;Liberation Mono&quot;, monospace; font-size: 13.6px; padding: 0.2em 0.4em; margin: 0px; white-space: break-spaces; background-color: var(--bgColor-neutral-muted, var(--color-neutral-muted)); border-radius: 6px;">inline code</code><span> </span>to parse</p><div class="snippet-clipboard-content notranslate position-relative overflow-auto" style="box-sizing: border-box; position: relative !important; overflow: auto !important; color: rgb(31, 35, 40); font-family: -apple-system, &quot;system-ui&quot;, &quot;Segoe UI&quot;, &quot;Noto Sans&quot;, Helvetica, Arial, sans-serif, &quot;Apple Color Emoji&quot;, &quot;Segoe UI Emoji&quot;; font-size: 16px; font-style: normal; font-variant-ligatures: normal; font-variant-caps: normal; font-weight: 400; letter-spacing: normal; orphans: 2; text-align: start; text-indent: 0px; text-transform: none; widows: 2; word-spacing: 0px; -webkit-text-stroke-width: 0px; white-space: normal; background-color: rgb(255, 255, 255); text-decoration-thickness: initial; text-decoration-style: initial; text-decoration-color: initial;"><pre class="notranslate" style="box-sizing: border-box; font-family: ui-monospace, SFMono-Regular, &quot;SF Mono&quot;, Menlo, Consolas, &quot;Liberation Mono&quot;, monospace; font-size: 13.6px; margin-top: 0px; margin-bottom: 16px; overflow-wrap: normal; padding: 16px; overflow: auto; line-height: 1.45; color: var(--fgColor-default, var(--color-fg-default)); background-color: var(--bgColor-muted, var(--color-canvas-subtle)); border-radius: 6px;"><code style="box-sizing: border-box; font-family: ui-monospace, SFMono-Regular, &quot;SF Mono&quot;, Menlo, Consolas, &quot;Liberation Mono&quot;, monospace; font-size: 13.6px; padding: 0px; margin: 0px; white-space: pre; background: transparent; border-radius: 6px; word-break: normal; border: 0px; display: inline; overflow: visible; line-height: inherit; overflow-wrap: normal;">Some code block</code></pre></div>"#;
    assert_eq!(
        test_parse_html(github_code_block),
        vec![
            FormattedTextLine::Line(vec![
                FormattedTextFragment::plain_text("Some"),
                FormattedTextFragment::plain_text(" "),
                FormattedTextFragment::inline_code("inline code"),
                FormattedTextFragment::plain_text(" "),
                FormattedTextFragment::plain_text("to parse"),
            ]),
            FormattedTextLine::CodeBlock(CodeBlockText {
                lang: RUNNABLE_BLOCK_MARKDOWN_LANG.to_string(),
                code: "Some code block".to_string()
            }),
        ]
    );
}

// Test for CLD-860
#[test]
fn test_confluence_code_block() {
    let confluence_code_block = r#"<span data-code-lang="shell" data-ds--code--code-block="" class="prismjs css-1vd0zfg"><code class="language-shell"><span class="comment linenumber ds-line-number" data-ds--line-number="1" style="flex-shrink: 0; box-sizing: border-box; padding-left: 8px; margin-right: 8px; text-align: right; user-select: none; display: inline-block !important; min-width: calc(1ch + 16px) !important; font-style: normal !important; color: var(--ds-text-subtlest, #505F79) !important; padding-right: 8px !important; float: left;"></span><span class="">This is a code block</span></code></span>"#;
    assert_eq!(
        test_parse_html(confluence_code_block),
        vec![FormattedTextLine::CodeBlock(CodeBlockText {
            lang: "shell".to_string(),
            code: "This is a code block".to_string()
        }),]
    );
}
