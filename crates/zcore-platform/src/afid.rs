//! Device knowledge from a Zairenkai-Database-Source (AFID) catalog checkout.
//!
//! Records are *data*: they can describe a device but never grant a permission or enable an
//! operation. Only records with `codename_status = "verified"` and `schema_version = "1.0"`
//! are used; anything else is reported as ignored.

use serde::{Deserialize, Serialize};

use crate::sysroot::SysRoot;

pub const DEFAULT_CATALOG: &str = "/data/adb/zairenkai-core/afid";
const MAX_RECORD: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceKnowledge {
    pub device_id: String,
    pub brand_id: String,
    pub codename: String,
    pub soc: Option<String>,
    /// Path of the record inside the catalog (provenance).
    pub record: String,
    pub sources: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lookup {
    /// No catalog present at the configured path.
    NoCatalog,
    NotFound,
    Found(DeviceKnowledge),
    /// A record exists but was rejected (schema, status or size).
    Rejected(String),
}

/// Look up a device by codename (`ro.product.device`) and brand in
/// `<catalog>/devices/<brand_id>/<codename_key>/device.json`.
pub fn lookup(root: &SysRoot, catalog: &str, brand: &str, codename: &str) -> Lookup {
    if !root.exists(catalog) {
        return Lookup::NoCatalog;
    }
    let brand_id = brand.to_ascii_lowercase();
    let key = codename.to_ascii_lowercase();
    let ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 64
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            && !s.starts_with('.')
    };
    if !ok(&brand_id) || !ok(&key) {
        return Lookup::Rejected("brand or codename contains unexpected characters".into());
    }
    let rel = format!("devices/{brand_id}/{key}/device.json");
    let path = format!("{catalog}/{rel}");
    let p = root.path(&path);
    let meta = match std::fs::symlink_metadata(&p) {
        Ok(m) => m,
        Err(_) => return Lookup::NotFound,
    };
    if !meta.is_file() || meta.len() > MAX_RECORD {
        return Lookup::Rejected(format!("{rel}: not a regular file or too large"));
    }
    let v: serde_json::Value = match std::fs::read_to_string(&p)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
    {
        Some(v) => v,
        None => return Lookup::Rejected(format!("{rel}: not valid JSON")),
    };
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    if s("schema_version").as_deref() != Some("1.0") {
        return Lookup::Rejected(format!("{rel}: unsupported schema_version"));
    }
    if s("codename_status").as_deref() != Some("verified") {
        return Lookup::Rejected(format!("{rel}: codename not verified"));
    }
    match (s("device_id"), s("brand_id"), s("codename")) {
        (Some(device_id), Some(b), Some(c))
            if b == brand_id && c.eq_ignore_ascii_case(codename) =>
        {
            Lookup::Found(DeviceKnowledge {
                device_id,
                brand_id: b,
                codename: c,
                soc: v
                    .get("soc")
                    .and_then(|x| x.get("model").or(Some(x)))
                    .and_then(|x| x.as_str())
                    .map(str::to_string),
                record: rel,
                sources: v
                    .get("sources")
                    .and_then(|x| x.as_array())
                    .map_or(0, |a| a.len()),
            })
        }
        _ => Lookup::Rejected(format!(
            "{rel}: identity fields missing or inconsistent with its path"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    const REC: &str = r#"{"schema_version":"1.0","device_id":"google_shiba","brand_id":"google","codename":"shiba","codename_status":"verified","soc":"Tensor G3","sources":[{},{}]}"#;

    #[test]
    fn lookup_cases() {
        let d = TempDir::new("afid");
        let root = SysRoot::at(d.path());
        assert_eq!(lookup(&root, "/cat", "google", "shiba"), Lookup::NoCatalog);
        d.file("cat/devices/google/shiba/device.json", REC);
        match lookup(&root, "/cat", "Google", "shiba") {
            Lookup::Found(k) => {
                assert_eq!(k.device_id, "google_shiba");
                assert_eq!(k.soc.as_deref(), Some("Tensor G3"));
                assert_eq!(k.sources, 2);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(lookup(&root, "/cat", "google", "husky"), Lookup::NotFound);
        assert!(matches!(
            lookup(&root, "/cat", "google", "../../etc"),
            Lookup::Rejected(_)
        ));
        d.file(
            "cat/devices/google/husky/device.json",
            &REC.replace("verified", "unverified")
                .replace("shiba", "husky"),
        );
        assert!(matches!(
            lookup(&root, "/cat", "google", "husky"),
            Lookup::Rejected(_)
        ));
        d.file("cat/devices/google/akita/device.json", REC);
        assert!(
            matches!(
                lookup(&root, "/cat", "google", "akita"),
                Lookup::Rejected(_)
            ),
            "record identity must match its path"
        );
    }
}
