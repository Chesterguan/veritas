//! Scenario 1: Drug Interaction Checker
//!
//! Demonstrates VERITAS enforcing capability-gated access to a clinical drug
//! interaction database. The agent checks whether two drugs have a known adverse
//! interaction and returns a structured recommendation.
//!
//! Pipeline walk-through for the demo run:
//!   1. Policy evaluates (drug-interaction-check, drug-database) → Allow
//!   2. Capability check: agent must hold "drug-database.read"
//!   3. Agent calls mock database → structured result with severity
//!   4. Verifier checks required fields: query, result, recommendation
//!   5. State transitions; audit record written to hash chain
//!   6. Audit log integrity verified at the end

use std::sync::Arc;

use serde_json::json;

use veritas_audit::InMemoryAuditWriter;
use veritas_contracts::{
    agent::{AgentId, AgentInput, AgentOutput, AgentState, ExecutionId},
    capability::{Capability, CapabilitySet},
    error::VeritasResult,
    execution::{StepRecord, StepResult},
    verify::{OutputSchema, VerificationRule, VerificationRuleType},
};
use veritas_core::{
    executor::Executor,
    traits::{Agent, AuditWriter},
};
use veritas_policy::engine::TomlPolicyEngine;
use veritas_verify::engine::SchemaVerifier;

use crate::mock_data::check_drug_interaction;

// ── Policy TOML ───────────────────────────────────────────────────────────────

/// Embedded healthcare policy covering the drug interaction scenario.
const HEALTHCARE_POLICY: &str = include_str!("../../policies/healthcare.toml");

// ── Agent implementation ──────────────────────────────────────────────────────

/// An agent that checks for adverse drug interactions using a mock database.
///
/// In a production system this would call a real clinical API. Here it calls
/// `check_drug_interaction` from the mock data module.
pub struct DrugInteractionAgent;

impl Agent for DrugInteractionAgent {
    fn propose(&self, _state: &AgentState, input: &AgentInput) -> VeritasResult<AgentOutput> {
        // Extract drug names from the input payload.
        let drug_a = input.payload["drug_a"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();
        let drug_b = input.payload["drug_b"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

        // Call the mock interaction database.
        let result = check_drug_interaction(&drug_a, &drug_b);

        Ok(AgentOutput {
            kind: "drug-interaction-result".to_string(),
            payload: result,
        })
    }

    fn transition(&self, state: &AgentState, _output: &AgentOutput) -> VeritasResult<AgentState> {
        // Single-step agent: one query completes the execution.
        Ok(AgentState {
            step: state.step + 1,
            phase: "complete".to_string(),
            ..state.clone()
        })
    }

    fn required_capabilities(&self, _state: &AgentState, _input: &AgentInput) -> Vec<String> {
        vec!["drug-database.read".to_string()]
    }

    fn describe_action(&self, _state: &AgentState, _input: &AgentInput) -> (String, String) {
        (
            "drug-interaction-check".to_string(),
            "drug-database".to_string(),
        )
    }

    fn is_terminal(&self, state: &AgentState) -> bool {
        state.phase == "complete"
    }
}

// ── Output schema ─────────────────────────────────────────────────────────────

/// Build the output schema requiring query, result, and recommendation fields.
pub fn drug_interaction_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "drug-interaction-v1".to_string(),
        // JSON Schema: output must be an object with the three required keys.
        json_schema: json!({
            "type": "object",
            "required": ["query", "result", "recommendation"]
        }),
        rules: vec![
            VerificationRule {
                rule_id: "req-query".to_string(),
                description: "Output must contain the queried drug pair".to_string(),
                rule_type: VerificationRuleType::RequiredField {
                    field_path: "query".to_string(),
                },
            },
            VerificationRule {
                rule_id: "req-result".to_string(),
                description: "Output must contain an interaction result with severity".to_string(),
                rule_type: VerificationRuleType::RequiredField {
                    field_path: "result".to_string(),
                },
            },
            VerificationRule {
                rule_id: "req-recommendation".to_string(),
                description: "Output must contain a clinical recommendation".to_string(),
                rule_type: VerificationRuleType::RequiredField {
                    field_path: "recommendation".to_string(),
                },
            },
        ],
    }
}

// ── Arc-wrapped audit writer helper ──────────────────────────────────────────

/// Thin newtype allowing an `Arc<InMemoryAuditWriter>` to be used as
/// `Box<dyn AuditWriter>`.  This lets us retain an inspectable handle after
/// the executor takes ownership via the Box.
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

