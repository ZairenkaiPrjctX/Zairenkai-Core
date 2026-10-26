// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Zairenkai-Tuning: I/O scheduler and GPU governor control, and telemetry, through `ztune`.
//!
//! `ztune` is a separate GPL-3.0 program packaged in the unified module. Core runs it as a
//! child process with a fixed argument vector (no shell) and reads one JSON document from
//! stdout: the `zairenkai.tuning` schema 1 contract (Zairenkai-Tuning `docs/CONTRACT.md`).
//! This file is written from that contract only; it contains no code from Zairenkai-Tuning
//! or its upstreams, and nothing here links to them (Zairenkai-Tuning `docs/LICENSING.md`).
//!
//! The adapter:
//! - runs only `capabilities`, `telemetry` and `apply <op> <value>`, with typed values from
//!   [`IoScheduler`] and [`GpuGovernor`]; `restore`, `gpu-table` and anything else are never
//!   run by Core;
//! - runs only the module-managed binary after a path-specific ownership/permission check;
//! - bounds time and output, and checks the schema, version, kind and component;
//! - reads the resource back through `capabilities` so Core verifies independently of the
//!   result `ztune` reports.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use zcore_domain::capability::{CapabilityDescriptor, CapabilityStatus};
use zcore_domain::resource::{GpuGovernor, IoScheduler, ResourceId, ResourceValue};
use zcore_engine::adapter::{Adapter, AdapterError, AdapterErrorCode};

use crate::component::{self, BinaryError};
use crate::exec::{ExecError, Runner, MAX_OUTPUT};
use crate::sysroot::SysRoot;

/// Configuration key in `[paths]`.
pub const PATH_KEY: &str = "tuning_binary";
/// Where the unified module installs it.
pub const DEFAULT_BINARY: &str = "/data/adb/modules/zairenkai/bin/ztune";
pub const BINARY_NAME: &str = "ztune";
pub const OWNER: &str = "tuning";
pub const SCHEMA: &str = "zairenkai.tuning";
pub const SUPPORTED_SCHEMA: u64 = 1;
pub const COMPONENT: &str = "zairenkai-tuning";
/// Runs on the engine thread. A fixture run takes milliseconds on a host; device timing is
/// unmeasured.
pub const TIMEOUT: Duration = Duration::from_secs(5);
/// Interval between the two `/proc/stat` samples `ztune telemetry` takes.
pub const TELEMETRY_INTERVAL_MS: &str = "200";

const RESOURCES: [ResourceId; 2] = [ResourceId::IoScheduler, ResourceId::GpuGovernor];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum TuningError {
    NotInstalled(String),
    UnsafeBinary(String),
    Timeout,
    /// `ztune` exited non-zero; `code` is the contract's error code from its stderr line.
    Command {
        code: String,
        message: String,
    },
    Failed(String),
    OutputTooLarge,
    Malformed(String),
    UnsupportedSchema(String),
}

impl std::fmt::Display for TuningError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInstalled(d) => write!(f, "not installed: {d}"),
            Self::UnsafeBinary(d) => write!(f, "refused to run: {d}"),
            Self::Timeout => write!(f, "timed out"),
            Self::Command { code, message } => write!(f, "ztune {code}: {message}"),
            Self::Failed(d) => write!(f, "failed: {d}"),
            Self::OutputTooLarge => write!(f, "output larger than {MAX_OUTPUT} bytes"),
            Self::Malformed(d) => write!(f, "malformed output: {d}"),
            Self::UnsupportedSchema(d) => write!(f, "unsupported contract: {d}"),
        }
    }
}

/// A capability as `ztune` reports it (its vocabulary, kept verbatim).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolCapability {
    pub id: String,
    pub state: String,
    pub allowed_values: Vec<String>,
    pub current: Option<String>,
    pub evidence: Vec<String>,
    /// `authorization.write_authorized`: true only when an exact-device, device_validated
    /// profile grants the control. Absent means false.
    pub write_authorized: bool,
    /// `authorization.granted_values`.
    pub granted_values: Vec<String>,
    pub tier: Option<String>,
    pub profile_id: Option<String>,
    pub authorization_reason: Option<String>,
}

