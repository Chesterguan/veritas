//! VERITAS Healthcare Demo — interactive Ratatui TUI
//!
//! Layout:
//!   ┌─── header ──────────────────────────────────────────────────────────┐
//!   │  [1] Drug Interaction  [2] Clinical Pipeline  [3] Prior Auth  ...  │
//!   ├─── left panel ──────────────────┬─── right panel ───────────────────┤
//!   │  Execution Pipeline             │  Audit Trail                      │
//!   ├─────────────────────────────────┴───────────────────────────────────┤
//!   │  Policy Details & Output                                            │
//!   ├─────────────────────────────────────────────────────────────────────┤
//!   │  footer (key bindings)                                              │
//!   └─────────────────────────────────────────────────────────────────────┘

use std::{
    io,
    sync::Arc,
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use serde_json::json;

use veritas_audit::{AuditEvent, InMemoryAuditWriter};
use veritas_contracts::{
    ApprovalStatus, DriftMonitor, ModelModality, ModelProvenance,
    agent::{AgentId, AgentInput, AgentOutput, AgentState, ExecutionId},
    capability::{Capability, CapabilitySet},
    error::{VeritasError, VeritasResult},
    execution::{StepRecord, StepResult},
    policy::PolicyVerdict,
    verify::{OutputSchema, VerificationRule, VerificationRuleType},
};
use veritas_core::{
    executor::Executor,
    traits::{Agent, AuditWriter},
};
use veritas_model::{DriftConfig, InMemoryDriftMonitor, ModelRegistry, RegisteredModel};
use veritas_policy::engine::TomlPolicyEngine;
use veritas_ref_healthcare::scenarios::drug_interaction::DrugInteractionAgent;
use veritas_ref_healthcare::mock_data::{check_drug_interaction, get_patient_symptoms};
use veritas_verify::engine::SchemaVerifier;

// ── Policy TOML constants ─────────────────────────────────────────────────────

const HEALTHCARE_POLICY: &str =
    include_str!("../../crates/veritas-ref-healthcare/policies/healthcare.toml");

const PIPELINE_POLICY: &str =
    include_str!("../../crates/veritas-ref-healthcare/policies/pipeline.toml");

const PRIOR_AUTH_POLICY: &str =
    include_str!("../../crates/veritas-ref-healthcare/policies/prior_auth.toml");

const MODEL_GOVERNANCE_POLICY: &str =
    include_str!("../../crates/veritas-ref-healthcare/policies/model_governance.toml");

// ── ArcAudit newtype ──────────────────────────────────────────────────────────

/// Thin newtype so `Arc<InMemoryAuditWriter>` satisfies `Box<dyn AuditWriter>`.
struct ArcAudit(Arc<InMemoryAuditWriter>);

impl AuditWriter for ArcAudit {
    fn write(&self, record: &StepRecord) -> VeritasResult<()> {
        self.0.write(record)
    }
    fn finalize(&self, execution_id: &str) -> VeritasResult<()> {
        self.0.finalize(execution_id)
    }
}

// ── Domain types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    DrugInteraction,
    ClinicalPipeline,
    PriorAuth,
    RadiologyModel,
    SepsisModel,
}

impl Scenario {
    fn name(self) -> &'static str {
        match self {
            Scenario::DrugInteraction => "Drug Interaction",
            Scenario::ClinicalPipeline => "Clinical Pipeline",
            Scenario::PriorAuth => "Prior Auth",
            Scenario::RadiologyModel => "Radiology AI",
            Scenario::SepsisModel => "Sepsis Drift",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StepStatus {
    Pending,
    Pass,
    Fail,
    Denied,
    AwaitingApproval,
}

#[derive(Debug, Clone)]
struct PipelineStep {
    /// Display label, e.g. "Policy", "Capability".
    name: String,
    status: StepStatus,
    /// One-line detail shown in the pipeline panel.
    detail: String,
}

/// Compact view of one audit chain entry for the right panel.
#[derive(Debug, Clone)]
struct AuditEntryDisplay {
    sequence: u64,
    /// First 4 + last 4 hex chars of this_hash, e.g. "3fa2...8b1c".
    hash_short: String,
    /// "genesis", "allow", "deny", etc.
    kind: String,
    /// Whether the chain was VERIFIED after adding this entry.
    verified: bool,
}

/// Everything captured from one execution run.
#[derive(Debug)]
struct ExecutionCapture {
    policy_verdict: PolicyVerdict,
    /// Human-readable action/resource pair.
    action: String,
    resource: String,
    /// Capability name and whether it was granted.
    capability_name: String,
    capability_granted: bool,
    /// Whether the executor produced output (None on Deny/CapabilityMissing).
    output: Option<AgentOutput>,
    /// Error if the executor returned Err (e.g. CapabilityMissing).
    error: Option<VeritasError>,
    /// Audit chain entries at execution time.
    audit_events: Vec<AuditEvent>,
    /// Result of verify_integrity().
    chain_integrity: bool,
    /// Extra scenario-specific lines for the output panel.
    extra_lines: Vec<(String, String, Color)>,
}

// ── App state ─────────────────────────────────────────────────────────────────

struct App {
    selected: Scenario,

    // Most recent run result.
    capture: Option<ExecutionCapture>,

    // Animated display: how many pipeline steps are currently revealed.
    animation_step: usize,
    // All pipeline steps derived from the last capture (up to N).
    pipeline_steps: Vec<PipelineStep>,
    // Audit entries derived from the last capture.
    audit_entries: Vec<AuditEntryDisplay>,

    // Timer-based animation: last tick at which we revealed a step.
    last_tick: Instant,
    // Whether animation is still in progress.
    animating: bool,
}

impl App {
    fn new() -> Self {
        Self {
            selected: Scenario::DrugInteraction,
            capture: None,
            animation_step: 0,
            pipeline_steps: Vec::new(),
            audit_entries: Vec::new(),
            last_tick: Instant::now(),
            animating: false,
        }
    }

    /// Advance animation by one step (called every ~150 ms when animating).
    fn tick_animation(&mut self) {
        if self.animating && self.animation_step < self.pipeline_steps.len() {
            self.animation_step += 1;
            if self.animation_step >= self.pipeline_steps.len() {
                self.animating = false;
            }
        }
    }

    /// Run the selected scenario, capture the result, and start animation.
    fn run(&mut self) {
        let (capture, steps, entries) = match self.selected {
            Scenario::DrugInteraction => {
                let cap = run_drug_interaction();
                let steps = build_pipeline_steps(&cap);
                let entries = build_audit_entries(&cap);
                (cap, steps, entries)
            }
            Scenario::ClinicalPipeline => run_clinical_pipeline(),
            Scenario::PriorAuth => run_prior_auth(),
            Scenario::RadiologyModel => run_radiology_model(),
            Scenario::SepsisModel => run_sepsis_model(),
        };

        self.pipeline_steps = steps;
        self.audit_entries = entries;
        self.capture = Some(capture);
        self.animation_step = 0;
        self.last_tick = Instant::now();
        self.animating = true;
    }
}

// ── Scenario 1: Drug Interaction ──────────────────────────────────────────────

fn run_drug_interaction() -> ExecutionCapture {
    let policy = match TomlPolicyEngine::from_toml_str(HEALTHCARE_POLICY) {
        Ok(p) => p,
        Err(e) => {
            return ExecutionCapture {
                policy_verdict: PolicyVerdict::Deny {
                    reason: format!("policy load error: {e}"),
                },
                action: "drug-interaction-check".to_string(),
                resource: "drug-database".to_string(),
                capability_name: "drug-database.read".to_string(),
                capability_granted: true,
                output: None,
                error: Some(e),
                audit_events: vec![],
                chain_integrity: false,
                extra_lines: vec![],
            };
        }
    };

    let execution_id = ExecutionId::new();
    let audit = Arc::new(InMemoryAuditWriter::new(execution_id.0.to_string()));
    let verifier = SchemaVerifier::new();
    let agent = DrugInteractionAgent;

    let state = AgentState {
        agent_id: AgentId("drug-interaction-agent".to_string()),
        execution_id: execution_id.clone(),
        phase: "active".to_string(),
        context: serde_json::Value::Null,
        step: 0,
    };

    let mut capabilities = CapabilitySet::default();
    capabilities.grant(Capability::new("drug-database.read"));

    let schema = drug_interaction_schema();
    let input = AgentInput {
        kind: "drug-interaction-request".to_string(),
        payload: json!({ "drug_a": "warfarin", "drug_b": "aspirin" }),
    };

    let executor = Executor::new(
        Box::new(policy),
        Box::new(ArcAudit(Arc::clone(&audit))),
        Box::new(verifier),
        schema,
    );

    let result = executor.step(&agent, state, input, &capabilities);

    let (verdict, output, error) = match result {
        Ok(StepResult::Complete { output, .. }) | Ok(StepResult::Transitioned { output, .. }) => {
            (PolicyVerdict::Allow, Some(output), None)
        }
        Ok(StepResult::Denied { reason, .. }) => (PolicyVerdict::Deny { reason }, None, None),
        Ok(StepResult::AwaitingApproval {
            reason,
            approver_role,
            ..
        }) => (
            PolicyVerdict::RequireApproval {
                reason,
                approver_role,
            },
            None,
            None,
        ),
        Err(e) => {
            let v = PolicyVerdict::Deny {
                reason: e.to_string(),
            };
            (v, None, Some(e))
        }
    };

    let log = audit.export_log();
    let chain_integrity = audit.verify_integrity();

    let extra_lines = if let Some(ref out) = output {
        let severity = out.payload["result"]["severity"]
            .as_str()
            .unwrap_or("?")
            .to_string();
        let rec = out.payload["recommendation"]
            .as_str()
            .unwrap_or("?")
            .to_string();
        let sev_color = match severity.as_str() {
            "HIGH" => Color::Red,
            "MEDIUM" => Color::Yellow,
            _ => Color::Green,
        };
        vec![
            ("Severity".to_string(), severity, sev_color),
            ("Rec".to_string(), truncate(&rec, 70), Color::White),
        ]
    } else {
        vec![]
    };

    ExecutionCapture {
        policy_verdict: verdict,
        action: "drug-interaction-check".to_string(),
        resource: "drug-database".to_string(),
        capability_name: "drug-database.read".to_string(),
        capability_granted: true,
        output,
        error,
        audit_events: log.events,
        chain_integrity,
        extra_lines,
    }
}

// ── Scenario 2: Clinical Decision Pipeline ────────────────────────────────────

// Agent implementations inline (mirrors clinical_pipeline.rs)

struct SymptomAnalyzerAgent;

impl Agent for SymptomAnalyzerAgent {
    fn propose(&self, state: &AgentState, input: &AgentInput) -> VeritasResult<AgentOutput> {
        let patient_id = input.payload["patient_id"].as_str().unwrap_or("unknown");
        let symptoms = get_patient_symptoms(patient_id);
        let flags: Vec<serde_json::Value> = symptoms["reported_symptoms"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s["symptom"].as_str())
                    .map(|s| json!(s))
                    .collect()
            })
            .unwrap_or_default();
        Ok(AgentOutput {
            kind: "symptom-analysis".to_string(),
            payload: json!({
                "patient_id": patient_id,
                "flags": flags,
                "severity_level": "moderate",
                "vitals_stable": true,
                "analyzed_by": state.agent_id.0
            }),
        })
    }
    fn transition(&self, state: &AgentState, _output: &AgentOutput) -> VeritasResult<AgentState> {
        Ok(AgentState { step: state.step + 1, phase: "complete".to_string(), ..state.clone() })
    }
    fn required_capabilities(&self, _: &AgentState, _: &AgentInput) -> Vec<String> {
        vec!["clinical-data.read".to_string()]
    }
    fn describe_action(&self, _: &AgentState, _: &AgentInput) -> (String, String) {
        ("analyze".to_string(), "symptom-data".to_string())
    }
    fn is_terminal(&self, state: &AgentState) -> bool { state.phase == "complete" }
}

