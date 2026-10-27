// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Device identity discovery. Evidence-based: every field records where it came from, and a
//! missing source stays missing.

use std::collections::BTreeMap;
use std::time::Duration;

use zcore_domain::capability::{CapabilityDescriptor, CapabilityStatus};
use zcore_domain::device::{DeviceIdentity, RootManager, SelinuxMode};
use zcore_domain::observation::{Availability, Observation};

use crate::exec::Runner;
use crate::sysroot::SysRoot;

/// Identity rarely changes; it is re-read on discovery only.
pub const IDENTITY_TTL_MS: u64 = 24 * 3600 * 1000;

/// `getprop` output: `[key]: [value]` per line.
pub fn parse_getprop(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (k, v) = l.strip_prefix('[')?.split_once("]: [")?;
            Some((k.to_string(), v.strip_suffix(']')?.to_string()))
        })
        .collect()
}

fn from_props(
    props: Option<&BTreeMap<String, String>>,
    keys: &[&str],
    now: u64,
) -> Observation<String> {
    match props {
        None => Observation::missing(
            Availability::Unavailable("getprop failed".into()),
            "getprop",
            now,
            IDENTITY_TTL_MS,
        ),
        Some(p) => match keys
            .iter()
            .find_map(|k| p.get(*k).filter(|v| !v.is_empty()).map(|v| (k, v)))
        {
            Some((k, v)) => {
                Observation::measured(v.clone(), format!("getprop {k}"), now, IDENTITY_TTL_MS)
            }
            None => Observation::missing(
                Availability::Unsupported,
                format!("getprop {}", keys.join("|")),
                now,
                IDENTITY_TTL_MS,
            ),
        },
    }
}

pub fn kernel_release() -> Option<String> {
    // SAFETY: uname fills a caller-provided struct.
    let mut u: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut u) } != 0 {
        return None;
    }
    let bytes: Vec<u8> = u
        .release
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| (*c).to_ne_bytes()[0])
        .collect();
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Root manager markers under `/data/adb` (directories created by each manager).
pub fn root_manager(root: &SysRoot) -> RootManager {
    if root.exists("/data/adb/ksu") || root.exists("/data/adb/ksud") {
        RootManager::KernelSu
    } else if root.exists("/data/adb/ap") || root.exists("/data/adb/apd") {
        RootManager::APatch
    } else if root.exists("/data/adb/magisk") {
        RootManager::Magisk
    } else {
        RootManager::None
    }
}

