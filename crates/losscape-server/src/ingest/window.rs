//! Tracks which batch seqs of a run are already stored, to drop retried batches.
//!
//! Stored as a watermark (`next`: every seq below it is stored) plus the few seqs stored
//! out of order above it. The SDK keeps only a handful of batches in flight, so the
//! out-of-order set stays tiny; seqs too far ahead are rejected to keep it bounded.

use std::collections::BTreeSet;

/// How far ahead of the watermark a seq may be.
pub const MAX_AHEAD: u64 = 4096;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeqWindow {
    next: u64,
    ahead: BTreeSet<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Insert {
    New,
    Duplicate,
    TooFarAhead,
}

impl SeqWindow {
    /// From the `runs.next_seq` / `runs.pending_seqs` columns.
    pub fn from_db(next: i64, pending: &[i64]) -> Self {
        Self {
            next: next.max(0) as u64,
            ahead: pending.iter().map(|&s| s as u64).collect(),
        }
    }

    pub fn to_db(&self) -> (i64, Vec<i64>) {
        (
            self.next as i64,
            self.ahead.iter().map(|&s| s as i64).collect(),
        )
    }

    pub fn contains(&self, seq: u64) -> bool {
        seq < self.next || self.ahead.contains(&seq)
    }

    pub fn insert(&mut self, seq: u64) -> Insert {
        if self.contains(seq) {
            return Insert::Duplicate;
        }
        if seq - self.next >= MAX_AHEAD {
            return Insert::TooFarAhead;
        }
        if seq == self.next {
            self.next += 1;
            while self.ahead.remove(&self.next) {
                self.next += 1;
            }
        } else {
            self.ahead.insert(seq);
        }
        Insert::New
    }

    /// The client says every seq below `floor` is acknowledged and won't be re-sent,
    /// so they can be treated as stored.
    pub fn advance_to(&mut self, floor: u64) {
        if floor <= self.next {
            return;
        }
        self.next = floor;
        self.ahead = self.ahead.split_off(&floor);
        while self.ahead.remove(&self.next) {
            self.next += 1;
        }
    }

    /// First seq a client without local state should use when resuming.
    pub fn resume_seq(&self) -> u64 {
        self.ahead.last().map_or(self.next, |&last| last + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_order_advances_watermark() {
        let mut w = SeqWindow::default();
        for seq in 0..5 {
            assert_eq!(w.insert(seq), Insert::New);
        }
        assert_eq!(w.to_db(), (5, vec![]));
        assert_eq!(w.insert(3), Insert::Duplicate);
    }

    #[test]
    fn out_of_order_fills_gaps() {
        let mut w = SeqWindow::default();
        assert_eq!(w.insert(2), Insert::New);
        assert_eq!(w.insert(1), Insert::New);
        assert_eq!(w.to_db(), (0, vec![1, 2]));
        assert_eq!(w.resume_seq(), 3);
        assert_eq!(w.insert(2), Insert::Duplicate);
        assert_eq!(w.insert(0), Insert::New);
        assert_eq!(w.to_db(), (3, vec![]));
    }

    #[test]
    fn rejects_seqs_too_far_ahead() {
        let mut w = SeqWindow::default();
        assert_eq!(w.insert(MAX_AHEAD), Insert::TooFarAhead);
        assert_eq!(w.insert(MAX_AHEAD - 1), Insert::New);
    }

    #[test]
    fn client_floor_closes_gaps() {
        // Seqs 0..3 were stored but their window update was lost; 3 and 4 arrived after.
        let mut w = SeqWindow::from_db(0, &[3, 4]);
        w.advance_to(3);
        assert_eq!(w.to_db(), (5, vec![]));
        w.advance_to(2);
        assert_eq!(w.to_db(), (5, vec![]), "never moves backwards");

        let mut w = SeqWindow::from_db(0, &[2, 7]);
        w.advance_to(5);
        assert_eq!(w.to_db(), (5, vec![7]));
    }

    #[test]
    fn db_round_trip() {
        let w = SeqWindow::from_db(10, &[12, 15]);
        assert!(w.contains(9) && w.contains(12) && !w.contains(11));
        assert_eq!(w.to_db(), (10, vec![12, 15]));
    }
}
