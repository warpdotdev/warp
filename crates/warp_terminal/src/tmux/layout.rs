//! Parse tmux `window-layout` strings into pane geometry.
//!
//! Format: optional 4-hex checksum, then a cell:
//! `WxH,X,Y,paneid` or `WxH,X,Y{cell,cell}` (side-by-side) or `WxH,X,Y[cell,cell]` (stacked).

use super::parser::PaneId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutLeaf {
    pub width: u32,
    pub height: u32,
    pub x: u32,
    pub y: u32,
    pub pane_id: PaneId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutNode {
    Leaf(LayoutLeaf),
    Split {
        horizontal: bool,
        width: u32,
        height: u32,
        x: u32,
        y: u32,
        children: Vec<LayoutNode>,
    },
}

impl LayoutNode {
    pub fn leaves(&self) -> Vec<&LayoutLeaf> {
        match self {
            Self::Leaf(leaf) => vec![leaf],
            Self::Split { children, .. } => children.iter().flat_map(Self::leaves).collect(),
        }
    }

    pub fn pane_ids(&self) -> Vec<PaneId> {
        self.leaves()
            .into_iter()
            .map(|leaf| leaf.pane_id.clone())
            .collect()
    }

    fn size_along(&self, horizontal: bool) -> u32 {
        match self {
            Self::Leaf(leaf) => {
                if horizontal {
                    leaf.width
                } else {
                    leaf.height
                }
            }
            Self::Split { width, height, .. } => {
                if horizontal {
                    *width
                } else {
                    *height
                }
            }
        }
    }
}

/// A Warp split that inserts a pane beside the previous sibling subtree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitStep {
    pub parent: Vec<PaneId>,
    pub new_pane: PaneId,
    pub side_by_side: bool,
    pub parent_size: u32,
    pub new_size: u32,
}

pub fn parse_window_layout(input: &str) -> Option<LayoutNode> {
    let rest = strip_checksum(input)?;
    let (node, leftover) = parse_cell(rest)?;
    leftover.trim().is_empty().then_some(node)
}

pub fn split_steps(layout: &LayoutNode) -> Vec<SplitStep> {
    let mut steps = Vec::new();
    collect_split_steps(layout, &mut steps);
    steps
}

pub fn missing_from_layout(bound: &[PaneId], layout: &LayoutNode) -> Vec<PaneId> {
    let live = layout.pane_ids();
    bound
        .iter()
        .filter(|id| !live.contains(id))
        .cloned()
        .collect()
}

fn collect_split_steps(node: &LayoutNode, steps: &mut Vec<SplitStep>) {
    let LayoutNode::Split {
        horizontal,
        children,
        ..
    } = node
    else {
        return;
    };
    if children.is_empty() {
        return;
    }
    collect_split_steps(&children[0], steps);
    let mut previous_leaves = children[0].pane_ids();
    let mut previous_size = children[0].size_along(*horizontal);
    for child in children.iter().skip(1) {
        let child_ids = child.pane_ids();
        if let Some(new_pane) = child_ids.first().cloned() {
            let new_size = child.size_along(*horizontal);
            steps.push(SplitStep {
                parent: previous_leaves,
                new_pane: new_pane.clone(),
                side_by_side: *horizontal,
                parent_size: previous_size,
                new_size,
            });
            previous_leaves = child_ids;
            previous_size = new_size;
        }
        collect_split_steps(child, steps);
    }
}

fn strip_checksum(input: &str) -> Option<&str> {
    let input = input.trim();
    if input.len() >= 5 && input.as_bytes()[4] == b',' && input[..4].bytes().all(is_hex) {
        Some(&input[5..])
    } else {
        Some(input)
    }
}

fn is_hex(b: u8) -> bool {
    b.is_ascii_hexdigit()
}

fn parse_cell(input: &str) -> Option<(LayoutNode, &str)> {
    let (width, rest) = parse_u32(input)?;
    let rest = rest.strip_prefix('x')?;
    let (height, rest) = parse_u32(rest)?;
    let rest = rest.strip_prefix(',')?;
    let (x, rest) = parse_u32(rest)?;
    let rest = rest.strip_prefix(',')?;
    let (y, rest) = parse_u32(rest)?;
    if let Some(rest) = rest.strip_prefix('{') {
        let (children, rest) = parse_children(rest)?;
        let rest = rest.strip_prefix('}')?;
        return Some((
            LayoutNode::Split {
                horizontal: true,
                width,
                height,
                x,
                y,
                children,
            },
            rest,
        ));
    }
    if let Some(rest) = rest.strip_prefix('[') {
        let (children, rest) = parse_children(rest)?;
        let rest = rest.strip_prefix(']')?;
        return Some((
            LayoutNode::Split {
                horizontal: false,
                width,
                height,
                x,
                y,
                children,
            },
            rest,
        ));
    }
    let rest = rest.strip_prefix(',')?;
    let (pane_index, rest) = parse_u32(rest)?;
    Some((
        LayoutNode::Leaf(LayoutLeaf {
            width,
            height,
            x,
            y,
            pane_id: PaneId::from(format!("%{pane_index}").as_str()),
        }),
        rest,
    ))
}

fn parse_children(mut input: &str) -> Option<(Vec<LayoutNode>, &str)> {
    let mut children = Vec::new();
    loop {
        let (child, rest) = parse_cell(input)?;
        children.push(child);
        if let Some(rest) = rest.strip_prefix(',') {
            input = rest;
        } else {
            return Some((children, rest));
        }
    }
}

fn parse_u32(input: &str) -> Option<(u32, &str)> {
    let end = input
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(input.len());
    if end == 0 {
        return None;
    }
    let value = input[..end].parse().ok()?;
    Some((value, &input[end..]))
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
