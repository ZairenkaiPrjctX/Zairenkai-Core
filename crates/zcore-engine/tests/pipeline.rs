//! End-to-end pipeline tests with the in-memory adapter and fault injection.

use zcore_domain::audit::AuditKind;
use zcore_domain::capability::CapabilityStatus;
use zcore_domain::decision::ActionOutcome;
use zcore_domain::device::{DeviceContext, ThermalStatus};
use zcore_domain::observation::Observation;
use zcore_domain::principal::Principal;
use zcore_domain::request::{ActionRequest, Operation, OperationKind, Origin};
use zcore_domain::resource::{FasProfile, ResourceId, ResourceValue};
use zcore_domain::units::Percent;
use zcore_engine::adapter::{AdapterErrorCode, FakeAdapter, FakeHandle, Faults};
use zcore_engine::config::{CoreConfig, PrincipalMap, Role};
use zcore_engine::Core;

const BAL: ResourceValue = ResourceValue::FasProfile(FasProfile::Balanced);
const PERF_V: ResourceValue = ResourceValue::FasProfile(FasProfile::Performance);
const PERF: Operation = Operation::FasSetProfile {
    profile: FasProfile::Performance,
};

fn ctx(now: u64, thermal: ThermalStatus) -> DeviceContext {
    let mut c = DeviceContext::unknown();
    c.collected_at_ms = now;
    c.telemetry.thermal_status = Observation::measured(thermal, "t", now, 10_000);
    c.telemetry.battery_level = Observation::measured(Percent::new(80).unwrap(), "t", now, 10_000);
    c.telemetry.charging = Observation::measured(false, "t", now, 10_000);
    c
}

fn core_with(faults: Faults) -> Core {
    core_and_handle(faults).0
}

fn core_and_handle(faults: Faults) -> (Core, FakeHandle) {
    let fake = FakeAdapter::new(
        "fake",
        &[
            (ResourceId::FasProfile, BAL),
            (ResourceId::FasEnabled, ResourceValue::Bool(true)),
            (ResourceId::LowPowerMode, ResourceValue::Bool(false)),
        ],
    );
    let handle = fake.handle();
    handle.lock().unwrap().faults = faults;
    let mut cfg = CoreConfig::default();
    cfg.principals.push(PrincipalMap {
        uid: 10300,
        role: Role::IntelligenceEngine,
        grants: vec![OperationKind::FasSetProfile],
    });
    cfg.principals.push(PrincipalMap {
        uid: 10234,
        role: Role::App,
        grants: vec![OperationKind::FasSetProfile],
    });
    let mut c = Core::new(cfg, vec![Box::new(fake)]);
    c.refresh_capabilities(0);
    c.set_context(ctx(1_000, ThermalStatus::None));
    (c, handle)
}

fn req(
    id: &str,
    p: Principal,
    origin: Origin,
    op: Operation,
    duration: Option<u64>,
) -> ActionRequest {
    ActionRequest {
        id: id.into(),
        principal: p,
        origin,
        operation: op,
        duration_ms: duration,
        dry_run: false,
        confirmed: false,
        reason: "test".into(),
    }
}

fn read(c: &mut Core) -> ResourceValue {
    // Observe through a dry run of a no-op: the plan carries the current value.
    let mut r = req(
        "probe",
        Principal::Root,
        Origin::User,
        Operation::FasSetProfile {
            profile: FasProfile::PowerSaver,
        },
        None,
    );
    r.dry_run = true;
    c.submit(r, 1_000).plan.unwrap().current.unwrap()
}

