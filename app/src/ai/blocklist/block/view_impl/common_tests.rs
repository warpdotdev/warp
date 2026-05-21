use itertools::Itertools;
use std::{collections::HashMap, path::PathBuf, sync::Arc};

use ai::skills::{ParsedSkill, SkillProvider, SkillScope};
#[cfg(feature = "local_fs")]
use warpui::assets::asset_cache::AssetSource;
use warpui::App;

#[cfg(feature = "local_fs")]
use super::{blocklist_image_asset_source, ResolvedBlocklistImageSources};
use super::{
    collect_visual_markdown_lightbox_collection, compute_visual_section_width,
    inline_image_source_label, is_supported_blocklist_image_source, lightbox_trigger_for_section,
    query_prefix_highlight_len, render_scrollable_collapsible_content, text_sections_with_indices,
    CollapsibleElementState, CollapsibleExpansionState, VisualMarkdownLightboxCollection,
};
use crate::{
    ai::agent::{
        AIAgentInput, AIAgentTextSection, AgentOutputImage, AgentOutputImageLayout,
        AgentOutputMermaidDiagram, MessageId, UserQueryMode,
    },
    features::FeatureFlag,
    search::slash_command_menu::static_commands::commands,
};
use ui_components::lightbox::{LightboxImage, LightboxImageSource};
use warpui::{elements::Empty, Element};

#[test]
fn query_prefix_highlight_len_highlights_invoke_skill_inputs() {
    let input = AIAgentInput::InvokeSkill {
        context: Arc::new([]),
        skill: ParsedSkill {
            path: PathBuf::from("/tmp/.agents/skills/review-pr/SKILL.md"),
            name: "review-pr".to_string(),
            description: "Review a pull request.".to_string(),
            content: String::new(),
            line_range: None,
            provider: SkillProvider::Agents,
            scope: SkillScope::Project,
        },
        user_query: None,
    };

    assert_eq!(
        query_prefix_highlight_len(&input, "/review-pr tighten the summary"),
        Some("/review-pr".len())
    );
}

#[test]
fn query_prefix_highlight_len_does_not_guess_from_plain_user_query_text() {
    let input = AIAgentInput::UserQuery {
        query: "/review-pr tighten the summary".to_string(),
        context: Arc::new([]),
        static_query_type: None,
        referenced_attachments: HashMap::new(),
        user_query_mode: UserQueryMode::Normal,
        running_command: None,
        intended_agent: None,
    };

    assert_eq!(
        query_prefix_highlight_len(&input, "/review-pr tighten the summary"),
        None
    );
}

#[test]
fn query_prefix_highlight_len_keeps_existing_plan_highlighting() {
    let input = AIAgentInput::UserQuery {
        query: "write tests".to_string(),
        context: Arc::new([]),
        static_query_type: None,
        referenced_attachments: HashMap::new(),
        user_query_mode: UserQueryMode::Plan,
        running_command: None,
        intended_agent: None,
    };

    assert_eq!(
        query_prefix_highlight_len(&input, "/plan write tests"),
        Some(commands::PLAN.name.len())
    );
}

#[test]
fn text_sections_with_indices_preserve_image_section_alignment_after_empty_text_sections() {
    let sections = vec![
        AIAgentTextSection::PlainText {
            text: "".to_string().into(),
        },
        AIAgentTextSection::PlainText {
            text: "Before".to_string().into(),
        },
        AIAgentTextSection::Image {
            image: AgentOutputImage {
                alt_text: "One".to_string(),
                source: "one.png".to_string(),
                title: None,
                markdown_source: "![One](one.png)".to_string(),
                layout: AgentOutputImageLayout::Block,
            },
        },
        AIAgentTextSection::PlainText {
            text: "   ".to_string().into(),
        },
        AIAgentTextSection::Image {
            image: AgentOutputImage {
                alt_text: "Two".to_string(),
                source: "two.png".to_string(),
                title: None,
                markdown_source: "![Two](two.png)".to_string(),
                layout: AgentOutputImageLayout::Block,
            },
        },
    ];

    let rendered_image_indices = text_sections_with_indices(&sections, 0)
        .filter_map(|(section_index, section)| match section {
            AIAgentTextSection::PlainText { text } if text.text().trim().is_empty() => None,
            AIAgentTextSection::Image { .. } => Some(section_index),
            _ => None,
        })
        .collect_vec();

    assert_eq!(rendered_image_indices, vec![2, 4]);
}

#[test]
fn render_scrollable_collapsible_content_returns_none_when_collapsed() {
    let state = CollapsibleElementState {
        expansion_state: CollapsibleExpansionState::Collapsed,
        ..Default::default()
    };
    let message_id = MessageId::new("message-1".to_string());

    let content = render_scrollable_collapsible_content(
        &message_id,
        &state,
        Empty::new().finish(),
        false,
        200.,
    );

    assert!(
        content.is_none(),
        "Expected no rendered content when collapsible state is collapsed",
    );
}

#[test]
fn compute_visual_section_width_rejects_non_finite_dimensions() {
    assert_eq!(compute_visual_section_width(f32::INFINITY, 20., 40.), None);
    assert_eq!(compute_visual_section_width(20., f32::NAN, 40.), None);
    assert_eq!(compute_visual_section_width(20., 40., f32::INFINITY), None);
    assert_eq!(compute_visual_section_width(20., 40., 10.), Some(5.));
}

