//! Policy decisions, plans, results, leases and verification.

use serde::{Deserialize, Serialize};

use crate::principal::Principal;
use crate::request::{Operation, RequestId, RiskClass};
use crate::resource::{ResourceId, ResourceValue};

/// Policy priority tiers, highest first. A lower tier can never override a higher one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyTier {
    /// Hardware, kernel and OEM safety constraints (thermal, missing capability).
    Safety,
    /// System integrity and authorization.
    Integrity,
    /// Explicit user settings.
    User,
    /// Approved thermal and power policies.
    ThermalPower,
    /// Performance / graphics optimization.
    Performance,
    /// Adaptive recommendations.
    Adaptive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Allow,
    Deny,
    /// Allowed only if resubmitted with `confirmed = true` by a principal that may confirm.
    NeedsConfirmation,
}

/// One rule's contribution to a decision, for explanation and audit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleOutcome {
    pub rule: String,
    pub tier: PolicyTier,
    pub verdict: Verdict,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub verdict: Verdict,
    /// Tier at which the request is arbitrated (its priority against other owners).
    pub tier: PolicyTier,
    /// Every rule that matched, in evaluation order.
    pub rules: Vec<RuleOutcome>,
    pub policy_version: String,
}

impl PolicyDecision {
    pub fn denial_reasons(&self) -> Vec<String> {
        self.rules
            .iter()
            .filter(|r| r.verdict != Verdict::Allow)
            .map(|r| format!("{}: {}", r.rule, r.explanation))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationMethod {
    /// Read the resource back through its adapter and compare.
    ReadBack,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackPlan {
    /// Value to restore (the observed baseline before the change).
    pub restore: ResourceValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionPlan {
    pub request_id: RequestId,
    pub operation: Operation,
    pub resource: ResourceId,
    pub adapter: String,
    /// Conditions that held when the plan was made.
    pub preconditions: Vec<String>,
    /// Value observed before the change (`None` if it could not be read).
    pub current: Option<ResourceValue>,
    pub expected: ResourceValue,
    pub verification: VerificationMethod,
    pub timeout_ms: u64,
    pub rollback: Option<RollbackPlan>,
    pub risk: RiskClass,
    /// Nothing to do: the resource already has the expected value.
    pub no_op: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationResult {
    pub expected: ResourceValue,
    pub observed: Option<ResourceValue>,
    pub matched: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionOutcome {
    /// Executed and verified by read-back.
    Applied,
    /// Already in the requested state; nothing executed.
    AlreadyApplied,
    /// Evaluated and planned only.
    DryRun,
    /// Policy or arbitration said no; nothing executed.
    Rejected,
    /// The resource cannot be controlled on this device.
    Unsupported,
    /// The adapter did not answer in time; state may be unknown.
    TimedOut,
    /// Executed, but the read-back did not confirm it (and rollback was attempted).
    Unverified,
    /// Executed, verification failed, and the baseline was restored.
    RolledBack,
    /// The adapter reported an error.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionResult {
    pub request_id: RequestId,
    pub outcome: ActionOutcome,
    pub decision: Option<PolicyDecision>,
    pub plan: Option<ActionPlan>,
    pub verification: Option<VerificationResult>,
    pub lease_id: Option<u64>,
    pub message: String,
}

/// Ownership of a resource while Core holds it at a non-baseline value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLease {
    pub id: u64,
    pub resource: ResourceId,
    pub owner: Principal,
    pub tier: PolicyTier,
    /// Value Core applied and verified.
    pub value: ResourceValue,
    /// Value observed before Core's first change; restored on expiry or release.
    pub baseline: Option<ResourceValue>,
    pub granted_at_ms: u64,
    pub expires_at_ms: Option<u64>,
    pub request_id: RequestId,
}
