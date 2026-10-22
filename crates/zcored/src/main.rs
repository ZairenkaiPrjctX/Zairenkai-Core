// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! `zcored`: Zairenkai-Core daemon and CLI.

mod daemon;
mod engine;
mod ratelimit;
mod server;
mod storage;
mod util;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use zcore_engine::adapter::Adapter;
use zcore_engine::config::CoreConfig;
use zcore_platform::exec::SystemRunner;
use zcore_platform::sysroot::SysRoot;

use crate::daemon::{DaemonOptions, DEFAULT_DIR};
use crate::engine::Platform;

const USAGE: &str = "\
zcored - Zairenkai-Core control plane

USAGE:
  zcored run [--dir DIR] [--config FILE]      run the daemon (root)
  zcored discover                             read-only device and capability report (JSON)
  zcored ctl [--dir DIR] METHOD [PARAMS_JSON] send one request to the daemon
      METHOD: health | capabilities | context | modules | leases | policy | audit
              submit | recommend | release
      e.g. zcored ctl submit '{\"operation\":{\"op\":\"fas_set_profile\",\"profile\":\"balanced\"},\"dry_run\":true}'
  zcored bench [ITERATIONS]                   host CPU cost of policy evaluation and the pipeline
  zcored check-config FILE
  zcored default-config
  zcored version
";

fn opt(args: &mut Vec<String>, name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.remove(i);
    (i < args.len()).then(|| args.remove(i))
}

fn platform() -> Platform {
    Platform {
        root: SysRoot::real(),
        runner: Arc::new(SystemRunner),
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprint!("{USAGE}");
        std::process::exit(64);
    }
    let cmd = args.remove(0);
    let dir = PathBuf::from(opt(&mut args, "--dir").unwrap_or_else(|| DEFAULT_DIR.into()));
    let code = match cmd.as_str() {
        "run" => {
            let config_path = opt(&mut args, "--config")
                .map(PathBuf::from)
                .unwrap_or_else(|| dir.join("config.toml"));
            daemon::run(DaemonOptions {
                dir,
                config_path,
                platform: platform(),
                stop: None,
            })
        }
        "discover" => discover(),
        "ctl" => ctl(&dir, &args),
        "check-config" => match args.first() {
            Some(f) => match std::fs::read_to_string(f)
                .map_err(|e| e.to_string())
                .and_then(|t| CoreConfig::parse(&t))
            {
                Ok((c, notes)) => {
                    for n in notes {
                        println!("note: {n}");
                    }
                    println!("ok (policy {})", c.policy_version());
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    2
                }
            },
            None => 64,
        },
        "bench" => bench(args.first().and_then(|n| n.parse().ok()).unwrap_or(20_000)),
        "default-config" => {
            print!(
                "{}",
                toml::to_string_pretty(&CoreConfig::default()).unwrap_or_default()
            );
            0
        }
        "version" => {
            println!(
                "zcored {} (core api {}, protocol {})",
                env!("CARGO_PKG_VERSION"),
                zcore_domain::module::CORE_API,
                zcore_domain::protocol::PROTOCOL_VERSION
            );
            0
        }
        _ => {
            eprint!("{USAGE}");
            64
        }
    };
    std::process::exit(code);
}

fn discover() -> i32 {
    let p = platform();
    let now = util::monotonic_ms();
    let cfg = CoreConfig::default();
    let identity = zcore_platform::discovery::discover_identity(&p.root, p.runner.as_ref(), now);
    let telemetry = zcore_platform::telemetry::collect(
        &p.root,
        p.runner.as_ref(),
        now,
        cfg.telemetry.max_context_age_ms,
    );
    let mut caps = zcore_platform::telemetry::capabilities(&telemetry, now);
    let marker = zcore_platform::fas::module_marker(zcore_platform::modules::MODULES_DIR);
    let root = p.root.clone();
    let mut fas = zcore_platform::fas::FasAdapter::new(
        zcore_platform::fas::DEFAULT_SOCKET,
        Box::new(move || root.exists(&marker)),
    );
    caps.extend(fas.discover(now));
    let mut power = zcore_platform::power::LowPowerAdapter::new(
        p.runner.clone(),
        identity.api_level.value.value().copied(),
    );
    caps.extend(power.discover(now));
    caps.push(zcore_platform::discovery::power_hal_capability(
        p.runner.as_ref(),
        now,
    ));
    let modules = zcore_platform::modules::inventory(&p.root, zcore_platform::modules::MODULES_DIR);
    let out = serde_json::json!({ "identity": identity, "telemetry": telemetry, "capabilities": caps, "modules": modules });
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    0
}

