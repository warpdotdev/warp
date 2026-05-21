use anyhow::Result;
use serde_yaml::{Mapping, Value};
use std::{
    cell::Cell,
    collections::{HashMap, VecDeque},
    rc::Rc,
};

use html5ever::{
    Attribute, ParseOpts, parse_document, tendril::TendrilSink, tree_builder::TreeBuilderOpts,
};
use markup5ever_rcdom::{Node, NodeData, RcDom};

use crate::{
    CodeBlockText, FormattedIndentTextInline, FormattedTaskList, FormattedText,
    FormattedTextFragment, FormattedTextHeader, FormattedTextInline, FormattedTextLine,
    FormattedTextStyles, Hyperlink, OrderedFormattedIndentTextInline,
    markdown_parser::RUNNABLE_BLOCK_MARKDOWN_LANG, weight::CustomWeight,
};

// Top element element tags we are not parsing for right now.
// Note that we have "<b>" here because GDocs always include a top level <b> element to add additional
// GDocs specific meta-data for its rich text content.
const TOP_LEVEL_ELEMENT_TAGS_TO_SKIP: &[&str] = &[
    "head", "body", "html", "meta", "table", "b", "div", "ul", "ol", "li", "input",
];
const PHRASING_ELEMENT_TAGS: &[&str] = &[
    "span", "i", "code", "strong", "em", "br", "a", "s", "u", "ins",
];

pub const WARP_EMBED_ATTRIBUTE_NAME: &str = "data-warp-embedded-item";

#[derive(Clone, Debug, PartialEq, Eq)]
struct ListArg {
    indent_level: usize,
    item_type: ListType,
    start_number: Rc<Cell<Option<usize>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ListType {
    Checkbox(bool),
    ListItem { ordered: bool },
}

#[derive(Clone, Default)]
struct Styling {
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    inline_code: bool,
    link: Option<String>,
}

impl Styling {
    fn update_with_attributes(&mut self, attributes: &[Attribute]) {
        for attribute in attributes {
            let attribute_name = attribute.name.local.to_string();

            if attribute_name == "style" {
                let attribute_value = attribute.value.to_string();
                let style_dict = parse_style_into_dict(attribute_value.as_str());

                if let Some(font_style) = style_dict.get("font-style") {
                    self.italic = *font_style == "italic";
                }

                if let Some(text_decoration) = style_dict.get("text-decoration") {
                    // `text-decoration` can be used for multiple things. If we are missing line-through
                    // here, don't unset the strikethrough.
                    if *text_decoration == "line-through" {
                        self.strikethrough = true;
                    }
                    if *text_decoration == "underline" {
                        self.underline = true;
                    }
                }

                if let Some(font_weight) = style_dict.get("font-weight") {
                    if *font_weight == "bold" || *font_weight == "bolder" {
                        self.bold = true;
                    } else {
                        let maybe_integer = font_weight.parse::<i32>();

                        if let Ok(weight_value) = maybe_integer {
                            self.bold = weight_value > 400;
                        }
                    }
                }

                // Note that there is not a definitive way in HTML to represent inline code.
                // Different text editors all use different ways to represent inline code and are
                // not always compatible to each other, here we chose Notion's way of representing
                // inline code which is a non-transparent background. We could revisit this in the future.
                if let Some(color) = style_dict.get("background") {
                    self.inline_code = *color != "transparent";
                }
            } else if attribute_name == "href" {
                let attribute_value = attribute.value.to_string();
                self.link = Some(attribute_value);
            }
        }
    }
}

/// Find an attribute by name, if it's present.
fn get_attribute<'a>(attributes: &'a [Attribute], name: &str) -> Option<&'a str> {
    attributes.iter().find_map(|attribute| {
        if &attribute.name.local == name {
            Some(attribute.value.as_ref())
        } else {
            None
        }
    })
}

fn includes_attribute(attributes: &[Attribute], name: &str) -> bool {
    get_attribute(attributes, name).is_some()
}

