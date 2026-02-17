//! Scenario 2: Clinical Note Summarizer
//!
//! Demonstrates VERITAS verifying that LLM-generated clinical note summaries
//! do not contain PII labels (DOB:, SSN:) before delivery.  A custom verifier
//! rule enforces the PII check — keeping healthcare-specific logic outside the
//! trusted runtime core.
//!
//! Pipeline walk-through for the demo run:
//!   1. Policy evaluates (summarize, clinical-notes) → Allow
//!   2. Capability check: agent must hold "clinical-notes.read"
//!   3. Agent returns a deterministic mock summary (simulating LLM output)
//!   4. Verifier runs the registered "no-pii-labels" custom rule
//!   5. State transitions; audit record written
//!   6. Audit chain integrity verified at the end

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
use veritas_core::{executor::Executor, traits::{Agent, AuditWriter}};
use veritas_policy::engine::TomlPolicyEngine;
use veritas_verify::engine::SchemaVerifier;

use crate::mock_data::get_patient_notes;

// ── Policy TOML ───────────────────────────────────────────────────────────────

const HEALTHCARE_POLICY: &str = include_str!("../../policies/healthcare.toml");

// ── Agent implementation ──────────────────────────────────────────────────────

/// An agent that summarizes clinical notes, simulating a call to an LLM.
///
/// The summary is deterministic and hardcoded to keep the demo reproducible.
/// In production this would call an LLM API and the output would vary.
pub struct NoteSummarizerAgent;

