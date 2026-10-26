// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Checks on Zairenkai component binaries that Core runs by absolute path.
//!
//! Android's `/data` directory is platform-managed and is not necessarily root-owned (some
//! builds report UID/GID `system`). Do not require it to have the same owner as the installed
//! module. Instead, allow only the declared `ztune` and `zthermal` names beneath the root-manager
//! tree, validate Android's
//! `/data` ancestor with its platform-specific policy, and require each component-owned tree and
//! executable to be root-owned and not group/other writable. Symlinks are rejected at every path
//! component.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::sysroot::SysRoot;

const ROOT_MANAGER_DIR: &str = "/data/adb";
const ANDROID_SYSTEM_UID: u32 = 1000;
const ANDROID_SYSTEM_GID: u32 = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryError {
    /// Missing, not a regular file, or not executable.
    NotInstalled(String),
    /// The path or its ownership is not acceptable.
    Unsafe(String),
}

/// Android's top-level data directory may be owned by root or system. Group write is accepted
/// only for the conventional system group; world write is always refused. Each approved component
/// directory itself and everything below it are subject to the stricter root-owned rule in
/// `check_binary`.
fn safe_android_data_directory(uid: u32, gid: u32, mode: u32) -> bool {
    (uid == 0 || uid == ANDROID_SYSTEM_UID)
        && (mode & 0o002 == 0)
        && (mode & 0o020 == 0 || gid == ANDROID_SYSTEM_GID)
}

fn unsafe_path(message: impl Into<String>) -> Result<(), BinaryError> {
    Err(BinaryError::Unsafe(message.into()))
}

