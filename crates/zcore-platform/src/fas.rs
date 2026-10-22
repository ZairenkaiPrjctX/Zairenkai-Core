// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Adapter for Zairenkai-FAS (`zfasd`), over its local control socket.
//!
//! Contract (Zairenkai-FAS `docs/INTEGRATION.md`, socket protocol 1):
//! - one request line per connection, one JSON line back;
//! - `status` returns the state snapshot, including `enabled` and `default_profile`
//!   (added in zfasd after 0.1.0; older daemons are reported as read-only);
//! - `profile <name>`, `enable`, `disable` persist to the FAS configuration and answer
//!   `{"ok":true,...}` or `{"ok":false,"error":...}`.
//!
//! Core never writes FAS's cpufreq nodes: FAS owns frequency control; Core owns the choice of
//! FAS profile and switch.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use zcore_domain::capability::{CapabilityDescriptor, CapabilityStatus};
use zcore_domain::device::FasState;
use zcore_domain::resource::{FasProfile, ResourceId, ResourceValue};
use zcore_engine::adapter::{Adapter, AdapterError, AdapterErrorCode};

pub const DEFAULT_SOCKET: &str = "/data/adb/zairenkai/fas/zfasd.sock";
/// zfasd inside the unified `zairenkai` module, relative to the module directory.
pub const MODULE_BINARY: &str = "bin/zfasd";

/// Path whose presence means zfasd is packaged in the installed `zairenkai` module.
pub fn module_marker(modules_dir: &str) -> String {
    format!(
        "{modules_dir}/{}/{MODULE_BINARY}",
        crate::modules::UNIFIED_ID
    )
}

pub struct FasAdapter {
    socket: PathBuf,
    module_installed: Box<dyn Fn() -> bool + Send>,
    timeout: Duration,
}

pub fn parse_status(json: &str) -> Result<FasState, AdapterError> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| {
        AdapterError::new(
            AdapterErrorCode::Protocol,
            format!("status is not JSON: {e}"),
        )
    })?;
    let field = |k: &str| {
        v.get(k).ok_or_else(|| {
            AdapterError::new(
                AdapterErrorCode::Unsupported,
                format!("FAS status lacks '{k}' (zfasd too old for Core control)"),
            )
        })
    };
    let enabled = field("enabled")?
        .as_bool()
        .ok_or_else(|| AdapterError::new(AdapterErrorCode::Protocol, "'enabled' is not a bool"))?;
    let profile = field("default_profile")?
        .as_str()
        .and_then(FasProfile::parse)
        .ok_or_else(|| AdapterError::new(AdapterErrorCode::Protocol, "unknown default_profile"))?;
    Ok(FasState {
        running: v.get("running").and_then(|x| x.as_bool()).unwrap_or(false),
        enabled,
        default_profile: profile,
        observe_only: v
            .get("observe_only")
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
    })
}

impl FasAdapter {
    pub fn new(socket: impl Into<PathBuf>, module_installed: Box<dyn Fn() -> bool + Send>) -> Self {
        Self {
            socket: socket.into(),
            module_installed,
            timeout: Duration::from_secs(3),
        }
    }

    fn request(&self, line: &str) -> Result<String, AdapterError> {
        let mut s = UnixStream::connect(&self.socket).map_err(|e| {
            let code = match e.kind() {
                std::io::ErrorKind::PermissionDenied => AdapterErrorCode::PermissionDenied,
                _ => AdapterErrorCode::Unavailable,
            };
            AdapterError::new(code, format!("{}: {e}", self.socket.display()))
        })?;
        s.set_read_timeout(Some(self.timeout)).ok();
        s.set_write_timeout(Some(self.timeout)).ok();
        s.write_all(format!("{line}\n").as_bytes())
            .map_err(|e| AdapterError::new(AdapterErrorCode::Io, e.to_string()))?;
        let mut out = String::new();
        BufReader::new(s.take(1 << 20))
            .read_line(&mut out)
            .map_err(|e| {
                let code = if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) {
                    AdapterErrorCode::Timeout
                } else {
                    AdapterErrorCode::Io
                };
                AdapterError::new(code, e.to_string())
            })?;
        if out.trim().is_empty() {
            return Err(AdapterError::new(AdapterErrorCode::Protocol, "empty reply"));
        }
        Ok(out)
    }

    pub fn state(&self) -> Result<FasState, AdapterError> {
        parse_status(&self.request("status")?)
    }
}

impl Adapter for FasAdapter {
    fn id(&self) -> &str {
        "fas"
    }

    fn resources(&self) -> &[ResourceId] {
        &[ResourceId::FasProfile, ResourceId::FasEnabled]
    }

    fn timeout_ms(&self) -> u64 {
        self.timeout.as_millis() as u64 * 2
    }

    fn discover(&mut self, now_ms: u64) -> Vec<CapabilityDescriptor> {
        let (status, evidence) = match self.state() {
            Ok(s) => (
                CapabilityStatus::ReadWrite,
                format!(
                    "zfasd answered status (running {}, observe_only {})",
                    s.running, s.observe_only
                ),
            ),
            Err(e) if e.code == AdapterErrorCode::Unsupported => {
                (CapabilityStatus::ReadOnly, e.message)
            }
            Err(e) if e.code == AdapterErrorCode::PermissionDenied => {
                (CapabilityStatus::PermissionRestricted, e.message)
            }
            Err(e) if (self.module_installed)() => (
                CapabilityStatus::TemporarilyUnavailable,
                format!(
                    "zfasd installed in module {} but unreachable: {}",
                    crate::modules::UNIFIED_ID,
                    e.message
                ),
            ),
            Err(_) => (
                CapabilityStatus::Unsupported,
                format!(
                    "zfasd not installed (module {} has no {MODULE_BINARY})",
                    crate::modules::UNIFIED_ID
                ),
            ),
        };
        ["fas.profile", "fas.enabled"]
            .iter()
            .map(|id| CapabilityDescriptor::new(id, status, "fas", vec![evidence.clone()], now_ms))
            .collect()
    }

