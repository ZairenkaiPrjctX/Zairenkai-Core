//! Bounded value types with explicit units.

use serde::{Deserialize, Serialize};

/// A percentage, 0..=100.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Percent(u8);

impl Percent {
    pub fn new(v: u8) -> Result<Self, String> {
        if v <= 100 {
            Ok(Self(v))
        } else {
            Err(format!("percentage {v} > 100"))
        }
    }
    pub fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for Percent {
    type Error = String;
    fn try_from(v: u8) -> Result<Self, String> {
        Self::new(v)
    }
}

impl From<Percent> for u8 {
    fn from(p: Percent) -> u8 {
        p.0
    }
}

/// Temperature in thousandths of a degree Celsius (Linux thermal sysfs unit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MilliCelsius(pub i64);

/// Memory size in KiB (`/proc/meminfo` unit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Kib(pub u64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_bounds() {
        assert!(Percent::new(100).is_ok());
        assert!(Percent::new(101).is_err());
        assert!(serde_json::from_str::<Percent>("150").is_err());
        assert_eq!(serde_json::from_str::<Percent>("55").unwrap().get(), 55);
    }
}
