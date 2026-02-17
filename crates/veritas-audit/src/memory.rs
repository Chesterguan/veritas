//! In-memory implementation of `AuditWriter`.
//!
//! `InMemoryAuditWriter` is the reference implementation of the
//! `AuditWriter` trait.  It keeps all events in a `Vec` protected by a
//! `Mutex`, making it safe to pass across threads while the executor calls
//! `write()` and `finalize()`.
//!
//! Use `export_log()` after execution completes to obtain a sealed
//! `AuditLog`, and `verify_integrity()` at any time to confirm the chain
//! has not been tampered with in memory.

use std::sync::{Arc, Mutex};

use chrono::Utc;
use tracing::info;

use veritas_contracts::{
    error::{VeritasError, VeritasResult},
    execution::StepRecord,
};
use veritas_core::traits::AuditWriter;

use crate::{
    chain::{hash_event, verify_chain},
    event::{AuditEvent, AuditLog},
};

// ── Internal mutable state ────────────────────────────────────────────────────

/// The mutable interior of an `InMemoryAuditWriter`.
///
/// Kept behind `Arc<Mutex<_>>` so that both `InMemoryAuditWriter` and any
/// clones of the `Arc` can safely observe or export the accumulated events.
pub(crate) struct InMemoryState {
    /// All events written so far, in append order.
    pub(crate) events: Vec<AuditEvent>,

    /// The next sequence number to assign (starts at 0).
    pub(crate) sequence: u64,

    /// The `this_hash` of the last written event, or `GENESIS_HASH` before
    /// any event has been written.
    pub(crate) last_hash: String,
}

// ── Public writer ─────────────────────────────────────────────────────────────

/// An in-memory, append-only audit writer backed by a SHA-256 hash chain.
///
/// # Thread safety
///
/// `write()` and `finalize()` both acquire a `Mutex` internally.  Multiple
/// threads may hold clones of the `Arc<Mutex<InMemoryState>>` without
/// additional synchronization.
pub struct InMemoryAuditWriter {
    execution_id: String,
    pub(crate) state: Arc<Mutex<InMemoryState>>,
}

impl InMemoryAuditWriter {
    /// Create a new writer for the given execution.
    ///
    /// The internal `last_hash` is initialized to `AuditEvent::GENESIS_HASH`
    /// so the first event's `prev_hash` is automatically correct.
    pub fn new(execution_id: impl Into<String>) -> Self {
        let execution_id = execution_id.into();
        let state = InMemoryState {
            events: Vec::new(),
            sequence: 0,
            last_hash: AuditEvent::GENESIS_HASH.to_string(),
        };
        Self {
            execution_id,
            state: Arc::new(Mutex::new(state)),
        }
    }

    /// Export a sealed `AuditLog` containing all events written so far.
    ///
    /// The `terminal_hash` is the `this_hash` of the last event, or an empty
    /// string when no events have been written.
    pub fn export_log(&self) -> AuditLog {
        let state = self.state.lock().expect("audit state lock poisoned");
        let terminal_hash = state
            .events
            .last()
            .map(|e| e.this_hash.clone())
            .unwrap_or_default();

        AuditLog {
            execution_id: self.execution_id.clone(),
            events: state.events.clone(),
            finalized_at: Utc::now(),
            terminal_hash,
        }
    }

    /// Verify that the in-memory chain has not been tampered with.
    ///
    /// Delegates to `verify_chain`, which checks both prev-hash linkage and
    /// hash correctness for every event.
    pub fn verify_integrity(&self) -> bool {
        let state = self.state.lock().expect("audit state lock poisoned");
        verify_chain(&state.events)
    }
}

// ── AuditWriter impl ──────────────────────────────────────────────────────────

impl AuditWriter for InMemoryAuditWriter {
    /// Append one step record to the hash chain.
    ///
    /// Computes `this_hash` from (execution_id, sequence, prev_hash, record),
    /// wraps the record in an `AuditEvent`, appends it, then advances the
    /// sequence counter and `last_hash`.
    ///
    /// Returns `Err(AuditWriteFailed)` only if the internal mutex is poisoned,
    /// which cannot happen under normal operation.
    fn write(&self, record: &StepRecord) -> VeritasResult<()> {
        let mut state = self.state.lock().map_err(|e| VeritasError::AuditWriteFailed {
            reason: format!("audit state lock poisoned: {}", e),
        })?;

        let prev_hash = state.last_hash.clone();
        let sequence = state.sequence;

        let this_hash = hash_event(&self.execution_id, sequence, record, &prev_hash);

        let event = AuditEvent {
            sequence,
            execution_id: self.execution_id.clone(),
            record: record.clone(),
            prev_hash,
            this_hash: this_hash.clone(),
        };

        state.events.push(event);
        state.sequence += 1;
        state.last_hash = this_hash;

        Ok(())
    }

    /// Mark the execution as complete in the audit log.
    ///
    /// Logs a structured message via `tracing`.  Implementations that persist
    /// to disk or a database would flush/seal here; the in-memory writer has
    /// nothing to flush.
    fn finalize(&self, execution_id: &str) -> VeritasResult<()> {
        let state = self.state.lock().map_err(|e| VeritasError::AuditWriteFailed {
            reason: format!("audit state lock poisoned: {}", e),
        })?;

        info!(
            execution_id = %execution_id,
            event_count = state.events.len(),
            terminal_hash = %state.last_hash,
            "audit log finalized"
        );

        Ok(())
    }
}
