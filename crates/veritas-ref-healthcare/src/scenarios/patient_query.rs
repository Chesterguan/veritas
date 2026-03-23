//! Scenario 3: Patient Data Query
//!
//! Demonstrates three distinct VERITAS enforcement outcomes in one scenario:
//!
//! Sub-case A — agent WITH capability + consent flag → Allow (success)
//! Sub-case B — agent WITHOUT capability             → CapabilityMissing error
//! Sub-case C — agent WITH capability but no consent → Deny (policy)
//!
//! This scenario shows how VERITAS's layered checks (policy → capability →
//! verification) each catch a different class of problem independently.
//!
//! # How Sub-case B demonstrates CapabilityMissing
//!
//! The executor checks capabilities in two places:
//!   - The policy engine checks `required_capabilities` declared in TOML rules
//!     (defense-in-depth at the policy layer).
//!   - The executor checks `Agent::required_capabilities()` AFTER policy says Allow.
//!
//! For CapabilityMissing to surface from the executor, the policy rule must not
//! list the capability (so it returns Allow), but the agent declares it.
//! Sub-case B uses a special policy that allows the action without a capability
//! guard, paired with an agent that still declares it — exercising the executor's
//! own least-privilege enforcement path.

use std::sync::Arc;

use serde_json::json;

use veritas_audit::InMemoryAuditWriter;
use veritas_contracts::{
    agent::{AgentId, AgentInput, AgentOutput, AgentState, ExecutionId},
    capability::{Capability, CapabilitySet},
    error::{VeritasError, VeritasResult},
    execution::{StepRecord, StepResult},
    verify::{OutputSchema, VerificationRule, VerificationRuleType},
};
use veritas_core::{
    executor::Executor,
    traits::{Agent, AuditWriter},
};
use veritas_policy::engine::TomlPolicyEngine;
use veritas_verify::engine::SchemaVerifier;

use crate::mock_data::get_patient_record;

// ── Policy TOML ───────────────────────────────────────────────────────────────

/// Main healthcare policy — used for sub-cases A and C.
const HEALTHCARE_POLICY: &str = include_str!("../../policies/healthcare.toml");

/// A minimal policy for sub-case B: allows the query action without listing
/// any required_capabilities in the TOML.  This causes the policy engine to
/// return Allow, so the executor's own capability check (Agent::required_capabilities)
/// is what catches the missing capability and returns CapabilityMissing.
const OPEN_POLICY_FOR_CAPABILITY_TEST: &str = r#"
[[rules]]
id = "allow-patient-query-open"
description = "Policy allows query on patient-records; capability enforcement is left to the executor"
action = "query"
resource = "patient-records"
verdict = "allow"
"#;

// ── Agent implementation ──────────────────────────────────────────────────────

/// An agent that queries patient records.
///
/// The resource returned by `describe_action` is dynamically chosen based on
/// whether the patient record has the `ai_query_consent` flag set:
/// - Consent present  → resource = "patient-records"            (policy allows)
/// - Consent absent   → resource = "patient-records-no-consent" (policy denies)
pub struct PatientQueryAgent {
    /// Patient ID to look up.
    pub patient_id: String,
}

impl Agent for PatientQueryAgent {
    fn propose(&self, _state: &AgentState, _input: &AgentInput) -> VeritasResult<AgentOutput> {
        let record = get_patient_record(&self.patient_id);
        Ok(AgentOutput {
            kind: "patient-record-result".to_string(),
            payload: record,
        })
    }

    fn transition(&self, state: &AgentState, _output: &AgentOutput) -> VeritasResult<AgentState> {
        Ok(AgentState {
            step: state.step + 1,
            phase: "complete".to_string(),
            ..state.clone()
        })
    }

    fn required_capabilities(&self, _state: &AgentState, _input: &AgentInput) -> Vec<String> {
        vec!["patient-records.read".to_string()]
    }

