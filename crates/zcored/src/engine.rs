// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! The engine thread: owns [`Core`], the platform adapters and all device I/O.
//!
//! Everything that touches the device runs here, one message at a time, so operations on the
//! same resource can never interleave. The async side only parses, authenticates the peer, and
//! forwards over a bounded queue.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::oneshot;
use zcore_domain::audit::AuditKind;
use zcore_domain::capability::CapabilityStatus;
use zcore_domain::device::DeviceContext;
use zcore_domain::module::{ModuleDescriptor, ModuleHealth, CORE_API};
use zcore_domain::observation::{Availability, Observation};
use zcore_domain::principal::Principal;
use zcore_domain::protocol::{ErrorCode, Method, Request, Response};
use zcore_domain::request::{ActionRequest, Origin};
use zcore_engine::adapter::Adapter;
use zcore_engine::config::CoreConfig;
use zcore_engine::Core;
use zcore_platform::afid::{self, Lookup};
use zcore_platform::exec::Runner;
use zcore_platform::fas::{FasAdapter, MODULE_ID as FAS_MODULE};
use zcore_platform::power::LowPowerAdapter;
use zcore_platform::sysroot::SysRoot;
use zcore_platform::{discovery, modules, telemetry, thermaltools};

use crate::storage::{self, AuditSink};
use crate::util::monotonic_ms;

pub enum Msg {
    Request {
        uid: u32,
        req: Request,
        reply: oneshot::Sender<Response>,
    },
    Reload,
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

/// Where the platform lives; tests point everything at a fixture root and fake runner.
#[derive(Clone)]
pub struct Platform {
    pub root: SysRoot,
    pub runner: Arc<dyn Runner + Sync>,
}

pub struct EngineOptions {
    pub dir: PathBuf,
    pub config_path: PathBuf,
    pub config: CoreConfig,
    pub platform: Platform,
}

/// First-party components without a released implementation are listed honestly.
const THERMALTOOLS_MODULE: &str = "zairenkai_thermaltools";

const NOT_YET_AVAILABLE: [(&str, &str); 7] = [
    ("zairenkai_thermaltools", "Zairenkai-ThermalTools"),
    ("zairenkai_sdk", "Zairenkai-SDK"),
    ("zairenkai_license_server", "Zairenkai-License-Server"),
    ("zairenkai_intelligence", "Zairenkai-Intelligence-Engine"),
    ("zairenkai_graphics", "Zairenkai-Graphics-Platform"),
    ("zairenkai_art", "Zairenkai-ART"),
    ("zairenkai_modules", "Zairenkai-Modules"),
];

struct Engine {
    core: Core,
    opts: EngineOptions,
    fas_probe: FasAdapter,
    knowledge: Lookup,
    started_ms: u64,
    sink: AuditSink,
    recent_ids: VecDeque<(String, String)>,
}

fn fas_socket(cfg: &CoreConfig) -> String {
    cfg.path("fas_socket", zcore_platform::fas::DEFAULT_SOCKET)
}

fn modules_dir(cfg: &CoreConfig) -> String {
    cfg.path("modules_dir", modules::MODULES_DIR)
}

fn fas_adapter(p: &Platform, cfg: &CoreConfig) -> FasAdapter {
    let root = p.root.clone();
    let marker = format!("{}/{FAS_MODULE}", modules_dir(cfg));
    FasAdapter::new(
        p.root.path(fas_socket(cfg)),
        Box::new(move || root.exists(&marker)),
    )
}

impl Engine {
    fn new(opts: EngineOptions) -> Self {
        let now = monotonic_ms();
        let p = opts.platform.clone();
        let identity = discovery::discover_identity(&p.root, p.runner.as_ref(), now);
        let api = identity.api_level.value.value().copied();
        let adapters: Vec<Box<dyn Adapter>> = vec![
            Box::new(fas_adapter(&p, &opts.config)),
            Box::new(LowPowerAdapter::new(p.runner.clone(), api)),
        ];
        let mut core = Core::new(opts.config.clone(), adapters);
        let mut ctx = DeviceContext::unknown();
        ctx.identity = identity;
        core.set_context(ctx);
        let sink = AuditSink::new(opts.dir.join("audit.jsonl"));
        let mut e = Engine {
            fas_probe: fas_adapter(&p, &opts.config),
            core,
            knowledge: Lookup::NoCatalog,
            started_ms: now,
            sink,
            recent_ids: VecDeque::new(),
            opts,
        };
        e.discover(now);
        e.refresh_context(now);
        match storage::load_leases(&e.opts.dir.join("leases.json")) {
            Ok(Some(f)) => e.core.load_leases(f.leases, f.saved_mono_ms, now),
            Ok(None) => {}
            Err(err) => {
                e.core.audit.record(
                    now,
                    None,
                    None,
                    AuditKind::ConfigRejected {
                        error: format!("lease file: {err}"),
                    },
                );
            }
        }
        let version = format!("zcored {} (core api {CORE_API})", env!("CARGO_PKG_VERSION"));
        let policy_version = e.core.policy().version().to_string();
        e.core.audit.record(
            now,
            None,
            Some(&Principal::Core),
            AuditKind::Started {
                version,
                policy_version,
            },
        );
        // Reconcile what a previous run left (expired temporary overrides, external changes).
        e.core.reconcile(now);
        e.persist(now);
        e
    }

