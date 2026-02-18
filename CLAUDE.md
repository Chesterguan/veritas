# VERITAS — Project Instructions

> Last updated: 2026-02-18

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

## Project Structure

```
crates/
  veritas-contracts/       # Shared types, traits, error types (15 tests)
  veritas-core/            # Executor pipeline (6 tests)
  veritas-policy/          # TOML deny-by-default policy engine (8 tests)
  veritas-audit/           # SHA-256 hash-chained audit trail (6 tests)
  veritas-verify/          # JSON Schema + semantic rule verification (10 tests)
  veritas-ref-healthcare/  # Healthcare reference runtime (3 scenarios)
demo/                      # CLI demo runner (clap)
tui/                       # Interactive TUI demo (ratatui 0.29 + crossterm 0.28)
assets/                    # Logo and demo GIF
docs/                      # Whitepaper v0.3 (EN, ZH, JA, FR)
```

## Core Components

| Component | Purpose |
|-----------|---------|
| `veritas-contracts` | Capability / policy / audit schemas, shared types |
| `veritas-core` | Deterministic runtime executor (ZeroClaw lineage) |
| `veritas-policy` | Deny-by-default permission & risk engine |
| `veritas-audit` | Immutable, append-only execution trace |
| `veritas-verify` | Output validation before delivery |

## Execution Model

```
State → Policy → Capability → Audit → Verify → Next State
```

## Key Crate Versions

- `thiserror = "2.0"` (not 1.0)
- `jsonschema = "0.28"` (API: `validator_for()` + `iter_errors()`, NOT `JSONSchema::compile()`)
- `ratatui = "0.29"` + `crossterm = "0.28"` (TUI)
- `clap = "4.0"` (CLI)

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
- Core runtime is fully synchronous — no tokio in the trusted path
- Agent.propose() NEVER runs unless policy returns Allow (structural guarantee in Executor)
