// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Zairenkai-ThermalTools: read-only thermal diagnostics from the `zthermal` CLI.
//!
//! Core runs `zthermal snapshot` (the JSON contract in ThermalTools `docs/INTEGRATION.md`,
//! `schema_version` 1) and reports what it learned as capabilities. The adapter:
//! - runs only a fixed, read-only subcommand; `apply` and `rollback` are refused before
//!   anything runs;
//! - runs only a binary configured in `paths.thermaltools_binary` that is owned by root and
//!   not group- or world-writable (Core runs as root, so a writable binary would be an
//!   escalation path);
//! - bounds time and output, and validates the schema version and the fields it uses;
//! - takes no identity data from the snapshot (Core has its own discovery).
//!
//! Nothing here can change a thermal setting: ThermalTools itself never writes in this
//! release, and Core never asks it to.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use zcore_domain::capability::{CapabilityDescriptor, CapabilityStatus};

use crate::component::{self, BinaryError};
use crate::exec::{ExecError, Runner, MAX_OUTPUT};
use crate::sysroot::SysRoot;

/// Configuration key in `[paths]`. There is no default: the component ships no module yet.
pub const PATH_KEY: &str = "thermaltools_binary";
pub const BINARY_NAME: &str = "zthermal";
pub const SUPPORTED_SCHEMA: u64 = 1;
/// Runs on the engine thread, so it bounds how long a hung binary can delay IPC. A snapshot
/// took 2-5 ms on a Linux host (fixtures and live, 2026-10-21); device timing is unmeasured.
pub const TIMEOUT: Duration = Duration::from_secs(5);
pub const OWNER: &str = "thermaltools";

/// What Core may ask ThermalTools for. Every query is read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Query {
    Snapshot,
}

