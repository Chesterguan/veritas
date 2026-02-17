//! # veritas-core
//!
//! The deterministic, policy-bound execution runtime for VERITAS agents.
//!
//! This crate provides:
//! - The four core traits (`Agent`, `PolicyEngine`, `AuditWriter`, `Verifier`)
//! - The `Executor` that wires them together in the correct trust order
//!
//! ## Usage
//!
//! ```rust,ignore
//! use veritas_core::{Executor, traits::{Agent, PolicyEngine, AuditWriter, Verifier}};
//! ```

pub mod executor;
pub mod traits;

pub use executor::Executor;
