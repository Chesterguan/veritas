//! Hash-chain primitives: hashing and chain integrity verification.
//!
//! The chain is built by XOR-free concatenation of deterministic byte
//! sequences fed into SHA-256.  Every field that contributes to an event's
//! hash is listed explicitly so nothing is accidentally omitted.
//!
//! Hash input layout (bytes, in order):
//!   1. execution_id as UTF-8 bytes
//!   2. sequence as 8-byte little-endian
//!   3. prev_hash as UTF-8 bytes (64 ASCII hex chars)
//!   4. canonical JSON of record (serde_json with no pretty-printing)

use sha2::{Digest, Sha256};

use veritas_contracts::execution::StepRecord;

use crate::event::AuditEvent;

/// Compute the SHA-256 hash for a single audit event.
///
/// The hash commits to every field that uniquely identifies an event:
/// its position in the chain (`sequence`), the execution it belongs to
/// (`execution_id`), its link to the previous event (`prev_hash`), and
/// the full step record (`record`).
///
/// Returns a lowercase 64-character hex string.
///
/// # Panics
///
/// Panics if `record` cannot be serialized to JSON — which cannot happen
/// for the well-formed `StepRecord` type.
pub fn hash_event(
    execution_id: &str,
    sequence: u64,
    record: &StepRecord,
    prev_hash: &str,
) -> String {
    // serde_json::to_vec produces canonical, deterministic JSON without
    // trailing whitespace or key reordering across calls on the same value.
    let record_json =
        serde_json::to_vec(record).expect("StepRecord must always be serializable to JSON");

    let mut hasher = Sha256::new();
    hasher.update(execution_id.as_bytes());
    hasher.update(sequence.to_le_bytes());
    hasher.update(prev_hash.as_bytes());
    hasher.update(&record_json);

    hex::encode(hasher.finalize())
}

/// Verify the integrity of a hash chain.
///
/// Returns `true` when the chain is valid according to both rules:
///
/// 1. **Prev-hash linkage** — each event's `prev_hash` equals the
///    `this_hash` of the preceding event (or `GENESIS_HASH` for event 0).
/// 2. **Hash correctness** — each event's `this_hash` matches the value
///    recomputed from its own fields.
///
/// Returns `false` the moment any mismatch is detected.  An empty chain
/// is defined as valid.
pub fn verify_chain(events: &[AuditEvent]) -> bool {
    let mut expected_prev = AuditEvent::GENESIS_HASH.to_string();

    for event in events {
        // Rule 1: the stored prev_hash must match what we expect.
        if event.prev_hash != expected_prev {
            return false;
        }

        // Rule 2: recompute this_hash and compare to the stored value.
        let recomputed = hash_event(
            &event.execution_id,
            event.sequence,
            &event.record,
            &event.prev_hash,
        );
        if event.this_hash != recomputed {
            return false;
        }

        // Advance the expected prev_hash to this event's hash.
        expected_prev = event.this_hash.clone();
    }

    true
}