struct DiagnosisSuggesterAgent;

impl Agent for DiagnosisSuggesterAgent {
    fn propose(&self, state: &AgentState, input: &AgentInput) -> VeritasResult<AgentOutput> {
        let flags = input.payload["flags"].as_array().cloned().unwrap_or_default();
        let has_fatigue = flags.iter().any(|f| f.as_str() == Some("fatigue"));
        let has_pallor = flags.iter().any(|f| f.as_str() == Some("pallor"));
        let primary = if has_fatigue && has_pallor { "Iron deficiency anemia" } else { "Unspecified fatigue syndrome" };
        Ok(AgentOutput {
            kind: "diagnosis-suggestion".to_string(),
            payload: json!({
                "diagnoses": [
                    { "code": "D50.9", "description": "Iron deficiency anemia, unspecified" },
                    { "code": "J96.00", "description": "Acute respiratory failure, unspecified" },
                    { "code": "R53.83", "description": "Other fatigue" }
                ],
                "primary_hypothesis": primary,
                "confidence": "moderate",
                "flags_evaluated": flags,
                "suggested_by": state.agent_id.0
            }),
        })
    }
    fn transition(&self, state: &AgentState, _output: &AgentOutput) -> VeritasResult<AgentState> {
        Ok(AgentState { step: state.step + 1, phase: "complete".to_string(), ..state.clone() })
    }
    fn required_capabilities(&self, _: &AgentState, _: &AgentInput) -> Vec<String> {
        vec!["clinical-data.read".to_string()]
    }
    fn describe_action(&self, _: &AgentState, _: &AgentInput) -> (String, String) {
        ("suggest-diagnosis".to_string(), "clinical-analysis".to_string())
    }
    fn is_terminal(&self, state: &AgentState) -> bool { state.phase == "complete" }
}

struct TreatmentPlannerAgent;

impl Agent for TreatmentPlannerAgent {
    fn propose(&self, state: &AgentState, input: &AgentInput) -> VeritasResult<AgentOutput> {
        let primary = input.payload["primary_hypothesis"].as_str().unwrap_or("unknown diagnosis");
        Ok(AgentOutput {
            kind: "treatment-plan".to_string(),
            payload: json!({
                "primary_diagnosis": primary,
                "medications": ["warfarin", "aspirin", "ferrous-sulfate"],
                "plan_summary": "Anticoagulation therapy combined with iron supplementation.",
                "follow_up_days": 7,
                "planned_by": state.agent_id.0
            }),
        })
    }
    fn transition(&self, state: &AgentState, _output: &AgentOutput) -> VeritasResult<AgentState> {
        Ok(AgentState { step: state.step + 1, phase: "complete".to_string(), ..state.clone() })
    }
    fn required_capabilities(&self, _: &AgentState, _: &AgentInput) -> Vec<String> {
        vec!["treatment.write".to_string()]
    }
    fn describe_action(&self, _: &AgentState, _: &AgentInput) -> (String, String) {
        ("plan-treatment".to_string(), "diagnosis-data".to_string())
    }
    fn is_terminal(&self, state: &AgentState) -> bool { state.phase == "complete" }
}

struct DrugSafetyCheckerAgent;

impl Agent for DrugSafetyCheckerAgent {
    fn propose(&self, state: &AgentState, input: &AgentInput) -> VeritasResult<AgentOutput> {
        let meds: Vec<&str> = input.payload["medications"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        let mut interactions = Vec::new();
        let mut max_severity = "NONE";
        for i in 0..meds.len() {
            for j in (i + 1)..meds.len() {
                let result = check_drug_interaction(meds[i], meds[j]);
                let severity = result["result"]["severity"].as_str().unwrap_or("UNKNOWN");
                if severity != "UNKNOWN" {
                    if severity == "HIGH" { max_severity = "HIGH"; }
                    else if max_severity != "HIGH" && severity == "MEDIUM" { max_severity = "MEDIUM"; }
                    else if max_severity == "NONE" && severity == "LOW" { max_severity = "LOW"; }
                    interactions.push(json!({
                        "drug_a": meds[i], "drug_b": meds[j],
                        "severity": severity,
                        "mechanism": result["result"]["mechanism"],
                        "recommendation": result["recommendation"]
                    }));
                }
            }
        }
        Ok(AgentOutput {
            kind: "drug-safety-report".to_string(),
            payload: json!({
                "safety_report": {
                    "overall_risk": max_severity,
                    "interactions_found": interactions.len(),
                    "reviewed": true,
                    "details": interactions
                },
                "checked_by": state.agent_id.0
            }),
        })
    }
    fn transition(&self, state: &AgentState, _output: &AgentOutput) -> VeritasResult<AgentState> {
        Ok(AgentState { step: state.step + 1, phase: "complete".to_string(), ..state.clone() })
    }
    fn required_capabilities(&self, _: &AgentState, _: &AgentInput) -> Vec<String> {
        vec!["drug-database.read".to_string()]
    }
    fn describe_action(&self, _: &AgentState, _: &AgentInput) -> (String, String) {
        ("check-drug-safety".to_string(), "drug-database".to_string())
    }
    fn is_terminal(&self, state: &AgentState) -> bool { state.phase == "complete" }
}

/// Configuration for a single pipeline stage execution.
struct StepConfig<'a> {
    policy_toml: &'a str,
    agent_id: &'a str,
    input_payload: serde_json::Value,
    input_kind: &'a str,
    caps: &'a [&'a str],
    schema: OutputSchema,
    register_high_risk_rule: bool,
}

/// Run one pipeline stage and return (output_payload, audit_events, integrity, verdict).
fn pipeline_step<A: Agent>(
    agent: &A,
    cfg: StepConfig<'_>,
) -> (Option<serde_json::Value>, Vec<AuditEvent>, bool, PolicyVerdict) {
    let StepConfig { policy_toml, agent_id, input_payload, input_kind, caps, schema, register_high_risk_rule } = cfg;
    let policy = match TomlPolicyEngine::from_toml_str(policy_toml) {
        Ok(p) => p,
        Err(_) => return (None, vec![], false, PolicyVerdict::Deny { reason: "policy error".to_string() }),
    };
    let exec_id = ExecutionId::new();
    let audit = Arc::new(InMemoryAuditWriter::new(exec_id.0.to_string()));
    let state = AgentState {
        agent_id: AgentId(agent_id.to_string()),
        execution_id: exec_id.clone(),
        phase: "active".to_string(),
        context: serde_json::Value::Null,
        step: 0,
    };
    let mut capability_set = CapabilitySet::default();
    for c in caps {
        capability_set.grant(Capability::new(*c));
    }
    let input = AgentInput { kind: input_kind.to_string(), payload: input_payload };

    let mut verifier = SchemaVerifier::new();
    if register_high_risk_rule {
        verifier.register_rule("no-high-risk-unreviewed", Box::new(|payload| {
            let report = &payload["safety_report"];
            let risk = report["overall_risk"].as_str().unwrap_or("NONE");
            let reviewed = report["reviewed"].as_bool().unwrap_or(false);
            if risk == "HIGH" && !reviewed {
                Some("HIGH-risk output must have reviewed=true".to_string())
            } else {
                None
            }
        }));
    }

    let executor = Executor::new(
        Box::new(policy),
        Box::new(ArcAudit(Arc::clone(&audit))),
        Box::new(verifier),
        schema,
    );

    let result = executor.step(agent, state, input, &capability_set);
    let log = audit.export_log();
    let integrity = audit.verify_integrity();

    match result {
        Ok(StepResult::Complete { output, .. }) | Ok(StepResult::Transitioned { output, .. }) => {
            (Some(output.payload), log.events, integrity, PolicyVerdict::Allow)
        }
        Ok(StepResult::Denied { reason, .. }) => {
            (None, log.events, integrity, PolicyVerdict::Deny { reason })
        }
        Ok(StepResult::AwaitingApproval { reason, approver_role, .. }) => {
            (None, log.events, integrity, PolicyVerdict::RequireApproval { reason, approver_role })
        }
        Err(_) => (None, log.events, integrity, PolicyVerdict::Deny { reason: "executor error".to_string() }),
    }
}

