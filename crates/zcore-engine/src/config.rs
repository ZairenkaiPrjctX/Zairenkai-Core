// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Core configuration (TOML, versioned, strict).
//!
//! Configuration can *grant* operations to principals and *add restrictions*. It cannot weaken
//! the built-in safety rules: those are compiled in (see `policy.rs`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use zcore_domain::device::ThermalStatus;
use zcore_domain::request::{Direction, OperationKind};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Telemetry {
    /// How often the daemon refreshes the device context.
    pub refresh_ms: u64,
    /// Context older than this is stale: performance-raising actions are denied.
    pub max_context_age_ms: u64,
    /// How often capabilities are re-discovered.
    pub discovery_ms: u64,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self {
            refresh_ms: 5_000,
            max_context_age_ms: 15_000,
            discovery_ms: 60_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Safety {
    /// At or below this battery level and not charging, raising actions are denied.
    pub battery_critical_percent: u8,
    /// Deny raising actions when the Android thermal status cannot be read.
    pub require_thermal_status: bool,
    /// Recommendations below this confidence are denied.
    pub recommendation_min_confidence: f64,
    /// Recommendations always become temporary leases of at most this long.
    pub recommendation_max_ms: u64,
}

impl Default for Safety {
    fn default() -> Self {
        Self {
            battery_critical_percent: 10,
            require_thermal_status: true,
            recommendation_min_confidence: 0.7,
            recommendation_max_ms: 600_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ipc {
    /// Sustained requests per second per peer uid.
    pub rate_per_sec: u32,
    pub burst: u32,
    /// Requests waiting for the engine; more are answered `busy`.
    pub queue: usize,
    pub request_timeout_ms: u64,
}

impl Default for Ipc {
    fn default() -> Self {
        Self {
            rate_per_sec: 5,
            burst: 20,
            queue: 32,
            request_timeout_ms: 10_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// An app client (e.g. Zairenkai-App). Gets only the operations granted to it.
    App,
    /// Zairenkai-Intelligence-Engine. Its requests are recommendations.
    IntelligenceEngine,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrincipalMap {
    pub uid: u32,
    pub role: Role,
    /// Operations this principal may request.
    #[serde(default)]
    pub grants: Vec<OperationKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleEffect {
    Deny,
    RequireConfirmation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginMatch {
    User,
    Recommendation,
}

/// Conditions of a configured rule. All set conditions must hold.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct When {
    pub operations: Vec<OperationKind>,
    pub direction: Option<Direction>,
    pub origin: Option<OriginMatch>,
    pub battery_below: Option<u8>,
    pub charging: Option<bool>,
    pub thermal_at_least: Option<ThermalStatus>,
}

/// A restriction. Rules never allow anything.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub effect: RuleEffect,
    pub when: When,
    #[serde(default)]
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CoreConfig {
    pub schema_version: u32,
    pub telemetry: Telemetry,
    pub safety: Safety,
    pub ipc: Ipc,
    /// Operations root (uid 0) may request.
    pub root_grants: Vec<OperationKind>,
    pub principals: Vec<PrincipalMap>,
    pub rules: Vec<Rule>,
    /// Paths used by platform adapters, overridable for testing and unusual layouts.
    pub paths: BTreeMap<String, String>,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            telemetry: Telemetry::default(),
            safety: Safety::default(),
            ipc: Ipc::default(),
            root_grants: OperationKind::ALL.to_vec(),
            principals: vec![],
            rules: vec![],
            paths: BTreeMap::new(),
        }
    }
}

pub const KNOWN_PATHS: [&str; 4] = [
    "fas_socket",
    "afid_catalog",
    "modules_dir",
    "thermaltools_binary",
];

impl CoreConfig {
    pub fn parse(text: &str) -> Result<(Self, Vec<String>), String> {
        let mut table: toml::Table = text
            .parse()
            .map_err(|e: toml::de::Error| format!("syntax: {e}"))?;
        let mut notes = vec![];
        match table.get("schema_version") {
            None => {
                notes.push(format!(
                    "schema_version missing; treated as {SCHEMA_VERSION}"
                ));
                table.insert(
                    "schema_version".into(),
                    toml::Value::Integer(SCHEMA_VERSION as i64),
                );
            }
            Some(toml::Value::Integer(v)) if *v == SCHEMA_VERSION as i64 => {}
            Some(v) => {
                return Err(format!(
                    "unsupported schema_version {v}; this build understands {SCHEMA_VERSION}"
                ))
            }
        }
        let cfg: CoreConfig = table
            .try_into()
            .map_err(|e: toml::de::Error| e.to_string())?;
        cfg.validate()?;
        Ok((cfg, notes))
    }

    pub fn validate(&self) -> Result<(), String> {
        let t = &self.telemetry;
        if !(500..=60_000).contains(&t.refresh_ms)
            || t.max_context_age_ms < t.refresh_ms * 2
            || t.discovery_ms < t.refresh_ms
        {
            return Err("telemetry: need 500 <= refresh_ms <= 60000, max_context_age_ms >= 2*refresh_ms, discovery_ms >= refresh_ms".into());
        }
        let s = &self.safety;
        if s.battery_critical_percent > 50 {
            return Err("safety.battery_critical_percent must be <= 50".into());
        }
        if !(0.0..=1.0).contains(&s.recommendation_min_confidence)
            || s.recommendation_max_ms == 0
            || s.recommendation_max_ms > 3_600_000
        {
            return Err("safety: recommendation_min_confidence in 0..=1, recommendation_max_ms in 1..=3600000".into());
        }
        let i = &self.ipc;
        if i.rate_per_sec == 0
            || i.burst < i.rate_per_sec
            || i.queue == 0
            || i.queue > 1024
            || !(100..=60_000).contains(&i.request_timeout_ms)
        {
            return Err("ipc: rate_per_sec >= 1, burst >= rate_per_sec, queue 1..=1024, request_timeout_ms 100..=60000".into());
        }
        let mut uids = std::collections::BTreeSet::new();
        for p in &self.principals {
            if p.uid == 0 || p.uid < 1000 {
                return Err(format!(
                    "principals: uid {} is a system uid; root is configured with root_grants",
                    p.uid
                ));
            }
            if !uids.insert(p.uid) {
                return Err(format!("principals: uid {} listed twice", p.uid));
            }
        }
        let mut ids = std::collections::BTreeSet::new();
        for r in &self.rules {
            if r.id.is_empty()
                || r.id.len() > 64
                || !r
                    .id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
            {
                return Err(format!("rules: invalid id {:?}", r.id));
            }
            if r.id.starts_with("builtin.") {
                return Err(format!(
                    "rules: id {:?} uses the reserved builtin. prefix",
                    r.id
                ));
            }
            if !ids.insert(r.id.clone()) {
                return Err(format!("rules: duplicate id {:?}", r.id));
            }
            if r.when.battery_below.is_some_and(|b| b > 100) {
                return Err(format!("rules.{}: battery_below must be <= 100", r.id));
            }
            if r.explanation.len() > 200 {
                return Err(format!("rules.{}: explanation too long", r.id));
            }
        }
        for (k, v) in &self.paths {
            if !KNOWN_PATHS.contains(&k.as_str()) {
                return Err(format!("paths: unknown key {k:?}"));
            }
            if !v.starts_with('/') || v.contains("..") || v.contains('\0') {
                return Err(format!("paths.{k}: must be an absolute path without '..'"));
            }
        }
        Ok(())
    }

    /// A short stable fingerprint of the policy-relevant configuration, for audit records.
    pub fn policy_version(&self) -> String {
        let canon = serde_json::to_string(&(
            &self.safety,
            &self.root_grants,
            &self.principals,
            &self.rules,
        ))
        .unwrap_or_default();
        let mut h: u64 = 0xcbf29ce484222325;
        for b in canon.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        format!("builtin-1/cfg-{:016x}", h)
    }

    pub fn path(&self, key: &str, default: &str) -> String {
        self.paths
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_partial() {
        let (c, notes) = CoreConfig::parse("").unwrap();
        assert_eq!(c, CoreConfig::default());
        assert_eq!(notes.len(), 1);
        let (c, _) = CoreConfig::parse(
            "schema_version = 1\n[[principals]]\nuid = 10234\nrole = \"app\"\ngrants = [\"fas_set_profile\"]\n\
             [[rules]]\nid = \"no-raise-on-low-battery\"\neffect = \"deny\"\nwhen = { direction = \"raise\", battery_below = 30, charging = false }\n",
        )
        .unwrap();
        assert_eq!(c.principals[0].grants, vec![OperationKind::FasSetProfile]);
        assert_eq!(c.rules[0].when.battery_below, Some(30));
    }

    #[test]
    fn rejects_unsafe_or_unknown() {
        for bad in [
            "schema_version = 2\n",
            "[safety]\nbattery_critical_percent = 90\n",
            "[[principals]]\nuid = 0\nrole = \"app\"\n",
            "[[principals]]\nuid = 10001\nrole = \"admin\"\n",
            "[[principals]]\nuid = 10001\nrole = \"app\"\ngrants = [\"shell\"]\n",
            "[[rules]]\nid = \"builtin.x\"\neffect = \"deny\"\nwhen = {}\n",
            "[[rules]]\nid = \"r\"\neffect = \"allow\"\nwhen = {}\n",
            "[paths]\nfas_socket = \"../x\"\n",
            "[paths]\nshell = \"/system/bin/sh\"\n",
            "[telemetry]\nrefresh_ms = 10\n",
            "unknown = 1\n",
        ] {
            assert!(CoreConfig::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn policy_version_changes_with_rules() {
        let a = CoreConfig::default();
        let mut b = a.clone();
        b.safety.battery_critical_percent = 15;
        assert_ne!(a.policy_version(), b.policy_version());
        assert_eq!(a.policy_version(), CoreConfig::default().policy_version());
    }
}
