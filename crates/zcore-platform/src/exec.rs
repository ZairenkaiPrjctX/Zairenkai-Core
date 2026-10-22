// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Running Android shell tools with an executable allowlist, argument validation, a timeout
//! and an output cap. There is no shell and no free-form command: every caller passes a fixed
//! argv built from typed values.

use std::io::{self, Read};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const MAX_OUTPUT: usize = 1024 * 1024;

/// Programs Core may run, and nothing else.
pub const ALLOWED_PROGRAMS: [&str; 5] = ["getprop", "dumpsys", "cmd", "settings", "service"];

/// Zairenkai component binaries Core may run by absolute path (from configuration). Their
/// owning adapter checks file ownership and the subcommand before calling the runner.
pub const ALLOWED_COMPONENTS: [&str; 2] = ["zthermal", "ztune"];

fn allowed_component(prog: &str) -> bool {
    prog.starts_with('/')
        && !prog.split('/').any(|c| c == ".." || c == ".")
        && prog.len() <= 256
        && prog
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c))
        && prog
            .rsplit('/')
            .next()
            .is_some_and(|n| ALLOWED_COMPONENTS.contains(&n))
}

#[derive(Debug)]
pub enum ExecError {
    NotAllowed(String),
    Spawn(io::Error),
    Timeout,
    Failed { status: Option<i32>, stderr: String },
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::NotAllowed(e) => write!(f, "not allowed: {e}"),
            ExecError::Spawn(e) => write!(f, "spawn failed: {e}"),
            ExecError::Timeout => write!(f, "timed out"),
            ExecError::Failed { status, stderr } => write!(f, "exit status {status:?}: {stderr}"),
        }
    }
}

impl std::error::Error for ExecError {}

/// Arguments: short, printable, no shell metacharacters (defence in depth; no shell is used).
pub fn valid_arg(a: &str) -> bool {
    !a.is_empty()
        && a.len() <= 128
        && a.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-/:=@".contains(c))
}

pub fn check_argv(argv: &[&str]) -> Result<(), ExecError> {
    let (prog, args) = argv
        .split_first()
        .ok_or_else(|| ExecError::NotAllowed("empty argv".into()))?;
    if !ALLOWED_PROGRAMS.contains(prog) && !allowed_component(prog) {
        return Err(ExecError::NotAllowed(format!(
            "program {prog:?} is not on the allowlist"
        )));
    }
    if let Some(a) = args.iter().find(|a| !valid_arg(a)) {
        return Err(ExecError::NotAllowed(format!("argument {a:?} rejected")));
    }
    Ok(())
}

pub trait Runner: Send {
    fn run(&self, argv: &[&str], timeout: Duration) -> Result<String, ExecError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, argv: &[&str], timeout: Duration) -> Result<String, ExecError> {
        check_argv(argv)?;
        let mut child = Command::new(argv[0])
            .args(&argv[1..])
            .env_clear()
            .env("PATH", "/system/bin:/system/xbin:/vendor/bin:/usr/bin:/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(ExecError::Spawn)?;
        let mut out = child.stdout.take().expect("piped");
        let mut err = child.stderr.take().expect("piped");
        let reader = thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = (&mut out).take(MAX_OUTPUT as u64).read_to_end(&mut buf);
            let _ = io::copy(&mut out, &mut io::sink());
            buf
        });
        let err_reader = thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = (&mut err).take(16 * 1024).read_to_end(&mut buf);
            let _ = io::copy(&mut err, &mut io::sink());
            buf
        });
        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait().map_err(ExecError::Spawn)? {
                Some(s) => break s,
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ExecError::Timeout);
                }
                None => thread::sleep(Duration::from_millis(5)),
            }
        };
        let stdout = reader.join().unwrap_or_default();
        let stderr = err_reader.join().unwrap_or_default();
        if !status.success() {
            return Err(ExecError::Failed {
                status: status.code(),
                stderr: String::from_utf8_lossy(&stderr).chars().take(300).collect(),
            });
        }
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    }
}

/// Canned outputs keyed by the joined argv, with the same allowlist check as production.
#[derive(Debug, Default)]
pub struct FakeRunner {
    pub outputs: std::sync::Mutex<std::collections::HashMap<String, Result<String, String>>>,
    pub calls: std::sync::Mutex<Vec<String>>,
}

impl FakeRunner {
    pub fn with(self, argv: &str, out: &str) -> Self {
        self.set(argv, Ok(out.to_string()));
        self
    }

    pub fn set(&self, argv: &str, out: Result<String, String>) {
        self.outputs.lock().unwrap().insert(argv.to_string(), out);
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Runner for FakeRunner {
    fn run(&self, argv: &[&str], _timeout: Duration) -> Result<String, ExecError> {
        check_argv(argv)?;
        let key = argv.join(" ");
        self.calls.lock().unwrap().push(key.clone());
        match self.outputs.lock().unwrap().get(&key) {
            Some(Ok(s)) => Ok(s.clone()),
            Some(Err(e)) if e == "timeout" => Err(ExecError::Timeout),
            Some(Err(e)) => Err(ExecError::Failed {
                status: Some(1),
                stderr: e.clone(),
            }),
            None => Err(ExecError::Spawn(io::Error::new(
                io::ErrorKind::NotFound,
                key,
            ))),
        }
    }
}

impl<R: Runner + Sync> Runner for std::sync::Arc<R> {
    fn run(&self, argv: &[&str], timeout: Duration) -> Result<String, ExecError> {
        (**self).run(argv, timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_and_argument_validation() {
        assert!(check_argv(&["getprop"]).is_ok());
        assert!(check_argv(&["settings", "get", "global", "low_power"]).is_ok());
        assert!(check_argv(&["sh", "-c", "id"]).is_err());
        assert!(check_argv(&["/system/bin/getprop"]).is_err());
        assert!(check_argv(&["/data/adb/zairenkai/bin/zthermal", "snapshot"]).is_ok());
        assert!(check_argv(&["zthermal", "snapshot"]).is_err());
        assert!(check_argv(&[
            "/data/adb/modules/zairenkai/bin/ztune",
            "apply",
            "io.scheduler",
            "mq-deadline"
        ])
        .is_ok());
        assert!(check_argv(&[
            "/data/adb/modules/zairenkai/bin/ztune",
            "apply",
            "io.scheduler",
            "a b"
        ])
        .is_err());
        assert!(check_argv(&["/data/../bin/zthermal", "snapshot"]).is_err());
        assert!(check_argv(&["/data/adb/zthermalx", "snapshot"]).is_err());
        assert!(check_argv(&["/data/adb/zthermal", "snapshot;id"]).is_err());
        assert!(check_argv(&["cmd", "power", "set-mode", "1;reboot"]).is_err());
        assert!(check_argv(&["settings", "get", "global", "$(id)"]).is_err());
        assert!(check_argv(&[]).is_err());
        assert!(matches!(
            SystemRunner.run(&["rm", "-rf", "/tmp/x"], Duration::from_secs(1)),
            Err(ExecError::NotAllowed(_))
        ));
    }
}
