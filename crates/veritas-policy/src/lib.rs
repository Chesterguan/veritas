//! # veritas-policy
//!
//! A TOML-driven, deny-by-default policy engine for the VERITAS runtime.
//!
//! ## Overview
//!
//! This crate provides [`TomlPolicyEngine`], which implements the
//! [`PolicyEngine`](veritas_core::traits::PolicyEngine) trait.  Rules are
//! declared in a TOML file, evaluated in order, and the first matching rule
//! wins.  If no rule matches, the request is denied.
//!
//! ## Quick start
//!
//! ```rust,ignore
//! use std::path::Path;
//! use veritas_policy::engine::TomlPolicyEngine;
//!
//! let engine = TomlPolicyEngine::from_file(Path::new("policies/healthcare.toml"))?;
//! // Pass `engine` to `veritas_core::Executor::new(...)`.
//! ```
//!
//! ## Rule matching
//!
//! Each rule specifies an `action` and `resource` pattern.  Both support the
//! wildcard `"*"` which matches any value.  Rules are applied in declaration
//! order; the first match wins.

pub mod engine;
pub mod rule;

pub use engine::TomlPolicyEngine;
pub use rule::{PolicyConfig, PolicyRule, RuleVerdict};

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use veritas_contracts::policy::{PolicyContext, PolicyVerdict};
    use veritas_core::traits::PolicyEngine;

    use crate::TomlPolicyEngine;

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Build a minimal `PolicyContext` for testing.  Capabilities default to
    /// empty; pass non-empty slices to test capability checks.
    fn ctx(action: &str, resource: &str, capabilities: &[&str]) -> PolicyContext {
        PolicyContext {
            agent_id: "test-agent".to_string(),
            execution_id: "exec-001".to_string(),
            current_phase: "active".to_string(),
            action: action.to_string(),
            resource: resource.to_string(),
            capabilities: capabilities.iter().map(|s| s.to_string()).collect(),
            metadata: serde_json::Value::Null,
        }
    }

    // ── 1. deny-by-default ────────────────────────────────────────────────────

    /// When no rules exist, every request must be denied.
    #[test]
    fn test_deny_by_default() {
        let toml = r#"
            rules = []
        "#;

        let engine = TomlPolicyEngine::from_toml_str(toml).unwrap();
        let verdict = engine.evaluate(&ctx("read_record", "patient/42", &[])).unwrap();

        match verdict {
            PolicyVerdict::Deny { reason } => {
                assert!(
                    reason.contains("denied by default"),
                    "expected 'denied by default' in reason, got: {reason}"
                );
            }
            other => panic!("expected Deny, got {:?}", other),
        }
    }

    // ── 2. explicit allow ─────────────────────────────────────────────────────

    /// A matching allow rule with no required capabilities returns Allow.
    #[test]
    fn test_explicit_allow() {
        let toml = r#"
            [[rules]]
            id = "allow-read"
            description = "Allow reading patient records"
            action = "read_record"
            resource = "patient/42"
            verdict = "allow"
        "#;

        let engine = TomlPolicyEngine::from_toml_str(toml).unwrap();
        let verdict = engine.evaluate(&ctx("read_record", "patient/42", &[])).unwrap();

        assert_eq!(verdict, PolicyVerdict::Allow);
    }

    // ── 3. explicit deny ──────────────────────────────────────────────────────

    /// A matching deny rule returns Deny with the configured reason.
    #[test]
    fn test_explicit_deny() {
        let toml = r#"
            [[rules]]
            id = "deny-delete"
            description = "Agents may not delete patient records"
            action = "delete_record"
            resource = "*"
            verdict = "deny"
            deny_reason = "deletion of patient records is prohibited"
        "#;

        let engine = TomlPolicyEngine::from_toml_str(toml).unwrap();
        let verdict = engine.evaluate(&ctx("delete_record", "patient/99", &[])).unwrap();

        match verdict {
            PolicyVerdict::Deny { reason } => {
                assert!(
                    reason.contains("deletion of patient records is prohibited"),
                    "unexpected reason: {reason}"
                );
            }
            other => panic!("expected Deny, got {:?}", other),
        }
    }

    // ── 4. require-approval ───────────────────────────────────────────────────

    /// A matching require-approval rule returns RequireApproval with the
    /// configured reason and approver_role.
    #[test]
    fn test_require_approval() {
        let toml = r#"
            [[rules]]
            id = "approve-prescribe"
            description = "Prescriptions require physician approval"
            action = "prescribe_medication"
            resource = "*"
            verdict = "require-approval"
            approval_reason = "high-risk prescription requires sign-off"
            approver_role = "attending_physician"
        "#;

        let engine = TomlPolicyEngine::from_toml_str(toml).unwrap();
        let verdict = engine.evaluate(&ctx("prescribe_medication", "patient/7", &[])).unwrap();

        match verdict {
            PolicyVerdict::RequireApproval { reason, approver_role } => {
                assert!(
                    reason.contains("high-risk prescription"),
                    "unexpected reason: {reason}"
                );
                assert_eq!(approver_role, "attending_physician");
            }
            other => panic!("expected RequireApproval, got {:?}", other),
        }
    }

    // ── 5. wildcard matching ──────────────────────────────────────────────────

    /// A rule with action="*" should match any action.
    /// A rule with resource="*" should match any resource.
    #[test]
    fn test_wildcard_matching() {
        let toml = r#"
            [[rules]]
            id = "allow-all-reads"
            description = "Any read on any resource is allowed"
            action = "read_record"
            resource = "*"
            verdict = "allow"

            [[rules]]
            id = "deny-all-writes"
            description = "Any write action on any resource is denied"
            action = "*"
            resource = "*"
            verdict = "deny"
            deny_reason = "write operations are not permitted"
        "#;

        let engine = TomlPolicyEngine::from_toml_str(toml).unwrap();

        // Wildcard resource: any resource string should match.
        assert_eq!(
            engine.evaluate(&ctx("read_record", "patient/1", &[])).unwrap(),
            PolicyVerdict::Allow
        );
        assert_eq!(
            engine.evaluate(&ctx("read_record", "some/other/resource", &[])).unwrap(),
            PolicyVerdict::Allow
        );

        // Wildcard action: an action not matched by the first rule falls through
        // to the wildcard action rule.
        match engine.evaluate(&ctx("update_record", "patient/1", &[])).unwrap() {
            PolicyVerdict::Deny { reason } => {
                assert!(reason.contains("write operations are not permitted"));
            }
            other => panic!("expected Deny from wildcard action rule, got {:?}", other),
        }
    }

    // ── 6. first-match wins ───────────────────────────────────────────────────

    /// When two rules match the same action and resource, only the first one
    /// should produce a verdict.
    #[test]
    fn test_first_match_wins() {
        let toml = r#"
            [[rules]]
            id = "first-allow"
            description = "First rule: allow"
            action = "read_record"
            resource = "*"
            verdict = "allow"

            [[rules]]
            id = "second-deny"
            description = "Second rule: deny (must never be reached)"
            action = "read_record"
            resource = "*"
            verdict = "deny"
            deny_reason = "this rule should never fire"
        "#;

        let engine = TomlPolicyEngine::from_toml_str(toml).unwrap();
        let verdict = engine.evaluate(&ctx("read_record", "patient/5", &[])).unwrap();

        // The first rule matches, so we get Allow — not Deny from the second.
        assert_eq!(verdict, PolicyVerdict::Allow);
    }

    // ── 7. capability mismatch overrides allow ────────────────────────────────

    /// Even when a rule's action/resource patterns match and its verdict is
    /// `allow`, the engine must deny if the agent lacks a required capability.
    #[test]
    fn test_capability_mismatch_on_allow_rule() {
        let toml = r#"
            [[rules]]
            id = "phi-read-allow"
            description = "Allow PHI reads for agents with phi:read capability"
            action = "read_phi"
            resource = "*"
            required_capabilities = ["phi:read"]
            verdict = "allow"
        "#;

        let engine = TomlPolicyEngine::from_toml_str(toml).unwrap();

        // Agent holds no capabilities — must be denied despite the allow rule.
        let verdict = engine.evaluate(&ctx("read_phi", "patient/33", &[])).unwrap();

        match verdict {
            PolicyVerdict::Deny { reason } => {
                assert!(
                    reason.contains("phi:read"),
                    "deny reason should mention the missing capability: {reason}"
                );
            }
            other => panic!("expected Deny due to missing capability, got {:?}", other),
        }

        // Agent holds the required capability — must now be allowed.
        let verdict_with_cap =
            engine.evaluate(&ctx("read_phi", "patient/33", &["phi:read"])).unwrap();
        assert_eq!(verdict_with_cap, PolicyVerdict::Allow);
    }

    // ── 8. TOML parse error ───────────────────────────────────────────────────

    /// Malformed TOML must produce a `VeritasError::ConfigError`.
    #[test]
    fn test_toml_parse_error() {
        let bad_toml = r#"
            this is not valid toml ][[[
        "#;

        let result = TomlPolicyEngine::from_toml_str(bad_toml);

        match result {
            Err(veritas_contracts::error::VeritasError::ConfigError { reason }) => {
                assert!(
                    reason.contains("failed to parse policy TOML"),
                    "expected parse error message, got: {reason}"
                );
            }
            other => panic!("expected ConfigError, got {:?}", other),
        }
    }
}