pub fn check_binary(
    root: &SysRoot,
    path: &str,
    name: &str,
    owner_uid: u32,
) -> Result<(), BinaryError> {
    if !path.starts_with('/')
        || path.split('/').skip(1).any(|c| c.is_empty() || c == ".." || c == ".")
    {
        return unsafe_path(format!("{path:?} is not a normalized absolute path"));
    }
    if path.rsplit('/').next() != Some(name) {
        return unsafe_path(format!("{path:?} is not named {name}"));
    }
    // Core only runs the two known component names and only from the root-manager's protected
    // tree. This keeps documented path overrides working without allowing arbitrary locations
    // such as /data/local/tmp. Every directory below /data/adb is validated as root-owned and
    // non-writable by group/others before the executable is handed to the runner.
    if !matches!(name, "ztune" | "zthermal") {
        return unsafe_path(format!("{name:?} is not an approved component"));
    }
    if !path.starts_with(&format!("{ROOT_MANAGER_DIR}/")) {
        return unsafe_path(format!("{path:?} is outside the protected {ROOT_MANAGER_DIR} tree"));
    }
    let trusted_root = PathBuf::from(ROOT_MANAGER_DIR);

    let local = root.path(path);
    let file_meta = match std::fs::symlink_metadata(&local) {
        Ok(m) => m,
        Err(e) => return Err(BinaryError::NotInstalled(format!("{path}: {e}"))),
    };
    if file_meta.file_type().is_symlink() || !file_meta.file_type().is_file() {
        return Err(BinaryError::NotInstalled(format!("{path} is not a regular, non-symlink file")));
    }
    if file_meta.mode() & 0o111 == 0 {
        return Err(BinaryError::NotInstalled(format!("{path} is not executable")));
    }

    let mut logical = PathBuf::from(path);
    loop {
        let mapped = root.path(&logical);
        let meta = std::fs::symlink_metadata(&mapped)
            .map_err(|e| BinaryError::NotInstalled(format!("{}: {e}", logical.display())))?;
        if meta.file_type().is_symlink() {
            return unsafe_path(format!("{} is a symlink", logical.display()));
        }
        if logical != Path::new(path) && !meta.file_type().is_dir() {
            return unsafe_path(format!("{} is not a directory", logical.display()));
        }

        let in_trusted_component = logical == trusted_root || logical.starts_with(&trusted_root);
        let mode = meta.mode();
        if in_trusted_component {
            if meta.uid() != owner_uid {
                return unsafe_path(format!(
                    "{} is owned by uid {}, not {owner_uid}",
                    logical.display(), meta.uid()
                ));
            }
            if mode & 0o022 != 0 {
                return unsafe_path(format!("{} is writable by group or others", logical.display()));
            }
        } else if logical == Path::new("/data") {
            // Test SysRoots live beneath a temporary host directory whose owner is the test user.
            // Production still applies Android's explicit UID/GID policy below.
            let acceptable = if root.is_real() {
                safe_android_data_directory(meta.uid(), meta.gid(), mode)
            } else {
                meta.uid() == owner_uid && mode & 0o022 == 0
            };
            if !acceptable {
                return unsafe_path(format!(
                    "/data has unacceptable Android ownership/permissions (uid {}, gid {}, mode {:04o})",
                    meta.uid(), meta.gid(), mode & 0o7777
                ));
            }
        } else {
            // The real platform root and the root-manager directory chain must remain root-owned.
            // Test roots are virtual, so their temporary sandbox owner substitutes for UID 0.
            let expected_owner = if root.is_real() { 0 } else { owner_uid };
            if meta.uid() != expected_owner {
                return unsafe_path(format!(
                    "{} is owned by uid {}, not {expected_owner}",
                    logical.display(), meta.uid()
                ));
            }
            if mode & 0o022 != 0 {
                return unsafe_path(format!("{} is writable by group or others", logical.display()));
            }
        }

        if logical == Path::new("/") {
            break;
        }
        logical = logical.parent().expect("non-root absolute path has a parent").to_path_buf();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;
    use std::os::unix::fs::{symlink, PermissionsExt};

    const BIN: &str = "/data/adb/modules/zairenkai/bin/ztune";
    const THERMALTOOLS_BIN: &str = "/data/adb/zairenkai/bin/zthermal";

    fn me() -> u32 {
        // SAFETY: getuid has no preconditions.
        unsafe { libc::getuid() }
    }

    fn installed() -> TempDir {
        let d = TempDir::new("component");
        d.file("data/adb/modules/zairenkai/bin/ztune", "#!/bin/false\n");
        let file = d.path().join("data/adb/modules/zairenkai/bin/ztune");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut dir = file.parent().map(Path::to_path_buf);
        while let Some(p) = dir {
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            if p == d.path() { break; }
            dir = p.parent().map(Path::to_path_buf);
        }
        d
    }

    #[test]
    fn accepts_root_owned_module_binary_under_virtual_sysroot() {
        let d = installed();
        assert_eq!(check_binary(&SysRoot::at(d.path()), BIN, "ztune", me()), Ok(()));
    }

    #[test]
    fn accepts_the_separately_installed_thermaltools_path() {
        let d = TempDir::new("thermaltools-component");
        d.file("data/adb/zairenkai/bin/zthermal", "#!/bin/false\n");
        let file = d.path().join("data/adb/zairenkai/bin/zthermal");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut dir = file.parent().map(Path::to_path_buf);
        while let Some(p) = dir {
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            if p == d.path() { break; }
            dir = p.parent().map(Path::to_path_buf);
        }
        assert_eq!(check_binary(&SysRoot::at(d.path()), THERMALTOOLS_BIN, "zthermal", me()), Ok(()));
    }

    #[test]
    fn refuses_binary_outside_the_module_bin_directory() {
        let d = installed();
        assert!(matches!(
            check_binary(&SysRoot::at(d.path()), "/data/local/tmp/ztune", "ztune", me()),
            Err(BinaryError::Unsafe(_))
        ));
    }

    #[test]
    fn refuses_a_symlinked_executable() {
        let d = installed();
        let file = d.path().join("data/adb/modules/zairenkai/bin/ztune");
        std::fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink("/bin/sh", &file).unwrap();
        assert!(matches!(
            check_binary(&SysRoot::at(d.path()), BIN, "ztune", me()),
            Err(BinaryError::NotInstalled(_))
        ));
    }

    #[test]
    fn refuses_group_or_world_writable_trusted_directories() {
        let d = installed();
        let bin_dir = d.path().join("data/adb/modules/zairenkai/bin");
        std::fs::set_permissions(&bin_dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(matches!(
            check_binary(&SysRoot::at(d.path()), BIN, "ztune", me()),
            Err(BinaryError::Unsafe(message)) if message.contains("writable by group or others")
        ));
    }

    #[test]
    fn android_data_policy_accepts_system_owned_data_but_never_world_write() {
        assert!(safe_android_data_directory(1000, 1000, 0o40771));
        assert!(safe_android_data_directory(0, 1000, 0o40771));
        assert!(!safe_android_data_directory(1000, 1000, 0o40777));
        assert!(!safe_android_data_directory(1000, 2000, 0o40771));
        assert!(!safe_android_data_directory(2000, 1000, 0o40755));
    }
}
