// SPDX-License-Identifier: GPL-3.0-or-later

//! Ownership of a press across delayed or reordered HTTP requests.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A browser instance and its monotonically increasing press number.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Lease {
    /// Random instance id, changed when the page is reloaded.
    pub client: String,
    /// Increased for each press within that instance.
    pub sequence: u64,
}

/// Watermarks make release terminal even when it arrives before start.
/// They are not evicted: an old delayed request must never become new again.
#[derive(Default)]
pub(crate) struct Leases(BTreeMap<String, u64>);

impl Leases {
    fn valid(lease: &Lease) -> Result<()> {
        if lease.client.is_empty()
            || lease.client.len() > 64
            || !lease.client.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            || lease.sequence == 0
            || lease.sequence > 9_007_199_254_740_991
        {
            return Err(Error::Refused("invalid press identity".into()));
        }
        Ok(())
    }

    pub(crate) fn start(&mut self, lease: &Lease) -> Result<()> {
        Self::valid(lease)?;
        if self.0.get(&lease.client).is_some_and(|last| lease.sequence <= *last) {
            return Err(Error::Refused("this press was already started or released".into()));
        }
        self.close(lease)
    }

    pub(crate) fn close(&mut self, lease: &Lease) -> Result<()> {
        Self::valid(lease)?;
        if self.0.len() >= 128 && !self.0.contains_key(&lease.client) {
            return Err(Error::Refused("too many control clients; restart the server".into()));
        }
        self.0
            .entry(lease.client.clone())
            .and_modify(|n| *n = (*n).max(lease.sequence))
            .or_insert(lease.sequence);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_before_start_and_old_requests_are_terminal() {
        let mut leases = Leases::default();
        let press = |sequence| Lease { client: "browser".into(), sequence };
        leases.close(&press(2)).unwrap();
        assert!(leases.start(&press(1)).is_err());
        assert!(leases.start(&press(2)).is_err());
        leases.start(&press(3)).unwrap();
        leases.close(&press(1)).unwrap();
        assert!(leases.start(&press(3)).is_err());
        leases.start(&press(4)).unwrap();
    }
}