fn type_matches(attributes: &[Attribute], value: &str) -> bool {
    get_attribute(attributes, "type") == Some(value)
}

// Top-level function to parse a HTML string into a FormattedText document.
pub fn parse_html(html: &str) -> Result<FormattedText> {
    let opts = ParseOpts {
        tree_builder: TreeBuilderOpts {
            drop_doctype: true,
            ..Default::default()
        },
        ..Default::default()
    };

    // Parse the document into a Dom element tree.
    let dom = parse_document(RcDom::default(), opts)
        .from_utf8()
        .read_from(&mut html.as_bytes())?;

    let mut result = VecDeque::new();

    // Top-level nodes to visit.
    let mut nodes: Vec<(Rc<Node>, Option<ListArg>)> = Vec::new();
    nodes.push((Rc::clone(&dom.document), None));
    let mut pending_inline_nodes = Vec::new();

    // Active indent level decorating the current node.
    let mut last_active_indent_level: Option<ListArg> = None;

    while let Some((node, mut indent_level)) = nodes.pop() {
        // If the indentation level has changed, we should push all pending inline nodes into the previous
        // indentation level first.
        if last_active_indent_level != indent_level && !pending_inline_nodes.is_empty() {
            if let Some(parsed_node) =
                parse_pending_inline_nodes(&pending_inline_nodes, last_active_indent_level.as_ref())
            {
                result.push_back(parsed_node);
            }
            pending_inline_nodes.clear();
        }
        match &node.data {
            // Nodes we are not processing. Just push its children into the visit queue.
            NodeData::Document
            | NodeData::Doctype { .. }
            | NodeData::ProcessingInstruction { .. }
            | NodeData::Comment { .. } => {
                for child in node.children.borrow().iter().rev() {
                    nodes.push((Rc::clone(child), indent_level.clone()));
                }
            }
            // If we observe plain text in the top level nodes. Add them as plain text lines.
            NodeData::Text { contents } => {
                if !contents.borrow().trim().is_empty() {
                    pending_inline_nodes.push(node);
                }
            }
            NodeData::Element { name, attrs, .. } => {
                let node_name = name.local.to_string();
                let mut decorated_styling = Styling::default();

                // Confluence does not follow the common pattern of marking code blocks in <pre> specifically in its view mode.
                // Instead, it marks code blocks in <span> with a specific attribute.
                let is_confluence_code_block = node_name.as_str() == "span"
                    && includes_attribute(&attrs.borrow(), "data-ds--code--code-block");

                // If the node is an element we are skip processing, push its children into the visit queue and skip
                // to the next iteration.
                if TOP_LEVEL_ELEMENT_TAGS_TO_SKIP.contains(&node_name.as_str()) {
                    let is_unordered_list = node_name.as_str() == "ul";
                    let is_ordered_list = node_name.as_str() == "ol";
                    let start_number = if is_ordered_list {
                        get_attribute(&attrs.borrow(), "start").and_then(|value| value.parse().ok())
                    } else {
                        None
                    };
                    if is_unordered_list || is_ordered_list {
                        let level = match &indent_level {
                            Some(level) => level.indent_level + 1,
                            _ => 0,
                        };
                        indent_level = Some(ListArg {
                            item_type: ListType::ListItem {
                                ordered: is_ordered_list,
                            },
                            start_number: Rc::new(Cell::new(start_number)),
                            indent_level: level,
                        });
                    };

                    // Check if the first node represents input.
                    if let Some(NodeData::Element {
                        name,
                        attrs: child_node_attr,
                        ..
                    }) = node.children.borrow().iter().next().map(|node| &node.data)
                    {
                        let child_node_name = name.local.to_string();
                        if let Some(indent_arg) = &mut indent_level
                            && child_node_name.as_str() == "input"
                            && type_matches(&child_node_attr.borrow(), "checkbox")
                        {
                            indent_arg.item_type = ListType::Checkbox(includes_attribute(
                                &child_node_attr.borrow(),
                                "checked",
                            ));
                        }
                    }

                    for child in node.children.borrow().iter().rev() {
                        nodes.push((Rc::clone(child), indent_level.clone()));
                    }

                    if !pending_inline_nodes.is_empty() {
                        if let Some(parsed_nodes) = parse_pending_inline_nodes(
                            &pending_inline_nodes,
                            last_active_indent_level.as_ref(),
                        ) {
                            result.push_back(parsed_nodes);
                        }
                        pending_inline_nodes.clear();
                    }

                    last_active_indent_level = indent_level;
                    continue;
                } else if PHRASING_ELEMENT_TAGS.contains(&node_name.as_str())
                    && !is_confluence_code_block
                {
                    pending_inline_nodes.push(node);
                    last_active_indent_level = indent_level;
                    continue;
                }

                if !pending_inline_nodes.is_empty() {
                    if let Some(parsed_nodes) = parse_pending_inline_nodes(
                        &pending_inline_nodes,
                        last_active_indent_level.as_ref(),
                    ) {
                        result.push_back(parsed_nodes);
                    }
                    pending_inline_nodes.clear();
                }

                // Update styling based on the node's attribute.
                decorated_styling.update_with_attributes(&attrs.borrow());

                result.push_back(match node_name.as_str() {
                    // If it's a code block, process its children node as plain text.
                    "pre" => {
                        if let Some(val) = get_attribute(&attrs.borrow(), WARP_EMBED_ATTRIBUTE_NAME)
                        {
                            FormattedTextLine::Embedded(Mapping::from_iter([(
                                Value::String("id".to_string()),
                                Value::String(val.to_string()),
                            )]))
                        } else {
                            // TODO: Support Github's code block representation.
                            let (content, language) =
                                parse_code_block_and_language(&node.children.borrow());
                            FormattedTextLine::CodeBlock(CodeBlockText {
                                lang: language.unwrap_or(RUNNABLE_BLOCK_MARKDOWN_LANG.to_string()),
                                code: content,
                            })
                        }
                    }
                    "span" if is_confluence_code_block => {
                        FormattedTextLine::CodeBlock(CodeBlockText {
                            lang: get_attribute(&attrs.borrow(), "data-code-lang")
                                .unwrap_or(RUNNABLE_BLOCK_MARKDOWN_LANG)
                                .to_string(),
                            code: parse_text_only(&node.children.borrow()),
                        })
                    }
                    "h1" => FormattedTextLine::Heading(FormattedTextHeader {
                        heading_size: 1,
                        text: parse_phrasing_content(
                            &node.children.borrow(),
                            decorated_styling.clone(),
                        ),
                    }),
                    "h2" => FormattedTextLine::Heading(FormattedTextHeader {
                        heading_size: 2,
                        text: parse_phrasing_content(
                            &node.children.borrow(),
                            decorated_styling.clone(),
                        ),
                    }),
                    "h3" => FormattedTextLine::Heading(FormattedTextHeader {
                        heading_size: 3,
                        text: parse_phrasing_content(
                            &node.children.borrow(),
                            decorated_styling.clone(),
                        ),
                    }),
                    "h4" => FormattedTextLine::Heading(FormattedTextHeader {
                        heading_size: 4,
                        text: parse_phrasing_content(
                            &node.children.borrow(),
                            decorated_styling.clone(),
                        ),
                    }),
                    "h5" => FormattedTextLine::Heading(FormattedTextHeader {
                        heading_size: 5,
                        text: parse_phrasing_content(
                            &node.children.borrow(),
                            decorated_styling.clone(),
                        ),
                    }),
                    "h6" => FormattedTextLine::Heading(FormattedTextHeader {
                        heading_size: 6,
                        text: parse_phrasing_content(
                            &node.children.borrow(),
                            decorated_styling.clone(),
                        ),
                    }),
                    "br" => FormattedTextLine::LineBreak,
                    "hr" => FormattedTextLine::HorizontalRule,
                    _ => {
                        // Take into consideration the indent level when parsing the nodes.
                        let parsed_node = parse_pending_inline_nodes(
                            &node.children.borrow(),
                            indent_level.as_ref(),
                        );

                        match parsed_node {
                            Some(node) => node,
                            None => FormattedTextLine::Line(Vec::new()),
                        }
                    }
                })
            }
        }

        last_active_indent_level = indent_level;
    }

    if !pending_inline_nodes.is_empty() {
        if let Some(parsed_nodes) =
            parse_pending_inline_nodes(&pending_inline_nodes, last_active_indent_level.as_ref())
        {
            result.push_back(parsed_nodes);
        }
        pending_inline_nodes.clear();
    }

    Ok(FormattedText { lines: result })
}

