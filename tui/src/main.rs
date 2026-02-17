//! VERITAS Healthcare Demo — interactive Ratatui TUI
//!
//! Layout:
//!   ┌─── header ──────────────────────────────────────────────────────────┐
//!   │  [1] Drug Interaction  [2] Note Summarizer  [3] Patient Query       │
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
    agent::{AgentId, AgentInput, AgentOutput, AgentState, ExecutionId},
    capability::{Capability, CapabilitySet},
    error::{VeritasError, VeritasResult},
    execution::{StepRecord, StepResult},
    policy::PolicyVerdict,
    verify::{OutputSchema, VerificationRule, VerificationRuleType},
};
use veritas_core::{executor::Executor, traits::AuditWriter};
use veritas_policy::engine::TomlPolicyEngine;
use veritas_ref_healthcare::{
    scenarios::drug_interaction::DrugInteractionAgent,
    scenarios::note_summarizer::NoteSummarizerAgent,
    scenarios::patient_query::PatientQueryAgent,
};
use veritas_verify::engine::SchemaVerifier;

// ── Policy TOML (same as the healthcare scenarios use) ────────────────────────

const HEALTHCARE_POLICY: &str = include_str!(
    "../../crates/veritas-ref-healthcare/policies/healthcare.toml"
);

/// Open policy for the capability-missing sub-case of scenario 3.
const OPEN_POLICY_FOR_CAPABILITY_TEST: &str = r#"
[[rules]]
id = "allow-patient-query-open"
description = "Policy allows query on patient-records; capability enforcement left to executor"
action = "query"
resource = "patient-records"
verdict = "allow"
"#;

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
    NoteSummarizer,
    PatientQuery,
}

impl Scenario {
    fn name(self) -> &'static str {
        match self {
            Scenario::DrugInteraction => "Drug Interaction",
            Scenario::NoteSummarizer => "Note Summarizer",
            Scenario::PatientQuery => "Patient Query",
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
}

// ── App state ─────────────────────────────────────────────────────────────────

struct App {
    selected: Scenario,

    // Toggle controls for Scenario 3.
    consent_enabled: bool,
    capability_enabled: bool,

    // Most recent run result.
    capture: Option<ExecutionCapture>,

    // Animated display: how many pipeline steps are currently revealed.
    animation_step: usize,
    // All pipeline steps derived from the last capture (up to 5).
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
            consent_enabled: true,
            capability_enabled: true,
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
        let capture = match self.selected {
            Scenario::DrugInteraction => run_drug_interaction(),
            Scenario::NoteSummarizer => run_note_summarizer(),
            Scenario::PatientQuery => {
                run_patient_query(self.consent_enabled, self.capability_enabled)
            }
        };

        self.pipeline_steps = build_pipeline_steps(&capture);
        self.audit_entries = build_audit_entries(&capture);
        self.capture = Some(capture);
        self.animation_step = 0;
        self.last_tick = Instant::now();
        self.animating = true;
    }
}

// ── Scenario runners ──────────────────────────────────────────────────────────