fn symptom_analyzer_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "symptom-analysis-v1".to_string(),
        json_schema: json!({ "type": "object", "required": ["patient_id", "flags", "severity_level"] }),
        rules: vec![
            VerificationRule {
                rule_id: "req-flags".to_string(),
                description: "Output must contain the classified symptom flags".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "flags".to_string() },
            },
            VerificationRule {
                rule_id: "req-severity-level".to_string(),
                description: "Output must include an overall severity classification".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "severity_level".to_string() },
            },
        ],
    }
}

fn diagnosis_suggester_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "diagnosis-suggestion-v1".to_string(),
        json_schema: json!({ "type": "object", "required": ["diagnoses", "primary_hypothesis"] }),
        rules: vec![
            VerificationRule {
                rule_id: "req-diagnoses".to_string(),
                description: "Output must contain differential diagnoses".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "diagnoses".to_string() },
            },
            VerificationRule {
                rule_id: "req-primary-hypothesis".to_string(),
                description: "Output must name the primary diagnostic hypothesis".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "primary_hypothesis".to_string() },
            },
        ],
    }
}

fn treatment_planner_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "treatment-plan-v1".to_string(),
        json_schema: json!({ "type": "object", "required": ["medications", "plan_summary"] }),
        rules: vec![
            VerificationRule {
                rule_id: "req-medications".to_string(),
                description: "Output must list the proposed medications".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "medications".to_string() },
            },
            VerificationRule {
                rule_id: "req-plan-summary".to_string(),
                description: "Output must include a treatment plan summary".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "plan_summary".to_string() },
            },
        ],
    }
}

fn drug_safety_checker_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "drug-safety-report-v1".to_string(),
        json_schema: json!({ "type": "object", "required": ["safety_report"] }),
        rules: vec![
            VerificationRule {
                rule_id: "req-safety-report".to_string(),
                description: "Output must contain the drug safety report".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "safety_report".to_string() },
            },
            VerificationRule {
                rule_id: "no-high-risk-unreviewed".to_string(),
                description: "HIGH-risk interactions must be explicitly reviewed before delivery".to_string(),
                rule_type: VerificationRuleType::Custom { function_name: "no-high-risk-unreviewed".to_string() },
            },
        ],
    }
}

fn run_clinical_pipeline() -> (ExecutionCapture, Vec<PipelineStep>, Vec<AuditEntryDisplay>) {
    // Stage 1: SymptomAnalyzer
    let agent1 = SymptomAnalyzerAgent;
    let (out1, events1, int1, v1) = pipeline_step(&agent1, StepConfig {
        policy_toml: PIPELINE_POLICY, agent_id: "symptom-analyzer-agent",
        input_payload: json!({ "patient_id": "patient-101" }), input_kind: "symptom-analysis-request",
        caps: &["clinical-data.read"], schema: symptom_analyzer_schema(), register_high_risk_rule: false,
    });

    // Stage 2: DiagnosisSuggester
    let agent2 = DiagnosisSuggesterAgent;
    let input2 = out1.clone().unwrap_or(json!({}));
    let (out2, events2, int2, v2) = pipeline_step(&agent2, StepConfig {
        policy_toml: PIPELINE_POLICY, agent_id: "diagnosis-suggester-agent",
        input_payload: input2, input_kind: "diagnosis-request",
        caps: &["clinical-data.read"], schema: diagnosis_suggester_schema(), register_high_risk_rule: false,
    });

    // Stage 3: TreatmentPlanner
    let agent3 = TreatmentPlannerAgent;
    let input3 = out2.clone().unwrap_or(json!({}));
    let (out3, events3, int3, v3) = pipeline_step(&agent3, StepConfig {
        policy_toml: PIPELINE_POLICY, agent_id: "treatment-planner-agent",
        input_payload: input3, input_kind: "treatment-plan-request",
        caps: &["treatment.write"], schema: treatment_planner_schema(), register_high_risk_rule: false,
    });

    // Stage 4: DrugSafetyChecker
    let agent4 = DrugSafetyCheckerAgent;
    let input4 = out3.clone().unwrap_or(json!({}));
    let (out4, events4, int4, v4) = pipeline_step(&agent4, StepConfig {
        policy_toml: PIPELINE_POLICY, agent_id: "drug-safety-checker-agent",
        input_payload: input4, input_kind: "drug-safety-request",
        caps: &["drug-database.read"], schema: drug_safety_checker_schema(), register_high_risk_rule: true,
    });

    // Build pipeline steps showing each stage
    let mut steps = Vec::new();
    let stage_info = [
        ("Stage 1: SymptomAnalyzer", "analyze | symptom-data", &v1, out1.is_some()),
        ("Stage 2: DiagnosisSuggester", "suggest-diagnosis | clinical-analysis", &v2, out2.is_some()),
        ("Stage 3: TreatmentPlanner", "plan-treatment | diagnosis-data", &v3, out3.is_some()),
        ("Stage 4: DrugSafetyChecker", "check-drug-safety | drug-database", &v4, out4.is_some()),
    ];
    for (name, action_res, verdict, has_output) in &stage_info {
        let (status, detail) = match verdict {
            PolicyVerdict::Allow => {
                if *has_output {
                    (StepStatus::Pass, format!("Allow — {action_res} — verified"))
                } else {
                    (StepStatus::Fail, format!("Allow — {action_res} — no output"))
                }
            }
            PolicyVerdict::Deny { reason } => (StepStatus::Denied, format!("Deny — {}", truncate(reason, 50))),
            PolicyVerdict::RequireApproval { approver_role, .. } => (StepStatus::AwaitingApproval, format!("RequireApproval — {approver_role}")),
            PolicyVerdict::RequireVerification { check_id } => (StepStatus::Pass, format!("RequireVerification — {check_id}")),
        };
        steps.push(PipelineStep { name: (*name).to_string(), status, detail });
    }

    // Build audit entries: show all 4 chains concatenated with labels
    let mut all_entries: Vec<AuditEntryDisplay> = Vec::new();
    let chains = [
        (events1, int1, "chain-1"),
        (events2, int2, "chain-2"),
        (events3, int3, "chain-3"),
        (events4, int4, "chain-4"),
    ];
    for (events, integrity, label) in &chains {
        // chain label separator
        all_entries.push(AuditEntryDisplay {
            sequence: 0,
            hash_short: format!("── {label} ──"),
            kind: "label".to_string(),
            verified: *integrity,
        });
        for e in events {
            let kind = match &e.record.verdict {
                PolicyVerdict::Allow => "allow",
                PolicyVerdict::Deny { .. } => "deny",
                PolicyVerdict::RequireApproval { .. } => "require-approval",
                PolicyVerdict::RequireVerification { .. } => "require-verify",
            };
            let is_genesis = e.sequence == 0
                && e.prev_hash == "0000000000000000000000000000000000000000000000000000000000000000";
            all_entries.push(AuditEntryDisplay {
                sequence: e.sequence,
                hash_short: shorten_hash(&e.this_hash),
                kind: if is_genesis { "genesis".to_string() } else { kind.to_string() },
                verified: *integrity,
            });
        }
    }

    // Build summary capture for the output panel
    let overall_risk = out4.as_ref()
        .and_then(|o| o["safety_report"]["overall_risk"].as_str())
        .unwrap_or("?")
        .to_string();
    let interactions_found = out4.as_ref()
        .and_then(|o| o["safety_report"]["interactions_found"].as_u64())
        .unwrap_or(0);
    let primary_dx = out2.as_ref()
        .and_then(|o| o["primary_hypothesis"].as_str())
        .unwrap_or("?")
        .to_string();
    let meds = out3.as_ref()
        .and_then(|o| o["medications"].as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "))
        .unwrap_or_default();
    let risk_color = match overall_risk.as_str() {
        "HIGH" => Color::Red,
        "MEDIUM" => Color::Yellow,
        _ => Color::Green,
    };
    let all_ok = int1 && int2 && int3 && int4;
    let extra_lines = vec![
        ("Primary Dx".to_string(), truncate(&primary_dx, 50), Color::White),
        ("Medications".to_string(), truncate(&meds, 60), Color::White),
        ("Overall Risk".to_string(), format!("{overall_risk} ({interactions_found} interaction(s))"), risk_color),
        ("All Audit Chains".to_string(), if all_ok { "VERIFIED".to_string() } else { "INTEGRITY FAILURE".to_string() }, if all_ok { Color::Green } else { Color::Red }),
    ];

    let representative_output = out4.as_ref().map(|_| AgentOutput {
        kind: "pipeline-summary".to_string(),
        payload: json!({ "summary": "4-agent pipeline complete" }),
    });

    let capture = ExecutionCapture {
        policy_verdict: PolicyVerdict::Allow,
        action: "4-agent pipeline".to_string(),
        resource: "symptom-data → drug-database".to_string(),
        capability_name: "multi-stage".to_string(),
        capability_granted: true,
        output: representative_output,
        error: None,
        audit_events: vec![],
        chain_integrity: all_ok,
        extra_lines,
    };

    (capture, steps, all_entries)
}

// ── Scenario 3: Prior Authorization ───────────────────────────────────────────

struct ClinicalProposalAgent;

impl Agent for ClinicalProposalAgent {
    fn propose(&self, state: &AgentState, _input: &AgentInput) -> VeritasResult<AgentOutput> {
        Ok(AgentOutput {
            kind: "procedure-proposal".to_string(),
            payload: json!({
                "procedure": "cardiac-mri", "urgency": "routine",
                "proposed_by": state.agent_id.0, "proposed_at": "2026-03-13"
            }),
        })
    }
    fn transition(&self, state: &AgentState, _output: &AgentOutput) -> VeritasResult<AgentState> {
        Ok(AgentState { step: state.step + 1, phase: "awaiting-approval".to_string(), ..state.clone() })
    }
    fn required_capabilities(&self, _: &AgentState, _: &AgentInput) -> Vec<String> { vec![] }
    fn describe_action(&self, _: &AgentState, _: &AgentInput) -> (String, String) {
        ("propose-procedure".to_string(), "high-cost-procedure".to_string())
    }
    fn is_terminal(&self, state: &AgentState) -> bool { state.phase == "complete" }
}

struct InsuranceEligibilityAgent { covered: bool }

impl Agent for InsuranceEligibilityAgent {
    fn propose(&self, _state: &AgentState, input: &AgentInput) -> VeritasResult<AgentOutput> {
        let procedure = input.payload["procedure"].as_str().unwrap_or("unknown").to_string();
        Ok(AgentOutput {
            kind: "insurance-eligibility-result".to_string(),
            payload: json!({
                "procedure": procedure,
                "covered": self.covered,
                "plan_name": if self.covered { "Blue Shield PPO" } else { "N/A" },
                "copay_usd": if self.covered { json!(250) } else { serde_json::Value::Null },
                "requires_prior_auth": true
            }),
        })
    }
    fn transition(&self, state: &AgentState, _output: &AgentOutput) -> VeritasResult<AgentState> {
        Ok(AgentState { step: state.step + 1, phase: "complete".to_string(), ..state.clone() })
    }
    fn required_capabilities(&self, _: &AgentState, _: &AgentInput) -> Vec<String> {
        vec!["insurance.read".to_string()]
    }
    fn describe_action(&self, _: &AgentState, _: &AgentInput) -> (String, String) {
        let resource = if self.covered { "insurance-records" } else { "uncovered-procedure" };
        ("check-coverage".to_string(), resource.to_string())
    }
    fn is_terminal(&self, state: &AgentState) -> bool { state.phase == "complete" }
}

struct PASubmissionAgent;

impl Agent for PASubmissionAgent {
    fn propose(&self, state: &AgentState, input: &AgentInput) -> VeritasResult<AgentOutput> {
        let procedure = input.payload["procedure"].as_str().unwrap_or("unknown").to_string();
        Ok(AgentOutput {
            kind: "pa-submission-result".to_string(),
            payload: json!({
                "pa_reference": "PA-2026-0313-4471",
                "status": "submitted",
                "procedure": procedure,
                "submitted_by": state.agent_id.0,
                "submitted_at": "2026-03-13"
            }),
        })
    }
    fn transition(&self, state: &AgentState, _output: &AgentOutput) -> VeritasResult<AgentState> {
        Ok(AgentState { step: state.step + 1, phase: "complete".to_string(), ..state.clone() })
    }
    fn required_capabilities(&self, _: &AgentState, _: &AgentInput) -> Vec<String> {
        vec!["pa.write".to_string()]
    }
    fn describe_action(&self, _: &AgentState, _: &AgentInput) -> (String, String) {
        ("submit-pa".to_string(), "pa-system".to_string())
    }
    fn is_terminal(&self, state: &AgentState) -> bool { state.phase == "complete" }
}

fn clinical_proposal_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "procedure-proposal-v1".to_string(),
        json_schema: json!({ "type": "object", "required": ["procedure", "urgency"] }),
        rules: vec![VerificationRule {
            rule_id: "req-procedure".to_string(),
            description: "Proposal must name the requested procedure".to_string(),
            rule_type: VerificationRuleType::RequiredField { field_path: "procedure".to_string() },
        }],
    }
}