// Push all pending inline nodes into the result. Take into consideration the active indent level.
fn parse_pending_inline_nodes(
    nodes: &[Rc<Node>],
    last_active_indent_level: Option<&ListArg>,
) -> Option<FormattedTextLine> {
    let internal = parse_phrasing_content(nodes, Default::default());

    if !internal.is_empty() {
        Some(match last_active_indent_level {
            Some(list) => match list.item_type {
                ListType::ListItem { ordered: true } => {
                    FormattedTextLine::OrderedList(OrderedFormattedIndentTextInline {
                        // Take the start number, so that it's only applied to the first item in
                        // the list.
                        number: list.start_number.take(),
                        indented_text: FormattedIndentTextInline {
                            indent_level: list.indent_level,
                            text: internal,
                        },
                    })
                }
                ListType::ListItem { ordered: false } => {
                    FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                        indent_level: list.indent_level,
                        text: internal,
                    })
                }
                ListType::Checkbox(checked) => FormattedTextLine::TaskList(FormattedTaskList {
                    complete: checked,
                    indent_level: list.indent_level,
                    text: internal,
                }),
            },
            None => FormattedTextLine::Line(internal),
        })
    } else {
        None
    }
}

// Parse the phrasing content: https://developer.mozilla.org/en-US/docs/Web/HTML/Content_categories#phrasing_content
// into an inline formatted text.
fn parse_phrasing_content(nodes: &[Rc<Node>], text_styling: Styling) -> FormattedTextInline {
    let mut result = Vec::new();
    for node in nodes {
        if is_spacing_span(node) {
            result.push(phrasing_to_formatted_text(" ", &text_styling));
            continue;
        }

        match &node.data {
            // We should not observe these in the phrasing content.
            NodeData::Document
            | NodeData::Doctype { .. }
            | NodeData::ProcessingInstruction { .. }
            | NodeData::Comment { .. } => {}
            // Push text fragment based on the current text_styling.
            NodeData::Text { contents } => {
                let content = contents.borrow().trim_end_matches('\n').to_string();

                // Filter out singular empty lines after trimming as they are from HTML formatting and shouldn't
                // be inserted into the content. The rare valid case we might be missing here is <p>\n</p>.
                // But all major rich text editors I have tested use <br> to represent it instead.
                if content.is_empty() {
                    continue;
                }

                result.push(phrasing_to_formatted_text(content, &text_styling));
            }
            NodeData::Element { name, attrs, .. } => {
                let node_name = name.local.to_string();
                let mut decorated_styling = text_styling.clone();
                decorated_styling.update_with_attributes(&attrs.borrow());
                match node_name.as_ref() {
                    "b" | "strong" => decorated_styling.bold = true,
                    "i" | "em" => decorated_styling.italic = true,
                    "s" => decorated_styling.strikethrough = true,
                    "u" | "ins" => decorated_styling.underline = true,
                    "code" => decorated_styling.inline_code = true,
                    // TODO: We need to add more phrasing styling we support (e.g. links) here.
                    // https://linear.app/warpdotdev/issue/CLD-335/add-html-parsing-for-headers-and-lists
                    _ => (),
                };

                result.extend(parse_phrasing_content(
                    node.children.borrow().as_ref(),
                    decorated_styling,
                ));
            }
        }
    }
    result
}

