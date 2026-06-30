//! Library APIs for conda-ship.
//!
//! The primary conda-ship surfaces are still the `cs` builder and
//! `cs-template` runtime binaries. The `fleet` module is experimental and is
//! intended for downstream orchestrators that need to manage multiple locked
//! conda prefixes through shared conda-ship install mechanics.

#![allow(dead_code)]

mod config;
mod constructor_metadata;
mod exec;
mod hash;
mod http;
mod install;
mod policy;
mod runtime_data;
mod tls;

#[cfg(feature = "fleet")]
pub mod fleet;
