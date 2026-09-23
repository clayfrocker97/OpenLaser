// SPDX-License-Identifier: GPL-3.0-or-later

//! Ownership of a press across delayed or reordered HTTP requests.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// A browser instance and its monotonically increasing press number.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Lease {
    /// Random instance id, changed when the page is reloaded.
    pub client: String,
    /// Increased for each press within that instance.
    pub sequence: u64,
}

/// How many clients keep a watermark at once.
const CLIENTS: usize = 128;

/// How long a client must have been silent before its watermark may be
/// forgotten. Far longer than any request can stay in flight: TCP abandons
/// an unacknowledged connection well within it, and a press that did start
/// without its heartbeats would lapse within the held-control deadman.
const IDLE: Duration = Duration::from_mins(30);

/// Watermarks make release terminal even when it arrives before start, so an
/// old delayed request never becomes new again. A client id changes with
/// every page load, so the table is bounded: when it is full, the watermark
/// of the client silent the longest is forgotten, and only once that client
/// has been silent for [`IDLE`].
#[derive(Default)]
pub(crate) struct Leases(BTreeMap<String, Mark>);

struct Mark {
    sequence: u64,
    seen: Instant,
}

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

    pub(crate) fn start(&mut self, lease: &Lease, now: Instant) -> Result<()> {
        Self::valid(lease)?;
        if self.0.get(&lease.client).is_some_and(|last| lease.sequence <= last.sequence) {
            return Err(Error::Refused("this press was already started or released".into()));
        }
        self.close(lease, now)
    }

    pub(crate) fn close(&mut self, lease: &Lease, now: Instant) -> Result<()> {
        Self::valid(lease)?;
        if self.0.len() >= CLIENTS && !self.0.contains_key(&lease.client) {
            self.forget_idle(now)?;
        }
        let mark = self
            .0
            .entry(lease.client.clone())
            .or_insert(Mark { sequence: lease.sequence, seen: now });
        mark.sequence = mark.sequence.max(lease.sequence);
        mark.seen = mark.seen.max(now);
        Ok(())
    }

    /// Makes room by forgetting the longest-silent client, if it is idle.
    fn forget_idle(&mut self, now: Instant) -> Result<()> {
        let oldest = self
            .0
            .iter()
            .min_by_key(|(_, mark)| mark.seen)
            .filter(|(_, mark)| now.saturating_duration_since(mark.seen) >= IDLE)
            .map(|(client, _)| client.clone())
            .ok_or_else(|| {
                Error::Refused("too many screens are using held controls; try again later".into())
            })?;
        self.0.remove(&oldest);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_before_start_and_old_requests_are_terminal() {
        let mut leases = Leases::default();
        let now = Instant::now();
        let press = |sequence| Lease { client: "browser".into(), sequence };
        leases.close(&press(2), now).unwrap();
        assert!(leases.start(&press(1), now).is_err());
        assert!(leases.start(&press(2), now).is_err());
        leases.start(&press(3), now).unwrap();
        leases.close(&press(1), now).unwrap();
        assert!(leases.start(&press(3), now).is_err());
        leases.start(&press(4), now).unwrap();
    }

    fn page(n: usize) -> Lease {
        Lease { client: format!("page-{n}"), sequence: 1 }
    }

    #[test]
    fn page_loads_beyond_the_table_forget_only_idle_clients() {
        let mut leases = Leases::default();
        let start = Instant::now();
        for n in 0..CLIENTS {
            leases.start(&page(n), start).unwrap();
        }
        // Every client pressed moments ago: none may be forgotten yet.
        let soon = start + Duration::from_mins(1);
        assert!(leases.start(&page(CLIENTS), soon).is_err());
        // One client stays active; the rest fall silent and make room.
        let active = Lease { client: "page-0".into(), sequence: 2 };
        leases.close(&active, start + IDLE).unwrap();
        let later = start + IDLE + Duration::from_secs(1);
        for n in CLIENTS..CLIENTS * 2 - 1 {
            leases.start(&page(n), later).unwrap();
        }
        assert_eq!(leases.0.len(), CLIENTS);
        assert!(leases.start(&active, later).is_err(), "the active client's watermark was kept");
        assert!(leases.start(&page(CLIENTS * 2), later).is_err(), "every client is active");
        leases.start(&page(CLIENTS * 2), later + IDLE).unwrap();
    }
}