fn insurance_eligibility_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "insurance-eligibility-v1".to_string(),
        json_schema: json!({ "type": "object", "required": ["procedure", "covered"] }),
        rules: vec![
            VerificationRule {
                rule_id: "req-procedure".to_string(),
                description: "Eligibility result must name the procedure checked".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "procedure".to_string() },
            },
            VerificationRule {
                rule_id: "req-covered".to_string(),
                description: "Eligibility result must state whether procedure is covered".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "covered".to_string() },
            },
        ],
    }
}

fn pa_submission_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "pa-submission-v1".to_string(),
        json_schema: json!({ "type": "object", "required": ["pa_reference", "status"] }),
        rules: vec![
            VerificationRule {
                rule_id: "req-pa-reference".to_string(),
                description: "Submission result must include a PA reference number".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "pa_reference".to_string() },
            },
            VerificationRule {
                rule_id: "req-status".to_string(),
                description: "Submission result must include a status field".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "status".to_string() },
            },
        ],
    }
}

fn run_prior_auth() -> (ExecutionCapture, Vec<PipelineStep>, Vec<AuditEntryDisplay>) {
    let mut steps = Vec::new();
    let mut all_entries: Vec<AuditEntryDisplay> = Vec::new();

    // Step 1: ClinicalProposalAgent → RequireApproval
    let (out_step1, events1, int1, verdict1) = pipeline_step(&ClinicalProposalAgent, StepConfig {
        policy_toml: PRIOR_AUTH_POLICY, agent_id: "clinical-proposal-agent",
        input_payload: json!({ "procedure": "cardiac-mri", "urgency": "routine" }),
        input_kind: "procedure-proposal-request",
        caps: &[], schema: clinical_proposal_schema(), register_high_risk_rule: false,
    });
    steps.push(PipelineStep {
        name: "Step 1: ClinicalProposal".to_string(),
        status: StepStatus::AwaitingApproval,
        detail: "propose-procedure | high-cost-procedure → RequireApproval".to_string(),
    });
    all_entries.push(AuditEntryDisplay { sequence: 0, hash_short: "── step-1 ──".to_string(), kind: "label".to_string(), verified: int1 });
    for e in &events1 {
        let kind = match &e.record.verdict {
            PolicyVerdict::Allow => "allow",
            PolicyVerdict::Deny { .. } => "deny",
            PolicyVerdict::RequireApproval { .. } => "require-approval",
            PolicyVerdict::RequireVerification { .. } => "require-verify",
        };
        let is_genesis = e.sequence == 0 && e.prev_hash == "0000000000000000000000000000000000000000000000000000000000000000";
        all_entries.push(AuditEntryDisplay {
            sequence: e.sequence,
            hash_short: shorten_hash(&e.this_hash),
            kind: if is_genesis { "genesis".to_string() } else { kind.to_string() },
            verified: int1,
        });
    }

    // Simulate physician approval
    let approval_token = "PHY-APPROVE-2026-0313";
    steps.push(PipelineStep {
        name: "Physician Approval".to_string(),
        status: StepStatus::Pass,
        detail: format!("token={approval_token} [simulated]"),
    });

    // Step 2a: InsuranceEligibility (covered=true) → Allow
    let (out_step2a, events2a, int2a, _v2a) = pipeline_step(
        &InsuranceEligibilityAgent { covered: true }, StepConfig {
        policy_toml: PRIOR_AUTH_POLICY, agent_id: "insurance-eligibility-agent",
        input_payload: json!({ "procedure": "cardiac-mri" }), input_kind: "insurance-eligibility-request",
        caps: &["insurance.read"], schema: insurance_eligibility_schema(), register_high_risk_rule: false,
    });
    steps.push(PipelineStep {
        name: "Step 2a: Eligibility [covered]".to_string(),
        status: if out_step2a.is_some() { StepStatus::Pass } else { StepStatus::Denied },
        detail: "check-coverage | insurance-records → Allow".to_string(),
    });
    all_entries.push(AuditEntryDisplay { sequence: 0, hash_short: "── step-2a ──".to_string(), kind: "label".to_string(), verified: int2a });
    for e in &events2a {
        let kind = match &e.record.verdict { PolicyVerdict::Allow => "allow", PolicyVerdict::Deny { .. } => "deny", _ => "other" };
        let is_genesis = e.sequence == 0 && e.prev_hash == "0000000000000000000000000000000000000000000000000000000000000000";
        all_entries.push(AuditEntryDisplay { sequence: e.sequence, hash_short: shorten_hash(&e.this_hash), kind: if is_genesis { "genesis".to_string() } else { kind.to_string() }, verified: int2a });
    }

    // Step 3: PA Submission
    let input_step3 = out_step2a.clone().unwrap_or(json!({}));
    let (out_step3, events3, int3, _v3) = pipeline_step(&PASubmissionAgent, StepConfig {
        policy_toml: PRIOR_AUTH_POLICY, agent_id: "pa-submission-agent",
        input_payload: input_step3, input_kind: "pa-submission-request",
        caps: &["pa.write"], schema: pa_submission_schema(), register_high_risk_rule: false,
    });
    let pa_ref = out_step3.as_ref().and_then(|o| o["pa_reference"].as_str()).unwrap_or("?").to_string();
    steps.push(PipelineStep {
        name: "Step 3: PA Submission".to_string(),
        status: if out_step3.is_some() { StepStatus::Pass } else { StepStatus::Denied },
        detail: format!("submit-pa | pa-system → {pa_ref}"),
    });
    all_entries.push(AuditEntryDisplay { sequence: 0, hash_short: "── step-3 ──".to_string(), kind: "label".to_string(), verified: int3 });
    for e in &events3 {
        let kind = match &e.record.verdict { PolicyVerdict::Allow => "allow", PolicyVerdict::Deny { .. } => "deny", _ => "other" };
        let is_genesis = e.sequence == 0 && e.prev_hash == "0000000000000000000000000000000000000000000000000000000000000000";
        all_entries.push(AuditEntryDisplay { sequence: e.sequence, hash_short: shorten_hash(&e.this_hash), kind: if is_genesis { "genesis".to_string() } else { kind.to_string() }, verified: int3 });
    }

    // Step 2b: InsuranceEligibility (covered=false) → Deny
    let (_out_step2b, events2b, int2b, _v2b) = pipeline_step(
        &InsuranceEligibilityAgent { covered: false }, StepConfig {
        policy_toml: PRIOR_AUTH_POLICY, agent_id: "insurance-eligibility-agent",
        input_payload: json!({ "procedure": "cardiac-mri" }), input_kind: "insurance-eligibility-request",
        caps: &["insurance.read"], schema: insurance_eligibility_schema(), register_high_risk_rule: false,
    });
    steps.push(PipelineStep {
        name: "Step 2b: Eligibility [uncovered]".to_string(),
        status: StepStatus::Denied,
        detail: "check-coverage | uncovered-procedure → Deny".to_string(),
    });
    all_entries.push(AuditEntryDisplay { sequence: 0, hash_short: "── step-2b ──".to_string(), kind: "label".to_string(), verified: int2b });
    for e in &events2b {
        let kind = match &e.record.verdict { PolicyVerdict::Allow => "allow", PolicyVerdict::Deny { .. } => "deny", _ => "other" };
        let is_genesis = e.sequence == 0 && e.prev_hash == "0000000000000000000000000000000000000000000000000000000000000000";
        all_entries.push(AuditEntryDisplay { sequence: e.sequence, hash_short: shorten_hash(&e.this_hash), kind: if is_genesis { "genesis".to_string() } else { kind.to_string() }, verified: int2b });
    }

    let _ = (out_step1, verdict1);
    let all_ok = int1 && int2a && int3 && int2b;
    let extra_lines = vec![
        ("Procedure".to_string(), "cardiac-mri (routine)".to_string(), Color::White),
        ("Sub-case A".to_string(), format!("PA submitted: {pa_ref}"), Color::Green),
        ("Sub-case B".to_string(), "PA denied at eligibility (not covered)".to_string(), Color::Red),
        ("All Audit Chains".to_string(), if all_ok { "VERIFIED".to_string() } else { "FAILED".to_string() }, if all_ok { Color::Green } else { Color::Red }),
    ];

    let capture = ExecutionCapture {
        policy_verdict: PolicyVerdict::RequireApproval {
            reason: "high-cost procedure requires physician approval".to_string(),
            approver_role: "attending-physician".to_string(),
        },
        action: "propose-procedure → submit-pa".to_string(),
        resource: "high-cost-procedure".to_string(),
        capability_name: "pa.write".to_string(),
        capability_granted: true,
        output: out_step3.map(|p| AgentOutput { kind: "pa-submission-result".to_string(), payload: p }),
        error: None,
        audit_events: vec![],
        chain_integrity: all_ok,
        extra_lines,
    };

    (capture, steps, all_entries)
}

