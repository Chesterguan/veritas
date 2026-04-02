# VERITAS — Trusted Agent Execution Runtime

**Whitepaper v0.3**

*Lightweight · Deterministic · Auditable · Verifiable*

> Last updated: 2026-04-02

---

## Table of Contents

1. [Vision](#1-vision)
2. [Motivation](#2-motivation)
3. [Positioning](#3-positioning)
4. [Design Philosophy](#4-design-philosophy)
5. [ZeroClaw and OpenClaw Lineage](#5-zeroclaw-and-openclaw-lineage)
6. [System Model](#6-system-model)
7. [Trust Model](#7-trust-model)
8. [Capability Model](#8-capability-model)
9. [Policy and Governance](#9-policy-and-governance)
10. [Audit and Traceability](#10-audit-and-traceability)
11. [Verification Model](#11-verification-model)
12. [Security Model](#12-security-model)
13. [Data Model Independence](#13-data-model-independence)
14. [Reference Domain: Healthcare](#14-reference-domain-healthcare)
15. [Landscape and Differentiation](#15-landscape-and-differentiation)
16. [Extensibility](#16-extensibility)
17. [Limitations](#17-limitations)

---

## 1. Vision

VERITAS is a lightweight, deterministic, policy-bound, auditable, and verifiable execution runtime for AI agents operating in regulated environments. The system prioritizes trust, control, and evidence over autonomy and opaque intelligence.

VERITAS is designed as a foundational execution layer — not an application, not an automation product, and not a heavy governance platform. It makes existing agent runtimes trustworthy without making them slow.

## 2. Motivation

Modern AI agents are powerful but fundamentally unreliable in regulated environments. Their behavior is often non-deterministic, non-auditable, and difficult to verify. Frameworks like OpenClaw and ZeroClaw have proven that agents can be fast, lightweight, and deployable anywhere — but they were not designed for environments where every action must be traceable, policy-constrained, and verifiable.

At the same time, enterprise governance solutions approach the problem from the opposite direction: they add heavy middleware, complex rule engines, and slow validation pipelines that destroy the speed and simplicity that made lightweight agents useful in the first place.

VERITAS takes a different path. Instead of rebuilding agents from scratch or wrapping them in bureaucracy, VERITAS provides a thin trusted execution layer that makes good agents better — safe, auditable, and verifiable — while preserving the lightweight, fast, and composable nature of the runtimes they already run on.

## 3. Positioning

### What VERITAS Is

- A trusted agent execution runtime
- A lightweight policy-enforcement and audit layer
- A trust boundary between agents and the real world
- A foundation for regulated AI agent deployment

### What VERITAS Is NOT

- Not an AI assistant or chatbot
- Not an automation platform
- Not a healthcare system or clinical tool
- Not a data platform or ETL pipeline
- Not a heavy governance middleware
- Not a replacement for agent frameworks — it wraps them

### The Red Hat Analogy

The Linux kernel is fast, minimal, and runs everywhere. Red Hat did not replace it or slow it down — it made it enterprise-ready by adding trust, support, certification, and governance around a proven foundation.

VERITAS follows the same model:

```
Linux Kernel        →  ZeroClaw / OpenClaw    (fast, minimal, runs anywhere)
Red Hat Enterprise  →  VERITAS                (trusted, governed, auditable)
```

Agent developers should not feel the weight of governance. Building on VERITAS should feel like building on ZeroClaw — fast, simple, composable — with trust enforced transparently by the runtime, not by the application code.

## 4. Design Philosophy

The ten principles that govern all VERITAS design decisions:

1. **Control over autonomy** — the system, not the LLM, decides what happens
2. **Evidence over intelligence** — provable output beats clever reasoning
3. **Determinism over emergence** — predictable behavior, always
4. **Deny by default** — nothing executes unless explicitly permitted
5. **Capability-based security** — fine-grained, declarative, composable
6. **Minimal trusted computing base** — small surface, fewer bugs, easier to audit
7. **Auditability by design** — not bolted on, built in from day one
8. **Verifiable execution** — every output can be independently validated
9. **Human override always possible** — machines propose, humans dispose. The runtime supports this through the `RequireApproval` policy verdict, which suspends execution and returns control to the hosting application for human review. The approval mechanism itself (UI, notification, approval ledger) is the responsibility of the hosting application, not the runtime.
10. **Data-model independence** — no coupling to FHIR, OMOP, or any domain schema

And one meta-principle above all:

> **Lightweight by conviction.** Governance must not be the reason agents become slow, heavy, or hard to build. If VERITAS makes the developer experience worse, VERITAS has failed.

## 5. ZeroClaw and OpenClaw Lineage

VERITAS inherits its execution philosophy from two open-source projects that proved agents can be fast, small, and practical:

### ZeroClaw

ZeroClaw is an ultra-lightweight AI agent runtime written in Rust. It boots in under 10ms, ships as a ~3.4MB binary, runs on ARM/x86/RISC-V, and deploys on hardware as cheap as $10. ZeroClaw emphasizes a minimal agent kernel, explicit execution flow, trait-based composability, zero external dependencies, and a small trusted computing base.

### OpenClaw

OpenClaw brought the agent-as-personal-assistant model to the mainstream — persistent, always-on, multi-channel (WhatsApp, Slack, Telegram, etc.), self-hosted, and extensible through 100+ AgentSkills. OpenClaw proved that AI agents are not toys or demos; they are infrastructure.

### What VERITAS Inherits

From ZeroClaw:
- Minimal kernel design
- Explicit execution flow (no hidden state)
- Composable trait-based architecture
- Small trusted computing base
- Portability and low resource requirements

From OpenClaw:
- Agent-as-infrastructure mindset
- Extensibility through skills/capabilities
- Real-world deployment patterns
- Multi-channel, always-on operation model

### What VERITAS Adds

What neither ZeroClaw nor OpenClaw were designed to provide — and what regulated environments demand:

- **Deny-by-default policy engine** — every action policy-checked before execution
- **Immutable audit trail** — append-only event stream, tamper-evident, hash-chained
- **Output verification** — schema, rule, and risk validation before delivery
- **Formal trust boundary** — LLM, tools, and data treated as untrusted by architecture
- **Capability-level security** — permissions and side-effect declarations per tool
- **Human override hooks** — approval workflows for sensitive operations

VERITAS does not fork or replace ZeroClaw. It layers trust on top of the same lightweight foundation.

## 6. System Model

Agent execution in VERITAS is modeled as a deterministic state machine operating over controlled capabilities.

### Execution Loop

```
State → Policy → Capability → Propose → Verify → Transition → Audit
```

Each step is explicit, policy-checked, and audited regardless of outcome. Successful outputs are verified before the agent advances to the next state. The loop is intentionally minimal — no hidden middleware, no heavy orchestration, no latency-adding abstraction layers.

### State Machine Properties

- **Deterministic** — the pipeline structure and policy evaluation are deterministic: same input + same policy = same execution path. Agent behavior inside `propose()` is untrusted and may be non-deterministic (e.g., LLM-backed agents). VERITAS does not verify or enforce determinism of agent outputs.
- **Observable** — every state transition is an audit event
- **Interruptible** — human override can halt or redirect at any transition
- **Auditable** — execution traces provide the evidence to audit past runs. There is no replay mechanism in v0; the audit trail establishes what happened, not a means to re-execute it.

## 7. Trust Model

Trust in VERITAS is derived from deterministic execution, immutable audit trails, explicit policy decisions, and verifiable outputs.

The system does not inherently trust LLM reasoning, external tools, input data, or execution environments. This is not paranoia — it is architectural honesty. LLMs hallucinate, tools have bugs, data can be poisoned, and environments can be compromised.

### Trust Boundary

| Trusted | Untrusted |
|---------|-----------|
| Runtime core | LLM |
| Policy engine | Tools |
| Audit engine | Input data |
| Verifier | External environment |

Trust is not assumed. Trust is derived from evidence — audit trails, policy logs, and verification results.

### How Trust Is Enforced

Trust boundaries are enforced by code structure, not by configuration:

- **Policy before proposal** — the policy engine runs before `agent.propose()`. Without an Allow verdict and the required capabilities, the proposal path is structurally unreachable.
- **Append-only audit trail** — the audit writer is append-only with SHA-256 hash chaining. Tampering with any record breaks chain integrity and is detectable during verification.
- **Verifier between proposal and transition** — the verifier runs after `propose()` but before `transition()`. Outputs that fail verification never reach the agent's next state.
- **Capabilities are immutable at runtime** — capabilities are granted at startup and never elevated during execution (INV-10). There is no runtime escalation path.

The boundary is structural. An agent that lacks the required capability cannot reach the code path that calls the tool — there is no permission check to bypass.

## 8. Capability Model

Capabilities represent constrained tools with explicit schemas, permissions, and side-effect declarations.

All interactions with the external world must occur through capabilities under policy control. A capability is not just a function call — it is a contract that declares:

- What it does (schema)
- What it requires (permissions)
- What it changes (side effects)
- What risks it carries (risk level)

This is capability-based security: agents cannot access anything that has not been explicitly granted as a capability, and every capability invocation is policy-checked and audited.

## 9. Policy and Governance

VERITAS enforces deny-by-default execution. Policy decisions evaluate subject, action, resource, and context to determine one of three outcomes:

- **Allow** — action proceeds, audit event recorded
- **Deny** — action blocked, audit event recorded with reason
- **Require Approval** — action suspended pending human review

Policy is deterministic, explainable, and auditable. Every policy decision can be traced back to a specific rule, and every rule can be inspected by a human auditor.

### Policy Rule Format

Rules are declared in TOML and evaluated in declaration order (first-match-wins). If no rule matches, the default verdict is Deny. Rules match on (action, resource) pairs with exact or wildcard matching.

Example rule:

```toml
[[rules]]
id = "allow-drug-interaction-check"
description = "Agent may query the drug interaction database"
action = "drug-interaction-check"
resource = "drug-database"
required_capabilities = ["drug-database.read"]
verdict = "allow"
```

### Lightweight by Design

The policy engine is not a heavyweight business-rules platform. It is a fast, deterministic evaluator — closer to a firewall than to an enterprise workflow engine. Policy evaluation is designed to be fast — O(n) linear scan over rules, with no I/O in the evaluation path. No performance benchmarks are provided in v0.

## 10. Audit and Traceability

All execution events are recorded in an append-only event stream forming a verifiable execution graph. Each event contains:

- State transitions
- Capability calls
- Policy decisions
- Verification results
- Timestamps and causal ordering

The chain is tamper-evident: each record includes a SHA-256 hash over the record and its predecessor, making any modification detectable.

### Hash Chain Construction

Each audit record's hash is computed as:

```
SHA-256( execution_id || sequence (8-byte little-endian) || prev_hash || canonical JSON of record )
```

The genesis hash (first event in an execution) uses 64 hex zeros as `prev_hash`. Chain verification recomputes each hash and checks that each record's `prev_hash` matches the hash of the preceding record.

**Reference implementation.** The v0 implementation uses an in-memory store (`InMemoryAuditWriter`). It does not persist across process restarts. Persistent backends — database, filesystem, blockchain — can be plugged in via the `AuditWriter` trait but are not included in v0.

**Known limitation.** Hash construction depends on `serde_json::to_vec()` field ordering, which may vary across VERITAS versions. Chains created by different versions may not be cross-version verifiable. A future hardening pass should adopt explicit canonical serialization (e.g., RFC 8785 JCS).

### Why This Matters

In regulated environments, "it worked" is not enough. You must prove *how* it worked, *why* each decision was made, and *what* was checked. The audit trail is not a log file — it is the evidence that the system behaved correctly.

## 11. Verification Model

Every agent output passes through a two-phase verifier before it can trigger a state transition.

**Phase 1 — Structural validation.** The output is validated against a JSON Schema. Structural errors (missing required fields, wrong types, disallowed properties) are caught here.

**Phase 2 — Semantic rule evaluation.** The structurally valid output is evaluated against a set of semantic rules. Rule types include:

- **RequiredField** — a specified field must be present and non-empty
- **AllowedValues** — a field value must belong to a declared set
- **ForbiddenPattern** — a field value must not match a given pattern
- **Custom** — domain-specific validation logic (e.g., PII detection in clinical notes, formulary membership checks)

All failures across both phases are accumulated before returning. The caller receives the full set of violations, not just the first one.

Verification failure is a hard stop — the output is rejected and never reaches the agent's state transition. There is no partial acceptance or conditional pass-through. The hosting application is responsible for retry logic, escalation, or human review.

## 12. Security Model

VERITAS enforces least privilege, isolated capability execution, and strict boundary control.

The runtime does not allow direct system access and assumes all external components are untrusted. Security is not a feature added on top — it is a consequence of the architecture:

- Deny-by-default policy → no unauthorized actions
- Capability-based access → no ambient authority
- Immutable audit trail → no undetected tampering
- Output verification → no unvalidated deliverables
- Trust boundary → no implicit trust in LLM or tools

## 13. Data Model Independence

The VERITAS core runtime is independent of specific healthcare or enterprise data models such as FHIR, OMOP, HL7, or proprietary schemas.

Domain-specific adapters are implemented externally via capabilities. This means VERITAS can govern agents operating in healthcare, finance, legal, or any other regulated domain — without the core runtime knowing or caring about the domain's data model.

The core speaks capabilities, policies, and audit events. The domain speaks whatever it needs to — through adapters that are themselves capabilities, subject to the same policy and audit controls as everything else.

## 14. Reference Domain: Healthcare

While VERITAS is domain-independent by design, its reference implementation targets **healthcare** — one of the most heavily regulated and highest-stakes environments for AI agent deployment.

### Why Healthcare

Healthcare is where the consequences of uncontrolled AI agents are most severe:

- A wrong drug interaction check can harm a patient
- An unauthorized data access can violate HIPAA/GDPR
- An unaudited clinical decision cannot be defended in court
- A non-deterministic output cannot be reproduced for review

If VERITAS can earn trust in healthcare, it can earn trust anywhere.

### Healthcare-Specific Challenges VERITAS Addresses

| Challenge | VERITAS Response |
|---|---|
| Patient data sensitivity | Capability-based access — agents only see what policy permits |
| Regulatory compliance (HIPAA, GDPR, MDR) | Immutable audit trail proves every access and decision |
| Clinical decision support | Output verification — every recommendation validated before delivery |
| Interoperability (FHIR, HL7, OMOP) | Domain adapters as capabilities — core stays clean |
| Human oversight requirements | Require Approval policy — clinician review for sensitive operations |
| Reproducibility for audits | Tamper-evident audit trail — provides the evidence to review and audit any past agent run |

### What VERITAS Does NOT Do in Healthcare

- Does not interpret clinical data
- Does not make clinical decisions
- Does not replace clinician judgment
- Does not store or manage patient records
- Does not implement FHIR/HL7 — adapters do

VERITAS governs the **agent** that does these things. It ensures the agent operates within policy, produces auditable evidence, and delivers verifiable outputs — regardless of what clinical system it connects to.

## 15. Landscape and Differentiation

### Agent Frameworks (Build Agents)

Frameworks like LangGraph, CrewAI, AutoGen, and OpenClaw help developers build agents. They focus on orchestration, tool use, and multi-agent coordination. They are not competitors — they are potential **consumers** of VERITAS. An agent built with any framework can run inside the VERITAS runtime.

### Guardrail Systems (Filter I/O)

Systems like Guardrails AI, LlamaFirewall, and Superagent validate inputs and outputs. They catch prompt injections, unsafe content, and schema violations. They are useful but incomplete — they filter at the boundary without controlling execution itself.

### Enterprise Governance (Policy Documents)

Frameworks like NIST AI RMF, Singapore's Model AI Governance, and AAGATE provide governance principles and assessment methodologies. They describe *what* should happen. VERITAS implements *how* it happens at runtime.

### Where VERITAS Sits

```
┌─────────────────────────────────────────────────────┐
│              Application / Agent Code               │
│         (LangGraph, CrewAI, OpenClaw, etc.)         │
├─────────────────────────────────────────────────────┤
│                    VERITAS                           │
│   Policy Engine │ Audit Trail │ Verifier │ Caps     │
├─────────────────────────────────────────────────────┤
│              Agent Runtime Kernel                    │
│            (ZeroClaw or equivalent)                  │
└─────────────────────────────────────────────────────┘
```

VERITAS is the **middle layer** — below the application, above the kernel. It adds trust without adding weight.

## 16. Extensibility

VERITAS provides standardized interfaces for:

- Capabilities (domain-specific tools and adapters)
- Policy engines (custom rule sets per environment)
- Audit storage (pluggable backends — local, cloud, blockchain)
- Verification modules (custom validators per domain)

External contributors may extend the system without modifying the trusted core. The extension model follows the same principle as ZeroClaw's trait-based architecture: composable, swappable, and minimal.

## 17. Limitations

The following limitations are documented to help adopters make informed decisions about fitness for purpose.

- **Policy engine scope.** Rules match on (action, resource) pairs. There is no field-level conditional logic, no time-based rules, and no rule composition (AND/OR). Complex policy decisions must be modeled through action/resource routing in the agent.

- **In-memory audit trail.** The reference `InMemoryAuditWriter` does not persist across process restarts. Production deployments must implement a persistent `AuditWriter` backend.

- **Verification failure is terminal.** Failed verification halts execution with no recovery, escalation, or remediation path within the runtime. The hosting application must handle retry or human escalation.

- **Determinism scope.** The pipeline structure and policy evaluation are deterministic. Agent behavior inside `propose()` is untrusted and may be non-deterministic (e.g., LLM inference). VERITAS does not verify or enforce determinism of agent outputs.

- **Hash chain versioning.** Audit chain integrity depends on `serde_json` field ordering. Chains created by different VERITAS versions may not be cross-version verifiable. A future hardening pass should adopt explicit canonical serialization.

- **Model drift baseline is immutable.** Once the drift baseline is computed from the first observation window, it is never updated. Legitimate model improvements cannot be captured without re-registration.

- **No performance benchmarks.** The whitepaper claims lightweight operation but provides no measured benchmarks for policy evaluation, audit writing, or verification overhead in v0.

---

*End of VERITAS Whitepaper v0.3*
