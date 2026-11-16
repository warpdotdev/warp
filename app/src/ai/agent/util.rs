use std::collections::HashMap;
use std::path::PathBuf;

use ai::gfm_table::{format_gfm_table, maybe_collect_gfm_table_lines};
use itertools::Itertools;
use lazy_static::lazy_static;
use markdown_parser::{
    FormattedImage, FormattedTextLine, parse_image_run_line, parse_markdown_with_gfm_tables,
};
use mermaid_to_svg::is_mermaid_diagram;
use regex::Regex;
use warp_util::path::LineAndColumnArg;

use super::{
    AIAgentTextSection, AgentOutputImage, AgentOutputImageLayout, AgentOutputMermaidDiagram,
    AgentOutputTable, ProgrammingLanguage,
};
use crate::code::editor_management::CodeSource;
use crate::features::FeatureFlag;

lazy_static! {
    /// Markdown prefix for code blocks. Matches on triple backticks followed by a language.
    /// Importantly, parameters for linked code blocks are captured into their own group.
    static ref CODE_START_REGEX: Regex = Regex::new(r"^\s*```([\w+-]*)(.*)$").expect("Regex is valid");

    /// Markdown suffix for code blocks.
    static ref CODE_END_REGEX: Regex = Regex::new(r"^\s*```\s*$").expect("Regex is valid");

    /// Extracts key-value parameters from text in the format: key=value, used for code block metadata.
    /// Expects to match on text with format path=/path/to/file start=<line_number>
    static ref CODE_PARAMS_REGEX: Regex = Regex::new(r"(\w+)=([^\s]+)").expect("Regex is valid");
}

/// Converts the given `markdown_text` into corresponding `Text` and `Code` `AIAgentOutputStep`s.
pub(super) fn parse_markdown_into_text_and_code_sections(
    markdown_text: &str,
) -> Vec<AIAgentTextSection> {
    let mut sections = vec![];
    let mut current_section = CurrentSection::PlainText(String::new());

    let mut lines = markdown_text.lines().peekable();
    while let Some(line) = lines.next() {
        match &mut current_section {
            CurrentSection::PlainText(text) => {
                // Detect tables and render them as formatted table sections.
                if let Some(table_lines) = maybe_collect_gfm_table_lines(line, &mut lines, |l| {
                    CODE_START_REGEX.is_match(l)
                }) {
                    let markdown_source = table_lines.join("\n");
                    let table_section = if FeatureFlag::BlocklistMarkdownTableRendering.is_enabled()
                    {
                        parse_agent_output_table(&markdown_source)
                    } else {
                        Some(AgentOutputTable::legacy(format_gfm_table(&table_lines)))
                    };
                    if let Some(table_section) = table_section {
                        if !text.is_empty() {
                            flush_plain_text_sections(text, &mut sections);
                            text.clear();
                        }
                        sections.push(AIAgentTextSection::Table {
                            table: table_section,
                        });
                        continue;
                    }

                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(&markdown_source);
                    continue;
                }

                if let Some((_, [language, param_str])) = CODE_START_REGEX
                    .captures(line)
                    .map(|capture_group| capture_group.extract())
                {
                    if !text.is_empty() {
                        flush_plain_text_sections(text, &mut sections);
                    }

                    let source = {
                        let mut params = HashMap::new();
                        for (_, [key, value]) in CODE_PARAMS_REGEX
                            .captures_iter(param_str)
                            .map(|c| c.extract())
                        {
                            params.insert(key, value);
                        }
                        match (params.get("path"), params.get("start")) {
                            (Some(path), Some(start)) => {
                                start
                                    .parse::<usize>()
                                    .ok()
                                    .map(|line_num| CodeSource::Link {
                                        path: PathBuf::from(path),
                                        range_start: Some(LineAndColumnArg {
                                            line_num,
                                            column_num: None,
                                        }),
                                        range_end: None,
                                    })
                            }
                            _ => None,
                        }
                    };
                    current_section = CurrentSection::Code {
                        code: String::new(),
                        language_token: Some(language.to_owned()).filter(|l| !l.is_empty()),
                        language: Some(language)
                            .filter(|l| !l.is_empty())
                            .map(|l| l.to_owned().into()),
                        source,
                    };
                } else {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(line);
                }
            }
            CurrentSection::Code {
                code,
                language,
                language_token,
                source,
            } => {
                if CODE_END_REGEX.is_match(line) {
                    if !code.is_empty() {
                        if let Some(CodeSource::Link {
                            range_start: Some(start),
                            range_end,
                            ..
                        }) = source.as_mut()
                        {
                            *range_end = Some(LineAndColumnArg {
                                line_num: start.line_num + code.lines().count() - 1,
                                column_num: None,
                            });
                        }
                        push_code_or_mermaid_section(
                            std::mem::take(code),
                            language.clone(),
                            language_token.as_deref(),
                            source.take(),
                            &mut sections,
                        );
                    }
                    current_section = CurrentSection::PlainText(String::new());
                } else {
                    if !code.is_empty() {
                        code.push('\n');
                    }
                    code.push_str(line);
                }
            }
        }
    }

    match current_section {
        CurrentSection::PlainText(text) => {
            flush_plain_text_sections(&text, &mut sections);
        }
        CurrentSection::Code {
            code,
            language,
            language_token,
            source,
        } => {
            push_code_or_mermaid_section(
                code,
                language,
                language_token.as_deref(),
                source,
                &mut sections,
            );
        }
    }

    if sections.is_empty() {
        sections.push(AIAgentTextSection::PlainText {
            text: String::new().into(),
        });
    }
    sections
}

