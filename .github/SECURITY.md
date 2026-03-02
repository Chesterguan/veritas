# Security Policy

> Last updated: 2026-03-02

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

## Reporting a Vulnerability

**Do NOT report security vulnerabilities through public GitHub issues.**

Please report vulnerabilities by emailing **ziyuan.guan@ufl.edu** with:

1. Description of the vulnerability
2. Steps to reproduce
3. Affected component(s)
4. Potential impact assessment

## Response Timeline

- **Acknowledgment:** within 48 hours
- **Initial assessment:** within 5 business days
- **Fix or mitigation:** depends on severity, targeting 30 days for critical issues

## Scope

The following components are in scope for security reports:

- `veritas-core` — runtime executor
- `veritas-policy` — policy engine
- `veritas-audit` — audit trail
- `veritas-verify` — output verification
- `veritas-contracts` — shared types and schemas

The following are **out of scope**:

- `demo/` — CLI demo runner
- `tui/` — interactive TUI demo
- Documentation and whitepaper content

## Disclosure

We follow coordinated disclosure. We will credit reporters in the advisory unless anonymity is requested.
