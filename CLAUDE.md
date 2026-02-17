# VERITAS — Project Instructions

> Last updated: 2026-02-17

## Overview

VERITAS is a lightweight, deterministic, policy-bound, auditable, and verifiable execution runtime for AI agents in regulated environments. Its reference domain is **healthcare**.

VERITAS inherits its execution philosophy from ZeroClaw (minimal Rust agent kernel) and OpenClaw (agent-as-infrastructure). It does NOT replace agent frameworks — it wraps them with a trust layer.

Think: Red Hat is to Linux what VERITAS is to ZeroClaw/OpenClaw.

## What VERITAS Is NOT

- NOT an AI assistant or chatbot
- NOT an automation platform
- NOT a healthcare system or clinical tool
- NOT a data platform
- NOT a heavy governance middleware
- NOT a replacement for agent frameworks

## Design Principles (always follow)

1. Control over autonomy
2. Evidence over intelligence
3. Determinism over emergence
4. Deny by default
5. Capability-based security
6. Minimal trusted computing base
7. Auditability by design
8. Verifiable execution
9. Human override always possible
10. Data-model independence

**Meta-principle: Lightweight by conviction.** If a change makes the system heavier, slower, or harder to build on — push back. Governance must not destroy developer experience.

## Trust Boundary

- **Trusted:** Runtime core, Policy engine, Audit engine, Verifier
- **Untrusted:** LLM, Tools, Input data, External environment

## Core Components

| Component | Purpose |
|-----------|---------|
| `veritas-core` | Deterministic runtime (ZeroClaw lineage) |
| `veritas-policy` | Deny-by-default permission & risk engine |
| `veritas-audit` | Immutable, append-only execution trace |
| `veritas-verify` | Output validation before delivery |
| `veritas-contracts` | Capability / policy / audit schemas |

## Execution Model

```
State → Policy → Capability → Audit → Verify → Next State
```

## Code Guidelines

- All agent actions must go through the policy engine (deny-by-default)
- All execution must produce audit events (append-only)
- All outputs must be verified before delivery
- Never trust LLM outputs directly — always verify
- Keep the trusted computing base minimal
- External integrations must be implemented as capabilities, not direct calls
- Healthcare-specific logic belongs in domain adapters (capabilities), NOT in the core
- Policy evaluation must be fast — microseconds, not milliseconds
- Prefer simplicity over abstraction — three similar lines beat a premature helper
- Follow ZeroClaw's principle: explicit over implicit, small over large
