//! Runtime error types for the VERITAS execution pipeline.
//!
//! All fallible operations in the VERITAS pipeline return `VeritasResult<T>`.
//! Error variants carry enough context to produce actionable audit entries.

use thiserror::Error;

/// The unified error type for the VERITAS runtime.
#[derive(Debug, Error)]
pub enum VeritasError {
    /// A policy rule explicitly denied the agent's requested action.
    #[error("policy denied action: {reason}")]
    PolicyDenied { reason: String },

    /// The agent requires a capability it was not granted.
    #[error("capability '{capability}' required for action '{action}' is not granted")]
    CapabilityMissing { capability: String, action: String },

    /// The verifier rejected the agent's output before it could be delivered.
    #[error("output verification failed: {reason}")]
    VerificationFailed { reason: String },

    /// The audit writer could not persist a step record.
    ///
    /// This is treated as fatal — a step that cannot be audited cannot proceed.
    #[error("audit write failed: {reason}")]
    AuditWriteFailed { reason: String },

    /// The agent's state machine encountered an illegal transition or corrupt state.
    #[error("state machine error: {reason}")]
    StateMachineError { reason: String },

    /// A required configuration value is missing or invalid.
    #[error("configuration error: {reason}")]
    ConfigError { reason: String },

    /// A JSON Schema validation check failed outside of the normal verification path.
    #[error("schema validation error: {reason}")]
    SchemaValidation { reason: String },
}

/// Convenience alias used throughout the VERITAS crates.
pub type VeritasResult<T> = Result<T, VeritasError>;