#[test]
fn allowed_request_is_applied_verified_leased_and_audited() {
    let mut c = core_with(Faults::default());
    let r = c.submit(req("a1", Principal::Root, Origin::User, PERF, None), 1_000);
    assert_eq!(r.outcome, ActionOutcome::Applied, "{}", r.message);
    assert!(r.verification.as_ref().unwrap().matched);
    assert_eq!(
        r.plan.as_ref().unwrap().rollback.as_ref().unwrap().restore,
        BAL
    );
    assert!(r.lease_id.is_some());
    assert!(c
        .capabilities()
        .iter()
        .any(|d| d.id == "fas.profile" && d.verified_on_device));
    let kinds: Vec<_> = c
        .audit
        .tail(10)
        .into_iter()
        .filter(|e| e.correlation_id.as_deref() == Some("a1"))
        .map(|e| e.event)
        .collect();
    assert!(matches!(kinds[0], AuditKind::RequestEvaluated { .. }));
    assert!(matches!(
        kinds[1],
        AuditKind::ActionCompleted {
            outcome: ActionOutcome::Applied,
            ..
        }
    ));
    assert!(c.take_leases_dirty());
}

#[test]
fn dry_run_executes_nothing() {
    let mut c = core_with(Faults::default());
    let mut r = req("d1", Principal::Root, Origin::User, PERF, None);
    r.dry_run = true;
    let res = c.submit(r, 1_000);
    assert_eq!(res.outcome, ActionOutcome::DryRun);
    assert_eq!(read(&mut c), BAL);
    assert!(c.leases().all().is_empty());
}

#[test]
fn unauthorized_and_unknown_principals_are_rejected_without_execution() {
    let mut c = core_with(Faults::default());
    let res = c.submit(
        req("u1", Principal::Unknown(10999), Origin::User, PERF, None),
        1_000,
    );
    assert_eq!(res.outcome, ActionOutcome::Rejected);
    assert!(res.message.contains("no grant"));
    let res = c.submit(
        req(
            "u2",
            Principal::App(10234),
            Origin::User,
            Operation::SetLowPowerMode { enabled: true },
            None,
        ),
        1_000,
    );
    assert_eq!(res.outcome, ActionOutcome::Rejected);
    assert_eq!(read(&mut c), BAL);
}

#[test]
fn missing_capability_is_unsupported() {
    let mut c = core_with(Faults {
        status: Some(CapabilityStatus::Unsupported),
        ..Default::default()
    });
    c.refresh_capabilities(0);
    let res = c.submit(req("m1", Principal::Root, Origin::User, PERF, None), 1_000);
    assert_eq!(res.outcome, ActionOutcome::Unsupported);
}

#[test]
fn stale_or_hot_context_blocks_raising() {
    let mut c = core_with(Faults::default());
    let res = c.submit(req("s1", Principal::Root, Origin::User, PERF, None), 60_000);
    assert_eq!(res.outcome, ActionOutcome::Rejected);
    c.set_context(ctx(60_000, ThermalStatus::Critical));
    let res = c.submit(req("s2", Principal::Root, Origin::User, PERF, None), 60_000);
    assert_eq!(res.outcome, ActionOutcome::Rejected);
    assert!(res.message.contains("thermal"));
    // Lowering stays possible when hot.
    let res = c.submit(
        req(
            "s3",
            Principal::Root,
            Origin::User,
            Operation::SetLowPowerMode { enabled: true },
            None,
        ),
        60_000,
    );
    assert_eq!(res.outcome, ActionOutcome::Applied, "{}", res.message);
}

#[test]
fn adapter_failure_and_timeout_are_reported_not_hidden() {
    let mut c = core_with(Faults {
        apply_error: Some(AdapterErrorCode::Unavailable),
        ..Default::default()
    });
    let res = c.submit(req("f1", Principal::Root, Origin::User, PERF, None), 1_000);
    assert_eq!(res.outcome, ActionOutcome::Failed);
    assert!(c.leases().all().is_empty());
    let mut c = core_with(Faults {
        apply_error: Some(AdapterErrorCode::Timeout),
        ..Default::default()
    });
    let res = c.submit(req("f2", Principal::Root, Origin::User, PERF, None), 1_000);
    assert_eq!(res.outcome, ActionOutcome::TimedOut);
}

#[test]
fn unverifiable_write_is_not_reported_as_applied() {
    let mut c = core_with(Faults {
        ignore_writes: true,
        ..Default::default()
    });
    let res = c.submit(req("v1", Principal::Root, Origin::User, PERF, None), 1_000);
    assert_eq!(res.outcome, ActionOutcome::Unverified);
    assert!(!res.verification.unwrap().matched);
    assert!(c.leases().all().is_empty());
}

