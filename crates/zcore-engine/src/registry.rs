// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Capability registry: the single answer to "can this be observed / controlled here?".

use std::collections::BTreeMap;

use zcore_domain::capability::{CapabilityDescriptor, CapabilityStatus};

#[derive(Debug, Clone, Default)]
pub struct CapabilityRegistry {
    caps: BTreeMap<String, CapabilityDescriptor>,
}

impl CapabilityRegistry {
    /// Replace descriptors reported by one owner. Returns ids whose status changed.
    pub fn update_owner(
        &mut self,
        owner: &str,
        descriptors: Vec<CapabilityDescriptor>,
    ) -> Vec<String> {
        let mut changed = Vec::new();
        let ids: Vec<String> = descriptors.iter().map(|d| d.id.clone()).collect();
        // Capabilities the owner no longer reports become Unknown, not silently removed.
        for (id, d) in self.caps.iter_mut() {
            if d.owner == owner && !ids.contains(id) && d.status != CapabilityStatus::Unknown {
                d.status = CapabilityStatus::Unknown;
                d.evidence = vec!["no longer reported by owner".into()];
                changed.push(id.clone());
            }
        }
        for mut d in descriptors {
            match self.caps.get(&d.id) {
                Some(old) if old.status == d.status => {
                    // Keep the on-device verification mark while the status is unchanged.
                    d.verified_on_device |= old.verified_on_device;
                }
                _ => changed.push(d.id.clone()),
            }
            self.caps.insert(d.id.clone(), d);
        }
        changed
    }

    pub fn get(&self, id: &str) -> Option<&CapabilityDescriptor> {
        self.caps.get(id)
    }

    pub fn status(&self, id: &str) -> CapabilityStatus {
        self.caps
            .get(id)
            .map(|d| d.status)
            .unwrap_or(CapabilityStatus::Unknown)
    }

    pub fn mark_verified(&mut self, id: &str) {
        if let Some(d) = self.caps.get_mut(id) {
            d.verified_on_device = true;
        }
    }

    pub fn all(&self) -> impl Iterator<Item = &CapabilityDescriptor> {
        self.caps.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(id: &str, s: CapabilityStatus) -> CapabilityDescriptor {
        CapabilityDescriptor::new(id, s, "fas", vec![], 0)
    }

    #[test]
    fn changes_and_disappearance() {
        let mut r = CapabilityRegistry::default();
        assert_eq!(
            r.update_owner("fas", vec![d("fas.profile", CapabilityStatus::ReadWrite)]),
            vec!["fas.profile"]
        );
        r.mark_verified("fas.profile");
        assert!(r
            .update_owner("fas", vec![d("fas.profile", CapabilityStatus::ReadWrite)])
            .is_empty());
        assert!(r.get("fas.profile").unwrap().verified_on_device);
        assert_eq!(r.update_owner("fas", vec![]), vec!["fas.profile"]);
        assert_eq!(r.status("fas.profile"), CapabilityStatus::Unknown);
        assert_eq!(r.status("nothing"), CapabilityStatus::Unknown);
    }
}
