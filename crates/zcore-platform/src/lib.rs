// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Zairenkai-Core platform layer: device discovery, telemetry, module inventory and the
//! production execution adapters. All file access goes through [`sysroot::SysRoot`] and all
//! commands through the allowlisted [`exec::Runner`], so tests never touch the host.

pub mod afid;
pub mod discovery;
pub mod exec;
pub mod fas;
pub mod fsutil;
pub mod modules;
pub mod power;
pub mod sysroot;
pub mod telemetry;
pub mod testutil;
pub mod thermaltools;