/// Run Scenario 1: Drug Interaction Checker.
///
/// Tests the warfarin + aspirin pair (known HIGH severity).  Prints each
/// VERITAS pipeline step clearly and verifies the audit chain at the end.
pub fn run_scenario() -> VeritasResult<()> {
    println!("=== Scenario 1: Drug Interaction Checker ===");
    println!();

    // ── Wire up the VERITAS components ────────────────────────────────────────

    let policy = TomlPolicyEngine::from_toml_str(HEALTHCARE_POLICY)?;

    let execution_id = ExecutionId::new();

    // Keep an Arc handle so we can call verify_integrity() after the executor
    // has consumed the Box<dyn AuditWriter>.
    let audit = Arc::new(InMemoryAuditWriter::new(execution_id.0.to_string()));

    let verifier = SchemaVerifier::new();
    let agent = DrugInteractionAgent;

    // ── Build initial agent state ──────────────────────────────────────────────

    let initial_state = AgentState {
        agent_id: AgentId("drug-interaction-agent".to_string()),
        execution_id: execution_id.clone(),
        phase: "active".to_string(),
        context: serde_json::Value::Null,
        step: 0,
    };

    // ── Grant the required capability ─────────────────────────────────────────

    let mut capabilities = CapabilitySet::default();
    capabilities.grant(Capability::new("drug-database.read"));

    println!("  Test: warfarin + aspirin (known HIGH severity interaction)");
    println!("  Action:   drug-interaction-check");
    println!("  Resource: drug-database");
    println!("  Agent capability: drug-database.read [GRANTED]");
    println!();

    let input = AgentInput {
        kind: "drug-interaction-request".to_string(),
        payload: json!({
            "drug_a": "warfarin",
            "drug_b": "aspirin"
        }),
    };

    // ── Run the executor step ─────────────────────────────────────────────────

    let executor = Executor::new(
        Box::new(policy),
        Box::new(ArcAudit(Arc::clone(&audit))),
        Box::new(verifier),
        drug_interaction_schema(),
    );

    let result = executor.step(&agent, initial_state, input, &capabilities)?;

    match &result {
        StepResult::Complete { output, .. } | StepResult::Transitioned { output, .. } => {
            let severity = output.payload["result"]["severity"].as_str().unwrap_or("?");
            let recommendation = output.payload["recommendation"].as_str().unwrap_or("?");

            println!("  Policy verdict:         Allow");
            println!("  Capability check:       PASS");
            println!("  Verification result:    PASS (all 3 required fields present)");
            println!("  Interaction severity:   {severity}");
            println!("  Recommendation:         {recommendation}");
        }
        StepResult::Denied { reason, .. } => {
            println!("  DENIED: {reason}");
        }
        StepResult::AwaitingApproval { reason, .. } => {
            println!("  AWAITING APPROVAL: {reason}");
        }
    }

    println!();

    // ── Verify audit chain integrity ──────────────────────────────────────────

    let integrity_ok = audit.verify_integrity().unwrap_or(false);
    let log = audit.export_log().unwrap();

    println!(
        "  Audit chain integrity:  {} ({} event(s) in chain)",
        if integrity_ok { "VERIFIED" } else { "FAILED" },
        log.events.len()
    );
    println!();
    println!("  Scenario 1 complete.");
    println!();

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use veritas_contracts::policy::{PolicyContext, PolicyVerdict};
    use veritas_core::traits::{PolicyEngine, Verifier};

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

    /// Policy allows the drug interaction check when the agent holds drug-database.read.
    #[test]
    fn test_policy_allows_drug_interaction_check() {
        let policy = TomlPolicyEngine::from_toml_str(HEALTHCARE_POLICY).unwrap();
        let ctx = make_policy_ctx(
            "drug-interaction-check",
            "drug-database",
            &["drug-database.read"],
        );
        assert_eq!(policy.evaluate(&ctx).unwrap(), PolicyVerdict::Allow);
    }

    /// Deny-by-default: the action is denied when the required capability is absent.
    #[test]
    fn test_policy_denies_without_capability() {
        let policy = TomlPolicyEngine::from_toml_str(HEALTHCARE_POLICY).unwrap();
        let ctx = make_policy_ctx("drug-interaction-check", "drug-database", &[]);
        match policy.evaluate(&ctx).unwrap() {
            PolicyVerdict::Deny { .. } => {}
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    /// The drug interaction output schema accepts the agent's actual output payload.
    #[test]
    fn test_schema_verification_passes() {
        let verifier = SchemaVerifier::new();
        let schema = drug_interaction_schema();

        // Replicate exactly what DrugInteractionAgent produces via check_drug_interaction.
        let payload = crate::mock_data::check_drug_interaction("warfarin", "aspirin");
        let output = veritas_contracts::agent::AgentOutput {
            kind: "drug-interaction-result".to_string(),
            payload,
        };

        let report = verifier.verify(&output, &schema).unwrap();
        assert!(
            report.passed,
            "schema verification must pass for valid payload; failures: {:?}",
            report.failures
        );
    }

    /// Full scenario runner succeeds end-to-end (sub-case A: warfarin + aspirin).
    #[test]
    fn test_sub_case_a_runs() {
        run_scenario().expect("drug interaction scenario should succeed");
    }
}
