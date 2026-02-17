//! Step-level execution results and audit records.
//!
//! `StepResult` is what the executor returns to the caller after each step.
//! `StepRecord` is what gets written to the audit log — one per step.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    agent::{AgentInput, AgentOutput, AgentState},
    policy::PolicyVerdict,
};

/// The outcome of a single executor step.
///
/// Callers pattern-match on this to decide what to do next:
/// - `Transitioned` → call step() again with the new state
/// - `Denied` → log the denial, surface to the user, stop
/// - `AwaitingApproval` → persist `suspended_state`, wait for approval, then resume
/// - `Complete` → the agent has finished; collect `final_state` and `output`
#[derive(Debug)]
pub enum StepResult {
    /// The step completed normally. The agent is not yet done.
    Transitioned {
        /// State the agent must receive on the next step.
        next_state: AgentState,
        /// The verified output from this step.
        output: AgentOutput,
    },

    /// A policy rule denied the action. The agent's proposal was never evaluated.
    Denied {
        /// The policy's denial reason.
        reason: String,
        /// The state at the time of denial, preserved for audit purposes.
        final_state: AgentState,
    },

    /// The action requires human approval before proceeding.
    ///
    /// The caller must persist `suspended_state` and resume execution
    /// after approval is obtained.
    AwaitingApproval {
        /// Why approval is required.
        reason: String,
        /// The role that must provide approval.
        approver_role: String,
        /// The full state at suspension time, to be restored when resuming.
        suspended_state: AgentState,
    },

    /// The agent reached a terminal state. Execution is finished.
    Complete {
        /// The terminal state.
        final_state: AgentState,
        /// The final output produced before termination.
        output: AgentOutput,
    },
}

/// An immutable record of one executor step, written to the audit log.
///
/// Every step — successful or not — produces exactly one `StepRecord`.
/// The audit writer appends this to its store; records are never modified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    /// The step counter from the agent state at the time of this record.
    pub step: u64,
    /// The input that triggered this step.
    pub input: AgentInput,
    /// The verdict the policy engine returned.
    pub verdict: PolicyVerdict,
    /// The agent's output, if the step produced one (absent on Deny/AwaitingApproval).
    pub output: Option<AgentOutput>,
    /// Wall-clock time the record was created (UTC).
    pub timestamp: DateTime<Utc>,
}
