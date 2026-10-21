// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Observed values with explicit availability, provenance and freshness.
//!
//! A value that could not be read is never replaced by a guess: it is carried as
//! [`Availability::Unavailable`], [`Availability::Unsupported`], etc., and consumers decide what
//! that means for them (policy treats most of them as "deny the risky direction").

use serde::{Deserialize, Serialize};

/// How a value was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Read directly from a kernel, HAL or Android service interface.
    Measured,
    /// Computed from measured values (e.g. a classification).
    Derived,
    /// Taken from configuration or a database record, not from the device.
    Declared,
}

/// Whether a value exists, and if not, why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum Availability<T> {
    Value(T),
    /// The interface exists but the read failed (I/O error, timeout, parse error).
    Unavailable(String),
    /// This device or Android version does not provide the interface.
    Unsupported,
    /// The interface exists but this process may not read it.
    PermissionDenied,
    /// Not collected (yet).
    Unknown,
}

impl<T> Availability<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Availability::Value(v) => Some(v),
            _ => None,
        }
    }
}

/// One observation: value, where it came from, when, and for how long it may be trusted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation<T> {
    pub value: Availability<T>,
    pub provenance: Provenance,
    /// Interface the value came from, e.g. `/sys/class/power_supply/battery/capacity`.
    pub source: String,
    /// Monotonic time of collection, ms.
    pub observed_at_ms: u64,
    /// The value is stale after this many ms.
    pub ttl_ms: u64,
}

impl<T> Observation<T> {
    pub fn measured(value: T, source: impl Into<String>, at_ms: u64, ttl_ms: u64) -> Self {
        Self {
            value: Availability::Value(value),
            provenance: Provenance::Measured,
            source: source.into(),
            observed_at_ms: at_ms,
            ttl_ms,
        }
    }

    pub fn missing(
        value: Availability<T>,
        source: impl Into<String>,
        at_ms: u64,
        ttl_ms: u64,
    ) -> Self {
        Self {
            value,
            provenance: Provenance::Measured,
            source: source.into(),
            observed_at_ms: at_ms,
            ttl_ms,
        }
    }

    pub fn unknown() -> Self {
        Self {
            value: Availability::Unknown,
            provenance: Provenance::Measured,
            source: String::new(),
            observed_at_ms: 0,
            ttl_ms: 0,
        }
    }

    pub fn is_fresh(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.observed_at_ms) <= self.ttl_ms && now_ms >= self.observed_at_ms
    }

    /// The value, only if it exists and is fresh at `now_ms`.
    pub fn fresh_value(&self, now_ms: u64) -> Option<&T> {
        if self.is_fresh(now_ms) {
            self.value.value()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freshness() {
        let o = Observation::measured(42u8, "x", 1_000, 500);
        assert_eq!(o.fresh_value(1_400), Some(&42));
        assert_eq!(o.fresh_value(1_501), None);
        // A clock that went backwards is not "fresh".
        assert_eq!(o.fresh_value(999), None);
        let m: Observation<u8> =
            Observation::missing(Availability::PermissionDenied, "x", 1_000, 500);
        assert_eq!(m.fresh_value(1_000), None);
    }
}
