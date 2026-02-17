//! # veritas-audit
//!
//! Immutable, append-only, SHA-256 hash-chained audit trail for the VERITAS
//! runtime.
//!
//! ## Overview
//!
//! Every step the executor records is wrapped in an `AuditEvent` that links
//! to the previous event via its SHA-256 hash.  Tampering with any event —
//! even a single byte — breaks the chain and is detected by `verify_chain`.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use veritas_audit::{InMemoryAuditWriter, AuditEvent};
//! use veritas_core::traits::AuditWriter;
//!
//! let writer = InMemoryAuditWriter::new("exec-001");
//! writer.write(&step_record)?;
//! writer.finalize("exec-001")?;
//!
//! assert!(writer.verify_integrity());
//! let log = writer.export_log();
//! ```

pub mod chain;
pub mod event;
pub mod memory;

pub use chain::{hash_event, verify_chain};
pub use event::{AuditEvent, AuditLog};
pub use memory::InMemoryAuditWriter;

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use veritas_contracts::{
        agent::{AgentInput, AgentOutput},
        execution::StepRecord,
        policy::PolicyVerdict,
    };
    use veritas_core::traits::AuditWriter;

    use super::{AuditEvent, InMemoryAuditWriter};

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Build a minimal `StepRecord` with a distinguishable payload.
    fn make_record(step: u64, payload: &str) -> StepRecord {
        StepRecord {
            step,
            input: AgentInput {
                kind: "user_message".to_string(),
                payload: json!({ "text": payload }),
            },
            verdict: PolicyVerdict::Allow,
            output: Some(AgentOutput {
                kind: "response".to_string(),
                payload: json!({ "text": "ok" }),
            }),
            timestamp: Utc::now(),
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    /// Writing three events and verifying produces a valid chain.
    #[test]
    fn test_hash_chain_integrity() {
        let writer = InMemoryAuditWriter::new("exec-integrity");
        writer.write(&make_record(0, "first")).unwrap();
        writer.write(&make_record(1, "second")).unwrap();
        writer.write(&make_record(2, "third")).unwrap();

        assert!(writer.verify_integrity(), "chain must be valid after sequential writes");
    }

    /// Mutating any event's record field breaks the chain.
    #[test]
    fn test_tamper_detection() {
        let writer = InMemoryAuditWriter::new("exec-tamper");
        writer.write(&make_record(0, "step-a")).unwrap();
        writer.write(&make_record(1, "step-b")).unwrap();
        writer.write(&make_record(2, "step-c")).unwrap();

        // Directly mutate the internal state to simulate tampering.
        {
            let mut state = writer.state.lock().unwrap();
            // Change the payload in the first event's record.
            state.events[0].record.input.payload =
                json!({ "text": "TAMPERED" });
        }

        // The chain must now fail verification because event 0's this_hash
        // no longer matches the recomputed hash of its (mutated) record.
        assert!(
            !writer.verify_integrity(),
            "chain must detect tampering with a stored event"
        );
    }

    /// The first event's `prev_hash` must equal `AuditEvent::GENESIS_HASH`.
    #[test]
    fn test_genesis_hash() {
        let writer = InMemoryAuditWriter::new("exec-genesis");
        writer.write(&make_record(0, "first")).unwrap();

        let log = writer.export_log();
        assert_eq!(log.events.len(), 1);
        assert_eq!(
            log.events[0].prev_hash,
            AuditEvent::GENESIS_HASH,
            "first event must link to the genesis sentinel hash"
        );
    }

    /// Sequence numbers must be 0, 1, 2, … with no gaps or skips.
    #[test]
    fn test_sequence_monotonic() {
        let writer = InMemoryAuditWriter::new("exec-seq");
        writer.write(&make_record(0, "a")).unwrap();
        writer.write(&make_record(1, "b")).unwrap();
        writer.write(&make_record(2, "c")).unwrap();

        let log = writer.export_log();
        for (idx, event) in log.events.iter().enumerate() {
            assert_eq!(
                event.sequence, idx as u64,
                "sequence at position {} should be {}",
                idx, idx
            );
        }
    }

    /// `export_log()` contains every written event in order.
    #[test]
    fn test_export_log() {
        let writer = InMemoryAuditWriter::new("exec-export");
        writer.write(&make_record(0, "alpha")).unwrap();
        writer.write(&make_record(1, "beta")).unwrap();
        writer.write(&make_record(2, "gamma")).unwrap();

        let log = writer.export_log();

        assert_eq!(log.execution_id, "exec-export");
        assert_eq!(log.events.len(), 3, "log must contain all written events");

        // The terminal_hash must equal the last event's this_hash.
        assert_eq!(
            log.terminal_hash,
            log.events.last().unwrap().this_hash,
            "terminal_hash must equal the last event's this_hash"
        );

        // Verify chain integrity on the exported log using the public helper.
        assert!(
            super::verify_chain(&log.events),
            "exported log must pass chain verification"
        );
    }

    /// An empty chain is trivially valid — there is nothing to verify.
    #[test]
    fn test_verify_empty() {
        let writer = InMemoryAuditWriter::new("exec-empty");
        assert!(
            writer.verify_integrity(),
            "an empty chain must be considered valid"
        );

        // Also verify via the public function directly.
        assert!(
            super::verify_chain(&[]),
            "verify_chain on empty slice must return true"
        );
    }
}
