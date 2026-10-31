//! How the mosaic's boxes tile, and the operator's own adjustments to them.
//!
//! **Mirrored where there is a tree to mirror.** A herdr tab, and native sessions split from one
//! another, reach the wire as each pane's path through the tree (#616), and they are laid out as
//! their keeper has them; anything else is tiled by count. The operator's adjustments are this
//! client's own, and nothing in this module may become an op (ADR 0015).

use crate::keymap::Dir;
use kampr_core::tile::{Cut, Split};
use ratatui::layout::Rect;
use std::collections::HashMap;

const WHOLE: u32 = kampr_core::tile::WHOLE as u32;

const LEAST: u16 = 100;
const MOST: u16 = 900;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Arrange {
    /// One full, two side by side, one beside two stacked, then two by two.
    #[default]
    Tiled,
    Columns,
    Rows,
}

impl Arrange {
    pub fn next(self) -> Self {
        match self {
            Arrange::Tiled => Arrange::Columns,
            Arrange::Columns => Arrange::Rows,
            Arrange::Rows => Arrange::Tiled,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Arrange::Tiled => "tiled",
            Arrange::Columns => "side by side",
            Arrange::Rows => "stacked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Across,
    Down,
}

/// What the boxes are: kampr's own arrangement of `count`, or a tree somebody else keeps (#616)
/// — herdr's tab, or the node's native sessions — as each pane's path through it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    Count(usize),
    Tree { group: String, paths: Vec<Vec<Cut>> },
}

impl Shape {
    pub fn panes(&self) -> usize {
        match self {
            Shape::Count(count) => *count,
            Shape::Tree { paths, .. } => paths.len(),
        }
    }

    pub fn is_tree(&self) -> bool {
        matches!(self, Shape::Tree { .. })
    }
}

/// A split by where it sits, so an adjustment survives a pane closing elsewhere in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SplitId {
    shape: Key,
    at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Count(usize, Arrange),
    Group(u64),
}

/// A boundary between two boxes as the last frame drew it: `area` is the split's whole box and
/// `at` the first cell of its second half, so a drag is measured against what is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divider {
    pub split: SplitId,
    pub axis: Axis,
    pub area: Rect,
    pub at: u16,
    pub grip: Rect,
    given: u16,
}

#[derive(Debug, Default)]
pub struct Tiles {
    pub boxes: Vec<Rect>,
    pub dividers: Vec<Divider>,
}

enum Node {
    Leaf(usize),
    Split(Axis, u16, Box<Node>, Box<Node>),
}

/// The operator's own ratios, each beside the one it replaced: a tree whose keeper moves that
/// divider has the last word, so the adjustment is dropped rather than fought over.
#[derive(Debug, Default)]
pub struct Tiling {
    pub arrange: Arrange,
    ratios: HashMap<SplitId, (u16, u16)>,
}

impl Tiling {
    /// Whether `area` is too small for the shape as its keeper lays it out. The operator's own
    /// ratios are left out: a divider dragged to make one box tiny is a choice, not a small screen.
    pub fn cramped(&self, area: Rect, shape: &Shape, least: (u16, u16)) -> bool {
        let plain = Tiling {
            arrange: self.arrange,
            ratios: HashMap::new(),
        };
        shape.panes() > 1
            && plain
                .lay(area, shape)
                .boxes
                .iter()
                .any(|b| b.width < least.0 || b.height < least.1)
    }

    pub fn lay(&self, area: Rect, shape: &Shape) -> Tiles {
        let mut tiles = Tiles {
            boxes: vec![Rect::default(); shape.panes()],
            dividers: Vec::new(),
        };
        if let Some(node) = self.tree(shape) {
            self.place(&node, &self.key(shape), area, 1, &mut tiles);
        }
        tiles
    }

    fn key(&self, shape: &Shape) -> Key {
        match shape {
            Shape::Count(count) => Key::Count(*count, self.arrange),
            Shape::Tree { group, .. } => {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                group.hash(&mut h);
                Key::Group(h.finish())
            }
        }
    }