#[test]
fn recommendation_cannot_override_user_choice_and_is_time_bounded() {
    let mut c = core_with(Faults::default());
    let res = c.submit(
        req(
            "r1",
            Principal::IntelligenceEngine,
            Origin::Recommendation { confidence: 0.9 },
            Operation::FasSetProfile {
                profile: FasProfile::FastResponse,
            },
            None,
        ),
        1_000,
    );
    assert_eq!(res.outcome, ActionOutcome::Applied, "{}", res.message);
    let lease = c.leases().get(ResourceId::FasProfile).unwrap().clone();
    assert_eq!(
        lease.expires_at_ms,
        Some(1_000 + CoreConfig::default().safety.recommendation_max_ms)
    );
    // The user takes over (higher tier) ...
    assert_eq!(
        c.submit(req("u1", Principal::Root, Origin::User, PERF, None), 1_000)
            .outcome,
        ActionOutcome::Applied
    );
    // ... and a later recommendation is rejected as a conflict.
    let res = c.submit(
        req(
            "r2",
            Principal::IntelligenceEngine,
            Origin::Recommendation { confidence: 0.95 },
            Operation::FasSetProfile {
                profile: FasProfile::Balanced,
            },
            None,
        ),
        1_000,
    );
    assert_eq!(res.outcome, ActionOutcome::Rejected);
    assert!(res.message.contains("conflict"), "{}", res.message);
    // Another app at the same tier cannot silently override either.
    let res = c.submit(
        req(
            "a1",
            Principal::App(10234),
            Origin::User,
            Operation::FasSetProfile {
                profile: FasProfile::PowerSaver,
            },
            None,
        ),
        1_000,
    );
    assert_eq!(res.outcome, ActionOutcome::Rejected);
}

#[test]
fn temporary_override_expires_and_restores_baseline() {
    let mut c = core_with(Faults::default());
    c.submit(
        req("t1", Principal::Root, Origin::User, PERF, Some(5_000)),
        1_000,
    );
    assert_eq!(c.reconcile(5_999), 0);
    assert_eq!(c.reconcile(6_000), 1);
    assert_eq!(read(&mut c), BAL);
    assert!(c
        .audit
        .tail(10)
        .iter()
        .any(|e| matches!(e.event, AuditKind::BaselineRestored { ok: true, .. })));
}

#[test]
fn external_change_is_respected_not_fought() {
    let (mut c, h) = core_and_handle(Faults::default());
    c.submit(
        req("x1", Principal::Root, Origin::User, PERF, Some(5_000)),
        1_000,
    );
    // The user changes the profile in the FAS WebUI, behind Core's back.
    h.lock().unwrap().values.insert(
        ResourceId::FasProfile,
        ResourceValue::FasProfile(FasProfile::PowerSaver),
    );
    let applies = h.lock().unwrap().applies;
    assert_eq!(c.reconcile(2_000), 1);
    assert!(c.leases().all().is_empty(), "lease dropped");
    assert_eq!(h.lock().unwrap().applies, applies, "nothing re-forced");
    assert_eq!(
        h.lock().unwrap().values[&ResourceId::FasProfile],
        ResourceValue::FasProfile(FasProfile::PowerSaver)
    );
    assert!(c
        .audit
        .tail(10)
        .iter()
        .any(|e| matches!(e.event, AuditKind::ExternallyModified { .. })));
    // Expiry later does not "restore" over the user's change either.
    assert_eq!(c.reconcile(10_000), 0);
    assert_eq!(
        h.lock().unwrap().values[&ResourceId::FasProfile],
        ResourceValue::FasProfile(FasProfile::PowerSaver)
    );
}

#[test]
fn shutdown_restores_temporary_but_keeps_user_settings() {
    let mut c = core_with(Faults::default());
    c.submit(req("p1", Principal::Root, Origin::User, PERF, None), 1_000);
    c.submit(
        req(
            "p2",
            Principal::Root,
            Origin::User,
            Operation::SetLowPowerMode { enabled: true },
            Some(60_000),
        ),
        1_000,
    );
    c.shutdown(2_000);
    assert_eq!(read(&mut c), PERF_V, "indefinite user choice kept");
    assert_eq!(c.leases().all().len(), 1);
}

