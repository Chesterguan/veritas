//! Healthcare reference runtime demo scenarios.
//!
//! Each scenario is a self-contained module that wires up real VERITAS
//! components (policy engine, audit writer, verifier, executor) with mock
//! clinical data and demonstrates a distinct enforcement pattern.

pub mod clinical_pipeline;
pub mod drug_interaction;
pub mod note_summarizer;
pub mod patient_query;
pub mod prior_auth;
