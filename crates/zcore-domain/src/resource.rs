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

/// Block I/O schedulers Core may ask Zairenkai-Tuning to select (contract
/// `zairenkai.tuning` schema 1, operation `io.scheduler`). The device must also advertise it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum IoScheduler {
    #[serde(rename = "mq-deadline")]
    MqDeadline,
    #[serde(rename = "kyber")]
    Kyber,
    #[serde(rename = "bfq")]
    Bfq,
    #[serde(rename = "none")]
    None,
    #[serde(rename = "cfq")]
    Cfq,
    #[serde(rename = "deadline")]
    Deadline,
    #[serde(rename = "noop")]
    Noop,
}

impl IoScheduler {
    pub const ALL: [IoScheduler; 7] = [
        IoScheduler::MqDeadline,
        IoScheduler::Kyber,
        IoScheduler::Bfq,
        IoScheduler::None,
        IoScheduler::Cfq,
        IoScheduler::Deadline,
        IoScheduler::Noop,
    ];

    /// The kernel's name, as `queue/scheduler` prints it.
    pub fn as_str(self) -> &'static str {
        match self {
            IoScheduler::MqDeadline => "mq-deadline",
            IoScheduler::Kyber => "kyber",
            IoScheduler::Bfq => "bfq",
            IoScheduler::None => "none",
            IoScheduler::Cfq => "cfq",
            IoScheduler::Deadline => "deadline",
            IoScheduler::Noop => "noop",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// GPU devfreq governors Core may ask Zairenkai-Tuning to select (operation `gpu.governor`).
/// `userspace` is deliberately absent: it hands frequency control to a writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GpuGovernor {
    #[serde(rename = "msm-adreno-tz")]
    MsmAdrenoTz,
    #[serde(rename = "simple_ondemand")]
    SimpleOndemand,
    #[serde(rename = "performance")]
    Performance,
    #[serde(rename = "powersave")]
    Powersave,
}

impl GpuGovernor {
    pub const ALL: [GpuGovernor; 4] = [
        GpuGovernor::MsmAdrenoTz,
        GpuGovernor::SimpleOndemand,
        GpuGovernor::Performance,
        GpuGovernor::Powersave,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            GpuGovernor::MsmAdrenoTz => "msm-adreno-tz",
            GpuGovernor::SimpleOndemand => "simple_ondemand",
            GpuGovernor::Performance => "performance",
            GpuGovernor::Powersave => "powersave",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.as_str() == s)
    }

    /// Relative performance demand: `performance` pins the highest level, `powersave` the
    /// lowest, the load-following governors sit between.
    pub fn intensity(self) -> u8 {
        match self {
            GpuGovernor::Powersave => 0,
            GpuGovernor::MsmAdrenoTz | GpuGovernor::SimpleOndemand => 1,
            GpuGovernor::Performance => 2,
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
    /// I/O scheduler of the internal storage devices (Zairenkai-Tuning).
    IoScheduler,
    /// GPU devfreq governor (Zairenkai-Tuning).
    GpuGovernor,
}

impl ResourceId {
    pub const ALL: [ResourceId; 5] = [
        ResourceId::FasProfile,
        ResourceId::FasEnabled,
        ResourceId::LowPowerMode,
        ResourceId::IoScheduler,
        ResourceId::GpuGovernor,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ResourceId::FasProfile => "fas.profile",
            ResourceId::FasEnabled => "fas.enabled",
            ResourceId::LowPowerMode => "android.low_power_mode",
            ResourceId::IoScheduler => "tuning.io_scheduler",
            ResourceId::GpuGovernor => "tuning.gpu_governor",
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
    IoScheduler(IoScheduler),
    GpuGovernor(GpuGovernor),
}

impl std::fmt::Display for ResourceValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceValue::FasProfile(p) => f.write_str(p.as_str()),
            ResourceValue::Bool(b) => write!(f, "{b}"),
            ResourceValue::IoScheduler(v) => f.write_str(v.as_str()),
            ResourceValue::GpuGovernor(v) => f.write_str(v.as_str()),
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
                | (ResourceId::IoScheduler, ResourceValue::IoScheduler(_))
                | (ResourceId::GpuGovernor, ResourceValue::GpuGovernor(_))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuning_values_use_kernel_names() {
        for v in IoScheduler::ALL {
            assert_eq!(IoScheduler::parse(v.as_str()), Some(v));
            assert_eq!(serde_json::to_value(v).unwrap(), v.as_str());
        }
        for v in GpuGovernor::ALL {
            assert_eq!(GpuGovernor::parse(v.as_str()), Some(v));
            assert_eq!(serde_json::to_value(v).unwrap(), v.as_str());
        }
        assert_eq!(GpuGovernor::parse("userspace"), None);
        let v = ResourceValue::IoScheduler(IoScheduler::MqDeadline);
        assert!(v.fits(ResourceId::IoScheduler));
        assert!(!v.fits(ResourceId::GpuGovernor));
        assert_eq!(v.to_string(), "mq-deadline");
    }
}