#[test]
fn render_scrollable_collapsible_content_returns_body_when_expanded() {
    let message_id = MessageId::new("message-2".to_string());
    let state = CollapsibleElementState::default();
    let content = render_scrollable_collapsible_content(
        &message_id,
        &state,
        Empty::new().finish(),
        false,
        200.,
    );

    assert!(
        content.is_some(),
        "Expected rendered content when collapsible state is expanded",
    );
}

#[test]
fn lightbox_trigger_uses_source_order_index_for_clicked_visual() {
    let collection = VisualMarkdownLightboxCollection {
        section_indices: vec![2, 4, 7],
        images: Arc::new(vec![
            LightboxImage {
                source: LightboxImageSource::Loading,
                description: Some("one".to_string()),
            },
            LightboxImage {
                source: LightboxImageSource::Loading,
                description: Some("two".to_string()),
            },
            LightboxImage {
                source: LightboxImageSource::Loading,
                description: Some("three".to_string()),
            },
        ]),
    };

    let trigger = lightbox_trigger_for_section(&collection, 4)
        .expect("Expected lightbox trigger for section present in collection");

    assert_eq!(trigger.initial_index, 1);
    assert_eq!(trigger.images.len(), 3);
}

#[test]
fn lightbox_trigger_returns_none_for_unknown_section() {
    let collection = VisualMarkdownLightboxCollection {
        section_indices: vec![1],
        images: Arc::new(vec![LightboxImage {
            source: LightboxImageSource::Loading,
            description: None,
        }]),
    };

    assert!(lightbox_trigger_for_section(&collection, 3).is_none());
}
#[test]
fn collect_visual_markdown_lightbox_collection_includes_mermaid_sections_in_source_order() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let _blocklist_markdown_images =
                FeatureFlag::BlocklistMarkdownImages.override_enabled(true);
            let _markdown_mermaid = FeatureFlag::MarkdownMermaid.override_enabled(true);

            let sections = vec![
                AIAgentTextSection::PlainText {
                    text: "before".to_string().into(),
                },
                AIAgentTextSection::MermaidDiagram {
                    diagram: AgentOutputMermaidDiagram {
                        source: "graph TD\nA-->B".to_string(),
                        markdown_source: "```mermaid\ngraph TD\nA-->B\n```".to_string(),
                    },
                },
                AIAgentTextSection::PlainText {
                    text: "between".to_string().into(),
                },
                AIAgentTextSection::MermaidDiagram {
                    diagram: AgentOutputMermaidDiagram {
                        source: "graph TD\nB-->C".to_string(),
                        markdown_source: "```mermaid\ngraph TD\nB-->C\n```".to_string(),
                    },
                },
            ];

            let indexed_sections = text_sections_with_indices(&sections, 10).collect_vec();
            let collection = collect_visual_markdown_lightbox_collection(
                &indexed_sections,
                None,
                #[cfg(feature = "local_fs")]
                None,
                ctx,
            );

            assert_eq!(collection.section_indices, vec![11, 13]);
            assert_eq!(collection.images.len(), 2);
            assert!(collection
                .images
                .iter()
                .all(|image| image.description.is_none()));
        });
    });
}

#[test]
fn inline_image_source_label_uses_file_name() {
    assert_eq!(
        inline_image_source_label("/tmp/screenshots/classic_1.png"),
        "classic_1.png"
    );
}

#[cfg(feature = "local_fs")]
#[test]
fn blocklist_image_asset_source_uses_cached_resolution_when_available() {
    let current_working_directory = "/tmp/session".to_string();
    let cached_path = "/tmp/cached/diagram.png".to_string();
    let resolved_sources = ResolvedBlocklistImageSources::from([(
        "diagram.png".to_string(),
        Some(AssetSource::LocalFile {
            path: cached_path.clone(),
        }),
    )]);

    let resolved = blocklist_image_asset_source(
        "diagram.png",
        Some(&current_working_directory),
        Some(&resolved_sources),
    );

    match resolved {
        Some(AssetSource::LocalFile { path }) => assert_eq!(path, cached_path),
        other => panic!("expected cached local file asset source, got {other:?}"),
    }
}

/// `is_supported_blocklist_image_source` should accept the same image extensions
/// that `warp_util::file_type::is_binary_file` recognises (plus `svg`, which is
/// text/XML and not in `is_binary_file`). Until #9395 / this fix landed the
/// blocklist list was only `jpg | jpeg | png | gif | webp | svg`, so inline
/// references to local `.bmp` / `.tiff` / `.tif` / `.ico` images failed the
/// support check and silently rendered as plain text.
#[test]
fn is_supported_blocklist_image_source_covers_common_local_formats() {
    for source in [
        "diagram.jpg",
        "diagram.jpeg",
        "diagram.png",
        "diagram.gif",
        "diagram.bmp",
        "diagram.tiff",
        "diagram.tif",
        "diagram.webp",
        "diagram.ico",
        "diagram.svg",
    ] {
        assert!(
            is_supported_blocklist_image_source(source),
            "{source} should be a supported local image source"
        );
    }
    // Case-insensitive on the extension.
    assert!(is_supported_blocklist_image_source("PHOTO.PNG"));
    assert!(is_supported_blocklist_image_source("scan.TIFF"));
    // HTTP / HTTPS sources are intentionally rejected regardless of extension.
    assert!(!is_supported_blocklist_image_source(
        "http://example.com/x.png"
    ));
    assert!(!is_supported_blocklist_image_source(
        "https://example.com/x.png"
    ));
    // Non-image extensions stay rejected.
    assert!(!is_supported_blocklist_image_source("doc.pdf"));
    assert!(!is_supported_blocklist_image_source("notes.md"));
}
