// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! The decision pipeline:
//! validate → observe current value → evaluate policy → arbitrate → plan → (dry run) →
//! execute through the owning adapter → verify by read-back → commit lease or roll back → audit.
//!
//! Plus reconciliation: leases whose resource was changed by someone else are dropped (never
//! re-forced), expired temporary leases restore their baseline.

use zcore_domain::audit::AuditKind;
use zcore_domain::capability::{CapabilityDescriptor, CapabilityStatus};
use zcore_domain::decision::{
    ActionOutcome, ActionPlan, ActionResult, PolicyTier, ResourceLease, RollbackPlan, Verdict,
    VerificationMethod, VerificationResult,
};
use zcore_domain::device::DeviceContext;
use zcore_domain::module::ModuleDescriptor;
use zcore_domain::principal::Principal;
use zcore_domain::request::{ActionRequest, Origin};
use zcore_domain::resource::{ResourceId, ResourceValue};

use crate::adapter::{Adapter, AdapterErrorCode};
use crate::arbitration::LeaseTable;
use crate::audit::AuditLog;
use crate::config::CoreConfig;
use crate::policy::{EvalInput, PolicyEngine};
use crate::registry::CapabilityRegistry;

pub struct Core {
    policy: PolicyEngine,
    registry: CapabilityRegistry,
    adapters: Vec<Box<dyn Adapter>>,
    leases: LeaseTable,
    context: DeviceContext,
    modules: Vec<ModuleDescriptor>,
    pub audit: AuditLog,
    leases_dirty: bool,
}

fn result(req_id: &str, outcome: ActionOutcome, message: impl Into<String>) -> ActionResult {
    ActionResult {
        request_id: req_id.into(),
        outcome,
        decision: None,
        plan: None,
        verification: None,
        lease_id: None,
        message: message.into(),
    }
}

impl Core {
    pub fn new(cfg: CoreConfig, adapters: Vec<Box<dyn Adapter>>) -> Self {
        Self {
            policy: PolicyEngine::new(cfg),
            registry: CapabilityRegistry::default(),
            adapters,
            leases: LeaseTable::default(),
            context: DeviceContext::unknown(),
            modules: vec![],
            audit: AuditLog::new(1_000),
            leases_dirty: false,
        }
    }

    pub fn policy(&self) -> &PolicyEngine {
        &self.policy
    }

    /// Swap the configuration. Leases and capabilities are kept.
    pub fn set_config(&mut self, cfg: CoreConfig) {
        self.policy = PolicyEngine::new(cfg);
    }

    pub fn context(&self) -> &DeviceContext {
        &self.context
    }

    pub fn set_context(&mut self, ctx: DeviceContext) {
        self.context = ctx;
    }

    pub fn modules(&self) -> &[ModuleDescriptor] {
        &self.modules
    }

    pub fn set_modules(&mut self, m: Vec<ModuleDescriptor>) {
        self.modules = m;
    }

