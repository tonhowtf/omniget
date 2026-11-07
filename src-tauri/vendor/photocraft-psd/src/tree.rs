//! Nested layer tree reconstruction from section dividers.
//!
//! In the file, layers are stored bottom-to-top. A group appears as a
//! *bounding section divider* (`lsct` type 3, usually named
//! `</Layer group>`) below its children, followed by the children, followed
//! by the group's own record (`lsct` type 1 = open or 2 = closed folder).

use crate::file::PsdFile;
use crate::tagged::SectionType;

/// A node of the layer tree. Children are in file order (bottom-to-top).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerNode {
    /// A non-group layer.
    Layer {
        /// Index into [`PsdFile::layers`].
        index: usize,
    },
    /// A group (folder).
    Group {
        /// Index of the group's own record (the folder, top of the group).
        index: usize,
        /// Index of the bounding section divider, if one was found.
        divider: Option<usize>,
        /// Children, bottom-to-top.
        children: Vec<LayerNode>,
    },
}

impl LayerNode {
    /// Record index of this node.
    pub fn index(&self) -> usize {
        match self {
            LayerNode::Layer { index } | LayerNode::Group { index, .. } => *index,
        }
    }
    /// Children (empty for layers).
    pub fn children(&self) -> &[LayerNode] {
        match self {
            LayerNode::Layer { .. } => &[],
            LayerNode::Group { children, .. } => children,
        }
    }
    /// `true` for groups.
    pub fn is_group(&self) -> bool {
        matches!(self, LayerNode::Group { .. })
    }
    /// Total number of nodes in this subtree (including self).
    pub fn count(&self) -> usize {
        1 + self.children().iter().map(LayerNode::count).sum::<usize>()
    }
}

impl PsdFile {
    /// Builds the nested layer tree. Root nodes and children are ordered
    /// bottom-to-top (file order).
    ///
    /// Malformed nesting is tolerated: a folder without a matching divider
    /// becomes an empty group; unclosed dividers at the end are dropped and
    /// their children are hoisted to the enclosing level.
    ///
    /// The tree can be as deep as the file nests its groups (up to ~16000
    /// levels). Code that walks it recursively should use
    /// [`layer_tree_max_depth`](Self::layer_tree_max_depth).
    pub fn layer_tree(&self) -> Vec<LayerNode> {
        self.layer_tree_max_depth(usize::MAX)
    }

    /// [`layer_tree`](Self::layer_tree) with at most `max_depth` levels of
    /// groups: a group nested `max_depth` or more groups deep (a root node is
    /// at depth 0) keeps its node but not its children. Layers under a divider
    /// that is never closed sit at the enclosing level, as in `layer_tree`. Built iteratively, so
    /// no file can make it recurse, and the result is no deeper than
    /// `max_depth + 1` levels, so walking or dropping it recursively is bounded.
    pub fn layer_tree_max_depth(&self, max_depth: usize) -> Vec<LayerNode> {
        let layers = self.layers();
        // First pass: find the dividers a folder closes. A divider that is never closed opens no
        // group; its layers belong to the enclosing level, so it opens no frame and every
        // frame's depth is the final depth of its nodes (#5758).
        let mut closed = vec![false; layers.len()];
        let mut open: Vec<usize> = Vec::new();
        for (i, rec) in layers.iter().enumerate() {
            match rec.section_type() {
                SectionType::BoundingDivider => open.push(i),
                SectionType::OpenFolder | SectionType::ClosedFolder => {
                    if let Some(slot) = open.pop().and_then(|d| closed.get_mut(d)) {
                        *slot = true;
                    }
                }
                _ => {}
            }
        }
        // Stack of (divider index, children); frame `k` holds the nodes at depth `k`. Frames
        // past `max_depth` are tracked (to match dividers to folders) but collect nothing.
        let mut stack: Vec<(Option<usize>, Vec<LayerNode>)> = vec![(None, Vec::new())];
        let push = |stack: &mut Vec<(Option<usize>, Vec<LayerNode>)>, node: LayerNode| {
            if stack.len() <= max_depth.saturating_add(1)
                && let Some(top) = stack.last_mut()
            {
                top.1.push(node);
            }
        };
        for (i, rec) in layers.iter().enumerate() {
            match rec.section_type() {
                SectionType::BoundingDivider => {
                    if closed.get(i).copied().unwrap_or(false) {
                        stack.push((Some(i), Vec::new()));
                    }
                }
                SectionType::OpenFolder | SectionType::ClosedFolder => {
                    let (divider, children) = if stack.len() > 1 { stack.pop().unwrap_or_default() } else { (None, Vec::new()) };
                    push(&mut stack, LayerNode::Group { index: i, divider, children });
                }
                _ => push(&mut stack, LayerNode::Layer { index: i }),
            }
        }
        // Only closed dividers open frames, so none is left here; this just guards the invariant.
        while stack.len() > 1 {
            let (_, children) = stack.pop().unwrap_or_default();
            if let Some(top) = stack.last_mut() {
                top.1.extend(children);
            }
        }
        stack.pop().map(|(_, c)| c).unwrap_or_default()
    }
}
