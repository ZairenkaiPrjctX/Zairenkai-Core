//! Android battery saver adapter.
//!
//! Write: `cmd power set-mode <0|1>` (PowerManagerService shell command; sets the power save
//! mode through the framework, which records `Settings.Global.LOW_POWER_MODE`).
//! Read-back: `settings get global low_power`.
//! Both are fixed argv vectors through the allowlisted runner; only `0`/`1` are ever passed.

use std::sync::Arc;
use std::time::Duration;

use zcore_domain::capability::{CapabilityDescriptor, CapabilityStatus};
use zcore_domain::resource::{ResourceId, ResourceValue};
use zcore_engine::adapter::{Adapter, AdapterError, AdapterErrorCode};

use crate::exec::{ExecError, Runner};
use crate::telemetry::parse_low_power;

pub struct LowPowerAdapter {
    runner: Arc<dyn Runner + Sync>,
    /// Android API level, if known (`cmd power set-mode` is not assumed on unknown levels).
    api_level: Option<u32>,
}

fn map_exec(e: ExecError) -> AdapterError {
    match e {
        ExecError::Timeout => AdapterError::new(AdapterErrorCode::Timeout, "command timed out"),
        ExecError::NotAllowed(m) => AdapterError::new(AdapterErrorCode::Rejected, m),
        ExecError::Spawn(e) => AdapterError::new(AdapterErrorCode::Unavailable, e.to_string()),
        ExecError::Failed { stderr, .. } if stderr.contains("ermission") => {
            AdapterError::new(AdapterErrorCode::PermissionDenied, stderr)
        }
        ExecError::Failed { stderr, .. } => AdapterError::new(AdapterErrorCode::Rejected, stderr),
    }
}

/// Lowest API level for which `cmd power set-mode` is used.
pub const MIN_API: u32 = 29;

impl LowPowerAdapter {
    pub fn new(runner: Arc<dyn Runner + Sync>, api_level: Option<u32>) -> Self {
        Self { runner, api_level }
    }
}

impl Adapter for LowPowerAdapter {
    fn id(&self) -> &str {
        "android_power"
    }

    fn resources(&self) -> &[ResourceId] {
        &[ResourceId::LowPowerMode]
    }

    fn timeout_ms(&self) -> u64 {
        4_000
    }

    fn discover(&mut self, now_ms: u64) -> Vec<CapabilityDescriptor> {
        let (status, ev) = match self.api_level {
            None => (CapabilityStatus::Unknown, "Android API level unknown; battery saver control not assumed".to_string()),
            Some(api) if api < MIN_API => (CapabilityStatus::Unsupported, format!("API {api} < {MIN_API}")),
            Some(api) => match self.read(ResourceId::LowPowerMode) {
                Ok(v) => (CapabilityStatus::ReadWrite, format!("API {api}; settings get global low_power = {v}; write via cmd power set-mode")),
                Err(e) if e.code == AdapterErrorCode::PermissionDenied => (CapabilityStatus::PermissionRestricted, e.message),
                Err(e) => (CapabilityStatus::TemporarilyUnavailable, e.message),
            },
        };
        vec![CapabilityDescriptor::new(
            "android.low_power_mode",
            status,
            "android_power",
            vec![ev],
            now_ms,
        )]
    }

    fn read(&mut self, resource: ResourceId) -> Result<ResourceValue, AdapterError> {
        if resource != ResourceId::LowPowerMode {
            return Err(AdapterError::new(
                AdapterErrorCode::Unsupported,
                "not an Android power resource",
            ));
        }
        let out = self
            .runner
            .run(
                &["settings", "get", "global", "low_power"],
                Duration::from_secs(2),
            )
            .map_err(map_exec)?;
        parse_low_power(&out)
            .map(ResourceValue::Bool)
            .ok_or_else(|| {
                AdapterError::new(
                    AdapterErrorCode::Protocol,
                    format!("unexpected low_power value {:?}", out.trim()),
                )
            })
    }

    fn apply(&mut self, resource: ResourceId, value: ResourceValue) -> Result<(), AdapterError> {
        let mode = match (resource, value) {
            (ResourceId::LowPowerMode, ResourceValue::Bool(true)) => "1",
            (ResourceId::LowPowerMode, ResourceValue::Bool(false)) => "0",
            _ => {
                return Err(AdapterError::new(
                    AdapterErrorCode::Unsupported,
                    "value does not fit android.low_power_mode",
                ))
            }
        };
        self.runner
            .run(&["cmd", "power", "set-mode", mode], Duration::from_secs(3))
            .map(|_| ())
            .map_err(map_exec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::FakeRunner;

    #[test]
    fn apply_and_read_back() {
        let r = Arc::new(
            FakeRunner::default()
                .with("settings get global low_power", "0\n")
                .with("cmd power set-mode 1", ""),
        );
        let mut a = LowPowerAdapter::new(r.clone(), Some(34));
        assert_eq!(a.discover(0)[0].status, CapabilityStatus::ReadWrite);
        a.apply(ResourceId::LowPowerMode, ResourceValue::Bool(true))
            .unwrap();
        assert!(r.calls().contains(&"cmd power set-mode 1".to_string()));
        assert!(a
            .apply(
                ResourceId::LowPowerMode,
                ResourceValue::FasProfile(zcore_domain::resource::FasProfile::Balanced)
            )
            .is_err());
    }

    #[test]
    fn unknown_or_old_api_is_not_assumed() {
        let r: Arc<dyn Runner + Sync> =
            Arc::new(FakeRunner::default().with("settings get global low_power", "0\n"));
        assert_eq!(
            LowPowerAdapter::new(r.clone(), None).discover(0)[0].status,
            CapabilityStatus::Unknown
        );
        assert_eq!(
            LowPowerAdapter::new(r, Some(28)).discover(0)[0].status,
            CapabilityStatus::Unsupported
        );
        let failing = Arc::new(FakeRunner::default());
        failing.set("settings get global low_power", Err("timeout".into()));
        let mut a = LowPowerAdapter::new(failing, Some(34));
        assert_eq!(
            a.discover(0)[0].status,
            CapabilityStatus::TemporarilyUnavailable
        );
        assert_eq!(
            a.read(ResourceId::LowPowerMode).unwrap_err().code,
            AdapterErrorCode::Timeout
        );
    }
}
