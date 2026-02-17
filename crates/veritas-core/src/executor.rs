//! The VERITAS executor: the deterministic policy-bound step runner.
//!
//! The executor enforces the VERITAS execution model:
//!
//!   State → Policy → Capability → [Agent::propose] → Verify → Transition → Audit
//!
//! The security invariant is absolute: `Agent::propose()` is NEVER called
//! unless `PolicyEngine::evaluate()` returns `PolicyVerdict::Allow` AND all
//! required capabilities are present. This is enforced structurally — the
//! code path to `propose()` is only reachable after both checks pass.

use chrono::Utc;
use tracing::{debug, info, warn};

use veritas_contracts::{
    agent::{AgentState, AgentInput},
    capability::{Capability, CapabilitySet},
    error::{VeritasError, VeritasResult},
    execution::{StepRecord, StepResult},
    policy::{PolicyContext, PolicyVerdict},
    verify::OutputSchema,
};

use crate::traits::{Agent, AuditWriter, PolicyEngine, Verifier};

/// The central executor that drives a single agent execution.
///
/// Construct one executor per agent execution (i.e. per `ExecutionId`).
/// The executor owns the trusted components — policy, audit, verifier — and
/// enforces the pipeline ordering on every call to `step()`.
pub struct Executor {
    policy: Box<dyn PolicyEngine>,
    audit: Box<dyn AuditWriter>,
    verifier: Box<dyn Verifier>,
    schema: OutputSchema,
}

impl Executor {
    /// Create a new executor with the given trusted components and output schema.
    pub fn new(
        policy: Box<dyn PolicyEngine>,
        audit: Box<dyn AuditWriter>,
        verifier: Box<dyn Verifier>,
        schema: OutputSchema,
    ) -> Self {
        Self { policy, audit, verifier, schema }
    }