fn parse_agent_output_table(markdown_source: &str) -> Option<AgentOutputTable> {
    let formatted_text = parse_markdown_with_gfm_tables(markdown_source).ok()?;
    let table = formatted_text
        .lines
        .into_iter()
        .exactly_one()
        .ok()
        .and_then(|line| match line {
            FormattedTextLine::Table(table) => Some(table),
            _ => None,
        })?;
    Some(AgentOutputTable::structured(
        markdown_source.to_owned(),
        table,
    ))
}

enum CurrentSection {
    PlainText(String),
    Code {
        code: String,
        language_token: Option<String>,
        language: Option<ProgrammingLanguage>,
        source: Option<CodeSource>,
    },
}

fn flush_plain_text_sections(markdown_text: &str, sections: &mut Vec<AIAgentTextSection>) {
    if markdown_text.is_empty() {
        return;
    }

    let mut plain_text = String::new();
    for line in markdown_text.split_inclusive('\n') {
        if let Some(images) = parse_image_run_line(line) {
            if !plain_text.is_empty() {
                sections.push(AIAgentTextSection::PlainText {
                    text: std::mem::take(&mut plain_text).into(),
                });
            }

            if images.len() == 1 {
                if let Some(image) = images.into_iter().next() {
                    sections.push(image_section(image, AgentOutputImageLayout::Block));
                }
            } else {
                sections.extend(
                    images
                        .into_iter()
                        .map(|image| image_section(image, AgentOutputImageLayout::Inline)),
                );
            }
        } else {
            plain_text.push_str(line);
        }
    }

    if !plain_text.is_empty() {
        sections.push(AIAgentTextSection::PlainText {
            text: plain_text.into(),
        });
    }
}
fn image_section(image: FormattedImage, layout: AgentOutputImageLayout) -> AIAgentTextSection {
    AIAgentTextSection::Image {
        image: AgentOutputImage {
            markdown_source: markdown_source_for_image(&image),
            alt_text: image.alt_text,
            source: image.source,
            title: image.title,
            layout,
        },
    }
}

fn markdown_source_for_image(image: &FormattedImage) -> String {
    warp_editor::content::text::format_image_markdown(
        &image.alt_text,
        &image.source,
        image.title.as_deref(),
    )
}

fn markdown_source_for_mermaid(source: &str) -> String {
    format!("```mermaid\n{source}\n```")
}

fn push_code_or_mermaid_section(
    code: String,
    language: Option<ProgrammingLanguage>,
    language_token: Option<&str>,
    source: Option<CodeSource>,
    sections: &mut Vec<AIAgentTextSection>,
) {
    if code.is_empty() {
        return;
    }

    if language_token.is_some_and(is_mermaid_diagram) {
        sections.push(AIAgentTextSection::MermaidDiagram {
            diagram: AgentOutputMermaidDiagram {
                markdown_source: markdown_source_for_mermaid(&code),
                source: code,
            },
        });
    } else {
        sections.push(AIAgentTextSection::Code {
            code,
            language,
            source,
        });
    }
}

#[cfg(test)]
#[path = "util_tests.rs"]
mod tests;