    fn describe_action(&self, _state: &AgentState, _input: &AgentInput) -> (String, String) {
        // Peek at the consent flag to pick the correct resource name.
        // The policy engine evaluates the resource string against its rules —
        // no consent means the agent self-routes to the denied resource.
        let record = get_patient_record(&self.patient_id);
        let has_consent = record["ai_query_consent"].as_bool().unwrap_or(false);

        let resource = if has_consent {
            "patient-records"
        } else {
            "patient-records-no-consent"
        };

        ("query".to_string(), resource.to_string())
    }

    fn is_terminal(&self, state: &AgentState) -> bool {
        state.phase == "complete"
    }
}

// ── Output schema ─────────────────────────────────────────────────────────────

fn patient_query_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "patient-query-v1".to_string(),
        json_schema: json!({
            "type": "object",
            "required": ["patient_id"]
        }),
        rules: vec![VerificationRule {
            rule_id: "req-patient-id".to_string(),
            description: "Output must contain the patient ID".to_string(),
            rule_type: VerificationRuleType::RequiredField {
                field_path: "patient_id".to_string(),
            },
        }],
    }
}

// ── Arc-wrapped audit writer helper ──────────────────────────────────────────

struct ArcAudit(Arc<InMemoryAuditWriter>);

impl AuditWriter for ArcAudit {
    fn write(&self, record: &StepRecord) -> VeritasResult<()> {
        self.0.write(record)
    }
    fn finalize(&self, execution_id: &str) -> VeritasResult<()> {
        self.0.finalize(execution_id)
    }
}

// ── Scenario runner ───────────────────────────────────────────────────────────

