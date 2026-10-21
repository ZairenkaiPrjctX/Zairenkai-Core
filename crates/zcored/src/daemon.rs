// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Daemon lifecycle: Starting → Ready (serving) → Stopping → Stopped.
//!
//! - single instance (flock on `zcored.lock`);
//! - invalid configuration refuses to start (exit 2) instead of running with guesses;
//! - SIGTERM/SIGINT: stop accepting, let the engine end temporary leases and persist state;
//! - SIGHUP: reload configuration (rejected configs keep the previous one, audited).

use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::net::UnixListener;
use tokio::sync::{oneshot, watch, Semaphore};
use zcore_engine::config::CoreConfig;

use crate::engine::{self, EngineOptions, Msg, Platform};
use crate::ratelimit::RateLimiter;
use crate::server::{self, Shared, MAX_CONNECTIONS};
use crate::util::{lock_instance, monotonic_ms};

pub const DEFAULT_DIR: &str = "/data/adb/zairenkai-core";
/// The engine heartbeat may be at most this old.
pub const WATCHDOG_MS: u64 = 90_000;

pub struct DaemonOptions {
    pub dir: PathBuf,
    pub config_path: PathBuf,
    pub platform: Platform,
    /// Test hook: stop when this flips to true (in addition to signals).
    pub stop: Option<watch::Receiver<bool>>,
}

