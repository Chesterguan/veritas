//! Policy verdict and evaluation context types.
//!
//! The policy engine consumes a `PolicyContext` and produces a `PolicyVerdict`.
//! VERITAS is deny-by-default: any verdict other than `Allow` blocks the agent.

use serde::{Deserialize, Serialize};

/// The decision emitted by the policy engine for a single agent action.
///
/// All variants except `Allow` prevent `agent.propose()` from being called.
/// This is the core security guarantee of the VERITAS runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyVerdict {
    /// The action is permitted. Execution continues.
    Allow,

    /// The action is permanently denied.
    Deny {
        /// Human-readable explanation, written to the audit log.
        reason: String,
    },

    /// The action is suspended pending human approval.
    ///
    /// The executor returns `StepResult::AwaitingApproval` and the caller
    /// must resume with an approval input (kind = "approval_granted") after
    /// obtaining sign-off from the specified role.
    RequireApproval {
        /// Why approval is required.
        reason: String,
        /// The role that must approve (e.g. "attending_physician", "compliance_officer").
        approver_role: String,
    },

    /// The action requires an external verification check before proceeding.
    ///
    /// The check referenced by `check_id` is resolved by the verifier.
    RequireVerification {
        /// Identifier for the verification check to run.
        check_id: String,
    },
}

/// Everything the policy engine needs to make a decision.
///
/// Built by the executor from agent metadata and the current step inputs.
/// All fields are plain strings so policy rules can be written without
/// depending on the full contract type hierarchy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyContext {
    /// String representation of the agent's AgentId.
    pub agent_id: String,
    /// String representation of the ExecutionId UUID.
    pub execution_id: String,
    /// The agent's current lifecycle phase.
    pub current_phase: String,
    /// The action the agent wants to perform (from `Agent::describe_action()`).
    pub action: String,
    /// The resource the action targets (from `Agent::describe_action()`).
    pub resource: String,
    /// All capabilities the agent holds in this execution.
    pub capabilities: Vec<String>,
    /// Arbitrary additional metadata the agent provides for richer policy evaluation.
    pub metadata: serde_json::Value,
}
