// SPDX-License-Identifier: GPL-3.0-or-later

//! Operator grouping overrides for placed contours. Unmentioned contours keep
//! their automatic outline-and-holes groups; a singleton explicitly ungroups.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Disjoint groups of placed-contour indices, independent of machining settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Grouping(pub Vec<Vec<usize>>);

impl Grouping {
    /// Rejects empty, overlapping, or out-of-range groups.
    pub fn validate(&self, count: usize) -> Result<(), String> {
        let mut used = BTreeSet::new();
        for group in &self.0 {
            if group.is_empty() {
                return Err("a group must contain at least one contour".into());
            }
            for &i in group {
                if i >= count || !used.insert(i) {
                    return Err("grouping contains an invalid or repeated contour".into());
                }
            }
        }
        Ok(())
    }

    /// Applies authored overrides to an automatic partition.
    #[must_use]
    pub fn resolve(&self, automatic: Vec<Vec<usize>>) -> Vec<Vec<usize>> {
        let owned: BTreeSet<_> = self.0.iter().flatten().copied().collect();
        let mut groups: Vec<_> = automatic
            .into_iter()
            .map(|g| g.into_iter().filter(|i| !owned.contains(i)).collect::<Vec<_>>())
            .filter(|g| !g.is_empty())
            .chain(self.0.iter().cloned())
            .collect();
        normalize(&mut groups);
        groups
    }

    /// Assigns an already expanded selection to one group or to singletons.
    pub fn assign(&mut self, selected: &[usize], together: bool) {
        let chosen: BTreeSet<_> = selected.iter().copied().collect();
        for group in &mut self.0 {
            group.retain(|i| !chosen.contains(i));
        }
        self.0.retain(|g| !g.is_empty());
        if together {
            self.0.push(chosen.into_iter().collect());
        } else {
            self.0.extend(chosen.into_iter().map(|i| vec![i]));
        }
        self.0.retain(|g| !g.is_empty());
        normalize(&mut self.0);
    }

    /// Keeps authored groups after deletion or copy, dropping absent members.
    #[must_use]
    pub fn remapped(&self, map: impl Fn(usize) -> Option<usize>) -> Self {
        let mut groups: Vec<_> = self
            .0
            .iter()
            .map(|g| g.iter().filter_map(|&i| map(i)).collect::<Vec<_>>())
            .filter(|g| !g.is_empty())
            .collect();
        normalize(&mut groups);
        Self(groups)
    }

    /// Adds a paste's relative groups to the current placement.
    pub fn append(&mut self, pasted: &Self, offset: usize) {
        self.0.extend(pasted.0.iter().map(|g| g.iter().map(|i| i + offset).collect()));
        normalize(&mut self.0);
    }
}

fn normalize(groups: &mut [Vec<usize>]) {
    for group in groups.iter_mut() {
        group.sort_unstable();
        group.dedup();
    }
    groups.sort_unstable();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn singletons_override_containment_without_affecting_new_contours() {
        let mut grouping = Grouping::default();
        grouping.assign(&[1, 0, 1], false);
        assert_eq!(
            grouping.resolve(vec![vec![0, 1, 2], vec![3, 4]]),
            vec![vec![0], vec![1], vec![2], vec![3, 4]]
        );
        grouping.assign(&[0, 1, 3, 4], true);
        assert_eq!(
            grouping.resolve(vec![vec![0, 1, 2], vec![3, 4]]),
            vec![vec![0, 1, 3, 4], vec![2]]
        );
    }

    #[test]
    fn pasted_and_deleted_members_keep_their_partition() {
        let mut grouping = Grouping(vec![vec![0, 2], vec![1]]);
        grouping.append(&Grouping(vec![vec![0, 1]]), 3);
        let remapped = grouping.remapped(|i| i.checked_sub(1));
        assert_eq!(remapped.0, vec![vec![0], vec![1], vec![2, 3]]);
        assert!(remapped.validate(4).is_ok());
        assert!(Grouping(vec![vec![0], vec![0]]).validate(1).is_err());
        assert!(Grouping(vec![vec![]]).validate(1).is_err());
        assert!(Grouping(vec![vec![1]]).validate(1).is_err());
    }
}
