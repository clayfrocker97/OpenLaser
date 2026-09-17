// SPDX-License-Identifier: GPL-3.0-or-later

//! The cutting order.

use crate::{Error, Result, feature};
use openlaser_core::features::{CutOrder, OrderStrategy};
use openlaser_core::geometry::Point;
use std::cmp::Reverse;
use std::collections::BTreeMap;

/// The most contours a search-based order will take.
const MAX_SEARCH: usize = 10_000;

/// What ordering needs to know about one prepared contour.
pub(crate) struct Item {
    /// The middle of its extent.
    pub center: Point,
    /// Whether it is a plain circle.
    pub circle: bool,
    /// How many contours enclose it.
    pub depth: usize,
    /// The drawing contours it came from.
    pub sources: Vec<usize>,
}

/// The indices of `items` in cutting order.
pub(crate) fn arrange(items: &[Item], order: &CutOrder) -> Result<Vec<usize>> {
    let mut sequence = base_order(items, &order.strategy)?;
    if order.circles_first {
        sequence.sort_by_key(|&i| !items[i].circle);
    }
    if order.inner_first {
        sequence.sort_by_key(|&i| Reverse(items[i].depth));
    }
    if order.spread_heat {
        sequence = spread(items, order, sequence)?;
    }
    Ok(sequence)
}

fn base_order(items: &[Item], strategy: &OrderStrategy) -> Result<Vec<usize>> {
    let mut sequence: Vec<usize> = (0..items.len()).collect();
    let by = |key: fn(&Point) -> f64, descending: bool| {
        move |a: &usize, b: &usize| {
            let (a, b) = (key(&items[*a].center), key(&items[*b].center));
            if descending { b.total_cmp(&a) } else { a.total_cmp(&b) }
        }
    };
    match strategy {
        OrderStrategy::AsDrawn => {}
        OrderStrategy::LeftToRight => sequence.sort_by(by(|p| p.x, false)),
        OrderStrategy::RightToLeft => sequence.sort_by(by(|p| p.x, true)),
        OrderStrategy::BottomToTop => sequence.sort_by(by(|p| p.y, false)),
        OrderStrategy::TopToBottom => sequence.sort_by(by(|p| p.y, true)),
        OrderStrategy::Nearest => sequence = nearest(items)?,
        OrderStrategy::Manual(listed) => {
            let ranks = manual_ranks(items, listed)?;
            sequence.sort_by_key(|&i| {
                items[i].sources.iter().filter_map(|s| ranks.get(s)).min().copied()
            });
        }
    }
    Ok(sequence)
}

/// Each next contour the nearest to the last, starting from the origin.
fn nearest(items: &[Item]) -> Result<Vec<usize>> {
    if items.len() > MAX_SEARCH {
        return Err(Error::Budget("nearest-next ordering"));
    }
    let mut pending: Vec<usize> = (0..items.len()).collect();
    let mut sequence = Vec::with_capacity(items.len());
    let mut at = Point::ORIGIN;
    while !pending.is_empty() {
        let (slot, &index) = pending
            .iter()
            .enumerate()
            .min_by(|a, b| {
                at.distance(items[*a.1].center).total_cmp(&at.distance(items[*b.1].center))
            })
            .unwrap_or((0, &pending[0]));
        sequence.push(index);
        at = items[index].center;
        pending.remove(slot);
    }
    Ok(sequence)
}

/// The rank of every drawing contour in a manual order, which must list
/// each exactly once.
fn manual_ranks(items: &[Item], listed: &[usize]) -> Result<BTreeMap<usize, usize>> {
    let mut sorted = listed.to_vec();
    sorted.sort_unstable();
    let mut sources: Vec<usize> =
        items.iter().flat_map(|item| item.sources.iter().copied()).collect();
    sources.sort_unstable();
    sources.dedup();
    if sorted != sources {
        return Err(feature(
            "order",
            "a manual order must list every drawing contour exactly once",
        ));
    }
    Ok(listed.iter().enumerate().map(|(rank, index)| (*index, rank)).collect())
}