fn ctl(dir: &std::path::Path, args: &[String]) -> i32 {
    let Some(method) = args.first() else {
        eprint!("{USAGE}");
        return 64;
    };
    let params: Option<serde_json::Value> = match args.get(1) {
        Some(p) => match serde_json::from_str(p) {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("ctl: params are not JSON: {e}");
                return 64;
            }
        },
        None => None,
    };
    let id = format!("cli-{}-{}", std::process::id(), util::monotonic_ms());
    let mut req = serde_json::json!({ "v": zcore_domain::protocol::PROTOCOL_VERSION, "id": id, "method": method });
    if let Some(p) = params {
        req["params"] = p;
    }
    let sock = dir.join("zcored.sock");
    let result = (|| -> std::io::Result<String> {
        let mut s = UnixStream::connect(&sock)?;
        s.set_read_timeout(Some(Duration::from_secs(15)))?;
        s.write_all(format!("{req}\n").as_bytes())?;
        let mut out = String::new();
        BufReader::new(s).read_line(&mut out)?;
        Ok(out)
    })();
    match result {
        Ok(out) => {
            let v: serde_json::Value =
                serde_json::from_str(&out).unwrap_or(serde_json::Value::String(out));
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            if v.get("ok").and_then(|o| o.as_bool()) == Some(true) {
                0
            } else {
                1
            }
        }
        Err(e) => {
            eprintln!("ctl: cannot reach zcored at {}: {e}", sock.display());
            2
        }
    }
}

/// Host CPU time of the decision hot paths with an in-memory adapter (no device I/O).
fn bench(n: usize) -> i32 {
    use zcore_domain::device::{DeviceContext, ThermalStatus};
    use zcore_domain::observation::Observation;
    use zcore_domain::principal::Principal;
    use zcore_domain::request::{ActionRequest, Operation, Origin};
    use zcore_domain::resource::{FasProfile, ResourceId, ResourceValue};
    use zcore_domain::units::Percent;
    use zcore_engine::adapter::FakeAdapter;
    let fake = FakeAdapter::new(
        "fake",
        &[(
            ResourceId::FasProfile,
            ResourceValue::FasProfile(FasProfile::Balanced),
        )],
    );
    let mut core = zcore_engine::Core::new(CoreConfig::default(), vec![Box::new(fake)]);
    core.refresh_capabilities(0);
    let mut ctx = DeviceContext::unknown();
    ctx.collected_at_ms = 1;
    ctx.telemetry.thermal_status =
        Observation::measured(ThermalStatus::None, "bench", 1, u64::MAX / 2);
    ctx.telemetry.battery_level =
        Observation::measured(Percent::new(80).unwrap(), "bench", 1, u64::MAX / 2);
    core.set_context(ctx);
    let mut samples = Vec::with_capacity(n);
    for i in 0..n.max(1) {
        let profile = if i % 2 == 0 {
            FasProfile::Performance
        } else {
            FasProfile::Balanced
        };
        let req = ActionRequest {
            id: format!("b{i}"),
            principal: Principal::Root,
            origin: Origin::User,
            operation: Operation::FasSetProfile { profile },
            duration_ms: None,
            dry_run: false,
            confirmed: false,
            reason: String::new(),
        };
        let t0 = std::time::Instant::now();
        std::hint::black_box(core.submit(req, 1));
        samples.push(t0.elapsed().as_nanos() as f64 / 1000.0);
        let _ = core.audit.drain_pending();
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let k = samples.len();
    let out = serde_json::json!({
        "note": "host CPU time per full submit (policy, arbitration, plan, in-memory execute, read-back, lease, audit); excludes adapter I/O; not a device measurement",
        "iterations": k,
        "mean_us": samples.iter().sum::<f64>() / k as f64,
        "p50_us": samples[k / 2],
        "p99_us": samples[((k as f64 * 0.99) as usize).min(k - 1)],
        "max_us": samples[k - 1],
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    0
}
