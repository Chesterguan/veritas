# Contributing to VERITAS

> Last updated: 2026-02-27

Thank you for your interest in contributing to VERITAS. This guide will help you get started.

## Development Setup

### Prerequisites

- Rust 1.70+ (`rustup` recommended)
- Git

### Getting Started

```bash
git clone https://github.com/veritas-rt/veritas.git
cd veritas
cargo test --workspace
cargo run -p demo -- run-all
```

If all 58 tests pass and the demo runs, you're ready to contribute.

## Project Structure

```
crates/
  veritas-contracts/       # Shared types, traits, error types
  veritas-core/            # Executor — enforces State → Policy → Capability → Audit → Verify → Next State
  veritas-policy/          # TOML-driven deny-by-default policy engine
  veritas-audit/           # SHA-256 hash-chained append-only audit trail
  veritas-verify/          # JSON Schema + semantic rule verification
  veritas-ref-healthcare/  # Healthcare reference runtime (5 demo scenarios)
demo/                      # CLI demo runner
tui/                       # Interactive TUI demo
docs/                      # Whitepaper v0.3 + Yellow Paper v0.1 (EN, ZH, JA, FR)
```

## How to Contribute

### Add a New Policy Rule

Edit `crates/veritas-policy/policies/healthcare.toml`:

```toml
[[rules]]
id = "your-rule-id"
description = "What this rule does"
action = "the-action"
resource = "the-resource"
required_capabilities = ["needed.capability"]
verdict = "allow"  # or "deny", "require-approval"
```

### Add a New Verification Rule

Register a custom rule in your scenario's `run_scenario()`:

```rust
verifier.register_rule("rule-name", |payload| {
    // Return None if valid, Some("error message") if invalid
    None
});
```

### Add a New Healthcare Scenario

1. Create `crates/veritas-ref-healthcare/src/scenarios/your_scenario.rs`
2. Implement the `Agent` trait from `veritas-core`
3. Add a `run_scenario()` function
4. Register it in `scenarios/mod.rs`
5. Add a subcommand in `demo/src/main.rs`

### Add a Translation

See [docs/README.md](docs/README.md) for translation guidelines.

## Code Guidelines

- Follow the 10 design principles (see CLAUDE.md)
- Keep the trusted computing base minimal
- Policy evaluation must be fast — microseconds, not milliseconds
- Agent code is untrusted — never bypass the executor pipeline
- All actions must be auditable
- Write tests for new functionality

## Commit Messages

Use clear, concise commit messages:

```
Add drug allergy checking scenario

Implements a new healthcare demo scenario that checks patient
allergies against prescribed medications.
```

## Pull Request Process

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/your-feature`)
3. Make your changes
4. Run `cargo test --workspace` — all tests must pass
5. Run `cargo clippy --workspace` — no warnings
6. Submit a PR with a clear description

## Code of Conduct

We follow the [Contributor Covenant](https://www.contributor-covenant.org/version/2/1/code_of_conduct/). Be respectful, constructive, and inclusive.

## Questions?

Open an issue or start a discussion on GitHub.