/// Keeps consecutive cuts apart: within each run of equal priority, the
/// next contour is the one farthest from the last.
fn spread(items: &[Item], order: &CutOrder, mut pending: Vec<usize>) -> Result<Vec<usize>> {
    if pending.len() > MAX_SEARCH {
        return Err(Error::Budget("heat-spreading order"));
    }
    let priority = |i: usize| {
        (if order.inner_first { items[i].depth } else { 0 }, order.circles_first && items[i].circle)
    };
    let mut sequence = Vec::with_capacity(pending.len());
    let mut previous: Option<usize> = None;
    while !pending.is_empty() {
        let group = priority(pending[0]);
        let mut chosen = 0;
        if let Some(last) = previous {
            let mut farthest = items[last].center.distance(items[pending[0]].center);
            for (slot, &candidate) in pending.iter().enumerate().skip(1) {
                if priority(candidate) != group {
                    continue;
                }
                let distance = items[last].center.distance(items[candidate].center);
                if distance > farthest {
                    farthest = distance;
                    chosen = slot;
                }
            }
        }
        let index = pending.remove(chosen);
        sequence.push(index);
        previous = Some(index);
    }
    Ok(sequence)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<Item> {
        // An outline at the origin with a round hole and a square hole, and
        // a second part to the right.
        vec![
            Item { center: Point::new(50., 50.), circle: false, depth: 0, sources: vec![0] },
            Item { center: Point::new(30., 50.), circle: true, depth: 1, sources: vec![1] },
            Item { center: Point::new(70., 50.), circle: false, depth: 1, sources: vec![2] },
            Item { center: Point::new(150., 50.), circle: false, depth: 0, sources: vec![3] },
        ]
    }

    /// Positional strategies sort by the centre, nearest-next walks from
    /// the origin, and inner-first and circles-first stack on top as
    /// stable sorts.
    #[test]
    fn strategies_and_priorities_compose() {
        let order = |strategy, inner_first, circles_first| CutOrder {
            strategy,
            inner_first,
            circles_first,
            spread_heat: false,
        };
        assert_eq!(
            arrange(&items(), &order(OrderStrategy::AsDrawn, false, false)).unwrap(),
            vec![0, 1, 2, 3]
        );
        assert_eq!(
            arrange(&items(), &order(OrderStrategy::RightToLeft, false, false)).unwrap(),
            vec![3, 2, 0, 1]
        );
        assert_eq!(
            arrange(&items(), &order(OrderStrategy::Nearest, false, false)).unwrap(),
            vec![1, 0, 2, 3]
        );
        assert_eq!(
            arrange(&items(), &order(OrderStrategy::AsDrawn, true, false)).unwrap(),
            vec![1, 2, 0, 3]
        );
        assert_eq!(
            arrange(&items(), &order(OrderStrategy::LeftToRight, true, true)).unwrap(),
            vec![1, 2, 0, 3]
        );
        assert_eq!(
            arrange(&items(), &order(OrderStrategy::RightToLeft, false, true)).unwrap(),
            vec![1, 3, 2, 0]
        );
    }

    /// A manual order lists drawing contours, and must list all of them
    /// exactly once; spreading heat alternates between far-apart contours
    /// within each priority group.
    #[test]
    fn manual_and_heat_spreading_orders() {
        let manual =
            CutOrder { strategy: OrderStrategy::Manual(vec![3, 2, 1, 0]), ..CutOrder::default() };
        assert_eq!(arrange(&items(), &manual).unwrap(), vec![3, 2, 1, 0]);
        let short = CutOrder { strategy: OrderStrategy::Manual(vec![3, 2]), ..CutOrder::default() };
        assert!(matches!(arrange(&items(), &short), Err(Error::Feature { .. })));
        let heat = CutOrder {
            strategy: OrderStrategy::AsDrawn,
            inner_first: true,
            circles_first: false,
            spread_heat: true,
        };
        assert_eq!(arrange(&items(), &heat).unwrap(), vec![1, 2, 3, 0]);
    }
}
