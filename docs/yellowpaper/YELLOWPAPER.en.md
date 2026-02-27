# VERITAS Yellow Paper — Technical Specification

> Version 0.1 · 2026-02-27

This document is the formal technical specification for the VERITAS runtime. It defines the type system, execution pipeline, policy engine semantics, audit trail construction, and verification protocol. For motivation, positioning, and design philosophy, see the [Whitepaper v0.3](../whitepaper/WHITEPAPER.en.md).

---

## Table of Contents

1. [Type System](#1-type-system)
2. [Trait Interfaces](#2-trait-interfaces)
3. [Execution Pipeline](#3-execution-pipeline)
4. [Policy Engine](#4-policy-engine)
5. [Audit Trail](#5-audit-trail)
6. [Verification Engine](#6-verification-engine)
7. [Security Properties](#7-security-properties)
8. [State Machine](#8-state-machine)
9. [Healthcare Reference Walkthrough](#9-healthcare-reference-walkthrough)
10. [Appendix: Crate Dependency Graph](#10-appendix-crate-dependency-graph)

---

## 1. Type System

All types are defined in `veritas-contracts` and shared across the workspace. No business logic lives in this crate — only data definitions, error types, and trait markers.

### 1.1 Agent Identity and State

```
AgentId(String)
```

Stable, human-readable identifier for an agent type. Used across policy rules, audit logs, and capability grants. Example: `AgentId("patient-intake-agent")`.

```
ExecutionId(uuid::Uuid)
```

Unique identifier for a single agent execution instance. Created via `Uuid::new_v4()`. Appears in every audit record.

```
AgentState {
    agent_id:     AgentId,
    execution_id: ExecutionId,
    phase:        String,              // e.g. "intake", "review", "complete"
    context:      serde_json::Value,   // opaque to runtime, never inspected
    step:         u64,                 // monotonically increasing, +1 per transition
}
```

The runtime reads `agent_id`, `execution_id`, `phase`, and `step`. The `context` field is entirely agent-internal — the runtime never reads, validates, or modifies it.

```
AgentInput {
    kind:    String,              // e.g. "user_message", "tool_result", "approval_granted"
    payload: serde_json::Value,   // opaque to runtime
}
```

```
AgentOutput {
    kind:    String,              // e.g. "tool_call", "message", "decision"
    payload: serde_json::Value,   // inspected by verifier against OutputSchema
}
```

### 1.2 Capability-Based Access Control

```
Capability(String)
```

Opaque capability token. Naming convention: `namespace:operation` (e.g. `"phi:read"`, `"order:submit"`).

```
CapabilitySet {
    inner: HashSet<Capability>
}
```

Methods: `grant(cap)`, `has(&cap) -> bool`, `all() -> Iterator`. Capabilities are granted at startup by the hosting application and **never elevated at runtime**.

### 1.3 Policy Verdict and Context

```
PolicyVerdict
    | Allow
    | Deny { reason: String }
    | RequireApproval { reason: String, approver_role: String }
    | RequireVerification { check_id: String }
```

All variants except `Allow` prevent `Agent::propose()` from being called. This is the core security guarantee.

```
PolicyContext {
    agent_id:      String,
    execution_id:  String,
    current_phase: String,
    action:        String,              // from Agent::describe_action()
    resource:      String,              // from Agent::describe_action()
    capabilities:  Vec<String>,         // all capabilities held
    metadata:      serde_json::Value,   // additional context for richer rules
}
```

### 1.4 Step Results and Audit Records

```
StepResult
    | Transitioned { next_state: AgentState, output: AgentOutput }
    | Denied { reason: String, final_state: AgentState }
    | AwaitingApproval { reason: String, approver_role: String, suspended_state: AgentState }
    | Complete { final_state: AgentState, output: AgentOutput }
```

```
StepRecord {
    step:      u64,
    input:     AgentInput,
    verdict:   PolicyVerdict,
    output:    Option<AgentOutput>,    // None on Deny/AwaitingApproval
    timestamp: DateTime<Utc>,
}
```

One `StepRecord` is produced per step, regardless of outcome. Records are never modified after creation.

### 1.5 Output Verification Types

```
OutputSchema {
    schema_id:   String,
    json_schema: serde_json::Value,    // null = skip structural validation
    rules:       Vec<VerificationRule>,
}
```

```
VerificationRule {
    rule_id:     String,
    description: String,
    rule_type:   VerificationRuleType,
}
```

```
VerificationRuleType
    | RequiredField { field_path: String }
    | AllowedValues { field_path: String, allowed: Vec<Value> }
    | ForbiddenPattern { field_path: String, pattern: String }
    | Custom { function_name: String }
```

```
VerificationReport {
    passed:   bool,
    failures: Vec<VerificationFailure>,
}

VerificationFailure {
    rule_id: String,
    message: String,
}
```

### 1.6 Error Types

```
VeritasError
    | PolicyDenied { reason }
    | CapabilityMissing { capability, action }
    | VerificationFailed { reason }
    | AuditWriteFailed { reason }           // fatal — step cannot proceed
    | StateMachineError { reason }
    | ConfigError { reason }
    | SchemaValidation { reason }
```

All fallible operations return `VeritasResult<T> = Result<T, VeritasError>`.

---

## 2. Trait Interfaces

Four traits define the complete trust boundary. Defined in `veritas-core/src/traits.rs`.

### 2.1 Agent (untrusted)

```rust
trait Agent: Send + Sync {
    fn propose(&self, state: &AgentState, input: &AgentInput) -> VeritasResult<AgentOutput>;
    fn transition(&self, state: &AgentState, output: &AgentOutput) -> VeritasResult<AgentState>;
    fn required_capabilities(&self, state: &AgentState, input: &AgentInput) -> Vec<String>;
    fn describe_action(&self, state: &AgentState, input: &AgentInput) -> (String, String);
    fn is_terminal(&self, state: &AgentState) -> bool;
}
```

| Method | Contract |
|--------|----------|
| `propose` | Pure from runtime's perspective. Only called after policy Allow + capability check. |
| `transition` | Returns next state with `step` incremented by exactly 1. Only called after verification passes. |
| `required_capabilities` | Capabilities the executor checks before calling `propose()`. |
| `describe_action` | Returns `(action, resource)` strings used to populate `PolicyContext`. |
| `is_terminal` | When true, executor calls `audit.finalize()` and returns `StepResult::Complete`. |

### 2.2 PolicyEngine (trusted)

```rust
trait PolicyEngine: Send + Sync {
    fn evaluate(&self, ctx: &PolicyContext) -> VeritasResult<PolicyVerdict>;
}
```

Must be deterministic. Must be fast (microseconds). Called before any agent logic.

### 2.3 AuditWriter (trusted)

```rust
trait AuditWriter: Send + Sync {
    fn write(&self, record: &StepRecord) -> VeritasResult<()>;
    fn finalize(&self, execution_id: &str) -> VeritasResult<()>;
}
```

`write()` is append-only — records are never modified or deleted. A failed write is fatal. `finalize()` is called when the agent reaches terminal state.

### 2.4 Verifier (trusted)

```rust
trait Verifier: Send + Sync {
    fn verify(&self, output: &AgentOutput, schema: &OutputSchema) -> VeritasResult<VerificationReport>;
}
```

Inspects raw `AgentOutput` against a declarative `OutputSchema`. Must not call agent logic.

---

## 3. Execution Pipeline

The `Executor` drives a single agent execution. One executor per `ExecutionId`. Defined in `veritas-core/src/executor.rs`.

```rust
struct Executor {
    policy:   Box<dyn PolicyEngine>,
    audit:    Box<dyn AuditWriter>,
    verifier: Box<dyn Verifier>,
    schema:   OutputSchema,
}
```

### 3.1 The `step()` Algorithm

```
fn step(agent, state, input, capabilities) -> VeritasResult<StepResult>
```

**Step 1 — Describe action.** Call `agent.describe_action(&state, &input)` to get `(action, resource)`. Build `PolicyContext` with agent_id, execution_id, phase, action, resource, capabilities, and null metadata.

**Step 2 — Policy evaluation.** Call `policy.evaluate(&ctx)`.
- `Deny` → audit the denial → return `StepResult::Denied`. Stop.
- `RequireApproval` → audit the suspension → return `StepResult::AwaitingApproval`. Stop.
- `Allow` or `RequireVerification` → continue.

**Step 3 — Capability check.** Call `agent.required_capabilities(&state, &input)`. For each required capability, check `capabilities.has(&cap)`. If any missing → audit synthetic denial → return `Err(CapabilityMissing)`. Stop.

**Step 4 — Agent proposal.** Call `agent.propose(&state, &input)`. This is the **only call site** for `propose()` in the runtime. It is structurally unreachable unless steps 2 and 3 passed.

**Step 5 — Output verification.** Call `verifier.verify(&output, &schema)`. If `report.passed == false` → return `Err(VerificationFailed)`. Stop.

**Step 6 — State transition.** Call `agent.transition(&state, &output)` → `next_state`.

**Step 7 — Audit.** Create `StepRecord` with step number, input, verdict, output, and timestamp. Call `audit.write(&record)`. If failed → return `Err(AuditWriteFailed)`. Fatal.

**Step 8 — Terminal check.** Call `agent.is_terminal(&next_state)`.

**Step 9 — Complete or continue.**
- Terminal → call `audit.finalize(&execution_id)` → return `StepResult::Complete`.
- Not terminal → return `StepResult::Transitioned`.

### 3.2 Control Flow

```
                     step()
                       │
              ┌────────▼────────┐
              │ describe_action  │
              │ build PolicyCtx  │
              └────────┬────────┘
                       │
              ┌────────▼────────┐
              │ policy.evaluate  │
              └───┬────┬────┬───┘
                  │    │    │
             Deny │    │    │ Allow / RequireVerification
                  │    │    │
                  ▼    │    ▼
             [audit]   │   capability check
             [return   │     │
              Denied]  │     ├── missing → [audit] [Err(CapabilityMissing)]
                       │     │
              RequireApproval ▼
                  │    agent.propose()
                  ▼         │
             [audit]        ▼
             [return   verifier.verify()
              Awaiting      │
              Approval]     ├── failed → [Err(VerificationFailed)]
                            │
                            ▼
                      agent.transition()
                            │
                            ▼
                       audit.write()
                            │
                            ├── is_terminal? ──yes──→ audit.finalize() → Complete
                            │
                            └── no → Transitioned
```

---

## 4. Policy Engine

The `TomlPolicyEngine` loads rules from TOML and implements `PolicyEngine`. Defined in `veritas-policy/`.

### 4.1 Rule Schema

```toml
[[rules]]
id = "rule-identifier"                        # stable, used in audit logs
description = "human-readable explanation"
action = "the-action"                          # or "*" for wildcard
resource = "the-resource"                      # or "*" for wildcard
required_capabilities = ["cap.name"]           # optional, default []
verdict = "allow"                              # allow | deny | require-approval | require-verification
deny_reason = "..."                            # required when verdict = deny
approval_reason = "..."                        # required when verdict = require-approval
approver_role = "..."                          # required when verdict = require-approval
verification_check_id = "..."                  # required when verdict = require-verification
```

### 4.2 Pattern Matching

A rule matches when **both** conditions hold:

- `rule.action == "*"` OR `rule.action == ctx.action` (exact, case-sensitive)
- `rule.resource == "*"` OR `rule.resource == ctx.resource` (exact, case-sensitive)

### 4.3 Evaluation Algorithm

```
function evaluate(ctx) -> PolicyVerdict:
    for rule in rules (declaration order):
        if not rule.matches(ctx.action, ctx.resource):
            continue

        // Defense-in-depth: capability override
        for cap in rule.required_capabilities:
            if cap not in ctx.capabilities:
                return Deny("rule '{id}' requires capability '{cap}' not granted")

        // Convert rule verdict to policy verdict
        match rule.verdict:
            Allow               → return Allow
            Deny                → return Deny(rule.deny_reason)
            RequireApproval     → return RequireApproval(rule.approval_reason, rule.approver_role)
            RequireVerification → return RequireVerification(rule.verification_check_id)

    // No rule matched
    return Deny("denied by default: no rule matched action '{action}' on resource '{resource}'")
```

**Key properties:**

| Property | Description |
|----------|-------------|
| First-match-wins | Only the first matching rule is evaluated |
| Deny-by-default | No rule match → automatic deny |
| Capability override | Missing capabilities override even Allow verdicts |
| Deterministic | Same input always produces same verdict |
| O(n) linear scan | Rules evaluated sequentially; microsecond-scale for typical rule sets |

### 4.4 Example: Healthcare Policy

```toml
# policies/healthcare.toml — scenarios 1–3

[[rules]]
id = "allow-drug-interaction-check"
description = "Allow drug interaction database queries"
action = "drug-interaction-check"
resource = "drug-database"
required_capabilities = ["drug-database.read"]
verdict = "allow"

[[rules]]
id = "deny-patient-query-no-consent"
description = "Block AI queries when patient has not consented"
action = "query"
resource = "patient-records-no-consent"
verdict = "deny"
deny_reason = "patient data access denied: patient has not provided consent for AI-assisted queries"

[[rules]]
id = "allow-patient-query-with-consent"
description = "Allow patient record queries when consent is given"
action = "query"
resource = "patient-records"
required_capabilities = ["patient-records.read"]
verdict = "allow"
```

### 4.5 Example: Prior Authorization Policy

```toml
# policies/prior_auth.toml — rule order matters

[[rules]]
id = "require-approval-high-cost-procedure"
action = "propose-procedure"
resource = "high-cost-procedure"
verdict = "require-approval"
approval_reason = "cardiac MRI is a high-cost procedure requiring attending physician sign-off"
approver_role = "attending-physician"

[[rules]]
id = "deny-uncovered-procedure"
action = "check-coverage"
resource = "uncovered-procedure"
verdict = "deny"
deny_reason = "procedure is not covered under the patient's current insurance plan"

[[rules]]
id = "allow-insurance-eligibility-check"
action = "check-coverage"
resource = "insurance-records"
required_capabilities = ["insurance.read"]
verdict = "allow"
```

Note: `deny-uncovered-procedure` must appear before `allow-insurance-eligibility-check` because both match `action = "check-coverage"`. First-match-wins ensures the deny rule fires when `resource = "uncovered-procedure"`.

---

## 5. Audit Trail

The audit subsystem implements a SHA-256 hash-chained, append-only, tamper-detectable execution trace. Defined in `veritas-audit/`.

### 5.1 AuditEvent Schema

```
AuditEvent {
    sequence:     u64,         // monotonic, starting at 0
    execution_id: String,
    record:       StepRecord,  // the immutable step record from the executor
    prev_hash:    String,      // SHA-256 hex of previous event, or GENESIS_HASH
    this_hash:    String,      // SHA-256 hex of this event's content
}
```

### 5.2 Genesis Sentinel

```
GENESIS_HASH = "0000000000000000000000000000000000000000000000000000000000000000"
```

64 hex zeros. Can never be the SHA-256 of real data, making first-event detection unambiguous.

### 5.3 Hash Construction

For each event, the hash is computed over four inputs concatenated in order:

```
this_hash = SHA-256(
    execution_id as UTF-8 bytes
    || sequence as 8-byte little-endian
    || prev_hash as UTF-8 bytes
    || canonical JSON of record (serde_json, no pretty-printing)
)
```

The result is a lowercase 64-character hex string.

**Chain formula:**

```
E₀.prev_hash = GENESIS_HASH
E₀.this_hash = SHA-256(exec_id ‖ 0_le ‖ GENESIS_HASH ‖ JSON(record₀))

Eₙ.prev_hash = Eₙ₋₁.this_hash
Eₙ.this_hash = SHA-256(exec_id ‖ n_le ‖ Eₙ₋₁.this_hash ‖ JSON(recordₙ))
```

### 5.4 Chain Verification

```
function verify_chain(events) -> bool:
    expected_prev = GENESIS_HASH

    for event in events:
        // Rule 1: prev-hash linkage
        if event.prev_hash != expected_prev:
            return false

        // Rule 2: hash correctness
        recomputed = hash_event(event.execution_id, event.sequence, event.record, event.prev_hash)
        if event.this_hash != recomputed:
            return false

        expected_prev = event.this_hash

    return true
```

An empty chain is defined as valid.

### 5.5 Tamper Detection

Modifying any field of any event's `record` causes `hash_event()` to produce a different hash, failing Rule 2. Changing `this_hash` to compensate breaks the linkage to the next event's `prev_hash`. The corruption cascades — only a complete chain rewrite from the tampered event onward could succeed, and the `terminal_hash` commitment would still expose it.

### 5.6 AuditLog

```
AuditLog {
    execution_id:  String,
    events:        Vec<AuditEvent>,   // chain order, sequence 0 first
    finalized_at:  DateTime<Utc>,
    terminal_hash: String,            // this_hash of last event, or "" if empty
}
```

The `terminal_hash` is a compact commitment to the entire execution log.

### 5.7 InMemoryAuditWriter

Reference implementation using `Arc<Mutex<InMemoryState>>`:

- `write()`: acquires lock, computes `hash_event()`, appends `AuditEvent`, increments sequence, updates `last_hash`.
- `finalize()`: logs structured message. Backends that persist may flush or seal here.
- `export_log()`: clones events under lock, produces `AuditLog`.
- `verify_integrity()`: delegates to `verify_chain()`.

---

## 6. Verification Engine

The `SchemaVerifier` implements two-phase output validation. Defined in `veritas-verify/src/engine.rs`.

### 6.1 Phase 1: JSON Schema Structural Validation

If `schema.json_schema` is null, structural validation is skipped. Otherwise:

```
validator = jsonschema::validator_for(&schema.json_schema)
for error in validator.iter_errors(&payload):
    failures.push({ rule_id: "json-schema", message: "violation at {path}: {error}" })
```

Uses `jsonschema` crate v0.28 API (`validator_for()` + `iter_errors()`).

### 6.2 Phase 2: Semantic Rule Evaluation

All rules in `schema.rules` are evaluated sequentially. All failures are accumulated — the caller sees the full picture, not just the first failure.

### 6.3 Rule Type Semantics

**RequiredField** — Field at dot-notation path must be present and non-null.

```
resolve_path(payload, "patient.id"):
    payload["patient"]["id"] → if present and non-null → PASS
    otherwise → FAIL("required field 'patient.id' is missing or null")
```

Path resolution: split on `.`, traverse nested objects, treat null as absent.

**AllowedValues** — Field value must appear in the exhaustive allowed set.

```
if field missing → FAIL("field missing; cannot check allowed values")
if value in allowed → PASS
otherwise → FAIL("value not in allowed set")
```

**ForbiddenPattern** — String field must not contain the pattern as a substring.

```
if field missing → PASS (nothing to check)
if field is not string → PASS (rule does not apply)
if string contains pattern → FAIL("contains forbidden pattern")
otherwise → PASS
```

**Custom** — Delegate to a named function registered by the hosting application.

```
if function registered → call function(payload)
    returns None → PASS
    returns Some(msg) → FAIL(msg)
if function not registered → FAIL("no custom rule registered for '{name}'")
```

Unregistered names are themselves failures — misconfigured schemas surface immediately.

### 6.4 Report Construction

```
report = {
    passed:   failures.is_empty(),
    failures: all accumulated failures
}
```

---

## 7. Security Properties

Ten invariants the runtime enforces:

**INV-1: Structural proposal gate.** `Agent::propose()` is only reachable after `PolicyEngine::evaluate()` returns `Allow` (or `RequireVerification`) AND all `Agent::required_capabilities()` are present in the `CapabilitySet`. This is enforced by control flow — `propose()` appears after the match arms for Deny and RequireApproval return early.

**INV-2: Step counter monotonicity.** `AgentState.step` is incremented by exactly 1 on each transition. The `Agent::transition()` contract requires this.

**INV-3: Audit completeness.** Every step produces exactly one `StepRecord`. Denials, suspensions, and successes are all audited.

**INV-4: Audit immutability.** Records are append-only. The audit writer never modifies or deletes records. The hash chain cryptographically enforces this.

**INV-5: Terminal finalization.** When `Agent::is_terminal()` returns true, `AuditWriter::finalize()` is called and execution halts with `StepResult::Complete`.

**INV-6: Deny-by-default.** If no policy rule matches the `(action, resource)` pair, the engine returns `Deny`.

**INV-7: First-match-wins.** Rules are evaluated in declaration order. Only the first matching rule is applied.

**INV-8: Capability override.** Missing capabilities override even Allow-verdict rules. A rule with `verdict = "allow"` and `required_capabilities = ["phi:read"]` produces `Deny` if `phi:read` is not in the agent's capability set.

**INV-9: Deterministic evaluation.** The policy engine and verifier contain no randomness, no I/O in the hot path, and no mutable state. Same input always produces same output.

**INV-10: No capability elevation.** Capabilities are granted at startup and never added, modified, or elevated during execution.

### Trust Boundary

| Trusted | Untrusted |
|---------|-----------|
| Executor | Agent (may be LLM-backed) |
| PolicyEngine | Tools |
| AuditWriter | Input data |
| Verifier | External environment |

The trusted computing base is four components. Everything else is untrusted by default.

---

## 8. State Machine

### 8.1 Transition Diagram

```
                            step()
                              │
                    ┌─────────▼──────────┐
                    │   Policy Evaluate   │
                    └──┬──────┬──────┬───┘
                       │      │      │
                  Deny │  RequireApproval  Allow
                       │      │      │
                       ▼      ▼      ▼
                   Denied  Awaiting  ┌──────────────────┐
                   (term)  Approval  │ Capability Check  │
                           (suspend) └────┬─────────┬───┘
                                     missing│    ok │
                                          │       ▼
                                    Err(Cap   ┌─────────┐
                                    Missing)  │ Propose  │
                                              └────┬────┘
                                                   ▼
                                            ┌────────────┐
                                            │   Verify   │
                                            └──┬──────┬──┘
                                          fail │   ok │
                                               │      ▼
                                         Err(Verif  ┌────────────┐
                                         Failed)    │ Transition  │
                                                    └──────┬─────┘
                                                           ▼
                                                    ┌────────────┐
                                                    │   Audit    │
                                                    └──────┬─────┘
                                                    terminal?
                                                    ├─ yes → Complete (terminal)
                                                    └─ no  → Transitioned (continue)
```

### 8.2 Terminal States

- `StepResult::Denied` — policy denied; execution ends.
- `StepResult::Complete` — agent reached terminal state; execution ends.
- `Err(CapabilityMissing)` — missing capability; execution ends.
- `Err(VerificationFailed)` — output rejected; execution ends.
- `Err(AuditWriteFailed)` — fatal; execution ends.

### 8.3 Suspended State

- `StepResult::AwaitingApproval` — execution paused. The caller must persist `suspended_state`, obtain approval from the specified `approver_role`, and resume by calling `step()` with an `AgentInput { kind: "approval_granted", ... }` carrying the approval token.

### 8.4 Continuing State

- `StepResult::Transitioned` — call `step()` again with `next_state` and the next `AgentInput`.

---

## 9. Healthcare Reference Walkthrough

Five scenarios demonstrate end-to-end VERITAS enforcement. All are implemented in `veritas-ref-healthcare/src/scenarios/`.

### 9.1 Scenario 1: Drug Interaction Checker

Demonstrates the Allow flow with output verification.

```
Input: drug_a="warfarin", drug_b="aspirin"

1. describe_action → ("drug-interaction-check", "drug-database")
2. policy.evaluate → Allow (rule: allow-drug-interaction-check)
3. capability check → "drug-database.read" present → PASS
4. agent.propose → { query: {...}, result: { severity: "HIGH" }, recommendation: "..." }
5. verify → RequiredField "query" ✓, "result" ✓, "recommendation" ✓ → PASS
6. transition → phase="complete", step=1
7. audit.write → StepRecord appended to hash chain
8. is_terminal → true → audit.finalize → Complete
```

### 9.2 Scenario 3: Patient Data Query (Three Sub-cases)

Demonstrates all three enforcement layers.

**Sub-case A: Allow.** Patient has consent, agent has capability. Policy allows. Step completes.

**Sub-case B: Capability missing.** Policy allows (no capability requirement in test policy), but agent declares `required_capabilities = ["patient-records.read"]`. Executor's own check catches the missing capability → `Err(CapabilityMissing)`. Agent's `propose()` is never called.

**Sub-case C: Policy deny.** Patient lacks consent. Agent dynamically routes `resource = "patient-records-no-consent"`, which matches the deny rule. Policy returns `Deny`. Agent's `propose()` is never called.

### 9.3 Scenario 4: Multi-Agent Clinical Pipeline

Four agents execute sequentially, each with its own Executor and audit chain:

```
SymptomAnalyzer → DiagnosisSuggester → TreatmentPlanner → DrugSafetyChecker
```

Each agent's verified output feeds the next agent's input. Stage 4 includes a custom verification rule `"no-high-risk-unreviewed"` that rejects `HIGH`-risk safety reports unless `reviewed = true`.

Four independent audit chains are produced and verified for integrity.

### 9.4 Scenario 5: Prior Authorization Workflow

Demonstrates the `RequireApproval` lifecycle.

```
Step 1: ClinicalProposalAgent
  → policy: RequireApproval (approver_role: "attending-physician")
  → agent.propose() NOT called
  → StepResult::AwaitingApproval returned
  → approval simulated: token="PHY-APPROVE-2026-0218"

Step 2A (covered): InsuranceEligibilityAgent
  → resource="insurance-records" → policy: Allow
  → agent.propose() → { covered: true, plan_name: "Blue Shield PPO", copay_usd: 250 }
  → verification passes

Step 2B (not covered): InsuranceEligibilityAgent
  → resource="uncovered-procedure" → policy: Deny
  → agent.propose() NOT called
  → PA workflow terminates

Step 3 (if covered): PASubmissionAgent
  → policy: Allow (requires "pa.write" capability)
  → agent.propose() → { pa_reference: "PA-2026-0218-4471", status: "submitted" }
```

---

## 10. Appendix: Crate Dependency Graph

```
veritas-contracts          (no dependencies — shared types only)
    │
    ├── veritas-core       (traits + Executor)
    │       │
    │       ├── veritas-policy       (TomlPolicyEngine impl PolicyEngine)
    │       ├── veritas-audit        (InMemoryAuditWriter impl AuditWriter)
    │       └── veritas-verify       (SchemaVerifier impl Verifier)
    │               │
    │               └── veritas-ref-healthcare   (5 scenarios, 3 policy files)
    │                       │
    │                       ├── demo             (CLI runner, clap)
    │                       └── tui              (interactive TUI, ratatui)
    │
    └── (all crates depend on veritas-contracts)
```

**Workspace:** 8 members. **Tests:** 58 across all crates.

---

*VERITAS Yellow Paper v0.1 — 2026-02-27*
*Licensed under Apache License 2.0*
