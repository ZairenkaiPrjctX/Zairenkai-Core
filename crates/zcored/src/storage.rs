//! Persistent state: the lease table (atomic JSON) and an append-only audit log with size
//! rotation. Stored state is never executed at startup; leases are only reconciled.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zcore_domain::audit::AuditEvent;
use zcore_engine::arbitration::LeaseTable;
use zcore_platform::fsutil::write_atomic;

pub const AUDIT_MAX_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
pub struct LeaseFile {
    pub version: u32,
    pub saved_mono_ms: u64,
    pub leases: LeaseTable,
}

pub fn save_leases(path: &Path, leases: &LeaseTable, now_ms: u64) -> std::io::Result<()> {
    let f = LeaseFile {
        version: 1,
        saved_mono_ms: now_ms,
        leases: leases.clone(),
    };
    write_atomic(
        path,
        &serde_json::to_vec_pretty(&f).expect("serializable"),
        0o600,
    )
}

/// Load leases. A corrupt file is moved aside (never trusted) and reported.
pub fn load_leases(path: &Path) -> Result<Option<LeaseFile>, String> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    match serde_json::from_str::<LeaseFile>(&text) {
        Ok(f) if f.version == 1 => Ok(Some(f)),
        Ok(f) => Err(format!(
            "{}: unsupported version {}",
            path.display(),
            f.version
        )),
        Err(e) => {
            let aside = path.with_extension("json.corrupt");
            let _ = fs::rename(path, &aside);
            Err(format!(
                "{}: corrupt ({e}); moved to {}",
                path.display(),
                aside.display()
            ))
        }
    }
}

pub struct AuditSink {
    path: PathBuf,
}

impl AuditSink {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn append(&self, events: &[AuditEvent]) -> std::io::Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        if fs::metadata(&self.path)
            .map(|m| m.len() > AUDIT_MAX_BYTES)
            .unwrap_or(false)
        {
            fs::rename(&self.path, self.path.with_extension("jsonl.1"))?;
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        for e in events {
            let mut line = serde_json::to_vec(e).expect("serializable");
            line.push(b'\n');
            f.write_all(&line)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zcore_domain::decision::PolicyTier;
    use zcore_domain::principal::Principal;
    use zcore_domain::resource::{ResourceId, ResourceValue};

    #[test]
    fn leases_roundtrip_and_corruption() {
        let d = zcore_platform::testutil::TempDir::new("store");
        let p = d.path().join("leases.json");
        assert!(load_leases(&p).unwrap().is_none());
        let mut t = LeaseTable::default();
        t.grant(
            ResourceId::FasEnabled,
            Principal::Root,
            PolicyTier::User,
            ResourceValue::Bool(false),
            Some(ResourceValue::Bool(true)),
            5,
            None,
            "r",
        );
        save_leases(&p, &t, 10).unwrap();
        let f = load_leases(&p).unwrap().unwrap();
        assert_eq!(f.leases, t);
        assert_eq!(f.saved_mono_ms, 10);
        std::fs::write(&p, "{not json").unwrap();
        assert!(load_leases(&p).is_err());
        assert!(!p.exists(), "corrupt file moved aside");
        assert!(d.path().join("leases.json.corrupt").exists());
    }
}