pub fn discover_identity(root: &SysRoot, runner: &dyn Runner, now: u64) -> DeviceIdentity {
    let props = runner
        .run(&["getprop"], Duration::from_secs(3))
        .ok()
        .map(|t| parse_getprop(&t));
    let p = props.as_ref();
    let api_level = match from_props(p, &["ro.build.version.sdk"], now) {
        Observation {
            value: Availability::Value(v),
            source,
            ..
        } => match v.parse::<u32>() {
            Ok(n) => Observation::measured(n, source, now, IDENTITY_TTL_MS),
            Err(_) => Observation::missing(
                Availability::Unavailable(format!("not a number: {v}")),
                source,
                now,
                IDENTITY_TTL_MS,
            ),
        },
        o => Observation::missing(
            match o.value {
                Availability::Unsupported => Availability::Unsupported,
                _ => Availability::Unavailable("getprop failed".into()),
            },
            o.source,
            now,
            IDENTITY_TTL_MS,
        ),
    };
    let selinux = match root.read_string("/sys/fs/selinux/enforce") {
        Ok(v) if v == "1" => Observation::measured(
            SelinuxMode::Enforcing,
            "/sys/fs/selinux/enforce",
            now,
            IDENTITY_TTL_MS,
        ),
        Ok(v) if v == "0" => Observation::measured(
            SelinuxMode::Permissive,
            "/sys/fs/selinux/enforce",
            now,
            IDENTITY_TTL_MS,
        ),
        Ok(v) => Observation::missing(
            Availability::Unavailable(format!("unexpected {v:?}")),
            "/sys/fs/selinux/enforce",
            now,
            IDENTITY_TTL_MS,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Observation::measured(
            SelinuxMode::Disabled,
            "/sys/fs/selinux/enforce (absent)",
            now,
            IDENTITY_TTL_MS,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Observation::missing(
            Availability::PermissionDenied,
            "/sys/fs/selinux/enforce",
            now,
            IDENTITY_TTL_MS,
        ),
        Err(e) => Observation::missing(
            Availability::Unavailable(e.to_string()),
            "/sys/fs/selinux/enforce",
            now,
            IDENTITY_TTL_MS,
        ),
    };
    DeviceIdentity {
        api_level,
        brand: from_props(p, &["ro.product.brand", "ro.product.vendor.brand"], now),
        model: from_props(p, &["ro.product.model", "ro.product.vendor.model"], now),
        device: from_props(p, &["ro.product.device", "ro.product.vendor.device"], now),
        soc: from_props(
            p,
            &["ro.soc.model", "ro.board.platform", "ro.hardware"],
            now,
        ),
        abi: from_props(p, &["ro.product.cpu.abi"], now),
        kernel_release: match kernel_release() {
            Some(k) => Observation::measured(k, "uname", now, IDENTITY_TTL_MS),
            None => Observation::missing(
                Availability::Unavailable("uname failed".into()),
                "uname",
                now,
                IDENTITY_TTL_MS,
            ),
        },
        root_manager: Observation::measured(
            root_manager(root),
            "/data/adb markers",
            now,
            IDENTITY_TTL_MS,
        ),
        selinux,
    }
}

/// Power HAL presence via `service check` (read-only; Core does not call the HAL).
pub fn power_hal_capability(runner: &dyn Runner, now: u64) -> CapabilityDescriptor {
    let name = "android.hardware.power.IPower/default";
    let (status, ev) = match runner.run(&["service", "check", name], Duration::from_secs(2)) {
        Ok(out) if out.contains(": found") => (
            CapabilityStatus::ReadOnly,
            format!("service check {name}: found; Core does not call the HAL in this release"),
        ),
        Ok(out) if out.contains("not found") => (
            CapabilityStatus::Unsupported,
            format!("service check {name}: not found"),
        ),
        Ok(out) => (
            CapabilityStatus::Unknown,
            format!(
                "unexpected output: {}",
                out.trim().chars().take(80).collect::<String>()
            ),
        ),
        Err(e) => (
            CapabilityStatus::Unknown,
            format!("service check failed: {e}"),
        ),
    };
    CapabilityDescriptor::new("android.power_hal", status, "discovery", vec![ev], now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::FakeRunner;
    use crate::testutil::TempDir;

    const PROPS: &str = "[ro.build.version.sdk]: [34]\n[ro.product.brand]: [google]\n[ro.product.model]: [Pixel 8]\n\
        [ro.product.device]: [shiba]\n[ro.soc.model]: [Tensor G3]\n[ro.product.cpu.abi]: [arm64-v8a]\n[ro.empty]: []\n";

    #[test]
    fn parses_getprop() {
        let p = parse_getprop(PROPS);
        assert_eq!(p["ro.product.device"], "shiba");
        assert_eq!(p["ro.empty"], "");
        assert!(parse_getprop("garbage\n[unterminated: [x").is_empty());
    }

    #[test]
    fn identity_from_fixture_with_missing_sources() {
        let d = TempDir::new("disc");
        d.file("data/adb/ksu/.keep", "");
        d.file("sys/fs/selinux/enforce", "1");
        let r = FakeRunner::default().with("getprop", PROPS);
        let id = discover_identity(&SysRoot::at(d.path()), &r, 5);
        assert_eq!(id.api_level.value.value(), Some(&34));
        assert_eq!(id.device.value.value().map(String::as_str), Some("shiba"));
        assert_eq!(id.soc.source, "getprop ro.soc.model");
        assert_eq!(id.root_manager.value.value(), Some(&RootManager::KernelSu));
        assert_eq!(id.selinux.value.value(), Some(&SelinuxMode::Enforcing));
        // getprop unavailable: fields are explicitly unavailable, not guessed.
        let id = discover_identity(&SysRoot::at(d.path()), &FakeRunner::default(), 5);
        assert!(matches!(id.model.value, Availability::Unavailable(_)));
        assert!(matches!(id.api_level.value, Availability::Unavailable(_)));
    }

    #[test]
    fn power_hal_detection() {
        let r = FakeRunner::default().with(
            "service check android.hardware.power.IPower/default",
            "Service android.hardware.power.IPower/default: found\n",
        );
        assert_eq!(
            power_hal_capability(&r, 0).status,
            CapabilityStatus::ReadOnly
        );
        let r = FakeRunner::default().with(
            "service check android.hardware.power.IPower/default",
            "Service android.hardware.power.IPower/default: not found\n",
        );
        assert_eq!(
            power_hal_capability(&r, 0).status,
            CapabilityStatus::Unsupported
        );
        assert_eq!(
            power_hal_capability(&FakeRunner::default(), 0).status,
            CapabilityStatus::Unknown
        );
    }
}