    pub fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        self.registry.all().cloned().collect()
    }

    /// Add read-only capability descriptors from discovery (telemetry sources, etc.).
    pub fn report_capabilities(&mut self, owner: &str, d: Vec<CapabilityDescriptor>, now_ms: u64) {
        let changed = self.registry.update_owner(owner, d);
        if !changed.is_empty() {
            self.audit.record(
                now_ms,
                None,
                None,
                AuditKind::CapabilitiesRefreshed { changed },
            );
        }
    }

    pub fn refresh_capabilities(&mut self, now_ms: u64) {
        let mut all_changed = vec![];
        for a in self.adapters.iter_mut() {
            let d = a.discover(now_ms);
            let owner = a.id().to_string();
            all_changed.extend(self.registry.update_owner(&owner, d));
        }
        if !all_changed.is_empty() {
            self.audit.record(
                now_ms,
                None,
                None,
                AuditKind::CapabilitiesRefreshed {
                    changed: all_changed,
                },
            );
        }
    }

    pub fn leases(&self) -> &LeaseTable {
        &self.leases
    }

    /// Load leases persisted by a previous run (times rebased onto this run's clock).
    pub fn load_leases(&mut self, mut t: LeaseTable, stored_now_ms: u64, now_ms: u64) {
        t.rebase(stored_now_ms, now_ms);
        self.leases = t;
    }

    /// `true` once after the lease table changed (the daemon persists it).
    pub fn take_leases_dirty(&mut self) -> bool {
        std::mem::take(&mut self.leases_dirty)
    }

    fn adapter_for(&mut self, r: ResourceId) -> Option<&mut Box<dyn Adapter>> {
        self.adapters
            .iter_mut()
            .find(|a| a.resources().contains(&r))
    }

    fn read(&mut self, r: ResourceId) -> Result<ResourceValue, String> {
        match self.adapter_for(r) {
            Some(a) => a.read(r).map_err(|e| e.to_string()),
            None => Err(format!("no adapter owns {}", r.as_str())),
        }
    }

    /// Run the full pipeline for one request.
    pub fn submit(&mut self, req: ActionRequest, now_ms: u64) -> ActionResult {
        let id = req.id.clone();
        let r = req.operation.resource();
        let target = req.operation.target();
        if self.adapter_for(r).is_none() {
            let res = result(
                &id,
                ActionOutcome::Unsupported,
                format!("no adapter owns {}", r.as_str()),
            );
            self.audit.record(
                now_ms,
                Some(&id),
                Some(&req.principal),
                AuditKind::ActionCompleted {
                    resource: r,
                    outcome: res.outcome,
                    value: None,
                },
            );
            return res;
        }
        let current = self.read(r).ok();

        let decision = self.policy.evaluate(&EvalInput {
            request: &req,
            context: &self.context,
            registry: &self.registry,
            current,
            now_ms,
        });
        self.audit.record(
            now_ms,
            Some(&id),
            Some(&req.principal),
            AuditKind::RequestEvaluated {
                verdict: decision.verdict,
                reasons: decision.denial_reasons(),
            },
        );
        if decision.verdict != Verdict::Allow {
            let only_capability = decision
                .rules
                .iter()
                .filter(|o| o.verdict != Verdict::Allow)
                .all(|o| o.rule == "builtin.capability");
            let outcome = if only_capability {
                ActionOutcome::Unsupported
            } else {
                ActionOutcome::Rejected
            };
            let msg = if decision.verdict == Verdict::NeedsConfirmation {
                format!(
                    "confirmation required: {}",
                    decision.denial_reasons().join("; ")
                )
            } else {
                decision.denial_reasons().join("; ")
            };
            let mut res = result(&id, outcome, msg);
            res.decision = Some(decision);
            self.audit.record(
                now_ms,
                Some(&id),
                Some(&req.principal),
                AuditKind::ActionCompleted {
                    resource: r,
                    outcome,
                    value: None,
                },
            );
            return res;
        }

        let tier = decision.tier;
        let duration = match req.origin {
            Origin::Recommendation { .. } => {
                let max = self.policy.config().safety.recommendation_max_ms;
                Some(req.duration_ms.unwrap_or(max).min(max))
            }
            _ => req.duration_ms,
        };
        if let Err(c) = self.leases.check(r, &req.principal, tier, now_ms) {
            let mut res = result(&id, ActionOutcome::Rejected, format!("conflict: {c}"));
            res.decision = Some(decision);
            self.audit.record(
                now_ms,
                Some(&id),
                Some(&req.principal),
                AuditKind::ActionCompleted {
                    resource: r,
                    outcome: res.outcome,
                    value: None,
                },
            );
            return res;
        }

        // Without a readable current value the change can be neither verified nor rolled back.
        if current.is_none() && !req.dry_run {
            let mut res = result(&id, ActionOutcome::Rejected, format!("precondition failed: current value of {} is unreadable, so the change could not be verified or rolled back", r.as_str()));
            res.decision = Some(decision);
            self.audit.record(
                now_ms,
                Some(&id),
                Some(&req.principal),
                AuditKind::ActionCompleted {
                    resource: r,
                    outcome: res.outcome,
                    value: None,
                },
            );
            return res;
        }
        let (adapter_id, timeout_ms) = {
            let a = self.adapter_for(r).expect("checked above");
            (a.id().to_string(), a.timeout_ms())
        };
        let mut preconditions = vec![format!("{} is read-write", r.capability())];
        match current {
            Some(v) => preconditions.push(format!("{} currently {v}", r.as_str())),
            None => preconditions.push(format!("{} current value unreadable", r.as_str())),
        }
        let plan = ActionPlan {
            request_id: id.clone(),
            operation: req.operation,
            resource: r,
            adapter: adapter_id,
            preconditions,
            current,
            expected: target,
            verification: VerificationMethod::ReadBack,
            timeout_ms,
            rollback: current.map(|v| RollbackPlan { restore: v }),
            risk: req.operation.risk(),
            no_op: current == Some(target),
        };

        let mut res = result(&id, ActionOutcome::DryRun, "dry run: nothing executed");
        res.decision = Some(decision);
        if req.dry_run {
            res.plan = Some(plan);
            return res;
        }

        if plan.no_op {
            let lease = self.leases.grant(
                r,
                req.principal.clone(),
                tier,
                target,
                current,
                now_ms,
                duration,
                &id,
            );
            self.leases_dirty = true;
            res.outcome = ActionOutcome::AlreadyApplied;
            res.message = format!("{} already {target}; ownership recorded", r.as_str());
            res.lease_id = Some(lease);
            res.plan = Some(plan);
            self.audit.record(
                now_ms,
                Some(&id),
                Some(&req.principal),
                AuditKind::ActionCompleted {
                    resource: r,
                    outcome: res.outcome,
                    value: Some(target),
                },
            );
            return res;
        }

        let applied = self.adapter_for(r).expect("checked").apply(r, target);
        let readback = self.read(r);
        let verification = VerificationResult {
            expected: target,
            observed: readback.as_ref().ok().copied(),
            matched: readback.as_ref().ok() == Some(&target),
            detail: match &readback {
                Ok(v) => format!("read back {v}"),
                Err(e) => format!("read-back failed: {e}"),
            },
        };
        match (&applied, verification.matched) {
            (_, true) => {
                // Verified, even if the adapter reported a timeout first.
                let lease = self.leases.grant(
                    r,
                    req.principal.clone(),
                    tier,
                    target,
                    current,
                    now_ms,
                    duration,
                    &id,
                );
                self.leases_dirty = true;
                self.registry.mark_verified(r.capability());
                res.outcome = ActionOutcome::Applied;
                res.message = format!("{} set to {target} and verified", r.as_str());
                res.lease_id = Some(lease);
            }
            (Err(e), false) => {
                self.audit.record(
                    now_ms,
                    Some(&id),
                    None,
                    AuditKind::AdapterFault {
                        adapter: plan.adapter.clone(),
                        error: e.to_string(),
                    },
                );
                res.outcome = if e.code == AdapterErrorCode::Timeout {
                    ActionOutcome::TimedOut
                } else {
                    ActionOutcome::Failed
                };
                res.message = format!("adapter error: {e}");
                // If the value moved anyway (not to the target), try to put it back.
                if let (Some(cur), Some(obs)) = (current, verification.observed) {
                    if obs != cur && self.restore(r, cur, now_ms, &id) {
                        res.outcome = ActionOutcome::RolledBack;
                        res.message.push_str("; previous value restored");
                    }
                }
            }
            (Ok(()), false) => {
                res.message = format!("adapter reported success but {}", verification.detail);
                res.outcome = ActionOutcome::Unverified;
                if let Some(cur) = current {
                    if verification.observed != Some(cur) && self.restore(r, cur, now_ms, &id) {
                        res.outcome = ActionOutcome::RolledBack;
                        res.message.push_str("; previous value restored");
                    }
                }
            }
        }
        res.verification = Some(verification);
        res.plan = Some(plan);
        let value = (res.outcome == ActionOutcome::Applied).then_some(target);
        self.audit.record(
            now_ms,
            Some(&id),
            Some(&req.principal),
            AuditKind::ActionCompleted {
                resource: r,
                outcome: res.outcome,
                value,
            },
        );
        res
    }

    /// Core-originated set + read-back; used for rollback and baseline restore.
    fn restore(
        &mut self,
        r: ResourceId,
        value: ResourceValue,
        now_ms: u64,
        correlation: &str,
    ) -> bool {
        let ok = match self.adapter_for(r) {
            Some(a) => a.apply(r, value).is_ok() && a.read(r).ok() == Some(value),
            None => false,
        };
        self.audit.record(
            now_ms,
            Some(correlation),
            Some(&Principal::Core),
            AuditKind::BaselineRestored {
                resource: r,
                value,
                ok,
            },
        );
        ok
    }

    /// End a lease, restoring the baseline if the resource still holds the leased value.
    fn end_lease(&mut self, lease: &ResourceLease, now_ms: u64) -> Option<bool> {
        let observed = self.read(lease.resource).ok();
        if observed != Some(lease.value) {
            if let Some(obs) = observed {
                self.audit.record(
                    now_ms,
                    Some(&lease.request_id),
                    None,
                    AuditKind::ExternallyModified {
                        resource: lease.resource,
                        expected: lease.value,
                        observed: obs,
                    },
                );
            }
            return None;
        }
        let base = lease.baseline?;
        if base == lease.value {
            return Some(true);
        }
        Some(self.restore(lease.resource, base, now_ms, &lease.request_id))
    }

    /// Release a lease on behalf of `by` (its owner, root, or Core).
    pub fn release(
        &mut self,
        lease_id: u64,
        by: &Principal,
        now_ms: u64,
    ) -> Result<ActionResult, String> {
        let lease = self
            .leases
            .all()
            .iter()
            .find(|l| l.id == lease_id)
            .cloned()
            .ok_or_else(|| format!("no lease {lease_id}"))?;
        if &lease.owner != by && !matches!(by, Principal::Root | Principal::Core) {
            return Err(format!(
                "lease {lease_id} is owned by {}",
                lease.owner.label()
            ));
        }
        self.leases.remove(lease_id);
        self.leases_dirty = true;
        self.audit.record(
            now_ms,
            Some(&lease.request_id),
            Some(by),
            AuditKind::LeaseReleased {
                resource: lease.resource,
                lease_id,
            },
        );
        let restored = self.end_lease(&lease, now_ms);
        let (outcome, msg) = match restored {
            Some(true) => (
                ActionOutcome::Applied,
                format!("released; {} restored to baseline", lease.resource.as_str()),
            ),
            Some(false) => (
                ActionOutcome::Failed,
                "released; restoring the baseline failed".to_string(),
            ),
            None => (
                ActionOutcome::AlreadyApplied,
                "released; resource no longer held Core's value or had no baseline, left as is"
                    .to_string(),
            ),
        };
        Ok(result(&lease.request_id, outcome, msg))
    }

    /// Periodic reconciliation. Returns the number of leases ended.
    pub fn reconcile(&mut self, now_ms: u64) -> usize {
        let mut ended = 0;
        for lease in self.leases.all().to_vec() {
            match self.read(lease.resource) {
                Err(e) => {
                    self.audit.record(
                        now_ms,
                        Some(&lease.request_id),
                        None,
                        AuditKind::AdapterFault {
                            adapter: lease.resource.as_str().into(),
                            error: e,
                        },
                    );
                }
                Ok(obs) if obs != lease.value => {
                    // Someone else (user, OEM service, the owning daemon) changed it: their
                    // decision stands. Drop the lease instead of fighting.
                    self.audit.record(
                        now_ms,
                        Some(&lease.request_id),
                        None,
                        AuditKind::ExternallyModified {
                            resource: lease.resource,
                            expected: lease.value,
                            observed: obs,
                        },
                    );
                    self.leases.remove(lease.id);
                    self.leases_dirty = true;
                    ended += 1;
                }
                Ok(_) => {
                    if lease.expires_at_ms.is_some_and(|e| e <= now_ms) {
                        self.leases.remove(lease.id);
                        self.leases_dirty = true;
                        self.audit.record(
                            now_ms,
                            Some(&lease.request_id),
                            None,
                            AuditKind::LeaseExpired {
                                resource: lease.resource,
                                lease_id: lease.id,
                            },
                        );
                        self.end_lease(&lease, now_ms);
                        ended += 1;
                    }
                }
            }
        }
        ended
    }

    /// Before stopping: end temporary leases (restore baselines). Indefinite leases are user
    /// choices; they stay recorded and the resource keeps its value.
    pub fn shutdown(&mut self, now_ms: u64) {
        self.audit.record(now_ms, None, None, AuditKind::Stopping);
        for lease in self.leases.all().to_vec() {
            if lease.expires_at_ms.is_some() {
                self.leases.remove(lease.id);
                self.leases_dirty = true;
                self.end_lease(&lease, now_ms);
            }
        }
    }

    /// Status of every capability Core can control, for health reporting.
    pub fn controllable(&self) -> Vec<(ResourceId, CapabilityStatus)> {
        ResourceId::ALL
            .iter()
            .map(|r| (*r, self.registry.status(r.capability())))
            .collect()
    }

    pub fn tier_of_lease(&self, r: ResourceId) -> Option<PolicyTier> {
        self.leases.get(r).map(|l| l.tier)
    }
}
