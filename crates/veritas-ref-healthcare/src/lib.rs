//! # veritas-ref-healthcare
//!
//! Healthcare reference runtime for the VERITAS policy-bound AI execution system.
//!
//! Demonstrates five clinical AI scenarios using mock data:
//!
//! 1. **Drug Interaction Checker** — capability-gated database query with
//!    structured output verification.
//! 2. **Clinical Note Summarizer** — LLM output verification with a custom
//!    PII label detection rule.
//! 3. **Patient Data Query** — layered enforcement showing Allow, CapabilityMissing,
//!    and Policy Deny outcomes across three sub-cases.
//! 4. **Multi-Agent Clinical Decision Pipeline** — 4-agent chain where each
//!    agent's verified output becomes the next agent's input, with per-agent
//!    audit trails and a custom HIGH-risk drug interaction verifier rule.
//! 5. **Prior Authorization Workflow** — `RequireApproval` verdict exercised to
//!    completion: physician approval simulated, then two sub-cases (PA approved
//!    vs. denied at insurance eligibility).
//!
//! All data is hardcoded and fictional. No external API calls are made.

pub mod mock_data;
pub mod scenarios;
