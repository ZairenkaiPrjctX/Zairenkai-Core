// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Capability descriptors: what can genuinely be observed or controlled on this device.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    /// Readable and writable through a validated adapter.
    ReadWrite,
    ReadOnly,
    /// Present but this process lacks permission.
    PermissionRestricted,
    /// Not present on this device / Android version.
    Unsupported,
    /// Present but currently not reachable (e.g. the owning daemon is not running).
    TemporarilyUnavailable,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    /// Dotted identifier, e.g. `fas.profile`, `android.low_power_mode`, `telemetry.battery`.
    pub id: String,
    pub status: CapabilityStatus,
    /// Component that owns the capability (adapter or module id).
    pub owner: String,
    /// What the status is based on (paths read, commands answered).
    pub evidence: Vec<String>,
    /// `true` only when the behaviour was confirmed on this device (e.g. a read-back matched).
    pub verified_on_device: bool,
    pub checked_at_ms: u64,
}

impl CapabilityDescriptor {
    pub fn new(
        id: &str,
        status: CapabilityStatus,
        owner: &str,
        evidence: Vec<String>,
        at_ms: u64,
    ) -> Self {
        Self {
            id: id.into(),
            status,
            owner: owner.into(),
            evidence,
            verified_on_device: false,
            checked_at_ms: at_ms,
        }
    }
}

/// Capability ids: lowercase dotted words.
pub fn valid_capability_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.split('.').all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn ids() {
        assert!(super::valid_capability_id("fas.profile"));
        assert!(!super::valid_capability_id("fas..profile"));
        assert!(!super::valid_capability_id("Fas.profile"));
        assert!(!super::valid_capability_id("a/b"));
    }
}