/// The contract operation that controls `r`.
pub fn operation_name(r: ResourceId) -> Option<&'static str> {
    match r {
        ResourceId::IoScheduler => Some("io.scheduler"),
        ResourceId::GpuGovernor => Some("gpu.governor"),
        _ => None,
    }
}

fn parse_value(r: ResourceId, s: &str) -> Option<ResourceValue> {
    match r {
        ResourceId::IoScheduler => IoScheduler::parse(s).map(ResourceValue::IoScheduler),
        ResourceId::GpuGovernor => GpuGovernor::parse(s).map(ResourceValue::GpuGovernor),
        _ => None,
    }
}

fn value_name(v: ResourceValue) -> Option<&'static str> {
    match v {
        ResourceValue::IoScheduler(s) => Some(s.as_str()),
        ResourceValue::GpuGovernor(g) => Some(g.as_str()),
        _ => None,
    }
}

/// Checks the envelope of a `ztune` document and returns it.
pub fn parse_document(text: &str, kind: &str) -> Result<Value, TuningError> {
    let v: Value = serde_json::from_str(text).map_err(|e| TuningError::Malformed(e.to_string()))?;
    if v.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err(TuningError::UnsupportedSchema(format!(
            "schema {:?}",
            v.get("schema")
        )));
    }
    if v.get("schema_version").and_then(Value::as_u64) != Some(SUPPORTED_SCHEMA) {
        return Err(TuningError::UnsupportedSchema(format!(
            "schema_version {:?} (expected {SUPPORTED_SCHEMA})",
            v.get("schema_version")
        )));
    }
    if v.pointer("/component/name").and_then(Value::as_str) != Some(COMPONENT) {
        return Err(TuningError::Malformed(
            "component is not zairenkai-tuning".into(),
        ));
    }
    if v.get("kind").and_then(Value::as_str) != Some(kind) {
        return Err(TuningError::Malformed(format!(
            "kind {:?}, expected {kind:?}",
            v.get("kind")
        )));
    }
    Ok(v)
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(|s| s.chars().take(200).collect())
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_capabilities(text: &str) -> Result<Vec<ToolCapability>, TuningError> {
    let v = parse_document(text, "capabilities")?;
    let caps = v
        .get("capabilities")
        .and_then(Value::as_array)
        .ok_or_else(|| TuningError::Malformed("capabilities is not an array".into()))?;
    caps.iter()
        .map(|c| {
            let id = c.get("id").and_then(Value::as_str);
            let state = c.get("state").and_then(Value::as_str);
            match (id, state) {
                (Some(id), Some(state)) => {
                    let auth = c.get("authorization");
                    let text = |k: &str| {
                        auth.and_then(|a| a.get(k))
                            .and_then(Value::as_str)
                            .map(|s| s.chars().take(500).collect::<String>())
                    };
                    Ok(ToolCapability {
                        id: id.into(),
                        state: state.into(),
                        allowed_values: strings(c.get("allowed_values")),
                        current: c.get("current").and_then(Value::as_str).map(str::to_string),
                        evidence: strings(c.get("evidence")),
                        write_authorized: auth
                            .and_then(|a| a.get("write_authorized"))
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        granted_values: strings(auth.and_then(|a| a.get("granted_values"))),
                        tier: text("tier"),
                        profile_id: text("profile_id"),
                        authorization_reason: text("reason"),
                    })
                }
                _ => Err(TuningError::Malformed(
                    "capability without id or state".into(),
                )),
            }
        })
        .collect()
}

/// Splits `ztune: <code>: <message>` (the stderr line of a non-zero exit).
fn command_error(stderr: &str, status: Option<i32>) -> TuningError {
    let line = stderr
        .lines()
        .rev()
        .find(|l| l.starts_with("ztune: "))
        .unwrap_or("");
    match line
        .strip_prefix("ztune: ")
        .and_then(|r| r.split_once(": "))
    {
        Some((code, msg)) if code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') => {
            TuningError::Command {
                code: code.into(),
                message: msg.chars().take(300).collect(),
            }
        }
        _ => TuningError::Failed(format!("exit status {status:?}: {stderr}")),
    }
}

pub struct TuningAdapter {
    root: SysRoot,
    runner: Arc<dyn Runner + Sync>,
    binary: String,
    owner_uid: u32,
}

impl TuningAdapter {
    pub fn new(root: SysRoot, runner: Arc<dyn Runner + Sync>, binary: Option<&str>) -> Self {
        Self::owned_by(root, runner, binary, 0)
    }

