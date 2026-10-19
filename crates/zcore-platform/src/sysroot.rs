// Shared with Zairenkai-FAS (same owner): crates/zfas-platform/src/sysroot.rs.
//! File-system root abstraction.
//!
//! Every platform read and write goes through a [`SysRoot`]. Production uses `/`; tests point
//! it at a temporary directory with fixture files, so no test can touch real device nodes.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysRoot {
    root: PathBuf,
}

impl SysRoot {
    pub fn real() -> Self {
        Self {
            root: PathBuf::from("/"),
        }
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn is_real(&self) -> bool {
        self.root == Path::new("/")
    }

    /// Map an absolute device path (`/sys/...`) into this root.
    pub fn path(&self, abs: impl AsRef<Path>) -> PathBuf {
        let abs = abs.as_ref();
        let rel = abs.strip_prefix("/").unwrap_or(abs);
        self.root.join(rel)
    }

    pub fn exists(&self, abs: impl AsRef<Path>) -> bool {
        self.path(abs).exists()
    }

    pub fn read_string(&self, abs: impl AsRef<Path>) -> io::Result<String> {
        Ok(fs::read_to_string(self.path(abs))?.trim().to_string())
    }

    pub fn read_u64(&self, abs: impl AsRef<Path>) -> io::Result<u64> {
        let p = abs.as_ref().to_path_buf();
        let s = self.read_string(&p)?;
        s.parse().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: not an integer: {s:?}", p.display()),
            )
        })
    }

    pub fn read_i64(&self, abs: impl AsRef<Path>) -> io::Result<i64> {
        let p = abs.as_ref().to_path_buf();
        let s = self.read_string(&p)?;
        s.parse().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: not an integer: {s:?}", p.display()),
            )
        })
    }

    /// Write an existing node. Never creates files: a missing node is an error, so a wrong
    /// path cannot silently create a regular file in sysfs or elsewhere.
    pub fn write_existing(&self, abs: impl AsRef<Path>, value: &str) -> io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(false)
            .open(self.path(abs))?;
        f.write_all(value.as_bytes())?;
        f.flush()
    }

    pub fn list_dir(&self, abs: impl AsRef<Path>) -> io::Result<Vec<String>> {
        let mut v: Vec<String> = fs::read_dir(self.path(abs))?
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        Ok(v)
    }
}
