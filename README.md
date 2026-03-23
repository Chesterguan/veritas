<p align="center">
  <img src="assets/logo.jpeg" alt="VERITAS" width="600">
</p>

<p align="center">
  <img src="https://img.shields.io/badge/license-Apache%202.0-blue" alt="License">
  <img src="https://github.com/Chesterguan/veritas/actions/workflows/ci.yml/badge.svg" alt="Build">
  <img src="https://img.shields.io/badge/tests-131%20passing-green" alt="Tests">
  <img src="https://img.shields.io/badge/rust-1.85%2B-orange" alt="Rust">
</p>

<p align="center">
  <strong>Making good agents better — safe, auditable, and verifiable — without making them slow.</strong>
</p>

---

Lightweight, deterministic, policy-bound, auditable, and verifiable execution runtime for AI agents operating in regulated environments.

> Reference domain: Healthcare

## Quick Start

```bash
git clone https://github.com/Chesterguan/veritas.git
cd veritas
cargo test --workspace       # 131 tests, all passing
cargo run -p demo -- run-all # run all 7 healthcare scenarios
```

<p align="center">
  <img src="assets/demo.gif" alt="VERITAS Healthcare Demo" width="900">
</p>

Or launch the interactive TUI:

```bash
cargo run -p veritas-tui
```

The TUI lets you select scenarios (1, 4, 5, 6, 7) and watch VERITAS enforce policy, model governance, and drift detection in real time.

**Prerequisites:** Rust 1.85+ ([install](https://rustup.rs/))

## Why VERITAS

Agent runtimes like [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw) and [OpenClaw](https://github.com/openclaw/openclaw) proved that AI agents can be fast, tiny, and deployable anywhere. But they were not built for environments where every action must be traceable, policy-constrained, and verifiable.

VERITAS does not replace them. It wraps them with trust.

```
Linux Kernel        →  ZeroClaw / OpenClaw    (fast, minimal, runs anywhere)
Red Hat Enterprise  →  VERITAS                (trusted, governed, auditable)
```

## Architecture

<p align="center">
  <img src="assets/arch.jpg" alt="VERITAS Architecture" width="900">
</p>

### Execution Model

Every agent action follows the same deterministic pipeline — no exceptions, no shortcuts:

```
State → Policy → Capability → Audit → Verify → Next State
```

### Trust Boundary

| Trusted | Untrusted |
|---------|-----------|
| Runtime core | LLM |
| Policy engine | Tools |
| Audit engine | Input data |
| Verifier | External environment |

## Core Components

| Crate | Purpose | Tests |
|-------|---------|-------|
| [`veritas-contracts`](crates/veritas-contracts) | Shared types, traits, error types | 27 |
| [`veritas-core`](crates/veritas-core) | Deterministic executor pipeline | 9 |
| [`veritas-policy`](crates/veritas-policy) | TOML deny-by-default policy engine | 8 |
| [`veritas-audit`](crates/veritas-audit) | SHA-256 hash-chained audit trail | 6 |
| [`veritas-verify`](crates/veritas-verify) | JSON Schema + semantic rule verification | 10 |
| [`veritas-model`](crates/veritas-model) | Model registry, drift detection, model governance | 31 |
| [`veritas-ref-healthcare`](crates/veritas-ref-healthcare) | Healthcare reference runtime (7 scenarios) | 40 |

## Healthcare Demo Scenarios

| # | Scenario | What it demonstrates |
|---|----------|---------------------|
| 1 | **Drug Interaction Checker** | Policy Allow flow, output schema verification |
| 2 | **Clinical Note Summarizer** | PII detection via custom verifier rule |
| 3 | **Patient Data Query** | Capability-based access control, consent enforcement |
| 4 | **Multi-Agent Clinical Pipeline** | 4-agent chain with independent audit trails |
| 5 | **Prior Authorization Workflow** | RequireApproval lifecycle with physician approval |
| 6 | **Radiology AI Model Governance** | ImageToLabel model with approval gate and registry |
| 7 | **Sepsis Risk Model with Drift** | TabularToScore model with drift detection and alerting |

Run individually:

```bash
cargo run -p demo -- drug-interaction
cargo run -p demo -- note-summarizer
cargo run -p demo -- patient-query
cargo run -p demo -- clinical-pipeline
cargo run -p demo -- prior-auth
cargo run -p demo -- radiology-model
cargo run -p demo -- sepsis-model
```

## Design Principles

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

> **Lightweight by conviction.** Governance must not be the reason agents become slow, heavy, or hard to build.

## Project Structure

```
crates/
  veritas-contracts/       # Shared types, traits, error types
  veritas-core/            # Deterministic executor pipeline
  veritas-policy/          # TOML deny-by-default policy engine
  veritas-audit/           # SHA-256 hash-chained audit trail
  veritas-verify/          # JSON Schema + semantic rule verification
  veritas-model/           # Model registry, drift detection, model governance
  veritas-ref-healthcare/  # Healthcare reference runtime (7 scenarios)
demo/                      # CLI demo runner (clap)
tui/                       # Interactive TUI demo (ratatui)
docs/
  whitepaper/              # Whitepaper v0.3 (EN, ZH, JA, FR)
  yellowpaper/             # Yellow Paper v0.2 (EN)
```

## Documentation

| Document | Description |
|----------|-------------|
| [Whitepaper v0.3](docs/whitepaper/WHITEPAPER.en.md) | Vision, design philosophy, system architecture |
| [Yellow Paper v0.2](docs/yellowpaper/YELLOWPAPER.en.md) | Formal execution semantics and specifications |
| [docs/](docs/README.md) | Full documentation index |

### Whitepaper Translations

| Language | Link |
|----------|------|
| English | [WHITEPAPER.en.md](docs/whitepaper/WHITEPAPER.en.md) |
| 简体中文 | [WHITEPAPER.zh.md](docs/whitepaper/WHITEPAPER.zh.md) |
| 日本語 | [WHITEPAPER.ja.md](docs/whitepaper/WHITEPAPER.ja.md) |
| Français | [WHITEPAPER.fr.md](docs/whitepaper/WHITEPAPER.fr.md) |

## Contributing

VERITAS is open source. Community contributions — including new translations — are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## License

Licensed under Apache License 2.0. See [LICENSE](LICENSE) for details.