impl Query {
    fn args(self) -> &'static [&'static str] {
        match self {
            // Redaction is the default; `--no-redact` is never passed.
            Query::Snapshot => &["snapshot"],
        }
    }

    /// Parses a requested subcommand name. Write commands are refused by name.
    pub fn parse(name: &str) -> Result<Query, ThermalToolsError> {
        match name {
            "snapshot" => Ok(Query::Snapshot),
            "apply" | "rollback" | "backup-config" => {
                Err(ThermalToolsError::WriteNotSupported(name.into()))
            }
            other => Err(ThermalToolsError::NotAllowed(other.into())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum ThermalToolsError {
    /// `paths.thermaltools_binary` is not set.
    NotConfigured,
    /// The configured file does not exist or is not a regular file.
    NotInstalled(String),
    /// The binary could be replaced by a non-root user, or the path is not acceptable.
    UnsafeBinary(String),
    /// A write subcommand was requested. Core never sends one.
    WriteNotSupported(String),
    /// A subcommand outside the read-only allowlist.
    NotAllowed(String),
    Timeout,
    /// The process ran and failed (exit status, signal, missing loader).
    Failed(String),
    OutputTooLarge,
    /// The output is not the expected JSON document.
    Malformed(String),
    UnsupportedSchema(u64),
}

impl std::fmt::Display for ThermalToolsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "paths.{PATH_KEY} is not configured"),
            Self::NotInstalled(d) => write!(f, "not installed: {d}"),
            Self::UnsafeBinary(d) => write!(f, "refused to run: {d}"),
            Self::WriteNotSupported(c) => {
                write!(f, "{c:?} is a write command; Core only reads ThermalTools")
            }
            Self::NotAllowed(c) => write!(f, "subcommand {c:?} is not allowed"),
            Self::Timeout => write!(f, "timed out"),
            Self::Failed(d) => write!(f, "failed: {d}"),
            Self::OutputTooLarge => write!(f, "output larger than {MAX_OUTPUT} bytes"),
            Self::Malformed(d) => write!(f, "malformed output: {d}"),
            Self::UnsupportedSchema(v) => write!(
                f,
                "schema_version {v} is not supported (expected {SUPPORTED_SCHEMA})"
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HottestZone {
    pub name: String,
    pub millicelsius: i64,
}

/// A capability as ThermalTools reports it (its own vocabulary, kept verbatim).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolCapability {
    pub id: String,
    pub state: String,
}

/// The parts of a snapshot Core uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThermalToolsReport {
    pub tool_version: String,
    pub schema_version: u64,
    pub zones: u64,
    pub zones_valid: u64,
    pub hottest_zone: Option<HottestZone>,
    /// ThrottlingSeverity name as reported, or `None` when unknown (never "cool").
    pub framework_status: Option<String>,
    pub capabilities: Vec<ToolCapability>,
}

/// Checks the configured path before anything runs. `owner_uid` is the uid every path
/// component must belong to: 0 in production (Core runs as root).
pub fn check_binary(root: &SysRoot, path: &str, owner_uid: u32) -> Result<(), ThermalToolsError> {
    component::check_binary(root, path, BINARY_NAME, owner_uid).map_err(|e| match e {
        BinaryError::NotInstalled(m) => ThermalToolsError::NotInstalled(m),
        BinaryError::Unsafe(m) => ThermalToolsError::UnsafeBinary(m),
    })
}

fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value, ThermalToolsError> {
    v.get(key)
        .ok_or_else(|| ThermalToolsError::Malformed(format!("missing {key:?}")))
}

fn uint(v: &Value, key: &str) -> Result<u64, ThermalToolsError> {
    field(v, key)?
        .as_u64()
        .ok_or_else(|| ThermalToolsError::Malformed(format!("{key:?} is not an unsigned integer")))
}

/// Parses `zthermal snapshot` output.
pub fn parse_snapshot(text: &str) -> Result<ThermalToolsReport, ThermalToolsError> {
    let v: Value =
        serde_json::from_str(text).map_err(|e| ThermalToolsError::Malformed(e.to_string()))?;
    if !v.is_object() {
        return Err(ThermalToolsError::Malformed(
            "top level is not an object".into(),
        ));
    }
    let schema = uint(&v, "schema_version")?;
    if schema != SUPPORTED_SCHEMA {
        return Err(ThermalToolsError::UnsupportedSchema(schema));
    }
    let tool_version = field(&v, "tool_version")?
        .as_str()
        .ok_or_else(|| ThermalToolsError::Malformed("tool_version is not a string".into()))?
        .to_string();
    let s = field(&v, "summary")?;
    let hottest_zone = match field(s, "hottest_zone")? {
        Value::Null => None,
        h => Some(HottestZone {
            name: field(h, "name")?
                .as_str()
                .unwrap_or("?")
                .chars()
                .take(64)
                .collect(),
            millicelsius: field(h, "millicelsius")?
                .as_i64()
                .ok_or_else(|| ThermalToolsError::Malformed("hottest_zone.millicelsius".into()))?,
        }),
    };
    let framework_status = match field(s, "framework_status")? {
        Value::Null => None,
        Value::String(x) => Some(x.clone()),
        _ => return Err(ThermalToolsError::Malformed("framework_status".into())),
    };
    let caps = field(&v, "capabilities")?
        .as_array()
        .ok_or_else(|| ThermalToolsError::Malformed("capabilities is not an array".into()))?;
    let capabilities = caps
        .iter()
        .map(|c| {
            match (
                c.get("id").and_then(Value::as_str),
                c.get("state").and_then(Value::as_str),
            ) {
                (Some(id), Some(state)) => Ok(ToolCapability {
                    id: id.into(),
                    state: state.into(),
                }),
                _ => Err(ThermalToolsError::Malformed(
                    "capability without id or state".into(),
                )),
            }
        })
        .collect::<Result<_, _>>()?;
    Ok(ThermalToolsReport {
        tool_version,
        schema_version: schema,
        zones: uint(s, "zones")?,
        zones_valid: uint(s, "zones_valid")?,
        hottest_zone,
        framework_status,
        capabilities,
    })
}

/// Runs one read-only query against the configured binary, which must be root-owned.
pub fn run(
    root: &SysRoot,
    runner: &dyn Runner,
    binary: Option<&str>,
    query: Query,
) -> Result<ThermalToolsReport, ThermalToolsError> {
    run_owned_by(root, runner, binary, query, 0)
}

/// [`run`] with an explicit owner uid for the binary and its directories.
pub fn run_owned_by(
    root: &SysRoot,
    runner: &dyn Runner,
    binary: Option<&str>,
    query: Query,
    owner_uid: u32,
) -> Result<ThermalToolsReport, ThermalToolsError> {
    let path = binary
        .filter(|p| !p.is_empty())
        .ok_or(ThermalToolsError::NotConfigured)?;
    check_binary(root, path, owner_uid)?;
    let mut argv = vec![path];
    argv.extend_from_slice(query.args());
    let out = runner.run(&argv, TIMEOUT).map_err(|e| match e {
        ExecError::Timeout => ThermalToolsError::Timeout,
        ExecError::Spawn(e) if e.kind() == std::io::ErrorKind::NotFound => {
            ThermalToolsError::NotInstalled(e.to_string())
        }
        ExecError::NotAllowed(m) => ThermalToolsError::UnsafeBinary(m),
        other => ThermalToolsError::Failed(other.to_string()),
    })?;
    if out.len() >= MAX_OUTPUT {
        return Err(ThermalToolsError::OutputTooLarge);
    }
    parse_snapshot(&out)
}

/// Capabilities Core reports for the result.
pub fn capabilities(
    result: &Result<ThermalToolsReport, ThermalToolsError>,
    binary: Option<&str>,
    now: u64,
) -> Vec<CapabilityDescriptor> {
    let src = binary.unwrap_or("(not configured)").to_string();
    match result {
        Ok(r) => {
            let sensors = if r.zones_valid > 0 {
                CapabilityStatus::ReadOnly
            } else {
                CapabilityStatus::Unsupported
            };
            vec![
                CapabilityDescriptor::new(
                    "thermaltools.diagnostics",
                    CapabilityStatus::ReadOnly,
                    OWNER,
                    vec![
                        format!("{src} snapshot"),
                        format!("zthermal {}", r.tool_version),
                    ],
                    now,
                ),
                CapabilityDescriptor::new(
                    "thermaltools.sensors",
                    sensors,
                    OWNER,
                    vec![format!(
                        "{} of {} thermal zones valid",
                        r.zones_valid, r.zones
                    )],
                    now,
                ),
            ]
        }
        Err(e) => {
            let status = match e {
                ThermalToolsError::NotConfigured | ThermalToolsError::NotInstalled(_) => {
                    CapabilityStatus::Unsupported
                }
                ThermalToolsError::UnsafeBinary(_) => CapabilityStatus::PermissionRestricted,
                ThermalToolsError::Timeout | ThermalToolsError::Failed(_) => {
                    CapabilityStatus::TemporarilyUnavailable
                }
                _ => CapabilityStatus::Unknown,
            };
            vec![CapabilityDescriptor::new(
                "thermaltools.diagnostics",
                status,
                OWNER,
                vec![src, e.to_string()],
                now,
            )]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::FakeRunner;
    use crate::testutil::TempDir;
    use std::os::unix::fs::PermissionsExt;

    const BIN: &str = "/data/adb/zairenkai/bin/zthermal";

    /// Shape of `zthermal snapshot` 0.2.0 output (trimmed; generated from its fixtures).
    const SNAPSHOT: &str = r#"{"schema_version": 1, "tool_version": "0.2.0", "generated_unix_ms": 1,
        "root": "device", "redacted": true,
        "identity": {"build_fingerprint": "redacted:sha256:1319fec65ffa", "device": "synthqcom"},
        "sources": [], "summary": {"zones": 5, "zones_valid": 4, "zones_invalid": 1, "cooling_devices": 2,
        "hottest_zone": {"source": "/sys/class/thermal/thermal_zone1", "name": "cpu-1-0-usr", "millicelsius": 43200},
        "framework_status": "light", "battery_millicelsius": 30500},
        "sysfs": {}, "thermalservice": null, "battery": null, "adapters": [],
        "capabilities": [{"id": "sensors.thermal_zones", "state": "AVAILABLE_READ_ONLY"},
                         {"id": "control.thermal_policy", "state": "UNSUPPORTED"}]}"#;

    /// A root-owned, non-writable binary inside a fixture root (tests run as root).
    fn install() -> TempDir {
        let d = TempDir::new("tt");
        d.file("data/adb/zairenkai/bin/zthermal", "#!/bin/false\n");
        let p = d.path().join("data/adb/zairenkai/bin/zthermal");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut dir = p.parent().map(|x| x.to_path_buf());
        while let Some(x) = dir {
            if x == d.path() {
                break;
            }
            std::fs::set_permissions(&x, std::fs::Permissions::from_mode(0o755)).unwrap();
            dir = x.parent().map(|y| y.to_path_buf());
        }
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        d
    }

    fn me() -> u32 {
        // SAFETY: getuid has no preconditions.
        unsafe { libc::getuid() }
    }

    /// Runs with the fixture owned by the test's own uid (production requires uid 0).
    fn go(
        root: &SysRoot,
        r: &FakeRunner,
        bin: Option<&str>,
    ) -> Result<ThermalToolsReport, ThermalToolsError> {
        run_owned_by(root, r, bin, Query::Snapshot, me())
    }

    #[test]
    fn successful_snapshot() {
        let d = install();
        let r = FakeRunner::default().with(&format!("{BIN} snapshot"), SNAPSHOT);
        let rep = go(&SysRoot::at(d.path()), &r, Some(BIN)).unwrap();
        assert_eq!(rep.tool_version, "0.2.0");
        assert_eq!(
            rep.hottest_zone,
            Some(HottestZone {
                name: "cpu-1-0-usr".into(),
                millicelsius: 43_200
            })
        );
        assert_eq!(rep.framework_status.as_deref(), Some("light"));
        assert_eq!(r.calls(), [format!("{BIN} snapshot")]);
        let caps = capabilities(&Ok(rep), Some(BIN), 0);
        assert_eq!(caps[0].status, CapabilityStatus::ReadOnly);
        assert_eq!(caps[1].status, CapabilityStatus::ReadOnly);
        assert!(caps
            .iter()
            .all(|c| c.status != CapabilityStatus::ReadWrite && !c.verified_on_device));
    }

    #[test]
    fn real_zthermal_output_matches_the_contract() {
        // Generated by `zthermal 0.2.0 snapshot --root tests/fixtures/devices/synthetic-qcom`
        // in Zairenkai-ThermalTools (synthetic fixture data; generated_unix_ms set to 0).
        let text = include_str!("../tests/fixtures/zthermal-0.2.0-snapshot.json");
        let rep = parse_snapshot(text).unwrap();
        assert_eq!(
            (rep.tool_version.as_str(), rep.zones, rep.zones_valid),
            ("0.2.0", 5, 4)
        );
        assert_eq!(
            rep.hottest_zone.as_ref().map(|h| h.millicelsius),
            Some(43_200)
        );
        assert!(rep
            .capabilities
            .iter()
            .any(|c| c.id == "control.thermal_policy" && c.state == "UNSUPPORTED"));
        assert!(!rep
            .capabilities
            .iter()
            .any(|c| c.state == "AVAILABLE_WRITABLE"));
    }

    #[test]
    fn identity_is_not_taken_from_the_snapshot() {
        let rep = parse_snapshot(SNAPSHOT).unwrap();
        let json = serde_json::to_string(&rep).unwrap();
        assert!(
            !json.contains("fingerprint") && !json.contains("synthqcom"),
            "{json}"
        );
    }

    #[test]
    fn missing_and_unconfigured_binaries_are_not_operational() {
        let d = TempDir::new("tt-none");
        let root = SysRoot::at(d.path());
        let r = FakeRunner::default();
        assert_eq!(go(&root, &r, None), Err(ThermalToolsError::NotConfigured));
        assert_eq!(
            go(&root, &r, Some("")),
            Err(ThermalToolsError::NotConfigured)
        );
        assert!(matches!(
            go(&root, &r, Some(BIN)),
            Err(ThermalToolsError::NotInstalled(_))
        ));
        assert!(matches!(
            run(&root, &r, Some(BIN), Query::Snapshot),
            Err(ThermalToolsError::NotInstalled(_))
        ));
        assert!(
            r.calls().is_empty(),
            "nothing may run when the binary is absent"
        );
        let caps = capabilities(&Err(ThermalToolsError::NotConfigured), None, 0);
        assert_eq!(caps[0].status, CapabilityStatus::Unsupported);
    }

    #[test]
    fn unsafe_binaries_are_refused_before_running() {
        let r = FakeRunner::default();
        let d = install();
        let root = SysRoot::at(d.path());
        for bad in [
            "zthermal",
            "/data/adb/zairenkai/bin/../bin/zthermal",
            "/data/adb/zairenkai/bin/sh",
        ] {
            assert!(
                matches!(
                    go(&root, &r, Some(bad)),
                    Err(ThermalToolsError::UnsafeBinary(_))
                ),
                "{bad}"
            );
        }
        // Owned by someone else than the required owner.
        let e =
            run_owned_by(&root, &r, Some(BIN), Query::Snapshot, me().wrapping_add(1)).unwrap_err();
        assert!(
            matches!(&e, ThermalToolsError::UnsafeBinary(m) if m.contains("owned by")),
            "{e:?}"
        );
        // World-writable binary, then a world-writable directory above it.
        let p = d.path().join("data/adb/zairenkai/bin/zthermal");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o777)).unwrap();
        let e = go(&root, &r, Some(BIN)).unwrap_err();
        assert!(
            matches!(&e, ThermalToolsError::UnsafeBinary(m) if m.contains("writable")),
            "{e:?}"
        );
        assert_eq!(
            capabilities(&Err(e), Some(BIN), 0)[0].status,
            CapabilityStatus::PermissionRestricted
        );
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(
            d.path().join("data/adb"),
            std::fs::Permissions::from_mode(0o777),
        )
        .unwrap();
        assert!(matches!(
            go(&root, &r, Some(BIN)),
            Err(ThermalToolsError::UnsafeBinary(_))
        ));
        assert!(r.calls().is_empty(), "nothing may run when a check fails");
    }

    #[test]
    fn subprocess_failures_and_bad_output() {
        let d = install();
        let root = SysRoot::at(d.path());
        let key = format!("{BIN} snapshot");
        let case = |out: Result<String, String>| {
            let r = FakeRunner::default();
            r.set(&key, out);
            go(&root, &r, Some(BIN)).unwrap_err()
        };
        assert_eq!(case(Err("timeout".into())), ThermalToolsError::Timeout);
        assert!(matches!(
            case(Err("Permission denied".into())),
            ThermalToolsError::Failed(_)
        ));
        assert!(matches!(
            case(Ok("not json".into())),
            ThermalToolsError::Malformed(_)
        ));
        assert!(matches!(
            case(Ok("[]".into())),
            ThermalToolsError::Malformed(_)
        ));
        assert_eq!(
            case(Ok(
                SNAPSHOT.replace("\"schema_version\": 1", "\"schema_version\": 2")
            )),
            ThermalToolsError::UnsupportedSchema(2)
        );
        assert!(matches!(
            case(Ok(SNAPSHOT.replace("\"summary\"", "\"summari\""))),
            ThermalToolsError::Malformed(_)
        ));
        assert!(matches!(
            case(Ok(
                SNAPSHOT.replace("\"zones_valid\": 4", "\"zones_valid\": -1")
            )),
            ThermalToolsError::Malformed(_)
        ));
        assert_eq!(
            case(Ok("x".repeat(MAX_OUTPUT))),
            ThermalToolsError::OutputTooLarge
        );
        let caps = capabilities(&Err(ThermalToolsError::Timeout), Some(BIN), 0);
        assert_eq!(caps[0].status, CapabilityStatus::TemporarilyUnavailable);
    }

    #[test]
    fn unsupported_android_interfaces_are_reported_through_the_snapshot() {
        // A host or device where zthermal finds no zones: diagnostics work, sensors do not.
        let none = SNAPSHOT.replace(
            "\"zones\": 5, \"zones_valid\": 4",
            "\"zones\": 0, \"zones_valid\": 0",
        );
        let none = none.replace(r#"{"source": "/sys/class/thermal/thermal_zone1", "name": "cpu-1-0-usr", "millicelsius": 43200}"#, "null");
        let none = none.replace(
            "\"framework_status\": \"light\"",
            "\"framework_status\": null",
        );
        let rep = parse_snapshot(&none).unwrap();
        assert_eq!(
            (rep.hottest_zone.as_ref(), rep.framework_status.as_ref()),
            (None, None)
        );
        let caps = capabilities(&Ok(rep), Some(BIN), 0);
        assert_eq!(caps[1].status, CapabilityStatus::Unsupported);
    }

    #[test]
    fn write_requests_are_refused() {
        for c in ["apply", "rollback", "backup-config"] {
            assert_eq!(
                Query::parse(c),
                Err(ThermalToolsError::WriteNotSupported(c.into()))
            );
        }
        assert!(matches!(
            Query::parse("doctor; reboot"),
            Err(ThermalToolsError::NotAllowed(_))
        ));
        assert_eq!(Query::parse("snapshot"), Ok(Query::Snapshot));
        // The only argv Core can build has no write subcommand and no --no-redact.
        assert_eq!(Query::Snapshot.args(), ["snapshot"]);
    }
}
