// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Checks on Zairenkai component binaries that Core runs by absolute path.
//!
//! Core runs as root, so a component binary that a non-root user could replace would be an
//! escalation path. Before anything runs, the configured file and every directory above it
//! must be owned by `owner_uid` (0 in production) and not writable by group or others.

use std::os::unix::fs::MetadataExt;

use crate::sysroot::SysRoot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryError {
    /// Missing, not a regular file, or not executable.
    NotInstalled(String),
    /// The path or its ownership is not acceptable.
    Unsafe(String),
}

pub fn check_binary(
    root: &SysRoot,
    path: &str,
    name: &str,
    owner_uid: u32,
) -> Result<(), BinaryError> {
    let unsafe_ = |m: String| Err(BinaryError::Unsafe(m));
    if !path.starts_with('/') || path.split('/').any(|c| c == ".." || c == ".") {
        return unsafe_(format!("{path:?} is not an absolute path without . or .."));
    }
    if path.rsplit('/').next() != Some(name) {
        return unsafe_(format!("{path:?} is not named {name}"));
    }
    let local = root.path(path);
    let meta = match std::fs::symlink_metadata(&local) {
        Ok(m) => m,
        Err(e) => return Err(BinaryError::NotInstalled(format!("{path}: {e}"))),
    };
    if !meta.file_type().is_file() {
        return Err(BinaryError::NotInstalled(format!(
            "{path} is not a regular file"
        )));
    }
    if meta.mode() & 0o111 == 0 {
        return Err(BinaryError::NotInstalled(format!(
            "{path} is not executable"
        )));
    }
    let mut p = Some(local.as_path());
    while let Some(cur) = p {
        let m = std::fs::metadata(cur)
            .map_err(|e| BinaryError::NotInstalled(format!("{}: {e}", cur.display())))?;
        if m.uid() != owner_uid {
            return unsafe_(format!(
                "{} is owned by uid {}, not {owner_uid}",
                cur.display(),
                m.uid()
            ));
        }
        if m.mode() & 0o022 != 0 {
            return unsafe_(format!("{} is writable by group or others", cur.display()));
        }
        if cur == root.path("/") {
            break;
        }
        p = cur.parent();
    }
    Ok(())
}
