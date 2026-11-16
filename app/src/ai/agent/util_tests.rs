use super::parse_markdown_into_text_and_code_sections;
use crate::ai::agent::{AIAgentTextSection, AgentOutputImageLayout, AgentOutputTableRendering};
use crate::features::FeatureFlag;

#[test]
fn extracts_gfm_pipe_table_into_table_section() {
    let _flag = FeatureFlag::BlocklistMarkdownTableRendering.override_enabled(true);
    let input = "Intro\n\n| A | B |\n| --- | --- |\n| 1 | 2 |\n\nOutro";
    let sections = parse_markdown_into_text_and_code_sections(input);

    assert_eq!(sections.len(), 3);

    match &sections[0] {
        AIAgentTextSection::PlainText { text } => {
            assert!(text.text().contains("Intro"));
        }
        _ => panic!("expected first section to be PlainText"),
    }

    match &sections[1] {
        AIAgentTextSection::Table { table } => {
            assert_eq!(table.markdown_source, "| A | B |\n| --- | --- |\n| 1 | 2 |");
            match &table.rendering {
                AgentOutputTableRendering::Legacy { .. } => {
                    panic!("expected structured table rendering")
                }
                AgentOutputTableRendering::Structured { table } => {
                    assert_eq!(table.headers.len(), 2);
                    assert_eq!(table.rows.len(), 1);
                }
            }
            assert_eq!(
                table.rendered_lines(),
                vec!["A\tB".to_string(), "1\t2".to_string()]
            );
        }
        _ => panic!("expected second section to be Table"),
    }

    match &sections[2] {
        AIAgentTextSection::PlainText { text } => {
            assert!(text.text().contains("Outro"));
        }
        _ => panic!("expected third section to be PlainText"),
    }
}

#[test]
fn does_not_extract_pipe_text_without_separator_row() {
    let _flag = FeatureFlag::BlocklistMarkdownTableRendering.override_enabled(true);
    let input = "a | b\nc | d";
    let sections = parse_markdown_into_text_and_code_sections(input);

    assert_eq!(sections.len(), 1);
    assert!(matches!(sections[0], AIAgentTextSection::PlainText { .. }));
}

#[test]
fn table_can_be_followed_immediately_by_text() {
    let _flag = FeatureFlag::BlocklistMarkdownTableRendering.override_enabled(true);
    let input = "| A | B |\n|---|---|\n| 1 | 2 |\nAfter";
    let sections = parse_markdown_into_text_and_code_sections(input);

    assert_eq!(sections.len(), 2);
    assert!(matches!(sections[0], AIAgentTextSection::Table { .. }));
    match &sections[1] {
        AIAgentTextSection::PlainText { text } => {
            assert!(text.text().contains("After"));
        }
        _ => panic!("expected second section to be PlainText"),
    }
}

#[test]
fn extracts_gfm_pipe_table_into_legacy_table_section_when_flag_disabled() {
    let _flag = FeatureFlag::BlocklistMarkdownTableRendering.override_enabled(false);
    let input = "Intro\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\nOutro";
    let sections = parse_markdown_into_text_and_code_sections(input);

    assert_eq!(sections.len(), 3);

    match &sections[1] {
        AIAgentTextSection::Table { table } => {
            assert_eq!(
                table.markdown_source,
                "| A   | B   |\n| --- | --- |\n| 1   | 2   |"
            );
            assert_eq!(
                table.rendered_lines(),
                vec![
                    "| A   | B   |".to_string(),
                    "| --- | --- |".to_string(),
                    "| 1   | 2   |".to_string()
                ]
            );
            assert!(matches!(
                &table.rendering,
                AgentOutputTableRendering::Legacy { .. }
            ));
        }
        _ => panic!("expected second section to be Table"),
    }
}

#[test]
fn extracts_markdown_image_into_image_section() {
    let input = "Intro\n\n![Diagram](./diagram.png)\n\nOutro";
    let sections = parse_markdown_into_text_and_code_sections(input);

    assert_eq!(sections.len(), 3);
    assert!(matches!(sections[0], AIAgentTextSection::PlainText { .. }));
    match &sections[1] {
        AIAgentTextSection::Image { image } => {
            assert_eq!(image.alt_text, "Diagram");
            assert_eq!(image.source, "./diagram.png");
            assert_eq!(image.markdown_source, "![Diagram](./diagram.png)");
            assert_eq!(image.layout, AgentOutputImageLayout::Block);
        }
        _ => panic!("expected second section to be Image"),
    }
    assert!(matches!(sections[2], AIAgentTextSection::PlainText { .. }));
}

#[test]
fn extracts_multiple_markdown_images_in_order() {
    let input = "![One](one.png)\n![Two](two.png)\n";
    let sections = parse_markdown_into_text_and_code_sections(input);

    assert_eq!(sections.len(), 2);
    match &sections[0] {
        AIAgentTextSection::Image { image } => {
            assert_eq!(image.markdown_source, "![One](one.png)");
            assert_eq!(image.layout, AgentOutputImageLayout::Block);
        }
        _ => panic!("expected first section to be Image"),
    }
    match &sections[1] {
        AIAgentTextSection::Image { image } => {
            assert_eq!(image.markdown_source, "![Two](two.png)");
            assert_eq!(image.layout, AgentOutputImageLayout::Block);
        }
        _ => panic!("expected second section to be Image"),
    }
}

