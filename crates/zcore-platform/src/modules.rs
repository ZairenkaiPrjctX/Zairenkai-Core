// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Installed root-manager modules (Magisk / KernelSU / APatch share `/data/adb/modules`).
//!
//! Only `module.prop` key=value lines and the optional `zairenkai-module.toml` manifest are
//! read. Nothing from a module directory is executed or loaded.

use zcore_domain::module::{ModuleDescriptor, ModuleHealth, ModuleManifest, Version, CORE_API};

use crate::sysroot::SysRoot;

pub const MODULES_DIR: &str = "/data/adb/modules";
/// The single Zairenkai root module (Zairenkai-Modules): zcored, and zfasd when packaged.
pub const UNIFIED_ID: &str = "zairenkai";
/// Data directory shared by the unified module's components (`core/`, `fas/`).
pub const DATA_DIR: &str = "/data/adb/zairenkai";
pub const MANIFEST: &str = "zairenkai-module.toml";
const MAX_FILE: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleProp {
    pub id: String,
    pub name: Option<String>,
    pub version: Option<String>,
    pub version_code: Option<u64>,
}

pub fn parse_module_prop(text: &str) -> Option<ModuleProp> {
    let mut id = None;
    let mut p = ModuleProp {
        id: String::new(),
        name: None,
        version: None,
        version_code: None,
    };
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().to_string();
        match k.trim() {
            "id" => id = Some(v),
            "name" => p.name = Some(v),
            "version" => p.version = Some(v),
            "versionCode" => p.version_code = v.parse().ok(),
            _ => {}
        }
    }
    p.id = id.filter(|i| {
        !i.is_empty()
            && i.len() <= 64
            && i.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    })?;
    Some(p)
}

fn small_file(root: &SysRoot, path: &str) -> Option<String> {
    let p = root.path(path);
    let meta = std::fs::symlink_metadata(&p).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        return None;
    }
    std::fs::read_to_string(p).ok()
}

/// Zairenkai modules installed on the device (ids starting with `zairenkai`).
pub fn inventory(root: &SysRoot, dir: &str) -> Vec<ModuleDescriptor> {
    let mut out = vec![];
    for name in root.list_dir(dir).unwrap_or_default() {
        let base = format!("{dir}/{name}");
        let Some(prop) = small_file(root, &format!("{base}/module.prop"))
            .as_deref()
            .and_then(parse_module_prop)
        else {
            continue;
        };
        if !prop.id.starts_with("zairenkai") {
            continue;
        }
        let disabled = root.exists(format!("{base}/disable"));
        let removing = root.exists(format!("{base}/remove"));
        let manifest =
            small_file(root, &format!("{base}/{MANIFEST}")).map(|t| ModuleManifest::parse(&t));
        let (compatible, mut detail, perms, owns) = match &manifest {
            None => (true, "no zairenkai-module.toml; known first-party modules are handled by built-in adapters".to_string(), vec![], vec![]),
            Some(Ok(m)) if m.id != prop.id => (false, format!("manifest id {} does not match module.prop id", m.id), vec![], vec![]),
            Some(Ok(m)) if !m.compatible_with(CORE_API) => (false, format!("requires Core API {}..{}, this Core is {CORE_API}", m.core_api[0], m.core_api[1]), m.requested_permissions.clone(), vec![]),
            Some(Ok(m)) => (true, "manifest valid (declarations only; grants come from Core config)".to_string(), m.requested_permissions.clone(), m.owns.clone()),
            Some(Err(e)) => (false, format!("invalid manifest: {e}"), vec![], vec![]),
        };
        let health = if removing || disabled {
            ModuleHealth::Disabled
        } else if compatible {
            ModuleHealth::Unknown
        } else {
            ModuleHealth::Degraded
        };
        if removing {
            detail = format!("scheduled for removal; {detail}");
        }
        out.push(ModuleDescriptor {
            id: prop.id,
            version: prop.version.as_deref().and_then(Version::parse),
            compatible,
            health,
            detail,
            requested_permissions: perms,
            owns,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn module_prop() {
        let p = parse_module_prop(
            "id=zairenkai_fas\nname=Zairenkai-FAS\nversion=v0.1.0\nversionCode=100\n",
        )
        .unwrap();
        assert_eq!(p.id, "zairenkai_fas");
        assert_eq!(p.version_code, Some(100));
        assert!(parse_module_prop("name=x\n").is_none());
        assert!(parse_module_prop("id=../../etc\n").is_none());
    }

    #[test]
    fn inventory_reads_declarations_only() {
        let d = TempDir::new("mods");
        let m = "data/adb/modules";
        d.file(
            &format!("{m}/zairenkai_fas/module.prop"),
            "id=zairenkai_fas\nversion=v0.1.0\n",
        );
        d.file(
            &format!("{m}/zairenkai_thermal/module.prop"),
            "id=zairenkai_thermal\nversion=0.2.0\n",
        );
        d.file(
            &format!("{m}/zairenkai_thermal/zairenkai-module.toml"),
            "id = \"zairenkai_thermal\"\nversion = \"0.2.0\"\ncore_api = [\"2.0.0\", \"2.9.0\"]\n",
        );
        d.file(
            &format!("{m}/zairenkai_old/module.prop"),
            "id=zairenkai_old\n",
        );
        d.file(&format!("{m}/zairenkai_old/disable"), "");
        d.file(&format!("{m}/other/module.prop"), "id=other\n");
        let inv = inventory(&SysRoot::at(d.path()), "/data/adb/modules");
        let get = |id: &str| inv.iter().find(|x| x.id == id).unwrap().clone();
        assert_eq!(inv.len(), 3, "non-Zairenkai modules ignored");
        assert!(get("zairenkai_fas").compatible);
        assert_eq!(get("zairenkai_fas").version.unwrap().to_string(), "0.1.0");
        assert!(!get("zairenkai_thermal").compatible);
        assert_eq!(get("zairenkai_thermal").health, ModuleHealth::Degraded);
        assert_eq!(get("zairenkai_old").health, ModuleHealth::Disabled);
    }
}
