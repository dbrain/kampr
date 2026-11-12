use super::index::Item;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Exact,
    Prefix,
    Substring,
    Subsequence,
}

/// `needle` is already lowercased. One containing `/` is a piece of a path, so it is looked for
/// in the whole relative path rather than in the name.
pub fn tier(item: &Item, needle: &str) -> Option<Tier> {
    if needle.contains('/') {
        return item.lower().contains(needle).then_some(Tier::Substring);
    }
    let name = item.lower_name();
    if name == needle {
        Some(Tier::Exact)
    } else if name.starts_with(needle) {
        Some(Tier::Prefix)
    } else if name.contains(needle) {
        Some(Tier::Substring)
    } else if in_order(name, needle) {
        Some(Tier::Subsequence)
    } else {
        None
    }
}

fn in_order(hay: &str, needle: &str) -> bool {
    let mut hay = hay.chars();
    needle.chars().all(|want| hay.any(|c| c == want))
}

/// The best `limit` items for `needle`, best first, and how many matched in all. Ties go to the
/// shallower, then the shorter, then by path, so the same query always lists in the same order.
pub fn best<'a>(items: &'a [Item], needle: &str, limit: usize) -> (Vec<&'a Item>, usize) {
    let key = |(tier, item): &(Tier, &'a Item)| (*tier, item.depth, item.rel.len(), &*item.rel);
    let mut hits: Vec<(Tier, &Item)> = items
        .iter()
        .filter_map(|item| tier(item, needle).map(|t| (t, item)))
        .collect();
    let total = hits.len();
    if hits.len() > limit && limit > 0 {
        hits.select_nth_unstable_by(limit - 1, |a, b| key(a).cmp(&key(b)));
        hits.truncate(limit);
    }
    hits.sort_unstable_by(|a, b| key(a).cmp(&key(b)));
    hits.truncate(limit);
    (hits.into_iter().map(|(_, item)| item).collect(), total)
}
