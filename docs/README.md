# VERITAS Documentation

> Last updated: 2026-02-18

## Whitepaper v0.3

The whitepaper describes VERITAS vision, design philosophy, system architecture, core models, healthcare reference domain, and competitive positioning.

| Language | File |
|----------|------|
| English | [WHITEPAPER.en.md](whitepaper/WHITEPAPER.en.md) |
| 简体中文 | [WHITEPAPER.zh.md](whitepaper/WHITEPAPER.zh.md) |
| 日本語 | [WHITEPAPER.ja.md](whitepaper/WHITEPAPER.ja.md) |
| Français | [WHITEPAPER.fr.md](whitepaper/WHITEPAPER.fr.md) |

## Yellow Paper (coming soon)

Technical specification and formal definitions for the VERITAS runtime — execution semantics, policy language, audit schema, and verification protocol.

## Project Status

VERITAS is fully implemented with 45 passing tests across 5 core crates, a healthcare reference runtime with 3 demo scenarios, a CLI demo runner, and an interactive TUI demo.

### Crate Overview

| Crate | Tests | Purpose |
|-------|-------|---------|
| `veritas-contracts` | 15 | Shared types, traits, error types |
| `veritas-core` | 6 | Deterministic executor pipeline |
| `veritas-policy` | 8 | TOML deny-by-default policy engine |
| `veritas-audit` | 6 | SHA-256 hash-chained audit trail |
| `veritas-verify` | 10 | JSON Schema + semantic rule verification |
| `veritas-ref-healthcare` | — | 3 healthcare demo scenarios |
| `demo` | — | CLI demo runner |
| `veritas-tui` | — | Interactive Ratatui TUI |

### Healthcare Demo Scenarios

1. **Drug Interaction Checker** — policy Allow flow, output schema verification
2. **Clinical Note Summarizer** — PII detection via custom verifier rule, audit trail
3. **Patient Data Query** — capability-based access control, consent enforcement (3 sub-cases: allow, capability-missing, consent-denied)

### Demos

```bash
cargo run -p demo -- run-all        # CLI: all 3 scenarios
cargo run -p demo -- drug-interaction  # individual scenario
cargo run -p veritas-tui             # interactive TUI
```

## Contributing Translations

We welcome community translations. To add a new language:

1. Copy `whitepaper/WHITEPAPER.en.md` as your base
2. Create `whitepaper/WHITEPAPER.<lang-code>.md` (use [ISO 639-1](https://en.wikipedia.org/wiki/List_of_ISO_639-1_codes) codes)
3. Translate all prose content — keep technical terms, markdown structure, and code blocks unchanged
4. Submit a pull request