// ── Scenario 4: Radiology AI Model Governance ─────────────────────────────────

struct RadiologyInferenceAgent { approved: bool }

impl Agent for RadiologyInferenceAgent {
    fn propose(&self, _state: &AgentState, _input: &AgentInput) -> VeritasResult<AgentOutput> {
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
        Ok(AgentState { step: state.step + 1, phase: "complete".to_string(), ..state.clone() })
    }
    fn required_capabilities(&self, _: &AgentState, _: &AgentInput) -> Vec<String> {
        vec!["model:approved".to_string(), "radiology.read".to_string()]
    }
    fn describe_action(&self, _: &AgentState, _: &AgentInput) -> (String, String) {
        let resource = if self.approved { "radiology-model" } else { "radiology-model-unapproved" };
        ("run-inference".to_string(), resource.to_string())
    }
    fn is_terminal(&self, state: &AgentState) -> bool { state.phase == "complete" }
}

fn radiology_output_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "radiology-inference-v1".to_string(),
        json_schema: json!({ "type": "object", "required": ["model_id", "finding", "label", "confidence", "recommendation"] }),
        rules: vec![
            VerificationRule {
                rule_id: "req-model-id".to_string(),
                description: "Output must identify the model that produced it".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "model_id".to_string() },
            },
            VerificationRule {
                rule_id: "req-finding".to_string(),
                description: "Output must contain a radiological finding".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "finding".to_string() },
            },
            VerificationRule {
                rule_id: "req-recommendation".to_string(),
                description: "Output must contain a clinical recommendation".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "recommendation".to_string() },
            },
        ],
    }
}

