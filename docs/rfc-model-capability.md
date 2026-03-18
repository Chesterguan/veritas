# RFC: Generalized ModelCapability — Governing All AI Models, Not Just LLMs

> Date: 2026-03-11
> Status: Implemented (Phase 1-4)
> Author: ClinicClaw/VERITAS team

## Problem

VERITAS currently governs **agent actions** but is model-agnostic at the capability level — the `Capability` type is an opaque string (`"phi:read"`, `"order:submit"`). This works, but it means:

1. **No model-level governance** — VERITAS can gate _what_ an agent does, but not _which model_ it uses or _how_ that model behaves
2. **LLM-only assumption** — ClinicClaw's `LlmCapability` trait is hardcoded for text-in/text-out; vision models, tabular ML, time-series forecasting don't fit
3. **No drift detection** — a model silently degrading (accuracy dropping, distribution shift) is invisible to the trust layer
4. **No model registry** — which model version produced which output? Not tracked at the VERITAS level
5. **No federated governance** — multi-institution deployments need model approval chains that cross organizational boundaries

## Proposal

Add a `ModelCapability` trait to `veritas-contracts` that generalizes model governance beyond LLMs.

### Core Abstraction

```rust
/// Describes a model that can be governed by VERITAS
pub trait ModelDescriptor: Send + Sync {
    /// Unique model identifier (e.g., "claude-sonnet-4-5-20250514", "chest-xray-v3.2")
    fn model_id(&self) -> &str;

    /// Model modality — what kind of input/output
    fn modality(&self) -> ModelModality;

    /// Version string for registry tracking
    fn version(&self) -> &str;

    /// Model provenance — where it came from, how it was trained
    fn provenance(&self) -> &ModelProvenance;
}

pub enum ModelModality {
    TextToText,           // LLMs (Claude, GPT, Ollama)
    ImageToText,          // Vision models (radiology, pathology)
    ImageToLabel,         // Classification (skin lesion, retinal scan)
    TabularToScore,       // Risk scores (sepsis, readmission)
    TimeSeriestoAlert,    // Monitoring (ECG, vitals trend)
    MultiModal,           // Combined input types
    Custom(String),       // Extensible
}

pub struct ModelProvenance {
    pub vendor: String,              // "anthropic", "internal", "huggingface"
    pub training_data_hash: Option<String>,  // reproducibility
    pub approval_status: ApprovalStatus,
    pub approved_by: Option<String>,  // governance chain
    pub approved_at: Option<DateTime<Utc>>,
    pub regulatory_class: Option<String>,  // "FDA-510k", "CE-IVD", "research-only"
}

pub enum ApprovalStatus {
    Approved,
    Pending,
    Revoked { reason: String },
    Experimental,  // allowed in non-production only
}
```

### ModelCapability Trait

```rust
/// A capability that wraps model invocation with governance
pub trait ModelCapability: Send + Sync {
    type Input;
    type Output;

    /// The model this capability governs
    fn descriptor(&self) -> &dyn ModelDescriptor;

    /// Invoke the model — ONLY callable through VERITAS pipeline
    fn invoke(&self, input: &Self::Input) -> VeritasResult<ModelResult<Self::Output>>;

    /// Pre-invocation check — can this model handle this input?
    fn validate_input(&self, input: &Self::Input) -> VeritasResult<()>;
}

pub struct ModelResult<T> {
    pub output: T,
    pub confidence: Option<f64>,      // model self-reported confidence
    pub latency_ms: u64,
    pub token_usage: Option<TokenUsage>,  // for LLMs
    pub model_id: String,             // which exact model produced this
    pub model_version: String,
}
```

### Model Registry

```rust
/// Central registry of approved models
pub struct ModelRegistry {
    models: HashMap<String, Box<dyn ModelDescriptor>>,
}

impl ModelRegistry {
    /// Register a model — requires governance approval
    pub fn register(&mut self, descriptor: Box<dyn ModelDescriptor>) -> VeritasResult<()>;

    /// Check if a model is approved for use
    pub fn is_approved(&self, model_id: &str) -> bool;

    /// Revoke a model (drift detected, safety issue, etc.)
    pub fn revoke(&mut self, model_id: &str, reason: &str) -> VeritasResult<()>;

    /// List all models by modality
    pub fn by_modality(&self, modality: &ModelModality) -> Vec<&dyn ModelDescriptor>;
}
```

### Drift Detection Hook

