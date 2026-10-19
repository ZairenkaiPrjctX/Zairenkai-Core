//! Deterministic, typed policy evaluation. Deny by default.
//!
//! Evaluation order (all rules are evaluated, so the explanation is complete):
//! 1. `builtin.request_valid`      (integrity)  structural validation
//! 2. `builtin.authorized`         (integrity)  principal holds a grant for the operation kind
//! 3. `builtin.capability`         (safety)     the resource is ReadWrite on this device
//! 4. `builtin.context_fresh`      (safety)     raising actions need a fresh device context
//! 5. `builtin.thermal`            (safety)     no raising action at SEVERE or hotter; an
//!    unknown thermal status blocks raising unless configured otherwise
//! 6. `builtin.battery_critical`   (safety)     no raising action at critical battery, unplugged
//! 7. `builtin.battery_saver_while_charging` (safety) battery saver cannot change while plugged in
//! 8. `builtin.recommendation`     (adaptive)   recommendations need confidence and a role
//! 9. configured rules             (thermal_power) restrict-only: deny or require
//!    confirmation
//!
//! Any Deny wins. Otherwise an unconfirmed NeedsConfirmation wins. Otherwise Allow.

use zcore_domain::capability::CapabilityStatus;
use zcore_domain::decision::{PolicyDecision, PolicyTier, RuleOutcome, Verdict};
use zcore_domain::device::{DeviceContext, ThermalStatus};
use zcore_domain::principal::Principal;
use zcore_domain::request::{ActionRequest, Direction, OperationKind, Origin};
use zcore_domain::resource::ResourceValue;

use crate::config::{CoreConfig, OriginMatch, Role, RuleEffect};
use crate::registry::CapabilityRegistry;

pub struct PolicyEngine {
    cfg: CoreConfig,
    version: String,
}

pub struct EvalInput<'a> {
    pub request: &'a ActionRequest,
    pub context: &'a DeviceContext,
    pub registry: &'a CapabilityRegistry,
    /// Current value of the target resource, if it could be read.
    pub current: Option<ResourceValue>,
    pub now_ms: u64,
}

fn outcome(
    rule: &str,
    tier: PolicyTier,
    verdict: Verdict,
    explanation: impl Into<String>,
) -> RuleOutcome {
    RuleOutcome {
        rule: rule.into(),
        tier,
        verdict,
        explanation: explanation.into(),
    }
}

