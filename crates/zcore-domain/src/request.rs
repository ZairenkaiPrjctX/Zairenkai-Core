//! Action requests.

use serde::{Deserialize, Serialize};

use crate::principal::Principal;
use crate::resource::{FasProfile, ResourceId, ResourceValue};

/// The typed operations Core can perform. There is no free-form command operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    FasSetProfile { profile: FasProfile },
    FasSetEnabled { enabled: bool },
    SetLowPowerMode { enabled: bool },
}

/// Operation kinds, the unit of permission grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    FasSetProfile,
    FasSetEnabled,
    SetLowPowerMode,
}

impl OperationKind {
    pub const ALL: [OperationKind; 3] = [
        OperationKind::FasSetProfile,
        OperationKind::FasSetEnabled,
        OperationKind::SetLowPowerMode,
    ];
}

/// Effect on power and heat, used by thermal and battery policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Raises performance demand (more power, more heat).
    Raise,
    /// Lowers demand or restores a conservative default.
    Lower,
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskClass {
    Low,
    Medium,
    High,
}

impl Operation {
    pub fn kind(self) -> OperationKind {
        match self {
            Operation::FasSetProfile { .. } => OperationKind::FasSetProfile,
            Operation::FasSetEnabled { .. } => OperationKind::FasSetEnabled,
            Operation::SetLowPowerMode { .. } => OperationKind::SetLowPowerMode,
        }
    }

    pub fn resource(self) -> ResourceId {
        match self {
            Operation::FasSetProfile { .. } => ResourceId::FasProfile,
            Operation::FasSetEnabled { .. } => ResourceId::FasEnabled,
            Operation::SetLowPowerMode { .. } => ResourceId::LowPowerMode,
        }
    }

    pub fn target(self) -> ResourceValue {
        match self {
            Operation::FasSetProfile { profile } => ResourceValue::FasProfile(profile),
            Operation::FasSetEnabled { enabled } | Operation::SetLowPowerMode { enabled } => {
                ResourceValue::Bool(enabled)
            }
        }
    }

    /// Build the operation that sets `resource` to `value` (used for rollback).
    pub fn set(resource: ResourceId, value: ResourceValue) -> Option<Self> {
        Some(match (resource, value) {
            (ResourceId::FasProfile, ResourceValue::FasProfile(profile)) => {
                Operation::FasSetProfile { profile }
            }
            (ResourceId::FasEnabled, ResourceValue::Bool(enabled)) => {
                Operation::FasSetEnabled { enabled }
            }
            (ResourceId::LowPowerMode, ResourceValue::Bool(enabled)) => {
                Operation::SetLowPowerMode { enabled }
            }
            _ => return None,
        })
    }

    /// Direction relative to the current value (unknown current value: judged by target alone).
    pub fn direction(self, current: Option<ResourceValue>) -> Direction {
        match (self, current) {
            (Operation::FasSetProfile { profile }, Some(ResourceValue::FasProfile(cur))) => {
                match profile.intensity().cmp(&cur.intensity()) {
                    std::cmp::Ordering::Greater => Direction::Raise,
                    std::cmp::Ordering::Less => Direction::Lower,
                    std::cmp::Ordering::Equal => Direction::Neutral,
                }
            }
            (Operation::FasSetProfile { profile }, _) => {
                if profile.intensity() >= 2 {
                    Direction::Raise
                } else {
                    Direction::Lower
                }
            }
            // Enabling FAS allows boosting; disabling releases all boosts.
            (Operation::FasSetEnabled { enabled }, _) => {
                if enabled {
                    Direction::Raise
                } else {
                    Direction::Lower
                }
            }
            // Turning battery saver off raises demand.
            (Operation::SetLowPowerMode { enabled }, _) => {
                if enabled {
                    Direction::Lower
                } else {
                    Direction::Raise
                }
            }
        }
    }

