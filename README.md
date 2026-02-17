# VERITAS

Deterministic, policy-bound, auditable, and verifiable execution runtime for AI agents operating in regulated environments.

**Control over autonomy. Evidence over intelligence. Determinism over emergence.**

## Execution Model

```
State → Policy → Capability → Audit → Verify → Next State
```

## Core Components

| Component | Purpose |
|-----------|---------|
| `veritas-core/` | Deterministic runtime (ZeroClaw lineage) |
| `veritas-policy/` | Permission & risk engine |
| `veritas-audit/` | Immutable execution trace |
| `veritas-verify/` | Output validation |
| `veritas-contracts/` | Capability / policy / audit schemas |

## Trust Boundary

| Trusted | Untrusted |
|---------|-----------|
| Runtime core | LLM |
| Policy engine | Tools |
| Audit engine | Input data |
| Verifier | External environment |

## Documentation

See [`docs/`](./docs) for full documentation.

### Whitepaper v0.2

| Language | Link |
|----------|------|
| English | [WHITEPAPER.en.md](docs/whitepaper/WHITEPAPER.en.md) |
| 简体中文 | [WHITEPAPER.zh.md](docs/whitepaper/WHITEPAPER.zh.md) |
| 日本語 | [WHITEPAPER.ja.md](docs/whitepaper/WHITEPAPER.ja.md) |
| Français | [WHITEPAPER.fr.md](docs/whitepaper/WHITEPAPER.fr.md) |

## Contributing

VERITAS is open source. Community contributions — including new translations — are welcome. See [docs/README.md](docs/README.md) for guidelines.

## License

TBD