    fn discover(&mut self, now: u64) {
        self.core.refresh_capabilities(now);
        let p = &self.opts.platform;
        let hal = discovery::power_hal_capability(p.runner.as_ref(), now);
        self.core.report_capabilities("discovery", vec![hal], now);
        let mut mods = modules::inventory(&p.root, &modules_dir(&self.opts.config));
        for (id, name) in NOT_YET_AVAILABLE {
            if !mods.iter().any(|m| m.id == id) {
                mods.push(ModuleDescriptor {
                    id: id.into(),
                    version: None,
                    compatible: false,
                    health: ModuleHealth::NotInstalled,
                    detail: format!(
                        "{name}: no released component; Core has no adapter for it yet"
                    ),
                    requested_permissions: vec![],
                    owns: vec![],
                });
            }
        }
        // ThermalTools has no module; it is reachable only through a configured binary.
        let tt_bin = self.opts.config.paths.get(thermaltools::PATH_KEY).cloned();
        let tt = thermaltools::run(
            &p.root,
            p.runner.as_ref(),
            tt_bin.as_deref(),
            thermaltools::Query::Snapshot,
        );
        for m in mods
            .iter_mut()
            .filter(|m| m.id == THERMALTOOLS_MODULE && m.health == ModuleHealth::NotInstalled)
        {
            match &tt {
                Ok(r) => {
                    m.health = ModuleHealth::Healthy;
                    m.compatible = true;
                    m.version = zcore_domain::module::Version::parse(&r.tool_version);
                    m.detail = format!(
                        "zthermal {} (snapshot schema {}): read-only diagnostics, no controls",
                        r.tool_version, r.schema_version
                    );
                }
                Err(e) => m.detail = format!("Zairenkai-ThermalTools: {e}"),
            }
        }
        let tt_caps = thermaltools::capabilities(&tt, tt_bin.as_deref(), now);
        self.core
            .report_capabilities(thermaltools::OWNER, tt_caps, now);
        // The FAS module's health comes from its adapter.
        let fas_status = self
            .core
            .capabilities()
            .into_iter()
            .find(|c| c.id == "fas.profile")
            .map(|c| c.status);
        for m in mods
            .iter_mut()
            .filter(|m| m.id == FAS_MODULE && m.health != ModuleHealth::Disabled)
        {
            m.health = match fas_status {
                Some(CapabilityStatus::ReadWrite) => ModuleHealth::Healthy,
                Some(CapabilityStatus::ReadOnly) => ModuleHealth::Degraded,
                _ => ModuleHealth::Unreachable,
            };
        }
        self.core.set_modules(mods);
        let id = &self.core.context().identity;
        self.knowledge = match (id.brand.value.value(), id.device.value.value()) {
            (Some(b), Some(d)) => afid::lookup(
                &p.root,
                &self.opts.config.path("afid_catalog", afid::DEFAULT_CATALOG),
                b,
                d,
            ),
            _ => Lookup::NotFound,
        };
    }

