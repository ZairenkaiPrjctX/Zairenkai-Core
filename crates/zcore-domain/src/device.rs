// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Device context: identity and a telemetry snapshot. Every field is an [`Observation`].

use serde::{Deserialize, Serialize};

use crate::observation::Observation;
use crate::resource::FasProfile;
use crate::units::{Kib, MilliCelsius, Percent};

/// Android thermal status (`PowerManager.THERMAL_STATUS_*`, vendor-defined thresholds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThermalStatus {
    None,
    Light,
    Moderate,
    Severe,
    Critical,
    Emergency,
    Shutdown,
}

impl ThermalStatus {
    pub fn from_code(c: i64) -> Option<Self> {
        Some(match c {
            0 => Self::None,
            1 => Self::Light,
            2 => Self::Moderate,
            3 => Self::Severe,
            4 => Self::Critical,
            5 => Self::Emergency,
            6 => Self::Shutdown,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootManager {
    KernelSu,
    Magisk,
    APatch,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelinuxMode {
    Enforcing,
    Permissive,
    Disabled,
}

/// Stable identity facts (re-read rarely).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub api_level: Observation<u32>,
    pub brand: Observation<String>,
    pub model: Observation<String>,
    /// `ro.product.device` codename.
    pub device: Observation<String>,
    /// `ro.soc.manufacturer` / `ro.soc.model` / `ro.board.platform`, as found.
    pub soc: Observation<String>,
    pub abi: Observation<String>,
    pub kernel_release: Observation<String>,
    pub root_manager: Observation<RootManager>,
    pub selinux: Observation<SelinuxMode>,
}

impl DeviceIdentity {
    pub fn unknown() -> Self {
        Self {
            api_level: Observation::unknown(),
            brand: Observation::unknown(),
            model: Observation::unknown(),
            device: Observation::unknown(),
            soc: Observation::unknown(),
            abi: Observation::unknown(),
            kernel_release: Observation::unknown(),
            root_manager: Observation::unknown(),
            selinux: Observation::unknown(),
        }
    }
}

/// State reported by the Zairenkai-FAS daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FasState {
    pub running: bool,
    pub enabled: bool,
    pub default_profile: FasProfile,
    pub observe_only: bool,
}

/// Frequently refreshed telemetry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetrySnapshot {
    pub battery_level: Observation<Percent>,
    pub charging: Observation<bool>,
    pub battery_temp: Observation<MilliCelsius>,
    pub thermal_status: Observation<ThermalStatus>,
    pub mem_available: Observation<Kib>,
    pub low_power_mode: Observation<bool>,
    pub fas: Observation<FasState>,
}

impl TelemetrySnapshot {
    pub fn unknown() -> Self {
        Self {
            battery_level: Observation::unknown(),
            charging: Observation::unknown(),
            battery_temp: Observation::unknown(),
            thermal_status: Observation::unknown(),
            mem_available: Observation::unknown(),
            low_power_mode: Observation::unknown(),
            fas: Observation::unknown(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceContext {
    pub identity: DeviceIdentity,
    pub telemetry: TelemetrySnapshot,
    /// When this snapshot was assembled (monotonic ms).
    pub collected_at_ms: u64,
}

impl DeviceContext {
    pub fn unknown() -> Self {
        Self {
            identity: DeviceIdentity::unknown(),
            telemetry: TelemetrySnapshot::unknown(),
            collected_at_ms: 0,
        }
    }
}