/// Run Scenario 1: Drug Interaction Checker.
fn run_drug_interaction() -> ExecutionCapture {
    let policy = match TomlPolicyEngine::from_toml_str(HEALTHCARE_POLICY) {
        Ok(p) => p,
        Err(e) => {
            return ExecutionCapture {
                policy_verdict: PolicyVerdict::Deny {
                    reason: format!("policy load error: {}", e),
                },
                action: "drug-interaction-check".to_string(),
                resource: "drug-database".to_string(),
                capability_name: "drug-database.read".to_string(),
                capability_granted: true,
                output: None,
                error: Some(e),
                audit_events: vec![],
                chain_integrity: false,
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
        Ok(StepResult::Denied { reason, .. }) => {
            (PolicyVerdict::Deny { reason }, None, None)
        }
        Ok(StepResult::AwaitingApproval { reason, approver_role, .. }) => {
            (PolicyVerdict::RequireApproval { reason, approver_role }, None, None)
        }
        Err(e) => {
            let v = PolicyVerdict::Deny {
                reason: e.to_string(),
            };
            (v, None, Some(e))
        }
    };

    let log = audit.export_log();
    let chain_integrity = audit.verify_integrity();

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
    }
}

/// Run Scenario 2: Clinical Note Summarizer.
fn run_note_summarizer() -> ExecutionCapture {
    let policy = match TomlPolicyEngine::from_toml_str(HEALTHCARE_POLICY) {
        Ok(p) => p,
        Err(e) => {
            return ExecutionCapture {
                policy_verdict: PolicyVerdict::Deny {
                    reason: format!("policy load error: {}", e),
                },
                action: "summarize".to_string(),
                resource: "clinical-notes".to_string(),
                capability_name: "clinical-notes.read".to_string(),
                capability_granted: true,
                output: None,
                error: Some(e),
                audit_events: vec![],
                chain_integrity: false,
            };
        }
    };

    let execution_id = ExecutionId::new();
    let audit = Arc::new(InMemoryAuditWriter::new(execution_id.0.to_string()));

    let mut verifier = SchemaVerifier::new();
    verifier.register_rule(
        "no-pii-labels",
        Box::new(|payload| {
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

    let agent = NoteSummarizerAgent;
    let schema = note_summarizer_schema();

    let state = AgentState {
        agent_id: AgentId("note-summarizer-agent".to_string()),
        execution_id: execution_id.clone(),
        phase: "active".to_string(),
        context: serde_json::Value::Null,
        step: 0,
    };

    let mut capabilities = CapabilitySet::default();
    capabilities.grant(Capability::new("clinical-notes.read"));

    let input = AgentInput {
        kind: "summarize-request".to_string(),
        payload: json!({ "patient_id": "patient-042" }),
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
        Ok(StepResult::Denied { reason, .. }) => {
            (PolicyVerdict::Deny { reason }, None, None)
        }
        Ok(StepResult::AwaitingApproval { reason, approver_role, .. }) => {
            (PolicyVerdict::RequireApproval { reason, approver_role }, None, None)
        }
        Err(e) => {
            let v = PolicyVerdict::Deny { reason: e.to_string() };
            (v, None, Some(e))
        }
    };

    let log = audit.export_log();
    let chain_integrity = audit.verify_integrity();

    ExecutionCapture {
        policy_verdict: verdict,
        action: "summarize".to_string(),
        resource: "clinical-notes".to_string(),
        capability_name: "clinical-notes.read".to_string(),
        capability_granted: true,
        output,
        error,
        audit_events: log.events,
        chain_integrity,
    }
}

/// Run Scenario 3: Patient Query with togglable consent and capability.
fn run_patient_query(consent_enabled: bool, capability_enabled: bool) -> ExecutionCapture {
    // Choose the patient ID based on consent toggle.
    // IDs ending in "nc" have ai_query_consent = false in mock_data.
    let patient_id = if consent_enabled {
        "patient-101".to_string()
    } else {
        "patient-201nc".to_string()
    };

    // When capability_enabled=false we use the open policy (which allows the action
    // without requiring the capability in TOML), so the executor's own capability
    // check fires and produces CapabilityMissing.
    let policy_toml = if capability_enabled {
        HEALTHCARE_POLICY
    } else {
        OPEN_POLICY_FOR_CAPABILITY_TEST
    };

    let policy = match TomlPolicyEngine::from_toml_str(policy_toml) {
        Ok(p) => p,
        Err(e) => {
            return ExecutionCapture {
                policy_verdict: PolicyVerdict::Deny {
                    reason: format!("policy load error: {}", e),
                },
                action: "query".to_string(),
                resource: "patient-records".to_string(),
                capability_name: "patient-records.read".to_string(),
                capability_granted: capability_enabled,
                output: None,
                error: Some(e),
                audit_events: vec![],
                chain_integrity: false,
            };
        }
    };

    let execution_id = ExecutionId::new();
    let audit = Arc::new(InMemoryAuditWriter::new(execution_id.0.to_string()));
    let verifier = SchemaVerifier::new();
    let agent = PatientQueryAgent { patient_id: patient_id.clone() };
    let schema = patient_query_schema();

    let state = AgentState {
        agent_id: AgentId("patient-query-agent".to_string()),
        execution_id: execution_id.clone(),
        phase: "active".to_string(),
        context: serde_json::Value::Null,
        step: 0,
    };

    let mut capabilities = CapabilitySet::default();
    if capability_enabled {
        capabilities.grant(Capability::new("patient-records.read"));
    }

    let input = AgentInput {
        kind: "patient-query".to_string(),
        payload: json!({ "patient_id": patient_id }),
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
        Ok(StepResult::Denied { reason, .. }) => {
            (PolicyVerdict::Deny { reason }, None, None)
        }
        Ok(StepResult::AwaitingApproval { reason, approver_role, .. }) => {
            (PolicyVerdict::RequireApproval { reason, approver_role }, None, None)
        }
        Err(e) => {
            let v = PolicyVerdict::Deny { reason: e.to_string() };
            (v, None, Some(e))
        }
    };

    let log = audit.export_log();
    let chain_integrity = audit.verify_integrity();

    // Determine the resource name the agent actually reported.
    let resource = if consent_enabled {
        "patient-records".to_string()
    } else {
        "patient-records-no-consent".to_string()
    };

    ExecutionCapture {
        policy_verdict: verdict,
        action: "query".to_string(),
        resource,
        capability_name: "patient-records.read".to_string(),
        capability_granted: capability_enabled,
        output,
        error,
        audit_events: log.events,
        chain_integrity,
    }
}

// ── Output schemas (mirrors what the scenario modules declare) ─────────────────

fn drug_interaction_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "drug-interaction-v1".to_string(),
        json_schema: json!({ "type": "object", "required": ["query", "result", "recommendation"] }),
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
                description: "Output must contain an interaction result".to_string(),
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

fn note_summarizer_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "clinical-summary-v1".to_string(),
        json_schema: json!({ "type": "object", "required": ["patient_id", "summary", "note_count"] }),
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

fn patient_query_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "patient-query-v1".to_string(),
        json_schema: json!({ "type": "object", "required": ["patient_id"] }),
        rules: vec![VerificationRule {
            rule_id: "req-patient-id".to_string(),
            description: "Output must contain the patient ID".to_string(),
            rule_type: VerificationRuleType::RequiredField {
                field_path: "patient_id".to_string(),
            },
        }],
    }
}

// ── Capture → display converters ──────────────────────────────────────────────

/// Build the 5 pipeline steps from a capture.
///
/// Steps: Policy → Capability → Agent → Verify → Audit
fn build_pipeline_steps(cap: &ExecutionCapture) -> Vec<PipelineStep> {
    let mut steps = Vec::with_capacity(5);

    // ── Step 1: Policy ────────────────────────────────────────────────────────
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
            format!("RequireApproval — approver: {}", approver_role),
        ),
        PolicyVerdict::RequireVerification { check_id } => (
            StepStatus::Pass,
            format!("RequireVerification — check: {}", check_id),
        ),
    };
    steps.push(PipelineStep {
        name: "Policy".to_string(),
        status: policy_status,
        detail: policy_detail,
    });

    // ── Step 2: Capability ────────────────────────────────────────────────────
    // If policy denied, capability was never reached — show as Pending.
    // If CapabilityMissing error, show as Fail.
    let (cap_status, cap_detail) = if matches!(
        cap.policy_verdict,
        PolicyVerdict::Deny { .. } | PolicyVerdict::RequireApproval { .. }
    ) {
        (StepStatus::Pending, "not reached".to_string())
    } else if matches!(&cap.error, Some(VeritasError::CapabilityMissing { .. })) {
        (
            StepStatus::Fail,
            format!("{} [MISSING]", cap.capability_name),
        )
    } else if cap.capability_granted {
        (
            StepStatus::Pass,
            format!("{} [GRANTED]", cap.capability_name),
        )
    } else {
        (
            StepStatus::Fail,
            format!("{} [NOT GRANTED]", cap.capability_name),
        )
    };
    steps.push(PipelineStep {
        name: "Capability".to_string(),
        status: cap_status,
        detail: cap_detail,
    });

    // ── Step 3: Agent ─────────────────────────────────────────────────────────
    let (agent_status, agent_detail) = if cap.output.is_some() {
        (StepStatus::Pass, "propose() called, output produced".to_string())
    } else if matches!(
        cap.policy_verdict,
        PolicyVerdict::Deny { .. } | PolicyVerdict::RequireApproval { .. }
    ) {
        (StepStatus::Pending, "propose() blocked by policy".to_string())
    } else if matches!(&cap.error, Some(VeritasError::CapabilityMissing { .. })) {
        (StepStatus::Pending, "propose() blocked by capability check".to_string())
    } else {
        (StepStatus::Fail, "propose() did not produce output".to_string())
    };
    steps.push(PipelineStep {
        name: "Agent".to_string(),
        status: agent_status,
        detail: agent_detail,
    });

    // ── Step 4: Verify ────────────────────────────────────────────────────────
    let (verify_status, verify_detail) = if cap.output.is_some() {
        (StepStatus::Pass, "schema + rules: PASS".to_string())
    } else if matches!(&cap.error, Some(VeritasError::VerificationFailed { .. })) {
        (StepStatus::Fail, "schema + rules: FAIL".to_string())
    } else {
        (StepStatus::Pending, "not reached".to_string())
    };
    steps.push(PipelineStep {
        name: "Verify".to_string(),
        status: verify_status,
        detail: verify_detail,
    });

    // ── Step 5: Audit ─────────────────────────────────────────────────────────
    let (audit_status, audit_detail) = if cap.audit_events.is_empty() {
        (StepStatus::Pending, "no events recorded".to_string())
    } else {
        let integrity_str = if cap.chain_integrity { "VERIFIED" } else { "FAILED" };
        (
            if cap.chain_integrity { StepStatus::Pass } else { StepStatus::Fail },
            format!(
                "{} event(s), chain: {}",
                cap.audit_events.len(),
                integrity_str
            ),
        )
    };
    steps.push(PipelineStep {
        name: "Audit".to_string(),
        status: audit_status,
        detail: audit_detail,
    });

    steps
}