    pub fn risk(self) -> RiskClass {
        match self {
            Operation::FasSetProfile { profile } if profile.intensity() >= 2 => RiskClass::Medium,
            Operation::FasSetEnabled { enabled: true } => RiskClass::Medium,
            Operation::SetLowPowerMode { enabled: false } => RiskClass::Medium,
            _ => RiskClass::Low,
        }
    }
}

/// Where a request originated.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Origin {
    /// A person (root CLI, WebUI, app UI).
    User,
    /// A recommendation from the Intelligence Engine, with its confidence 0..=1.
    Recommendation { confidence: f64 },
    /// Core itself (rollback, lease expiry, reconciliation).
    Core,
}

/// Correlation id, carried through plan, result and audit.
pub type RequestId = String;

pub const MAX_REASON_LEN: usize = 200;
pub const MAX_LEASE_MS: u64 = 24 * 3600 * 1000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionRequest {
    pub id: RequestId,
    pub principal: Principal,
    pub origin: Origin,
    pub operation: Operation,
    /// Temporary override: revert to the baseline after this many ms. `None` = until released.
    pub duration_ms: Option<u64>,
    /// Plan, evaluate and report, but do not execute.
    pub dry_run: bool,
    /// The user confirmed an action that a policy marked as needing confirmation.
    pub confirmed: bool,
    pub reason: String,
}

impl ActionRequest {
    /// Structural validation (independent of policy).
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err("request id must be 1..=64 chars of [A-Za-z0-9_-]".into());
        }
        if self.reason.len() > MAX_REASON_LEN || self.reason.chars().any(|c| c.is_control()) {
            return Err(format!(
                "reason must be <= {MAX_REASON_LEN} bytes without control characters"
            ));
        }
        if let Some(d) = self.duration_ms {
            if d == 0 || d > MAX_LEASE_MS {
                return Err(format!("duration_ms must be within 1..={MAX_LEASE_MS}"));
            }
        }
        if let Origin::Recommendation { confidence } = self.origin {
            if !(0.0..=1.0).contains(&confidence) {
                return Err("recommendation confidence must be within 0..=1".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> ActionRequest {
        ActionRequest {
            id: "r1".into(),
            principal: Principal::Root,
            origin: Origin::User,
            operation: Operation::FasSetProfile {
                profile: FasProfile::Performance,
            },
            duration_ms: None,
            dry_run: false,
            confirmed: false,
            reason: String::new(),
        }
    }

    #[test]
    fn validation() {
        assert!(req().validate().is_ok());
        let mut r = req();
        r.id = "a b".into();
        assert!(r.validate().is_err());
        let mut r = req();
        r.duration_ms = Some(0);
        assert!(r.validate().is_err());
        let mut r = req();
        r.reason = "x\ny".into();
        assert!(r.validate().is_err());
        let mut r = req();
        r.origin = Origin::Recommendation { confidence: 1.5 };
        assert!(r.validate().is_err());
    }

    #[test]
    fn directions() {
        let p = |p| Operation::FasSetProfile { profile: p };
        assert_eq!(
            p(FasProfile::Performance)
                .direction(Some(ResourceValue::FasProfile(FasProfile::Balanced))),
            Direction::Raise
        );
        assert_eq!(
            p(FasProfile::PowerSaver)
                .direction(Some(ResourceValue::FasProfile(FasProfile::Balanced))),
            Direction::Lower
        );
        assert_eq!(p(FasProfile::Performance).direction(None), Direction::Raise);
        assert_eq!(
            Operation::SetLowPowerMode { enabled: false }.direction(None),
            Direction::Raise
        );
        for r in ResourceId::ALL {
            let v = match r {
                ResourceId::FasProfile => ResourceValue::FasProfile(FasProfile::Balanced),
                _ => ResourceValue::Bool(true),
            };
            let op = Operation::set(r, v).unwrap();
            assert_eq!(op.resource(), r);
            assert_eq!(op.target(), v);
        }
        assert!(Operation::set(ResourceId::FasProfile, ResourceValue::Bool(true)).is_none());
    }
}
