//! Scenario 6: Radiology AI Model Governance
//!
//! A hospital operates a chest X-ray AI model (`chest-xray-v3.2`).  Before
//! any model may produce clinical output it must be registered in the
//! `ModelRegistry` and carry `ApprovalStatus::Approved`.  The VERITAS policy
//! engine enforces this by requiring the `model:approved` capability, which
//! `ModelRegistry::capabilities_for` emits only for approved models.
//!
//! Three sub-cases:
//!
//!   A. Approved model  — `chest-xray-v3.2` (FDA-510k, Approved) runs through
//!      the full VERITAS pipeline and produces a verified radiology report.
//!
//!   B. Unapproved model blocked — `chest-xray-v4.0-beta` (Experimental) is
//!      denied before `propose()` is ever called.  The audit record captures
//!      the denial.
//!
//!   C. Drift detection — `chest-xray-v3.2` starts confident (0.92) but
//!      degrades.  `ModelRegistry::check_and_update` detects Drifted and
//!      auto-revokes the model.

use std::sync::Arc;

use serde_json::json;

use veritas_audit::InMemoryAuditWriter;
use veritas_contracts::{
    agent::{AgentId, AgentInput, AgentOutput, AgentState, ExecutionId},
    capability::{Capability, CapabilitySet},
    error::VeritasResult,
    execution::{StepRecord, StepResult},
    verify::{OutputSchema, VerificationRule, VerificationRuleType},
    ApprovalStatus, DriftMonitor, ModelModality, ModelProvenance,
};
use veritas_core::{
    executor::Executor,
    traits::{Agent, AuditWriter},
};
use veritas_model::{DriftConfig, InMemoryDriftMonitor, ModelRegistry, RegisteredModel};
use veritas_policy::engine::TomlPolicyEngine;
use veritas_verify::engine::SchemaVerifier;

// ── Policy TOML ───────────────────────────────────────────────────────────────

const MODEL_GOVERNANCE_POLICY: &str = include_str!("../../policies/model_governance.toml");

// ── Agent implementation ──────────────────────────────────────────────────────

/// A mock radiology inference agent that returns a hardcoded chest X-ray
/// classification result.  In production this would call the registered model
/// via a VERITAS capability; here it returns a fixed payload for demo clarity.
pub struct RadiologyInferenceAgent {
    /// True when the model is approved and should return a clinical label.
    pub approved: bool,
}