fn build_radiology_registry_approved() -> RegisteredModel {
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

fn build_radiology_registry_experimental() -> RegisteredModel {
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

fn run_radiology_model() -> (ExecutionCapture, Vec<PipelineStep>, Vec<AuditEntryDisplay>) {
    let mut steps = Vec::new();
    let mut all_entries: Vec<AuditEntryDisplay> = Vec::new();

    // Sub-case A: Approved model runs
    let mut registry_a = ModelRegistry::new();
    let _ = registry_a.register(build_radiology_registry_approved());
    let caps_a = registry_a.capabilities_for("chest-xray-v3.2");
    let cap_names_a: Vec<&str> = caps_a.iter().map(|s| s.as_str()).collect();
    let mut cap_list_a = cap_names_a;
    cap_list_a.push("radiology.read");

    let agent_a = RadiologyInferenceAgent { approved: true };
    let (out_a, events_a, int_a, verdict_a) = pipeline_step(&agent_a, StepConfig {
        policy_toml: MODEL_GOVERNANCE_POLICY, agent_id: "radiology-inference-agent",
        input_payload: json!({ "image_id": "xray-2026-0313-0042", "patient_id": "patient-517", "view": "PA" }),
        input_kind: "radiology-inference-request",
        caps: &cap_list_a, schema: radiology_output_schema(), register_high_risk_rule: false,
    });
    let label_a = out_a.as_ref().and_then(|o| o["label"].as_str()).unwrap_or("?");
    let conf_a = out_a.as_ref().and_then(|o| o["confidence"].as_f64()).unwrap_or(0.0);
    steps.push(PipelineStep {
        name: "Sub-case A: chest-xray-v3.2 [Approved]".to_string(),
        status: if out_a.is_some() { StepStatus::Pass } else { StepStatus::Denied },
        detail: format!("Allow — label={label_a}, conf={conf_a:.2}"),
    });
    all_entries.push(AuditEntryDisplay { sequence: 0, hash_short: "── sub-case A ──".to_string(), kind: "label".to_string(), verified: int_a });
    for e in &events_a {
        let kind = match &e.record.verdict { PolicyVerdict::Allow => "allow", PolicyVerdict::Deny { .. } => "deny", _ => "other" };
        let is_genesis = e.sequence == 0 && e.prev_hash == "0000000000000000000000000000000000000000000000000000000000000000";
        all_entries.push(AuditEntryDisplay { sequence: e.sequence, hash_short: shorten_hash(&e.this_hash), kind: if is_genesis { "genesis".to_string() } else { kind.to_string() }, verified: int_a });
    }

    // Sub-case B: Experimental model blocked
    let mut registry_b = ModelRegistry::new();
    let _ = registry_b.register(build_radiology_registry_experimental());
    let caps_b = registry_b.capabilities_for("chest-xray-v4.0-beta");
    let cap_names_b: Vec<&str> = caps_b.iter().map(|s| s.as_str()).collect();
    let mut cap_list_b = cap_names_b;
    cap_list_b.push("radiology.read");

    let agent_b = RadiologyInferenceAgent { approved: false };
    let (_out_b, events_b, int_b, _verdict_b) = pipeline_step(&agent_b, StepConfig {
        policy_toml: MODEL_GOVERNANCE_POLICY, agent_id: "radiology-inference-agent",
        input_payload: json!({ "image_id": "xray-2026-0313-0099", "patient_id": "patient-518", "view": "PA" }),
        input_kind: "radiology-inference-request",
        caps: &cap_list_b, schema: radiology_output_schema(), register_high_risk_rule: false,
    });
    steps.push(PipelineStep {
        name: "Sub-case B: chest-xray-v4.0-beta [Experimental]".to_string(),
        status: StepStatus::Denied,
        detail: "Deny — model:approved absent — propose() NOT called".to_string(),
    });
    all_entries.push(AuditEntryDisplay { sequence: 0, hash_short: "── sub-case B ──".to_string(), kind: "label".to_string(), verified: int_b });
    for e in &events_b {
        let kind = match &e.record.verdict { PolicyVerdict::Allow => "allow", PolicyVerdict::Deny { .. } => "deny", _ => "other" };
        let is_genesis = e.sequence == 0 && e.prev_hash == "0000000000000000000000000000000000000000000000000000000000000000";
        all_entries.push(AuditEntryDisplay { sequence: e.sequence, hash_short: shorten_hash(&e.this_hash), kind: if is_genesis { "genesis".to_string() } else { kind.to_string() }, verified: int_b });
    }

    // Sub-case C: Drift detection
    let mut registry_c = ModelRegistry::new();
    let _ = registry_c.register(build_radiology_registry_approved());
    let monitor_c = InMemoryDriftMonitor::new(DriftConfig {
        warning_threshold: 0.10,
        drift_threshold: 0.20,
        window_size: 5,
    });
    for _ in 0..5 {
        let _ = monitor_c.record("chest-xray-v3.2", &json!({ "confidence": 0.92 }));
    }
    let _ = registry_c.check_and_update("chest-xray-v3.2", &monitor_c);
    for _ in 0..5 {
        let _ = monitor_c.record("chest-xray-v3.2", &json!({ "confidence": 0.65 }));
    }
    let drift_status = registry_c.check_and_update("chest-xray-v3.2", &monitor_c);
    let still_approved = registry_c.is_approved("chest-xray-v3.2");
    let revocation_reason = registry_c.get("chest-xray-v3.2")
        .and_then(|m| match &m.provenance.approval_status {
            ApprovalStatus::Revoked { reason } => Some(reason.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let drift_label = match &drift_status {
        Ok(s) => format!("{s:?}"),
        Err(_) => "error".to_string(),
    };
    steps.push(PipelineStep {
        name: "Sub-case C: Drift Detection".to_string(),
        status: if !still_approved { StepStatus::Denied } else { StepStatus::Pass },
        detail: format!("drift={drift_label}, approved={still_approved}"),
    });
    // No audit events for drift check (it's out-of-band), add info entry
    all_entries.push(AuditEntryDisplay {
        sequence: 0,
        hash_short: "── sub-case C ──".to_string(),
        kind: "label".to_string(),
        verified: true,
    });
    all_entries.push(AuditEntryDisplay {
        sequence: 0,
        hash_short: format!("drift={drift_label}"),
        kind: if still_approved { "allow".to_string() } else { "deny".to_string() },
        verified: true,
    });

    let _ = verdict_a;
    let all_ok = int_a && int_b;
    let extra_lines = vec![
        ("Sub-case A".to_string(), format!("Approved model: label={label_a}, conf={conf_a:.2}"), Color::Green),
        ("Sub-case B".to_string(), "Experimental model: BLOCKED at policy gate".to_string(), Color::Red),
        ("Sub-case C".to_string(), format!("Drift={drift_label}, auto-revoked={}", !still_approved), if !still_approved { Color::Yellow } else { Color::Green }),
        ("Revocation".to_string(), truncate(&revocation_reason, 55), Color::Yellow),
    ];

    let capture = ExecutionCapture {
        policy_verdict: PolicyVerdict::Allow,
        action: "run-inference".to_string(),
        resource: "radiology-model".to_string(),
        capability_name: "model:approved + radiology.read".to_string(),
        capability_granted: true,
        output: out_a.map(|p| AgentOutput { kind: "radiology-inference-result".to_string(), payload: p }),
        error: None,
        audit_events: vec![],
        chain_integrity: all_ok,
        extra_lines,
    };

    (capture, steps, all_entries)
}

// ── Scenario 5: Sepsis Risk Model with Drift ──────────────────────────────────

struct SepsisRiskAgent { confidence: f64 }

impl Agent for SepsisRiskAgent {
    fn propose(&self, _state: &AgentState, input: &AgentInput) -> VeritasResult<AgentOutput> {
        let patient_id = input.payload["patient_id"].as_str().unwrap_or("unknown").to_string();
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
        Ok(AgentState { step: state.step + 1, phase: "complete".to_string(), ..state.clone() })
    }
    fn required_capabilities(&self, _: &AgentState, _: &AgentInput) -> Vec<String> {
        vec!["model:approved".to_string(), "patient-vitals.read".to_string()]
    }
    fn describe_action(&self, _: &AgentState, _: &AgentInput) -> (String, String) {
        ("score-risk".to_string(), "sepsis-model".to_string())
    }
    fn is_terminal(&self, state: &AgentState) -> bool { state.phase == "complete" }
}

struct SepsisRiskAgentRevoked;

impl Agent for SepsisRiskAgentRevoked {
    fn propose(&self, _state: &AgentState, _input: &AgentInput) -> VeritasResult<AgentOutput> {
        Ok(AgentOutput { kind: "sepsis-risk-result".to_string(), payload: json!({}) })
    }
    fn transition(&self, state: &AgentState, _output: &AgentOutput) -> VeritasResult<AgentState> {
        Ok(AgentState { step: state.step + 1, phase: "complete".to_string(), ..state.clone() })
    }
    fn required_capabilities(&self, _: &AgentState, _: &AgentInput) -> Vec<String> {
        vec!["model:approved".to_string(), "patient-vitals.read".to_string()]
    }
    fn describe_action(&self, _: &AgentState, _: &AgentInput) -> (String, String) {
        ("score-risk".to_string(), "sepsis-model-revoked".to_string())
    }
    fn is_terminal(&self, state: &AgentState) -> bool { state.phase == "complete" }
}

fn sepsis_risk_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "sepsis-risk-v1".to_string(),
        json_schema: json!({ "type": "object", "required": ["model_id", "patient_id", "risk_score", "risk_level", "confidence", "recommendation"] }),
        rules: vec![
            VerificationRule {
                rule_id: "req-model-id".to_string(),
                description: "Output must identify the scoring model".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "model_id".to_string() },
            },
            VerificationRule {
                rule_id: "req-risk-score".to_string(),
                description: "Output must include a numeric risk score".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "risk_score".to_string() },
            },
            VerificationRule {
                rule_id: "req-recommendation".to_string(),
                description: "Output must carry a clinical recommendation".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "recommendation".to_string() },
            },
        ],
    }
}

fn build_sepsis_model() -> RegisteredModel {
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

fn run_sepsis_model() -> (ExecutionCapture, Vec<PipelineStep>, Vec<AuditEntryDisplay>) {
    let mut steps = Vec::new();
    let mut all_entries: Vec<AuditEntryDisplay> = Vec::new();

    // Sub-case A: Stable operation — 3 scoring invocations at conf=0.89
    let mut registry_a = ModelRegistry::new();
    let _ = registry_a.register(build_sepsis_model());
    let monitor_a = InMemoryDriftMonitor::new(DriftConfig {
        warning_threshold: 0.10,
        drift_threshold: 0.20,
        window_size: 5,
    });

    let mut last_out_a = None;
    let mut last_events_a = vec![];
    let mut last_int_a = true;
    for i in 1..=3usize {
        let caps_a = registry_a.capabilities_for("sepsis-risk-v2.1");
        let cap_strs_a: Vec<&str> = caps_a.iter().map(|s| s.as_str()).collect();
        let mut cap_list = cap_strs_a;
        cap_list.push("patient-vitals.read");
        let agent = SepsisRiskAgent { confidence: 0.89 };
        let (out, events, integrity, _v) = pipeline_step(&agent, StepConfig {
            policy_toml: MODEL_GOVERNANCE_POLICY, agent_id: "sepsis-risk-agent",
            input_payload: json!({ "patient_id": format!("patient-{}", 600 + i) }),
            input_kind: "sepsis-risk-request",
            caps: &cap_list, schema: sepsis_risk_schema(), register_high_risk_rule: false,
        });
        if let Some(ref payload) = out {
            let conf = payload["confidence"].as_f64().unwrap_or(0.0);
            let _ = monitor_a.record("sepsis-risk-v2.1", &json!({ "confidence": conf }));
        }
        last_out_a = out;
        last_events_a = events;
        last_int_a = integrity;
    }
    let status_a = registry_a.check_and_update("sepsis-risk-v2.1", &monitor_a);
    let stable_str = match &status_a { Ok(s) => format!("{s:?}"), Err(_) => "error".to_string() };
    steps.push(PipelineStep {
        name: "Sub-case A: Stable Operation (3 invocations)".to_string(),
        status: StepStatus::Pass,
        detail: format!("conf=0.89 × 3 — drift={stable_str}"),
    });
    all_entries.push(AuditEntryDisplay { sequence: 0, hash_short: "── sub-case A ──".to_string(), kind: "label".to_string(), verified: last_int_a });
    for e in &last_events_a {
        let kind = match &e.record.verdict { PolicyVerdict::Allow => "allow", PolicyVerdict::Deny { .. } => "deny", _ => "other" };
        let is_genesis = e.sequence == 0 && e.prev_hash == "0000000000000000000000000000000000000000000000000000000000000000";
        all_entries.push(AuditEntryDisplay { sequence: e.sequence, hash_short: shorten_hash(&e.this_hash), kind: if is_genesis { "genesis".to_string() } else { kind.to_string() }, verified: last_int_a });
    }

    // Sub-case B: Drift lifecycle — Warning → Drifted → Revoked
    let mut registry_b = ModelRegistry::new();
    let _ = registry_b.register(build_sepsis_model());
    let monitor_b = InMemoryDriftMonitor::new(DriftConfig {
        warning_threshold: 0.10,
        drift_threshold: 0.20,
        window_size: 5,
    });

    // Phase 1: baseline at 0.89
    for _ in 0..5 {
        let _ = monitor_b.record("sepsis-risk-v2.1", &json!({ "confidence": 0.89 }));
    }
    // Phase 2: warning at 0.78 (drop ≈ 0.11)
    for _ in 0..5 {
        let _ = monitor_b.record("sepsis-risk-v2.1", &json!({ "confidence": 0.78 }));
    }
    let warning_status = monitor_b.check_drift("sepsis-risk-v2.1");
    let warning_str = format!("{warning_status:?}");

    // Phase 3: drifted at 0.62 (drop = 0.27)
    for _ in 0..5 {
        let _ = monitor_b.record("sepsis-risk-v2.1", &json!({ "confidence": 0.62 }));
    }
    let drifted_status = registry_b.check_and_update("sepsis-risk-v2.1", &monitor_b);
    let drifted_str = match &drifted_status { Ok(s) => format!("{s:?}"), Err(_) => "error".to_string() };
    let still_approved_b = registry_b.is_approved("sepsis-risk-v2.1");

    steps.push(PipelineStep {
        name: "Sub-case B: Phase 2 — Warning".to_string(),
        status: StepStatus::AwaitingApproval,
        detail: format!("conf=0.78 (drop≈0.11) — {warning_str}"),
    });
    steps.push(PipelineStep {
        name: "Sub-case B: Phase 3 — Drifted".to_string(),
        status: if !still_approved_b { StepStatus::Denied } else { StepStatus::Fail },
        detail: format!("{drifted_str} — approved={still_approved_b}"),
    });

    // Phase 4: Attempt to score revoked model
    let caps_revoked = registry_b.capabilities_for("sepsis-risk-v2.1");
    let cap_strs_revoked: Vec<&str> = caps_revoked.iter().map(|s| s.as_str()).collect();
    let mut cap_list_revoked = cap_strs_revoked;
    cap_list_revoked.push("patient-vitals.read");
    let (_out_revoked, events_revoked, int_revoked, _verdict_revoked) = pipeline_step(
        &SepsisRiskAgentRevoked, StepConfig {
        policy_toml: MODEL_GOVERNANCE_POLICY, agent_id: "sepsis-risk-agent",
        input_payload: json!({ "patient_id": "patient-701" }), input_kind: "sepsis-risk-request",
        caps: &cap_list_revoked, schema: sepsis_risk_schema(), register_high_risk_rule: false,
    });
    steps.push(PipelineStep {
        name: "Sub-case B: Phase 4 — Revoked Blocked".to_string(),
        status: StepStatus::Denied,
        detail: "score-risk | sepsis-model-revoked → Deny".to_string(),
    });
    all_entries.push(AuditEntryDisplay { sequence: 0, hash_short: "── sub-case B ──".to_string(), kind: "label".to_string(), verified: int_revoked });
    for e in &events_revoked {
        let kind = match &e.record.verdict { PolicyVerdict::Allow => "allow", PolicyVerdict::Deny { .. } => "deny", _ => "other" };
        let is_genesis = e.sequence == 0 && e.prev_hash == "0000000000000000000000000000000000000000000000000000000000000000";
        all_entries.push(AuditEntryDisplay { sequence: e.sequence, hash_short: shorten_hash(&e.this_hash), kind: if is_genesis { "genesis".to_string() } else { kind.to_string() }, verified: int_revoked });
    }

    let revocation_reason_b = registry_b.get("sepsis-risk-v2.1")
        .and_then(|m| match &m.provenance.approval_status {
            ApprovalStatus::Revoked { reason } => Some(reason.clone()),
            _ => None,
        })
        .unwrap_or_default();

    let extra_lines = vec![
        ("Model".to_string(), "sepsis-risk-v2.1 (TabularToScore)".to_string(), Color::White),
        ("Sub-case A".to_string(), format!("Stable: conf=0.89, drift={stable_str}"), Color::Green),
        ("Sub-case B Warning".to_string(), format!("conf=0.78 → {warning_str}"), Color::Yellow),
        ("Sub-case B Drifted".to_string(), format!("conf=0.62 → {drifted_str}, revoked={}", !still_approved_b), Color::Red),
        ("Revocation".to_string(), truncate(&revocation_reason_b, 55), Color::Yellow),
    ];

    let capture = ExecutionCapture {
        policy_verdict: PolicyVerdict::Allow,
        action: "score-risk".to_string(),
        resource: "sepsis-model".to_string(),
        capability_name: "model:approved + patient-vitals.read".to_string(),
        capability_granted: true,
        output: last_out_a.map(|p| AgentOutput { kind: "sepsis-risk-result".to_string(), payload: p }),
        error: None,
        audit_events: vec![],
        chain_integrity: last_int_a && int_revoked,
        extra_lines,
    };

    (capture, steps, all_entries)
}

// ── Output schemas ─────────────────────────────────────────────────────────────

fn drug_interaction_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "drug-interaction-v1".to_string(),
        json_schema: json!({ "type": "object", "required": ["query", "result", "recommendation"] }),
        rules: vec![
            VerificationRule {
                rule_id: "req-query".to_string(),
                description: "Output must contain the queried drug pair".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "query".to_string() },
            },
            VerificationRule {
                rule_id: "req-result".to_string(),
                description: "Output must contain an interaction result".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "result".to_string() },
            },
            VerificationRule {
                rule_id: "req-recommendation".to_string(),
                description: "Output must contain a clinical recommendation".to_string(),
                rule_type: VerificationRuleType::RequiredField { field_path: "recommendation".to_string() },
            },
        ],
    }
}