    fn refresh_context(&mut self, now: u64) {
        let p = &self.opts.platform;
        let ttl = self.opts.config.telemetry.max_context_age_ms;
        let mut t = telemetry::collect(&p.root, p.runner.as_ref(), now, ttl);
        t.fas = match self.fas_probe.state() {
            Ok(s) => Observation::measured(s, "zfasd status", now, ttl),
            Err(e) => Observation::missing(
                Availability::Unavailable(e.to_string()),
                "zfasd status",
                now,
                ttl,
            ),
        };
        let caps = telemetry::capabilities(&t, now);
        let mut ctx = self.core.context().clone();
        ctx.telemetry = t;
        ctx.collected_at_ms = now;
        self.core.set_context(ctx);
        self.core.report_capabilities("telemetry", caps, now);
    }

    fn persist(&mut self, now: u64) {
        if self.core.take_leases_dirty() {
            if let Err(e) =
                storage::save_leases(&self.opts.dir.join("leases.json"), self.core.leases(), now)
            {
                eprintln!("zcored: saving leases: {e}");
            }
        }
        let events = self.core.audit.drain_pending();
        if let Err(e) = self.sink.append(&events) {
            eprintln!("zcored: audit log: {e}");
        }
    }

    fn reload(&mut self, now: u64) {
        let text = std::fs::read_to_string(&self.opts.config_path).unwrap_or_default();
        match CoreConfig::parse(&text) {
            Ok((cfg, _)) => {
                self.core.set_config(cfg.clone());
                self.opts.config = cfg;
                self.core.audit.record(
                    now,
                    None,
                    Some(&Principal::Core),
                    AuditKind::Started {
                        version: "config reloaded".into(),
                        policy_version: self.core.policy().version().into(),
                    },
                );
            }
            Err(e) => {
                self.core.audit.record(
                    now,
                    None,
                    Some(&Principal::Core),
                    AuditKind::ConfigRejected { error: e },
                );
            }
        }
    }

    fn health(&self, now: u64) -> serde_json::Value {
        let ctx = self.core.context();
        let age = now.saturating_sub(ctx.collected_at_ms);
        let mut reasons = vec![];
        if age > self.opts.config.telemetry.max_context_age_ms {
            reasons.push(format!("device context is {age} ms old"));
        }
        for (r, st) in self.core.controllable() {
            if st != CapabilityStatus::ReadWrite {
                reasons.push(format!("{} is {st:?}", r.as_str()));
            }
        }
        json!({
            "state": if reasons.is_empty() { "ready" } else { "degraded" },
            "reasons": reasons,
            "version": env!("CARGO_PKG_VERSION"),
            "core_api": CORE_API.to_string(),
            "protocol": zcore_domain::protocol::PROTOCOL_VERSION,
            "policy_version": self.core.policy().version(),
            "uptime_ms": now.saturating_sub(self.started_ms),
            "context_age_ms": age,
            "leases": self.core.leases().all().len(),
            "audit_dropped": self.core.audit.dropped_pending,
        })
    }

    fn duplicate(&mut self, principal: &Principal, id: &str) -> bool {
        let key = (principal.label(), id.to_string());
        if self.recent_ids.contains(&key) {
            return true;
        }
        if self.recent_ids.len() == 256 {
            self.recent_ids.pop_front();
        }
        self.recent_ids.push_back(key);
        false
    }

