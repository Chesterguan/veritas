//! Core trait definitions for the VERITAS execution pipeline.
//!
//! These four traits define the complete trust boundary:
//!
//! - `Agent`        — untrusted logic (may be backed by an LLM)
//! - `PolicyEngine` — trusted gate (evaluated before the agent acts)
//! - `AuditWriter`  — trusted sink (records every step immutably)
//! - `Verifier`     — trusted checker (validates output before delivery)
//!
//! The executor wires them together in the correct order. Implementations
//! of `Agent` are never called unless the policy engine first returns Allow.

use veritas_contracts::{
    agent::{AgentInput, AgentOutput, AgentState},
    error::VeritasResult,
    execution::StepRecord,
    policy::{PolicyContext, PolicyVerdict},
    verify::{OutputSchema, VerificationReport},
};

/// An agent that proposes outputs and manages its own state transitions.
///
/// Implementations of this trait are considered **untrusted** — they may be
/// backed by an LLM, external tool, or arbitrary code. The executor ensures
/// `propose()` is never called unless the policy engine has allowed the action.
pub trait Agent: Send + Sync {
    /// Produce an output for the given input, without side effects.
    ///
    /// This method MUST be pure from the runtime's perspective: it reads state
    /// and input, produces an output, and does nothing else. The executor
    /// calls `transition()` separately to advance state after verification.
    ///
    /// The executor guarantees this is only called after `PolicyEngine::evaluate()`
    /// returns `PolicyVerdict::Allow`.
    fn propose(&self, state: &AgentState, input: &AgentInput) -> VeritasResult<AgentOutput>;

    /// Apply `output` to `state` and return the next state.
    ///
    /// Only called after the verifier has approved the output. The returned
    /// state must have `step` incremented by exactly 1.
    fn transition(&self, state: &AgentState, output: &AgentOutput) -> VeritasResult<AgentState>;

    /// Return the capability names required to perform this action.
    ///
    /// The executor checks these against the `CapabilitySet` before calling
    /// `propose()`. If any are missing, the step is denied without touching
    /// the agent's logic.
    fn required_capabilities(&self, state: &AgentState, input: &AgentInput) -> Vec<String>;

    /// Describe the action and resource this step would affect.
    ///
    /// Returns `(action, resource)` — plain strings the policy engine uses
    /// to populate `PolicyContext`. The agent defines the semantics.
    ///
    /// Example: `("read_patient_record", "patient/12345")`
    fn describe_action(&self, state: &AgentState, input: &AgentInput) -> (String, String);

    /// Return true if the agent has reached a terminal state.
    ///
    /// When this returns true after a step completes, the executor calls
    /// `AuditWriter::finalize()` and returns `StepResult::Complete`.
    fn is_terminal(&self, state: &AgentState) -> bool;
}

/// The policy engine: the first and most critical gate in the execution pipeline.
///
/// Implementations are **trusted** and must be deterministic. Policy evaluation
/// should be fast (microseconds) — avoid I/O in hot-path implementations.
pub trait PolicyEngine: Send + Sync {
    /// Evaluate whether the described action is permitted.
    ///
    /// The executor calls this before any agent logic runs. A non-`Allow`
    /// verdict prevents `Agent::propose()` from being called.
    fn evaluate(&self, ctx: &PolicyContext) -> VeritasResult<PolicyVerdict>;
}

/// The audit writer: the immutable execution record.
///
/// Every step — regardless of verdict — produces exactly one `StepRecord`
/// that must be persisted by this writer. A failed write is fatal: the step
/// is rolled back and `VeritasError::AuditWriteFailed` is returned.
pub trait AuditWriter: Send + Sync {
    /// Append one step record to the audit log.
    ///
    /// Implementations must treat this as an append-only operation.
    /// Records written here are never modified or deleted by the runtime.
    fn write(&self, record: &StepRecord) -> VeritasResult<()>;

    /// Mark an execution as complete in the audit log.
    ///
    /// Called by the executor when `Agent::is_terminal()` returns true.
    /// Implementations may use this to flush, sign, or seal the log.
    fn finalize(&self, execution_id: &str) -> VeritasResult<()>;
}

/// The output verifier: the last gate before state advances.
///
/// Implementations are **trusted** and must not call agent logic. They inspect
/// the raw `AgentOutput` against a declarative `OutputSchema` and return a
/// report. A failing report prevents the step from completing.
pub trait Verifier: Send + Sync {
    /// Verify `output` against `schema`.
    ///
    /// Return a `VerificationReport` with `passed = true` if all rules pass,
    /// or `passed = false` with populated `failures` if any rule fails.
    fn verify(&self, output: &AgentOutput, schema: &OutputSchema) -> VeritasResult<VerificationReport>;
}
