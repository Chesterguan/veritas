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

use veritas_contracts::{
    error::{VeritasError, VeritasResult},
    execution::StepRecord,
};

use crate::event::AuditEvent;

/// Compute the SHA-256 hash for a single audit event.
///
/// The hash commits to every field that uniquely identifies an event:
/// its position in the chain (`sequence`), the execution it belongs to
/// (`execution_id`), its link to the previous event (`prev_hash`), and
/// the full step record (`record`).
///
/// Returns a lowercase 64-character hex string, or `Err(AuditWriteFailed)`
/// if `record` cannot be serialized to JSON (which cannot happen for
/// well-formed `StepRecord` values, but is handled explicitly rather than
/// via a panic).
pub fn hash_event(
    execution_id: &str,
    sequence: u64,
    record: &StepRecord,
    prev_hash: &str,
) -> VeritasResult<String> {
    // serde_json::to_vec serializes fields in the order they are declared in
    // the Rust struct (determined by the derive macro), without pretty-printing
    // or trailing whitespace.  This is deterministic within a single build, but
    // is implementation-dependent: future versions of serde_json or changes to
    // the StepRecord struct layout could alter field ordering and therefore
    // produce different hashes for the same logical record.
    //
    // KNOWN LIMITATION: audit chains created by different versions of VERITAS
    // may not be cross-version verifiable if StepRecord's field order changes.
    // A future hardening pass should adopt an explicit canonical serialization
    // format (e.g. sorted-key JSON or CBOR with a fixed schema) to eliminate
    // this dependency on implementation-defined ordering.
    let record_json = serde_json::to_vec(record).map_err(|e| VeritasError::AuditWriteFailed {
        reason: format!("failed to serialize StepRecord: {e}"),
    })?;

    let mut hasher = Sha256::new();
    hasher.update(execution_id.as_bytes());
    hasher.update(sequence.to_le_bytes());
    hasher.update(prev_hash.as_bytes());
    hasher.update(&record_json);

    Ok(hex::encode(hasher.finalize()))
}

/// Verify the integrity of a hash chain.
///
/// Returns `Ok(true)` when the chain is valid according to both rules:
///
/// 1. **Prev-hash linkage** — each event's `prev_hash` equals the
///    `this_hash` of the preceding event (or `GENESIS_HASH` for event 0).
/// 2. **Hash correctness** — each event's `this_hash` matches the value
///    recomputed from its own fields.
///
/// Returns `Ok(false)` the moment any mismatch is detected.  An empty
/// chain is defined as valid.  Returns `Err(AuditWriteFailed)` only if
/// an event's `StepRecord` cannot be serialized, which cannot happen for
/// well-formed records.
pub fn verify_chain(events: &[AuditEvent]) -> VeritasResult<bool> {
    let mut expected_prev = AuditEvent::GENESIS_HASH.to_string();

    for event in events {
        // Rule 1: the stored prev_hash must match what we expect.
        if event.prev_hash != expected_prev {
            return Ok(false);
        }

        // Rule 2: recompute this_hash and compare to the stored value.
        let recomputed = hash_event(
            &event.execution_id,
            event.sequence,
            &event.record,
            &event.prev_hash,
        )?;
        if event.this_hash != recomputed {
            return Ok(false);
        }

        // Advance the expected prev_hash to this event's hash.
        expected_prev = event.this_hash.clone();
    }

    Ok(true)
}
