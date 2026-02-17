//! TOML-driven policy engine implementation.
//!
//! `TomlPolicyEngine` loads a `PolicyConfig` from a TOML string or file and
//! implements the `PolicyEngine` trait from veritas-core.
//!
//! Evaluation algorithm:
//!
//! 1. Iterate rules in declaration order.
//! 2. For the first rule whose `action` and `resource` patterns match:
//!    a. Verify the agent holds every capability listed in `required_capabilities`.
//!       If any are missing → `Deny` (defense-in-depth; the `allow` verdict is
//!       overridden by missing capabilities, not by the rule order).
//!    b. Convert `RuleVerdict` → `PolicyVerdict` and return.
//! 3. If no rule matched → `Deny` with "denied by default" (deny-by-default policy).

use std::path::Path;

use tracing::{debug, warn};

use veritas_contracts::{
    error::{VeritasError, VeritasResult},
    policy::{PolicyContext, PolicyVerdict},
};
use veritas_core::traits::PolicyEngine;

use crate::rule::{PolicyConfig, RuleVerdict};

/// A `PolicyEngine` implementation that reads rules from a TOML document.
///
/// Construct via `from_toml_str` or `from_file`, then pass to the executor.
///
/// ```rust,ignore
/// use veritas_policy::engine::TomlPolicyEngine;
///
/// let engine = TomlPolicyEngine::from_file(Path::new("policies/healthcare.toml"))?;
/// ```
#[derive(Debug)]
pub struct TomlPolicyEngine {
    config: PolicyConfig,
}

impl TomlPolicyEngine {
    /// Parse `s` as TOML and build a `TomlPolicyEngine`.
    ///
    /// Returns `VeritasError::ConfigError` if the TOML is malformed or does
    /// not match the expected `PolicyConfig` schema.
    pub fn from_toml_str(s: &str) -> VeritasResult<Self> {
        let config: PolicyConfig = toml::from_str(s).map_err(|e| VeritasError::ConfigError {
            reason: format!("failed to parse policy TOML: {}", e),
        })?;
        Ok(Self { config })
    }

    /// Read the file at `path` and parse it as TOML policy configuration.
    ///
    /// Returns `VeritasError::ConfigError` if the file cannot be read or its
    /// contents are not valid TOML matching `PolicyConfig`.
    pub fn from_file(path: &Path) -> VeritasResult<Self> {
        let contents = std::fs::read_to_string(path).map_err(|e| VeritasError::ConfigError {
            reason: format!("failed to read policy file '{}': {}", path.display(), e),
        })?;
        Self::from_toml_str(&contents)
    }
}

impl PolicyEngine for TomlPolicyEngine {
    /// Evaluate the `PolicyContext` against the loaded rule set.
    ///
    /// Rules are tested in declaration order.  The first rule that matches
    /// `ctx.action` and `ctx.resource` is applied.  If the rule lists
    /// `required_capabilities`, they are verified against `ctx.capabilities`
    /// before the rule's own verdict is returned — a missing capability always
    /// produces `Deny`, even for an `allow` rule.
    ///
    /// If no rule matches, returns `PolicyVerdict::Deny` with the message
    /// "denied by default: no policy rule matched action '…' on resource '…'".
    fn evaluate(&self, ctx: &PolicyContext) -> VeritasResult<PolicyVerdict> {
        debug!(
            agent_id = %ctx.agent_id,
            action = %ctx.action,
            resource = %ctx.resource,
            "evaluating policy"
        );

        for rule in &self.config.rules {
            if !rule.matches(&ctx.action, &ctx.resource) {
                continue;
            }

            debug!(
                rule_id = %rule.id,
                action = %ctx.action,
                resource = %ctx.resource,
                "rule matched"
            );

            // Defense-in-depth capability check: even a matching allow rule is
            // overridden if the agent lacks a required capability.
            for required_cap in &rule.required_capabilities {
                if !ctx.capabilities.contains(required_cap) {
                    warn!(
                        rule_id = %rule.id,
                        capability = %required_cap,
                        agent_id = %ctx.agent_id,
                        "matched rule requires capability agent does not hold"
                    );
                    return Ok(PolicyVerdict::Deny {
                        reason: format!(
                            "rule '{}' requires capability '{}' which is not granted to agent '{}'",
                            rule.id, required_cap, ctx.agent_id
                        ),
                    });
                }
            }

            // Capability check passed — convert RuleVerdict to PolicyVerdict.
            let verdict = match rule.verdict {
                RuleVerdict::Allow => PolicyVerdict::Allow,

                RuleVerdict::Deny => PolicyVerdict::Deny {
                    reason: rule
                        .deny_reason
                        .clone()
                        .unwrap_or_else(|| format!("denied by rule '{}'", rule.id)),
                },

                RuleVerdict::RequireApproval => PolicyVerdict::RequireApproval {
                    reason: rule
                        .approval_reason
                        .clone()
                        .unwrap_or_else(|| format!("approval required by rule '{}'", rule.id)),
                    approver_role: rule
                        .approver_role
                        .clone()
                        .unwrap_or_else(|| "unspecified".to_string()),
                },

                RuleVerdict::RequireVerification => PolicyVerdict::RequireVerification {
                    check_id: rule
                        .verification_check_id
                        .clone()
                        .unwrap_or_else(|| format!("check-{}", rule.id)),
                },
            };

            return Ok(verdict);
        }

        // No rule matched — deny by default.
        warn!(
            action = %ctx.action,
            resource = %ctx.resource,
            agent_id = %ctx.agent_id,
            "no policy rule matched; denying by default"
        );

        Ok(PolicyVerdict::Deny {
            reason: format!(
                "denied by default: no policy rule matched action '{}' on resource '{}'",
                ctx.action, ctx.resource
            ),
        })
    }
}