#[test]
fn extracts_same_line_markdown_images_into_inline_image_sections() {
    let input = "![One](one.png) ![Two](two.png)\n";
    let sections = parse_markdown_into_text_and_code_sections(input);

    assert_eq!(sections.len(), 2);
    for (section, expected_markdown) in sections.iter().zip(["![One](one.png)", "![Two](two.png)"])
    {
        match section {
            AIAgentTextSection::Image { image } => {
                assert_eq!(image.markdown_source, expected_markdown);
                assert_eq!(image.layout, AgentOutputImageLayout::Inline);
            }
            _ => panic!("expected inline image section"),
        }
    }
}

#[test]
fn does_not_extract_inline_image_run_from_mixed_text_line() {
    let input = "Intro ![One](one.png) ![Two](two.png)\n";
    let sections = parse_markdown_into_text_and_code_sections(input);

    assert_eq!(sections.len(), 1);
    assert!(matches!(sections[0], AIAgentTextSection::PlainText { .. }));
}

#[test]
fn extracts_block_image_with_commonmark_title() {
    let input = "![Rex](./rex.png \"My dog Rex\")\n";
    let sections = parse_markdown_into_text_and_code_sections(input);

    assert_eq!(sections.len(), 1);
    match &sections[0] {
        AIAgentTextSection::Image { image } => {
            assert_eq!(image.alt_text, "Rex");
            assert_eq!(image.source, "./rex.png");
            assert_eq!(image.title.as_deref(), Some("My dog Rex"));
            // Right-click copy uses `markdown_source`, so it must round-trip
            // the authored title (product invariant 9).
            assert_eq!(image.markdown_source, "![Rex](./rex.png \"My dog Rex\")");
        }
        _ => panic!("expected image section"),
    }
}

#[test]
fn extracts_inline_image_run_with_partial_title() {
    // The inline run contains two images where only the second carries a title.
    let input = "![One](one.png) ![Two](two.png \"caption\")\n";
    let sections = parse_markdown_into_text_and_code_sections(input);

    assert_eq!(sections.len(), 2);
    match &sections[0] {
        AIAgentTextSection::Image { image } => {
            assert_eq!(image.title, None);
            assert_eq!(image.markdown_source, "![One](one.png)");
            assert_eq!(image.layout, AgentOutputImageLayout::Inline);
        }
        _ => panic!("expected first inline image"),
    }
    match &sections[1] {
        AIAgentTextSection::Image { image } => {
            assert_eq!(image.title.as_deref(), Some("caption"));
            assert_eq!(image.markdown_source, "![Two](two.png \"caption\")");
            assert_eq!(image.layout, AgentOutputImageLayout::Inline);
        }
        _ => panic!("expected second inline image"),
    }
}

#[test]
fn block_image_with_empty_title_normalizes_to_none() {
    let input = "![Alt](image.png \"\")\n";
    let sections = parse_markdown_into_text_and_code_sections(input);

    assert_eq!(sections.len(), 1);
    match &sections[0] {
        AIAgentTextSection::Image { image } => {
            assert_eq!(image.title, None);
            // Empty titles normalize away, so `markdown_source` is the
            // canonical untitled form, not the original source text.
            assert_eq!(image.markdown_source, "![Alt](image.png)");
        }
        _ => panic!("expected image section"),
    }
}

#[test]
fn block_image_with_unclosed_title_falls_back_to_plain_text() {
    let input = "![Alt](image.png \"unterminated)\n";
    let sections = parse_markdown_into_text_and_code_sections(input);

    assert_eq!(sections.len(), 1);
    // Unclosed titles cause the whole image to render as plain text.
    assert!(matches!(sections[0], AIAgentTextSection::PlainText { .. }));
}

#[test]
fn extracts_mermaid_code_block_into_mermaid_section() {
    let input = "```mermaid\ngraph TD\nA[Start] --> B[Finish]\n```";
    let sections = parse_markdown_into_text_and_code_sections(input);

    assert_eq!(sections.len(), 1);
    match &sections[0] {
        AIAgentTextSection::MermaidDiagram { diagram } => {
            assert_eq!(diagram.source, "graph TD\nA[Start] --> B[Finish]");
            assert_eq!(
                diagram.markdown_source,
                "```mermaid\ngraph TD\nA[Start] --> B[Finish]\n```"
            );
        }
        _ => panic!("expected mermaid diagram section"),
    }
}

#[test]
fn extracts_multiple_mermaid_code_blocks_in_order() {
    let input = "```mermaid\ngraph TD\nA --> B\n```\n\n```mermaid\ngraph TD\nB --> C\n```";
    let sections = parse_markdown_into_text_and_code_sections(input);

    assert_eq!(sections.len(), 2);
    match &sections[0] {
        AIAgentTextSection::MermaidDiagram { diagram } => {
            assert_eq!(diagram.source, "graph TD\nA --> B");
        }
        _ => panic!("expected first section to be MermaidDiagram"),
    }
    match &sections[1] {
        AIAgentTextSection::MermaidDiagram { diagram } => {
            assert_eq!(diagram.source, "graph TD\nB --> C");
        }
        _ => panic!("expected second section to be MermaidDiagram"),
    }
}
