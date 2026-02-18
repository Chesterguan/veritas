//! Scenario 5: Prior Authorization Workflow
//!
//! Demonstrates the full lifecycle of the `RequireApproval` policy verdict,
//! including the simulated approval handoff and downstream agent execution.
//!
//! Pipeline (both sub-cases share Step 1):
//!
//!   Step 1: ClinicalProposalAgent   → RequireApproval (high-cost procedure)
//!              ↓ [physician approves — simulated]
//!   Step 2: InsuranceEligibilityAgent → Allow (covered) OR Deny (not covered)
//!   Step 3: PASubmissionAgent        → Allow (only reached in Sub-case A)
//!
//! Sub-case A (happy path): procedure is covered → PA submitted successfully.
//! Sub-case B (denied):     procedure is not covered → Denied at eligibility.
//!
//! Key VERITAS enforcement points shown here:
//! - `RequireApproval` suspends execution structurally — `agent.propose()` is
//!   NEVER called until after physician sign-off is simulated.
//! - The approval token is carried in `AgentState.context` for audit traceability.
//! - Sub-case B's denial is audited before any agent logic runs.

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

// ── Policy TOML ───────────────────────────────────────────────────────────────

const PRIOR_AUTH_POLICY: &str = include_str!("../../policies/prior_auth.toml");

// ── Agent implementations ─────────────────────────────────────────────────────

/// Step 1: Proposes a high-cost clinical procedure.
///
/// The policy always returns `RequireApproval` for this agent, so
/// `propose()` is NEVER called in the demo — the executor short-circuits
/// before reaching it.  The impl is still required by the `Agent` trait.
pub struct ClinicalProposalAgent;

impl Agent for ClinicalProposalAgent {
    fn propose(&self, state: &AgentState, _input: &AgentInput) -> VeritasResult<AgentOutput> {
        Ok(AgentOutput {
            kind: "procedure-proposal".to_string(),
            payload: json!({
                "procedure": "cardiac-mri",
                "urgency": "routine",
                "proposed_by": state.agent_id.0,
                "proposed_at": "2026-02-18"
            }),
        })
    }

    fn transition(&self, state: &AgentState, _output: &AgentOutput) -> VeritasResult<AgentState> {
        Ok(AgentState {
            step: state.step + 1,
            phase: "awaiting-approval".to_string(),
            ..state.clone()
        })
    }

    fn required_capabilities(&self, _state: &AgentState, _input: &AgentInput) -> Vec<String> {
        // RequireApproval short-circuits before capability check — no capabilities needed.
        vec![]
    }

    fn describe_action(&self, _state: &AgentState, _input: &AgentInput) -> (String, String) {
        ("propose-procedure".to_string(), "high-cost-procedure".to_string())
    }

    fn is_terminal(&self, state: &AgentState) -> bool {
        state.phase == "complete"
    }
}

/// Step 2: Checks whether the procedure is covered by the patient's insurance.
///
/// `covered = true`  → `describe_action` returns resource `"insurance-records"`
///                     → policy allows (sub-case A).
/// `covered = false` → `describe_action` returns resource `"uncovered-procedure"`
///                     → policy denies (sub-case B).
///
/// This mirrors the consent-routing pattern used by `PatientQueryAgent`.
pub struct InsuranceEligibilityAgent {
    pub covered: bool,
}

