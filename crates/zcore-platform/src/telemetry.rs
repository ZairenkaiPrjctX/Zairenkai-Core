//! Telemetry collection. Each value carries its source and freshness; failures are explicit.

use std::time::Duration;

use zcore_domain::capability::{CapabilityDescriptor, CapabilityStatus};
use zcore_domain::device::{TelemetrySnapshot, ThermalStatus};
use zcore_domain::observation::{Availability, Observation};
use zcore_domain::units::{Kib, MilliCelsius, Percent};

use crate::exec::Runner;
use crate::sysroot::SysRoot;

pub const BATTERY: &str = "/sys/class/power_supply/battery";

fn read_obs<T>(
    root: &SysRoot,
    path: &str,
    now: u64,
    ttl: u64,
    parse: impl Fn(&str) -> Option<T>,
) -> Observation<T> {
    match root.read_string(path) {
        Ok(s) => match parse(&s) {
            Some(v) => Observation::measured(v, path, now, ttl),
            None => Observation::missing(
                Availability::Unavailable(format!("unparseable {s:?}")),
                path,
                now,
                ttl,
            ),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Observation::missing(Availability::Unsupported, path, now, ttl)
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            Observation::missing(Availability::PermissionDenied, path, now, ttl)
        }
        Err(e) => Observation::missing(Availability::Unavailable(e.to_string()), path, now, ttl),
    }
}

/// `MemAvailable` from `/proc/meminfo`.
pub fn parse_mem_available(text: &str) -> Option<Kib> {
    text.lines()
        .find_map(|l| l.strip_prefix("MemAvailable:"))
        .and_then(|v| v.split_whitespace().next()?.parse().ok())
        .map(Kib)
}

/// `Thermal Status: N` from `dumpsys thermalservice`.
pub fn parse_thermal_status(text: &str) -> Option<ThermalStatus> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("Thermal Status:"))
        .and_then(|v| v.trim().parse::<i64>().ok())
        .and_then(ThermalStatus::from_code)
}

/// `settings get global low_power`: "1", "0" or "null" (never set = off).
pub fn parse_low_power(text: &str) -> Option<bool> {
    match text.trim() {
        "1" => Some(true),
        "0" | "null" => Some(false),
        _ => None,
    }
}

pub fn read_low_power(runner: &dyn Runner, now: u64, ttl: u64) -> Observation<bool> {
    let src = "settings get global low_power";
    match runner.run(
        &["settings", "get", "global", "low_power"],
        Duration::from_secs(2),
    ) {
        Ok(out) => match parse_low_power(&out) {
            Some(v) => Observation::measured(v, src, now, ttl),
            None => Observation::missing(
                Availability::Unavailable(format!("unexpected {:?}", out.trim())),
                src,
                now,
                ttl,
            ),
        },
        Err(e) => Observation::missing(Availability::Unavailable(e.to_string()), src, now, ttl),
    }
}

pub fn collect(root: &SysRoot, runner: &dyn Runner, now: u64, ttl: u64) -> TelemetrySnapshot {
    let thermal_status = match runner.run(&["dumpsys", "thermalservice"], Duration::from_secs(2)) {
        Ok(t) => match parse_thermal_status(&t) {
            Some(s) => Observation::measured(s, "dumpsys thermalservice", now, ttl),
            None => Observation::missing(
                Availability::Unsupported,
                "dumpsys thermalservice",
                now,
                ttl,
            ),
        },
        Err(e) => Observation::missing(
            Availability::Unavailable(e.to_string()),
            "dumpsys thermalservice",
            now,
            ttl,
        ),
    };
    TelemetrySnapshot {
        battery_level: read_obs(root, &format!("{BATTERY}/capacity"), now, ttl, |s| {
            s.parse::<u8>().ok().and_then(|v| Percent::new(v).ok())
        }),
        charging: read_obs(root, &format!("{BATTERY}/status"), now, ttl, |s| match s {
            "Charging" | "Full" => Some(true),
            "Discharging" | "Not charging" => Some(false),
            _ => None,
        }),
        // Power supply `temp` is in tenths of a degree Celsius.
        battery_temp: read_obs(root, &format!("{BATTERY}/temp"), now, ttl, |s| {
            s.parse::<i64>().ok().map(|t| MilliCelsius(t * 100))
        }),
        thermal_status,
        mem_available: read_obs(root, "/proc/meminfo", now, ttl, parse_mem_available),
        low_power_mode: read_low_power(runner, now, ttl),
        fas: Observation::unknown(),
    }
}

