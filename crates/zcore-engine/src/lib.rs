// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 ZairenkaiPrjctX contributors
//! Zairenkai-Core decision engine. No I/O: adapters, clock and device context are injected,
//! so every decision is deterministic and testable without a device.

pub mod adapter;
pub mod arbitration;
pub mod audit;
pub mod config;
pub mod core;
pub mod policy;
pub mod registry;

pub use crate::core::Core;