    fn tree(&self, shape: &Shape) -> Option<Node> {
        match shape {
            Shape::Count(0) => None,
            Shape::Count(count) => Some(tree(*count, self.arrange)),
            Shape::Tree { paths, .. } => {
                let all: Vec<(usize, &[Cut])> = paths.iter().map(Vec::as_slice).enumerate().collect();
                grown(&all, 0)
            }
        }
    }

    fn ratio(&self, split: &SplitId, given: u16) -> u16 {
        match self.ratios.get(split) {
            Some(&(over, mine)) if over == given => mine,
            _ => given,
        }
    }

    fn place(&self, node: &Node, key: &Key, area: Rect, at: u64, tiles: &mut Tiles) {
        match node {
            Node::Leaf(i) => tiles.boxes[*i] = area,
            Node::Split(axis, given, first, second) => {
                let split = SplitId { shape: *key, at };
                let (a, b) = cut(area, *axis, self.ratio(&split, *given));
                let edge = match axis {
                    Axis::Across => b.x,
                    Axis::Down => b.y,
                };
                let grip = match axis {
                    Axis::Across => Rect {
                        x: edge.saturating_sub(1),
                        width: 2.min(area.right() - edge.saturating_sub(1)),
                        ..area
                    },
                    Axis::Down => Rect {
                        y: edge.saturating_sub(1),
                        height: 2.min(area.bottom() - edge.saturating_sub(1)),
                        ..area
                    },
                };
                tiles.dividers.push(Divider {
                    split,
                    axis: *axis,
                    area,
                    at: edge,
                    grip,
                    given: *given,
                });
                self.place(first, key, a, at << 1, tiles);
                self.place(second, key, b, at << 1 | 1, tiles);
            }
        }
    }

    /// The focused box's share along `axis`, from the nearest split that divides it that way.
    /// False when nothing does — a lone row has no height to give.
    pub fn grow(&mut self, shape: &Shape, focused: usize, axis: Axis, by: i32) -> bool {
        let Some(node) = self.tree(shape) else {
            return false;
        };
        let mut path = Vec::new();
        if !trail(&node, focused, 1, &mut path) {
            return false;
        }
        let Some(&(at, given, first, _)) = path.iter().rev().find(|(_, _, _, along)| *along == axis) else {
            return false;
        };
        let split = SplitId {
            shape: self.key(shape),
            at,
        };
        let ratio = self.ratio(&split, given) as i32;
        let ratio = match first {
            true => ratio + by,
            false => ratio - by,
        };
        self.ratios.insert(split, (given, clamp(ratio)));
        true
    }

    /// `to` is the cell the boundary's first half should now end on.
    pub fn drag(&mut self, divider: &Divider, to: u16) {
        let (start, span) = match divider.axis {
            Axis::Across => (divider.area.x, divider.area.width),
            Axis::Down => (divider.area.y, divider.area.height),
        };
        if span == 0 {
            return;
        }
        let first = to.saturating_sub(start) as u32;
        let ratio = (first * WHOLE + span as u32 / 2) / span as u32;
        self.ratios
            .insert(divider.split, (divider.given, clamp(ratio as i32)));
    }

    /// The box on `dir`'s side of the focused one that overlaps it most, nearest first. No wrap:
    /// a key that would leave the mosaic stays where it is.
    pub fn neighbour(&self, shape: &Shape, from: usize, dir: Dir) -> Option<usize> {
        let boxes = self.lay(Rect::new(0, 0, 1000, 1000), shape).boxes;
        let here = *boxes.get(from)?;
        boxes
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != from)
            .filter_map(|(i, there)| {
                let (gap, overlap) = match dir {
                    Dir::Left => (
                        here.x.checked_sub(there.right())?,
                        span(here.y, here.bottom(), there.y, there.bottom()),
                    ),
                    Dir::Right => (
                        there.x.checked_sub(here.right())?,
                        span(here.y, here.bottom(), there.y, there.bottom()),
                    ),
                    Dir::Up => (
                        here.y.checked_sub(there.bottom())?,
                        span(here.x, here.right(), there.x, there.right()),
                    ),
                    Dir::Down => (
                        there.y.checked_sub(here.bottom())?,
                        span(here.x, here.right(), there.x, there.right()),
                    ),
                };
                (overlap > 0).then_some((gap, u16::MAX - overlap, i))
            })
            .min()
            .map(|(_, _, i)| i)
    }
}