```rust
/// Hook for monitoring model behavior over time
pub trait DriftMonitor: Send + Sync {
    /// Record a model invocation result for drift analysis
    fn record(&self, model_id: &str, result: &dyn std::any::Any) -> VeritasResult<()>;

    /// Check if model has drifted beyond acceptable bounds
    fn check_drift(&self, model_id: &str) -> DriftStatus;
}

pub enum DriftStatus {
    Stable,
    Warning { metric: String, current: f64, threshold: f64 },
    Drifted { metric: String, current: f64, threshold: f64 },
}
```

### Integration with Existing Pipeline

The key insight: **ModelCapability slots into the existing Capability system without breaking it.**

```
State → Policy → Capability Check → Agent.propose() → Verify → Audit
                      ↓
              CapabilitySet now includes:
              - "phi:read" (existing opaque capabilities)
              - "model:claude-sonnet-4-5-20250514" (model-specific)
              - "model:chest-xray-v3.2" (non-LLM model)
                      ↓
              Policy rules can now gate by model:
              - "only approved models for clinical decisions"
              - "experimental models only in sandbox phase"
              - "revoked models blocked everywhere"
```

**Policy rule example (TOML):**
```toml
[[rules]]
id = "require-approved-model"
description = "Only approved models may produce clinical outputs"
action = "model:invoke"
resource = "*"
required_capabilities = ["model:approved"]
verdict = "allow"

[[rules]]
id = "block-revoked-models"
description = "Revoked models are blocked from all invocations"
action = "model:invoke"
resource = "model:revoked:*"
verdict = "deny"
deny_reason = "Model has been revoked — check registry for details"
```

**Audit trail enhancement:**
```rust
// StepRecord already captures input/output as serde_json::Value
// ModelResult metadata gets serialized into the output payload:
{
  "kind": "model_invocation",
  "model_id": "chest-xray-v3.2",
  "model_version": "3.2.1",
  "modality": "ImageToLabel",
  "confidence": 0.94,
  "latency_ms": 230,
  "provenance": { "vendor": "internal", "regulatory_class": "FDA-510k" }
}
```

### Federated Governance (Future)

For multi-institution deployments:
- Each institution maintains its own `ModelRegistry`
- A `FederatedRegistry` aggregates approval status across institutions
- Model approval requires N-of-M institutional sign-off
- Revocation propagates immediately (any institution can revoke)

## Implementation Plan

### Phase 1: Core Types (veritas-contracts)
- [x] Add `ModelModality`, `ModelProvenance`, `ApprovalStatus` to contracts
- [x] Add `ModelDescriptor` trait
- [x] Add `ModelResult<T>` struct
- [x] Add `ModelCapability` trait

### Phase 2: Model Registry (new crate: veritas-model)
- [x] `ModelRegistry` with register/revoke/query
- [x] Integration with `CapabilitySet` — auto-grant `model:<id>` capabilities
- [x] Policy rules for model-level governance

### Phase 3: Drift Detection (veritas-model)
- [x] `DriftMonitor` trait
- [x] `InMemoryDriftMonitor` reference implementation
- [x] Hook into audit pipeline — record model results for drift analysis

### Phase 4: Healthcare Reference Scenarios (veritas-ref-healthcare)
- [x] Scenario: Radiology AI (ImageToLabel) with approval gate
- [x] Scenario: Sepsis risk model (TabularToScore) with drift detection
- [x] Scenario: Multi-model pipeline (LLM + vision) with model registry

### Phase 5: ClinicClaw Integration
- [ ] Migrate `LlmCapability` to implement `ModelCapability`
- [ ] Register Claude/Ollama/Mock in `ModelRegistry`
- [ ] Add model metadata to SSE events and audit trail
- [ ] Demo: show model governance in hospital simulation

## Design Principles

1. **Backwards compatible** — existing opaque `Capability` strings still work; `ModelCapability` is additive
2. **Model-agnostic** — works for LLMs, vision, tabular, time-series, custom
3. **Governance-first** — approval status is mandatory, not optional
4. **Auditable** — every model invocation is recorded with full provenance
5. **Drift-aware** — models degrade; governance must detect and respond
6. **Federated-ready** — designed for multi-institution from day one

## Open Questions

1. Should `ModelCapability::invoke()` be sync (like VERITAS core) or async (like ClinicClaw)? Probably need both — sync trait in contracts, async adapter in cliniclaw.
2. How granular should model capabilities be? `model:claude-sonnet-4-5-20250514` vs `model:claude:*` vs `model:llm:*`?
3. Should drift detection be in the critical path (blocking) or async (alerting)?
4. How to handle model ensembles — one capability per model, or one for the ensemble?
