//! # veritas-ref-healthcare
//!
//! Healthcare reference runtime for the VERITAS policy-bound AI execution system.
//!
//! Demonstrates three clinical AI scenarios using mock data:
//!
//! 1. **Drug Interaction Checker** — capability-gated database query with
//!    structured output verification.
//! 2. **Clinical Note Summarizer** — LLM output verification with a custom
//!    PII label detection rule.
//! 3. **Patient Data Query** — layered enforcement showing Allow, CapabilityMissing,
//!    and Policy Deny outcomes across three sub-cases.
//!
//! All data is hardcoded and fictional. No external API calls are made.

pub mod mock_data;
pub mod scenarios;