impl PolicyEngine {
    pub fn new(cfg: CoreConfig) -> Self {
        let version = cfg.policy_version();
        Self { cfg, version }
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn config(&self) -> &CoreConfig {
        &self.cfg
    }

    /// Map a peer uid to a principal using the configuration.
    pub fn principal_for_uid(&self, uid: u32) -> Principal {
        if uid == 0 {
            return Principal::Root;
        }
        match self.cfg.principals.iter().find(|p| p.uid == uid) {
            Some(p) if p.role == Role::IntelligenceEngine => Principal::IntelligenceEngine,
            Some(_) => Principal::App(uid),
            None => Principal::Unknown(uid),
        }
    }

    fn grants(&self, p: &Principal) -> Vec<OperationKind> {
        match p {
            Principal::Core => OperationKind::ALL.to_vec(),
            Principal::Root => self.cfg.root_grants.clone(),
            Principal::App(uid) => self
                .cfg
                .principals
                .iter()
                .find(|m| m.uid == *uid && m.role == Role::App)
                .map(|m| m.grants.clone())
                .unwrap_or_default(),
            Principal::IntelligenceEngine => self
                .cfg
                .principals
                .iter()
                .filter(|m| m.role == Role::IntelligenceEngine)
                .flat_map(|m| m.grants.clone())
                .collect(),
            Principal::Unknown(_) => vec![],
        }
    }

    /// Arbitration priority of a request.
    pub fn tier_of(req: &ActionRequest) -> PolicyTier {
        match req.origin {
            Origin::Core => PolicyTier::Safety,
            Origin::User => PolicyTier::User,
            Origin::Recommendation { .. } => PolicyTier::Adaptive,
        }
    }

    pub fn evaluate(&self, inp: &EvalInput<'_>) -> PolicyDecision {
        let req = inp.request;
        let op = req.operation;
        let dir = op.direction(inp.current);
        let ctx = inp.context;
        let tel = &ctx.telemetry;
        let s = &self.cfg.safety;
        let mut rules = Vec::new();
        use PolicyTier::*;
        use Verdict::*;

        match req.validate() {
            Ok(()) => rules.push(outcome(
                "builtin.request_valid",
                Integrity,
                Allow,
                "request is well-formed",
            )),
            Err(e) => rules.push(outcome("builtin.request_valid", Integrity, Deny, e)),
        }

        let origin_ok = match (&req.principal, req.origin) {
            (Principal::IntelligenceEngine, Origin::Recommendation { .. }) => true,
            (Principal::IntelligenceEngine, _) => false,
            (_, Origin::Recommendation { .. }) => false,
            (Principal::Core, Origin::Core) => true,
            (_, Origin::Core) | (Principal::Core, _) => false,
            _ => true,
        };
        if !origin_ok {
            rules.push(outcome(
                "builtin.authorized",
                Integrity,
                Deny,
                format!(
                    "{} may not submit {:?}-origin requests",
                    req.principal.label(),
                    req.origin
                ),
            ));
        } else if self.grants(&req.principal).contains(&op.kind()) {
            rules.push(outcome(
                "builtin.authorized",
                Integrity,
                Allow,
                format!(
                    "{} holds a grant for {:?}",
                    req.principal.label(),
                    op.kind()
                ),
            ));
        } else {
            rules.push(outcome(
                "builtin.authorized",
                Integrity,
                Deny,
                format!("{} has no grant for {:?}", req.principal.label(), op.kind()),
            ));
        }
        if req.confirmed && !matches!(req.principal, Principal::Root | Principal::App(_)) {
            rules.push(outcome(
                "builtin.authorized",
                Integrity,
                Deny,
                "only a user principal can confirm",
            ));
        }

        let cap = op.resource().capability();
        match inp.registry.status(cap) {
            CapabilityStatus::ReadWrite => rules.push(outcome(
                "builtin.capability",
                Safety,
                Allow,
                format!("{cap} is read-write"),
            )),
            st => rules.push(outcome(
                "builtin.capability",
                Safety,
                Deny,
                format!("{cap} is {st:?} on this device"),
            )),
        }

        let raising = dir == Direction::Raise && req.origin != Origin::Core;
        let age = inp.now_ms.saturating_sub(ctx.collected_at_ms);
        let fresh = ctx.collected_at_ms != 0
            && inp.now_ms >= ctx.collected_at_ms
            && age <= self.cfg.telemetry.max_context_age_ms;
        if raising && !fresh {
            rules.push(outcome(
                "builtin.context_fresh",
                Safety,
                Deny,
                format!(
                    "device context is stale ({age} ms old); raising actions need fresh telemetry"
                ),
            ));
        }

        if raising {
            match tel.thermal_status.fresh_value(inp.now_ms) {
                Some(t) if *t >= ThermalStatus::Severe => rules.push(outcome(
                    "builtin.thermal",
                    Safety,
                    Deny,
                    format!("thermal status {t:?}: no performance-raising actions"),
                )),
                Some(t) => rules.push(outcome(
                    "builtin.thermal",
                    Safety,
                    Allow,
                    format!("thermal status {t:?}"),
                )),
                None if s.require_thermal_status => rules.push(outcome(
                    "builtin.thermal",
                    Safety,
                    Deny,
                    "thermal status unknown or stale; raising actions denied",
                )),
                None => rules.push(outcome(
                    "builtin.thermal",
                    Safety,
                    Allow,
                    "thermal status unknown; allowed by safety.require_thermal_status = false",
                )),
            }
            let level = tel.battery_level.fresh_value(inp.now_ms).map(|p| p.get());
            let charging = tel.charging.fresh_value(inp.now_ms).copied();
            if let (Some(l), Some(false)) = (level, charging) {
                if l <= s.battery_critical_percent {
                    rules.push(outcome(
                        "builtin.battery_critical",
                        Safety,
                        Deny,
                        format!("battery {l}% and not charging"),
                    ));
                }
            }
        }

        // Android ignores battery saver changes while plugged in (PowerManagerService returns
        // false when powered); refuse up front instead of reporting an unverifiable change.
        if op.kind() == OperationKind::SetLowPowerMode && req.origin != Origin::Core {
            if let Some(true) = tel.charging.fresh_value(inp.now_ms) {
                rules.push(outcome(
                    "builtin.battery_saver_while_charging",
                    Safety,
                    Deny,
                    "Android does not change battery saver while the device is charging",
                ));
            }
        }

        if let Origin::Recommendation { confidence } = req.origin {
            if confidence < s.recommendation_min_confidence {
                rules.push(outcome(
                    "builtin.recommendation",
                    Adaptive,
                    Deny,
                    format!(
                        "confidence {confidence:.2} below {:.2}",
                        s.recommendation_min_confidence
                    ),
                ));
            } else {
                rules.push(outcome(
                    "builtin.recommendation",
                    Adaptive,
                    Allow,
                    format!("confidence {confidence:.2}"),
                ));
            }
        }

        for r in &self.cfg.rules {
            let w = &r.when;
            let level = tel.battery_level.fresh_value(inp.now_ms).map(|p| p.get());
            let charging = tel.charging.fresh_value(inp.now_ms).copied();
            let thermal = tel.thermal_status.fresh_value(inp.now_ms).copied();
            let matches = (w.operations.is_empty() || w.operations.contains(&op.kind()))
                && w.direction.is_none_or(|d| d == dir)
                && w.origin.is_none_or(|o| match o {
                    OriginMatch::User => req.origin == Origin::User,
                    OriginMatch::Recommendation => matches!(req.origin, Origin::Recommendation { .. }),
                })
                // Conditions on unknown telemetry match (restrictions apply when in doubt).
                && w.battery_below.is_none_or(|b| level.is_none_or(|l| l < b))
                && w.charging.is_none_or(|c| charging.is_none_or(|v| v == c))
                && w.thermal_at_least.is_none_or(|t| thermal.is_none_or(|v| v >= t));
            if !matches || req.origin == Origin::Core {
                continue;
            }
            let expl = if r.explanation.is_empty() {
                "configured restriction".to_string()
            } else {
                r.explanation.clone()
            };
            match r.effect {
                RuleEffect::Deny => rules.push(outcome(&r.id, ThermalPower, Deny, expl)),
                RuleEffect::RequireConfirmation if req.confirmed => rules.push(outcome(
                    &r.id,
                    ThermalPower,
                    Allow,
                    format!("confirmed: {expl}"),
                )),
                RuleEffect::RequireConfirmation => {
                    rules.push(outcome(&r.id, ThermalPower, NeedsConfirmation, expl))
                }
            }
        }

        let verdict = if rules.iter().any(|r| r.verdict == Deny) {
            Deny
        } else if rules.iter().any(|r| r.verdict == NeedsConfirmation) {
            NeedsConfirmation
        } else {
            Allow
        };
        PolicyDecision {
            verdict,
            tier: Self::tier_of(req),
            rules,
            policy_version: self.version.clone(),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::{PrincipalMap, Rule, When};
    use zcore_domain::capability::CapabilityDescriptor;
    use zcore_domain::observation::Observation;
    use zcore_domain::request::Operation;
    use zcore_domain::resource::FasProfile;
    use zcore_domain::units::Percent;

    pub fn ctx(
        now: u64,
        thermal: Option<ThermalStatus>,
        battery: u8,
        charging: bool,
    ) -> DeviceContext {
        let mut c = DeviceContext::unknown();
        c.collected_at_ms = now;
        if let Some(t) = thermal {
            c.telemetry.thermal_status = Observation::measured(t, "test", now, 10_000);
        }
        c.telemetry.battery_level =
            Observation::measured(Percent::new(battery).unwrap(), "test", now, 10_000);
        c.telemetry.charging = Observation::measured(charging, "test", now, 10_000);
        c
    }

    pub fn registry_rw() -> CapabilityRegistry {
        let mut r = CapabilityRegistry::default();
        r.update_owner(
            "test",
            ["fas.profile", "fas.enabled", "android.low_power_mode"]
                .iter()
                .map(|id| {
                    CapabilityDescriptor::new(id, CapabilityStatus::ReadWrite, "test", vec![], 0)
                })
                .collect(),
        );
        r
    }

    pub fn req(principal: Principal, origin: Origin, op: Operation) -> ActionRequest {
        ActionRequest {
            id: "t1".into(),
            principal,
            origin,
            operation: op,
            duration_ms: None,
            dry_run: false,
            confirmed: false,
            reason: String::new(),
        }
    }

    const PERF: Operation = Operation::FasSetProfile {
        profile: FasProfile::Performance,
    };
    const SAVER: Operation = Operation::FasSetProfile {
        profile: FasProfile::PowerSaver,
    };

    fn eval(
        cfg: CoreConfig,
        r: &ActionRequest,
        c: &DeviceContext,
        reg: &CapabilityRegistry,
        now: u64,
    ) -> PolicyDecision {
        PolicyEngine::new(cfg).evaluate(&EvalInput {
            request: r,
            context: c,
            registry: reg,
            current: Some(ResourceValue::FasProfile(FasProfile::Balanced)),
            now_ms: now,
        })
    }

    #[test]
    fn root_allowed_in_normal_conditions_with_explanation() {
        let d = eval(
            CoreConfig::default(),
            &req(Principal::Root, Origin::User, PERF),
            &ctx(1000, Some(ThermalStatus::None), 80, false),
            &registry_rw(),
            1000,
        );
        assert_eq!(d.verdict, Verdict::Allow, "{:?}", d.rules);
        assert!(d.rules.iter().any(|r| r.rule == "builtin.thermal"));
        assert_eq!(d.tier, PolicyTier::User);
    }

    #[test]
    fn unknown_and_ungranted_principals_are_denied() {
        let c = ctx(1000, Some(ThermalStatus::None), 80, false);
        let reg = registry_rw();
        assert_eq!(
            eval(
                CoreConfig::default(),
                &req(Principal::Unknown(10500), Origin::User, PERF),
                &c,
                &reg,
                1000
            )
            .verdict,
            Verdict::Deny
        );
        let mut cfg = CoreConfig::default();
        cfg.principals.push(PrincipalMap {
            uid: 10234,
            role: Role::App,
            grants: vec![OperationKind::SetLowPowerMode],
        });
        assert_eq!(
            eval(
                cfg.clone(),
                &req(Principal::App(10234), Origin::User, PERF),
                &c,
                &reg,
                1000
            )
            .verdict,
            Verdict::Deny
        );
        assert_eq!(
            eval(
                cfg,
                &req(
                    Principal::App(10234),
                    Origin::User,
                    Operation::SetLowPowerMode { enabled: true }
                ),
                &c,
                &reg,
                1000
            )
            .verdict,
            Verdict::Allow
        );
    }

    #[test]
    fn thermal_battery_staleness_block_raising_but_not_lowering() {
        let reg = registry_rw();
        let root = |op| req(Principal::Root, Origin::User, op);
        let hot = ctx(1000, Some(ThermalStatus::Severe), 80, false);
        assert_eq!(
            eval(CoreConfig::default(), &root(PERF), &hot, &reg, 1000).verdict,
            Verdict::Deny
        );
        assert_eq!(
            eval(CoreConfig::default(), &root(SAVER), &hot, &reg, 1000).verdict,
            Verdict::Allow
        );
        let unknown = ctx(1000, None, 80, false);
        assert_eq!(
            eval(CoreConfig::default(), &root(PERF), &unknown, &reg, 1000).verdict,
            Verdict::Deny
        );
        let mut relaxed = CoreConfig::default();
        relaxed.safety.require_thermal_status = false;
        assert_eq!(
            eval(relaxed, &root(PERF), &unknown, &reg, 1000).verdict,
            Verdict::Allow
        );
        let low = ctx(1000, Some(ThermalStatus::None), 5, false);
        assert_eq!(
            eval(CoreConfig::default(), &root(PERF), &low, &reg, 1000).verdict,
            Verdict::Deny
        );
        let low_charging = ctx(1000, Some(ThermalStatus::None), 5, true);
        assert_eq!(
            eval(
                CoreConfig::default(),
                &root(PERF),
                &low_charging,
                &reg,
                1000
            )
            .verdict,
            Verdict::Allow
        );
        // 60 s later the context is stale.
        let d = eval(
            CoreConfig::default(),
            &root(PERF),
            &ctx(1000, Some(ThermalStatus::None), 80, false),
            &reg,
            61_000,
        );
        assert_eq!(d.verdict, Verdict::Deny);
        assert!(d.denial_reasons().iter().any(|r| r.contains("stale")));
    }

    #[test]
    fn capability_must_be_writable() {
        let mut reg = CapabilityRegistry::default();
        reg.update_owner(
            "t",
            vec![CapabilityDescriptor::new(
                "fas.profile",
                CapabilityStatus::TemporarilyUnavailable,
                "t",
                vec![],
                0,
            )],
        );
        let d = eval(
            CoreConfig::default(),
            &req(Principal::Root, Origin::User, SAVER),
            &ctx(1000, Some(ThermalStatus::None), 80, false),
            &reg,
            1000,
        );
        assert_eq!(d.verdict, Verdict::Deny);
        assert!(d.denial_reasons()[0].contains("TemporarilyUnavailable"));
    }

    #[test]
    fn recommendations_cannot_self_authorize() {
        let c = ctx(1000, Some(ThermalStatus::None), 80, false);
        let reg = registry_rw();
        let mut cfg = CoreConfig::default();
        cfg.principals.push(PrincipalMap {
            uid: 10300,
            role: Role::IntelligenceEngine,
            grants: vec![OperationKind::FasSetProfile],
        });
        let rec = |conf| {
            req(
                Principal::IntelligenceEngine,
                Origin::Recommendation { confidence: conf },
                PERF,
            )
        };
        let d = eval(cfg.clone(), &rec(0.9), &c, &reg, 1000);
        assert_eq!(d.verdict, Verdict::Allow);
        assert_eq!(d.tier, PolicyTier::Adaptive);
        assert_eq!(
            eval(cfg.clone(), &rec(0.5), &c, &reg, 1000).verdict,
            Verdict::Deny
        );
        // The engine may not pose as a user, nor confirm, and others may not pose as the engine.
        assert_eq!(
            eval(
                cfg.clone(),
                &req(Principal::IntelligenceEngine, Origin::User, PERF),
                &c,
                &reg,
                1000
            )
            .verdict,
            Verdict::Deny
        );
        let mut confirmed = rec(0.9);
        confirmed.confirmed = true;
        assert_eq!(
            eval(cfg.clone(), &confirmed, &c, &reg, 1000).verdict,
            Verdict::Deny
        );
        assert_eq!(
            eval(
                cfg.clone(),
                &req(
                    Principal::Root,
                    Origin::Recommendation { confidence: 1.0 },
                    PERF
                ),
                &c,
                &reg,
                1000
            )
            .verdict,
            Verdict::Deny
        );
        assert_eq!(
            eval(
                cfg,
                &req(Principal::Root, Origin::Core, PERF),
                &c,
                &reg,
                1000
            )
            .verdict,
            Verdict::Deny
        );
    }

    #[test]
    fn configured_rules_only_restrict() {
        let c = ctx(1000, Some(ThermalStatus::None), 25, false);
        let reg = registry_rw();
        let mut cfg = CoreConfig::default();
        cfg.rules.push(Rule {
            id: "confirm-raise-on-battery".into(),
            effect: RuleEffect::RequireConfirmation,
            when: When {
                direction: Some(Direction::Raise),
                battery_below: Some(30),
                charging: Some(false),
                ..Default::default()
            },
            explanation: "raising on battery below 30%".into(),
        });
        let mut r = req(Principal::Root, Origin::User, PERF);
        assert_eq!(
            eval(cfg.clone(), &r, &c, &reg, 1000).verdict,
            Verdict::NeedsConfirmation
        );
        r.confirmed = true;
        assert_eq!(
            eval(cfg.clone(), &r, &c, &reg, 1000).verdict,
            Verdict::Allow
        );
        // A rule cannot turn a builtin denial into an allow.
        let hot = ctx(1000, Some(ThermalStatus::Critical), 25, false);
        assert_eq!(eval(cfg, &r, &hot, &reg, 1000).verdict, Verdict::Deny);
    }
}