pub fn load_config(path: &Path) -> Result<CoreConfig, String> {
    match std::fs::read_to_string(path) {
        Ok(t) => CoreConfig::parse(&t)
            .map(|(c, _)| c)
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CoreConfig::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn bind(path: &Path) -> std::io::Result<UnixListener> {
    if let Ok(m) = std::fs::symlink_metadata(path) {
        if m.file_type().is_socket() {
            std::fs::remove_file(path)?;
        } else {
            return Err(std::io::Error::other(format!(
                "{} exists and is not a socket",
                path.display()
            )));
        }
    }
    let l = UnixListener::bind(path)?;
    // Root-only: the socket lives under a 0700 directory and is 0600 itself.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(l)
}

pub fn run(opts: DaemonOptions) -> i32 {
    if let Err(e) = std::fs::create_dir_all(&opts.dir)
        .and_then(|_| std::fs::set_permissions(&opts.dir, std::fs::Permissions::from_mode(0o700)))
    {
        eprintln!("zcored: {}: {e}", opts.dir.display());
        return 1;
    }
    let _lock = match lock_instance(&opts.dir.join("zcored.lock")) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("zcored: {e}");
            return 3;
        }
    };
    let config = match load_config(&opts.config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("zcored: refusing to start with an invalid configuration: {e}");
            return 2;
        }
    };
    let ipc = config.ipc.clone();
    let (tx, rx) = sync_channel::<Msg>(ipc.queue);
    let (ready_tx, ready_rx) = sync_channel(1);
    let eopts = EngineOptions {
        dir: opts.dir.clone(),
        config_path: opts.config_path.clone(),
        config,
        platform: opts.platform.clone(),
    };
    let heartbeat = Arc::new(AtomicU64::new(monotonic_ms()));
    let hb = heartbeat.clone();
    let engine = std::thread::Builder::new()
        .name("zcore-engine".into())
        .spawn(move || engine::run(eopts, rx, ready_tx, hb))
        .expect("spawn engine");
    if ready_rx.recv_timeout(Duration::from_secs(120)).is_err() {
        eprintln!("zcored: engine failed to start");
        return 1;
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let sock = opts.dir.join("zcored.sock");
    let code = rt.block_on(async {
        let listener = match bind(&sock) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("zcored: socket {}: {e}", sock.display());
                return 1;
            }
        };
        let (stop_tx, stop_rx) = watch::channel(false);
        let shared = Shared {
            tx: tx.clone(),
            ipc: ipc.clone(),
            limiter: Arc::new(Mutex::new(RateLimiter::new(ipc.rate_per_sec, ipc.burst))),
            conns: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
        };
        let srv = tokio::spawn(server::serve(listener, shared, stop_rx));
        // Watchdog: the engine loop wakes at least every refresh interval. If it stalls (an
        // adapter blocked past its own timeouts), abort so the supervisor restarts Core; the
        // next start reconciles leases and restores expired temporary overrides.
        let hb = heartbeat.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(5));
            loop {
                tick.tick().await;
                let age = monotonic_ms().saturating_sub(hb.load(Ordering::SeqCst));
                if age > WATCHDOG_MS {
                    eprintln!("zcored: engine stalled for {age} ms; aborting for supervised restart");
                    std::process::abort();
                }
            }
        });
        eprintln!("zcored {}: ready on {}", env!("CARGO_PKG_VERSION"), sock.display());

        use tokio::signal::unix::{signal, SignalKind};
        let (Ok(mut term), Ok(mut int), Ok(mut hup)) =
            (signal(SignalKind::terminate()), signal(SignalKind::interrupt()), signal(SignalKind::hangup()))
        else {
            eprintln!("zcored: cannot install signal handlers");
            return 1;
        };
        let mut test_stop = opts.stop.clone();
        loop {
            tokio::select! {
                _ = term.recv() => break,
                _ = int.recv() => break,
                _ = hup.recv() => { let _ = tx.try_send(Msg::Reload); }
                r = async { match test_stop.as_mut() { Some(s) => s.changed().await, None => std::future::pending().await } } => {
                    if r.is_err() || test_stop.as_ref().is_some_and(|s| *s.borrow()) { break; }
                }
            }
        }
        eprintln!("zcored: stopping");
        let _ = stop_tx.send(true);
        let _ = srv.await;
        let (otx, orx) = oneshot::channel();
        let tx2 = tx.clone();
        let sent = tokio::task::spawn_blocking(move || tx2.send(Msg::Shutdown { reply: otx }).is_ok()).await.unwrap_or(false);
        if sent && tokio::time::timeout(Duration::from_secs(30), orx).await.is_err() {
            eprintln!("zcored: engine did not stop in time");
            return 4;
        }
        0
    });
    drop(tx);
    let _ = std::fs::remove_file(&sock);
    let _ = engine.join();
    eprintln!("zcored: stopped");
    code
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener as StdListener, UnixStream};
    use zcore_platform::exec::{check_argv, ExecError, Runner};
    use zcore_platform::sysroot::SysRoot;
    use zcore_platform::testutil::TempDir;

    /// Android shell tools with state: `cmd power set-mode` changes what `settings get` reports.
    struct DeviceSim {
        low_power: Mutex<bool>,
    }

    impl Runner for DeviceSim {
        fn run(&self, argv: &[&str], _t: Duration) -> Result<String, ExecError> {
            check_argv(argv)?;
            let mut lp = self.low_power.lock().unwrap();
            Ok(match argv.join(" ").as_str() {
                "getprop" => "[ro.build.version.sdk]: [34]\n[ro.product.brand]: [google]\n[ro.product.device]: [shiba]\n[ro.product.model]: [Pixel 8]\n[ro.product.cpu.abi]: [arm64-v8a]\n".into(),
                "dumpsys thermalservice" => "Thermal Status: 0\n".into(),
                "settings get global low_power" => if *lp { "1\n".into() } else { "0\n".into() },
                "cmd power set-mode 1" => { *lp = true; String::new() }
                "cmd power set-mode 0" => { *lp = false; String::new() }
                "service check android.hardware.power.IPower/default" => "Service android.hardware.power.IPower/default: found\n".into(),
                other => return Err(ExecError::Failed { status: Some(1), stderr: format!("unexpected {other}") }),
            })
        }
    }

    fn fake_zfasd(path: std::path::PathBuf) {
        let l = StdListener::bind(path).unwrap();
        std::thread::spawn(move || {
            let mut profile = "balanced".to_string();
            for c in l.incoming() {
                let Ok(mut c) = c else { break };
                let mut line = String::new();
                BufReader::new(&c).read_line(&mut line).unwrap();
                let reply = match line.trim() {
                    "status" => format!(
                        r#"{{"running":true,"enabled":true,"default_profile":"{profile}","observe_only":false}}"#
                    ),
                    l if l.starts_with("profile ") => {
                        profile = l[8..].to_string();
                        r#"{"ok":true}"#.into()
                    }
                    _ => r#"{"ok":true}"#.into(),
                };
                let _ = c.write_all(format!("{reply}\n").as_bytes());
            }
        });
    }

    fn call(sock: &Path, line: &str) -> serde_json::Value {
        let mut s = UnixStream::connect(sock).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s.write_all(format!("{line}\n").as_bytes()).unwrap();
        let mut out = String::new();
        BufReader::new(s).read_line(&mut out).unwrap();
        serde_json::from_str(&out).unwrap()
    }

    #[test]
    fn end_to_end_over_the_socket() {
        let d = TempDir::new("zcored");
        d.file("sys/class/power_supply/battery/capacity", "80");
        d.file("sys/class/power_supply/battery/status", "Discharging");
        d.file("proc/meminfo", "MemAvailable: 1000 kB\n");
        d.file(
            "data/adb/modules/zairenkai_fas/module.prop",
            "id=zairenkai_fas\nversion=v0.1.0\n",
        );
        std::fs::create_dir_all(d.path().join("data/adb/zairenkai-fas")).unwrap();
        fake_zfasd(d.path().join("data/adb/zairenkai-fas/zfasd.sock"));
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        let principals = if uid == 0 {
            String::new()
        } else {
            format!("[[principals]]\nuid = {uid}\nrole = \"app\"\ngrants = [\"fas_set_profile\", \"fas_set_enabled\", \"set_low_power_mode\"]\n")
        };
        let dir = d.path().join("core");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), format!("[telemetry]\nrefresh_ms = 500\nmax_context_age_ms = 5000\ndiscovery_ms = 1000\n{principals}")).unwrap();
        let (stop_tx, stop_rx) = watch::channel(false);
        let opts = DaemonOptions {
            dir: dir.clone(),
            config_path: dir.join("config.toml"),
            platform: Platform {
                root: SysRoot::at(d.path()),
                runner: Arc::new(DeviceSim {
                    low_power: Mutex::new(false),
                }),
            },
            stop: Some(stop_rx),
        };
        let h = std::thread::spawn(move || run(opts));
        let sock = dir.join("zcored.sock");
        for _ in 0..200 {
            if sock.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        // A second instance refuses to start.
        let second = DaemonOptions {
            dir: dir.clone(),
            config_path: dir.join("config.toml"),
            platform: Platform {
                root: SysRoot::at(d.path()),
                runner: Arc::new(DeviceSim {
                    low_power: Mutex::new(false),
                }),
            },
            stop: None,
        };
        assert_eq!(run(second), 3);

        let health = call(&sock, r#"{"v":1,"id":"h1","method":"health"}"#);
        assert_eq!(health["ok"], true, "{health}");
        assert_eq!(health["result"]["state"], "ready", "{health}");

        let ctx = call(&sock, r#"{"v":1,"id":"c1","method":"context"}"#);
        assert_eq!(
            ctx["result"]["context"]["identity"]["device"]["value"]["value"],
            "shiba"
        );
        assert_eq!(
            ctx["result"]["context"]["telemetry"]["fas"]["value"]["value"]["default_profile"],
            "balanced"
        );

        let r = call(
            &sock,
            r#"{"v":1,"id":"s1","method":"submit","params":{"operation":{"op":"fas_set_profile","profile":"performance"}}}"#,
        );
        assert_eq!(r["result"]["outcome"], "applied", "{r}");
        assert_eq!(r["result"]["verification"]["matched"], true);
        let dup = call(
            &sock,
            r#"{"v":1,"id":"s1","method":"submit","params":{"operation":{"op":"fas_set_profile","profile":"performance"}}}"#,
        );
        assert_eq!(dup["error"]["code"], "bad_request");

        // Temporary battery saver: applied, then restored by reconciliation after expiry.
        let r = call(
            &sock,
            r#"{"v":1,"id":"s2","method":"submit","params":{"operation":{"op":"set_low_power_mode","enabled":true},"duration_ms":800}}"#,
        );
        assert_eq!(r["result"]["outcome"], "applied", "{r}");
        let mut restored = false;
        for _ in 0..60 {
            std::thread::sleep(Duration::from_millis(100));
            let leases = call(&sock, r#"{"v":1,"id":"l1","method":"leases"}"#);
            if leases["result"].as_array().unwrap().len() == 1 {
                restored = true;
                break;
            }
        }
        assert!(restored, "temporary lease expired and was reconciled");
        let audit = call(
            &sock,
            r#"{"v":1,"id":"a1","method":"audit","params":{"limit":100}}"#,
        );
        assert!(
            audit["result"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["event"]["kind"] == "baseline_restored" && e["event"]["ok"] == true),
            "{audit}"
        );

        assert_eq!(
            call(
                &sock,
                r#"{"v":1,"id":"x","method":"exec","params":{"cmd":"id"}}"#
            )["error"]["code"],
            "bad_request"
        );
        assert_eq!(
            call(&sock, r#"{"v":2,"id":"x","method":"health"}"#)["error"]["code"],
            "unsupported_version"
        );
        let mods = call(&sock, r#"{"v":1,"id":"m1","method":"modules"}"#);
        let fas = mods["result"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == "zairenkai_fas")
            .unwrap()
            .clone();
        assert_eq!(fas["health"], "healthy");
        assert!(mods["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "zairenkai_thermaltools"
                && m["health"] == "not_installed"
                && m["detail"].as_str().unwrap().contains("not configured")));
        // Without a configured zthermal binary the diagnostics capability is unsupported.
        let caps = call(&sock, r#"{"v":1,"id":"k1","method":"capabilities"}"#);
        let tt = caps["result"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == "thermaltools.diagnostics")
            .unwrap()
            .clone();
        assert_eq!(tt["status"], "unsupported", "{tt}");

        stop_tx.send(true).unwrap();
        assert_eq!(h.join().unwrap(), 0);
        assert!(!sock.exists());
        assert!(dir.join("leases.json").exists());
        let audit_file = std::fs::read_to_string(dir.join("audit.jsonl")).unwrap();
        assert!(audit_file.contains("\"stopping\""));
    }

    #[test]
    fn invalid_config_refuses_to_start() {
        let d = TempDir::new("zcored-cfg");
        std::fs::write(
            d.path().join("config.toml"),
            "[safety]\nbattery_critical_percent = 99\n",
        )
        .unwrap();
        let opts = DaemonOptions {
            dir: d.path().to_path_buf(),
            config_path: d.path().join("config.toml"),
            platform: Platform {
                root: SysRoot::at(d.path()),
                runner: Arc::new(DeviceSim {
                    low_power: Mutex::new(false),
                }),
            },
            stop: None,
        };
        assert_eq!(run(opts), 2);
    }
}