impl Agent for RadiologyInferenceAgent {
    fn propose(&self, _state: &AgentState, _input: &AgentInput) -> VeritasResult<AgentOutput> {
        // The executor guarantees this is never reached for a Denied or
        // CapabilityMissing path.  Both sub-cases A and B share the same
        // agent implementation; only sub-case A will actually call this.
        Ok(AgentOutput {
            kind: "radiology-inference-result".to_string(),
            payload: json!({
                "model_id": "chest-xray-v3.2",
                "finding": "No acute cardiopulmonary abnormality",
                "label": "normal",
                "confidence": 0.92,
                "recommendation": "Routine follow-up; no immediate action required."
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
        // Sub-case A: agent holds model:approved (from registry) + radiology.read.
        // Sub-case B: agent holds only model:chest-xray-v4.0-beta, NOT model:approved.
        // The capability set is constructed externally from the registry, so the
        // agent itself always declares what it needs.
        vec!["model:approved".to_string(), "radiology.read".to_string()]
    }

    fn describe_action(&self, _state: &AgentState, _input: &AgentInput) -> (String, String) {
        let resource = if self.approved {
            "radiology-model"
        } else {
            // Routes to the deny rule in the policy for unapproved models.
            "radiology-model-unapproved"
        };
        ("run-inference".to_string(), resource.to_string())
    }

    fn is_terminal(&self, state: &AgentState) -> bool {
        state.phase == "complete"
    }
}

// ── Output schema ─────────────────────────────────────────────────────────────

fn radiology_output_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "radiology-inference-v1".to_string(),
        json_schema: json!({
            "type": "object",
            "required": ["model_id", "finding", "label", "confidence", "recommendation"]
        }),
        rules: vec![
            VerificationRule {
                rule_id: "req-model-id".to_string(),
                description: "Output must identify the model that produced it".to_string(),
                rule_type: VerificationRuleType::RequiredField {
                    field_path: "model_id".to_string(),
                },
            },
            VerificationRule {
                rule_id: "req-finding".to_string(),
                description: "Output must contain a radiological finding".to_string(),
                rule_type: VerificationRuleType::RequiredField {
                    field_path: "finding".to_string(),
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

struct ArcAudit(Arc<InMemoryAuditWriter>);

impl AuditWriter for ArcAudit {
    fn write(&self, record: &StepRecord) -> VeritasResult<()> {
        self.0.write(record)
    }
    fn finalize(&self, execution_id: &str) -> VeritasResult<()> {
        self.0.finalize(execution_id)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Build the approved chest-xray-v3.2 model.
fn approved_chest_xray() -> RegisteredModel {
    RegisteredModel {
        model_id: "chest-xray-v3.2".to_string(),
        modality: ModelModality::ImageToLabel,
        version: "3.2.0".to_string(),
        provenance: ModelProvenance {
            vendor: "Acme Radiology AI".to_string(),
            training_data_hash: Some("sha256:a3f1c2...".to_string()),
            approval_status: ApprovalStatus::Approved,
            approved_by: Some("Chief Medical Officer".to_string()),
            approved_at: Some("2026-01-10T09:00:00Z".to_string()),
            regulatory_class: Some("FDA-510k".to_string()),
        },
    }
}

/// Build the experimental chest-xray-v4.0-beta model.
fn experimental_chest_xray() -> RegisteredModel {
    RegisteredModel {
        model_id: "chest-xray-v4.0-beta".to_string(),
        modality: ModelModality::ImageToLabel,
        version: "4.0.0-beta".to_string(),
        provenance: ModelProvenance {
            vendor: "Acme Radiology AI".to_string(),
            training_data_hash: None,
            approval_status: ApprovalStatus::Experimental,
            approved_by: None,
            approved_at: None,
            regulatory_class: None,
        },
    }
}

// ── Scenario runner ───────────────────────────────────────────────────────────

/// Run Scenario 6: Radiology AI Model Governance.
pub fn run_scenario() -> VeritasResult<()> {
    println!("=== Scenario 6: Radiology AI Model Governance ===");
    println!();
    println!("  Model modality: ImageToLabel (chest X-ray classification)");
    println!("  Governance:     ModelRegistry + model:approved capability gate");
    println!();

    run_sub_case_a()?;
    run_sub_case_b()?;
    run_sub_case_c()?;

    println!("  Scenario 6 complete.");
    println!();

    Ok(())
}

/// Sub-case A: Approved model runs through the full VERITAS pipeline.
fn run_sub_case_a() -> VeritasResult<()> {
    println!("  ── Sub-case A: Approved model (chest-xray-v3.2, FDA-510k) ──");
    println!();

    // Register the model and derive capabilities from the registry.
    let mut registry = ModelRegistry::new();
    registry.register(approved_chest_xray())?;

    let caps_from_registry = registry.capabilities_for("chest-xray-v3.2");

    println!("  Model registered:    chest-xray-v3.2 (Approved, FDA-510k)");
    println!("  Registry capabilities: {caps_from_registry:?}");
    println!();

    // Build CapabilitySet from registry output + domain capability.
    let mut capabilities = CapabilitySet::default();
    for cap in &caps_from_registry {
        capabilities.grant(Capability::new(cap.as_str()));
    }
    capabilities.grant(Capability::new("radiology.read"));

    let policy = TomlPolicyEngine::from_toml_str(MODEL_GOVERNANCE_POLICY)?;
    let exec_id = ExecutionId::new();
    let audit = Arc::new(InMemoryAuditWriter::new(exec_id.0.to_string()));
    let agent = RadiologyInferenceAgent { approved: true };

    let state = AgentState {
        agent_id: AgentId("radiology-inference-agent".to_string()),
        execution_id: exec_id.clone(),
        phase: "active".to_string(),
        context: serde_json::Value::Null,
        step: 0,
    };

    let input = AgentInput {
        kind: "radiology-inference-request".to_string(),
        payload: json!({
            "image_id": "xray-2026-0313-0042",
            "patient_id": "patient-517",
            "view": "PA"
        }),
    };

    let executor = Executor::new(
        Box::new(policy),
        Box::new(ArcAudit(Arc::clone(&audit))),
        Box::new(SchemaVerifier::new()),
        radiology_output_schema(),
    );

    let result = executor.step(&agent, state, input, &capabilities)?;

    match &result {
        StepResult::Complete { output, .. } | StepResult::Transitioned { output, .. } => {
            let label = output.payload["label"].as_str().unwrap_or("?");
            let confidence = output.payload["confidence"].as_f64().unwrap_or(0.0);
            let recommendation = output.payload["recommendation"].as_str().unwrap_or("?");

            println!("  Action:          run-inference | Resource: radiology-model");
            println!("  Policy verdict:  Allow");
            println!("  Capability check: PASS (model:approved + radiology.read)");
            println!("  Verification:    PASS (all 3 required fields present)");
            println!("  Label:           {label}");
            println!("  Confidence:      {confidence:.2}");
            println!("  Recommendation:  {recommendation}");
        }
        StepResult::Denied { reason, .. } => {
            println!("  DENIED (unexpected): {reason}");
        }
        StepResult::AwaitingApproval { reason, .. } => {
            println!("  AWAITING APPROVAL (unexpected): {reason}");
        }
    }

    let log = audit.export_log();
    println!(
        "  Audit chain:     {} ({} event(s))",
        if audit.verify_integrity() {
            "VERIFIED"
        } else {
            "FAILED"
        },
        log.events.len()
    );
    println!();
    println!("  Sub-case A complete: approved model executed and verified.");
    println!();

    Ok(())
}

/// Sub-case B: Experimental model is blocked by the policy before propose() runs.
fn run_sub_case_b() -> VeritasResult<()> {
    println!("  ── Sub-case B: Unapproved model blocked (chest-xray-v4.0-beta, Experimental) ──");
    println!();

    // Register the experimental model — it will NOT receive model:approved.
    let mut registry = ModelRegistry::new();
    registry.register(experimental_chest_xray())?;

    let caps_from_registry = registry.capabilities_for("chest-xray-v4.0-beta");

    println!("  Model registered:    chest-xray-v4.0-beta (Experimental)");
    println!("  Registry capabilities: {caps_from_registry:?}");
    println!("  Note: model:approved is absent — policy will deny.");
    println!();

    // Build CapabilitySet — model:approved is intentionally NOT present.
    let mut capabilities = CapabilitySet::default();
    for cap in &caps_from_registry {
        capabilities.grant(Capability::new(cap.as_str()));
    }
    capabilities.grant(Capability::new("radiology.read"));

    let policy = TomlPolicyEngine::from_toml_str(MODEL_GOVERNANCE_POLICY)?;
    let exec_id = ExecutionId::new();
    let audit = Arc::new(InMemoryAuditWriter::new(exec_id.0.to_string()));
    // approved=false routes describe_action to "radiology-model-unapproved" resource.
    let agent = RadiologyInferenceAgent { approved: false };

    let state = AgentState {
        agent_id: AgentId("radiology-inference-agent".to_string()),
        execution_id: exec_id.clone(),
        phase: "active".to_string(),
        context: serde_json::Value::Null,
        step: 0,
    };

    let input = AgentInput {
        kind: "radiology-inference-request".to_string(),
        payload: json!({
            "image_id": "xray-2026-0313-0099",
            "patient_id": "patient-518",
            "view": "PA"
        }),
    };

    let executor = Executor::new(
        Box::new(policy),
        Box::new(ArcAudit(Arc::clone(&audit))),
        Box::new(SchemaVerifier::new()),
        radiology_output_schema(),
    );

    let result = executor.step(&agent, state, input, &capabilities)?;

    match &result {
        StepResult::Denied { reason, .. } => {
            println!("  Action:          run-inference | Resource: radiology-model-unapproved");
            println!("  Policy verdict:  Deny");
            println!("  Reason:          {reason}");
            println!("  Agent propose(): NOT called (blocked before capability check)");
        }
        other => {
            println!("  UNEXPECTED result: {other:?}");
        }
    }

    let log = audit.export_log();
    println!(
        "  Audit chain:     {} ({} event(s) — denial recorded)",
        if audit.verify_integrity() {
            "VERIFIED"
        } else {
            "FAILED"
        },
        log.events.len()
    );
    println!();
    println!("  Sub-case B complete: experimental model blocked at policy gate.");
    println!();

    Ok(())
}

/// Sub-case C: Drift detection auto-revokes chest-xray-v3.2.
fn run_sub_case_c() -> VeritasResult<()> {
    println!("  ── Sub-case C: Drift detection auto-revokes chest-xray-v3.2 ──");
    println!();

    // Registry starts with the approved model.
    let mut registry = ModelRegistry::new();
    registry.register(approved_chest_xray())?;

    // Drift monitor: window of 5, warning at 0.10 drop, drift at 0.20 drop.
    let monitor = InMemoryDriftMonitor::new(DriftConfig {
        warning_threshold: 0.10,
        drift_threshold: 0.20,
        window_size: 5,
    });

    println!("  Step 1: Establish baseline — 5 invocations at confidence 0.92");
    for i in 1..=5 {
        let result = json!({ "confidence": 0.92 });
        monitor.record("chest-xray-v3.2", &result)?;
        println!("    Invocation {i}: confidence=0.92");
    }

    let status_after_baseline = registry.check_and_update("chest-xray-v3.2", &monitor)?;
    println!(
        "  Drift check after baseline: {:?} (model still approved: {})",
        status_after_baseline,
        registry.is_approved("chest-xray-v3.2")
    );
    println!();

    println!("  Step 2: Model degrades — 5 invocations at confidence 0.65");
    println!("          (drop = 0.27, exceeds drift_threshold of 0.20)");
    for i in 1..=5 {
        let result = json!({ "confidence": 0.65 });
        monitor.record("chest-xray-v3.2", &result)?;
        println!("    Invocation {i}: confidence=0.65");
    }
    println!();

    let status_after_drift = registry.check_and_update("chest-xray-v3.2", &monitor)?;
    println!("  Drift check after degradation: {status_after_drift:?}");
    println!(
        "  Model still approved: {} (auto-revoked by check_and_update)",
        registry.is_approved("chest-xray-v3.2")
    );

    // Show the revocation reason stored in provenance.
    if let Some(model) = registry.get("chest-xray-v3.2") {
        match &model.provenance.approval_status {
            ApprovalStatus::Revoked { reason } => {
                println!("  Revocation reason:  \"{reason}\"");
            }
            other => {
                println!("  Unexpected status: {other:?}");
            }
        }
    }

    println!();
    println!("  Sub-case C complete: drifted model auto-revoked.");
    println!();

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use veritas_contracts::{
        policy::{PolicyContext, PolicyVerdict},
        DriftMonitor, DriftStatus,
    };
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

    /// Approved radiology model is allowed when it holds model:approved + radiology.read.
    #[test]
    fn test_approved_model_policy_allow() {
        let policy = TomlPolicyEngine::from_toml_str(MODEL_GOVERNANCE_POLICY).unwrap();
        let ctx = make_policy_ctx(
            "run-inference",
            "radiology-model",
            &["model:approved", "radiology.read"],
        );
        let verdict = policy.evaluate(&ctx).unwrap();
        assert_eq!(verdict, PolicyVerdict::Allow);
    }

    /// Unapproved radiology model is denied even when radiology.read is present.
    #[test]
    fn test_unapproved_model_policy_deny() {
        let policy = TomlPolicyEngine::from_toml_str(MODEL_GOVERNANCE_POLICY).unwrap();
        let ctx = make_policy_ctx(
            "run-inference",
            "radiology-model-unapproved",
            &["radiology.read"],
        );
        let verdict = policy.evaluate(&ctx).unwrap();
        match verdict {
            PolicyVerdict::Deny { reason } => {
                assert!(
                    reason.contains("not approved"),
                    "deny reason should mention approval: {reason}"
                );
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    /// ModelRegistry emits model:approved only for approved models.
    #[test]
    fn test_registry_capabilities_approved_vs_experimental() {
        let mut registry = ModelRegistry::new();
        registry.register(approved_chest_xray()).unwrap();
        registry.register(experimental_chest_xray()).unwrap();

        let approved_caps = registry.capabilities_for("chest-xray-v3.2");
        assert!(
            approved_caps.contains(&"model:approved".to_string()),
            "approved model must carry model:approved capability"
        );

        let experimental_caps = registry.capabilities_for("chest-xray-v4.0-beta");
        assert!(
            !experimental_caps.contains(&"model:approved".to_string()),
            "experimental model must NOT carry model:approved capability"
        );
    }

    /// Drift detection auto-revokes an approved model after confidence degrades.
    #[test]
    fn test_drift_auto_revokes_approved_model() {
        let mut registry = ModelRegistry::new();
        registry.register(approved_chest_xray()).unwrap();
        assert!(registry.is_approved("chest-xray-v3.2"));

        let monitor = InMemoryDriftMonitor::new(DriftConfig {
            warning_threshold: 0.10,
            drift_threshold: 0.20,
            window_size: 5,
        });

        // Establish baseline at 0.92.
        for _ in 0..5 {
            monitor
                .record("chest-xray-v3.2", &json!({ "confidence": 0.92 }))
                .unwrap();
        }
        // Degrade to 0.65 — drop of 0.27 exceeds drift_threshold.
        for _ in 0..5 {
            monitor
                .record("chest-xray-v3.2", &json!({ "confidence": 0.65 }))
                .unwrap();
        }

        let status = registry
            .check_and_update("chest-xray-v3.2", &monitor)
            .unwrap();

        assert!(
            matches!(status, DriftStatus::Drifted { .. }),
            "expected Drifted, got {status:?}"
        );
        assert!(
            !registry.is_approved("chest-xray-v3.2"),
            "model must be revoked after drift"
        );
    }

    /// Stable confidence does not revoke the model.
    #[test]
    fn test_stable_confidence_does_not_revoke() {
        let mut registry = ModelRegistry::new();
        registry.register(approved_chest_xray()).unwrap();

        let monitor = InMemoryDriftMonitor::new(DriftConfig {
            warning_threshold: 0.10,
            drift_threshold: 0.20,
            window_size: 5,
        });

        // Keep confidence steady at 0.92.
        for _ in 0..10 {
            monitor
                .record("chest-xray-v3.2", &json!({ "confidence": 0.92 }))
                .unwrap();
        }

        let status = registry
            .check_and_update("chest-xray-v3.2", &monitor)
            .unwrap();

        assert_eq!(status, DriftStatus::Stable);
        assert!(
            registry.is_approved("chest-xray-v3.2"),
            "stable model must remain approved"
        );
    }

    /// Full sub-case A runs without error.
    #[test]
    fn test_sub_case_a_runs() {
        run_sub_case_a().expect("sub-case A should succeed");
    }

    /// Full sub-case B runs without error (denial is an expected outcome, not an Err).
    #[test]
    fn test_sub_case_b_runs() {
        run_sub_case_b().expect("sub-case B should succeed");
    }

    /// Full sub-case C runs without error.
    #[test]
    fn test_sub_case_c_runs() {
        run_sub_case_c().expect("sub-case C should succeed");
    }
}
