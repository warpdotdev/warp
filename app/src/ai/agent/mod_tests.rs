use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use anyhow::anyhow;
use chrono::{Local, TimeZone, Utc};
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use warp_editor::render::model::LineCount;
use warp_multi_agent_api::{FileContent, FileContentLineRange};

use crate::ai::agent::{
    AIAgentAttachment, AIAgentContext, AIAgentOutput, AIAgentOutputMessage,
    AIAgentOutputMessageType, AIAgentText, AIAgentTextSection, AgentOutputImage,
    AgentOutputImageLayout, AgentOutputMermaidDiagram, AnyFileContent, CurrentHead, DiffBase,
    DiffSetHunk, DocumentContentAttachmentSource, DriveObjectPayload, FileContext,
    FormattedTextWrapper, ImageContext, MessageId, ProgrammingLanguage, RenderableAIError,
    TransientNetworkErrorKind,
};
use crate::ai::block_context::BlockContext;
use crate::ai_assistant::execution_context::{WarpAiExecutionContext, WarpAiOsContext};
use crate::server::server_api::AIApiError;
use crate::terminal::shell::ShellType;

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

fn deserialize_pull_request_number_from_json(number_json: &str) -> serde_json::Result<i32> {
    let context = serde_json::from_str::<AIAgentContext>(&format!(
        r#"{{"PullRequest":{{"number":{number_json}}}}}"#
    ))?;
    match context {
        AIAgentContext::PullRequest { number, .. } => Ok(number),
        other => panic!("expected pull request context, got {other:?}"),
    }
}

