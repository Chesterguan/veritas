//! Audit event and log types.
//!
//! `AuditEvent` is a single entry in the hash chain — it wraps a `StepRecord`
//! with sequence numbering and the SHA-256 hashes that make tampering
//! detectable.  `AuditLog` is the sealed record produced when an execution
//! finalizes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use veritas_contracts::execution::StepRecord;

/// A single entry in the SHA-256 hash chain for one execution.
///
/// Each event commits to the previous event via `prev_hash`, forming an
/// append-only chain.  Modifying any field — including those of the embedded
/// `record` — invalidates `this_hash` and every subsequent `prev_hash`,
/// which `verify_chain` detects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Monotonically increasing position in the chain, starting at 0.
    pub sequence: u64,

    /// The execution this event belongs to.
    pub execution_id: String,

    /// The immutable step record produced by the executor.
    pub record: StepRecord,

    /// SHA-256 hash (hex) of the previous event, or `GENESIS_HASH` for the
    /// first event.
    pub prev_hash: String,

    /// SHA-256 hash (hex) of this event's canonical content.
    ///
    /// Computed by `hash_event()` over (execution_id, sequence, prev_hash,
    /// canonical JSON of record).
    pub this_hash: String,
}

impl AuditEvent {
    /// The sentinel `prev_hash` used for the first event in every chain.
    ///
    /// 64 hex zeros — a value that can never be the SHA-256 of real data,
    /// making genesis detection unambiguous.
    pub const GENESIS_HASH: &'static str =
        "0000000000000000000000000000000000000000000000000000000000000000";
}

/// A sealed, finalized audit log for a single execution.
///
/// Produced by `InMemoryAuditWriter::export_log()` after the execution
/// completes.  The `terminal_hash` is the `this_hash` of the last event and
/// can be used as a compact commitment to the entire log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    /// The execution whose steps are recorded here.
    pub execution_id: String,

    /// All audit events in chain order (sequence 0 first).
    pub events: Vec<AuditEvent>,

    /// Wall-clock time (UTC) the log was exported / finalized.
    pub finalized_at: DateTime<Utc>,

    /// The `this_hash` of the last event.  Empty string if the log is empty.
    pub terminal_hash: String,
}