/// Converts styled phrasing text to a fragment of formatted text.
fn phrasing_to_formatted_text(text: impl Into<String>, styling: &Styling) -> FormattedTextFragment {
    let weight = if styling.bold {
        Some(CustomWeight::Bold)
    } else {
        None
    };

    FormattedTextFragment {
        text: text.into(),
        styles: FormattedTextStyles {
            weight,
            italic: styling.italic,
            underline: styling.underline,
            strikethrough: styling.strikethrough,
            hyperlink: styling.link.clone().map(Hyperlink::Url),
            inline_code: styling.inline_code,
        },
    }
}

/// Chrome and Safari replace some spaces in copied content with non-breaking spaces. This matches
/// such spaces.
///
/// See [this ProseMirror thread](https://discuss.prosemirror.net/t/non-breaking-spaces-being-added-to-pasted-html/3911/4).
fn is_spacing_span(node: &Rc<Node>) -> bool {
    if let NodeData::Element { name, attrs, .. } = &node.data {
        if &name.local != "span" {
            return false;
        }

        // The span must have either no attrs or a single `class="Apple-converted-space"` class.
        let attrs = attrs.borrow();
        if attrs.len() > 1 {
            return false;
        }
        let css_class = attrs.first().filter(|attr| &attr.name.local == "class");
        if css_class.is_some_and(|class| &*class.value != "Apple-converted-space") {
            return false;
        }

        let content = node.children.borrow();
        if content.len() != 1 {
            return false;
        }
        match &content[0].data {
            NodeData::Text { contents } => &**contents.borrow() == "\u{00a0}",
            _ => false,
        }
    } else {
        false
    }
}