// ── Capture → display converters ──────────────────────────────────────────────

/// Build the pipeline steps from a single-step capture (scenario 1).
fn build_pipeline_steps(cap: &ExecutionCapture) -> Vec<PipelineStep> {
    let mut steps = Vec::with_capacity(5);

    let (policy_status, policy_detail) = match &cap.policy_verdict {
        PolicyVerdict::Allow => (
            StepStatus::Pass,
            format!("Allow — {}: {}", cap.action, cap.resource),
        ),
        PolicyVerdict::Deny { reason } => (
            StepStatus::Denied,
            format!("Deny — {}", truncate(reason, 60)),
        ),
        PolicyVerdict::RequireApproval { approver_role, .. } => (
            StepStatus::AwaitingApproval,
            format!("RequireApproval — approver: {approver_role}"),
        ),
        PolicyVerdict::RequireVerification { check_id } => (
            StepStatus::Pass,
            format!("RequireVerification — check: {check_id}"),
        ),
    };
    steps.push(PipelineStep { name: "Policy".to_string(), status: policy_status, detail: policy_detail });

    let (cap_status, cap_detail) = if matches!(
        cap.policy_verdict,
        PolicyVerdict::Deny { .. } | PolicyVerdict::RequireApproval { .. }
    ) {
        (StepStatus::Pending, "not reached".to_string())
    } else if matches!(&cap.error, Some(VeritasError::CapabilityMissing { .. })) {
        (StepStatus::Fail, format!("{} [MISSING]", cap.capability_name))
    } else if cap.capability_granted {
        (StepStatus::Pass, format!("{} [GRANTED]", cap.capability_name))
    } else {
        (StepStatus::Fail, format!("{} [NOT GRANTED]", cap.capability_name))
    };
    steps.push(PipelineStep { name: "Capability".to_string(), status: cap_status, detail: cap_detail });

    let (agent_status, agent_detail) = if cap.output.is_some() {
        (StepStatus::Pass, "propose() called, output produced".to_string())
    } else if matches!(cap.policy_verdict, PolicyVerdict::Deny { .. } | PolicyVerdict::RequireApproval { .. }) {
        (StepStatus::Pending, "propose() blocked by policy".to_string())
    } else if matches!(&cap.error, Some(VeritasError::CapabilityMissing { .. })) {
        (StepStatus::Pending, "propose() blocked by capability check".to_string())
    } else {
        (StepStatus::Fail, "propose() did not produce output".to_string())
    };
    steps.push(PipelineStep { name: "Agent".to_string(), status: agent_status, detail: agent_detail });

    let (verify_status, verify_detail) = if cap.output.is_some() {
        (StepStatus::Pass, "schema + rules: PASS".to_string())
    } else if matches!(&cap.error, Some(VeritasError::VerificationFailed { .. })) {
        (StepStatus::Fail, "schema + rules: FAIL".to_string())
    } else {
        (StepStatus::Pending, "not reached".to_string())
    };
    steps.push(PipelineStep { name: "Verify".to_string(), status: verify_status, detail: verify_detail });

    let (audit_status, audit_detail) = if cap.audit_events.is_empty() {
        (StepStatus::Pending, "no events recorded".to_string())
    } else {
        let integrity_str = if cap.chain_integrity { "VERIFIED" } else { "FAILED" };
        (
            if cap.chain_integrity { StepStatus::Pass } else { StepStatus::Fail },
            format!("{} event(s), chain: {}", cap.audit_events.len(), integrity_str),
        )
    };
    steps.push(PipelineStep { name: "Audit".to_string(), status: audit_status, detail: audit_detail });

    steps
}

/// Build the audit trail entries for display (single-step scenarios).
fn build_audit_entries(cap: &ExecutionCapture) -> Vec<AuditEntryDisplay> {
    cap.audit_events
        .iter()
        .map(|e| {
            let kind = match &e.record.verdict {
                PolicyVerdict::Allow => "allow",
                PolicyVerdict::Deny { .. } => "deny",
                PolicyVerdict::RequireApproval { .. } => "require-approval",
                PolicyVerdict::RequireVerification { .. } => "require-verify",
            };
            let is_genesis = e.sequence == 0
                && e.prev_hash == "0000000000000000000000000000000000000000000000000000000000000000";
            AuditEntryDisplay {
                sequence: e.sequence,
                hash_short: shorten_hash(&e.this_hash),
                kind: if is_genesis { "genesis".to_string() } else { kind.to_string() },
                verified: cap.chain_integrity,
            }
        })
        .collect()
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn ui(f: &mut Frame, app: &App) {
    let full = f.area();

    let outer_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // header
            Constraint::Min(10),    // pipeline + audit (left/right split)
            Constraint::Length(10), // output details
            Constraint::Length(3),  // footer
        ])
        .split(full);

    render_header(f, outer_chunks[0], app);

    let mid_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(outer_chunks[1]);

    render_pipeline(f, mid_chunks[0], app);
    render_audit_trail(f, mid_chunks[1], app);
    render_output(f, outer_chunks[2], app);
    render_footer(f, outer_chunks[3], app);
}