    fn read(&mut self, resource: ResourceId) -> Result<ResourceValue, AdapterError> {
        let s = self.state()?;
        match resource {
            ResourceId::FasProfile => Ok(ResourceValue::FasProfile(s.default_profile)),
            ResourceId::FasEnabled => Ok(ResourceValue::Bool(s.enabled)),
            _ => Err(AdapterError::new(
                AdapterErrorCode::Unsupported,
                "not a FAS resource",
            )),
        }
    }

    fn apply(&mut self, resource: ResourceId, value: ResourceValue) -> Result<(), AdapterError> {
        let line = match (resource, value) {
            (ResourceId::FasProfile, ResourceValue::FasProfile(p)) => {
                format!("profile {}", p.as_str())
            }
            (ResourceId::FasEnabled, ResourceValue::Bool(true)) => "enable".to_string(),
            (ResourceId::FasEnabled, ResourceValue::Bool(false)) => "disable".to_string(),
            _ => {
                return Err(AdapterError::new(
                    AdapterErrorCode::Unsupported,
                    "value does not fit a FAS resource",
                ))
            }
        };
        let reply = self.request(&line)?;
        let v: serde_json::Value = serde_json::from_str(&reply)
            .map_err(|e| AdapterError::new(AdapterErrorCode::Protocol, e.to_string()))?;
        if v.get("ok").and_then(|o| o.as_bool()) == Some(true) {
            Ok(())
        } else {
            let msg = v
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("refused")
                .chars()
                .take(200)
                .collect::<String>();
            Err(AdapterError::new(AdapterErrorCode::Rejected, msg))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};

    /// A minimal stand-in for zfasd's socket that keeps `enabled` and `default_profile`.
    fn fake_zfasd(path: PathBuf, with_fields: bool) -> Arc<Mutex<(bool, String)>> {
        let state = Arc::new(Mutex::new((true, "balanced".to_string())));
        let st = state.clone();
        let l = UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            for conn in l.incoming() {
                let Ok(mut c) = conn else { break };
                let mut line = String::new();
                BufReader::new(&c).read_line(&mut line).unwrap();
                let mut s = st.lock().unwrap();
                let reply = match line.trim() {
                    "status" if with_fields => format!(
                        r#"{{"version":1,"running":true,"enabled":{},"default_profile":"{}","observe_only":false,"profile":"performance"}}"#,
                        s.0, s.1
                    ),
                    "status" => r#"{"version":1,"running":true,"profile":"balanced"}"#.to_string(),
                    "enable" => {
                        s.0 = true;
                        r#"{"ok":true}"#.into()
                    }
                    "disable" => {
                        s.0 = false;
                        r#"{"ok":true}"#.into()
                    }
                    l if l.starts_with("profile ") => {
                        s.1 = l[8..].to_string();
                        r#"{"ok":true}"#.into()
                    }
                    _ => r#"{"ok":false,"error":"unknown command"}"#.into(),
                };
                c.write_all(format!("{reply}\n").as_bytes()).unwrap();
            }
        });
        state
    }

    #[test]
    fn controls_and_reads_back_through_the_socket() {
        let d = crate::testutil::TempDir::new("fas");
        let sock = d.path().join("zfasd.sock");
        let st = fake_zfasd(sock.clone(), true);
        let mut a = FasAdapter::new(&sock, Box::new(|| true));
        assert!(a
            .discover(0)
            .iter()
            .all(|c| c.status == CapabilityStatus::ReadWrite));
        // The app-effective `profile` (performance) is not confused with the default.
        assert_eq!(
            a.read(ResourceId::FasProfile).unwrap(),
            ResourceValue::FasProfile(FasProfile::Balanced)
        );
        a.apply(
            ResourceId::FasProfile,
            ResourceValue::FasProfile(FasProfile::PowerSaver),
        )
        .unwrap();
        assert_eq!(st.lock().unwrap().1, "power_saver");
        a.apply(ResourceId::FasEnabled, ResourceValue::Bool(false))
            .unwrap();
        assert_eq!(
            a.read(ResourceId::FasEnabled).unwrap(),
            ResourceValue::Bool(false)
        );
        assert!(a
            .apply(
                ResourceId::FasEnabled,
                ResourceValue::FasProfile(FasProfile::Balanced)
            )
            .is_err());
    }

    #[test]
    fn old_or_absent_daemon_is_reported_honestly() {
        let d = crate::testutil::TempDir::new("fas2");
        let sock = d.path().join("zfasd.sock");
        fake_zfasd(sock.clone(), false);
        let mut a = FasAdapter::new(&sock, Box::new(|| true));
        assert!(a
            .discover(0)
            .iter()
            .all(|c| c.status == CapabilityStatus::ReadOnly));
        assert_eq!(
            a.read(ResourceId::FasEnabled).unwrap_err().code,
            AdapterErrorCode::Unsupported
        );
        let mut gone = FasAdapter::new(d.path().join("none.sock"), Box::new(|| true));
        assert!(gone
            .discover(0)
            .iter()
            .all(|c| c.status == CapabilityStatus::TemporarilyUnavailable));
        let mut absent = FasAdapter::new(d.path().join("none.sock"), Box::new(|| false));
        assert!(absent
            .discover(0)
            .iter()
            .all(|c| c.status == CapabilityStatus::Unsupported));
        assert_eq!(
            gone.read(ResourceId::FasProfile).unwrap_err().code,
            AdapterErrorCode::Unavailable
        );
    }
}