    /// Execute one step of the agent's state machine.
    ///
    /// # Pipeline
    ///
    /// 1. Build `PolicyContext` from `agent.describe_action()`
    /// 2. Call `policy.evaluate()`:
    ///    - `Deny` → audit the denial, return `StepResult::Denied`
    ///    - `RequireApproval` → audit, return `StepResult::AwaitingApproval`
    ///    - `RequireVerification` / `Allow` → continue
    /// 3. Check that the agent holds all `required_capabilities()`; if not,
    ///    audit a synthetic denial and return `VeritasError::CapabilityMissing`
    /// 4. Call `agent.propose()` — **only reachable after steps 2 & 3 pass**
    /// 5. Call `verifier.verify()`; if failed, return `VeritasError::VerificationFailed`
    /// 6. Call `agent.transition()` to advance state
    /// 7. Audit the completed step
    /// 8. If `agent.is_terminal()`, finalize the audit and return `StepResult::Complete`
    /// 9. Otherwise return `StepResult::Transitioned`
    ///
    /// # Errors
    ///
    /// Returns `Err` for capability failures, verification failures, audit
    /// write failures, and agent state machine errors. Policy `Deny` and
    /// `RequireApproval` are NOT errors — they are valid `StepResult` variants.
    pub fn step(
        &self,
        agent: &dyn Agent,
        state: AgentState,
        input: AgentInput,
        capabilities: &CapabilitySet,
    ) -> VeritasResult<StepResult> {
        let execution_id = state.execution_id.0.to_string();
        let step_num = state.step;

        debug!(
            execution_id = %execution_id,
            step = step_num,
            phase = %state.phase,
            input_kind = %input.kind,
            "executor step starting"
        );

        // ── Step 1: Describe the action the agent wants to take ──────────────
        let (action, resource) = agent.describe_action(&state, &input);

        let policy_ctx = PolicyContext {
            agent_id: state.agent_id.0.clone(),
            execution_id: execution_id.clone(),
            current_phase: state.phase.clone(),
            action: action.clone(),
            resource: resource.clone(),
            capabilities: capabilities.all().map(|c| c.0.clone()).collect(),
            metadata: serde_json::Value::Null,
        };

        // ── Step 2: Policy evaluation ────────────────────────────────────────
        //
        // This is the primary trust gate. No agent logic runs until Allow.
        let verdict = self.policy.evaluate(&policy_ctx)?;

        match &verdict {
            PolicyVerdict::Deny { reason } => {
                warn!(
                    execution_id = %execution_id,
                    step = step_num,
                    reason = %reason,
                    "policy denied action"
                );

                // Audit the denial so every denied step is on record.
                let record = StepRecord {
                    step: step_num,
                    input,
                    verdict: verdict.clone(),
                    output: None,
                    timestamp: Utc::now(),
                };
                self.audit.write(&record)?;

                return Ok(StepResult::Denied {
                    reason: reason.clone(),
                    final_state: state,
                });
            }

            PolicyVerdict::RequireApproval { reason, approver_role } => {
                info!(
                    execution_id = %execution_id,
                    step = step_num,
                    approver_role = %approver_role,
                    "execution suspended awaiting approval"
                );

                let record = StepRecord {
                    step: step_num,
                    input,
                    verdict: verdict.clone(),
                    output: None,
                    timestamp: Utc::now(),
                };
                self.audit.write(&record)?;

                return Ok(StepResult::AwaitingApproval {
                    reason: reason.clone(),
                    approver_role: approver_role.clone(),
                    suspended_state: state,
                });
            }

            // Allow and RequireVerification both proceed to capability check.
            PolicyVerdict::Allow | PolicyVerdict::RequireVerification { .. } => {
                debug!(
                    execution_id = %execution_id,
                    step = step_num,
                    "policy allowed action, checking capabilities"
                );
            }
        }

        // ── Step 3: Capability check ─────────────────────────────────────────
        //
        // Even after Allow, the agent must hold every declared capability.
        // This enforces principle of least privilege at the runtime level.
        let required = agent.required_capabilities(&state, &input);
        for cap_name in &required {
            let cap = Capability::new(cap_name.as_str());
            if !capabilities.has(&cap) {
                warn!(
                    execution_id = %execution_id,
                    step = step_num,
                    capability = %cap_name,
                    action = %action,
                    "capability missing, step denied"
                );

                // Audit the capability failure as a synthetic denial.
                let denial_verdict = PolicyVerdict::Deny {
                    reason: format!(
                        "capability '{}' required for action '{}' is not granted",
                        cap_name, action
                    ),
                };
                let record = StepRecord {
                    step: step_num,
                    input,
                    verdict: denial_verdict,
                    output: None,
                    timestamp: Utc::now(),
                };
                self.audit.write(&record)?;

                return Err(VeritasError::CapabilityMissing {
                    capability: cap_name.clone(),
                    action: action.clone(),
                });
            }
        }

        // ── Step 4: Agent proposal ───────────────────────────────────────────
        //
        // Only reachable if policy returned Allow AND all capabilities present.
        // This is the ONLY call site for agent.propose() in the runtime.
        debug!(
            execution_id = %execution_id,
            step = step_num,
            "capabilities verified, calling agent.propose()"
        );
        let output = agent.propose(&state, &input)?;

        // ── Step 5: Output verification ──────────────────────────────────────
        //
        // The verifier inspects the raw LLM/agent output before it touches state.
        let report = self.verifier.verify(&output, &self.schema)?;
        if !report.passed {
            let failure_summary = report
                .failures
                .iter()
                .map(|f| format!("[{}] {}", f.rule_id, f.message))
                .collect::<Vec<_>>()
                .join("; ");

            warn!(
                execution_id = %execution_id,
                step = step_num,
                failures = %failure_summary,
                "output verification failed"
            );
            return Err(VeritasError::VerificationFailed {
                reason: failure_summary,
            });
        }

        // ── Step 6: State transition ─────────────────────────────────────────
        let next_state = agent.transition(&state, &output)?;

        // ── Step 7: Audit the completed step ─────────────────────────────────
        let record = StepRecord {
            step: step_num,
            input,
            verdict,
            output: Some(output.clone()),
            timestamp: Utc::now(),
        };
        self.audit.write(&record)?;

        // ── Steps 8 & 9: Terminal check ──────────────────────────────────────
        if agent.is_terminal(&next_state) {
            info!(
                execution_id = %execution_id,
                step = step_num,
                "agent reached terminal state, finalizing audit"
            );
            self.audit.finalize(&execution_id)?;
            Ok(StepResult::Complete {
                final_state: next_state,
                output,
            })
        } else {
            Ok(StepResult::Transitioned {
                next_state,
                output,
            })
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use veritas_contracts::{
        agent::{AgentId, AgentInput, AgentOutput, AgentState, ExecutionId},
        capability::CapabilitySet,
        error::{VeritasError, VeritasResult},
        execution::{StepRecord, StepResult},
        policy::{PolicyContext, PolicyVerdict},
        verify::{OutputSchema, VerificationFailure, VerificationReport},
    };

    use crate::traits::{Agent, AuditWriter, PolicyEngine, Verifier};

    use super::Executor;

    // ── Mock helpers ─────────────────────────────────────────────────────────

    fn make_state(phase: &str) -> AgentState {
        AgentState {
            agent_id: AgentId("test-agent".to_string()),
            execution_id: ExecutionId::new(),
            phase: phase.to_string(),
            context: serde_json::Value::Null,
            step: 0,
        }
    }

    fn make_input() -> AgentInput {
        AgentInput {
            kind: "user_message".to_string(),
            payload: serde_json::json!({ "text": "hello" }),
        }
    }

    fn make_schema() -> OutputSchema {
        OutputSchema {
            schema_id: "test-schema-v1".to_string(),
            json_schema: serde_json::Value::Null,
            rules: vec![],
        }
    }

    /// A policy that always returns a pre-configured verdict.
    struct MockPolicy {
        verdict: PolicyVerdict,
    }

    impl PolicyEngine for MockPolicy {
        fn evaluate(&self, _ctx: &PolicyContext) -> VeritasResult<PolicyVerdict> {
            Ok(self.verdict.clone())
        }
    }

    /// An audit writer that records every call for later inspection.
    struct MockAudit {
        records: Arc<Mutex<Vec<StepRecord>>>,
        finalized: Arc<Mutex<Vec<String>>>,
    }

    impl MockAudit {
        fn new() -> Self {
            Self {
                records: Arc::new(Mutex::new(vec![])),
                finalized: Arc::new(Mutex::new(vec![])),
            }
        }
    }

    impl AuditWriter for MockAudit {
        fn write(&self, record: &StepRecord) -> VeritasResult<()> {
            self.records.lock().unwrap().push(record.clone());
            Ok(())
        }

        fn finalize(&self, execution_id: &str) -> VeritasResult<()> {
            self.finalized.lock().unwrap().push(execution_id.to_string());
            Ok(())
        }
    }

    /// A verifier that can be configured to pass or fail.
    struct MockVerifier {
        pass: bool,
    }

    impl Verifier for MockVerifier {
        fn verify(
            &self,
            _output: &AgentOutput,
            _schema: &OutputSchema,
        ) -> VeritasResult<VerificationReport> {
            if self.pass {
                Ok(VerificationReport { passed: true, failures: vec![] })
            } else {
                Ok(VerificationReport {
                    passed: false,
                    failures: vec![VerificationFailure {
                        rule_id: "required-field".to_string(),
                        message: "field 'patient_id' is missing".to_string(),
                    }],
                })
            }
        }
    }

    /// An agent that tracks how many times propose() was called.
    struct MockAgent {
        propose_count: Arc<Mutex<u32>>,
        /// When true, is_terminal() returns true after the first propose.
        terminal: bool,
    }

    impl MockAgent {
        fn new() -> Self {
            Self {
                propose_count: Arc::new(Mutex::new(0)),
                terminal: false,
            }
        }

        fn terminal() -> Self {
            Self {
                propose_count: Arc::new(Mutex::new(0)),
                terminal: true,
            }
        }
    }

    impl Agent for MockAgent {
        fn propose(&self, _state: &AgentState, _input: &AgentInput) -> VeritasResult<AgentOutput> {
            *self.propose_count.lock().unwrap() += 1;
            Ok(AgentOutput {
                kind: "response".to_string(),
                payload: serde_json::json!({ "text": "ok" }),
            })
        }

        fn transition(
            &self,
            state: &AgentState,
            _output: &AgentOutput,
        ) -> VeritasResult<AgentState> {
            Ok(AgentState {
                step: state.step + 1,
                phase: "next".to_string(),
                ..state.clone()
            })
        }

        fn required_capabilities(
            &self,
            _state: &AgentState,
            _input: &AgentInput,
        ) -> Vec<String> {
            vec![]
        }

        fn describe_action(
            &self,
            _state: &AgentState,
            _input: &AgentInput,
        ) -> (String, String) {
            ("respond".to_string(), "user".to_string())
        }

        fn is_terminal(&self, _state: &AgentState) -> bool {
            self.terminal
        }
    }

    /// An agent that requires a specific capability.
    struct CapRequiringAgent {
        required: String,
    }

    impl Agent for CapRequiringAgent {
        fn propose(&self, _state: &AgentState, _input: &AgentInput) -> VeritasResult<AgentOutput> {
            panic!("propose() must not be called when capability is missing");
        }

        fn transition(
            &self,
            state: &AgentState,
            _output: &AgentOutput,
        ) -> VeritasResult<AgentState> {
            Ok(AgentState { step: state.step + 1, ..state.clone() })
        }

        fn required_capabilities(
            &self,
            _state: &AgentState,
            _input: &AgentInput,
        ) -> Vec<String> {
            vec![self.required.clone()]
        }

        fn describe_action(
            &self,
            _state: &AgentState,
            _input: &AgentInput,
        ) -> (String, String) {
            ("read_phi".to_string(), "patient_record".to_string())
        }

        fn is_terminal(&self, _state: &AgentState) -> bool {
            false
        }
    }

    // ── Test cases ────────────────────────────────────────────────────────────

    /// Core security test: a policy Deny must prevent agent.propose() from
    /// being called under any circumstances.
    #[test]
    fn test_policy_deny_blocks_agent() {
        let agent = MockAgent::new();

        // Capture the propose_count handle before moving agent into the step call.
        let propose_count = agent.propose_count.clone();

        let audit = MockAudit::new();
        let audit_records = audit.records.clone();

        let executor = Executor::new(
            Box::new(MockPolicy { verdict: PolicyVerdict::Deny { reason: "not allowed".to_string() } }),
            Box::new(audit),
            Box::new(MockVerifier { pass: true }),
            make_schema(),
        );

        let caps = CapabilitySet::default();
        let result = executor.step(&agent, make_state("active"), make_input(), &caps).unwrap();

        // Proposal must NEVER have been called.
        assert_eq!(*propose_count.lock().unwrap(), 0, "propose() must not be called on Deny");

        // Result must be Denied.
        assert!(matches!(result, StepResult::Denied { .. }));

        // Denial must be audited.
        assert_eq!(audit_records.lock().unwrap().len(), 1);
    }

    /// RequireApproval suspends execution and never calls agent.propose().
    #[test]
    fn test_require_approval_suspends() {
        let agent = MockAgent::new();
        let propose_count = agent.propose_count.clone();

        let executor = Executor::new(
            Box::new(MockPolicy {
                verdict: PolicyVerdict::RequireApproval {
                    reason: "high risk action".to_string(),
                    approver_role: "attending_physician".to_string(),
                },
            }),
            Box::new(MockAudit::new()),
            Box::new(MockVerifier { pass: true }),
            make_schema(),
        );

        let caps = CapabilitySet::default();
        let result = executor.step(&agent, make_state("active"), make_input(), &caps).unwrap();

        assert_eq!(*propose_count.lock().unwrap(), 0, "propose() must not be called on RequireApproval");

        match result {
            StepResult::AwaitingApproval { reason, approver_role, .. } => {
                assert_eq!(reason, "high risk action");
                assert_eq!(approver_role, "attending_physician");
            }
            other => panic!("expected AwaitingApproval, got {:?}", other),
        }
    }

    /// A missing capability blocks the step even when policy says Allow.
    #[test]
    fn test_capability_missing_blocks() {
        let agent = CapRequiringAgent { required: "phi:read".to_string() };

        let executor = Executor::new(
            Box::new(MockPolicy { verdict: PolicyVerdict::Allow }),
            Box::new(MockAudit::new()),
            Box::new(MockVerifier { pass: true }),
            make_schema(),
        );

        // No capabilities granted.
        let caps = CapabilitySet::default();
        let result = executor.step(&agent, make_state("active"), make_input(), &caps);

        match result {
            Err(VeritasError::CapabilityMissing { capability, .. }) => {
                assert_eq!(capability, "phi:read");
            }
            other => panic!("expected CapabilityMissing, got {:?}", other),
        }
    }

    /// A successful step: policy allows, capabilities present, verifier passes.
    /// Audit must contain one record. Result must be Transitioned.
    #[test]
    fn test_successful_step() {
        let agent = MockAgent::new();
        let propose_count = agent.propose_count.clone();
        let audit = MockAudit::new();
        let audit_records = audit.records.clone();

        let executor = Executor::new(
            Box::new(MockPolicy { verdict: PolicyVerdict::Allow }),
            Box::new(audit),
            Box::new(MockVerifier { pass: true }),
            make_schema(),
        );

        let caps = CapabilitySet::default();
        let result = executor.step(&agent, make_state("active"), make_input(), &caps).unwrap();

        // propose() must have been called exactly once.
        assert_eq!(*propose_count.lock().unwrap(), 1);

        // One audit record for the completed step.
        assert_eq!(audit_records.lock().unwrap().len(), 1);

        match result {
            StepResult::Transitioned { next_state, output } => {
                assert_eq!(next_state.step, 1);
                assert_eq!(next_state.phase, "next");
                assert_eq!(output.kind, "response");
            }
            other => panic!("expected Transitioned, got {:?}", other),
        }
    }

    /// When is_terminal() returns true, the executor returns Complete and
    /// finalizes the audit.
    #[test]
    fn test_terminal_state() {
        let agent = MockAgent::terminal();
        let audit = MockAudit::new();
        let was_finalized = audit.finalized.clone();

        let executor = Executor::new(
            Box::new(MockPolicy { verdict: PolicyVerdict::Allow }),
            Box::new(audit),
            Box::new(MockVerifier { pass: true }),
            make_schema(),
        );

        let caps = CapabilitySet::default();
        let result = executor.step(&agent, make_state("active"), make_input(), &caps).unwrap();

        match result {
            StepResult::Complete { final_state, output } => {
                assert_eq!(output.kind, "response");
                // The transitioned state has step = 1.
                assert_eq!(final_state.step, 1);
            }
            other => panic!("expected Complete, got {:?}", other),
        }

        // audit.finalize() must have been called.
        assert!(!was_finalized.lock().unwrap().is_empty(), "audit must be finalized on Complete");
    }

    /// When the verifier returns a failing report, the step returns
    /// VerificationFailed and state does NOT advance.
    #[test]
    fn test_verification_failure() {
        let agent = MockAgent::new();
        let propose_count = agent.propose_count.clone();

        let executor = Executor::new(
            Box::new(MockPolicy { verdict: PolicyVerdict::Allow }),
            Box::new(MockAudit::new()),
            Box::new(MockVerifier { pass: false }),
            make_schema(),
        );

        let caps = CapabilitySet::default();
        let result = executor.step(&agent, make_state("active"), make_input(), &caps);

        // propose() was called (policy allowed it), but verification blocked delivery.
        assert_eq!(*propose_count.lock().unwrap(), 1);

        match result {
            Err(VeritasError::VerificationFailed { reason }) => {
                assert!(reason.contains("patient_id"), "reason should mention the failed rule: {}", reason);
            }
            other => panic!("expected VerificationFailed, got {:?}", other),
        }
    }
}