/// Build the audit trail entries for display.
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
            // Genesis detection: first event's prev_hash is the genesis sentinel.
            let is_genesis = e.sequence == 0
                && e.prev_hash
                    == "0000000000000000000000000000000000000000000000000000000000000000";

            AuditEntryDisplay {
                sequence: e.sequence,
                hash_short: shorten_hash(&e.this_hash),
                kind: if is_genesis {
                    "genesis".to_string()
                } else {
                    kind.to_string()
                },
                verified: cap.chain_integrity,
            }
        })
        .collect()
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn ui(f: &mut Frame, app: &App) {
    let full = f.area();

    // Split into: header, main body, output panel, footer.
    let outer_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(10),   // pipeline + audit (left/right split)
            Constraint::Length(10), // output details
            Constraint::Length(3), // footer
        ])
        .split(full);

    render_header(f, outer_chunks[0], app);

    // Split the middle row into left (pipeline) and right (audit trail).
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
    let title_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let mut spans: Vec<Span> = vec![Span::styled("VERITAS Healthcare Demo    ", title_style)];

    let scenarios = [
        ("[1]", Scenario::DrugInteraction),
        ("[2]", Scenario::NoteSummarizer),
        ("[3]", Scenario::PatientQuery),
    ];

    for (key, scenario) in &scenarios {
        let is_selected = app.selected == *scenario;
        let style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        spans.push(Span::styled(format!("{} {}  ", key, scenario.name()), style));
    }

    let header_line = Line::from(spans);
    let header = Paragraph::new(header_line)
        .block(Block::default().borders(Borders::ALL).border_style(
            Style::default().fg(Color::DarkGray),
        ));
    f.render_widget(header, area);
}

