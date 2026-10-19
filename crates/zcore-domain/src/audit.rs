//! Audit events: what Core decided and did, with correlation ids. No secrets, no app data
//! beyond package/uid identifiers needed to explain a decision.

use serde::{Deserialize, Serialize};

use crate::decision::ActionOutcome;
use crate::principal::Principal;
use crate::resource::{ResourceId, ResourceValue};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuditKind {
    Started {
        version: String,
        policy_version: String,
    },
    Stopping,
    RequestEvaluated {
        verdict: crate::decision::Verdict,
        reasons: Vec<String>,
    },
    ActionCompleted {
        resource: ResourceId,
        outcome: ActionOutcome,
        value: Option<ResourceValue>,
    },
    LeaseExpired {
        resource: ResourceId,
        lease_id: u64,
    },
    LeaseReleased {
        resource: ResourceId,
        lease_id: u64,
    },
    ExternallyModified {
        resource: ResourceId,
        expected: ResourceValue,
        observed: ResourceValue,
    },
    BaselineRestored {
        resource: ResourceId,
        value: ResourceValue,
        ok: bool,
    },
    CapabilitiesRefreshed {
        changed: Vec<String>,
    },
    ConfigRejected {
        error: String,
    },
    AdapterFault {
        adapter: String,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub seq: u64,
    pub at_ms: u64,
    pub correlation_id: Option<String>,
    pub principal: Option<Principal>,
    pub event: AuditKind,
}
