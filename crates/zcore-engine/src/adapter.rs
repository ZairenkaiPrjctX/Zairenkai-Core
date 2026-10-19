//! Execution adapter contract.
//!
//! An adapter owns one or more resources and is the only code allowed to change them. It must:
//! - report capabilities with evidence (`discover`);
//! - validate inputs (a value that does not fit the resource is an error, not a no-op);
//! - bound every operation in time (`timeout_ms`), enforcing it internally;
//! - report failures as typed [`AdapterError`]s, never as silent success;
//! - read the resource back (`read`) so Core can verify.
//!
//! Adapters never receive free-form commands.

use serde::{Deserialize, Serialize};
use zcore_domain::capability::{CapabilityDescriptor, CapabilityStatus};
use zcore_domain::resource::{ResourceId, ResourceValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterErrorCode {
    /// The resource or value is not supported here.
    Unsupported,
    PermissionDenied,
    /// The owning service did not answer in time.
    Timeout,
    /// The owning service is not running / not reachable.
    Unavailable,
    /// The owning service answered but refused the change.
    Rejected,
    /// Unexpected answer (format drift).
    Protocol,
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterError {
    pub code: AdapterErrorCode,
    pub message: String,
}

impl AdapterError {
    pub fn new(code: AdapterErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

pub trait Adapter: Send {
    fn id(&self) -> &str;
    fn resources(&self) -> &[ResourceId];
    fn timeout_ms(&self) -> u64;
    fn discover(&mut self, now_ms: u64) -> Vec<CapabilityDescriptor>;
    fn read(&mut self, resource: ResourceId) -> Result<ResourceValue, AdapterError>;
    fn apply(&mut self, resource: ResourceId, value: ResourceValue) -> Result<(), AdapterError>;
}

/// Fault injection for the in-memory adapter.
#[derive(Debug, Clone, Default)]
pub struct Faults {
    pub apply_error: Option<AdapterErrorCode>,
    pub read_error: Option<AdapterErrorCode>,
    /// `apply` reports success but nothing changes (unverifiable write).
    pub ignore_writes: bool,
    /// `apply` reports success but stores this value instead (wrong effect).
    pub write_instead: Option<ResourceValue>,
    /// Limit `write_instead` to this many applies (`None` = every apply).
    pub write_instead_times: Option<u32>,
    /// Fail the N-th next apply (1 = the next one).
    pub fail_nth_apply: Option<u32>,
    pub status: Option<CapabilityStatus>,
}

#[derive(Debug, Default)]
pub struct FakeState {
    pub values: std::collections::BTreeMap<ResourceId, ResourceValue>,
    pub faults: Faults,
    pub applies: u32,
}

/// Shared view of a [`FakeAdapter`]'s state, so tests can change the "device" behind Core's
/// back (external modification) and inject faults mid-test.
pub type FakeHandle = std::sync::Arc<std::sync::Mutex<FakeState>>;

/// In-memory adapter for tests and `zcored simulate`. Never touches the host.
#[derive(Debug, Clone)]
pub struct FakeAdapter {
    id: String,
    resources: Vec<ResourceId>,
    state: FakeHandle,
}

impl FakeAdapter {
    pub fn new(id: &str, initial: &[(ResourceId, ResourceValue)]) -> Self {
        let state = FakeState {
            values: initial.iter().copied().collect(),
            ..Default::default()
        };
        Self {
            id: id.into(),
            resources: initial.iter().map(|(r, _)| *r).collect(),
            state: std::sync::Arc::new(std::sync::Mutex::new(state)),
        }
    }

    pub fn handle(&self) -> FakeHandle {
        self.state.clone()
    }

    fn st(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Adapter for FakeAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    fn resources(&self) -> &[ResourceId] {
        &self.resources
    }

    fn timeout_ms(&self) -> u64 {
        1_000
    }

    fn discover(&mut self, now_ms: u64) -> Vec<CapabilityDescriptor> {
        let st = self
            .st()
            .faults
            .status
            .unwrap_or(CapabilityStatus::ReadWrite);
        self.resources
            .iter()
            .map(|r| {
                CapabilityDescriptor::new(
                    r.capability(),
                    st,
                    &self.id,
                    vec!["simulated (fake adapter)".into()],
                    now_ms,
                )
            })
            .collect()
    }

    fn read(&mut self, resource: ResourceId) -> Result<ResourceValue, AdapterError> {
        let st = self.st();
        if let Some(c) = st.faults.read_error {
            return Err(AdapterError::new(c, "injected read fault"));
        }
        st.values
            .get(&resource)
            .copied()
            .ok_or_else(|| AdapterError::new(AdapterErrorCode::Unsupported, "not simulated"))
    }

    fn apply(&mut self, resource: ResourceId, value: ResourceValue) -> Result<(), AdapterError> {
        let owned = self.resources.contains(&resource);
        let mut st = self.st();
        st.applies += 1;
        if !value.fits(resource) || !owned {
            return Err(AdapterError::new(
                AdapterErrorCode::Unsupported,
                "value does not fit resource",
            ));
        }
        if let Some(n) = st.faults.fail_nth_apply.as_mut() {
            *n -= 1;
            if *n == 0 {
                st.faults.fail_nth_apply = None;
                return Err(AdapterError::new(
                    AdapterErrorCode::Io,
                    "injected nth-apply fault",
                ));
            }
        }
        if let Some(c) = st.faults.apply_error {
            return Err(AdapterError::new(c, "injected apply fault"));
        }
        if let Some(w) = st.faults.write_instead {
            if let Some(n) = st.faults.write_instead_times.as_mut() {
                *n -= 1;
                if *n == 0 {
                    st.faults.write_instead = None;
                    st.faults.write_instead_times = None;
                }
            }
            st.values.insert(resource, w);
        } else if !st.faults.ignore_writes {
            st.values.insert(resource, value);
        }
        Ok(())
    }
}