fn render_pipeline(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let mut items: Vec<ListItem> = Vec::new();

    // State line.
    let state_str = if app.animating {
        "State: running..."
    } else if app.capture.is_some() {
        "State: complete"
    } else {
        "State: idle"
    };
    items.push(ListItem::new(Line::from(Span::styled(
        state_str,
        Style::default().fg(Color::DarkGray),
    ))));
    items.push(ListItem::new("")); // blank line

    let visible_count = app.animation_step.min(app.pipeline_steps.len());

    for (i, step) in app.pipeline_steps.iter().enumerate() {
        if i >= visible_count {
            break;
        }

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
            Span::styled(
                status_label,
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" — {}", step.detail),
                Style::default().fg(Color::Gray),
            ),
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
            let kind_color = match entry.kind.as_str() {
                "allow" | "genesis" => Color::Green,
                "deny" => Color::Red,
                "require-approval" => Color::Yellow,
                _ => Color::Gray,
            };
            let check = if entry.verified { " ✓" } else { " ✗" };
            let check_color = if entry.verified { Color::Green } else { Color::Red };

            let line = Line::from(vec![
                Span::styled(
                    format!("  #{}", entry.sequence),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(" ["),
                Span::styled(
                    entry.kind.as_str(),
                    Style::default().fg(kind_color).add_modifier(Modifier::BOLD),
                ),
                Span::raw("] "),
                Span::styled(
                    entry.hash_short.as_str(),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(check, Style::default().fg(check_color)),
            ]);
            items.push(ListItem::new(line));
        }

        // Chain integrity summary line.
        items.push(ListItem::new(""));
        let (integrity_label, integrity_color) = if app
            .capture
            .as_ref()
            .map(|c| c.chain_integrity)
            .unwrap_or(false)
        {
            ("  Chain integrity: VERIFIED", Color::Green)
        } else if app.capture.is_some() {
            ("  Chain integrity: FAILED", Color::Red)
        } else {
            ("", Color::DarkGray)
        };
        items.push(ListItem::new(Span::styled(
            integrity_label,
            Style::default()
                .fg(integrity_color)
                .add_modifier(Modifier::BOLD),
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
        Span::styled(
            verdict_label,
            Style::default()
                .fg(verdict_color)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Action / resource.
    lines.push(Line::from(vec![
        Span::styled("  Action:      ", Style::default().fg(Color::Gray)),
        Span::raw(cap.action.as_str()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Resource:    ", Style::default().fg(Color::Gray)),
        Span::raw(cap.resource.as_str()),
    ]));

    // Capability.
    let cap_color = if cap.capability_granted {
        Color::Green
    } else {
        Color::Red
    };
    let cap_granted_label = if cap.capability_granted { "[GRANTED]" } else { "[NOT GRANTED]" };
    lines.push(Line::from(vec![
        Span::styled("  Capability:  ", Style::default().fg(Color::Gray)),
        Span::raw(format!("{} ", cap.capability_name)),
        Span::styled(cap_granted_label, Style::default().fg(cap_color)),
    ]));

    lines.push(Line::from(""));

    // Output or denial reason.
    if let Some(output) = &cap.output {
        match app.selected {
            Scenario::DrugInteraction => {
                let severity = output.payload["result"]["severity"]
                    .as_str()
                    .unwrap_or("?");
                let recommendation = output.payload["recommendation"]
                    .as_str()
                    .unwrap_or("?");
                let severity_color = match severity {
                    "HIGH" => Color::Red,
                    "MEDIUM" => Color::Yellow,
                    "LOW" => Color::Green,
                    _ => Color::Gray,
                };
                lines.push(Line::from(vec![
                    Span::styled("  Severity:    ", Style::default().fg(Color::Gray)),
                    Span::styled(severity, Style::default().fg(severity_color).add_modifier(Modifier::BOLD)),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  Rec:         ", Style::default().fg(Color::Gray)),
                    Span::styled(
                        truncate(recommendation, 80),
                        Style::default().fg(Color::White),
                    ),
                ]));
            }
            Scenario::NoteSummarizer => {
                let summary = output.payload["summary"].as_str().unwrap_or("?");
                let note_count = output.payload["note_count"].as_u64().unwrap_or(0);
                lines.push(Line::from(vec![
                    Span::styled("  Notes:       ", Style::default().fg(Color::Gray)),
                    Span::raw(format!("{} clinical note(s) summarized", note_count)),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  Summary:     ", Style::default().fg(Color::Gray)),
                    Span::styled(
                        truncate(summary, 80),
                        Style::default().fg(Color::White),
                    ),
                ]));
            }
            Scenario::PatientQuery => {
                let cond_count = output.payload["conditions"]
                    .as_array()
                    .map(|a| a.len())
                    .unwrap_or(0);
                let patient_id = output.payload["patient_id"].as_str().unwrap_or("?");
                let consent = output.payload["ai_query_consent"]
                    .as_bool()
                    .unwrap_or(false);
                lines.push(Line::from(vec![
                    Span::styled("  Patient:     ", Style::default().fg(Color::Gray)),
                    Span::raw(patient_id),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  Consent:     ", Style::default().fg(Color::Gray)),
                    Span::styled(
                        if consent { "true" } else { "false" },
                        Style::default().fg(if consent { Color::Green } else { Color::Red }),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("  Conditions:  ", Style::default().fg(Color::Gray)),
                    Span::raw(format!("{} condition(s) returned", cond_count)),
                ]));
            }
        }
    } else {
        // No output — show denial / error reason.
        let reason = match &cap.policy_verdict {
            PolicyVerdict::Deny { reason } => reason.clone(),
            PolicyVerdict::RequireApproval { reason, .. } => reason.clone(),
            _ => cap
                .error
                .as_ref()
                .map(|e| e.to_string())
                .unwrap_or_default(),
        };
        if !reason.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  Reason:      ", Style::default().fg(Color::Gray)),
                Span::styled(
                    truncate(&reason, 80),
                    Style::default().fg(Color::Red),
                ),
            ]));
        }
    }

    let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    f.render_widget(paragraph, area);
}

fn render_footer(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let mut spans: Vec<Span> = vec![
        Span::styled(" [1-3] ", Style::default().fg(Color::Cyan)),
        Span::raw("Select scenario  "),
        Span::styled("[r] ", Style::default().fg(Color::Cyan)),
        Span::raw("Run  "),
    ];

    // Scenario-3-specific toggles.
    if app.selected == Scenario::PatientQuery {
        let consent_label = if app.consent_enabled {
            "consent: ON"
        } else {
            "consent: OFF"
        };
        let consent_color = if app.consent_enabled { Color::Green } else { Color::Red };
        spans.push(Span::styled("[c] ", Style::default().fg(Color::Cyan)));
        spans.push(Span::styled(
            consent_label,
            Style::default().fg(consent_color),
        ));
        spans.push(Span::raw("  "));

        let cap_label = if app.capability_enabled {
            "capability: ON"
        } else {
            "capability: OFF"
        };
        let cap_color = if app.capability_enabled { Color::Green } else { Color::Red };
        spans.push(Span::styled("[Tab] ", Style::default().fg(Color::Cyan)));
        spans.push(Span::styled(
            cap_label,
            Style::default().fg(cap_color),
        ));
        spans.push(Span::raw("  "));
    }

    spans.push(Span::styled("[q] ", Style::default().fg(Color::Cyan)));
    spans.push(Span::raw("Quit"));

    let footer = Paragraph::new(Line::from(spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
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
        format!("{}…", cut)
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
        // Best-effort terminal restore on panic.
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

        // Determine how long to wait before the next poll.  When animating, we
        // poll on short ticks so the animation feels smooth.
        let timeout = if app.animating {
            let elapsed = app.last_tick.elapsed();
            let tick_dur = Duration::from_millis(TICK_MS);
            tick_dur.saturating_sub(elapsed)
        } else {
            // When idle, long timeout to avoid burning CPU.
            Duration::from_millis(200)
        };

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    // Quit.
                    KeyCode::Char('q') => break,
                    KeyCode::Char('Q') => break,
                    // Ctrl-C also quits.
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,

                    // Scenario selection.
                    KeyCode::Char('1') => {
                        app.selected = Scenario::DrugInteraction;
                        app.capture = None;
                        app.pipeline_steps.clear();
                        app.audit_entries.clear();
                        app.animating = false;
                    }
                    KeyCode::Char('2') => {
                        app.selected = Scenario::NoteSummarizer;
                        app.capture = None;
                        app.pipeline_steps.clear();
                        app.audit_entries.clear();
                        app.animating = false;
                    }
                    KeyCode::Char('3') => {
                        app.selected = Scenario::PatientQuery;
                        app.capture = None;
                        app.pipeline_steps.clear();
                        app.audit_entries.clear();
                        app.animating = false;
                    }

                    // Run selected scenario.
                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        app.run();
                    }

                    // Toggle consent (Patient Query only).
                    KeyCode::Char('c') | KeyCode::Char('C')
                        if app.selected == Scenario::PatientQuery =>
                    {
                        app.consent_enabled = !app.consent_enabled;
                    }

                    // Toggle capability (Patient Query only).
                    KeyCode::Tab if app.selected == Scenario::PatientQuery => {
                        app.capability_enabled = !app.capability_enabled;
                    }

                    _ => {}
                }
            }
        }

        // Advance animation on each tick.
        if app.animating && app.last_tick.elapsed() >= Duration::from_millis(TICK_MS) {
            app.tick_animation();
            app.last_tick = Instant::now();
        }
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}