#[test]
fn pull_request_number_deserializer_accepts_positive_number_and_string() {
    assert_eq!(deserialize_pull_request_number_from_json("42").unwrap(), 42);
    assert_eq!(
        deserialize_pull_request_number_from_json(r#""42""#).unwrap(),
        42
    );
}

#[test]
fn pull_request_number_deserializer_defaults_invalid_numbers() {
    for number_json in ["null", "0", "-1", "1.5", "2147483648", r#""""#, r#""abc""#] {
        assert_eq!(
            deserialize_pull_request_number_from_json(number_json).unwrap(),
            0,
            "expected {number_json} to deserialize to default pull request number",
        );
    }
}

#[test]
fn pull_request_number_deserializer_rejects_unsupported_json_types() {
    for number_json in ["true", "[]", "{}"] {
        assert!(
            deserialize_pull_request_number_from_json(number_json).is_err(),
            "expected {number_json} to fail deserialization",
        );
    }
}

#[test]
fn transient_network_error_includes_user_facing_message_and_debug_details() {
    let error = RenderableAIError::transient_network_error(
        false,
        false,
        TransientNetworkErrorKind::Api(Arc::new(AIApiError::Other(anyhow!("connection reset")))),
    );

    let rendered = error.to_string();
    assert!(
        rendered.starts_with(
            "Warp lost connection while receiving the agent response. This is usually temporary.\n\nDebug info: "
        ),
        "unexpected rendering: {rendered}"
    );
    // The raw underlying API error must survive into the debug section.
    assert!(
        rendered.contains("connection reset"),
        "raw error detail should surface in debug info: {rendered}"
    );
    assert!(!error.will_attempt_resume());
}

#[test]
fn transient_network_error_reports_pending_resume() {
    let error = RenderableAIError::transient_network_error(
        true,
        false,
        TransientNetworkErrorKind::Api(Arc::new(AIApiError::Other(anyhow!("connection reset")))),
    );

    assert!(error.will_attempt_resume());
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
fn test_programming_language_to_extension() {
    // Each entry is (markdown language token, expected extension). The expected extension
    // must resolve back to a recognized language via `languages::language_by_filename` so that
    // syntax highlighting is applied to the AI block.
    let cases: &[(&str, &str)] = &[
        // Canonical names.
        ("rust", "rs"),
        ("go", "go"),
        ("python", "py"),
        ("javascript", "js"),
        ("typescript", "ts"),
        ("yaml", "yaml"),
        ("cpp", "cpp"),
        ("java", "java"),
        ("c#", "cs"),
        ("csharp", "cs"),
        ("html", "html"),
        ("css", "css"),
        ("c", "c"),
        ("json", "json"),
        ("hcl", "hcl"),
        ("lua", "lua"),
        ("ruby", "rb"),
        ("php", "php"),
        ("toml", "toml"),
        ("swift", "swift"),
        ("kotlin", "kt"),
        ("powershell", "ps1"),
        ("elixir", "exs"),
        ("scala", "scala"),
        ("sql", "sql"),
        // Languages newly covered by this fix — previously fell through to None and rendered
        // without syntax highlighting in AI blocks even though the `languages` crate supports them.
        ("jsx", "jsx"),
        ("tsx", "tsx"),
        ("xml", "xml"),
        ("vue", "vue"),
        ("dockerfile", "dockerfile"),
        ("starlark", "bzl"),
        ("objective-c", "m"),
        ("objc", "m"),
        // Common markdown code-fence aliases.
        ("rs", "rs"),
        ("golang", "go"),
        ("py", "py"),
        ("js", "js"),
        ("ts", "ts"),
        ("yml", "yaml"),
        ("c++", "cpp"),
        ("rb", "rb"),
        ("kt", "kt"),
        ("terraform", "hcl"),
        ("tf", "hcl"),
        ("docker", "dockerfile"),
        ("containerfile", "dockerfile"),
        ("markdown", "md"),
        ("md", "md"),
    ];
    for (token, expected_extension) in cases {
        let language = ProgrammingLanguage::from((*token).to_string());
        assert_eq!(
            language.to_extension(),
            Some(*expected_extension),
            "expected to_extension({token:?}) to be Some({expected_extension:?})",
        );
    }

    // PowerShell remains the only Shell variant whose extension is exposed; this preserves
    // existing behavior for the other Shell variants which are intentionally not extended here.
    assert_eq!(
        ProgrammingLanguage::Shell(ShellType::PowerShell).to_extension(),
        Some("ps1"),
    );

    // Unrecognized tokens still return None.
    assert_eq!(
        ProgrammingLanguage::Other("definitely-not-a-language".to_string()).to_extension(),
        None,
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

fn sample_block_context() -> BlockContext {
    BlockContext {
        id: "block-1".to_string().into(),
        index: 0.into(),
        command: "ls".to_string(),
        output: "file.txt".to_string(),
        exit_code: 0.into(),
        is_auto_attached: false,
        started_ts: None,
        finished_ts: None,
        pwd: Some("/tmp".to_string()),
        shell: None,
        username: None,
        hostname: None,
        git_branch: None,
        os: None,
        session_id: None,
    }
}

#[test]
fn ai_agent_context_round_trips_tagged_variants() {
    let contexts = vec![
        AIAgentContext::Directory {
            pwd: Some("/tmp/project".to_string()),
            home_dir: Some("/Users/me".to_string()),
            are_file_symbols_indexed: true,
        },
        AIAgentContext::SelectedText("selected text".to_string()),
        AIAgentContext::ExecutionEnvironment(WarpAiExecutionContext {
            os: WarpAiOsContext {
                category: Some("MacOS".to_string()),
                distribution: None,
            },
            shell_name: "zsh".to_string(),
            shell_version: Some("5.9".to_string()),
        }),
        AIAgentContext::CurrentTime {
            current_time: Utc
                .with_ymd_and_hms(2024, 1, 15, 10, 30, 0)
                .unwrap()
                .with_timezone(&Local),
        },
        AIAgentContext::Image(ImageContext {
            data: "aGVsbG8=".to_string(),
            mime_type: "image/png".to_string(),
            file_name: "shot.png".to_string(),
            is_figma: false,
        }),
        AIAgentContext::Codebase {
            path: "/tmp/project".to_string(),
            name: "project".to_string(),
        },
        AIAgentContext::ProjectRules {
            root_path: "/tmp/project".to_string(),
            active_rules: vec![FileContext::new(
                "WARP.md".to_string(),
                AnyFileContent::StringContent("Be nice.".to_string()),
                None,
                None,
            )],
            additional_rule_paths: vec!["sub/WARP.md".to_string()],
        },
        AIAgentContext::File(FileContext::new(
            "a.txt".to_string(),
            AnyFileContent::StringContent("hey\nyou".to_string()),
            None,
            None,
        )),
        AIAgentContext::Git {
            head: "abc1234".to_string(),
            branch: Some("main".to_string()),
        },
        AIAgentContext::Repository {
            name: "warp".to_string(),
            owner: Some("warpdotdev".to_string()),
            host: Some("github.com".to_string()),
        },
        AIAgentContext::PullRequest {
            number: 42,
            state: "OPEN".to_string(),
            draft: true,
            base_branch: "main".to_string(),
            url: "https://github.com/warpdotdev/warp/pull/42".to_string(),
        },
        AIAgentContext::Skills { skills: vec![] },
    ];
    for context in contexts {
        let json = serde_json::to_value(&context).unwrap();
        let deserialized: AIAgentContext = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, context);
    }
}

#[test]
fn ai_agent_context_round_trips_untagged_block_variant() {
    let context = AIAgentContext::Block(Box::new(sample_block_context()));
    let json = serde_json::to_value(&context).unwrap();
    // The Block variant must serialize untagged, as a bare object.
    assert!(
        json.get("block_id").is_some(),
        "expected untagged block object, got {json}"
    );
    let deserialized: AIAgentContext = serde_json::from_value(json).unwrap();
    assert_eq!(deserialized, context);
}

#[test]
fn ai_agent_context_rejects_unknown_variants() {
    let result = serde_json::from_str::<AIAgentContext>(r#"{"NotARealVariant":{}}"#);
    assert!(result.is_err());
}

#[test]
fn ai_agent_attachment_round_trips_tagged_variants() {
    let attachments = vec![
        AIAgentAttachment::PlainText("hello".to_string()),
        AIAgentAttachment::DocumentContent {
            document_id: "doc-1".to_string(),
            content: "# Plan".to_string(),
            source: DocumentContentAttachmentSource::UserAttached,
            line_range: Some(LineCount::range(1..5)),
        },
        AIAgentAttachment::DriveObject {
            uid: "drive-1".to_string(),
            payload: Some(DriveObjectPayload::Workflow {
                name: "deploy".to_string(),
                description: "Deploy the app".to_string(),
                command: "make deploy".to_string(),
            }),
        },
        AIAgentAttachment::DiffHunk {
            file_path: "src/main.rs".to_string(),
            line_range: LineCount::range(1..3),
            diff_content: "+fn main() {}".to_string(),
            lines_added: 1,
            lines_removed: 0,
            current: Some(CurrentHead::BranchName("feature".to_string())),
            base: DiffBase::BranchName("main".to_string()),
        },
        AIAgentAttachment::DiffSet {
            file_diffs: HashMap::from([(
                "src/main.rs".to_string(),
                vec![DiffSetHunk {
                    line_range: LineCount::range(1..3),
                    diff_content: "+use std::fmt;".to_string(),
                    lines_added: 1,
                    lines_removed: 0,
                }],
            )]),
            current: None,
            base: DiffBase::UncommittedChanges,
        },
        AIAgentAttachment::FilePathReference {
            file_id: "file-1".to_string(),
            file_name: "report.txt".to_string(),
            file_path: "/tmp/report.txt".to_string(),
        },
    ];
    for attachment in attachments {
        let json = serde_json::to_value(&attachment).unwrap();
        let deserialized: AIAgentAttachment = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized, attachment);
    }
}

#[test]
fn ai_agent_attachment_round_trips_untagged_block_variant() {
    let attachment = AIAgentAttachment::Block(sample_block_context());
    let json = serde_json::to_value(&attachment).unwrap();
    // The Block variant must serialize untagged, as a bare object.
    assert!(
        json.get("block_id").is_some(),
        "expected untagged block object, got {json}"
    );
    let deserialized: AIAgentAttachment = serde_json::from_value(json).unwrap();
    assert_eq!(deserialized, attachment);
}

#[path = "suggestions_tests.rs"]
mod suggestions;
