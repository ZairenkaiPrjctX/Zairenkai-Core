// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Resource leases and conflict resolution.
//!
//! Rules (deterministic):
//! - One lease per resource. A request from the lease owner renews/replaces it.
//! - A request from another principal preempts only with a strictly higher-priority tier;
//!   otherwise it is rejected with the holder named. Core (safety tier) always preempts.
//! - The baseline is the value observed before the *first* lease, so stacked overrides restore
//!   the true original.
//! - Expired leases are returned to the caller for baseline restoration.

use serde::{Deserialize, Serialize};
use zcore_domain::decision::{PolicyTier, ResourceLease};
use zcore_domain::principal::Principal;
use zcore_domain::resource::{ResourceId, ResourceValue};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub holder: Principal,
    pub tier: PolicyTier,
    pub expires_at_ms: Option<u64>,
}

impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "resource is held by {} at tier {:?}",
            self.holder.label(),
            self.tier
        )?;
        if let Some(t) = self.expires_at_ms {
            write!(f, " until t={t} ms")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LeaseTable {
    leases: Vec<ResourceLease>,
    next_id: u64,
}

impl LeaseTable {
    pub fn all(&self) -> &[ResourceLease] {
        &self.leases
    }

    pub fn get(&self, r: ResourceId) -> Option<&ResourceLease> {
        self.leases.iter().find(|l| l.resource == r)
    }

    fn live(&self, r: ResourceId, now_ms: u64) -> Option<&ResourceLease> {
        self.get(r)
            .filter(|l| l.expires_at_ms.is_none_or(|e| e > now_ms))
    }

    /// May `owner` at `tier` change `r` now?
    pub fn check(
        &self,
        r: ResourceId,
        owner: &Principal,
        tier: PolicyTier,
        now_ms: u64,
    ) -> Result<(), Conflict> {
        match self.live(r, now_ms) {
            None => Ok(()),
            Some(l) if &l.owner == owner || *owner == Principal::Core || tier < l.tier => Ok(()),
            Some(l) => Err(Conflict {
                holder: l.owner.clone(),
                tier: l.tier,
                expires_at_ms: l.expires_at_ms,
            }),
        }
    }

    /// Record ownership after a verified change. `observed_before` becomes the baseline unless
    /// an earlier lease already captured one.
    #[allow(clippy::too_many_arguments)]
    pub fn grant(
        &mut self,
        r: ResourceId,
        owner: Principal,
        tier: PolicyTier,
        value: ResourceValue,
        observed_before: Option<ResourceValue>,
        now_ms: u64,
        duration_ms: Option<u64>,
        request_id: &str,
    ) -> u64 {
        let baseline = self.get(r).map(|l| l.baseline).unwrap_or(observed_before);
        self.leases.retain(|l| l.resource != r);
        self.next_id += 1;
        self.leases.push(ResourceLease {
            id: self.next_id,
            resource: r,
            owner,
            tier,
            value,
            baseline,
            granted_at_ms: now_ms,
            expires_at_ms: duration_ms.map(|d| now_ms.saturating_add(d)),
            request_id: request_id.into(),
        });
        self.next_id
    }

    pub fn remove(&mut self, id: u64) -> Option<ResourceLease> {
        let i = self.leases.iter().position(|l| l.id == id)?;
        Some(self.leases.remove(i))
    }

    pub fn expired(&self, now_ms: u64) -> Vec<ResourceLease> {
        self.leases
            .iter()
            .filter(|l| l.expires_at_ms.is_some_and(|e| e <= now_ms))
            .cloned()
            .collect()
    }

    /// After a restart, monotonic clocks restart too: rebase stored times onto `now_ms`
    /// conservatively (remaining durations are kept, never extended).
    pub fn rebase(&mut self, stored_now_ms: u64, now_ms: u64) {
        for l in &mut self.leases {
            if let Some(e) = l.expires_at_ms {
                let remaining = e.saturating_sub(stored_now_ms);
                l.expires_at_ms = Some(now_ms + remaining);
            }
            l.granted_at_ms = now_ms;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zcore_domain::resource::FasProfile;

    const R: ResourceId = ResourceId::FasProfile;
    fn v(p: FasProfile) -> ResourceValue {
        ResourceValue::FasProfile(p)
    }

    #[test]
    fn priority_and_ownership() {
        let mut t = LeaseTable::default();
        t.grant(
            R,
            Principal::Root,
            PolicyTier::User,
            v(FasProfile::Performance),
            Some(v(FasProfile::Balanced)),
            0,
            None,
            "a",
        );
        // A recommendation (lower priority) cannot override the user's choice.
        let e = t
            .check(R, &Principal::IntelligenceEngine, PolicyTier::Adaptive, 10)
            .unwrap_err();
        assert_eq!(e.holder, Principal::Root);
        // Same owner renews; Core always may (safety).
        assert!(t.check(R, &Principal::Root, PolicyTier::User, 10).is_ok());
        assert!(t.check(R, &Principal::Core, PolicyTier::Safety, 10).is_ok());
        // Equal tier, different owner: rejected (no silent last-writer-wins).
        assert!(t
            .check(R, &Principal::App(10234), PolicyTier::User, 10)
            .is_err());
    }

    #[test]
    fn baseline_survives_stacking_and_expiry() {
        let mut t = LeaseTable::default();
        t.grant(
            R,
            Principal::IntelligenceEngine,
            PolicyTier::Adaptive,
            v(FasProfile::FastResponse),
            Some(v(FasProfile::Balanced)),
            0,
            Some(1_000),
            "a",
        );
        // Higher tier preempts and the original baseline is kept.
        assert!(t.check(R, &Principal::Root, PolicyTier::User, 10).is_ok());
        t.grant(
            R,
            Principal::Root,
            PolicyTier::User,
            v(FasProfile::Performance),
            Some(v(FasProfile::FastResponse)),
            10,
            Some(5_000),
            "b",
        );
        assert_eq!(t.all().len(), 1);
        assert_eq!(t.get(R).unwrap().baseline, Some(v(FasProfile::Balanced)));
        assert!(t.expired(4_999).is_empty());
        assert_eq!(t.expired(5_010).len(), 1);
        // An expired lease no longer blocks others.
        assert!(t
            .check(
                R,
                &Principal::IntelligenceEngine,
                PolicyTier::Adaptive,
                6_000
            )
            .is_ok());
    }

    #[test]
    fn rebase_keeps_remaining_time() {
        let mut t = LeaseTable::default();
        t.grant(
            R,
            Principal::Root,
            PolicyTier::User,
            v(FasProfile::Performance),
            None,
            1_000,
            Some(10_000),
            "a",
        );
        t.rebase(4_000, 50);
        assert_eq!(t.get(R).unwrap().expires_at_ms, Some(50 + 7_000));
    }
}