impl Agent for InsuranceEligibilityAgent {
    fn propose(&self, _state: &AgentState, input: &AgentInput) -> VeritasResult<AgentOutput> {
        let procedure = input.payload["procedure"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

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
        Ok(AgentState {
            step: state.step + 1,
            phase: "complete".to_string(),
            ..state.clone()
        })
    }

    fn required_capabilities(&self, _state: &AgentState, _input: &AgentInput) -> Vec<String> {
        vec!["insurance.read".to_string()]
    }

    fn describe_action(&self, _state: &AgentState, _input: &AgentInput) -> (String, String) {
        let resource = if self.covered {
            "insurance-records"
        } else {
            "uncovered-procedure"
        };
        ("check-coverage".to_string(), resource.to_string())
    }

    fn is_terminal(&self, state: &AgentState) -> bool {
        state.phase == "complete"
    }
}

/// Step 3: Submits the prior authorization request to the insurance system.
///
/// Only reached in Sub-case A (procedure is covered and physician approved).
pub struct PASubmissionAgent;

impl Agent for PASubmissionAgent {
    fn propose(&self, state: &AgentState, input: &AgentInput) -> VeritasResult<AgentOutput> {
        let procedure = input.payload["procedure"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

        Ok(AgentOutput {
            kind: "pa-submission-result".to_string(),
            payload: json!({
                "pa_reference": "PA-2026-0218-4471",
                "status": "submitted",
                "procedure": procedure,
                "submitted_by": state.agent_id.0,
                "submitted_at": "2026-02-18"
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
        vec!["pa.write".to_string()]
    }

    fn describe_action(&self, _state: &AgentState, _input: &AgentInput) -> (String, String) {
        ("submit-pa".to_string(), "pa-system".to_string())
    }

    fn is_terminal(&self, state: &AgentState) -> bool {
        state.phase == "complete"
    }
}

// ── Output schemas ────────────────────────────────────────────────────────────

fn clinical_proposal_schema() -> OutputSchema {
    // Attached to the Step 1 executor. Verification never runs in practice
    // because RequireApproval returns before propose() is called. Still
    // required by Executor::new().
    OutputSchema {
        schema_id: "procedure-proposal-v1".to_string(),
        json_schema: json!({
            "type": "object",
            "required": ["procedure", "urgency"]
        }),
        rules: vec![
            VerificationRule {
                rule_id: "req-procedure".to_string(),
                description: "Proposal must name the requested procedure".to_string(),
                rule_type: VerificationRuleType::RequiredField {
                    field_path: "procedure".to_string(),
                },
            },
        ],
    }
}

fn insurance_eligibility_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "insurance-eligibility-v1".to_string(),
        json_schema: json!({
            "type": "object",
            "required": ["procedure", "covered"]
        }),
        rules: vec![
            VerificationRule {
                rule_id: "req-procedure".to_string(),
                description: "Eligibility result must name the procedure checked".to_string(),
                rule_type: VerificationRuleType::RequiredField {
                    field_path: "procedure".to_string(),
                },
            },
            VerificationRule {
                rule_id: "req-covered".to_string(),
                description: "Eligibility result must state whether procedure is covered".to_string(),
                rule_type: VerificationRuleType::RequiredField {
                    field_path: "covered".to_string(),
                },
            },
        ],
    }
}

fn pa_submission_schema() -> OutputSchema {
    OutputSchema {
        schema_id: "pa-submission-v1".to_string(),
        json_schema: json!({
            "type": "object",
            "required": ["pa_reference", "status"]
        }),
        rules: vec![
            VerificationRule {
                rule_id: "req-pa-reference".to_string(),
                description: "Submission result must include a PA reference number".to_string(),
                rule_type: VerificationRuleType::RequiredField {
                    field_path: "pa_reference".to_string(),
                },
            },
            VerificationRule {
                rule_id: "req-status".to_string(),
                description: "Submission result must include a status field".to_string(),
                rule_type: VerificationRuleType::RequiredField {
                    field_path: "status".to_string(),
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

// ── Shared Step 1 runner ──────────────────────────────────────────────────────

/// Run Step 1 (ClinicalProposalAgent) and return the suspended state and
/// approver role.  Prints the RequireApproval outcome and simulates approval.
///
/// Returns `(approval_token, approver_role)` for use in subsequent steps.
fn run_step1_and_simulate_approval() -> VeritasResult<(String, String)> {
    let policy = TomlPolicyEngine::from_toml_str(PRIOR_AUTH_POLICY)?;
    let exec_id = ExecutionId::new();
    let audit = Arc::new(InMemoryAuditWriter::new(exec_id.0.to_string()));
    let agent = ClinicalProposalAgent;

    let state = AgentState {
        agent_id: AgentId("clinical-proposal-agent".to_string()),
        execution_id: exec_id.clone(),
        phase: "active".to_string(),
        context: serde_json::Value::Null,
        step: 0,
    };

    let caps = CapabilitySet::default(); // no capabilities needed for RequireApproval path

    let input = AgentInput {
        kind: "procedure-proposal-request".to_string(),
        payload: json!({ "procedure": "cardiac-mri", "urgency": "routine" }),
    };

    let executor = Executor::new(
        Box::new(policy),
        Box::new(ArcAudit(Arc::clone(&audit))),
        Box::new(SchemaVerifier::new()),
        clinical_proposal_schema(),
    );

    let result = executor.step(&agent, state, input, &caps)?;

    match result {
        StepResult::AwaitingApproval { reason, approver_role, .. } => {
            println!("  Step 1 — ClinicalProposalAgent");
            println!("  Action:         propose-procedure | Resource: high-cost-procedure");
            println!("  Policy verdict: RequireApproval");
            println!("  Reason:         {}", reason);
            println!("  Approver role:  {}", approver_role);
            let log = audit.export_log();
            println!(
                "  Audit chain:    {} ({} event(s))",
                if audit.verify_integrity() { "VERIFIED" } else { "FAILED" },
                log.events.len()
            );
            println!();
            println!("  *** EXECUTION PAUSED — awaiting {} approval ***", approver_role);
            println!();
            println!("  [Simulating physician approval...]");

            let token = "PHY-APPROVE-2026-0218".to_string();
            println!("  Approval token: {}", token);
            println!("  Approved by:    {}", approver_role);
            println!("  Approved at:    2026-02-18T10:30:00Z");
            println!();

            Ok((token, approver_role))
        }
        other => {
            println!("  UNEXPECTED Step 1 result: {:?}", other);
            Err(veritas_contracts::error::VeritasError::StateMachineError {
                reason: "expected AwaitingApproval from Step 1".to_string(),
            })
        }
    }
}

// ── Scenario runner ───────────────────────────────────────────────────────────

/// Run Scenario 5: Prior Authorization Workflow.
///
/// Sub-case A: Full PA approval — procedure is covered, physician approved,
/// PA submitted successfully.
///
/// Sub-case B: PA denied — procedure is not covered by the patient's plan.
pub fn run_scenario() -> VeritasResult<()> {
    println!("=== Scenario 5: Prior Authorization Workflow ===");
    println!();
    println!("  Procedure:  cardiac-mri (urgency: routine)");
    println!("  Patient:    patient-101");
    println!();

    // ── Sub-case A: Full PA approval ──────────────────────────────────────────

    println!("  ── Sub-case A: Full PA approval (happy path) ──");
    println!();

    let (approval_token, approver_role) = run_step1_and_simulate_approval()?;

    // Step 2 — InsuranceEligibilityAgent (covered = true → Allow)
    {
        println!("  Step 2 — InsuranceEligibilityAgent [covered=true]");
        println!("  Action:     check-coverage | Resource: insurance-records");
        println!("  Capability: insurance.read [GRANTED]");

        let policy = TomlPolicyEngine::from_toml_str(PRIOR_AUTH_POLICY)?;
        let exec_id = ExecutionId::new();
        let audit = Arc::new(InMemoryAuditWriter::new(exec_id.0.to_string()));
        let agent = InsuranceEligibilityAgent { covered: true };

        // Carry the approval token in state.context for audit traceability.
        let state = AgentState {
            agent_id: AgentId("insurance-eligibility-agent".to_string()),
            execution_id: exec_id.clone(),
            phase: "active".to_string(),
            context: json!({
                "approval_token": approval_token,
                "approved_by": approver_role
            }),
            step: 0,
        };

        let mut caps = CapabilitySet::default();
        caps.grant(Capability::new("insurance.read"));

        let input = AgentInput {
            kind: "insurance-eligibility-request".to_string(),
            payload: json!({ "procedure": "cardiac-mri" }),
        };

        let executor = Executor::new(
            Box::new(policy),
            Box::new(ArcAudit(Arc::clone(&audit))),
            Box::new(SchemaVerifier::new()),
            insurance_eligibility_schema(),
        );

        let result = executor.step(&agent, state, input, &caps)?;

        let step2_output = match result {
            StepResult::Complete { ref output, .. } | StepResult::Transitioned { ref output, .. } => {
                let plan = output.payload["plan_name"].as_str().unwrap_or("?");
                let copay = output.payload["copay_usd"].as_u64().unwrap_or(0);
                println!("  Policy verdict: Allow");
                println!("  Capability:     PASS");
                println!("  Verification:   PASS");
                println!("  Coverage:       COVERED ({}, copay ${copay})", plan);
                let log = audit.export_log();
                println!(
                    "  Audit chain:    {} ({} event(s))",
                    if audit.verify_integrity() { "VERIFIED" } else { "FAILED" },
                    log.events.len()
                );
                output.clone()
            }
            StepResult::Denied { reason, .. } => {
                println!("  DENIED: {}", reason);
                println!();
                return Ok(());
            }
            other => {
                println!("  UNEXPECTED: {:?}", other);
                return Ok(());
            }
        };

        println!();

        // Step 3 — PASubmissionAgent
        println!("  Step 3 — PASubmissionAgent");
        println!("  Action:     submit-pa | Resource: pa-system");
        println!("  Capability: pa.write [GRANTED]");

        let policy_3 = TomlPolicyEngine::from_toml_str(PRIOR_AUTH_POLICY)?;
        let exec_id_3 = ExecutionId::new();
        let audit_3 = Arc::new(InMemoryAuditWriter::new(exec_id_3.0.to_string()));
        let agent_3 = PASubmissionAgent;

        let state_3 = AgentState {
            agent_id: AgentId("pa-submission-agent".to_string()),
            execution_id: exec_id_3.clone(),
            phase: "active".to_string(),
            context: serde_json::Value::Null,
            step: 0,
        };

        let mut caps_3 = CapabilitySet::default();
        caps_3.grant(Capability::new("pa.write"));

        let input_3 = AgentInput {
            kind: "pa-submission-request".to_string(),
            payload: step2_output.payload.clone(),
        };

        let executor_3 = Executor::new(
            Box::new(policy_3),
            Box::new(ArcAudit(Arc::clone(&audit_3))),
            Box::new(SchemaVerifier::new()),
            pa_submission_schema(),
        );

        let result_3 = executor_3.step(&agent_3, state_3, input_3, &caps_3)?;

        match result_3 {
            StepResult::Complete { output, .. } | StepResult::Transitioned { output, .. } => {
                let pa_ref = output.payload["pa_reference"].as_str().unwrap_or("?");
                let status = output.payload["status"].as_str().unwrap_or("?");
                println!("  Policy verdict: Allow");
                println!("  Capability:     PASS");
                println!("  Verification:   PASS");
                println!("  PA Reference:   {}", pa_ref);
                println!("  Status:         {}", status);
                let log_3 = audit_3.export_log();
                println!(
                    "  Audit chain:    {} ({} event(s))",
                    if audit_3.verify_integrity() { "VERIFIED" } else { "FAILED" },
                    log_3.events.len()
                );
            }
            other => {
                println!("  UNEXPECTED: {:?}", other);
            }
        }
    }

    println!();
    println!("  Sub-case A complete: PA submitted successfully.");
    println!();
    println!("  ────────────────────────────────────────────────────");
    println!();

    // ── Sub-case B: Procedure not covered → Denied at eligibility ────────────

    println!("  ── Sub-case B: PA denied — procedure not covered ──");
    println!();

    let (approval_token_b, approver_role_b) = run_step1_and_simulate_approval()?;

    // Step 2 — InsuranceEligibilityAgent (covered = false → Deny)
    {
        println!("  Step 2 — InsuranceEligibilityAgent [covered=false]");
        println!("  Action:     check-coverage | Resource: uncovered-procedure");
        println!("  Capability: insurance.read [GRANTED]");

        let policy = TomlPolicyEngine::from_toml_str(PRIOR_AUTH_POLICY)?;
        let exec_id = ExecutionId::new();
        let audit = Arc::new(InMemoryAuditWriter::new(exec_id.0.to_string()));
        let agent = InsuranceEligibilityAgent { covered: false };

        let state = AgentState {
            agent_id: AgentId("insurance-eligibility-agent".to_string()),
            execution_id: exec_id.clone(),
            phase: "active".to_string(),
            context: json!({
                "approval_token": approval_token_b,
                "approved_by": approver_role_b
            }),
            step: 0,
        };

        let mut caps = CapabilitySet::default();
        caps.grant(Capability::new("insurance.read"));

        let input = AgentInput {
            kind: "insurance-eligibility-request".to_string(),
            payload: json!({ "procedure": "cardiac-mri" }),
        };

        let executor = Executor::new(
            Box::new(policy),
            Box::new(ArcAudit(Arc::clone(&audit))),
            Box::new(SchemaVerifier::new()),
            insurance_eligibility_schema(),
        );

        let result = executor.step(&agent, state, input, &caps)?;

        match result {
            StepResult::Denied { reason, .. } => {
                println!("  Policy verdict: Deny");
                println!("  Reason:         {}", reason);
                println!("  Agent propose(): NOT called (blocked before capability check)");
                let log = audit.export_log();
                println!(
                    "  Audit chain:    {} ({} denial event(s))",
                    if audit.verify_integrity() { "VERIFIED" } else { "FAILED" },
                    log.events.len()
                );
                println!("  RESULT:         PA denied at eligibility — no Step 3.");
            }
            other => {
                println!("  UNEXPECTED: {:?}", other);
            }
        }
    }

    println!();
    println!("  Sub-case B complete: PA denied at eligibility check.");
    println!();
    println!("  Scenario 5 complete.");
    println!();

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use veritas_contracts::{
        policy::{PolicyContext, PolicyVerdict},
    };
    use veritas_core::traits::PolicyEngine;
    use veritas_policy::engine::TomlPolicyEngine;

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

    /// ClinicalProposalAgent always triggers RequireApproval from the policy.
    #[test]
    fn test_clinical_proposal_requires_approval() {
        let policy = TomlPolicyEngine::from_toml_str(PRIOR_AUTH_POLICY).unwrap();
        let ctx = make_policy_ctx("propose-procedure", "high-cost-procedure", &[]);
        let verdict = policy.evaluate(&ctx).unwrap();
        match verdict {
            PolicyVerdict::RequireApproval { approver_role, .. } => {
                assert_eq!(approver_role, "attending-physician");
            }
            other => panic!("expected RequireApproval, got {:?}", other),
        }
    }

    /// InsuranceEligibilityAgent is allowed when procedure is covered.
    #[test]
    fn test_insurance_eligibility_allow_when_covered() {
        let policy = TomlPolicyEngine::from_toml_str(PRIOR_AUTH_POLICY).unwrap();
        let ctx = make_policy_ctx("check-coverage", "insurance-records", &["insurance.read"]);
        let verdict = policy.evaluate(&ctx).unwrap();
        assert_eq!(verdict, PolicyVerdict::Allow);
    }

    /// InsuranceEligibilityAgent is denied when procedure is not covered.
    #[test]
    fn test_insurance_eligibility_deny_when_uncovered() {
        let policy = TomlPolicyEngine::from_toml_str(PRIOR_AUTH_POLICY).unwrap();
        let ctx = make_policy_ctx("check-coverage", "uncovered-procedure", &["insurance.read"]);
        let verdict = policy.evaluate(&ctx).unwrap();
        match verdict {
            PolicyVerdict::Deny { reason } => {
                assert!(
                    reason.contains("not covered"),
                    "deny reason should mention coverage: {}",
                    reason
                );
            }
            other => panic!("expected Deny, got {:?}", other),
        }
    }

    /// PASubmissionAgent is allowed when pa.write capability is present.
    #[test]
    fn test_pa_submission_allowed() {
        let policy = TomlPolicyEngine::from_toml_str(PRIOR_AUTH_POLICY).unwrap();
        let ctx = make_policy_ctx("submit-pa", "pa-system", &["pa.write"]);
        let verdict = policy.evaluate(&ctx).unwrap();
        assert_eq!(verdict, PolicyVerdict::Allow);
    }

    /// PASubmissionAgent is denied when pa.write capability is missing.
    #[test]
    fn test_pa_submission_denied_without_capability() {
        let policy = TomlPolicyEngine::from_toml_str(PRIOR_AUTH_POLICY).unwrap();
        let ctx = make_policy_ctx("submit-pa", "pa-system", &[]);
        let verdict = policy.evaluate(&ctx).unwrap();
        match verdict {
            PolicyVerdict::Deny { .. } => {}
            other => panic!("expected Deny for missing pa.write capability, got {:?}", other),
        }
    }

    /// InsuranceEligibilityAgent routes to correct resource based on covered field.
    #[test]
    fn test_eligibility_agent_resource_routing() {
        use veritas_contracts::agent::{AgentId, AgentInput, AgentState, ExecutionId};

        let make_state = |id: &str| AgentState {
            agent_id: AgentId(id.to_string()),
            execution_id: ExecutionId::new(),
            phase: "active".to_string(),
            context: serde_json::Value::Null,
            step: 0,
        };

        let input = AgentInput {
            kind: "test".to_string(),
            payload: json!({}),
        };

        let covered_agent = InsuranceEligibilityAgent { covered: true };
        let (_, resource_a) = covered_agent.describe_action(&make_state("a"), &input);
        assert_eq!(resource_a, "insurance-records");

        let uncovered_agent = InsuranceEligibilityAgent { covered: false };
        let (_, resource_b) = uncovered_agent.describe_action(&make_state("b"), &input);
        assert_eq!(resource_b, "uncovered-procedure");
    }
}
