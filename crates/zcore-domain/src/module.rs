// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Module contract: what a Zairenkai component declares about itself.
//!
//! A manifest is a *declaration*. It never grants anything: permissions come only from Core's
//! configuration, and a module is not trusted because of its name or origin.

use serde::{Deserialize, Serialize};

use crate::request::OperationKind;
use crate::resource::ResourceId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.strip_prefix('v').unwrap_or(s);
        let mut it = s.split('.');
        let v = Version {
            major: it.next()?.parse().ok()?,
            minor: it.next()?.parse().ok()?,
            patch: it.next()?.split(['-', '+']).next()?.parse().ok()?,
        };
        it.next().is_none().then_some(v)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The Core API version this build implements.
pub const CORE_API: Version = Version {
    major: 1,
    minor: 0,
    patch: 0,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleHealth {
    Healthy,
    Degraded,
    Unreachable,
    Disabled,
    NotInstalled,
    Unknown,
}

/// `zairenkai-module.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleManifest {
    pub id: String,
    pub version: String,
    /// Inclusive range of Core API versions, e.g. `["1.0.0", "1.99.99"]`.
    pub core_api: [String; 2],
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    /// Operations the module would like to request. Declarative only.
    #[serde(default)]
    pub requested_permissions: Vec<OperationKind>,
    /// Resources the module owns (only first-party adapters may own resources in Core 1.x).
    #[serde(default)]
    pub owns: Vec<ResourceId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleDescriptor {
    pub id: String,
    pub version: Option<Version>,
    pub compatible: bool,
    pub health: ModuleHealth,
    pub detail: String,
    pub requested_permissions: Vec<OperationKind>,
    pub owns: Vec<ResourceId>,
}

pub fn valid_module_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl ModuleManifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let m: ModuleManifest = toml::from_str(text).map_err(|e| e.to_string())?;
        m.validate()?;
        Ok(m)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !valid_module_id(&self.id) {
            return Err(format!("invalid module id {:?}", self.id));
        }
        Version::parse(&self.version)
            .ok_or_else(|| format!("invalid version {:?}", self.version))?;
        let lo = Version::parse(&self.core_api[0]).ok_or("invalid core_api lower bound")?;
        let hi = Version::parse(&self.core_api[1]).ok_or("invalid core_api upper bound")?;
        if lo > hi {
            return Err("core_api range is empty".into());
        }
        for c in &self.required_capabilities {
            if !crate::capability::valid_capability_id(c) {
                return Err(format!("invalid capability id {c:?}"));
            }
        }
        Ok(())
    }

    pub fn compatible_with(&self, core: Version) -> bool {
        match (
            Version::parse(&self.core_api[0]),
            Version::parse(&self.core_api[1]),
        ) {
            (Some(lo), Some(hi)) => lo <= core && core <= hi,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(
            Version::parse("v1.2.3"),
            Some(Version {
                major: 1,
                minor: 2,
                patch: 3
            })
        );
        assert_eq!(
            Version::parse("1.2.3-rc1"),
            Some(Version {
                major: 1,
                minor: 2,
                patch: 3
            })
        );
        assert_eq!(Version::parse("1.2"), None);
        assert_eq!(Version::parse("1.2.3.4"), None);
    }

    #[test]
    fn manifest_validation() {
        let ok = "id = \"zairenkai_thermal\"\nversion = \"0.1.0\"\ncore_api = [\"1.0.0\", \"1.99.0\"]\nrequested_permissions = [\"set_low_power_mode\"]\n";
        let m = ModuleManifest::parse(ok).unwrap();
        assert!(m.compatible_with(CORE_API));
        assert!(!m.compatible_with(Version {
            major: 2,
            minor: 0,
            patch: 0
        }));
        assert!(ModuleManifest::parse(
            "id = \"Bad\"\nversion = \"0.1.0\"\ncore_api = [\"1.0.0\", \"1.0.0\"]\n"
        )
        .is_err());
        assert!(ModuleManifest::parse(
            "id = \"m\"\nversion = \"0.1.0\"\ncore_api = [\"2.0.0\", \"1.0.0\"]\n"
        )
        .is_err());
        assert!(
            ModuleManifest::parse(&format!("{ok}exec = \"/bin/sh\"\n")).is_err(),
            "unknown keys rejected"
        );
        assert!(ModuleManifest::parse("id = \"m\"\nversion = \"0.1.0\"\ncore_api = [\"1.0.0\", \"1.0.0\"]\nrequested_permissions = [\"shell\"]\n").is_err());
    }
}