    /// [`TuningAdapter::new`] with an explicit owner uid for the binary (tests).
    pub fn owned_by(
        root: SysRoot,
        runner: Arc<dyn Runner + Sync>,
        binary: Option<&str>,
        owner_uid: u32,
    ) -> Self {
        Self {
            root,
            runner,
            binary: binary
                .filter(|b| !b.is_empty())
                .unwrap_or(DEFAULT_BINARY)
                .to_string(),
            owner_uid,
        }
    }

    pub fn binary(&self) -> &str {
        &self.binary
    }

    fn run(&self, args: &[&str], kind: &str) -> Result<Value, TuningError> {
        component::check_binary(&self.root, &self.binary, BINARY_NAME, self.owner_uid).map_err(
            |e| match e {
                BinaryError::NotInstalled(m) => TuningError::NotInstalled(m),
                BinaryError::Unsafe(m) => TuningError::UnsafeBinary(m),
            },
        )?;
        let mut argv = vec![self.binary.as_str()];
        argv.extend_from_slice(args);
        let out = self.runner.run(&argv, TIMEOUT).map_err(|e| match e {
            ExecError::Timeout => TuningError::Timeout,
            ExecError::Spawn(e) if e.kind() == std::io::ErrorKind::NotFound => {
                TuningError::NotInstalled(e.to_string())
            }
            ExecError::NotAllowed(m) => TuningError::UnsafeBinary(m),
            ExecError::Failed { status, stderr } => command_error(&stderr, status),
            other => TuningError::Failed(other.to_string()),
        })?;
        if out.len() >= MAX_OUTPUT {
            return Err(TuningError::OutputTooLarge);
        }
        parse_document(&out, kind)
    }

    pub fn capabilities(&self) -> Result<Vec<ToolCapability>, TuningError> {
        let v = self.run(&["capabilities"], "capabilities")?;
        parse_capabilities(&v.to_string())
    }

    /// The `ztune telemetry` document, validated, for Core's `tuning` method.
    pub fn telemetry(&self) -> Result<Value, TuningError> {
        self.run(
            &["telemetry", "--interval-ms", TELEMETRY_INTERVAL_MS],
            "telemetry",
        )
    }

    fn tool_cap(caps: &[ToolCapability], r: ResourceId) -> Option<&ToolCapability> {
        let op = operation_name(r)?;
        caps.iter().find(|c| c.id == op)
    }
}

fn map_err(e: TuningError) -> AdapterError {
    let code = match &e {
        TuningError::NotInstalled(_) => AdapterErrorCode::Unavailable,
        TuningError::UnsafeBinary(_) => AdapterErrorCode::PermissionDenied,
        TuningError::Timeout => AdapterErrorCode::Timeout,
        TuningError::Command { code, .. } => match code.as_str() {
            "invalid_value"
            | "refused"
            | "device_mismatch"
            | "confirmation_required"
            | "not_authorized" => AdapterErrorCode::Rejected,
            "unsupported" | "not_implemented" => AdapterErrorCode::Unsupported,
            "permission_denied" => AdapterErrorCode::PermissionDenied,
            "busy" => AdapterErrorCode::Unavailable,
            _ => AdapterErrorCode::Io,
        },
        TuningError::Failed(_) => AdapterErrorCode::Io,
        TuningError::OutputTooLarge
        | TuningError::Malformed(_)
        | TuningError::UnsupportedSchema(_) => AdapterErrorCode::Protocol,
    };
    AdapterError::new(code, e.to_string())
}

impl Adapter for TuningAdapter {
    fn id(&self) -> &str {
        OWNER
    }

    fn resources(&self) -> &[ResourceId] {
        &RESOURCES
    }

    fn timeout_ms(&self) -> u64 {
        TIMEOUT.as_millis() as u64
    }