/// Read-only telemetry capabilities derived from a snapshot.
pub fn capabilities(t: &TelemetrySnapshot, now: u64) -> Vec<CapabilityDescriptor> {
    fn st<T>(o: &Observation<T>) -> CapabilityStatus {
        match o.value {
            Availability::Value(_) => CapabilityStatus::ReadOnly,
            Availability::Unsupported => CapabilityStatus::Unsupported,
            Availability::PermissionDenied => CapabilityStatus::PermissionRestricted,
            Availability::Unavailable(_) => CapabilityStatus::TemporarilyUnavailable,
            Availability::Unknown => CapabilityStatus::Unknown,
        }
    }
    vec![
        CapabilityDescriptor::new(
            "telemetry.battery_level",
            st(&t.battery_level),
            "telemetry",
            vec![t.battery_level.source.clone()],
            now,
        ),
        CapabilityDescriptor::new(
            "telemetry.charging",
            st(&t.charging),
            "telemetry",
            vec![t.charging.source.clone()],
            now,
        ),
        CapabilityDescriptor::new(
            "telemetry.battery_temp",
            st(&t.battery_temp),
            "telemetry",
            vec![t.battery_temp.source.clone()],
            now,
        ),
        CapabilityDescriptor::new(
            "telemetry.thermal_status",
            st(&t.thermal_status),
            "telemetry",
            vec![t.thermal_status.source.clone()],
            now,
        ),
        CapabilityDescriptor::new(
            "telemetry.mem_available",
            st(&t.mem_available),
            "telemetry",
            vec![t.mem_available.source.clone()],
            now,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::FakeRunner;
    use crate::testutil::TempDir;

    #[test]
    fn collects_from_fixture() {
        let d = TempDir::new("tel");
        d.file("sys/class/power_supply/battery/capacity", "57");
        d.file("sys/class/power_supply/battery/status", "Discharging");
        d.file("sys/class/power_supply/battery/temp", "312");
        d.file(
            "proc/meminfo",
            "MemTotal: 8000000 kB\nMemAvailable:    3100000 kB\n",
        );
        let r = FakeRunner::default()
            .with(
                "dumpsys thermalservice",
                "IsStatusOverride: false\nThermal Status: 1\n",
            )
            .with("settings get global low_power", "null\n");
        let t = collect(&SysRoot::at(d.path()), &r, 100, 5_000);
        assert_eq!(t.battery_level.value.value().map(|p| p.get()), Some(57));
        assert_eq!(t.charging.value.value(), Some(&false));
        assert_eq!(t.battery_temp.value.value(), Some(&MilliCelsius(31_200)));
        assert_eq!(t.mem_available.value.value(), Some(&Kib(3_100_000)));
        assert_eq!(t.thermal_status.value.value(), Some(&ThermalStatus::Light));
        assert_eq!(t.low_power_mode.value.value(), Some(&false));
        let caps = capabilities(&t, 100);
        assert!(caps.iter().all(|c| c.status == CapabilityStatus::ReadOnly));
    }

    #[test]
    fn missing_and_bad_values_are_explicit() {
        let d = TempDir::new("tel2");
        d.file("sys/class/power_supply/battery/capacity", "250");
        let t = collect(&SysRoot::at(d.path()), &FakeRunner::default(), 100, 5_000);
        assert!(
            matches!(t.battery_level.value, Availability::Unavailable(_)),
            "out-of-range capacity rejected"
        );
        assert!(matches!(t.charging.value, Availability::Unsupported));
        assert!(matches!(
            t.thermal_status.value,
            Availability::Unavailable(_)
        ));
        assert!(matches!(
            t.low_power_mode.value,
            Availability::Unavailable(_)
        ));
        assert_eq!(parse_low_power("garbage"), None);
    }
}
