//! Agent identity and state types.
//!
//! These types define the data flowing through the VERITAS execution pipeline.
//! They are intentionally minimal — VERITAS does not prescribe agent internals.

use serde::{Deserialize, Serialize};

/// Stable, human-readable identifier for an agent type.
///
/// Used across policy rules, audit logs, and capability grants.
/// Example: AgentId("patient-intake-agent")
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

/// Unique identifier for a single agent execution instance.
///
/// Every call to Executor::step() operates within an execution identified
/// by this UUID, which appears in every audit record.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExecutionId(pub uuid::Uuid);

impl ExecutionId {
    /// Create a new, unique execution ID.
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for ExecutionId {
    fn default() -> Self {
        Self::new()
    }
}

/// A snapshot of all state the agent carries between steps.
///
/// The runtime treats this as an opaque blob it passes to `Agent::propose()`
/// and receives back from `Agent::transition()`. The `phase` and `step` fields
/// are read by the executor to drive policy evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    /// Which agent type owns this state.
    pub agent_id: AgentId,
    /// The execution this state belongs to.
    pub execution_id: ExecutionId,
    /// Human-readable current lifecycle phase (e.g. "intake", "review", "complete").
    pub phase: String,
    /// Arbitrary agent-internal state. The runtime never inspects this.
    pub context: serde_json::Value,
    /// Monotonically increasing step counter within this execution.
    pub step: u64,
}

/// An input event delivered to the agent at the start of a step.
///
/// `kind` is a discriminant that policy rules can match on.
/// `payload` carries the full event body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInput {
    /// Discriminant string (e.g. "user_message", "tool_result", "approval_granted").
    pub kind: String,
    /// Arbitrary JSON body. The runtime does not validate or inspect this.
    pub payload: serde_json::Value,
}

/// The output produced by `Agent::propose()` before verification.
///
/// After the verifier approves the output, it is passed to `Agent::transition()`
/// to advance state, and then stored in the audit record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOutput {
    /// Discriminant string (e.g. "tool_call", "message", "decision").
    pub kind: String,
    /// Arbitrary JSON body. The verifier inspects this against the OutputSchema.
    pub payload: serde_json::Value,
}