#[test]
fn release_restores_and_checks_ownership() {
    let mut c = core_with(Faults::default());
    let res = c.submit(
        req(
            "l1",
            Principal::App(10234),
            Origin::User,
            Operation::FasSetProfile {
                profile: FasProfile::PowerSaver,
            },
            None,
        ),
        1_000,
    );
    let id = res.lease_id.unwrap();
    assert!(c.release(id, &Principal::App(10999), 1_100).is_err());
    let r = c.release(id, &Principal::App(10234), 1_100).unwrap();
    assert_eq!(r.outcome, ActionOutcome::Applied);
    assert_eq!(read(&mut c), BAL);
    assert!(c.release(id, &Principal::Root, 1_100).is_err());
}

const FAST: ResourceValue = ResourceValue::FasProfile(FasProfile::FastResponse);

#[test]
fn wrong_effect_is_rolled_back() {
    // The first write lands on the wrong value; the rollback write works.
    let (mut c, h) = core_and_handle(Faults {
        write_instead: Some(FAST),
        write_instead_times: Some(1),
        ..Default::default()
    });
    let res = c.submit(req("w1", Principal::Root, Origin::User, PERF, None), 1_000);
    assert_eq!(res.outcome, ActionOutcome::RolledBack, "{}", res.message);
    assert_eq!(res.verification.unwrap().observed, Some(FAST));
    assert_eq!(h.lock().unwrap().values[&ResourceId::FasProfile], BAL);
    assert!(c.leases().all().is_empty());
}

#[test]
fn failed_rollback_is_reported_as_unverified() {
    // Every write lands on the wrong value, so the baseline cannot be restored either.
    let (mut c, h) = core_and_handle(Faults {
        write_instead: Some(FAST),
        ..Default::default()
    });
    let res = c.submit(req("w2", Principal::Root, Origin::User, PERF, None), 1_000);
    assert_eq!(res.outcome, ActionOutcome::Unverified);
    assert_eq!(h.lock().unwrap().values[&ResourceId::FasProfile], FAST);
    assert!(c
        .audit
        .tail(10)
        .iter()
        .any(|e| matches!(e.event, AuditKind::BaselineRestored { ok: false, .. })));
    assert!(c.leases().all().is_empty());
}

#[test]
fn rollback_write_error_is_reported() {
    // Wrong first write, then the rollback write itself errors.
    let (mut c, h) = core_and_handle(Faults {
        write_instead: Some(FAST),
        write_instead_times: Some(1),
        fail_nth_apply: Some(2),
        ..Default::default()
    });
    let res = c.submit(req("pc", Principal::Root, Origin::User, PERF, None), 1_000);
    assert_eq!(res.outcome, ActionOutcome::Unverified);
    assert_eq!(h.lock().unwrap().values[&ResourceId::FasProfile], FAST);
    assert!(c
        .audit
        .tail(10)
        .iter()
        .any(|e| matches!(e.event, AuditKind::BaselineRestored { ok: false, .. })));
}

#[test]
fn unreadable_resource_is_not_changed() {
    // The current value cannot be read: no verification or rollback would be possible.
    let (mut c, h) = core_and_handle(Faults::default());
    h.lock().unwrap().faults.read_error = Some(AdapterErrorCode::PermissionDenied);
    let res = c.submit(req("rf", Principal::Root, Origin::User, PERF, None), 1_000);
    assert_eq!(res.outcome, ActionOutcome::Rejected);
    assert!(res.message.contains("unreadable"));
    assert_eq!(h.lock().unwrap().applies, 0);
    // A dry run still shows the plan, with no rollback available.
    let mut r = req("rf2", Principal::Root, Origin::User, PERF, None);
    r.dry_run = true;
    let plan = c.submit(r, 1_000).plan.unwrap();
    assert!(plan.rollback.is_none());
    assert!(plan.preconditions.iter().any(|p| p.contains("unreadable")));
}
