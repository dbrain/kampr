//! Where a pane sits among the panes laid out beside it: the splits from the root of its group's
//! tree down to it. A client rebuilds the whole tree from its panes' paths, so a patch that moves
//! one divider carries only the panes it moved.

use serde::{Deserialize, Serialize};

/// A ratio in thousandths of the box a split divides, given to its first half.
pub const WHOLE: u16 = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tile {
    /// herdr's tab, or a group of native sessions split from one another (docs/17).
    pub group: String,
    pub path: Vec<Cut>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cut {
    pub split: Split,
    pub ratio: u16,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub second: bool,
}

/// herdr's own words: `right` puts the second half beside the first, `down` below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Split {
    Right,
    Down,
}

impl Split {
    pub fn parse(direction: &str) -> Option<Self> {
        match direction {
            "right" => Some(Split::Right),
            "down" => Some(Split::Down),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Box4 {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Box4 {
    fn holds(&self, other: &Box4) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.x + other.width <= self.x + self.width
            && other.y + other.height <= self.y + self.height
    }
}

pub fn ratio(fraction: f64) -> u16 {
    (fraction.clamp(0.0, 1.0) * WHOLE as f64).round() as u16
}

/// herdr's snapshot flattens a tab's tree (#616): every split with the box it divides, every pane
/// with its own. The splits whose box holds the pane are its ancestors, largest first, and which
/// half it is in is which side of the boundary its middle falls.
pub fn path_of(splits: &[(Split, u16, Box4)], pane: Box4) -> Vec<Cut> {
    let mut over: Vec<&(Split, u16, Box4)> = splits.iter().filter(|(_, _, area)| area.holds(&pane)).collect();
    over.sort_by_key(|(_, _, area)| std::cmp::Reverse(area.width as u64 * area.height as u64));
    over.into_iter()
        .map(|&(split, ratio, area)| {
            let (start, span, middle) = match split {
                Split::Right => (area.x, area.width, pane.x * 2 + pane.width),
                Split::Down => (area.y, area.height, pane.y * 2 + pane.height),
            };
            let boundary = start as u64 * 2 * WHOLE as u64 + span as u64 * 2 * ratio as u64;
            Cut {
                split,
                ratio,
                second: middle as u64 * WHOLE as u64 > boundary,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: u32, y: u32, width: u32, height: u32) -> Box4 {
        Box4 { x, y, width, height }
    }

    #[test]
    fn a_pane_is_placed_by_the_splits_whose_box_holds_it() {
        let splits = [
            (Split::Down, 300, at(72, 0, 48, 40)),
            (Split::Right, 600, at(0, 0, 120, 40)),
        ];
        let cut = |split, ratio, second| Cut { split, ratio, second };
        assert_eq!(
            path_of(&splits, at(0, 0, 72, 40)),
            vec![cut(Split::Right, 600, false)]
        );
        assert_eq!(
            path_of(&splits, at(72, 0, 48, 12)),
            vec![cut(Split::Right, 600, true), cut(Split::Down, 300, false)]
        );
        assert_eq!(
            path_of(&splits, at(72, 12, 48, 28)),
            vec![cut(Split::Right, 600, true), cut(Split::Down, 300, true)]
        );
        assert_eq!(path_of(&[], at(0, 0, 120, 40)), Vec::new());
    }

    #[test]
    fn an_f32_ratio_lands_on_the_nearest_thousandth() {
        assert_eq!(ratio(0.700_000_05), 700);
        assert_eq!(ratio(1.5), WHOLE);
    }
}
