// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Controllable resources and their typed values.

use serde::{Deserialize, Serialize};

/// FAS profile names (contract with Zairenkai-FAS `zfasd`, schema 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FasProfile {
    PowerSaver,
    Balanced,
    Performance,
    FastResponse,
    Adaptive,
}

impl FasProfile {
    pub const ALL: [FasProfile; 5] = [
        FasProfile::PowerSaver,
        FasProfile::Balanced,
        FasProfile::Performance,
        FasProfile::FastResponse,
        FasProfile::Adaptive,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            FasProfile::PowerSaver => "power_saver",
            FasProfile::Balanced => "balanced",
            FasProfile::Performance => "performance",
            FasProfile::FastResponse => "fast_response",
            FasProfile::Adaptive => "adaptive",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == s)
    }

    /// Relative performance demand, used to decide whether a change raises power/heat.
    pub fn intensity(self) -> u8 {
        match self {
            FasProfile::PowerSaver => 0,
            FasProfile::Balanced | FasProfile::Adaptive => 1,
            FasProfile::FastResponse => 2,
            FasProfile::Performance => 3,
        }
    }
}

/// A resource Core arbitrates. Each has exactly one owning adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceId {
    /// Zairenkai-FAS default profile.
    FasProfile,
    /// Zairenkai-FAS master switch.
    FasEnabled,
    /// Android battery saver (`Settings.Global.LOW_POWER_MODE`).
    LowPowerMode,
}

impl ResourceId {
    pub const ALL: [ResourceId; 3] = [
        ResourceId::FasProfile,
        ResourceId::FasEnabled,
        ResourceId::LowPowerMode,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ResourceId::FasProfile => "fas.profile",
            ResourceId::FasEnabled => "fas.enabled",
            ResourceId::LowPowerMode => "android.low_power_mode",
        }
    }

    /// Capability that must be writable before this resource can be changed.
    pub fn capability(self) -> &'static str {
        self.as_str()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ResourceValue {
    FasProfile(FasProfile),
    Bool(bool),
}

impl std::fmt::Display for ResourceValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceValue::FasProfile(p) => f.write_str(p.as_str()),
            ResourceValue::Bool(b) => write!(f, "{b}"),
        }
    }
}

impl ResourceValue {
    /// Whether the value fits the resource's type.
    pub fn fits(self, r: ResourceId) -> bool {
        matches!(
            (r, self),
            (ResourceId::FasProfile, ResourceValue::FasProfile(_))
                | (ResourceId::FasEnabled, ResourceValue::Bool(_))
                | (ResourceId::LowPowerMode, ResourceValue::Bool(_))
        )
    }
}
