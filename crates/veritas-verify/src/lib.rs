//! # veritas-verify
//!
//! Output verification for the VERITAS runtime.
//!
//! This crate provides [`engine::SchemaVerifier`], which implements the
//! [`veritas_core::traits::Verifier`] trait.  It validates `AgentOutput`
//! payloads in two phases:
//!
//! 1. **Structural** — JSON Schema validation via the `jsonschema` crate.
//! 2. **Semantic** — domain rules (`RequiredField`, `AllowedValues`,
//!    `ForbiddenPattern`, `Custom`) evaluated against the payload.
//!
//! ## Quick start
//!
//! ```rust,ignore
//! use veritas_verify::engine::SchemaVerifier;
//!
//! let mut verifier = SchemaVerifier::new();
//! verifier.register_rule("phi-check", Box::new(|payload| {
//!     if payload.get("contains_phi").and_then(|v| v.as_bool()).unwrap_or(false) {
//!         Some("output must not contain PHI in this phase".to_string())
//!     } else {
//!         None
//!     }
//! }));
//! ```

pub mod engine;
