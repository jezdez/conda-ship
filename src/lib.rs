//! Reusable conda-ship library APIs.
//!
//! The primary conda-ship surfaces remain the `cs` builder and stamped runtime
//! artifacts. This library contains small primitives for downstream installers
//! and orchestration tools.

#![cfg_attr(feature = "fleet", allow(dead_code))]

pub mod launcher_receipt;

#[cfg(feature = "fleet")]
mod bootstrap_lock;
#[cfg(feature = "fleet")]
mod bootstrap_state;
#[cfg(feature = "fleet")]
mod commands;
#[cfg(feature = "fleet")]
mod config;
#[cfg(feature = "fleet")]
mod constructor_metadata;
#[cfg(feature = "fleet")]
mod exec;
#[cfg(feature = "fleet")]
mod hash;
#[cfg(feature = "fleet")]
mod http;
#[cfg(feature = "fleet")]
mod install;
#[cfg(feature = "fleet")]
mod policy;
#[cfg(feature = "fleet")]
mod runtime_data;
#[cfg(feature = "fleet")]
mod tls;

#[cfg(feature = "fleet")]
pub mod fleet;