    fn discover(&mut self, now_ms: u64) -> Vec<CapabilityDescriptor> {
        let caps = match self.capabilities() {
            Ok(c) => c,
            Err(e) => {
                let status = match e {
                    TuningError::NotInstalled(_) => CapabilityStatus::Unsupported,
                    TuningError::UnsafeBinary(_) => CapabilityStatus::PermissionRestricted,
                    TuningError::Timeout | TuningError::Failed(_) | TuningError::Command { .. } => {
                        CapabilityStatus::TemporarilyUnavailable
                    }
                    _ => CapabilityStatus::Unknown,
                };
                return RESOURCES
                    .iter()
                    .map(|r| r.capability())
                    .chain(["tuning.telemetry"])
                    .map(|id| {
                        CapabilityDescriptor::new(
                            id,
                            status,
                            OWNER,
                            vec![self.binary.clone(), e.to_string()],
                            now_ms,
                        )
                    })
                    .collect();
            }
        };
        let mut out = Vec::new();
        for r in RESOURCES {
            let Some(c) = Self::tool_cap(&caps, r) else {
                out.push(CapabilityDescriptor::new(
                    r.capability(),
                    CapabilityStatus::Unknown,
                    OWNER,
                    vec![format!(
                        "ztune reports no {:?} capability",
                        operation_name(r)
                    )],
                    now_ms,
                ));
                continue;
            };
            // Writable only with an exact-device grant, and only for values Core models.
            let accepted: Vec<&str> = c
                .granted_values
                .iter()
                .filter(|v| c.allowed_values.contains(v) && parse_value(r, v).is_some())
                .map(String::as_str)
                .collect();
            let status = match c.state.as_str() {
                "not_verified" | "verified" if c.write_authorized && !accepted.is_empty() => {
                    CapabilityStatus::ReadWrite
                }
                // The node is there and readable, but no profile authorizes writing it.
                "not_verified" | "verified" => CapabilityStatus::ReadOnly,
                "unsupported" => CapabilityStatus::Unsupported,
                "unavailable" => CapabilityStatus::TemporarilyUnavailable,
                _ => CapabilityStatus::Unknown,
            };
            let mut evidence = vec![
                format!("{} capabilities: {} is {}", self.binary, c.id, c.state),
                format!(
                    "write authorized: {} (tier {}, profile {})",
                    c.write_authorized,
                    c.tier.as_deref().unwrap_or("?"),
                    c.profile_id.as_deref().unwrap_or("none")
                ),
                format!("device advertises: {}", c.allowed_values.join(" ")),
                format!("Core accepts: {}", accepted.join(" ")),
            ];
            if let Some(reason) = &c.authorization_reason {
                evidence.push(reason.clone());
            }
            evidence.extend(c.evidence.iter().take(8).cloned());
            let mut d = CapabilityDescriptor::new(r.capability(), status, OWNER, evidence, now_ms);
            d.verified_on_device = c.state == "verified" && status == CapabilityStatus::ReadWrite;
            out.push(d);
        }
        let telemetry: Vec<&ToolCapability> = caps
            .iter()
            .filter(|c| c.id.starts_with("telemetry.") && c.state == "available")
            .collect();
        out.push(CapabilityDescriptor::new(
            "tuning.telemetry",
            if telemetry.is_empty() {
                CapabilityStatus::Unsupported
            } else {
                CapabilityStatus::ReadOnly
            },
            OWNER,
            vec![format!(
                "available: {}",
                telemetry
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            )],
            now_ms,
        ));
        out
    }

    fn read(&mut self, resource: ResourceId) -> Result<ResourceValue, AdapterError> {
        if operation_name(resource).is_none() {
            return Err(AdapterError::new(
                AdapterErrorCode::Unsupported,
                "not a tuning resource",
            ));
        }
        let caps = self.capabilities().map_err(map_err)?;
        let c = Self::tool_cap(&caps, resource).ok_or_else(|| {
            AdapterError::new(
                AdapterErrorCode::Unsupported,
                "ztune reports no such capability",
            )
        })?;
        let cur = c.current.as_deref().ok_or_else(|| {
            AdapterError::new(
                AdapterErrorCode::Unavailable,
                format!("{} has no current value ({})", c.id, c.state),
            )
        })?;
        parse_value(resource, cur).ok_or_else(|| {
            AdapterError::new(
                AdapterErrorCode::Protocol,
                format!("{} is {cur:?}, which Core does not model", c.id),
            )
        })
    }

