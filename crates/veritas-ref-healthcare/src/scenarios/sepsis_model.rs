//! Scenario 7: Sepsis Risk Model with Drift
//!
//! A `TabularToScore` model predicts sepsis risk from patient vital signs.
//! The scenario demonstrates the full model governance lifecycle: normal
//! operation, a warning phase, drift detection, and automatic revocation.
//!
//! Two sub-cases:
//!
//!   A. Normal operation — Model produces risk scores with stable confidence
//!      (0.89).  `DriftMonitor` reports `Stable` after each batch.
//!
//!   B. Drift detected — After the stable baseline is established, confidence
//!      degrades through Warning (0.81) then fully Drifted (0.62).  The
//!      registry auto-revokes the model and the policy denies further scoring.

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

/// A mock sepsis risk scoring agent.  Returns a hardcoded score and confidence
/// to simulate model invocation without a real ML runtime.
///
/// `confidence` is set externally so sub-cases can vary it to trigger drift.
pub struct SepsisRiskAgent {
    pub confidence: f64,
}

impl Agent for SepsisRiskAgent {
    fn propose(&self, _state: &AgentState, input: &AgentInput) -> VeritasResult<AgentOutput> {
        let patient_id = input.payload["patient_id"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

        Ok(AgentOutput {
            kind: "sepsis-risk-result".to_string(),
            payload: json!({
                "model_id": "sepsis-risk-v2.1",
                "patient_id": patient_id,
                "risk_score": 0.74,
                "risk_level": "HIGH",
                "confidence": self.confidence,
                "recommendation": "Initiate sepsis protocol; notify attending physician immediately."
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
        vec![
            "model:approved".to_string(),
            "patient-vitals.read".to_string(),
        ]
    }

    fn describe_action(&self, _state: &AgentState, _input: &AgentInput) -> (String, String) {
        ("score-risk".to_string(), "sepsis-model".to_string())
    }

    fn is_terminal(&self, state: &AgentState) -> bool {
        state.phase == "complete"
    }
}

/// Variant agent that routes to `sepsis-model-revoked` resource, used to
/// demonstrate that the policy blocks a revoked model before propose() runs.
pub struct SepsisRiskAgentRevoked;

impl Agent for SepsisRiskAgentRevoked {
    fn propose(&self, _state: &AgentState, _input: &AgentInput) -> VeritasResult<AgentOutput> {
        // Never reached — the policy denies before the executor calls this.
        Ok(AgentOutput {
            kind: "sepsis-risk-result".to_string(),
            payload: json!({}),
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
        vec![
            "model:approved".to_string(),
            "patient-vitals.read".to_string(),
        ]
    }

    fn describe_action(&self, _state: &AgentState, _input: &AgentInput) -> (String, String) {
        // Routes to the deny rule for revoked models.
        ("score-risk".to_string(), "sepsis-model-revoked".to_string())
    }

    fn is_terminal(&self, state: &AgentState) -> bool {
        state.phase == "complete"
    }
}

// ── Output schema ─────────────────────────────────────────────────────────────

fn sepsis_risk_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "sepsis-risk-v1".to_string(),
        json_schema: json!({
            "type": "object",
            "required": ["model_id", "patient_id", "risk_score", "risk_level", "confidence", "recommendation"]
        }),
        rules: vec![
            VerificationRule {
                rule_id: "req-model-id".to_string(),
                description: "Output must identify the scoring model".to_string(),
                rule_type: VerificationRuleType::RequiredField {
                    field_path: "model_id".to_string(),
                },
            },
            VerificationRule {
                rule_id: "req-risk-score".to_string(),
                description: "Output must include a numeric risk score".to_string(),
                rule_type: VerificationRuleType::RequiredField {
                    field_path: "risk_score".to_string(),
                },
            },
            VerificationRule {
                rule_id: "req-recommendation".to_string(),
                description: "Output must carry a clinical recommendation".to_string(),
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

/// Build the approved sepsis risk model.
fn approved_sepsis_model() -> RegisteredModel {
    RegisteredModel {
        model_id: "sepsis-risk-v2.1".to_string(),
        modality: ModelModality::TabularToScore,
        version: "2.1.0".to_string(),
        provenance: ModelProvenance {
            vendor: "Acme Clinical AI".to_string(),
            training_data_hash: Some("sha256:b7e4d9...".to_string()),
            approval_status: ApprovalStatus::Approved,
            approved_by: Some("Chief Medical Officer".to_string()),
            approved_at: Some("2026-02-01T08:00:00Z".to_string()),
            regulatory_class: Some("FDA-510k".to_string()),
        },
    }
}

/// Run a single scoring step through the VERITAS executor, returning the
/// confidence value from the output payload (for drift recording).
fn run_scoring_step(
    registry: &ModelRegistry,
    confidence: f64,
    patient_id: &str,
) -> VeritasResult<Option<f64>> {
    let caps_from_registry = registry.capabilities_for("sepsis-risk-v2.1");
    let mut capabilities = CapabilitySet::default();
    for cap in &caps_from_registry {
        capabilities.grant(Capability::new(cap.as_str()));
    }
    capabilities.grant(Capability::new("patient-vitals.read"));

    let policy = TomlPolicyEngine::from_toml_str(MODEL_GOVERNANCE_POLICY)?;
    let exec_id = ExecutionId::new();
    let audit = Arc::new(InMemoryAuditWriter::new(exec_id.0.to_string()));
    let agent = SepsisRiskAgent { confidence };

    let state = AgentState {
        agent_id: AgentId("sepsis-risk-agent".to_string()),
        execution_id: exec_id.clone(),
        phase: "active".to_string(),
        context: serde_json::Value::Null,
        step: 0,
    };

    let input = AgentInput {
        kind: "sepsis-risk-request".to_string(),
        payload: json!({ "patient_id": patient_id }),
    };

    let executor = Executor::new(
        Box::new(policy),
        Box::new(ArcAudit(Arc::clone(&audit))),
        Box::new(SchemaVerifier::new()),
        sepsis_risk_schema(),
    );

    let result = executor.step(&agent, state, input, &capabilities)?;

    match result {
        StepResult::Complete { output, .. } | StepResult::Transitioned { output, .. } => {
            Ok(output.payload["confidence"].as_f64())
        }
        StepResult::Denied { .. } => Ok(None),
        StepResult::AwaitingApproval { .. } => Ok(None),
    }
}

// ── Scenario runner ───────────────────────────────────────────────────────────

/// Run Scenario 7: Sepsis Risk Model with Drift.
pub fn run_scenario() -> VeritasResult<()> {
    println!("=== Scenario 7: Sepsis Risk Model with Drift ===");
    println!();
    println!("  Model:    sepsis-risk-v2.1 (TabularToScore)");
    println!("  Monitor:  InMemoryDriftMonitor (window=5, warn=0.10, drift=0.20)");
    println!();

    run_sub_case_a()?;
    run_sub_case_b()?;

    println!("  Scenario 7 complete.");
    println!();

    Ok(())
}

/// Sub-case A: Stable operation — model scores patients, drift monitor stays Stable.
fn run_sub_case_a() -> VeritasResult<()> {
    println!("  ── Sub-case A: Normal operation (stable confidence) ──");
    println!();

    let mut registry = ModelRegistry::new();
    registry.register(approved_sepsis_model())?;

    let monitor = InMemoryDriftMonitor::new(DriftConfig {
        warning_threshold: 0.10,
        drift_threshold: 0.20,
        window_size: 5,
    });

    println!("  Running 5 scoring invocations at confidence 0.89...");
    println!();

    for i in 1..=5 {
        let patient = format!("patient-{}", 600 + i);
        match run_scoring_step(&registry, 0.89, &patient)? {
            Some(conf) => {
                println!("    Invocation {i}: patient={patient}, confidence={conf:.2}  → PASS");
                // Record result in the drift monitor for governance tracking.
                monitor.record("sepsis-risk-v2.1", &json!({ "confidence": conf }))?;
            }
            None => {
                println!("    Invocation {i}: BLOCKED (unexpected)");
            }
        }
    }

    println!();

    let status = registry.check_and_update("sepsis-risk-v2.1", &monitor)?;
    println!("  Drift check after 5 stable invocations: {status:?}");
    println!(
        "  Model still approved: {}",
        registry.is_approved("sepsis-risk-v2.1")
    );
    println!();
    println!("  Sub-case A complete: model operating normally, no drift.");
    println!();

    Ok(())
}

/// Sub-case B: Drift lifecycle — Warning then Drifted, ending in auto-revocation.
fn run_sub_case_b() -> VeritasResult<()> {
    println!("  ── Sub-case B: Drift lifecycle (Warning → Drifted → Revoked) ──");
    println!();

    let mut registry = ModelRegistry::new();
    registry.register(approved_sepsis_model())?;

    let monitor = InMemoryDriftMonitor::new(DriftConfig {
        warning_threshold: 0.10,
        drift_threshold: 0.20,
        window_size: 5,
    });

    // Phase 1: Establish stable baseline at 0.89.
    println!("  Phase 1: Establish baseline (5 invocations at confidence 0.89)");
    for _ in 0..5 {
        monitor.record("sepsis-risk-v2.1", &json!({ "confidence": 0.89 }))?;
    }
    let status_baseline = registry.check_and_update("sepsis-risk-v2.1", &monitor)?;
    println!("    Drift status after baseline: {status_baseline:?}");
    println!();

    // Phase 2: Slight degradation — triggers Warning (drop ~0.11).
    println!("  Phase 2: Confidence degrades to 0.78 (drop ≈ 0.11 → Warning threshold)");
    for _ in 0..5 {
        monitor.record("sepsis-risk-v2.1", &json!({ "confidence": 0.78 }))?;
    }
    let status_warning = registry.check_and_update("sepsis-risk-v2.1", &monitor)?;
    println!("    Drift status:    {status_warning:?}");
    println!(
        "    Model approved:  {} (Warning does not auto-revoke)",
        registry.is_approved("sepsis-risk-v2.1")
    );
    println!();

    // Phase 3: Severe degradation — triggers Drifted (drop = 0.27).
    println!("  Phase 3: Confidence collapses to 0.62 (drop = 0.27 → Drift threshold)");
    for _ in 0..5 {
        monitor.record("sepsis-risk-v2.1", &json!({ "confidence": 0.62 }))?;
    }
    let status_drifted = registry.check_and_update("sepsis-risk-v2.1", &monitor)?;
    println!("    Drift status:   {status_drifted:?}");
    println!(
        "    Model approved: {} (auto-revoked by drift)",
        registry.is_approved("sepsis-risk-v2.1")
    );

    if let Some(model) = registry.get("sepsis-risk-v2.1") {
        match &model.provenance.approval_status {
            ApprovalStatus::Revoked { reason } => {
                println!("    Revocation:     \"{reason}\"");
            }
            other => println!("    Status: {other:?}"),
        }
    }

    println!();

    // Phase 4: Demonstrate that the policy now denies the revoked model.
    println!("  Phase 4: Attempt to score with the revoked model — policy must deny");
    {
        let policy = TomlPolicyEngine::from_toml_str(MODEL_GOVERNANCE_POLICY)?;
        let exec_id = ExecutionId::new();
        let audit = Arc::new(InMemoryAuditWriter::new(exec_id.0.to_string()));

        // Build capabilities from the now-revoked registry entry — model:approved is absent.
        let caps_from_registry = registry.capabilities_for("sepsis-risk-v2.1");
        let mut capabilities = CapabilitySet::default();
        for cap in &caps_from_registry {
            capabilities.grant(Capability::new(cap.as_str()));
        }
        capabilities.grant(Capability::new("patient-vitals.read"));

        // Use the revoked-path agent so the resource routes to the deny rule.
        let agent = SepsisRiskAgentRevoked;

        let state = AgentState {
            agent_id: AgentId("sepsis-risk-agent".to_string()),
            execution_id: exec_id.clone(),
            phase: "active".to_string(),
            context: serde_json::Value::Null,
            step: 0,
        };

        let input = AgentInput {
            kind: "sepsis-risk-request".to_string(),
            payload: json!({ "patient_id": "patient-701" }),
        };

        let executor = Executor::new(
            Box::new(policy),
            Box::new(ArcAudit(Arc::clone(&audit))),
            Box::new(SchemaVerifier::new()),
            sepsis_risk_schema(),
        );

        let result = executor.step(&agent, state, input, &capabilities)?;

        match &result {
            StepResult::Denied { reason, .. } => {
                println!("    Action:          score-risk | Resource: sepsis-model-revoked");
                println!("    Policy verdict:  Deny");
                println!("    Reason:          {reason}");
                println!("    Agent propose(): NOT called (blocked at policy gate)");
            }
            other => {
                println!("    UNEXPECTED result: {other:?}");
            }
        }

        let log = audit.export_log();
        println!(
            "    Audit chain:     {} ({} event(s))",
            if audit.verify_integrity() {
                "VERIFIED"
            } else {
                "FAILED"
            },
            log.events.len()
        );
    }

    println!();
    println!("  Sub-case B complete: drift detected, model revoked, further scoring blocked.");
    println!();

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use veritas_contracts::{
        policy::{PolicyContext, PolicyVerdict},
        DriftStatus,
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

    /// Approved sepsis model is allowed when it holds model:approved + patient-vitals.read.
    #[test]
    fn test_approved_sepsis_model_policy_allow() {
        let policy = TomlPolicyEngine::from_toml_str(MODEL_GOVERNANCE_POLICY).unwrap();
        let ctx = make_policy_ctx(
            "score-risk",
            "sepsis-model",
            &["model:approved", "patient-vitals.read"],
        );
        assert_eq!(policy.evaluate(&ctx).unwrap(), PolicyVerdict::Allow);
    }

    /// Revoked sepsis model routes to deny rule and is blocked.
    #[test]
    fn test_revoked_sepsis_model_policy_deny() {
        let policy = TomlPolicyEngine::from_toml_str(MODEL_GOVERNANCE_POLICY).unwrap();
        let ctx = make_policy_ctx(
            "score-risk",
            "sepsis-model-revoked",
            &["patient-vitals.read"],
        );
        match policy.evaluate(&ctx).unwrap() {
            PolicyVerdict::Deny { reason } => {
                assert!(
                    reason.contains("revoked"),
                    "deny reason should mention revocation: {reason}"
                );
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    /// DriftMonitor transitions: Stable → Warning → Drifted.
    #[test]
    fn test_drift_monitor_transitions_stable_warning_drifted() {
        let mut registry = ModelRegistry::new();
        registry.register(approved_sepsis_model()).unwrap();

        let monitor = InMemoryDriftMonitor::new(DriftConfig {
            warning_threshold: 0.10,
            drift_threshold: 0.20,
            window_size: 5,
        });

        // Baseline at 0.89.
        for _ in 0..5 {
            monitor
                .record("sepsis-risk-v2.1", &json!({ "confidence": 0.89 }))
                .unwrap();
        }
        assert_eq!(
            monitor.check_drift("sepsis-risk-v2.1"),
            DriftStatus::Stable,
            "must be Stable after baseline"
        );

        // Warning phase: drop to 0.78 (drop ≈ 0.11 > 0.10 warning threshold).
        for _ in 0..5 {
            monitor
                .record("sepsis-risk-v2.1", &json!({ "confidence": 0.78 }))
                .unwrap();
        }
        assert!(
            matches!(
                monitor.check_drift("sepsis-risk-v2.1"),
                DriftStatus::Warning { .. }
            ),
            "expected Warning after mild degradation"
        );
        // Warning must NOT auto-revoke.
        assert!(
            registry.is_approved("sepsis-risk-v2.1"),
            "Warning must not auto-revoke"
        );

        // Drift phase: drop to 0.62 (drop = 0.27 > 0.20 drift threshold).
        for _ in 0..5 {
            monitor
                .record("sepsis-risk-v2.1", &json!({ "confidence": 0.62 }))
                .unwrap();
        }
        let status = registry
            .check_and_update("sepsis-risk-v2.1", &monitor)
            .unwrap();
        assert!(
            matches!(status, DriftStatus::Drifted { .. }),
            "expected Drifted after severe degradation"
        );
        assert!(
            !registry.is_approved("sepsis-risk-v2.1"),
            "drifted model must be auto-revoked"
        );
    }

    /// Stable operation: 10 invocations at constant confidence, model remains approved.
    #[test]
    fn test_stable_operation_model_remains_approved() {
        let mut registry = ModelRegistry::new();
        registry.register(approved_sepsis_model()).unwrap();

        let monitor = InMemoryDriftMonitor::new(DriftConfig {
            warning_threshold: 0.10,
            drift_threshold: 0.20,
            window_size: 5,
        });

        for _ in 0..10 {
            monitor
                .record("sepsis-risk-v2.1", &json!({ "confidence": 0.89 }))
                .unwrap();
        }

        let status = registry
            .check_and_update("sepsis-risk-v2.1", &monitor)
            .unwrap();
        assert_eq!(status, DriftStatus::Stable);
        assert!(registry.is_approved("sepsis-risk-v2.1"));
    }

    /// Full sub-case A runs without error.
    #[test]
    fn test_sub_case_a_runs() {
        run_sub_case_a().expect("sub-case A should succeed");
    }

    /// Full sub-case B runs without error.
    #[test]
    fn test_sub_case_b_runs() {
        run_sub_case_b().expect("sub-case B should succeed");
    }
}
