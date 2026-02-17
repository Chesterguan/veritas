# VERITAS — Project Instructions

> Last updated: 2026-02-17

## Overview

VERITAS is a deterministic, policy-bound, auditable, and verifiable execution runtime for AI agents in regulated environments. It is NOT an AI assistant, automation platform, or healthcare system.

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

## Trust Boundary

- **Trusted:** Runtime core, Policy engine, Audit engine, Verifier
- **Untrusted:** LLM, Tools, Input data, External environment

## Core Components

| Component | Purpose |
|-----------|---------|
| `veritas-core` | Deterministic runtime (ZeroClaw lineage) |
| `veritas-policy` | Permission & risk engine |
| `veritas-audit` | Immutable execution trace |
| `veritas-verify` | Output validation |
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
