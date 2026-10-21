// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Bounded in-memory audit log with a drain for persistent sinks.

use std::collections::VecDeque;

use zcore_domain::audit::{AuditEvent, AuditKind};
use zcore_domain::principal::Principal;

#[derive(Debug)]
pub struct AuditLog {
    ring: VecDeque<AuditEvent>,
    pending: Vec<AuditEvent>,
    capacity: usize,
    seq: u64,
    pub dropped_pending: u64,
}

impl AuditLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            ring: VecDeque::with_capacity(capacity),
            pending: Vec::new(),
            capacity: capacity.max(1),
            seq: 0,
            dropped_pending: 0,
        }
    }

    pub fn record(
        &mut self,
        at_ms: u64,
        correlation_id: Option<&str>,
        principal: Option<&Principal>,
        event: AuditKind,
    ) -> u64 {
        self.seq += 1;
        let e = AuditEvent {
            seq: self.seq,
            at_ms,
            correlation_id: correlation_id.map(str::to_string),
            principal: principal.cloned(),
            event,
        };
        if self.ring.len() == self.capacity {
            self.ring.pop_front();
        }
        self.ring.push_back(e.clone());
        // The pending buffer is bounded too: a stuck sink must not grow memory without limit.
        if self.pending.len() >= self.capacity {
            self.pending.remove(0);
            self.dropped_pending += 1;
        }
        self.pending.push(e);
        self.seq
    }

    pub fn tail(&self, n: usize) -> Vec<AuditEvent> {
        self.ring.iter().rev().take(n).rev().cloned().collect()
    }

    pub fn drain_pending(&mut self) -> Vec<AuditEvent> {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded() {
        let mut a = AuditLog::new(3);
        for i in 0..5 {
            a.record(i, None, None, AuditKind::Stopping);
        }
        assert_eq!(
            a.tail(10).iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
        assert_eq!(a.drain_pending().len(), 3);
        assert_eq!(a.dropped_pending, 2);
        assert!(a.drain_pending().is_empty());
    }
}