fn span(a0: u16, a1: u16, b0: u16, b1: u16) -> u16 {
    a1.min(b1).saturating_sub(a0.max(b0))
}

fn clamp(ratio: i32) -> u16 {
    ratio.clamp(LEAST as i32, MOST as i32) as u16
}

fn cut(area: Rect, axis: Axis, ratio: u16) -> (Rect, Rect) {
    let span = match axis {
        Axis::Across => area.width,
        Axis::Down => area.height,
    };
    let first = ((span as u32 * ratio as u32 + WHOLE / 2) / WHOLE) as u16;
    let first = first.clamp(span.min(1), span.saturating_sub(1).max(span.min(1)));
    match axis {
        Axis::Across => (
            Rect { width: first, ..area },
            Rect {
                x: area.x + first,
                width: span - first,
                ..area
            },
        ),
        Axis::Down => (
            Rect {
                height: first,
                ..area
            },
            Rect {
                y: area.y + first,
                height: span - first,
                ..area
            },
        ),
    }
}

fn tree(count: usize, arrange: Arrange) -> Node {
    let leaf = |i| Box::new(Node::Leaf(i));
    let half = (WHOLE / 2) as u16;
    match (arrange, count) {
        (_, 0 | 1) => Node::Leaf(0),
        (Arrange::Tiled, 2) => Node::Split(Axis::Across, half, leaf(0), leaf(1)),
        (Arrange::Tiled, 3) => Node::Split(
            Axis::Across,
            half,
            leaf(0),
            Box::new(Node::Split(Axis::Down, half, leaf(1), leaf(2))),
        ),
        (Arrange::Tiled, 4) => Node::Split(
            Axis::Across,
            half,
            Box::new(Node::Split(Axis::Down, half, leaf(0), leaf(2))),
            Box::new(Node::Split(Axis::Down, half, leaf(1), leaf(3))),
        ),
        (Arrange::Rows, _) => chain(Axis::Down, 0, count),
        (Arrange::Columns | Arrange::Tiled, _) => chain(Axis::Across, 0, count),
    }
}

fn chain(axis: Axis, from: usize, count: usize) -> Node {
    let left = count - from;
    match left {
        1 => Node::Leaf(from),
        _ => Node::Split(
            axis,
            (WHOLE / left as u32) as u16,
            Box::new(Node::Leaf(from)),
            Box::new(chain(axis, from + 1, count)),
        ),
    }
}

/// Rebuilds a tree from its leaves' paths. A pane whose path disagrees with its neighbours' — a
/// patch that has reached one of them and not the other yet — gets no box until they agree.
fn grown(leaves: &[(usize, &[Cut])], depth: usize) -> Option<Node> {
    let (first_leaf, path) = *leaves.first()?;
    let Some(cut) = path.get(depth) else {
        return Some(Node::Leaf(first_leaf));
    };
    let (seconds, firsts): (Vec<_>, Vec<_>) = leaves
        .iter()
        .copied()
        .partition(|(_, path)| path.get(depth).is_some_and(|c| c.second));
    let axis = match cut.split {
        Split::Right => Axis::Across,
        Split::Down => Axis::Down,
    };
    match (grown(&firsts, depth + 1), grown(&seconds, depth + 1)) {
        (Some(a), Some(b)) => Some(Node::Split(axis, cut.ratio, Box::new(a), Box::new(b))),
        (one, other) => one.or(other),
    }
}

/// The splits from the root down to `leaf`, each with its ratio, whether the leaf sits in its
/// first half, and its axis.
fn trail(node: &Node, leaf: usize, at: u64, path: &mut Vec<(u64, u16, bool, Axis)>) -> bool {
    match node {
        Node::Leaf(here) => *here == leaf,
        Node::Split(axis, given, first, second) => {
            path.push((at, *given, true, *axis));
            if trail(first, leaf, at << 1, path) {
                return true;
            }
            path.pop();
            path.push((at, *given, false, *axis));
            if trail(second, leaf, at << 1 | 1, path) {
                return true;
            }
            path.pop();
            false
        }
    }
}