impl Agent for NoteSummarizerAgent {
    fn propose(&self, state: &AgentState, input: &AgentInput) -> VeritasResult<AgentOutput> {
        let patient_id = input.payload["patient_id"]
            .as_str()
            .unwrap_or("unknown");

        // Fetch mock clinical notes to simulate reading source data.
        let notes = get_patient_notes(patient_id);
        let note_count = notes["notes"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0);

        // Deterministic mock summary — simulates what an LLM would produce.
        // Deliberately contains no PII labels (DOB:, SSN:) so the verifier passes.
        let summary = format!(
            "Patient (ID: {}) presents with a history reviewed across {} clinical notes. \
             Key findings: mild anemia (Hgb 10.2 g/dL) identified in follow-up labs, \
             with type 2 diabetes and hypertension as active chronic conditions. \
             Current medications include metformin, lisinopril, and iron supplementation. \
             Renal function is preserved (eGFR 74). \
             Plan: continue current regimen, recheck CBC in four weeks, \
             refer to hematology if no improvement.",
            patient_id, note_count
        );

        Ok(AgentOutput {
            kind: "clinical-summary".to_string(),
            payload: json!({
                "patient_id": patient_id,
                "note_count": note_count,
                "summary": summary,
                "generated_by": state.agent_id.0
            }),
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
        vec!["clinical-notes.read".to_string()]
    }

    fn describe_action(&self, _state: &AgentState, _input: &AgentInput) -> (String, String) {
        ("summarize".to_string(), "clinical-notes".to_string())
    }

    fn is_terminal(&self, state: &AgentState) -> bool {
        state.phase == "complete"
    }
}

// ── Output schema with PII custom rule ───────────────────────────────────────

/// Build the output schema for clinical note summaries.
///
/// Requires patient_id, summary, and note_count fields.  Registers a custom
/// rule "no-pii-labels" that checks the summary text for PII label patterns.
fn note_summarizer_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "clinical-summary-v1".to_string(),
        json_schema: json!({
            "type": "object",
            "required": ["patient_id", "summary", "note_count"]
        }),
        rules: vec![
            VerificationRule {
                rule_id: "req-patient-id".to_string(),
                description: "Output must identify the patient".to_string(),
                rule_type: VerificationRuleType::RequiredField {
                    field_path: "patient_id".to_string(),
                },
            },
            VerificationRule {
                rule_id: "req-summary".to_string(),
                description: "Output must contain a summary text".to_string(),
                rule_type: VerificationRuleType::RequiredField {
                    field_path: "summary".to_string(),
                },
            },
            // Custom rule: delegate PII label detection to a registered function.
            // This keeps the verifier generic; healthcare logic lives in the adapter.
            VerificationRule {
                rule_id: "no-pii-labels".to_string(),
                description: "Summary must not contain PII labels such as DOB: or SSN:".to_string(),
                rule_type: VerificationRuleType::Custom {
                    function_name: "no-pii-labels".to_string(),
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

/// Run Scenario 2: Clinical Note Summarizer.
///
/// Demonstrates the custom PII verifier rule passing on a clean summary.
/// Also shows the full VERITAS pipeline and exports the audit log.
pub fn run_scenario() -> VeritasResult<()> {
    println!("=== Scenario 2: Clinical Note Summarizer ===");
    println!();

    let patient_id = "patient-042";

    // ── Wire up the VERITAS components ────────────────────────────────────────

    let policy = TomlPolicyEngine::from_toml_str(HEALTHCARE_POLICY)?;

    let execution_id = ExecutionId::new();
    let audit_inner = Arc::new(InMemoryAuditWriter::new(execution_id.0.to_string()));

    // Build the verifier and register the PII label detection custom rule.
    let mut verifier = SchemaVerifier::new();
    verifier.register_rule(
        "no-pii-labels",
        Box::new(|payload| {
            // Check the "summary" field for forbidden PII label patterns.
            let summary = payload["summary"].as_str().unwrap_or("");
            let forbidden = ["DOB:", "SSN:", "MRN:", "Date of Birth:"];
            for label in &forbidden {
                if summary.contains(label) {
                    return Some(format!(
                        "summary contains forbidden PII label '{}'; remove before delivery",
                        label
                    ));
                }
            }
            None
        }),
    );

    let schema = note_summarizer_schema();
    let agent = NoteSummarizerAgent;

    let initial_state = AgentState {
        agent_id: AgentId("note-summarizer-agent".to_string()),
        execution_id: execution_id.clone(),
        phase: "active".to_string(),
        context: serde_json::Value::Null,
        step: 0,
    };

    let mut capabilities = CapabilitySet::default();
    capabilities.grant(Capability::new("clinical-notes.read"));

    println!("  Test: summarize clinical notes for patient '{}'", patient_id);
    println!("  Action:   summarize");
    println!("  Resource: clinical-notes");
    println!("  Agent capability: clinical-notes.read [GRANTED]");
    println!("  Custom verifier rule: no-pii-labels [REGISTERED]");
    println!();

    let input = AgentInput {
        kind: "summarize-request".to_string(),
        payload: json!({ "patient_id": patient_id }),
    };

    let executor = Executor::new(
        Box::new(policy),
        Box::new(ArcAudit(Arc::clone(&audit_inner))),
        Box::new(verifier),
        schema,
    );

    let result = executor.step(&agent, initial_state, input, &capabilities)?;

    match &result {
        StepResult::Complete { output, .. } | StepResult::Transitioned { output, .. } => {
            let summary = output.payload["summary"]
                .as_str()
                .unwrap_or("?");
            let note_count = output.payload["note_count"].as_u64().unwrap_or(0);

            println!("  Policy verdict:         Allow");
            println!("  Capability check:       PASS");
            println!("  PII label check:        PASS (no forbidden labels detected)");
            println!("  Verification result:    PASS");
            println!("  Notes summarized:       {}", note_count);
            println!("  Summary preview:        {}...", &summary[..summary.len().min(120)]);
        }
        StepResult::Denied { reason, .. } => {
            println!("  DENIED: {}", reason);
        }
        StepResult::AwaitingApproval { reason, .. } => {
            println!("  AWAITING APPROVAL: {}", reason);
        }
    }

    println!();

    // ── Verify audit chain integrity ──────────────────────────────────────────

    let integrity_ok = audit_inner.verify_integrity();
    let log = audit_inner.export_log();

    println!(
        "  Audit chain integrity:  {} ({} event(s) in chain)",
        if integrity_ok { "VERIFIED" } else { "FAILED" },
        log.events.len()
    );
    println!();
    println!("  Scenario 2 complete.");
    println!();

    Ok(())
}
