//! # veritas-contracts
//!
//! Shared types, schemas, and contracts for the VERITAS runtime.
//!
//! All crates in the workspace import from here. No business logic lives in
//! this crate — only data definitions and error types.

pub mod agent;
pub mod capability;
pub mod error;
pub mod execution;
pub mod policy;
pub mod verify;

#[cfg(test)]
mod tests {
    use super::*;
    use agent::ExecutionId;
    use capability::{Capability, CapabilitySet};
    use error::VeritasError;
    use policy::PolicyVerdict;

    // ── CapabilitySet ────────────────────────────────────────────────────────

    #[test]
    fn capability_set_grant_and_has() {
        let mut caps = CapabilitySet::default();
        let phi_read = Capability::new("phi:read");
        let phi_write = Capability::new("phi:write");

        // Nothing granted yet.
        assert!(!caps.has(&phi_read));
        assert!(!caps.has(&phi_write));

        caps.grant(phi_read.clone());
        assert!(caps.has(&phi_read));
        assert!(!caps.has(&phi_write));

        caps.grant(phi_write.clone());
        assert!(caps.has(&phi_read));
        assert!(caps.has(&phi_write));
    }

    #[test]
    fn capability_set_all_returns_all_granted() {
        let mut caps = CapabilitySet::default();
        caps.grant(Capability::new("a"));
        caps.grant(Capability::new("b"));
        caps.grant(Capability::new("c"));

        let names: std::collections::HashSet<String> =
            caps.all().map(|c| c.0.clone()).collect();

        assert_eq!(names.len(), 3);
        assert!(names.contains("a"));
        assert!(names.contains("b"));
        assert!(names.contains("c"));
    }

    #[test]
    fn capability_set_duplicate_grant_is_idempotent() {
        let mut caps = CapabilitySet::default();
        caps.grant(Capability::new("phi:read"));
        caps.grant(Capability::new("phi:read"));

        // HashSet semantics: duplicates are silently dropped.
        assert_eq!(caps.all().count(), 1);
    }

    // ── PolicyVerdict serde round-trip ───────────────────────────────────────

    #[test]
    fn policy_verdict_allow_round_trips() {
        let original = PolicyVerdict::Allow;
        let json = serde_json::to_string(&original).unwrap();
        let decoded: PolicyVerdict = serde_json::from_str(&json).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn policy_verdict_deny_round_trips() {
        let original = PolicyVerdict::Deny {
            reason: "patient data access outside care team".to_string(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let decoded: PolicyVerdict = serde_json::from_str(&json).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn policy_verdict_require_approval_round_trips() {
        let original = PolicyVerdict::RequireApproval {
            reason: "high-risk prescription".to_string(),
            approver_role: "attending_physician".to_string(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let decoded: PolicyVerdict = serde_json::from_str(&json).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn policy_verdict_require_verification_round_trips() {
        let original = PolicyVerdict::RequireVerification {
            check_id: "phi-content-scan".to_string(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let decoded: PolicyVerdict = serde_json::from_str(&json).unwrap();
        assert_eq!(original, decoded);
    }

    // ── ExecutionId ──────────────────────────────────────────────────────────

    #[test]
    fn execution_id_new_produces_unique_values() {
        let ids: Vec<ExecutionId> = (0..100).map(|_| ExecutionId::new()).collect();

        // All 100 IDs should be distinct.
        let unique: std::collections::HashSet<String> =
            ids.iter().map(|id| id.0.to_string()).collect();
        assert_eq!(unique.len(), 100);
    }

    // ── VeritasError display messages ────────────────────────────────────────

    #[test]
    fn error_policy_denied_display() {
        let err = VeritasError::PolicyDenied {
            reason: "no access".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("policy denied action"));
        assert!(msg.contains("no access"));
    }

    #[test]
    fn error_capability_missing_display() {
        let err = VeritasError::CapabilityMissing {
            capability: "phi:write".to_string(),
            action: "update_record".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("phi:write"));
        assert!(msg.contains("update_record"));
    }

    #[test]
    fn error_verification_failed_display() {
        let err = VeritasError::VerificationFailed {
            reason: "required field missing".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("verification failed"));
        assert!(msg.contains("required field missing"));
    }

    #[test]
    fn error_audit_write_failed_display() {
        let err = VeritasError::AuditWriteFailed {
            reason: "disk full".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("audit write failed"));
        assert!(msg.contains("disk full"));
    }

    #[test]
    fn error_state_machine_error_display() {
        let err = VeritasError::StateMachineError {
            reason: "illegal transition".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("state machine error"));
        assert!(msg.contains("illegal transition"));
    }

    #[test]
    fn error_config_error_display() {
        let err = VeritasError::ConfigError {
            reason: "missing policy path".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("configuration error"));
        assert!(msg.contains("missing policy path"));
    }

    #[test]
    fn error_schema_validation_display() {
        let err = VeritasError::SchemaValidation {
            reason: "type mismatch at $.patient.id".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("schema validation error"));
        assert!(msg.contains("$.patient.id"));
    }
}