// Only parse out text content in the provided nodes and their children.
fn parse_text_only(nodes: &[Rc<Node>]) -> String {
    let mut text = String::new();
    for node in nodes {
        if let NodeData::Text { contents } = &node.data {
            text.push_str(contents.borrow().to_string().as_str());
        } else {
            text.push_str(&parse_text_only(&node.children.borrow()));
        }
    }
    text
}

// Recursively parse out the code block content and it's language info.
fn parse_code_block_and_language(nodes: &[Rc<Node>]) -> (String, Option<String>) {
    let mut text = String::new();
    let mut language = None;
    for node in nodes {
        match &node.data {
            NodeData::Text { contents } => text.push_str(contents.borrow().to_string().as_str()),
            NodeData::Element { name, attrs, .. } => {
                let node_name = name.local.to_string();

                if node_name == "code"
                    && let Some(parsed_lang) = get_attribute(&attrs.borrow(), "class")
                        .and_then(|s| s.strip_prefix("language-").map(|result| result.to_string()))
                {
                    language = Some(parsed_lang);
                }

                let (child_str, new_lang) = parse_code_block_and_language(&node.children.borrow());
                text.push_str(&child_str);

                if new_lang.is_some() {
                    language = new_lang;
                }
            }
            _ => text.push_str(&parse_text_only(&node.children.borrow())),
        }
    }

    (text, language)
}

// Parse a HTML style string into its corresponding name -> value hashmap
// For example "font-style:italic;font-weight:400" will be parsed into
// {"font-style": "italic", "font-weight": "400"}.
fn parse_style_into_dict(style: &str) -> HashMap<&str, &str> {
    let style_pairs = style.split(';');
    let mut style_dict = HashMap::new();

    for pair in style_pairs {
        if pair.contains(':') {
            let name_and_value: Vec<&str> = pair.split(':').collect();
            style_dict.insert(name_and_value[0].trim(), name_and_value[1].trim());
        }
    }

    style_dict
}

#[cfg(test)]
#[path = "html_parser_tests.rs"]
mod tests;