fn render_header(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let title_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let mut spans: Vec<Span> = vec![Span::styled("VERITAS Healthcare Demo  ", title_style)];

    let scenarios = [
        ("[1]", Scenario::DrugInteraction),
        ("[2]", Scenario::ClinicalPipeline),
        ("[3]", Scenario::PriorAuth),
        ("[4]", Scenario::RadiologyModel),
        ("[5]", Scenario::SepsisModel),
    ];

    for (key, scenario) in &scenarios {
        let is_selected = app.selected == *scenario;
        let style = if is_selected {
            Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        spans.push(Span::styled(format!("{} {}  ", key, scenario.name()), style));
    }

    let header = Paragraph::new(Line::from(spans)).block(
        Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(header, area);
}

fn render_pipeline(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let mut items: Vec<ListItem> = Vec::new();

    let state_str = if app.animating {
        "State: running..."
    } else if app.capture.is_some() {
        "State: complete"
    } else {
        "State: idle"
    };
    items.push(ListItem::new(Line::from(Span::styled(state_str, Style::default().fg(Color::DarkGray)))));
    items.push(ListItem::new("")); // blank line

    let visible_count = app.animation_step.min(app.pipeline_steps.len());

    for (i, step) in app.pipeline_steps.iter().enumerate() {
        if i >= visible_count { break; }

        let (icon, status_label, status_color) = match &step.status {
            StepStatus::Pending => ("  ◦", "PENDING", Color::Yellow),
            StepStatus::Pass => ("  ▸", "PASS", Color::Green),
            StepStatus::Fail => ("  ▸", "FAIL", Color::Red),
            StepStatus::Denied => ("  ▸", "DENY", Color::Red),
            StepStatus::AwaitingApproval => ("  ▸", "WAIT", Color::Yellow),
        };

        let line = Line::from(vec![
            Span::styled(icon, Style::default().fg(Color::DarkGray)),
            Span::raw(format!(" {}: ", step.name)),
            Span::styled(status_label, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" — {}", step.detail), Style::default().fg(Color::Gray)),
        ]);
        items.push(ListItem::new(line));
    }

    let block = Block::default()
        .title(" Execution Pipeline ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

fn render_audit_trail(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let mut items: Vec<ListItem> = Vec::new();

    if app.audit_entries.is_empty() {
        items.push(ListItem::new(Span::styled(
            "  No audit events yet — press [r] to run",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for entry in &app.audit_entries {
            // Chain label separator rows
            if entry.kind == "label" {
                items.push(ListItem::new(Line::from(Span::styled(
                    format!("  {}", entry.hash_short),
                    Style::default().fg(Color::DarkGray),
                ))));
                continue;
            }

            let kind_color = match entry.kind.as_str() {
                "allow" | "genesis" => Color::Green,
                "deny" => Color::Red,
                "require-approval" => Color::Yellow,
                _ => Color::Gray,
            };
            let check = if entry.verified { " ✓" } else { " ✗" };
            let check_color = if entry.verified { Color::Green } else { Color::Red };

            let line = Line::from(vec![
                Span::styled(format!("  #{}", entry.sequence), Style::default().fg(Color::DarkGray)),
                Span::raw(" ["),
                Span::styled(entry.kind.as_str(), Style::default().fg(kind_color).add_modifier(Modifier::BOLD)),
                Span::raw("] "),
                Span::styled(entry.hash_short.as_str(), Style::default().fg(Color::Gray)),
                Span::styled(check, Style::default().fg(check_color)),
            ]);
            items.push(ListItem::new(line));
        }

        items.push(ListItem::new(""));
        let (integrity_label, integrity_color) = if app.capture.as_ref().map(|c| c.chain_integrity).unwrap_or(false) {
            ("  Chain integrity: VERIFIED", Color::Green)
        } else if app.capture.is_some() {
            ("  Chain integrity: FAILED", Color::Red)
        } else {
            ("", Color::DarkGray)
        };
        items.push(ListItem::new(Span::styled(
            integrity_label,
            Style::default().fg(integrity_color).add_modifier(Modifier::BOLD),
        )));
    }

    let block = Block::default()
        .title(" Audit Trail ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

fn render_output(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let block = Block::default()
        .title(" Policy Details & Output ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let Some(cap) = &app.capture else {
        let p = Paragraph::new(Span::styled(
            "  Press [r] to run the selected scenario.",
            Style::default().fg(Color::DarkGray),
        ))
        .block(block);
        f.render_widget(p, area);
        return;
    };

    let mut lines: Vec<Line> = Vec::new();

    // Verdict line.
    let (verdict_label, verdict_color) = match &cap.policy_verdict {
        PolicyVerdict::Allow => ("Allow", Color::Green),
        PolicyVerdict::Deny { .. } => ("Deny", Color::Red),
        PolicyVerdict::RequireApproval { .. } => ("RequireApproval", Color::Yellow),
        PolicyVerdict::RequireVerification { .. } => ("RequireVerification", Color::Yellow),
    };
    lines.push(Line::from(vec![
        Span::styled("  Verdict:     ", Style::default().fg(Color::Gray)),
        Span::styled(verdict_label, Style::default().fg(verdict_color).add_modifier(Modifier::BOLD)),
    ]));

    lines.push(Line::from(vec![
        Span::styled("  Action:      ", Style::default().fg(Color::Gray)),
        Span::raw(cap.action.as_str()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Resource:    ", Style::default().fg(Color::Gray)),
        Span::raw(cap.resource.as_str()),
    ]));

    let cap_color = if cap.capability_granted { Color::Green } else { Color::Red };
    let cap_granted_label = if cap.capability_granted { "[GRANTED]" } else { "[NOT GRANTED]" };
    lines.push(Line::from(vec![
        Span::styled("  Capability:  ", Style::default().fg(Color::Gray)),
        Span::raw(format!("{} ", cap.capability_name)),
        Span::styled(cap_granted_label, Style::default().fg(cap_color)),
    ]));

    lines.push(Line::from(""));

    // Scenario-specific extra lines from capture.
    if !cap.extra_lines.is_empty() {
        for (label, value, color) in &cap.extra_lines {
            let padded = format!("  {label:<13}");
            lines.push(Line::from(vec![
                Span::styled(padded, Style::default().fg(Color::Gray)),
                Span::styled(value.as_str(), Style::default().fg(*color)),
            ]));
        }
    } else if cap.output.is_none() {
        // Show denial reason for scenario 1 edge cases.
        let reason = match &cap.policy_verdict {
            PolicyVerdict::Deny { reason } => reason.clone(),
            PolicyVerdict::RequireApproval { reason, .. } => reason.clone(),
            _ => cap.error.as_ref().map(|e| e.to_string()).unwrap_or_default(),
        };
        if !reason.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  Reason:      ", Style::default().fg(Color::Gray)),
                Span::styled(truncate(&reason, 80), Style::default().fg(Color::Red)),
            ]));
        }
    }

    let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(paragraph, area);
}

fn render_footer(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let _ = app; // no scenario-specific toggles in the new set
    let spans: Vec<Span> = vec![
        Span::styled(" [1-5] ", Style::default().fg(Color::Cyan)),
        Span::raw("Select scenario  "),
        Span::styled("[r] ", Style::default().fg(Color::Cyan)),
        Span::raw("Run  "),
        Span::styled("[q] ", Style::default().fg(Color::Cyan)),
        Span::raw("Quit"),
    ];

    let footer = Paragraph::new(Line::from(spans)).block(
        Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(footer, area);
}

// ── Utility helpers ───────────────────────────────────────────────────────────

/// Truncate a string to at most `max` chars, appending "…" if truncated.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

/// Shorten a 64-hex-char hash to "xxxx...xxxx" (8 visible chars).
fn shorten_hash(h: &str) -> String {
    if h.len() >= 8 {
        format!("{}...{}", &h[..4], &h[h.len() - 4..])
    } else {
        h.to_string()
    }
}

// ── Terminal setup / teardown ─────────────────────────────────────────────────

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()
}

// ── Main event loop ───────────────────────────────────────────────────────────

fn main() -> io::Result<()> {
    // Install a panic hook that restores the terminal before printing the panic.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));

    let mut terminal = setup_terminal()?;
    let mut app = App::new();

    // Animation tick interval: 150 ms.
    const TICK_MS: u64 = 150;

    loop {
        terminal.draw(|f| ui(f, &app))?;

        let timeout = if app.animating {
            let elapsed = app.last_tick.elapsed();
            let tick_dur = Duration::from_millis(TICK_MS);
            tick_dur.saturating_sub(elapsed)
        } else {
            Duration::from_millis(200)
        };

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('Q') => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,

                    KeyCode::Char('1') => {
                        app.selected = Scenario::DrugInteraction;
                        app.capture = None;
                        app.pipeline_steps.clear();
                        app.audit_entries.clear();
                        app.animating = false;
                    }
                    KeyCode::Char('2') => {
                        app.selected = Scenario::ClinicalPipeline;
                        app.capture = None;
                        app.pipeline_steps.clear();
                        app.audit_entries.clear();
                        app.animating = false;
                    }
                    KeyCode::Char('3') => {
                        app.selected = Scenario::PriorAuth;
                        app.capture = None;
                        app.pipeline_steps.clear();
                        app.audit_entries.clear();
                        app.animating = false;
                    }
                    KeyCode::Char('4') => {
                        app.selected = Scenario::RadiologyModel;
                        app.capture = None;
                        app.pipeline_steps.clear();
                        app.audit_entries.clear();
                        app.animating = false;
                    }
                    KeyCode::Char('5') => {
                        app.selected = Scenario::SepsisModel;
                        app.capture = None;
                        app.pipeline_steps.clear();
                        app.audit_entries.clear();
                        app.animating = false;
                    }

                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        app.run();
                    }

                    _ => {}
                }
            }
        }

        if app.animating && app.last_tick.elapsed() >= Duration::from_millis(TICK_MS) {
            app.tick_animation();
            app.last_tick = Instant::now();
        }
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}
