// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Clock and process helpers.

use std::os::unix::io::AsRawFd;
use std::path::Path;

/// `CLOCK_MONOTONIC` in ms.
pub fn monotonic_ms() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid pointer to a timespec.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1000 + ts.tv_nsec as u64 / 1_000_000
}

/// Hold an exclusive lock for the life of the process: a second daemon refuses to start.
pub struct InstanceLock(#[allow(dead_code)] std::fs::File);

pub fn lock_instance(path: &Path) -> std::io::Result<InstanceLock> {
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    // SAFETY: valid fd.
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(std::io::Error::other(format!(
            "another zcored holds {}",
            path.display()
        )));
    }
    Ok(InstanceLock(f))
}