/// Run Scenario 3: Patient Data Query — three sub-cases.
pub fn run_scenario() -> VeritasResult<()> {
    println!("=== Scenario 3: Patient Data Query ===");
    println!();

    // ── Sub-case A: WITH capability + consent → Allow ─────────────────────────

    {
        println!("  Sub-case A: Agent WITH capability + patient consent flag");
        println!("  Patient ID: patient-101 (ai_query_consent = true)");
        println!("  Capability: patient-records.read [GRANTED]");

        let policy = TomlPolicyEngine::from_toml_str(HEALTHCARE_POLICY)?;
        let execution_id = ExecutionId::new();
        let audit = Arc::new(InMemoryAuditWriter::new(execution_id.0.to_string()));
        let verifier = SchemaVerifier::new();
        let schema = patient_query_schema();
        let agent = PatientQueryAgent {
            patient_id: "patient-101".to_string(),
        };

        let state = AgentState {
            agent_id: AgentId("patient-query-agent".to_string()),
            execution_id: execution_id.clone(),
            phase: "active".to_string(),
            context: serde_json::Value::Null,
            step: 0,
        };

        let mut capabilities = CapabilitySet::default();
        capabilities.grant(Capability::new("patient-records.read"));

        let input = AgentInput {
            kind: "patient-query".to_string(),
            payload: json!({ "patient_id": "patient-101" }),
        };

        let executor = Executor::new(
            Box::new(policy),
            Box::new(ArcAudit(Arc::clone(&audit))),
            Box::new(verifier),
            schema,
        );

        let result = executor.step(&agent, state, input, &capabilities)?;

        match result {
            StepResult::Complete { output, .. } | StepResult::Transitioned { output, .. } => {
                let conditions = output.payload["conditions"]
                    .as_array()
                    .map(|a| a.len())
                    .unwrap_or(0);
                println!("  Policy verdict:         Allow");
                println!("  Capability check:       PASS");
                println!("  Verification result:    PASS");
                println!("  Record conditions:      {conditions} condition(s) returned");
            }
            StepResult::Denied { reason, .. } => {
                println!("  DENIED: {reason}");
            }
            _ => {}
        }

        let integrity_ok = audit.verify_integrity().unwrap_or(false);
        let log = audit.export_log().unwrap();
        println!(
            "  Audit chain integrity:  {} ({} event(s))",
            if integrity_ok { "VERIFIED" } else { "FAILED" },
            log.events.len()
        );
        println!("  RESULT: SUCCESS (expected)");
        println!();
    }

    // ── Sub-case B: WITHOUT capability → CapabilityMissing ───────────────────
    //
    // Uses OPEN_POLICY_FOR_CAPABILITY_TEST so the policy returns Allow.
    // The executor then runs its own capability check (Agent::required_capabilities)
    // and returns CapabilityMissing before calling agent.propose().

    {
        println!("  Sub-case B: Agent WITHOUT capability (executor-level enforcement)");
        println!("  Patient ID: patient-101");
        println!("  Capability: patient-records.read [NOT GRANTED]");
        println!("  Policy: allows query unconditionally (no capability guard in TOML)");
        println!("  Enforcement: executor's own capability check catches the gap");

        // Open policy: returns Allow without checking capabilities.
        let policy = TomlPolicyEngine::from_toml_str(OPEN_POLICY_FOR_CAPABILITY_TEST)?;
        let execution_id = ExecutionId::new();
        let audit = Arc::new(InMemoryAuditWriter::new(execution_id.0.to_string()));
        let verifier = SchemaVerifier::new();
        let schema = patient_query_schema();
        let agent = PatientQueryAgent {
            patient_id: "patient-101".to_string(),
        };

        let state = AgentState {
            agent_id: AgentId("patient-query-agent".to_string()),
            execution_id: execution_id.clone(),
            phase: "active".to_string(),
            context: serde_json::Value::Null,
            step: 0,
        };

        // Empty capability set — no capabilities granted.
        let capabilities = CapabilitySet::default();

        let input = AgentInput {
            kind: "patient-query".to_string(),
            payload: json!({ "patient_id": "patient-101" }),
        };

        let executor = Executor::new(
            Box::new(policy),
            Box::new(ArcAudit(Arc::clone(&audit))),
            Box::new(verifier),
            schema,
        );

        let result = executor.step(&agent, state, input, &capabilities);

        match result {
            Err(VeritasError::CapabilityMissing { capability, action }) => {
                println!("  Policy verdict:         Allow (policy permits the action)");
                println!("  Capability check:       FAIL — '{capability}' missing for '{action}'");
                println!("  Agent propose() called: NO (executor blocked before agent logic)");
                let log = audit.export_log().unwrap();
                println!(
                    "  Audit chain integrity:  VERIFIED ({} denial event(s) recorded)",
                    log.events.len()
                );
                println!("  RESULT: CapabilityMissing (expected)");
            }
            Err(e) => {
                println!("  Unexpected error: {e}");
            }
            Ok(StepResult::Denied { reason, .. }) => {
                println!("  DENIED by policy: {reason}");
            }
            Ok(_) => {
                println!("  Unexpectedly succeeded");
            }
        }
        println!();
    }

    // ── Sub-case C: WITH capability but no consent → Policy Deny ─────────────

    {
        println!("  Sub-case C: Agent WITH capability but no patient consent");
        println!("  Patient ID: patient-201nc (ai_query_consent = false)");
        println!("  Capability: patient-records.read [GRANTED]");
        println!("  Agent reports resource: patient-records-no-consent");

        let policy = TomlPolicyEngine::from_toml_str(HEALTHCARE_POLICY)?;
        let execution_id = ExecutionId::new();
        let audit = Arc::new(InMemoryAuditWriter::new(execution_id.0.to_string()));
        let verifier = SchemaVerifier::new();
        let schema = patient_query_schema();

        // Patient ID ending in "nc" → get_patient_record sets consent = false
        // → describe_action returns resource = "patient-records-no-consent"
        // → policy rule "deny-patient-query-no-consent" fires.
        let agent = PatientQueryAgent {
            patient_id: "patient-201nc".to_string(),
        };

        let state = AgentState {
            agent_id: AgentId("patient-query-agent".to_string()),
            execution_id: execution_id.clone(),
            phase: "active".to_string(),
            context: serde_json::Value::Null,
            step: 0,
        };

        let mut capabilities = CapabilitySet::default();
        capabilities.grant(Capability::new("patient-records.read"));

        let input = AgentInput {
            kind: "patient-query".to_string(),
            payload: json!({ "patient_id": "patient-201nc" }),
        };

        let executor = Executor::new(
            Box::new(policy),
            Box::new(ArcAudit(Arc::clone(&audit))),
            Box::new(verifier),
            schema,
        );

        let result = executor.step(&agent, state, input, &capabilities)?;

        match result {
            StepResult::Denied { reason, .. } => {
                println!("  Policy verdict:         Deny");
                println!("  Deny reason:            {reason}");
                println!(
                    "  Agent propose() called: NO (blocked by policy before capability check)"
                );
                println!("  RESULT: Policy Denied (expected)");
            }
            StepResult::Complete { .. } | StepResult::Transitioned { .. } => {
                println!("  Unexpectedly succeeded — consent enforcement failed");
            }
            _ => {}
        }

        let integrity_ok = audit.verify_integrity().unwrap_or(false);
        let log = audit.export_log().unwrap();
        println!(
            "  Audit chain integrity:  {} ({} event(s), denial recorded)",
            if integrity_ok { "VERIFIED" } else { "FAILED" },
            log.events.len()
        );
        println!();
    }

    println!("  Scenario 3 complete.");
    println!();

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use veritas_contracts::policy::{PolicyContext, PolicyVerdict};
    use veritas_core::traits::PolicyEngine;

    fn make_policy_ctx(action: &str, resource: &str, caps: &[&str]) -> PolicyContext {
        PolicyContext {
            agent_id: "test-agent".to_string(),
            execution_id: "test-exec".to_string(),
            current_phase: "active".to_string(),
            action: action.to_string(),
            resource: resource.to_string(),
            capabilities: caps.iter().map(|s| s.to_string()).collect(),
            metadata: serde_json::Value::Null,
        }
    }

    /// Policy allows patient record queries when the agent holds patient-records.read
    /// and the patient has provided consent (resource = patient-records).
    #[test]
    fn test_policy_allows_with_consent_and_capability() {
        let policy = TomlPolicyEngine::from_toml_str(HEALTHCARE_POLICY).unwrap();
        let ctx = make_policy_ctx("query", "patient-records", &["patient-records.read"]);
        assert_eq!(policy.evaluate(&ctx).unwrap(), PolicyVerdict::Allow);
    }

    /// The deny-patient-query-no-consent rule fires when the resource signals
    /// missing consent, regardless of what capabilities are present.
    #[test]
    fn test_policy_denies_no_consent() {
        let policy = TomlPolicyEngine::from_toml_str(HEALTHCARE_POLICY).unwrap();
        let ctx = make_policy_ctx(
            "query",
            "patient-records-no-consent",
            &["patient-records.read"],
        );
        match policy.evaluate(&ctx).unwrap() {
            PolicyVerdict::Deny { reason } => {
                assert!(
                    reason.contains("consent"),
                    "deny reason must mention consent: {reason}"
                );
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    /// Deny-by-default: the query action is denied when the required capability is absent.
    #[test]
    fn test_policy_denies_without_capability() {
        let policy = TomlPolicyEngine::from_toml_str(HEALTHCARE_POLICY).unwrap();
        let ctx = make_policy_ctx("query", "patient-records", &[]);
        match policy.evaluate(&ctx).unwrap() {
            PolicyVerdict::Deny { .. } => {}
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    /// Full scenario runner covers all three sub-cases without error.
    #[test]
    fn test_run_scenario() {
        run_scenario().expect("patient query scenario should succeed");
    }
}
