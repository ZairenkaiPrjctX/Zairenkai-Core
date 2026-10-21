// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Who is asking. Principals are derived by Core from transport credentials (peer uid), never
//! from what a request claims about itself.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Principal {
    /// Core itself (rollback, reconciliation).
    Core,
    /// uid 0 on the local socket (root shell, root manager WebUI, service scripts).
    Root,
    /// An Android app uid that the configuration maps to a client.
    App(u32),
    /// The configured Zairenkai-Intelligence-Engine uid. Its requests are recommendations.
    IntelligenceEngine,
    /// Any other local uid.
    Unknown(u32),
}

impl Principal {
    pub fn label(&self) -> String {
        match self {
            Principal::Core => "core".into(),
            Principal::Root => "root".into(),
            Principal::App(u) => format!("app:{u}"),
            Principal::IntelligenceEngine => "intelligence_engine".into(),
            Principal::Unknown(u) => format!("uid:{u}"),
        }
    }
}