    fn handle(&mut self, uid: u32, req: Request, now: u64) -> Response {
        let principal = self.core.policy().principal_for_uid(uid);
        if matches!(principal, Principal::Unknown(_)) {
            return Response::err(
                &req.id,
                ErrorCode::PermissionDenied,
                format!("uid {uid} is not a configured Core client"),
            );
        }
        let to_json = |v: &dyn erased::Ser| v.value();
        match req.method {
            Method::Health => Response::ok(&req.id, self.health(now)),
            Method::Capabilities => Response::ok(&req.id, to_json(&self.core.capabilities())),
            Method::Context => Response::ok(
                &req.id,
                json!({ "context": self.core.context(), "knowledge": self.knowledge }),
            ),
            Method::Modules => Response::ok(&req.id, to_json(&self.core.modules().to_vec())),
            Method::Leases => Response::ok(&req.id, to_json(&self.core.leases().all().to_vec())),
            Method::Policy => {
                let c = self.core.policy().config();
                Response::ok(
                    &req.id,
                    json!({
                        "version": self.core.policy().version(),
                        "safety": c.safety,
                        "root_grants": c.root_grants,
                        "principals": c.principals,
                        "rules": c.rules,
                    }),
                )
            }
            Method::Audit { limit } => Response::ok(&req.id, to_json(&self.core.audit.tail(limit))),
            Method::Submit {
                operation,
                duration_ms,
                dry_run,
                confirmed,
                reason,
            } => {
                if principal == Principal::IntelligenceEngine {
                    return Response::err(
                        &req.id,
                        ErrorCode::PermissionDenied,
                        "the Intelligence Engine must use 'recommend'",
                    );
                }
                if self.duplicate(&principal, &req.id) {
                    return Response::err(&req.id, ErrorCode::BadRequest, "duplicate request id");
                }
                let ar = ActionRequest {
                    id: req.id.clone(),
                    principal,
                    origin: Origin::User,
                    operation,
                    duration_ms,
                    dry_run,
                    confirmed,
                    reason,
                };
                let r = self.core.submit(ar, now);
                Response::ok(&req.id, to_json(&r))
            }
            Method::Recommend {
                operation,
                confidence,
                duration_ms,
                reason,
            } => {
                if principal != Principal::IntelligenceEngine {
                    return Response::err(
                        &req.id,
                        ErrorCode::PermissionDenied,
                        "only the configured Intelligence Engine may recommend",
                    );
                }
                if self.duplicate(&principal, &req.id) {
                    return Response::err(&req.id, ErrorCode::BadRequest, "duplicate request id");
                }
                let ar = ActionRequest {
                    id: req.id.clone(),
                    principal,
                    origin: Origin::Recommendation { confidence },
                    operation,
                    duration_ms,
                    dry_run: false,
                    confirmed: false,
                    reason,
                };
                Response::ok(&req.id, to_json(&self.core.submit(ar, now)))
            }
            Method::Release { lease_id } => match self.core.release(lease_id, &principal, now) {
                Ok(r) => Response::ok(&req.id, to_json(&r)),
                Err(e) => Response::err(&req.id, ErrorCode::NotFound, e),
            },
        }
    }

    fn shutdown(&mut self) {
        let now = monotonic_ms();
        self.core.shutdown(now);
        self.persist(now);
    }
}

/// Tiny helper so heterogeneous serializable values become JSON without repetition.
mod erased {
    pub trait Ser {
        fn value(&self) -> serde_json::Value;
    }
    impl<T: serde::Serialize> Ser for T {
        fn value(&self) -> serde_json::Value {
            serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
        }
    }
}

/// Run the engine loop on the current thread until `Shutdown` or all senders are gone.
pub fn run(
    opts: EngineOptions,
    rx: Receiver<Msg>,
    ready: SyncSender<()>,
    heartbeat: Arc<AtomicU64>,
) {
    let mut e = Engine::new(opts);
    let _ = ready.send(());
    let refresh = e.opts.config.telemetry.refresh_ms;
    let mut next_refresh = monotonic_ms() + refresh;
    let mut next_discovery = monotonic_ms() + e.opts.config.telemetry.discovery_ms;
    loop {
        let now = monotonic_ms();
        heartbeat.store(now, Ordering::SeqCst);
        let wait = next_refresh.saturating_sub(now).max(1);
        match rx.recv_timeout(Duration::from_millis(wait)) {
            Ok(Msg::Request { uid, req, reply }) => {
                let resp = e.handle(uid, req, monotonic_ms());
                let _ = reply.send(resp);
            }
            Ok(Msg::Reload) => e.reload(monotonic_ms()),
            Ok(Msg::Shutdown { reply }) => {
                e.shutdown();
                let _ = reply.send(());
                return;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                e.shutdown();
                return;
            }
        }
        let now = monotonic_ms();
        if now >= next_discovery {
            e.discover(now);
            next_discovery = now + e.opts.config.telemetry.discovery_ms;
        }
        if now >= next_refresh {
            e.refresh_context(now);
            e.core.reconcile(now);
            next_refresh = now + e.opts.config.telemetry.refresh_ms;
        }
        e.persist(now);
    }
}