    fn apply(&mut self, resource: ResourceId, value: ResourceValue) -> Result<(), AdapterError> {
        let (Some(op), Some(name)) = (operation_name(resource), value_name(value)) else {
            return Err(AdapterError::new(
                AdapterErrorCode::Unsupported,
                "not a tuning resource or value",
            ));
        };
        if !value.fits(resource) {
            return Err(AdapterError::new(
                AdapterErrorCode::Unsupported,
                "value does not fit resource",
            ));
        }
        let v = self
            .run(&["apply", op, name], "operation")
            .map_err(map_err)?;
        let request = v.pointer("/request/status").and_then(Value::as_str);
        let status = v.pointer("/operation/status").and_then(Value::as_str);
        match (request, status) {
            (Some("accepted"), Some("applied_verified" | "no_change")) => Ok(()),
            _ => Err(AdapterError::new(
                AdapterErrorCode::Protocol,
                format!("unexpected result: request {request:?}, operation {status:?}"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::FakeRunner;
    use crate::testutil::TempDir;
    use std::os::unix::fs::PermissionsExt;

    const BIN: &str = "/data/adb/modules/zairenkai/bin/ztune";

    /// `ztune capabilities` on Zairenkai-Tuning's synthetic fixture without profiles, captured
    /// with the real binary (`tests/fixtures/tuning/SOURCE`): what every device gets today.
    const CAPS: &str = include_str!("../tests/fixtures/tuning/capabilities.json");
    /// The same with Tuning's SYNTHETIC device_validated test profile.
    const CAPS_GRANTED: &str =
        include_str!("../tests/fixtures/tuning/capabilities-synthetic-profile.json");
    /// `ztune apply gpu.governor powersave` with the synthetic profile.
    const APPLIED: &str = include_str!("../tests/fixtures/tuning/apply-gpu-governor.json");
    /// `ztune telemetry --interval-ms 200` on the same fixture.
    const TELEMETRY: &str = include_str!("../tests/fixtures/tuning/telemetry.json");

    fn install() -> TempDir {
        let d = TempDir::new("tuning");
        d.file("data/adb/modules/zairenkai/bin/ztune", "#!/bin/false\n");
        let p = d.path().join("data/adb/modules/zairenkai/bin/ztune");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut dir = p.parent().map(|x| x.to_path_buf());
        while let Some(x) = dir {
            std::fs::set_permissions(&x, std::fs::Permissions::from_mode(0o755)).unwrap();
            if x == d.path() {
                break;
            }
            dir = x.parent().map(|y| y.to_path_buf());
        }
        d
    }

    fn me() -> u32 {
        // SAFETY: getuid has no preconditions.
        unsafe { libc::getuid() }
    }

    fn adapter(d: &TempDir, r: Arc<FakeRunner>) -> TuningAdapter {
        TuningAdapter::owned_by(SysRoot::at(d.path()), r, Some(BIN), me())
    }

    #[test]
    fn without_a_device_profile_the_controls_are_read_only() {
        let d = install();
        let r = Arc::new(FakeRunner::default().with(&format!("{BIN} capabilities"), CAPS));
        let caps = adapter(&d, r).discover(1);
        for id in ["tuning.io_scheduler", "tuning.gpu_governor"] {
            let c = caps.iter().find(|c| c.id == id).unwrap();
            assert_eq!(c.status, CapabilityStatus::ReadOnly, "{id}");
            assert!(!c.verified_on_device);
            assert!(c
                .evidence
                .iter()
                .any(|e| e.starts_with("write authorized: false (tier S")));
        }
    }

    #[test]
    fn discovers_read_write_capabilities_from_the_contract() {
        let d = install();
        let r = Arc::new(FakeRunner::default().with(&format!("{BIN} capabilities"), CAPS_GRANTED));
        let caps = adapter(&d, r).discover(1);
        let io = caps.iter().find(|c| c.id == "tuning.io_scheduler").unwrap();
        assert_eq!(io.status, CapabilityStatus::ReadWrite);
        assert!(!io.verified_on_device);
        assert!(io
            .evidence
            .iter()
            .any(|e| e == "Core accepts: mq-deadline kyber none"));
        let gpu = caps.iter().find(|c| c.id == "tuning.gpu_governor").unwrap();
        assert_eq!(gpu.status, CapabilityStatus::ReadWrite);
        // Only the granted values, not every value the node advertises.
        assert!(gpu
            .evidence
            .iter()
            .any(|e| e == "Core accepts: msm-adreno-tz powersave"));
        let tele = caps.iter().find(|c| c.id == "tuning.telemetry").unwrap();
        assert_eq!(tele.status, CapabilityStatus::ReadOnly);
    }

    #[test]
    fn reads_current_values() {
        let d = install();
        let r = Arc::new(FakeRunner::default().with(&format!("{BIN} capabilities"), CAPS));
        let mut a = adapter(&d, r);
        assert_eq!(
            a.read(ResourceId::IoScheduler).unwrap(),
            ResourceValue::IoScheduler(IoScheduler::MqDeadline)
        );
        assert_eq!(
            a.read(ResourceId::GpuGovernor).unwrap(),
            ResourceValue::GpuGovernor(GpuGovernor::MsmAdrenoTz)
        );
        assert_eq!(
            a.read(ResourceId::FasProfile).unwrap_err().code,
            AdapterErrorCode::Unsupported
        );
    }

    #[test]
    fn applies_with_a_fixed_argv_and_maps_refusals() {
        let d = install();
        let r = Arc::new(
            FakeRunner::default().with(&format!("{BIN} apply gpu.governor powersave"), APPLIED),
        );
        let mut a = adapter(&d, r.clone());
        a.apply(
            ResourceId::GpuGovernor,
            ResourceValue::GpuGovernor(GpuGovernor::Powersave),
        )
        .unwrap();
        assert_eq!(
            r.calls(),
            vec![format!("{BIN} apply gpu.governor powersave")]
        );
        r.set(
            &format!("{BIN} apply io.scheduler bfq"),
            Err("ztune: invalid_value: sda: \"bfq\" is not advertised".into()),
        );
        let e = a
            .apply(
                ResourceId::IoScheduler,
                ResourceValue::IoScheduler(IoScheduler::Bfq),
            )
            .unwrap_err();
        assert_eq!(e.code, AdapterErrorCode::Rejected);
        assert!(e.message.contains("not advertised"));
        r.set(
            &format!("{BIN} apply gpu.governor performance"),
            Err("ztune: not_authorized: tier S: no device_validated profile".into()),
        );
        let e = a
            .apply(
                ResourceId::GpuGovernor,
                ResourceValue::GpuGovernor(GpuGovernor::Performance),
            )
            .unwrap_err();
        assert_eq!(e.code, AdapterErrorCode::Rejected);
        r.set(
            &format!("{BIN} apply io.scheduler kyber"),
            Err("ztune: permission_denied: writing /sys/block/sda/queue/scheduler".into()),
        );
        let e = a
            .apply(
                ResourceId::IoScheduler,
                ResourceValue::IoScheduler(IoScheduler::Kyber),
            )
            .unwrap_err();
        assert_eq!(e.code, AdapterErrorCode::PermissionDenied);
        assert_eq!(
            a.apply(ResourceId::IoScheduler, ResourceValue::Bool(true))
                .unwrap_err()
                .code,
            AdapterErrorCode::Unsupported
        );
    }

    #[test]
    fn telemetry_document_is_validated() {
        let d = install();
        let r = Arc::new(
            FakeRunner::default().with(&format!("{BIN} telemetry --interval-ms 200"), TELEMETRY),
        );
        let t = adapter(&d, r).telemetry().unwrap();
        assert_eq!(t["memory"]["total"]["unit"], "bytes");
        assert!(parse_document(TELEMETRY, "capabilities").is_err());
        let wrong = TELEMETRY.replacen("\"schema_version\": 1", "\"schema_version\": 2", 1);
        assert!(matches!(
            parse_document(&wrong, "telemetry"),
            Err(TuningError::UnsupportedSchema(_))
        ));
    }

    #[test]
    fn missing_or_unsafe_binary_is_reported_not_run() {
        let d = TempDir::new("tuning-missing");
        let r = Arc::new(FakeRunner::default());
        let caps = adapter(&d, r.clone()).discover(1);
        assert!(caps
            .iter()
            .all(|c| c.status == CapabilityStatus::Unsupported));
        let d = install();
        let p = d.path().join("data/adb/modules/zairenkai/bin/ztune");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o777)).unwrap();
        let caps = adapter(&d, r.clone()).discover(1);
        assert!(caps
            .iter()
            .all(|c| c.status == CapabilityStatus::PermissionRestricted));
        assert!(r.calls().is_empty());
    }
}
