# VERITAS Yellow Paper — Technical Specification

> Version 0.2 · 2026-03-18

This document is the formal technical specification for the VERITAS runtime. It defines the type system, execution pipeline, policy engine semantics, audit trail construction, verification protocol, and model governance layer. For motivation, positioning, and design philosophy, see the [Whitepaper v0.3](../whitepaper/WHITEPAPER.en.md).

---

## Table of Contents

1. [Type System](#1-type-system)
2. [Trait Interfaces](#2-trait-interfaces)
3. [Execution Pipeline](#3-execution-pipeline)
4. [Policy Engine](#4-policy-engine)
5. [Audit Trail](#5-audit-trail)
6. [Verification Engine](#6-verification-engine)
7. [Model Governance](#7-model-governance)
8. [Security Properties](#8-security-properties)
9. [State Machine](#9-state-machine)
10. [Healthcare Reference Walkthrough](#10-healthcare-reference-walkthrough)
11. [Appendix: Crate Dependency Graph](#11-appendix-crate-dependency-graph)

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

In the target architecture, model invocation outputs include `ModelInvocationMetadata` (see §1.7) in the payload, enabling full provenance in audit records. This metadata originates from the trusted `ModelRegistry`, not from the agent.

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

Model governance extends this: the `ModelRegistry` auto-generates `"model:<id>"` and `"model:approved"` capabilities for registered, approved models (see §7.1).

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
    | InvalidInput { reason }               // malformed or unacceptable input
```

All fallible operations return `VeritasResult<T> = Result<T, VeritasError>`.

### 1.7 Model Governance Types

Defined in `veritas-contracts`. Used by `veritas-model` and downstream crates.

```
ModelModality
    | TextToText          // LLMs (Claude, GPT, Ollama)
    | ImageToText         // Vision models (radiology reports, pathology)
    | ImageToLabel        // Classification (skin lesion, retinal scan)
    | TabularToScore      // Risk scores (sepsis, readmission)
    | TimeSeriesToAlert   // Monitoring (ECG, vitals trend)
    | MultiModal          // Combined input types
    | Custom(String)      // Extensible
```

```
ApprovalStatus
    | Approved
    | Pending
    | Revoked { reason: String }
    | Experimental        // allowed in non-production environments only
```

```
ModelProvenance {
    vendor:              String,                  // "anthropic", "internal", "huggingface"
    training_data_hash:  Option<String>,          // reproducibility
    approval_status:     ApprovalStatus,
    approved_by:         Option<String>,          // governance chain
    approved_at:         Option<String>,          // ISO 8601 string (no chrono dependency)
    regulatory_class:    Option<String>,          // "FDA-510k", "CE-IVD", "research-only"
}
```

```
TokenUsage {
    input_tokens:  u64,
    output_tokens: u64,
}
```

```
ModelInvocationMetadata {
    model_id:          String,
    model_version:     String,
    modality:          ModelModality,
    confidence:        Option<f64>,       // model self-reported confidence
    latency_ms:        u64,
    provenance_summary: String,           // condensed from ModelProvenance for audit
}
```

`ModelInvocationMetadata` is designed to be attached to `AgentOutput.payload` when an agent invokes a registered model, ensuring audit records contain full model provenance without widening the `AgentOutput` struct. In the target architecture (see §3), the Executor populates this from the trusted `ModelRegistry` rather than relying on agent self-reporting.

```
DriftStatus
    | Stable
    | Warning { metric: String, current: f64, threshold: f64 }
    | Drifted { metric: String, current: f64, threshold: f64 }
```

---

## 2. Trait Interfaces

Seven traits define the complete trust boundary. Agent, PolicyEngine, AuditWriter, and Verifier are defined in `veritas-core/src/traits.rs`. ModelDescriptor, ModelCapability, and DriftMonitor are defined in `veritas-contracts` and implemented in `veritas-model`.

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

Must be deterministic and efficient. The reference implementation (`TomlPolicyEngine`) performs O(n) linear scan over rules with no I/O. Called before any agent logic.

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

### 2.5 ModelDescriptor (trusted)

```rust
trait ModelDescriptor: Send + Sync {
    fn model_id(&self) -> &str;
    fn modality(&self) -> &ModelModality;
    fn version(&self) -> &str;
    fn provenance(&self) -> &ModelProvenance;
}
```

Describes a model registered in the trusted `ModelRegistry`. Implemented by `RegisteredModel` in `veritas-model`. The registry accepts models with any `ApprovalStatus` — it is `capabilities_for()` that withholds `"model:approved"` for non-Approved models (see §7.1).

### 2.6 DriftMonitor (trusted)

```rust
trait DriftMonitor: Send + Sync {
    fn record(&self, model_id: &str, result: &serde_json::Value) -> VeritasResult<()>;
    fn check_drift(&self, model_id: &str) -> DriftStatus;
}
```

Monitors model behavior over time. `record()` accumulates invocation results; `check_drift()` compares current statistics against the registered baseline. Reference implementation: `InMemoryDriftMonitor` in `veritas-model`. See §7.2 for semantics.

### 2.7 ModelCapability (untrusted)

```rust
trait ModelCapability: Send + Sync {
    fn descriptor(&self) -> &dyn ModelDescriptor;
    fn invoke(&self, input: &serde_json::Value) -> VeritasResult<ModelResult<serde_json::Value>>;
    fn validate_input(&self, input: &serde_json::Value) -> VeritasResult<()>;
}
```

Wraps a concrete model backend behind an object-safe interface. Uses `serde_json::Value` for input/output so `Box<dyn ModelCapability>` is valid. Classified as **untrusted** because `invoke()` delegates to an external model — outputs must still pass through the Verifier. Currently defined in `veritas-contracts`; no implementors exist yet in the codebase. In the target architecture (see §3), invocation will be mediated by the Executor rather than called directly by agents.

---

## 3. Execution Pipeline

The `Executor` drives a single agent execution. One executor per `ExecutionId`. Defined in `veritas-core/src/executor.rs`.

```rust
// Current implementation
struct Executor {
    policy:   Box<dyn PolicyEngine>,
    audit:    Box<dyn AuditWriter>,
    verifier: Box<dyn Verifier>,
    schema:   OutputSchema,
}
```

> **Target architecture:** A `registry: Option<Arc<ModelRegistry>>` field is planned for the Executor as defined by the [ModelCapability RFC](../rfc-model-capability.md). The current implementation provides `ModelRegistry` as a standalone trusted component outside the Executor; full integration is planned.

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
approval_reason = "..."                        # optional; defaults to "approval required by rule '{id}'"
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
id = "allow-summarize-clinical-notes"
description = "Agent may summarize clinical notes when it holds clinical-notes.read"
action = "summarize"
resource = "clinical-notes"
required_capabilities = ["clinical-notes.read"]
verdict = "allow"

[[rules]]
id = "deny-patient-query-no-consent"
description = "Patient record queries are unconditionally denied when consent flag is absent"
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

### 4.6 Model Governance Policy

Model approval is enforced through ordinary policy rules that require the `model:approved` capability. The `ModelRegistry` generates this capability automatically for models with `ApprovalStatus::Approved`.

```toml
# policies/model_governance.toml — scenarios 6–7

[[rules]]
id = "allow-radiology-inference-approved"
description = "Approved radiology models may run clinical inference"
action = "run-inference"
resource = "radiology-model"
required_capabilities = ["model:approved", "radiology.read"]
verdict = "allow"

[[rules]]
id = "deny-radiology-inference-unapproved"
description = "Unapproved radiology models are blocked from clinical inference"
action = "run-inference"
resource = "radiology-model-unapproved"
verdict = "deny"
deny_reason = "model not approved for clinical use: FDA 510(k) clearance required"

[[rules]]
id = "allow-sepsis-scoring-approved"
description = "Approved sepsis risk model may score patient vitals"
action = "score-risk"
resource = "sepsis-model"
required_capabilities = ["model:approved", "patient-vitals.read"]
verdict = "allow"

[[rules]]
id = "deny-sepsis-scoring-revoked"
description = "Revoked or unapproved sepsis model is blocked from scoring"
action = "score-risk"
resource = "sepsis-model-revoked"
verdict = "deny"
deny_reason = "model has been revoked: drift detected, human review required before re-activation"
```

This design is backwards compatible: existing opaque capability strings (`"phi:read"`, `"order:submit"`) are unaffected. Model governance is additive.

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

**Known limitation:** Hash construction depends on `serde_json::to_vec()` for the canonical JSON representation of `StepRecord`. If the field order of `StepRecord` changes between VERITAS versions (due to struct field reordering or serde updates), audit chains created by one version may fail verification under another. A future hardening pass should adopt an explicit canonical serialization format (sorted-key JSON or CBOR with a fixed schema) to eliminate this dependency.

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

## 7. Model Governance

Model governance is implemented in the `veritas-model` crate. It sits alongside the core runtime as a trusted component — not in the agent path, not in the LLM. The trust level is the same as `PolicyEngine`.

### 7.1 ModelRegistry

`ModelRegistry` is the single source of truth for which models are permitted to operate.

```rust
struct ModelRegistry {
    models: HashMap<String, RegisteredModel>,
}
```

`RegisteredModel` is a concrete type implementing `ModelDescriptor` with fields: `model_id`, `modality`, `version`, `provenance`.

**Operations:**

| Method | Behavior |
|--------|----------|
| `register(model: RegisteredModel)` | Adds a model. Fails if `model_id` is already registered. |
| `revoke(model_id, reason)` | Updates the model's `ApprovalStatus` to `Revoked { reason }`. |
| `is_approved(model_id)` | Returns true only for `ApprovalStatus::Approved`. |
| `by_modality(modality)` | Lists all registered models of a given modality. |
| `capabilities_for(model_id)` | Generates capabilities for policy evaluation (see below). |

**Capability generation.** For a model with `ApprovalStatus::Approved`, `capabilities_for()` returns:

```
["model:<model_id>", "model:approved"]
```

For non-approved models (Pending, Experimental, Revoked), only `"model:<model_id>"` is returned — the `"model:approved"` capability is withheld. Policy rules that require `"model:approved"` will deny access, even if the model is registered.

This means approval status flows into the standard `CapabilitySet` mechanism. No special handling is required in the executor.

### 7.2 Drift Detection

`InMemoryDriftMonitor` implements the `DriftMonitor` trait using a rolling window over recent invocation results.

**Configuration (`DriftConfig`):**

```
window_size:        usize    // number of recent results to retain (must be > 0)
warning_threshold:  f64      // e.g. 0.10 = 10 pp drop triggers Warning
drift_threshold:    f64      // e.g. 0.20 = 20 pp drop triggers Drifted
```

The baseline is **not configured** — it is computed automatically as the mean confidence of the first complete window of observations. Once set, the baseline is fixed and never updated.

**Semantics:**

1. `record(model_id, result)` extracts the `"confidence"` field from the JSON result. If absent, the call is a silent no-op. The value is appended to the model's rolling window, evicting the oldest entry when the window is full. When the window first reaches `window_size`, the baseline is set as the mean of that window.
2. `check_drift(model_id)` computes `drop = baseline - current_window_mean`:
   - `drop < warning_threshold` → `DriftStatus::Stable`
   - `warning_threshold ≤ drop < drift_threshold` → `DriftStatus::Warning { metric: "confidence", current, threshold }`
   - `drop ≥ drift_threshold` → `DriftStatus::Drifted { metric: "confidence", current, threshold }`
3. Models with no baseline (not enough data) or unknown model IDs return `DriftStatus::Stable`.

### 7.3 Auto-Revocation

`registry.check_and_update(model_id, &monitor)` combines drift detection with registry mutation. It is a method on `ModelRegistry` that takes a `&dyn DriftMonitor`:

```
status = monitor.check_drift(model_id)
if status == Drifted:
    registry.revoke(model_id, "auto-revoked: model drift detected")
    return Drifted
return status
```

After auto-revocation, `registry.capabilities_for(model_id)` no longer returns `"model:approved"`. Any subsequent policy evaluation for that model will fail the capability check without additional configuration. The revocation is recorded in the registry's model provenance and appears in downstream audit records.

### 7.4 Trust Boundary

`ModelRegistry` is a **trusted** component. It holds the same trust level as `PolicyEngine` and `AuditWriter`.

This matters because model metadata in policy evaluation and audit records originates from the registry, not from the agent. An agent cannot self-report its own model identity to obtain approval — the capability grant comes exclusively from the registry. An LLM-backed agent that claims to be using an approved model but is actually using a different model cannot forge the `"model:approved"` capability, because capabilities are granted by the runtime at startup from the registry, not by the agent at runtime.

The registry itself is not network-accessible during execution. It is loaded once at startup, like policy rules.

---

## 8. Security Properties

Twelve invariants the runtime enforces:

**INV-1: Structural proposal gate.** `Agent::propose()` is only reachable after `PolicyEngine::evaluate()` returns `Allow` (or `RequireVerification`) AND all `Agent::required_capabilities()` are present in the `CapabilitySet`. This is enforced by control flow — `propose()` appears after the match arms for Deny and RequireApproval return early.

**INV-2: Step counter monotonicity.** `AgentState.step` is incremented by exactly 1 on each transition. The `Agent::transition()` contract requires this.

**INV-3: Audit completeness.** Every step produces exactly one `StepRecord`. Denials, suspensions, successes, verification failures, and agent errors are all audited.

**INV-4: Audit immutability.** Records are append-only. The audit writer never modifies or deletes records. The hash chain cryptographically enforces this.

**INV-5: Terminal finalization.** When `Agent::is_terminal()` returns true, `AuditWriter::finalize()` is called and execution halts with `StepResult::Complete`.

**INV-6: Deny-by-default.** If no policy rule matches the `(action, resource)` pair, the engine returns `Deny`.

**INV-7: First-match-wins.** Rules are evaluated in declaration order. Only the first matching rule is applied.

**INV-8: Capability override.** Missing capabilities override even Allow-verdict rules. A rule with `verdict = "allow"` and `required_capabilities = ["phi:read"]` produces `Deny` if `phi:read` is not in the agent's capability set.

**INV-9: Deterministic evaluation.** The policy engine and verifier contain no randomness, no I/O in the hot path, and no mutable state. Same input always produces same output.

**INV-10: No capability elevation.** Capabilities are granted at startup and never added, modified, or elevated during execution.

**INV-11: Model approval gate.** Models must be registered with `ApprovalStatus::Approved` in the `ModelRegistry` to receive the `"model:approved"` capability. Experimental, Pending, and Revoked models are structurally denied by the capability mechanism — no special-case logic is required.

**INV-12: Trusted model identity.** Model metadata in policy evaluation and audit records originates from the trusted `ModelRegistry`, not from agent self-reporting. An agent cannot forge model identity to obtain approval capabilities.

### Trust Boundary

| Trusted | Untrusted |
|---------|-----------|
| Executor | Agent (may be LLM-backed) |
| PolicyEngine | Tools |
| AuditWriter | Input data |
| Verifier | External environment |
| ModelRegistry | LLM outputs |

The trusted computing base is five components. Everything else is untrusted by default.

---

## 9. State Machine

### 9.1 Transition Diagram

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

### 9.2 Terminal States

- `StepResult::Denied` — policy denied; execution ends.
- `StepResult::Complete` — agent reached terminal state; execution ends.
- `Err(CapabilityMissing)` — missing capability; execution ends.
- `Err(VerificationFailed)` — output rejected; execution ends.
- `Err(AuditWriteFailed)` — fatal; execution ends.

### 9.3 Suspended State

- `StepResult::AwaitingApproval` — execution paused. The caller (hosting application) must persist `suspended_state`, obtain approval from the specified `approver_role` through its own approval workflow (e.g., notification system, approval UI, or integration with clinical decision support), and resume by calling `step()` with an `AgentInput { kind: "approval_granted", ... }` carrying the approval token. VERITAS does not implement the approval workflow itself — it provides the suspension point and the resume protocol. The approval mechanism (UI, notifications, approval ledger, token validation) is the responsibility of the hosting application.

### 9.4 Continuing State

- `StepResult::Transitioned` — call `step()` again with `next_state` and the next `AgentInput`.

---

## 10. Healthcare Reference Walkthrough

Seven scenarios demonstrate end-to-end VERITAS enforcement. Scenarios 1–5 are implemented in `veritas-ref-healthcare/src/scenarios/`. Scenarios 6–7 use `veritas-model` in addition.

### 10.1 Scenario 1: Drug Interaction Checker

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

### 10.2 Scenario 2: Clinical Note Summarizer

Demonstrates PII detection via a custom verification rule. The agent summarizes a clinical note; the verifier checks for forbidden patterns (e.g., SSN, phone numbers) in the output. If PII is detected → `Err(VerificationFailed)`.

### 10.3 Scenario 3: Patient Data Query (Three Sub-cases)

Demonstrates all three enforcement layers.

**Sub-case A: Allow.** Patient has consent, agent has capability. Policy allows. Step completes.

**Sub-case B: Capability missing.** Policy allows (no capability requirement in test policy), but agent declares `required_capabilities = ["patient-records.read"]`. Executor's own check catches the missing capability → `Err(CapabilityMissing)`. Agent's `propose()` is never called.

**Sub-case C: Policy deny.** Patient lacks consent. Agent dynamically routes `resource = "patient-records-no-consent"`, which matches the deny rule. Policy returns `Deny`. Agent's `propose()` is never called.

### 10.4 Scenario 4: Multi-Agent Clinical Pipeline

Four agents execute sequentially, each with its own Executor and audit chain:

```
SymptomAnalyzer → DiagnosisSuggester → TreatmentPlanner → DrugSafetyChecker
```

Each agent's verified output feeds the next agent's input. Stage 4 includes a custom verification rule `"no-high-risk-unreviewed"` that rejects `HIGH`-risk safety reports unless `reviewed = true`.

Four independent audit chains are produced and verified for integrity.

### 10.5 Scenario 5: Prior Authorization Workflow

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

### 10.6 Scenario 6: Radiology AI Model Governance

Demonstrates the `ModelRegistry` approval gate with an `ImageToLabel` model.

```
Model: chest-xray-v3.2
  ApprovalStatus: Approved
  regulatory_class: "FDA-510k"
  capabilities_for → ["model:chest-xray-v3.2", "model:approved"]
```

**Sub-case A: Approved model.** `CapabilitySet` includes `"model:approved"` and `"radiology.read"`. Policy rule `allow-radiology-inference-approved` allows. Agent invokes model via `propose()`. Audit record captures the step.

**Sub-case B: Experimental model blocked.**

```
Model: chest-xray-v4.0-beta
  ApprovalStatus: Experimental
  capabilities_for → ["model:chest-xray-v4.0-beta"]  // no "model:approved"
```

The agent's `describe_action()` returns resource `"radiology-model-unapproved"`, which routes to the explicit `deny-radiology-inference-unapproved` policy rule. Policy returns `Deny` with reason "model not approved for clinical use: FDA 510(k) clearance required". `propose()` is never called.

**Sub-case C: Drift detected → auto-revoke → deny.**

```
1. InMemoryDriftMonitor accumulates confidence scores for chest-xray-v3.2
2. Rolling mean drops below drift_threshold
3. check_and_update() calls registry.revoke("chest-xray-v3.2", "auto-revoked: model drift detected")
4. capabilities_for("chest-xray-v3.2") → ["model:chest-xray-v3.2"]  // "model:approved" withheld
5. Next step: capability check fails → Deny
6. Revocation reason appears in audit record
```

### 10.7 Scenario 7: Sepsis Risk Model with Drift

Demonstrates `TabularToScore` model governance and the full drift lifecycle.

```
Model: sepsis-risk-v2.1
  ApprovalStatus: Approved
  modality: TabularToScore
  (baseline auto-computed from first window of observations)
```

**Sub-case A: Stable operation.** Confidence scores in rolling window remain within `warning_threshold` of baseline. `DriftStatus::Stable`. Policy allows on each step.

**Sub-case B: Confidence degrades → auto-revoke → deny.**

```
1. Baseline established: 5 invocations at confidence 0.89
2. Phase 2: 5 invocations at confidence 0.78 (drop ≈ 0.11 > warning_threshold 0.10)
   → check_drift → DriftStatus::Warning; model remains approved (Warning does not auto-revoke)
3. Phase 3: 5 invocations at confidence 0.62 (drop = 0.27 > drift_threshold 0.20)
4. check_drift → DriftStatus::Drifted
5. check_and_update() → registry.revoke("sepsis-risk-v2.1", "auto-revoked: model drift detected")
6. Phase 4: SepsisRiskAgentRevoked routes to resource "sepsis-model-revoked"
7. Policy rule deny-sepsis-scoring-revoked → Deny
8. Human review required before re-approval and re-registration
```

The Warning phase provides an observation window before irreversible revocation. Only `Drifted` triggers auto-revocation.

---

## 11. Appendix: Crate Dependency Graph

```
veritas-contracts          (no dependencies — shared types only)
    │
    ├── veritas-core       (traits + Executor)
    │       │
    │       ├── veritas-policy       (TomlPolicyEngine impl PolicyEngine)
    │       ├── veritas-audit        (InMemoryAuditWriter impl AuditWriter)
    │       └── veritas-verify       (SchemaVerifier impl Verifier)
    │
    ├── veritas-model      (ModelRegistry, InMemoryDriftMonitor)
    │
    ├── veritas-ref-healthcare   (7 scenarios, 4 policy files)
    │       │  depends on: veritas-core, veritas-policy, veritas-audit,
    │       │              veritas-verify, veritas-model
    │       │
    │       ├── demo             (CLI runner, clap)
    │       └── tui              (interactive TUI, ratatui + veritas-model)
    │
    └── (all crates depend on veritas-contracts)
```

**Workspace:** 9 members. **Tests:** 131 across all crates.

---

*VERITAS Yellow Paper v0.2 — 2026-03-18*
*Licensed under Apache License 2.0*
