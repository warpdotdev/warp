use std::ops::Range;
use std::sync::Arc;

use warp_multi_agent_api::{FileContent, FileContentLineRange};

use crate::ai::agent::{
    AIAgentOutput, AIAgentOutputMessage, AIAgentOutputMessageType, AIAgentText, AIAgentTextSection,
    AgentOutputImage, AgentOutputImageLayout, AgentOutputMermaidDiagram, AnyFileContent,
    FileContext, FormattedTextWrapper, MessageId, ProgrammingLanguage,
};
use crate::terminal::shell::ShellType;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};

fn to_range(range: Range<u32>) -> Option<FileContentLineRange> {
    Some(FileContentLineRange {
        start: range.start,
        end: range.end,
    })
}

#[test]
fn formatted_text_wrapper_shares_arc_across_calls() {
    let text = FormattedText::new([FormattedTextLine::Line(vec![
        FormattedTextFragment::plain_text("hello world"),
    ])]);
    let wrapper = FormattedTextWrapper::from(text);
    let arc1 = wrapper.formatted_text_arc();
    let arc2 = wrapper.formatted_text_arc();
    // Both calls must return the same allocation — not independent deep copies.
    assert!(Arc::ptr_eq(&arc1, &arc2));
}

#[test]
fn formatted_text_wrapper_preserves_content() {
    let text = FormattedText::new([
        FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("line one")]),
        FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("line two")]),
    ]);
    let wrapper = FormattedTextWrapper::from(text);
    // lines() metadata matches the cached Arc
    assert_eq!(wrapper.lines().len(), 2);
    assert_eq!(wrapper.lines()[0].raw_text(), "line one\n");
    assert_eq!(wrapper.lines()[1].raw_text(), "line two\n");
    // Arc contains the same lines
    let ft = wrapper.formatted_text_arc();
    assert_eq!(ft.lines.len(), 2);
}

#[test]
fn test_convert_files() {
    let a = FileContext::new(
        "a.txt".to_string(),
        AnyFileContent::StringContent("hey\nyou".to_string()),
        None,
        None,
    );

    assert_eq!(
        Into::<Vec<FileContent>>::into(a),
        vec![FileContent {
            file_path: "a.txt".to_string(),
            content: "hey\nyou".to_string(),
            line_range: None,
        }]
    );
}

#[test]
fn test_convert_files_range() {
    // Content is pre-sliced to match the line range.
    let a = FileContext::new(
        "a.txt".to_string(),
        AnyFileContent::StringContent("hey\nyou".to_string()),
        Some(1..2),
        None,
    );

    assert_eq!(
        Into::<Vec<FileContent>>::into(a),
        vec![FileContent {
            file_path: "a.txt".to_string(),
            content: "hey\nyou".to_string(),
            line_range: to_range(1..2),
        }]
    );
}

#[test]
fn test_convert_files_range_out_of_bounds() {
    // Even with an out-of-bounds range, content is passed through as-is.
    let a = FileContext::new(
        "a.txt".to_string(),
        AnyFileContent::StringContent(String::new()),
        Some(10..20),
        None,
    );

    assert_eq!(
        Into::<Vec<FileContent>>::into(a),
        vec![FileContent {
            file_path: "a.txt".to_string(),
            content: String::new(),
            line_range: to_range(10..20),
        }]
    );
}

#[test]
fn test_programming_language_from_string() {
    // Shell language specifiers should produce Shell variants
    assert_eq!(
        ProgrammingLanguage::from("bash".to_string()),
        ProgrammingLanguage::Shell(ShellType::Bash)
    );
    assert_eq!(
        ProgrammingLanguage::from("shell".to_string()),
        ProgrammingLanguage::Shell(ShellType::Bash)
    );
    assert_eq!(
        ProgrammingLanguage::from("sh".to_string()),
        ProgrammingLanguage::Shell(ShellType::Bash)
    );
    assert_eq!(
        ProgrammingLanguage::from("zsh".to_string()),
        ProgrammingLanguage::Shell(ShellType::Zsh)
    );
    assert_eq!(
        ProgrammingLanguage::from("fish".to_string()),
        ProgrammingLanguage::Shell(ShellType::Fish)
    );
    assert_eq!(
        ProgrammingLanguage::from("powershell".to_string()),
        ProgrammingLanguage::Shell(ShellType::PowerShell)
    );
    assert_eq!(
        ProgrammingLanguage::from("pwsh".to_string()),
        ProgrammingLanguage::Shell(ShellType::PowerShell)
    );

    // Non-shell languages should produce Other variants
    assert_eq!(
        ProgrammingLanguage::from("python".to_string()),
        ProgrammingLanguage::Other("python".to_string())
    );
    assert_eq!(
        ProgrammingLanguage::from("rust".to_string()),
        ProgrammingLanguage::Other("rust".to_string())
    );
    assert_eq!(
        ProgrammingLanguage::from("javascript".to_string()),
        ProgrammingLanguage::Other("javascript".to_string())
    );
}

#[test]
fn format_for_copy_preserves_visual_markdown_sections() {
    let output = AIAgentOutput {
        messages: vec![AIAgentOutputMessage {
            id: MessageId::new("message-1".to_string()),
            message: AIAgentOutputMessageType::Text(AIAgentText {
                sections: vec![
                    AIAgentTextSection::PlainText {
                        text: "Intro".to_string().into(),
                    },
                    AIAgentTextSection::Image {
                        image: AgentOutputImage {
                            alt_text: "Diagram".to_string(),
                            source: "./diagram.png".to_string(),
                            title: None,
                            markdown_source: "![Diagram](./diagram.png)".to_string(),
                            layout: AgentOutputImageLayout::Block,
                        },
                    },
                    AIAgentTextSection::MermaidDiagram {
                        diagram: AgentOutputMermaidDiagram {
                            source: "graph TD\nA --> B".to_string(),
                            markdown_source: "```mermaid\ngraph TD\nA --> B\n```".to_string(),
                        },
                    },
                ],
            }),
            citations: Vec::new(),
        }],
        ..Default::default()
    };

    assert_eq!(
        output.format_for_copy(None),
        "Intro\n![Diagram](./diagram.png)\n```mermaid\ngraph TD\nA --> B\n```"
    );
}
